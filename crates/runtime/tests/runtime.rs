use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use brainwashed_runtime::{
    chat::{stream_chat, Delta, SamplingOptions},
    download::{download, extract, find_file, sha256_file},
    llama::{LlamaServer, ServerOptions},
    Attachment, ChatMessage, Endpoint, ReplyStats, Role,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

const BLOB: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

async fn ranged(State(blob): State<Arc<Vec<u8>>>, headers: HeaderMap) -> Response {
    let start = headers
        .get("range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
        .and_then(|v| v.trim_end_matches('-').parse::<usize>().ok());
    match start {
        Some(s) => Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .body(Body::from(blob[s..].to_vec()))
            .unwrap(),
        None => Response::new(Body::from(blob.to_vec())),
    }
}

#[tokio::test]
async fn download_resumes_and_verifies_checksum() {
    let base = serve(
        Router::new()
            .route("/blob", get(ranged))
            .with_state(Arc::new(BLOB.to_vec())),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("model.gguf");
    // Simulate an interrupted earlier download.
    std::fs::write(dir.path().join("model.gguf.part"), &BLOB[..10]).unwrap();

    let expected = {
        std::fs::write(dir.path().join("ref"), BLOB).unwrap();
        sha256_file(&dir.path().join("ref")).await.unwrap()
    };
    let last = std::sync::Mutex::new((0, None));
    download(
        &client(),
        &format!("{base}/blob"),
        &dest,
        Some(&expected),
        &|d, t| *last.lock().unwrap() = (d, t),
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&dest).unwrap(), BLOB);
    assert_eq!(
        *last.lock().unwrap(),
        (BLOB.len() as u64, Some(BLOB.len() as u64))
    );
    assert!(!dir.path().join("model.gguf.part").exists());
}

#[tokio::test]
async fn download_rejects_bad_checksum() {
    let base = serve(Router::new().route("/blob", get(|| async { "data" }))).await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("f");
    let err = download(
        &client(),
        &format!("{base}/blob"),
        &dest,
        Some("00"),
        &|_, _| {},
    )
    .await;
    assert!(err.is_err());
    assert!(!dest.exists());
}

#[tokio::test]
async fn extracts_zip_and_finds_binary() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("llama.zip");
    {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        zip.start_file(
            "build/bin/llama-server",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"bin").unwrap();
        zip.finish().unwrap();
    }
    let out = dir.path().join("out");
    extract(&archive, &out).await.unwrap();
    assert!(find_file(&out, "llama-server")
        .unwrap()
        .ends_with("build/bin/llama-server"));
}

#[tokio::test]
async fn streams_content_and_reasoning() {
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":null}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"length\"}],",
        "\"timings\":{\"prompt_n\":12,\"predicted_n\":2,\"predicted_per_second\":40.5}}\n\n",
        "data: [DONE]\n\n",
    );
    let seen = Arc::new(Mutex::new(serde_json::Value::Null));
    let seen_by_server = seen.clone();
    let base = serve(Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<serde_json::Value>| async move {
            *seen_by_server.lock().unwrap() = body;
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(sse))
                .unwrap()
        }),
    ))
    .await;

    let message = ChatMessage {
        role: Role::User,
        content: "What's this?".into(),
        attachments: vec![
            Attachment::File {
                name: "notes.txt".into(),
                text: "buy milk".into(),
            },
            Attachment::Image {
                name: "cat.png".into(),
                mime: "image/png".into(),
                data: "AAAA".into(),
            },
        ],
    };
    let options = SamplingOptions {
        temperature: Some(0.2),
        top_k: Some(20),
        reasoning: Some(false),
        ..Default::default()
    };
    let mut deltas = Vec::new();
    let answer = stream_chat(
        &client(),
        &Endpoint::llama(&base),
        std::slice::from_ref(&message),
        &options,
        true,
        |d| deltas.push(d),
    )
    .await
    .unwrap();

    assert_eq!(answer, "Hello");
    assert_eq!(
        deltas,
        vec![
            Delta::Reasoning("hmm".into()),
            Delta::Content("Hel".into()),
            Delta::Content("lo".into()),
            Delta::Stats(ReplyStats {
                prompt_tokens: 12,
                tokens: 2,
                tokens_per_second: 40.5,
                truncated: true,
            }),
        ]
    );

    let body = seen.lock().unwrap().clone();
    assert_eq!(body["stream"], true);
    assert!((body["temperature"].as_f64().unwrap() - 0.2).abs() < 1e-6);
    assert_eq!(body["top_k"], 20);
    assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
    assert!(body.get("top_p").is_none());
    let parts = &body["messages"][0]["content"];
    assert_eq!(parts[0]["image_url"]["url"], "data:image/png;base64,AAAA");
    assert_eq!(
        parts[1]["text"],
        "<file name=\"notes.txt\">\nbuy milk\n</file>\n\nWhat's this?"
    );

    // A model that can't see gets the picture's name instead.
    stream_chat(
        &client(),
        &Endpoint::llama(&base),
        &[message],
        &options,
        false,
        |_| {},
    )
    .await
    .unwrap();
    let body = seen.lock().unwrap().clone();
    assert_eq!(
        body["messages"][0]["content"],
        "<file name=\"notes.txt\">\nbuy milk\n</file>\n\n[Picture: cat.png]\nWhat's this?"
    );
}

