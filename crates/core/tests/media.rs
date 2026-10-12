//! Picture and video models: downloading their files, installing
//! stable-diffusion.cpp and running a generation, against a fake Hugging
//! Face, a fake GitHub release and a fake `sd-cli` script.

use axum::{extract::Path as UrlPath, routing::get, Json, Router};
use base64::Engine as _;
use brainwashed_core::{
    Backend, Engine, EngineConfig, FileRole, MediaEvent, MediaKind, MediaRequest,
};
use std::sync::{Arc, Mutex};

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// Every file is its own path's bytes, so sizes and contents are predictable.
fn bytes_of(path: &str) -> Vec<u8> {
    format!("weights:{path}").into_bytes()
}

/// A Hugging Face that has any file asked for, and records the downloads.
async fn fake_hf(fetched: Arc<Mutex<Vec<String>>>) -> String {
    let tree = |dir: String| {
        // Names the catalog uses, in whatever folder was asked for.
        let names = [
            "stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf",
            "z_image_turbo-Q4_K.gguf",
            "ae.safetensors",
            "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            "flux1-schnell-q4_k.gguf",
            "clip_l.safetensors",
            "t5-v1_1-xxl-encoder-Q4_K_M.gguf",
        ];
        let entries: Vec<_> = names
            .iter()
            .map(|n| {
                let path = if dir.is_empty() {
                    n.to_string()
                } else {
                    format!("{dir}/{n}")
                };
                serde_json::json!({"type": "file", "path": path, "size": bytes_of(&path).len()})
            })
            .collect();
        Json(entries)
    };
    let app =
        Router::new()
            .route(
                "/api/models/{owner}/{repo}/tree/main",
                get(move || async move { tree(String::new()) }),
            )
            .route(
                "/api/models/{owner}/{repo}/tree/main/{*dir}",
                get(
                    move |UrlPath((_, _, dir)): UrlPath<(String, String, String)>| async move {
                        tree(dir)
                    },
                ),
            )
            .route(
                "/{owner}/{repo}/resolve/main/{*path}",
                get(
                    move |UrlPath((owner, repo, path)): UrlPath<(String, String, String)>| {
                        let fetched = fetched.clone();
                        async move {
                            fetched
                                .lock()
                                .unwrap()
                                .push(format!("{owner}/{repo}/{path}"));
                            bytes_of(&path)
                        }
                    },
                ),
            );
    serve(app).await
}

fn engine(dir: &std::path::Path, hf: &str, releases: &str) -> Engine {
    let mut cfg = EngineConfig::new(dir, "0.1.0");
    cfg.hf_base = hf.to_string();
    cfg.sd_release_url = releases.to_string();
    let engine = Engine::new(cfg).unwrap();
    let mut s = engine.settings();
    s.backend = Backend::Cpu;
    engine.update_settings(s).unwrap();
    engine
}

#[tokio::test]
async fn downloads_shares_and_deletes_model_files() {
    let dir = tempfile::tempdir().unwrap();
    let fetched = Arc::new(Mutex::new(Vec::new()));
    let hf = fake_hf(fetched.clone()).await;
    let engine = engine(dir.path(), &hf, "http://127.0.0.1:9/none");
    let mut events = engine.subscribe();

    let z = engine.download_media_model("z-image-turbo").await.unwrap();
    assert_eq!(z.kind, MediaKind::Image);
    assert_eq!(z.files.len(), 3);
    let vae = z
        .files
        .iter()
        .find(|(r, _)| *r == FileRole::Vae)
        .unwrap()
        .1
        .clone();
    assert_eq!(
        std::fs::read(&vae).unwrap(),
        bytes_of("split_files/vae/ae.safetensors")
    );
    let mut finished = false;
    while let Ok(e) = events.try_recv() {
        if let brainwashed_core::Event::MediaDownloadFinished { repo, .. } = e {
            assert_eq!(repo, "z-image-turbo");
            finished = true;
        }
    }
    assert!(finished);

    // FLUX uses the same VAE: it isn't downloaded again.
    let flux = engine.download_media_model("flux1-schnell").await.unwrap();
    assert!(flux.files.iter().any(|(_, p)| *p == vae));
    let vae_downloads = fetched
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f.ends_with("/ae.safetensors"))
        .count();
    assert_eq!(vae_downloads, 1);
    assert_eq!(engine.media_models().len(), 2);
    assert!(engine
        .media_catalog()
        .iter()
        .any(|c| c.recipe.id == "flux1-schnell" && c.installed));

    // Deleting one keeps what the other still uses.
    engine.delete_media_model("z-image-turbo").await.unwrap();
    assert!(vae.exists());
    let z_main = &z.files[0].1;
    assert!(!z_main.exists());
    engine.delete_media_model("flux1-schnell").await.unwrap();
    assert!(!vae.exists());
    assert!(engine.media_models().is_empty());

    assert!(engine.download_media_model("no-such-model").await.is_err());
}

#[cfg(unix)]
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0, 2, 0, 0, 0,
    1, 0,
];

/// A stand-in for sd-cli: records its arguments, prints progress bars the way
/// the real one does, and writes the picture.
#[cfg(unix)]
const FAKE_SD: &str = r#"#!/bin/sh
here="$(cd "$(dirname "$0")" && pwd)"
printf '%s\n' "$@" > "$here/args.txt"
out=""
while [ $# -gt 0 ]; do
  case "$1" in --output) out="$2"; shift;; esac
  shift
