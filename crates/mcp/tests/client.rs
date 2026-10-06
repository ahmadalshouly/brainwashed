use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use brainwashed_mcp::{Client, Error, Options, Transport};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn options() -> Options {
    Options {
        start_timeout: Duration::from_secs(20),
        ..Options::new("BrainWashed tests", "0.0.0")
    }
}

fn test_server() -> Transport {
    Transport::Stdio {
        command: env!("CARGO_BIN_EXE_mcp-test-server").into(),
        args: vec![],
        env: BTreeMap::new(),
        cwd: None,
    }
}

#[tokio::test]
async fn runs_tools_of_a_command_server() {
    let client = Client::connect(&test_server(), &options()).await.unwrap();
    assert_eq!(client.info().name, "test-server");
    assert_eq!(
        client.info().instructions.as_deref(),
        Some("Use echo to repeat things.")
    );

    let tools = client.list_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["echo", "add", "fail", "slow", "pinged", "crash"]);
    // A tool without a schema gets an empty one.
    assert_eq!(tools[2].input_schema["type"], "object");

    let long = Duration::from_secs(10);
    let echo = client
        .call_tool("echo", json!({ "text": "hello there" }), long)
        .await
        .unwrap();
    assert_eq!(echo.text(), "hello there");
    assert!(!echo.is_error);
    let sum = client
        .call_tool("add", json!({ "a": 2, "b": 3.5 }), long)
        .await
        .unwrap();
    assert_eq!(sum.text(), "5.5");
    let failed = client.call_tool("fail", json!({}), long).await.unwrap();
    assert!(failed.is_error);
    assert_eq!(failed.text(), "it broke");
    match client.call_tool("nope", json!({}), long).await {
        Err(Error::Rpc { code, message }) => {
            assert_eq!(code, -32602);
            assert_eq!(message, "Unknown tool");
        }
        other => panic!("expected an error, got {other:?}"),
    }
    // The server pinged us right after the handshake.
    let pinged = client.call_tool("pinged", json!({}), long).await.unwrap();
    assert_eq!(pinged.text(), "true");

    // Giving up on a call tells the server to stop.
    let slow = client
        .call_tool("slow", json!({ "ms": 3000 }), Duration::from_millis(300))
        .await;
    assert!(matches!(slow, Err(Error::Timeout(_))), "{slow:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        client.log().iter().any(|l| l.starts_with("cancelled")),
        "{:?}",
        client.log()
    );
    // Calls still work after that.
    let echo = client
        .call_tool("echo", json!({ "text": "still here" }), long)
        .await
        .unwrap();
    assert_eq!(echo.text(), "still here");

    // A server that dies says why.
    let crash = client.call_tool("crash", json!({}), long).await;
    match crash {
        Err(Error::Closed(why)) => assert!(why.contains("crashing on purpose"), "{why}"),
        other => panic!("expected the server to stop, got {other:?}"),
    }
    assert!(!client.is_alive());
    assert!(client.call_tool("echo", json!({}), long).await.is_err());
}

#[tokio::test]
async fn explains_commands_that_dont_start() {
    let missing = Transport::Stdio {
        command: "brainwashed-no-such-command".into(),
        args: vec![],
        env: BTreeMap::new(),
        cwd: None,
    };
    let e = Client::connect(&missing, &options()).await.err().unwrap();
    assert!(e.to_string().contains("couldn't find"), "{e}");
}

#[tokio::test]
async fn closing_stops_the_command() {
    let client = Client::connect(&test_server(), &options()).await.unwrap();
    client.close().await;
    assert!(!client.is_alive());
    let e = client
        .call_tool("echo", json!({ "text": "x" }), Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(matches!(e, Error::Closed(_)), "{e:?}");
}

// ----- HTTP -----

#[derive(Default)]
struct Seen {
    sessions: Vec<Option<String>>,
    deleted: bool,
    auth: Vec<Option<String>>,
}

type Shared = Arc<Mutex<Seen>>;

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn answer(id: &Value, method: &str, params: &Value) -> Value {
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "http-test", "version": "2.0" },
        }),
        "tools/list" => json!({ "tools": [
            { "name": "echo", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } } } }
        ] }),
        "tools/call" => json!({
            "content": [{ "type": "text", "text": params["arguments"]["text"].as_str().unwrap_or_default() }]
        }),
        _ => json!({}),
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

