//! MCP tools in the chat, with a scripted cloud model and an MCP server over
//! HTTP, both fakes served by the test.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use brainwashed_core::{
    ChatAccess, ChatEvent, ChatMessage, Engine, EngineConfig, McpServer, McpStatus, McpTransport,
    Provider, Role, SamplingOptions,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// An MCP server with one tool, `shout`, which upper-cases its text.
async fn mcp_server(calls: Arc<Mutex<Vec<Value>>>) -> String {
    async fn handle(
        State(calls): State<Arc<Mutex<Vec<Value>>>>,
        Json(msg): Json<Value>,
    ) -> Response {
        let Some(id) = msg.get("id").cloned() else {
            return StatusCode::ACCEPTED.into_response();
        };
        let result = match msg["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "shouter", "version": "1.0" },
            }),
            "tools/list" => json!({ "tools": [{
                "name": "shout",
                "description": "Upper-cases text.",
                "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] },
            }] }),
            "tools/call" => {
                calls.lock().unwrap().push(msg["params"].clone());
                let text = msg["params"]["arguments"]["text"]
                    .as_str()
                    .unwrap_or_default();
                json!({ "content": [{ "type": "text", "text": text.to_uppercase() }] })
            }
            _ => json!({}),
        };
        Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
    }
    let base = serve(Router::new().route("/mcp", post(handle)).with_state(calls)).await;
    format!("{base}/mcp")
}

fn sse(chunks: &[Value]) -> Response {
    let mut body = String::new();
    for c in chunks {
        body.push_str(&format!("data: {c}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    ([("content-type", "text/event-stream")], body).into_response()
}

/// A model that calls `tool` (when offered any tools) and, once it has a
/// tool's result, answers with it.
async fn model_server(requests: Arc<Mutex<Vec<Value>>>, tool: &'static str) -> String {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let requests = requests.clone();
            async move {
                requests.lock().unwrap().push(body.clone());
                let messages = body["messages"].as_array().unwrap();
                let last = messages.last().unwrap();
                let usage = json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 4 } });
                if last["role"] == "tool" {
                    let said = last["content"].as_str().unwrap_or_default();
                    return sse(&[
                        json!({ "choices": [{ "delta": { "content": format!("It says {said}.") } }] }),
                        usage,
                    ]);
                }
                if body["tools"].as_array().is_some_and(|t| !t.is_empty()) {
                    // Arguments streamed in two parts, as providers do.
                    return sse(&[
                        json!({ "choices": [{ "delta": { "content": "Let me check" } }] }),
                        json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "x", "type": "function",
                            "function": { "name": tool, "arguments": "{\"text\": " } }] } }] }),
                        json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0,
                            "function": { "arguments": "\"hello\"}" } }] }, "finish_reason": "tool_calls" }] }),
                        usage,
                    ]);
                }
                sse(&[
                    json!({ "choices": [{ "delta": { "content": "No tools here." } }] }),
                    usage,
                ])
            }
        }),
    );
    format!("{}/v1", serve(app).await)
}

async fn setup(
    tool: &'static str,
) -> (
    tempfile::TempDir,
    Engine,
    Arc<Mutex<Vec<Value>>>,
    Arc<Mutex<Vec<Value>>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(EngineConfig::new(dir.path(), "0.1.0")).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let model = model_server(requests.clone(), tool).await;
    engine
        .save_provider(Provider {
            id: "fake".into(),
            name: "Fake".into(),
            base_url: model,
            api_key: None,
            models: vec!["m".into()],
            members: true,
        })
        .unwrap();
    let url = mcp_server(calls.clone()).await;
    engine
        .save_mcp_server(McpServer {
            id: "shouter".into(),
            name: "Shouter".into(),
            enabled: true,
            members: false,
            transport: McpTransport::Http {
                url,
                headers: Default::default(),
            },
        })
        .await
        .unwrap();
    (dir, engine, requests, calls)
}

async fn chat(engine: &Engine, access: ChatAccess) -> (String, Vec<ChatEvent>) {
    let mut events = Vec::new();
    let answer = engine
        .chat_with(
            &[ChatMessage::new(Role::User, "What does the shouter say?")],
            &SamplingOptions::default(),
            Some("fake/m"),
            access,
            |e| events.push(e),
        )
        .await
        .unwrap();
    (answer, events)
}

#[tokio::test]
async fn runs_mcp_tools_the_model_calls() {
    let (_dir, engine, requests, calls) = setup("shout").await;

    let (answer, events) = chat(&engine, ChatAccess::ADMIN).await;
    assert_eq!(answer, "Let me check\n\nIt says HELLO.");
    assert_eq!(
        calls.lock().unwrap()[0],
        json!({ "name": "shout", "arguments": { "text": "hello" } })
    );

    // Calls, results and one set of stats for the whole reply.
    let call = events.iter().find_map(|e| match e {
        ChatEvent::ToolCall(c) => Some(c.clone()),
        _ => None,
    });
    assert_eq!(call.unwrap().arguments, json!({ "text": "hello" }));
    let result = events.iter().find_map(|e| match e {
        ChatEvent::ToolResult(r) => Some(r.clone()),
        _ => None,
    });
    let result = result.unwrap();
    assert_eq!(
        (
            result.name.as_str(),
            result.server.as_str(),
            result.content.as_str()
        ),
        ("shout", "Shouter", "HELLO")
    );
    assert!(!result.is_error);
    let stats: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Stats(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(stats.len(), 1);
    assert_eq!((stats[0].prompt_tokens, stats[0].tokens), (20, 8));

    // The tool was offered, and its result went back to the model.
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["tools"][0]["function"]["name"], "shout");
    assert_eq!(
        requests[0]["tools"][0]["function"]["parameters"]["required"],
        json!(["text"])
    );
    let sent = requests[1]["messages"].as_array().unwrap();
    let n = sent.len();
    assert_eq!(sent[n - 2]["role"], "assistant");
    assert_eq!(sent[n - 2]["content"], "Let me check");
    assert_eq!(sent[n - 2]["tool_calls"][0]["id"], "call00000");
    assert_eq!(
        sent[n - 2]["tool_calls"][0]["function"]["arguments"],
        "{\"text\":\"hello\"}"
    );
    assert_eq!(
        sent[n - 1],
        json!({ "role": "tool", "tool_call_id": "call00000", "content": "HELLO" })
    );

    // Admins see the server running with its tool.
    let servers = engine.mcp_servers();
    assert_eq!(servers[0].status, McpStatus::Ready);
    assert_eq!(servers[0].tools[0].name, "shout");
    assert_eq!(servers[0].server_info.as_deref(), Some("shouter 1.0"));
}

