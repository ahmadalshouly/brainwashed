use crate::settings::Settings;
use crate::skills::{ChatEvent, SkillState};
use crate::store::{self, InstalledModel, InstalledRuntime};
use crate::{Error, Result};
use brainwashed_runtime::{
    catalog::{self, CatalogEntry},
    chat::{self, SamplingOptions},
    download,
    llama::{self, LlamaServer, ServerOptions},
    release, Attachment, ChatMessage, Hardware, Role,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

/// Where the engine keeps its files and which services it downloads from.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub data_dir: PathBuf,
    pub app_version: String,
    pub github_release_url: String,
    pub hf_base: String,
    /// How long a model may take to load before we give up.
    pub load_timeout: Duration,
}

impl EngineConfig {
    pub fn new(data_dir: impl Into<PathBuf>, app_version: impl Into<String>) -> Self {
        EngineConfig {
            data_dir: data_dir.into(),
            app_version: app_version.into(),
            github_release_url: release::RELEASES_URL.to_string(),
            hf_base: catalog::HF_BASE.to_string(),
            load_timeout: Duration::from_secs(300),
        }
    }
}

/// Mirrors `HostInfo` in `packages/api/src/types.ts`.
#[derive(Debug, Clone, Serialize)]
pub struct HostInfo {
    pub name: String,
    pub version: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum EngineState {
    /// No model loaded.
    Idle,
    /// Downloading the llama.cpp runtime.
    #[serde(rename_all = "camelCase")]
    InstallingRuntime {
        done: u64,
        total: Option<u64>,
    },
    Loading {
        model: String,
    },
    Ready {
        model: String,
    },
    Error {
        message: String,
    },
}

/// Things the UI and paired phones may want to show live.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    State(EngineState),
    #[serde(rename_all = "camelCase")]
    DownloadProgress {
        repo: String,
        done: u64,
        total: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    DownloadFinished {
        repo: String,
        model: InstalledModel,
    },
    #[serde(rename_all = "camelCase")]
    DownloadFailed {
        repo: String,
        error: String,
    },
    ModelsChanged,
    SkillsChanged,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogItem {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub installed: bool,
    /// Whether it should run comfortably on this computer.
    pub fits: bool,
}

#[derive(Clone)]
pub struct Engine {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    config: EngineConfig,
    pub(crate) client: reqwest::Client,
    /// Talks to llama-server on localhost, bypassing any system proxy.
    local: reqwest::Client,
    hardware: Hardware,
    settings: RwLock<Settings>,
    models: RwLock<Vec<InstalledModel>>,
    state: RwLock<EngineState>,
    server: Mutex<Option<LlamaServer>>,
    base_url: RwLock<Option<String>>,
    /// Context size of the loaded model as llama-server reports it.
    pub(crate) loaded_context: RwLock<Option<u32>>,
    /// The loaded model can look at pictures.
    vision: RwLock<bool>,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) skills: RwLock<SkillState>,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.data_dir)?;
        let client = reqwest::Client::builder()
            .user_agent(format!("BrainWashed/{}", config.app_version))
            .connect_timeout(Duration::from_secs(20))
            .build()
            .map_err(brainwashed_runtime::Error::from)?;
        let local = reqwest::Client::builder()
            .no_proxy()
            .build()
            .map_err(brainwashed_runtime::Error::from)?;
        let settings: Settings = store::load(&config.data_dir.join("settings.json"))?;
        let models: Vec<InstalledModel> = store::load(&config.data_dir.join("models.json"))?;
        kill_stale_server(&config.data_dir);
        let skills = SkillState::open(&config.data_dir.join("skills"))?;
        Ok(Engine {
            inner: Arc::new(Inner {
                client,
                local,
                hardware: Hardware::detect(),
                settings: RwLock::new(settings),
                models: RwLock::new(models),
                state: RwLock::new(EngineState::Idle),
                server: Mutex::new(None),
                base_url: RwLock::new(None),
                loaded_context: RwLock::new(None),
                vision: RwLock::new(false),
                events: broadcast::channel(256).0,
                skills: RwLock::new(skills),
                config,
            }),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub(crate) fn emit(&self, event: Event) {
        // No subscribers is fine.
        let _ = self.inner.events.send(event);
    }

    fn set_state(&self, state: EngineState) {
        *self.inner.state.write().unwrap() = state.clone();
        self.emit(Event::State(state));
    }

    pub fn state(&self) -> EngineState {
        self.inner.state.read().unwrap().clone()
    }

    pub fn hardware(&self) -> &Hardware {
        &self.inner.hardware
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.config.data_dir
    }

    pub fn info(&self) -> HostInfo {
        let model = match self.state() {
            EngineState::Ready { model } => Some(model),
            _ => None,
        };
        HostInfo {
            name: self.host_name(),
            version: self.inner.config.app_version.clone(),
            model,
        }
    }

    /// A newer release than this one, if GitHub lists one. Returns None when
    /// the user turned update checks off or GitHub can't be reached.
    pub async fn check_for_update(&self) -> Option<crate::UpdateInfo> {
        if !self.settings().check_for_updates {
            return None;
        }
        let res = self
            .inner
            .client
            .get(crate::updates::RELEASES_URL)
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        let body = res.text().await.ok()?;
        crate::updates::newer_release(&self.inner.config.app_version, &body)
    }

    pub fn host_name(&self) -> String {
        self.settings()
            .host_name
            .filter(|n| !n.trim().is_empty())
            .or_else(sysinfo::System::host_name)
            .unwrap_or_else(|| "BrainWashed host".into())
    }

    // ----- settings -----

    pub fn settings(&self) -> Settings {
        self.inner.settings.read().unwrap().clone()
    }

    pub fn update_settings(&self, settings: Settings) -> Result<()> {
        settings
            .chat_defaults
            .validate()
            .map_err(|e| Error::Invalid(format!("chat defaults: {e}")))?;
        let path = self.inner.config.data_dir.join("settings.json");
        store::save(&path, &settings)?;
        // It can hold a tunnel token.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        *self.inner.settings.write().unwrap() = settings;
        Ok(())
    }

    fn edit_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut s = self.settings();
        f(&mut s);
        self.update_settings(s)
    }

    // ----- models -----

    fn models_dir(&self) -> PathBuf {
        self.inner.config.data_dir.join("models")
    }

    pub fn models(&self) -> Vec<InstalledModel> {
        self.inner.models.read().unwrap().clone()
    }

    fn save_models(&self, models: Vec<InstalledModel>) -> Result<()> {
        store::save(&self.inner.config.data_dir.join("models.json"), &models)?;
        *self.inner.models.write().unwrap() = models;
        self.emit(Event::ModelsChanged);
        Ok(())
    }

    pub fn catalog(&self) -> Vec<CatalogItem> {
        let models = self.models();
        let max = self.inner.hardware.recommended_max_params_b() as f32;
        catalog::curated()
            .into_iter()
            .map(|entry| CatalogItem {
                installed: models
                    .iter()
                    .any(|m| m.repo.as_deref() == Some(&entry.repo)),
                fits: entry.params_b <= max + 0.5,
                entry,
            })
            .collect()
    }

    /// Downloads the best GGUF file from a Hugging Face repo and registers it.
    pub async fn download_model(&self, repo: &str, quant: Option<&str>) -> Result<InstalledModel> {
        let result = self.download_model_inner(repo, quant).await;
        match &result {
            Ok(model) => self.emit(Event::DownloadFinished {
                repo: repo.to_string(),
                model: model.clone(),
            }),
            Err(e) => self.emit(Event::DownloadFailed {
                repo: repo.to_string(),
                error: e.to_string(),
            }),
        }
        result
    }

    async fn download_model_inner(
        &self,
        repo: &str,
        quant: Option<&str>,
    ) -> Result<InstalledModel> {
        let repo = repo.trim().trim_matches('/');
        if repo.split('/').count() != 2 || repo.contains("..") {
            return Err(Error::Invalid(format!(
                "`{repo}` is not a Hugging Face repo name like `owner/model-GGUF`"
            )));
        }
        let cfg = &self.inner.config;
        let files = catalog::resolve(&self.inner.client, &cfg.hf_base, repo, quant).await?;
        let dir = store::dir_for_repo(&self.models_dir(), repo);
        let file_name = file_name(&files.model.file)?;
        let dest = dir.join(&file_name);
        let total = files.model.size + files.projector.as_ref().map_or(0, |p| p.size);

        // Progress covers the model and its projector together.
        let fetch = |file: catalog::ModelFile, dest: PathBuf, offset: u64| {
            let events = self.inner.events.clone();
            let repo = repo.to_string();
            async move {
                // Downloading again only fetches what's missing, such as the
                // projector of a model installed before pictures worked.
                let have = tokio::fs::metadata(&dest).await.map(|m| m.len()).ok();
                if have == Some(file.size) && file.size > 0 {
                    return Ok::<_, Error>(());
                }
                download::download(
                    &self.inner.client,
                    &file.url(&cfg.hf_base),
                    &dest,
                    file.sha256.as_deref(),
                    &move |done, _| {
                        let _ = events.send(Event::DownloadProgress {
                            repo: repo.clone(),
                            done: offset + done,
                            total: Some(total),
                        });
                    },
                )
                .await?;
                Ok(())
            }
        };
        let model_size = files.model.size;
        fetch(files.model.clone(), dest.clone(), 0).await?;
        let mmproj = match files.projector {
            Some(projector) => {
                let path = dir.join(file_name_of(&projector)?);
                fetch(projector, path.clone(), model_size).await?;
                Some(path)
            }
            None => None,
        };

        let model = InstalledModel {
            id: store::model_id(&file_name),
            name: file_name.trim_end_matches(".gguf").to_string(),
            repo: Some(repo.to_string()),
            size: std::fs::metadata(&dest)?.len(),
            path: dest,
            mmproj,
        };
        let mut models = self.models();
        models.retain(|m| m.id != model.id);
        models.push(model.clone());
        self.save_models(models)?;
        Ok(model)
    }

    /// Registers a GGUF file already on disk without copying it.
    pub fn import_model(&self, path: &Path) -> Result<InstalledModel> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| n.to_ascii_lowercase().ends_with(".gguf"))
            .ok_or_else(|| Error::Invalid("pick a .gguf model file".into()))?;
        let model = InstalledModel {
            id: store::model_id(name),
            name: name.trim_end_matches(".gguf").to_string(),
            repo: None,
            size: std::fs::metadata(path)?.len(),
            path: path.to_path_buf(),
            mmproj: None,
        };
        let mut models = self.models();
        models.retain(|m| m.id != model.id);
        models.push(model.clone());
        self.save_models(models)?;
        Ok(model)
    }

    /// Forgets a model, deleting its file if BrainWashed downloaded it.
    pub async fn delete_model(&self, id: &str) -> Result<()> {
        if matches!(self.state(), EngineState::Ready { model } | EngineState::Loading { model } if model == id)
        {
            self.unload().await?;
        }
        let mut models = self.models();
        if let Some(pos) = models.iter().position(|m| m.id == id) {
            let model = models.remove(pos);
            if model.path.starts_with(self.models_dir()) {
                let _ = tokio::fs::remove_file(&model.path).await;
                if let Some(mmproj) = &model.mmproj {
                    // Other quantizations of the same repo share the projector.
                    let shared = models.iter().any(|m| m.mmproj.as_ref() == Some(mmproj));
                    if !shared {
                        let _ = tokio::fs::remove_file(mmproj).await;
                    }
                }
            }
            self.save_models(models)?;
        }
        Ok(())
    }

    // ----- runtime -----

    fn runtime_dir(&self) -> PathBuf {
        self.inner.config.data_dir.join("runtime")
    }

    /// Returns a llama-server binary, downloading llama.cpp the first time.
    pub async fn ensure_runtime(&self) -> Result<PathBuf> {
        let settings = self.settings();
        if let Some(path) = settings.llama_server_path {
            return Ok(path);
        }
        let hw = &self.inner.hardware;
        let platform = release::platform_token(hw, settings.backend)?;
        let manifest = self.runtime_dir().join("runtime.json");
        let installed: Option<InstalledRuntime> = store::load(&manifest)?;
        if let Some(rt) = installed.filter(|rt| rt.platform == platform && rt.binary.exists()) {
            return Ok(rt.binary);
        }

        self.set_state(EngineState::InstallingRuntime {
            done: 0,
            total: None,
        });
        let cfg = &self.inner.config;
        let releases = release::fetch_releases(&self.inner.client, &cfg.github_release_url).await?;
        let (rel, asset) = release::select_newest(&releases, hw, settings.backend)?;
        let archive = self.runtime_dir().join("downloads").join(&asset.name);
        let engine = self.clone();
        download::download(
            &self.inner.client,
            &asset.browser_download_url,
            &archive,
            None,
            &move |done, total| {
                engine.set_state(EngineState::InstallingRuntime { done, total });
            },
        )
        .await?;

        let target = self
            .runtime_dir()
            .join(format!("{}-{platform}", rel.tag_name));
        let _ = tokio::fs::remove_dir_all(&target).await;
        download::extract(&archive, &target).await?;
        let _ = tokio::fs::remove_file(&archive).await;
        let binary = download::find_file(&target, llama::SERVER_BINARY).ok_or_else(|| {
            Error::Invalid(format!(
                "{} does not contain {}",
                asset.name,
                llama::SERVER_BINARY
            ))
        })?;
        make_executable(&binary)?;

        store::save(
            &manifest,
            &InstalledRuntime {
                tag: rel.tag_name.clone(),
                platform: platform.to_string(),
                binary: binary.clone(),
            },
        )?;
        Ok(binary)
    }

    /// Loads a model, replacing any model already loaded.
    pub async fn load_model(&self, id: &str) -> Result<()> {
        let model = self
            .models()
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| Error::Invalid(format!("no installed model `{id}`")))?;

        let mut server = self.inner.server.lock().await;
        if let Some(old) = server.take() {
            *self.inner.base_url.write().unwrap() = None;
            *self.inner.vision.write().unwrap() = false;
            let _ = old.stop().await;
        }

        let result = async {
            let binary = self.ensure_runtime().await?;
            self.set_state(EngineState::Loading {
                model: model.id.clone(),
            });
            let settings = self.settings();
            let opts = ServerOptions {
                binary,
                model: model.path.clone(),
                mmproj: model.mmproj.clone().filter(|p| p.exists()),
                context_size: settings.context_size,
                gpu_layers: settings.gpu_layers,
                port: None,
            };
            Ok::<_, Error>(
                LlamaServer::start(&opts, &self.inner.local, self.inner.config.load_timeout)
                    .await?,
            )
        }
        .await;

        match result {
            Ok(started) => {
                if let Some(pid) = started.pid() {
                    let _ = std::fs::write(pid_file(self.data_dir()), pid.to_string());
                }
                *self.inner.base_url.write().unwrap() = Some(started.base_url());
                *self.inner.loaded_context.write().unwrap() = started.context_size();
                *self.inner.vision.write().unwrap() =
                    model.mmproj.as_ref().is_some_and(|p| p.exists());
                *server = Some(started);
                self.edit_settings(|s| s.active_model = Some(model.id.clone()))?;
                self.set_state(EngineState::Ready { model: model.id });
                Ok(())
            }
            Err(e) => {
                self.set_state(EngineState::Error {
                    message: e.to_string(),
                });
                Err(e)
            }
        }
    }

    pub async fn unload(&self) -> Result<()> {
        let mut server = self.inner.server.lock().await;
        *self.inner.base_url.write().unwrap() = None;
        *self.inner.vision.write().unwrap() = false;
        if let Some(old) = server.take() {
            old.stop().await.map_err(Error::from)?;
        }
        let _ = std::fs::remove_file(pid_file(self.data_dir()));
        self.set_state(EngineState::Idle);
        Ok(())
    }

    /// Loads the model that was active last time, if it is still installed.
    pub async fn restore(&self) -> Result<()> {
        if let Some(id) = self.settings().active_model {
            if self.models().iter().any(|m| m.id == id) {
                return self.load_model(&id).await;
            }
        }
        Ok(())
    }

    // ----- chat -----

    /// Streams the local model's reply to `conversation`. The first event
    /// names the skills used; the rest are pieces of the answer, then stats.
    /// Returns the full answer.
    pub async fn chat(
        &self,
        conversation: &[ChatMessage],
        sampling: &SamplingOptions,
        on_event: impl FnMut(ChatEvent),
    ) -> Result<String> {
        self.chat_with(conversation, sampling, None, true, on_event)
            .await
    }

    /// Like [`Engine::chat`], with the model picked by id: `local` (or None)
    /// for the model on this computer, `<provider>/<model>` for a cloud
    /// provider. `admin` decides which provider models are allowed.
    pub async fn chat_with(
        &self,
        conversation: &[ChatMessage],
        sampling: &SamplingOptions,
        model: Option<&str>,
        admin: bool,
        mut on_event: impl FnMut(ChatEvent),
    ) -> Result<String> {
        // The chat's own settings win; the admin's defaults fill the rest.
        let sampling = &sampling.clone().or(&self.settings().chat_defaults);
        sampling
            .validate()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        check_attachments(conversation)?;
        let (endpoint, vision, context) =
            match model.filter(|m| *m != crate::providers::LOCAL_MODEL) {
                Some(id) => (
                    self.provider_endpoint(id, admin)?,
                    true,
                    Some(CLOUD_CONTEXT),
                ),
                None => {
                    let base_url = self
                        .inner
                        .base_url
                        .read()
                        .unwrap()
                        .clone()
                        .ok_or_else(|| Error::Invalid("no model is loaded yet".into()))?;
                    (chat::Endpoint::llama(base_url), self.vision(), None)
                }
            };
        let asks_about_pictures = conversation
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .is_some_and(|m| m.images().next().is_some());
        if asks_about_pictures && !vision {
            return Err(Error::Invalid(
                "This model can't see pictures. Switch to a model that can, like Gemma 3 4B or Qwen2.5 VL 3B, or ask an admin to download one.".into(),
            ));
        }
        let (prompt, skills) = self.build_prompt_with(conversation, context);
        on_event(ChatEvent::Skills { names: skills });
        let client = if endpoint.llama {
            &self.inner.local
        } else {
            &self.inner.client
        };
        Ok(
            chat::stream_chat(client, &endpoint, &prompt, sampling, vision, |d| {
                on_event(d.into())
            })
            .await?,
        )
    }

    /// Whether the loaded model can look at pictures.
    pub fn vision(&self) -> bool {
        *self.inner.vision.read().unwrap()
    }
}