done
printf '[INFO ] loading tensors\n'
printf '\r|==>  | 300/600 - 50.00it/s'
printf '\r|=>   | 1/3 - 1.00s/it\033[K'
printf '\r|===> | 2/3 - 1.00s/it\033[K'
printf '\r|=====| 3/3 - 1.00s/it\033[K\n'
printf '[INFO ] decoding 1 latents\n' >&2
cp "$here/fixture.png" "$out"
"#;

#[cfg(unix)]
fn write_fake_sd(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join("sd-cli");
    std::fs::write(&bin, FAKE_SD).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("fixture.png"), PNG).unwrap();
    bin
}

#[cfg(unix)]
#[tokio::test]
async fn makes_a_picture() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(dir.path(), "http://127.0.0.1:9", "http://127.0.0.1:9/none");
    let bin_dir = dir.path().join("bin");
    let bin = write_fake_sd(&bin_dir);
    let mut s = engine.settings();
    s.sd_cli_path = Some(bin);
    engine.update_settings(s).unwrap();

    let checkpoint = dir.path().join("dreamshaper_xl.safetensors");
    std::fs::write(&checkpoint, b"weights").unwrap();
    let model = engine.import_media_model(&checkpoint).unwrap();
    assert_eq!(model.defaults.width, 1024, "SDXL files default to 1024");
    assert!(engine
        .import_media_model(&dir.path().join("notes.txt"))
        .is_err());

    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let out = engine
        .generate(
            &MediaRequest {
                model: model.id.clone(),
                prompt: "a lighthouse at dusk".into(),
                width: Some(500),
                steps: Some(3),
                seed: Some(42),
                ..Default::default()
            },
            move |e| seen.lock().unwrap().push(e),
        )
        .await
        .unwrap();
    assert_eq!(out.mime, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&out.data)
            .unwrap(),
        PNG
    );
    // The size comes from the file itself.
    assert_eq!((out.width, out.height), (512, 256));
    assert_eq!(out.seed, 42);

    let args = std::fs::read_to_string(bin_dir.join("args.txt")).unwrap();
    let args: Vec<&str> = args.lines().collect();
    let after = |flag: &str| args[args.iter().position(|a| *a == flag).unwrap() + 1];
    assert_eq!(after("--model"), checkpoint.to_str().unwrap());
    assert_eq!(after("--prompt"), "a lighthouse at dusk");
    // Rounded to a multiple of 16.
    assert_eq!(after("--width"), "496");
    assert_eq!(after("--height"), "1024");
    assert_eq!(after("--seed"), "42");
    assert!(!args.contains(&"vid_gen"));

    let progress: Vec<(u32, u32)> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            MediaEvent::Progress(p) => Some((p.step, p.steps)),
            _ => None,
        })
        .collect();
    // Loading (0/3), then each step; the loading bar of 600 isn't a step.
    assert_eq!(progress, vec![(0, 3), (1, 3), (2, 3), (3, 3)]);

    // Nothing is left behind in the work folder.
    let work = dir.path().join("media").join("work");
    assert_eq!(std::fs::read_dir(work).unwrap().count(), 0);

    let empty = engine
        .generate(
            &MediaRequest {
                model: model.id.clone(),
                prompt: "  ".into(),
                ..Default::default()
            },
            |_| {},
        )
        .await;
    assert!(empty.is_err());
}

#[cfg(all(unix, target_arch = "x86_64", target_os = "linux"))]
#[tokio::test]
async fn installs_stable_diffusion_cpp_once() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let mut zip_bytes = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
        let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        zip.start_file("sd-cli", opts).unwrap();
        zip.write_all(FAKE_SD.as_bytes()).unwrap();
        zip.start_file("fixture.png", opts).unwrap();
        zip.write_all(PNG).unwrap();
        zip.finish().unwrap();
    }
    let downloads = Arc::new(Mutex::new(0));
    let counted = downloads.clone();
    let files = Router::new().route(
        "/sd.zip",
        get(move || {
            *counted.lock().unwrap() += 1;
            let body = zip_bytes.clone();
            async move { body }
        }),
    );
    let base = serve(files).await;
    let release = serde_json::json!([{
        "tag_name": "master-1-abc",
        "assets": [
            {"name": "sd-master-abc-bin-Linux-Ubuntu-24.04-x86_64-vulkan.zip", "browser_download_url": "http://127.0.0.1:9/no"},
            {"name": "sd-master-abc-bin-Linux-Ubuntu-24.04-x86_64.zip", "browser_download_url": format!("{base}/sd.zip")}
        ]
    }]);
    let gh =
        serve(Router::new().route("/releases", get(move || async move { Json(release) }))).await;
    let engine = engine(dir.path(), "http://127.0.0.1:9", &format!("{gh}/releases"));

    let mut events = Vec::new();
    let bin = engine
        .ensure_sd_runtime(&mut |e| events.push(e))
        .await
        .unwrap();
    assert!(bin.ends_with("sd-cli"));
    assert!(matches!(events[0], MediaEvent::Installing { .. }));
    // The second time it's already there.
    let again = engine
        .ensure_sd_runtime(&mut |_| panic!("no events"))
        .await
        .unwrap();
    assert_eq!(bin, again);
    assert_eq!(*downloads.lock().unwrap(), 1);
}