#[tokio::test]
async fn only_offers_tools_to_who_may_use_them() {
    let (_dir, engine, requests, calls) = setup("shout").await;
    let member = ChatAccess {
        admin: false,
        ask_user: true,
        tools: true,
    };
    // Members don't get servers the admin kept to admins.
    let (answer, _) = chat(&engine, member).await;
    assert_eq!(answer, "No tools here.");
    assert!(requests.lock().unwrap()[0].get("tools").is_none());

    // Once shared, they do.
    let mut server = engine.mcp_servers()[0].server.clone();
    server.members = true;
    engine.save_mcp_server(server).await.unwrap();
    let (answer, _) = chat(&engine, member).await;
    assert_eq!(answer, "Let me check\n\nIt says HELLO.");

    // Switched off, nobody does.
    engine
        .set_mcp_server_enabled("shouter", false)
        .await
        .unwrap();
    assert_eq!(engine.mcp_servers()[0].status, McpStatus::Off);
    let (answer, _) = chat(&engine, ChatAccess::ADMIN).await;
    assert_eq!(answer, "No tools here.");
    // Nor do chats that don't want tools, like the OpenAI-compatible API.
    engine
        .set_mcp_server_enabled("shouter", true)
        .await
        .unwrap();
    let api = ChatAccess {
        admin: true,
        ask_user: false,
        tools: false,
    };
    let (answer, _) = chat(&engine, api).await;
    assert_eq!(answer, "No tools here.");
    assert_eq!(calls.lock().unwrap().len(), 1);

    engine.delete_mcp_server("shouter").await.unwrap();
    assert!(engine.mcp_servers().is_empty());
}

#[tokio::test]
async fn tells_the_model_about_tools_that_dont_exist() {
    let (_dir, engine, requests, calls) = setup("whisper").await;
    let (answer, events) = chat(&engine, ChatAccess::ADMIN).await;
    assert!(calls.lock().unwrap().is_empty());
    let result = events
        .iter()
        .find_map(|e| match e {
            ChatEvent::ToolResult(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains("no tool named whisper"),
        "{}",
        result.content
    );
    assert!(
        answer.ends_with("It says Error: There is no tool named whisper. The tools are: shout.."),
        "{answer}"
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn history_with_tools_reaches_models_without_them() {
    let (_dir, engine, requests, _) = setup("shout").await;
    engine
        .set_mcp_server_enabled("shouter", false)
        .await
        .unwrap();
    let call: brainwashed_core::runtime::toolcalls::ToolCall = serde_json::from_value(
        json!({ "id": "call_0", "name": "shout", "arguments": { "text": "hello" } }),
    )
    .unwrap();
    let history = [
        ChatMessage::new(Role::User, "What does the shouter say?"),
        ChatMessage::calling("", vec![call]),
        ChatMessage::tool_result("call_0", "HELLO"),
        ChatMessage::new(Role::Assistant, "It says HELLO."),
        ChatMessage::new(Role::User, "Thanks"),
    ];
    engine
        .chat_with(
            &history,
            &SamplingOptions::default(),
            Some("fake/m"),
            ChatAccess::ADMIN,
            |_| {},
        )
        .await
        .unwrap();
    // No tools offered, so the calls are folded away.
    let sent = requests.lock().unwrap()[0]["messages"].clone();
    let roles: Vec<&str> = sent
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(sent[2]["content"], "It says HELLO.");
}

#[tokio::test]
async fn reports_servers_that_dont_start() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(EngineConfig::new(dir.path(), "0.1.0")).unwrap();
    let info = engine
        .save_mcp_server(McpServer {
            id: "broken".into(),
            name: "Broken".into(),
            enabled: true,
            members: false,
            transport: McpTransport::Stdio {
                command: "brainwashed-no-such-command".into(),
                args: vec![],
                env: Default::default(),
                cwd: None,
            },
        })
        .await
        .unwrap();
    assert_eq!(info.status, McpStatus::Starting);
    let mut status = info.status;
    for _ in 0..50 {
        status = engine.mcp_servers()[0].status.clone();
        if status != McpStatus::Starting {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    match status {
        McpStatus::Error { message } => assert!(message.contains("couldn't find"), "{message}"),
        other => panic!("expected an error, got {other:?}"),
    }
    // Survives a restart of the host, still on.
    drop(engine);
    let engine = Engine::new(EngineConfig::new(dir.path(), "0.1.0")).unwrap();
    assert!(engine.mcp_servers()[0].server.enabled);
}
