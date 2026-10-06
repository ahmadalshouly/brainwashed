use axum::{extract::Path as UrlPath, routing::get, Json, Router};
use brainwashed_core::{
    Attachment, Backend, ChatMessage, Engine, EngineConfig, EngineState, Role, SamplingOptions,
};
use std::time::Duration;

const MODEL_BYTES: &[u8] = b"GGUF-not-really";
const PROJECTOR_BYTES: &[u8] = b"GGUF-projector";

/// `repo/file` for each file the fake Hugging Face served, across tests in
/// this file (they run in parallel).
static FETCHED: std::sync::OnceLock<std::sync::Arc<std::sync::Mutex<Vec<String>>>> =
    std::sync::OnceLock::new();

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// A fake Hugging Face serving one repo, plus a fake GitHub release whose
/// llama.cpp "build" is a zip made by the test.
async fn fake_services(model: Vec<u8>, runtime_zip: Vec<u8>) -> String {
    let size = model.len();
    let fetched = FETCHED.get_or_init(Default::default).clone();
    let app = Router::new()
        .route(
            "/api/models/{owner}/{repo}/tree/main",
            // Only `-VL-` repos come with a vision projector, so the real
            // llama-server in the end-to-end test isn't handed a fake one.
            get(
                move |UrlPath((_, repo)): UrlPath<(String, String)>| async move {
                    let mut tree = vec![
                        serde_json::json!({"type": "file", "path": "Tiny-Q4_K_M.gguf", "size": size}),
                        serde_json::json!({"type": "file", "path": "Tiny-Q8_0.gguf", "size": 1}),
                    ];
                    if repo.contains("-VL-") {
                        tree.push(serde_json::json!({"type": "file", "path": "mmproj-Tiny-F16.gguf", "size": PROJECTOR_BYTES.len()}));
                    }
                    Json(tree)
                },
            ),
        )
        .route(
            "/{owner}/{repo}/resolve/main/{file}",
            get(
                move |UrlPath((_, repo, file)): UrlPath<(String, String, String)>| {
                    let model = model.clone();
                    let fetched = fetched.clone();
                    async move {
                        fetched.lock().unwrap().push(format!("{repo}/{file}"));
                        match file.as_str() {
                            "Tiny-Q4_K_M.gguf" => model,
                            "mmproj-Tiny-F16.gguf" => PROJECTOR_BYTES.to_vec(),
                            other => panic!("unexpected download {other}"),
                        }
                    }
                },
            ),
        )
        .route("/runtime.zip", get(move || async move { runtime_zip }));
    let base = serve(app).await;
    let release = serde_json::json!([{
        "tag_name": "b1",
        "assets": [{"name": "llama-b1-bin-ubuntu-x64.zip", "browser_download_url": format!("{base}/runtime.zip"), "size": 0}]
    }]);
    let rel_app = Router::new().route("/releases", get(move || async move { Json(release) }));
    let rel_base = serve(rel_app).await;
    format!("{base}|{rel_base}/releases")
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

    let model = engine
        .download_model("acme/Tiny-VL-GGUF", None)
        .await
        .unwrap();
    assert_eq!(model.id, "Tiny-Q4_K_M");
    assert_eq!(std::fs::read(&model.path).unwrap(), MODEL_BYTES);
    // The vision projector comes along.
    let mmproj = model.mmproj.clone().unwrap();
    assert_eq!(std::fs::read(&mmproj).unwrap(), PROJECTOR_BYTES);
    assert_eq!(engine.models(), vec![model.clone()]);

    // Downloading again fetches only what's missing.
    std::fs::remove_file(&mmproj).unwrap();
    let mine = || {
        FETCHED
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.starts_with("Tiny-VL-GGUF/"))
            .cloned()
            .collect::<Vec<_>>()
    };
    let before = mine().len();
    engine
        .download_model("acme/Tiny-VL-GGUF", None)
        .await
        .unwrap();
    assert_eq!(
        mine()[before..],
        ["Tiny-VL-GGUF/mmproj-Tiny-F16.gguf".to_string()]
    );

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
    assert!(!mmproj.exists());
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