/// Context assumed for cloud models when trimming long conversations. Most
/// offer far more; this keeps requests and bills reasonable.
const CLOUD_CONTEXT: u32 = 64_000;

const IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];

/// Pictures must be base64 in a format llama.cpp reads, since they go into a
/// data URL.
fn check_attachments(conversation: &[ChatMessage]) -> Result<()> {
    for m in conversation {
        for a in &m.attachments {
            if let Attachment::Image { name, mime, data } = a {
                if !IMAGE_TYPES.contains(&mime.as_str()) {
                    return Err(Error::Invalid(format!(
                        "{name} is a {mime}. Attach PNG, JPEG, WebP or GIF pictures."
                    )));
                }
                let base64 = data
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='));
                if !base64 || data.is_empty() {
                    return Err(Error::Invalid(format!("{name} is not a valid picture.")));
                }
            }
        }
    }
    Ok(())
}

fn file_name(path: &str) -> Result<String> {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| Error::Invalid(format!("bad file name {path}")))
}

fn file_name_of(file: &catalog::ModelFile) -> Result<String> {
    file_name(&file.file)
}

fn pid_file(data_dir: &Path) -> PathBuf {
    data_dir.join("llama-server.pid")
}

/// If the app crashed or was force-quit, its llama-server may still be running
/// and holding memory. Stop it before starting a new one.
fn kill_stale_server(data_dir: &Path) {
    let Some(pid) = std::fs::read_to_string(pid_file(data_dir))
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
    else {
        return;
    };
    let pid = sysinfo::Pid::from(pid);
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    if let Some(proc) = sys.process(pid) {
        if proc.name().to_string_lossy().starts_with("llama-server") {
            tracing::info!("stopping llama-server left over from a previous run (pid {pid})");
            proc.kill();
        }
    }
    let _ = std::fs::remove_file(pid_file(data_dir));
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}
