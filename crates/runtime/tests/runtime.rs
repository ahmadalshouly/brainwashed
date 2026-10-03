use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use brainwashed_runtime::{
    chat::{stream_chat, Delta, SamplingOptions},
    download::{download, extract, find_file, sha256_file},
    llama::{LlamaServer, ServerOptions},
    ChatMessage, Role,
};
use std::{path::PathBuf, sync::Arc, time::Duration};

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
        "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let base = serve(Router::new().route(
        "/v1/chat/completions",
        post(move || async move {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(sse))
                .unwrap()
        }),
    ))
    .await;

    let mut deltas = Vec::new();
    let answer = stream_chat(
        &client(),
        &base,
        &[ChatMessage::new(Role::User, "hi")],
        &SamplingOptions::default(),
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
            Delta::Content("lo".into())
        ]
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
        context_size: 512,
        gpu_layers: 0,
        port: None,
    };
    let client = client();
    let server = LlamaServer::start(&opts, &client, Duration::from_secs(60))
        .await
        .unwrap();
    let mut pieces = 0;
    stream_chat(
        &client,
        &server.base_url(),
        &[ChatMessage::new(Role::User, "hi")],
        &SamplingOptions {
            max_tokens: Some(8),
            ..Default::default()
        },
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