#[tokio::test]
async fn chat_checks_options_and_pictures() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), "http://127.0.0.1:9|http://127.0.0.1:9");
    let hot = SamplingOptions {
        temperature: Some(5.0),
        ..Default::default()
    };
    let err = engine
        .chat(&[ChatMessage::new(Role::User, "hi")], &hot, |_| {})
        .await
        .unwrap_err();
    assert!(err.to_string().contains("temperature"), "{err}");

    let mut message = ChatMessage::new(Role::User, "what's this?");
    message.attachments.push(Attachment::Image {
        name: "x.svg".into(),
        mime: "image/svg+xml".into(),
        data: "PHN2Zz4=".into(),
    });
    let err = engine
        .chat(&[message], &SamplingOptions::default(), |_| {})
        .await
        .unwrap_err();
    assert!(err.to_string().contains("PNG, JPEG"), "{err}");

    // The admin's chat defaults are checked when saved.
    let mut settings = engine.settings();
    settings.chat_defaults.temperature = Some(9.0);
    let err = engine.update_settings(settings).unwrap_err();
    assert!(err.to_string().contains("temperature"), "{err}");
}

#[test]
fn system_prompt_is_kept_and_client_system_appended() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_blocking(dir.path());
    let (prompt, used) = engine.build_prompt(&[
        ChatMessage::new(Role::System, "Speak French."),
        ChatMessage::new(Role::User, "hi"),
    ]);
    assert!(used.is_empty());
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
    // The test model spends a token per byte, and the skill index alone is
    // a few hundred bytes.
    s.context_size = 2048;
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
            |e| {
                if !matches!(e, brainwashed_core::ChatEvent::Skills { .. }) {
                    pieces += 1
                }
            },
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
    // Another test forking while the copy is still open for writing makes
    // the exec fail with "text file busy" until that child execs too.
    let mut orphan = (0..50)
        .find_map(
            |_| match std::process::Command::new(&fake).arg("30").spawn() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    None
                }
                other => Some(other.unwrap()),
            },
        )
        .expect("the copied program stayed busy");
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

/// The start of a GGUF file: its architecture and one tensor name.
fn gguf_header(arch: &str) -> Vec<u8> {
    let string = |out: &mut Vec<u8>, s: &str| {
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s.as_bytes());
    };
    let mut out = b"GGUF".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend(1u64.to_le_bytes());
    out.extend(1u64.to_le_bytes());
    string(&mut out, "general.architecture");
    out.extend(8u32.to_le_bytes());
    string(&mut out, arch);
    string(&mut out, "blk.0.attn_q.weight");
    out.extend(1u32.to_le_bytes());
    out.extend(64u64.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out
}

#[cfg(unix)]
#[tokio::test]
async fn drafts_only_run_as_a_speedup() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), "http://127.0.0.1:9|http://127.0.0.1:9");
    let main = dir.path().join("Qwen3-4B-Q4_K_M.gguf");
    let draft = dir.path().join("Qwen3-4B-DFlash-Q4_K_M.gguf");
    std::fs::write(&main, gguf_header("qwen3")).unwrap();
    std::fs::write(&draft, gguf_header("dflash")).unwrap();
    let main = engine.import_model(&main).unwrap();
    let draft = engine.import_model(&draft).unwrap();
    assert_eq!(main.draft, None);
    assert_eq!(draft.draft.as_deref(), Some("draft-dflash"));

    // A draft alone is refused with a clear reason, before llama-server runs.
    let err = engine.loadable(&draft.id).unwrap_err().to_string();
    assert!(err.contains("DFlash speed-up"), "{err}");
    assert!(engine.load_model(&draft.id).await.is_err());
    assert!(engine.set_speedup(&draft.id, Some(&main.id)).is_err());
    assert!(engine.set_speedup(&main.id, Some(&main.id)).is_err());

    // Paired, the draft goes to llama-server next to the main model.
    engine.set_speedup(&main.id, Some(&draft.id)).unwrap();
    let args = dir.path().join("args.txt");
    let fake = dir.path().join("llama-server");
    std::fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$@\" > {}\nexit 1\n", args.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut s = engine.settings();
    s.llama_server_path = Some(fake);
    engine.update_settings(s).unwrap();
    let err = engine.load_model(&main.id).await.unwrap_err().to_string();
    assert!(err.contains("speed-up"), "{err}");
    let args = std::fs::read_to_string(args).unwrap();
    assert!(
        args.contains(&format!("--model-draft {}", draft.path.display())),
        "{args}"
    );
    assert!(args.contains("--spec-type draft-dflash"), "{args}");

    // Deleting the draft turns the speed-up off.
    engine.delete_model(&draft.id).await.unwrap();
    let main = engine
        .models()
        .into_iter()
        .find(|m| m.id == main.id)
        .unwrap();
    assert_eq!(main.speedup, None);
}
