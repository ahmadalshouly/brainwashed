use axum::{extract::Path as UrlPath, routing::get, Json, Router};
use brainwashed_core::{
    Backend, ChatMessage, Engine, EngineConfig, EngineState, Role, SamplingOptions,
};
use std::time::Duration;

const MODEL_BYTES: &[u8] = b"GGUF-not-really";

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A fake Hugging Face serving one repo, plus a fake GitHub release whose
/// llama.cpp "build" is a zip made by the test.
async fn fake_services(model: Vec<u8>, runtime_zip: Vec<u8>) -> String {
    let tree = serde_json::json!([
        {"type": "file", "path": "Tiny-Q4_K_M.gguf", "size": model.len()},
        {"type": "file", "path": "Tiny-Q8_0.gguf", "size": 1}
    ]);
    let app = Router::new()
        .route(
            "/api/models/{owner}/{repo}/tree/main",
            get(move || async move { Json(tree) }),
        )
        .route(
            "/{owner}/{repo}/resolve/main/{file}",
            get(
                move |UrlPath((_, _, file)): UrlPath<(String, String, String)>| {
                    let model = model.clone();
                    async move {
                        assert_eq!(file, "Tiny-Q4_K_M.gguf");
                        model
                    }
                },
            ),
        )
        .route("/runtime.zip", get(move || async move { runtime_zip }));
    let base = serve(app).await;
    let release = serde_json::json!({
        "tag_name": "b1",
        "assets": [{"name": "llama-b1-bin-ubuntu-x64.zip", "browser_download_url": format!("{base}/runtime.zip"), "size": 0}]
    });
    let rel_app = Router::new().route("/latest", get(move || async move { Json(release) }));
    let rel_base = serve(rel_app).await;
    format!("{base}|{rel_base}/latest")
}

fn engine(dir: &std::path::Path, services: &str) -> Engine {
    let (hf, gh) = services.split_once('|').unwrap();
    let mut cfg = EngineConfig::new(dir, "0.1.0");
    cfg.hf_base = hf.to_string();
    cfg.github_release_url = gh.to_string();
    cfg.load_timeout = Duration::from_secs(60);
    let engine = Engine::new(cfg).unwrap();
    let mut s = engine.settings();
    s.backend = Backend::Cpu;
    engine.update_settings(s).unwrap();
    engine
}

#[tokio::test]
async fn downloads_lists_and_deletes_models() {
    let dir = tempfile::tempdir().unwrap();
    let services = fake_services(MODEL_BYTES.to_vec(), vec![]).await;
    let engine = engine(dir.path(), &services);
    let mut events = engine.subscribe();

    let model = engine.download_model("acme/Tiny-GGUF", None).await.unwrap();
    assert_eq!(model.id, "Tiny-Q4_K_M");
    assert_eq!(std::fs::read(&model.path).unwrap(), MODEL_BYTES);
    assert_eq!(engine.models(), vec![model.clone()]);

    let mut saw_progress = false;
    while let Ok(ev) = events.try_recv() {
        saw_progress |= matches!(ev, brainwashed_core::Event::DownloadProgress { .. });
    }
    assert!(saw_progress);

    // Survives a restart.
    let reopened = self::engine(dir.path(), &services);
    assert_eq!(reopened.models().len(), 1);

    engine.delete_model(&model.id).await.unwrap();
    assert!(engine.models().is_empty());
    assert!(!model.path.exists());
}

#[tokio::test]
async fn rejects_bad_repo_names() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), "http://127.0.0.1:9|http://127.0.0.1:9");
    assert!(engine.download_model("../etc", None).await.is_err());
    assert!(engine.download_model("just-a-name", None).await.is_err());
}

#[tokio::test]
async fn chat_needs_a_loaded_model() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), "http://127.0.0.1:9|http://127.0.0.1:9");
    let err = engine
        .chat(
            &[ChatMessage::new(Role::User, "hi")],
            &SamplingOptions::default(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no model"));
}

#[test]
fn system_prompt_is_kept_and_client_system_appended() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_blocking(dir.path());
    let prompt = engine.build_prompt(&[
        ChatMessage::new(Role::System, "Speak French."),
        ChatMessage::new(Role::User, "hi"),
    ]);
    assert_eq!(prompt.len(), 2);
    assert_eq!(prompt[0].role, Role::System);
    assert!(prompt[0].content.starts_with("You are BrainWashed"));
    assert!(prompt[0].content.ends_with("Speak French."));
}

fn engine_blocking(dir: &std::path::Path) -> Engine {
    Engine::new(EngineConfig::new(dir, "0.1.0")).unwrap()
}

/// Full flow against a real llama-server: install the runtime from a fake
/// release, download a model from a fake Hugging Face, load it and chat.
/// Set BRAINWASHED_TEST_LLAMA_SERVER and BRAINWASHED_TEST_MODEL to run it.
#[cfg(unix)]
#[tokio::test]
async fn end_to_end_with_real_llama_server() {
    let (Some(server), Some(model)) = (
        std::env::var("BRAINWASHED_TEST_LLAMA_SERVER").ok(),
        std::env::var("BRAINWASHED_TEST_MODEL").ok(),
    ) else {
        eprintln!("skipping: BRAINWASHED_TEST_LLAMA_SERVER / BRAINWASHED_TEST_MODEL not set");
        return;
    };
    // The "release" contains a launcher that execs the real binary.
    let mut zip_bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
        zip.start_file(
            "llama-b1/llama-server",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(
            &mut zip,
            format!("#!/bin/sh\nexec '{server}' \"$@\"\n").as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    let services = fake_services(std::fs::read(model).unwrap(), zip_bytes).await;
    let engine = engine(dir.path(), &services);
    let mut s = engine.settings();
    s.context_size = 512;
    s.gpu_layers = 0;
    engine.update_settings(s).unwrap();

    let model = engine.download_model("acme/Tiny-GGUF", None).await.unwrap();
    engine.load_model(&model.id).await.unwrap();
    assert_eq!(
        engine.state(),
        EngineState::Ready {
            model: model.id.clone()
        }
    );
    assert_eq!(engine.info().model.as_deref(), Some(model.id.as_str()));

    let mut pieces = 0;
    engine
        .chat(
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

    // Reloading reuses the installed runtime and restores the active model.
    engine.unload().await.unwrap();
    assert_eq!(engine.state(), EngineState::Idle);
    engine.restore().await.unwrap();
    assert!(matches!(engine.state(), EngineState::Ready { .. }));
    engine.unload().await.unwrap();
}

#[cfg(unix)]
#[test]
fn stops_llama_server_left_over_from_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    // Stand-in for a llama-server that kept running after its app died.
    let fake = dir.path().join("llama-server");
    std::fs::copy("/bin/sleep", &fake).unwrap();
    let mut orphan = std::process::Command::new(&fake).arg("30").spawn().unwrap();
    std::fs::write(dir.path().join("llama-server.pid"), orphan.id().to_string()).unwrap();

    let _engine = engine_blocking(dir.path());

    let status = orphan.wait().unwrap();
    assert!(
        status.code().is_none(),
        "stale server should have been killed, got {status}"
    );
    assert!(!dir.path().join("llama-server.pid").exists());
}

#[cfg(unix)]
#[test]
fn leaves_unrelated_processes_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut other = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    std::fs::write(dir.path().join("llama-server.pid"), other.id().to_string()).unwrap();

    let _engine = engine_blocking(dir.path());

    assert!(
        other.try_wait().unwrap().is_none(),
        "unrelated process was killed"
    );
    other.kill().unwrap();
}