/// Runs only when a real llama-server and GGUF model are provided, e.g.
/// `BRAINWASHED_TEST_LLAMA_SERVER=/path/llama-server BRAINWASHED_TEST_MODEL=/path/m.gguf`.
#[tokio::test]
async fn real_llama_server_round_trip() {
    let (Some(binary), Some(model)) = (
        std::env::var_os("BRAINWASHED_TEST_LLAMA_SERVER"),
        std::env::var_os("BRAINWASHED_TEST_MODEL"),
    ) else {
        eprintln!("skipping: BRAINWASHED_TEST_LLAMA_SERVER / BRAINWASHED_TEST_MODEL not set");
        return;
    };
    let opts = ServerOptions {
        binary: PathBuf::from(binary),
        model: PathBuf::from(model),
        mmproj: None,
        draft: None,
        context_size: 512,
        gpu_layers: 0,
        port: None,
    };
    let client = client();
    let server = LlamaServer::start(&opts, &client, Duration::from_secs(60))
        .await
        .unwrap();
    assert!(server.context_size().is_some_and(|n| n > 0 && n <= 512));
    let mut pieces = 0;
    stream_chat(
        &client,
        &Endpoint::llama(server.base_url()),
        &[ChatMessage::new(Role::User, "hi")],
        &SamplingOptions {
            max_tokens: Some(8),
            ..Default::default()
        },
        false,
        |_| pieces += 1,
    )
    .await
    .unwrap();
    assert!(pieces > 0);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn reports_startup_failure_with_log() {
    let dir = tempfile::tempdir().unwrap();
    let opts = ServerOptions {
        binary: dir.path().join("does-not-exist"),
        model: dir.path().join("m.gguf"),
        mmproj: None,
        draft: None,
        context_size: 512,
        gpu_layers: 0,
        port: None,
    };
    let err = LlamaServer::start(&opts, &client(), Duration::from_secs(5))
        .await
        .err()
        .unwrap();
    assert!(err.to_string().contains("could not start"));
}

#[tokio::test]
async fn talks_to_cloud_providers() {
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning\":\"plan\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3}}\n\n",
        "data: [DONE]\n\n",
    );
    let seen = Arc::new(Mutex::new((String::new(), serde_json::Value::Null)));
    let seen_by_server = seen.clone();
    let base = serve(
        Router::new()
            .route(
                "/v1/chat/completions",
                post(
                    move |headers: HeaderMap, Json(body): Json<serde_json::Value>| async move {
                        let auth = headers["authorization"].to_str().unwrap().to_string();
                        *seen_by_server.lock().unwrap() = (auth, body);
                        Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(Body::from(sse))
                            .unwrap()
                    },
                ),
            )
            .route(
                "/v1/models",
                get(|headers: HeaderMap| async move {
                    if headers.get("authorization").and_then(|h| h.to_str().ok())
                        != Some("Bearer sk-good")
                    {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(serde_json::json!({"error": {"message": "Incorrect API key"}})),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"data": [{"id": "gpt-b"}, {"id": "gpt-a"}]})),
                    )
                }),
            ),
    )
    .await;
    let endpoint = Endpoint {
        base_url: format!("{base}/v1/"),
        api_key: Some("sk-good".into()),
        model: Some("gpt-a".into()),
        llama: false,
    };

    assert_eq!(
        brainwashed_runtime::chat::list_models(&client(), &endpoint)
            .await
            .unwrap(),
        vec!["gpt-a", "gpt-b"]
    );
    let wrong = Endpoint {
        api_key: Some("sk-bad".into()),
        ..endpoint.clone()
    };
    let err = brainwashed_runtime::chat::list_models(&client(), &wrong)
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("refused the API key: Incorrect API key"),
        "{err}"
    );

    let options = SamplingOptions {
        temperature: Some(0.5),
        top_k: Some(40),
        reasoning: Some(false),
        ..Default::default()
    };
    let mut deltas = Vec::new();
    let answer = stream_chat(
        &client(),
        &endpoint,
        &[ChatMessage::new(Role::User, "hello")],
        &options,
        true,
        |d| deltas.push(d),
    )
    .await
    .unwrap();
    assert_eq!(answer, "Hi");
    assert_eq!(deltas[0], Delta::Reasoning("plan".into()));
    let Delta::Stats(stats) = deltas.last().unwrap() else {
        panic!("no stats")
    };
    assert_eq!((stats.prompt_tokens, stats.tokens), (9, 3));

    let (auth, body) = seen.lock().unwrap().clone();
    assert_eq!(auth, "Bearer sk-good");
    assert_eq!(body["model"], "gpt-a");
    assert_eq!(body["stream_options"]["include_usage"], true);
    // llama.cpp-only fields stay home.
    assert!(body.get("top_k").is_none());
    assert!(body.get("chat_template_kwargs").is_none());
}