async fn streamable(
    State(seen): State<Shared>,
    headers: HeaderMap,
    Json(msg): Json<Value>,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    {
        let mut seen = seen.lock().unwrap();
        seen.sessions.push(header("mcp-session-id"));
        seen.auth.push(header("authorization"));
    }
    let method = msg["method"].as_str().unwrap_or_default();
    if method != "initialize" && header("mcp-session-id").as_deref() != Some("s-1") {
        return (StatusCode::BAD_REQUEST, "missing session").into_response();
    }
    if method != "initialize" && header("mcp-protocol-version").as_deref() != Some("2025-03-26") {
        return (StatusCode::BAD_REQUEST, "missing protocol version").into_response();
    }
    if msg.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let reply = answer(&msg["id"], method, &msg["params"]);
    match method {
        "initialize" => ([("mcp-session-id", "s-1")], Json(reply)).into_response(),
        // Lists come as an event stream, after a notification and a ping.
        "tools/list" => {
            let body = format!(
                "event: message\ndata: {}\n\nevent: message\ndata: {}\n\ndata: {}\n\n",
                json!({ "jsonrpc": "2.0", "method": "notifications/message", "params": { "level": "info", "data": "hi" } }),
                json!({ "jsonrpc": "2.0", "id": 99, "method": "ping" }),
                reply
            );
            ([("content-type", "text/event-stream")], body).into_response()
        }
        _ => Json(reply).into_response(),
    }
}

async fn end_session(State(seen): State<Shared>) -> StatusCode {
    seen.lock().unwrap().deleted = true;
    StatusCode::OK
}

#[tokio::test]
async fn talks_streamable_http() {
    let seen: Shared = Default::default();
    let base = serve(
        Router::new()
            .route("/mcp", post(streamable).delete(end_session))
            .with_state(seen.clone()),
    )
    .await;
    let transport = Transport::Http {
        url: format!("{base}/mcp"),
        headers: BTreeMap::from([("Authorization".into(), "Bearer t0k".into())]),
    };
    let client = Client::connect(&transport, &options()).await.unwrap();
    assert_eq!(client.info().name, "http-test");
    assert_eq!(client.info().protocol_version, "2025-03-26");
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 1);
    let echo = client
        .call_tool(
            "echo",
            json!({ "text": "over http" }),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(echo.text(), "over http");
    client.close().await;

    let seen = seen.lock().unwrap();
    assert_eq!(seen.sessions[0], None, "no session before initialize");
    assert!(seen.sessions[1..]
        .iter()
        .all(|s| s.as_deref() == Some("s-1")));
    assert!(seen.auth.iter().all(|a| a.as_deref() == Some("Bearer t0k")));
    assert!(seen.deleted, "closing ends the session");
}

#[tokio::test]
async fn explains_http_failures() {
    let base =
        serve(Router::new().route("/mcp", post(|| async { (StatusCode::UNAUTHORIZED, "no") })))
            .await;
    let e = Client::connect(
        &Transport::Http {
            url: format!("{base}/mcp"),
            headers: BTreeMap::new(),
        },
        &options(),
    )
    .await
    .err()
    .unwrap();
    assert!(e.to_string().contains("refused access"), "{e}");

    let e = Client::connect(
        &Transport::Http {
            url: format!("{base}/nothing-here"),
            headers: BTreeMap::new(),
        },
        &options(),
    )
    .await
    .err()
    .unwrap();
    assert!(e.to_string().contains("returned 404"), "{e}");
}

// The older HTTP+SSE transport: a GET event stream says where to POST, and
// answers come back on the stream.
type Outbox = Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>>;

async fn legacy_stream(State(outbox): State<Outbox>) -> Response {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tx.send("event: endpoint\ndata: /messages?sessionId=abc\n\n".into())
        .unwrap();
    *outbox.lock().unwrap() = Some(tx);
    let stream = tokio_stream_from(rx);
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

fn tokio_stream_from(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
) -> impl futures_util::Stream<Item = Result<String, std::convert::Infallible>> {
    futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx).map(|o| o.map(Ok)))
}

async fn legacy_post(State(outbox): State<Outbox>, Json(msg): Json<Value>) -> StatusCode {
    if let (Some(id), Some(method)) = (msg.get("id"), msg["method"].as_str()) {
        let reply = answer(id, method, &msg["params"]);
        if let Some(tx) = outbox.lock().unwrap().as_ref() {
            let _ = tx.send(format!("event: message\ndata: {reply}\n\n"));
        }
    }
    StatusCode::ACCEPTED
}

#[tokio::test]
async fn falls_back_to_the_older_sse_transport() {
    let outbox: Outbox = Default::default();
    let base = serve(
        Router::new()
            .route("/sse", get(legacy_stream))
            .route("/messages", post(legacy_post))
            .with_state(outbox),
    )
    .await;
    let transport = Transport::Http {
        url: format!("{base}/sse"),
        headers: BTreeMap::new(),
    };
    let client = Client::connect(&transport, &options()).await.unwrap();
    assert_eq!(client.info().name, "http-test");
    assert_eq!(client.list_tools().await.unwrap()[0].name, "echo");
    let echo = client
        .call_tool(
            "echo",
            json!({ "text": "old school" }),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(echo.text(), "old school");
}
