//! Picture and video models, run with stable-diffusion.cpp.
//!
//! Unlike chat models, these are usually several files (a diffusion model,
//! text encoders and a VAE), so the catalog lists each model's files and
//! the settings it works best with. Files shared between models, like the
//! FLUX VAE, are downloaded once.

use crate::engine::{make_executable, Engine, Event};
use crate::store;
use crate::{Error, Result};
use base64::Engine as _;
use brainwashed_runtime::{
    catalog::ModelFile,
    download, release,
    sd::{self, FileRole, GenerateOptions, MediaKind, Progress},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Instant;

/// A file of a catalog model. Mirrors `MediaFile` in packages/api.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeFile {
    pub role: FileRole,
    pub repo: String,
    pub path: String,
    /// Approximate, for showing; the real size comes from Hugging Face.
    #[serde(default)]
    pub size: u64,
}

/// The settings a model works best with. Mirrors `MediaDefaults` in packages/api.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDefaults {
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg_scale: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampler: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_shift: Option<f32>,
    /// Videos only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
}

/// A picture or video model in the catalog. Mirrors `MediaCatalogItem` in packages/api.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRecipe {
    pub id: String,
    pub name: String,
    pub kind: MediaKind,
    pub description: String,
    pub license: String,
    /// Memory (RAM, or RAM plus graphics memory) it needs to run well.
    pub min_memory_gb: u32,
    /// Videos: it can start from an attached picture.
    #[serde(default)]
    pub picture_input: bool,
    pub files: Vec<RecipeFile>,
    pub defaults: MediaDefaults,
    /// Extra sd-cli flags, e.g. `--diffusion-fa`.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Models offered on the Models page.
pub fn media_catalog() -> Vec<MediaRecipe> {
    serde_json::from_str(include_str!("media_catalog.json")).expect("media_catalog.json is valid")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCatalogItem {
    #[serde(flatten)]
    pub recipe: MediaRecipe,
    /// All files together, in bytes.
    pub size: u64,
    pub installed: bool,
    pub fits: bool,
}

/// An installed picture or video model. Mirrors `MediaModel` in packages/api.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaModel {
    pub id: String,
    pub name: String,
    pub kind: MediaKind,
    /// Files and the part each plays.
    pub files: Vec<(FileRole, PathBuf)>,
    /// All files together, in bytes.
    pub size: u64,
    pub defaults: MediaDefaults,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub picture_input: bool,
    /// The catalog entry it came from; None for imported files.
    #[serde(default)]
    pub recipe: Option<String>,
}

/// What to make. Mirrors `MediaRequest` in packages/api.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaRequest {
    pub model: String,
    pub prompt: String,
    pub negative_prompt: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub steps: Option<u32>,
    pub cfg_scale: Option<f32>,
    /// Left out picks one at random; the result says which.
    pub seed: Option<i64>,
    /// Videos only.
    pub frames: Option<u32>,
    pub fps: Option<u32>,
    /// A picture to start from, base64. Pictures turn it into a new one;
    /// video models that take pictures set it in motion.
    pub image: Option<String>,
    /// How much of the starting picture to change, 0 to 1.
    pub strength: Option<f32>,
}

/// A finished picture or video. Mirrors `GeneratedMedia` in packages/api.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedMedia {
    pub kind: MediaKind,
    /// `image/png`, or `image/webp` (animated) for videos.
    pub mime: String,
    /// Base64.
    pub data: String,
    pub width: u32,
    pub height: u32,
    pub seed: i64,
    pub model: String,
    pub model_name: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
    pub seconds: f64,
}

/// What streams back while something is made. Mirrors `MediaEvent` in packages/api.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MediaEvent {
    /// Waiting for another picture or video to finish first.
    Queued,
    /// Downloading stable-diffusion.cpp, the first time.
    #[serde(rename_all = "camelCase")]
    Installing {
        done: u64,
        total: Option<u64>,
    },
    Progress(Progress),
}

#[derive(Default)]
pub(crate) struct MediaState {
    models: RwLock<Vec<MediaModel>>,
    /// One generation at a time: each uses most of the memory.
    busy: tokio::sync::Mutex<()>,
    install: tokio::sync::Mutex<()>,
}

impl MediaState {
    pub(crate) fn open(data_dir: &Path) -> Result<Self> {
        Ok(MediaState {
            models: RwLock::new(store::load(&data_dir.join("media.json"))?),
            ..Default::default()
        })
    }
}

/// The stable-diffusion.cpp build currently installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledSd {
    tag: String,
    platform: String,
    binary: PathBuf,
}

pub const MAX_PROMPT_CHARS: usize = 4000;
/// Largest starting picture, in base64 characters (about 15 MB).
const MAX_IMAGE_B64: usize = 20 * 1024 * 1024;

impl Engine {
    fn media_path(&self) -> PathBuf {
        self.data_dir().join("media.json")
    }

    pub fn media_models(&self) -> Vec<MediaModel> {
        self.inner.media.models.read().unwrap().clone()
    }

    fn save_media(&self, models: Vec<MediaModel>) -> Result<()> {
        store::save(&self.media_path(), &models)?;
        *self.inner.media.models.write().unwrap() = models;
        self.emit(Event::ModelsChanged);
        Ok(())
    }

    pub fn media_catalog(&self) -> Vec<MediaCatalogItem> {
        let installed = self.media_models();
        let gb = self.hardware().total_memory_bytes / (1024 * 1024 * 1024);
        media_catalog()
            .into_iter()
            .map(|recipe| MediaCatalogItem {
                size: unique_size(&recipe.files),
                installed: installed
                    .iter()
                    .any(|m| m.recipe.as_deref() == Some(&recipe.id)),
                fits: gb + 1 >= recipe.min_memory_gb as u64,
                recipe,
            })
            .collect()
    }

    /// Downloads a catalog model's files and registers it. Progress goes out
    /// as `DownloadProgress` events named after the model's id.
    pub async fn download_media_model(&self, id: &str) -> Result<MediaModel> {
        let result = self.download_media_inner(id).await;
        match &result {
            Ok(model) => self.emit(Event::MediaDownloadFinished {
                repo: id.to_string(),
                model: model.clone(),
            }),
            Err(e) => self.emit(Event::DownloadFailed {
                repo: id.to_string(),
                error: e.to_string(),
            }),
        }
        result
    }

    async fn download_media_inner(&self, id: &str) -> Result<MediaModel> {
        let recipe = media_catalog()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| Error::Invalid(format!("no picture or video model `{id}`")))?;
        let hf = self.hf_base().to_string();
        // Real sizes and checksums, from Hugging Face.
        let mut files = Vec::new();
        for f in &recipe.files {
            files.push((f.role, self.resolve_hf_file(&f.repo, &f.path).await?));
        }
        let total: u64 = {
            let mut seen = std::collections::HashSet::new();
            files
                .iter()
                .filter(|(_, f)| seen.insert((f.repo.clone(), f.file.clone())))
                .map(|(_, f)| f.size)
                .sum()
        };
        let mut offset = 0;
        let mut seen = std::collections::HashSet::new();
        let mut paths = Vec::new();
        for (role, file) in files {
            let dest = self.media_file_path(&file)?;
            if seen.insert(dest.clone()) {
                let have = tokio::fs::metadata(&dest).await.map(|m| m.len()).ok();
                if have != Some(file.size) || file.size == 0 {
                    let events = self.inner.events.clone();
                    let repo = id.to_string();
                    let base = offset;
                    download::download(
                        &self.inner.client,
                        &file.url(&hf),
                        &dest,
                        file.sha256.as_deref(),
                        &move |done, _| {
                            let _ = events.send(Event::DownloadProgress {
                                repo: repo.clone(),
                                done: base + done,
                                total: Some(total),
                            });
                        },
                    )
                    .await?;
                }
                offset += file.size;
            }
            paths.push((role, dest));
        }
        let model = MediaModel {
            id: recipe.id.clone(),
            name: recipe.name.clone(),
            kind: recipe.kind,
            size: total,
            files: paths,
            defaults: recipe.defaults.clone(),
            args: recipe.args.clone(),
            picture_input: recipe.picture_input,
            recipe: Some(recipe.id.clone()),
        };
        let mut models = self.media_models();
        models.retain(|m| m.id != model.id);
        models.push(model.clone());
        self.save_media(models)?;
        Ok(model)
    }

    /// Where a model file goes: one folder per repo, keeping the file's name.
    fn media_file_path(&self, file: &ModelFile) -> Result<PathBuf> {
        let name = Path::new(&file.file)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.starts_with('.'))
            .ok_or_else(|| Error::Invalid(format!("bad file name {}", file.file)))?;
        Ok(store::dir_for_repo(&self.data_dir().join("models"), &file.repo).join(name))
    }

    /// Looks a file up on Hugging Face for its size and SHA-256.
    async fn resolve_hf_file(&self, repo: &str, path: &str) -> Result<ModelFile> {
        #[derive(Deserialize)]
        struct Entry {
            path: String,
            #[serde(default)]
            size: u64,
            lfs: Option<Lfs>,
        }
        #[derive(Deserialize)]
        struct Lfs {
            oid: String,
            size: u64,
        }
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
        let url = format!("{}/api/models/{repo}/tree/main/{dir}", self.hf_base());
        let entries: Vec<Entry> = self
            .inner
            .client
            .get(url.trim_end_matches('/'))
            .send()
            .await
            .map_err(brainwashed_runtime::Error::from)?
            .error_for_status()
            .map_err(brainwashed_runtime::Error::from)?
            .json()
            .await
            .map_err(brainwashed_runtime::Error::from)?;
        let e = entries
            .into_iter()
            .find(|e| e.path == path)
            .ok_or_else(|| Error::Invalid(format!("{repo} no longer has {path}")))?;
        Ok(ModelFile {
            repo: repo.to_string(),
            file: path.to_string(),
            size: e.lfs.as_ref().map_or(e.size, |l| l.size),
            sha256: e.lfs.map(|l| l.oid),
        })
    }

    /// Registers a single-file checkpoint already on disk (`.safetensors`,
    /// `.ckpt` or `.gguf`), such as an SD 1.5 or SDXL model.
    pub fn import_media_model(&self, path: &Path) -> Result<MediaModel> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| {
                let n = n.to_ascii_lowercase();
                [".safetensors", ".ckpt", ".gguf", ".sft"]
                    .iter()
                    .any(|e| n.ends_with(e))
            })
            .ok_or_else(|| Error::Invalid("pick a .safetensors, .ckpt or .gguf file".into()))?;
        let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
        let xl = stem.to_ascii_lowercase().contains("xl");
        let model = MediaModel {
            id: format!("file-{}", store::model_id(stem)),
            name: stem.to_string(),
            kind: MediaKind::Image,
            files: vec![(FileRole::Model, path.to_path_buf())],
            size: std::fs::metadata(path)?.len(),
            defaults: MediaDefaults {
                width: if xl { 1024 } else { 512 },
                height: if xl { 1024 } else { 512 },
                steps: 20,
                cfg_scale: 7.0,
                sampler: Some("euler_a".into()),
                negative_prompt: None,
                flow_shift: None,
                frames: None,
                fps: None,
            },
            args: Vec::new(),
            picture_input: false,
            recipe: None,
        };
        let mut models = self.media_models();
        models.retain(|m| m.id != model.id);
        models.push(model.clone());
        self.save_media(models)?;
        Ok(model)
    }

    /// Forgets a model, deleting the files BrainWashed downloaded for it that
    /// no other model uses.
    pub async fn delete_media_model(&self, id: &str) -> Result<()> {
        let mut models = self.media_models();
        let Some(pos) = models.iter().position(|m| m.id == id) else {
            return Ok(());
        };
        let model = models.remove(pos);
        let models_dir = self.data_dir().join("models");
        let chat_files: Vec<PathBuf> = self.models().into_iter().map(|m| m.path).collect();
        for (_, path) in &model.files {
            let shared = models
                .iter()
                .any(|m| m.files.iter().any(|(_, p)| p == path))
                || chat_files.contains(path);
            if path.starts_with(&models_dir) && !shared {
                let _ = tokio::fs::remove_file(path).await;
            }
        }
        self.save_media(models)
    }

    // ----- runtime -----

    /// Returns an sd-cli binary, downloading stable-diffusion.cpp the first time.
    pub async fn ensure_sd_runtime(
        &self,
        on_event: &mut impl FnMut(MediaEvent),
    ) -> Result<PathBuf> {
        let settings = self.settings();
        if let Some(path) = settings.sd_cli_path {
            return Ok(path);
        }
        let _installing = self.inner.media.install.lock().await;
        let hw = self.hardware();
        let platform = sd::platform_label(hw, settings.backend);
        let dir = self.data_dir().join("runtime");
        let manifest = dir.join("sd.json");
        let installed: Option<InstalledSd> = store::load(&manifest)?;
        if let Some(rt) = installed.filter(|rt| rt.platform == platform && rt.binary.exists()) {
            return Ok(rt.binary);
        }
        on_event(MediaEvent::Installing {
            done: 0,
            total: None,
        });
        let releases = release::fetch_releases(&self.inner.client, &self.sd_release_url()).await?;
        let (rel, asset) = sd::select_newest(&releases, hw, settings.backend)?;
        let archive = dir.join("downloads").join(&asset.name);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let report = move |done, total| {
            let _ = tx.send((done, total));
        };
        let fetch = download::download(
            &self.inner.client,
            &asset.browser_download_url,
            &archive,
            None,
            &report,
        );
        tokio::pin!(fetch);
        let mut last = 0u64;
        loop {
            tokio::select! {
                r = &mut fetch => { r?; break; }
                Some((done, total)) = rx.recv() => {
                    // About every megabyte is plenty.
                    if done - last.min(done) >= 1 << 20 || Some(done) == total {
                        last = done;
                        on_event(MediaEvent::Installing { done, total });
                    }
                }
            }
        }
        let target = dir.join(format!("sd-{}-{platform}", rel.tag_name));
        let _ = tokio::fs::remove_dir_all(&target).await;
        download::extract(&archive, &target).await?;
        let _ = tokio::fs::remove_file(&archive).await;
        let binary = sd::BINARIES
            .iter()
            .find_map(|b| download::find_file(&target, b))
            .ok_or_else(|| Error::Invalid(format!("{} does not contain sd-cli", asset.name)))?;
        make_executable(&binary)?;
        store::save(
            &manifest,
            &InstalledSd {
                tag: rel.tag_name.clone(),
                platform,
                binary: binary.clone(),
            },
        )?;
        Ok(binary)
    }

    // ----- making things -----

    /// Makes a picture or a video. Waits for any other generation to finish
    /// first. Dropping the future stops it.
    pub async fn generate(
        &self,
        req: &MediaRequest,
        mut on_event: impl FnMut(MediaEvent),
    ) -> Result<GeneratedMedia> {
        let model = self
            .media_models()
            .into_iter()
            .find(|m| m.id == req.model)
            .ok_or_else(|| Error::Invalid(format!("no picture or video model `{}`", req.model)))?;
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(Error::Invalid("Describe what to make.".into()));
        }
        if prompt.chars().count() > MAX_PROMPT_CHARS {
            return Err(Error::Invalid(format!(
                "Keep the description under {MAX_PROMPT_CHARS} characters."
            )));
        }
        if let Some((_, missing)) = model.files.iter().find(|(_, p)| !p.exists()) {
            return Err(Error::Invalid(format!(
                "{} is missing {}. Download it again on the Models page.",
                model.name,
                missing.display()
            )));
        }
        let d = &model.defaults;
        let video = model.kind == MediaKind::Video;
        let size = |v: Option<u32>, default: u32| (v.unwrap_or(default).clamp(64, 2048) / 16) * 16;
        let width = size(req.width, d.width);
        let height = size(req.height, d.height);
        let steps = req.steps.unwrap_or(d.steps).clamp(1, 100);
        let cfg_scale = req.cfg_scale.unwrap_or(d.cfg_scale).clamp(0.0, 30.0);
        let frames = video.then(|| {
            // Wan makes 4n+1 frames.
            let f = req.frames.or(d.frames).unwrap_or(33).clamp(1, 121);
            f - (f - 1) % 4
        });
        let fps = video.then(|| req.fps.or(d.fps).unwrap_or(16).clamp(1, 60));
        let strength = req.strength.map(|s| s.clamp(0.0, 1.0));
        let seed = req.seed.filter(|s| *s >= 0).unwrap_or_else(random_seed);
        if req.image.is_some() && video && !model.picture_input {
            return Err(Error::Invalid(format!(
                "{} makes videos from a description only. Remove the picture, or pick Wan 2.2 5B.",
                model.name
            )));
        }

        let _turn = match self.inner.media.busy.try_lock() {
            Ok(g) => g,
            Err(_) => {
                on_event(MediaEvent::Queued);
                self.inner.media.busy.lock().await
            }
        };
        let binary = self.ensure_sd_runtime(&mut on_event).await?;
        on_event(MediaEvent::Progress(Progress {
            stage: sd::Stage::Loading,
            step: 0,
            steps,
        }));

        let work = self.data_dir().join("media").join("work");
        tokio::fs::create_dir_all(&work).await?;
        let tag = format!("{}-{seed}", std::process::id());
        let output = work.join(format!("{tag}.{}", if video { "webp" } else { "png" }));
        let init_image = match &req.image {
            Some(b64) => {
                if b64.len() > MAX_IMAGE_B64 {
                    return Err(Error::Invalid("The picture is too big.".into()));
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64.trim())
                    .map_err(|_| Error::Invalid("The picture didn't arrive intact.".into()))?;
                let path = work.join(format!("{tag}-start.{}", image_extension(&bytes)?));
                tokio::fs::write(&path, bytes).await?;
                Some(path)
            }
            None => None,
        };
        let opts = GenerateOptions {
            binary,
            kind: model.kind,
            files: model.files.clone(),
            prompt: prompt.to_string(),
            negative_prompt: req
                .negative_prompt
                .clone()
                .or_else(|| d.negative_prompt.clone()),
            width,
            height,
            steps,
            cfg_scale,
            seed,
            sampler: d.sampler.clone(),
            flow_shift: d.flow_shift,
            frames,
            fps,
            init_image: init_image.clone(),
            strength: init_image.as_ref().map(|_| strength.unwrap_or(0.75)),
            extra_args: model.args.clone(),
            output: output.clone(),
        };
        let started = Instant::now();
        let result = sd::generate(&opts, |p| on_event(MediaEvent::Progress(p))).await;
        if let Some(p) = &init_image {
            let _ = tokio::fs::remove_file(p).await;
        }
        result?;
        let bytes = tokio::fs::read(&output).await?;
        let _ = tokio::fs::remove_file(&output).await;
        let (w, h) = dimensions(&bytes).unwrap_or((width, height));
        Ok(GeneratedMedia {
            kind: model.kind,
            mime: if video { "image/webp" } else { "image/png" }.into(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            width: w,
            height: h,
            seed,
            model: model.id.clone(),
            model_name: model.name.clone(),
            prompt: prompt.to_string(),
            frames,
            fps,
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}

/// All files of a recipe, counting shared ones once.
fn unique_size(files: &[RecipeFile]) -> u64 {
    let mut seen = std::collections::HashSet::new();
    files
        .iter()
        .filter(|f| seen.insert((&f.repo, &f.path)))
        .map(|f| f.size)
        .sum()
}

fn random_seed() -> i64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    (h.finish() & 0x7fff_ffff) as i64
}

/// The file extension for a starting picture, from its first bytes.
fn image_extension(bytes: &[u8]) -> Result<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Ok("png"),
        [0xFF, 0xD8, 0xFF, ..] => Ok("jpg"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Ok("webp"),
        [b'B', b'M', ..] => Ok("bmp"),
        _ => Err(Error::Invalid(
            "Start from a PNG, JPEG, WebP or BMP picture.".into(),
        )),
    }
}

/// Width and height of a PNG, or of a WebP (still or animated).
fn dimensions(b: &[u8]) -> Option<(u32, u32)> {
    let be =
        |i: usize| -> Option<u32> { Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?)) };
    let le24 = |i: usize| -> Option<u32> {
        let s = b.get(i..i + 3)?;
        Some(u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16)
    };
    match b {
        [0x89, b'P', b'N', b'G', ..] => Some((be(16)?, be(20)?)),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', b'V', b'P', b'8', b'X', ..] => {
            Some((le24(24)? + 1, le24(27)? + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_parses_and_files_have_roles() {
        let all = media_catalog();
        assert!(all.iter().any(|r| r.kind == MediaKind::Image));
        assert!(all.iter().any(|r| r.kind == MediaKind::Video));
        for r in &all {
            assert!(!r.files.is_empty(), "{} has no files", r.id);
            let main = r
                .files
                .iter()
                .filter(|f| matches!(f.role, FileRole::Model | FileRole::DiffusionModel));
            assert_eq!(main.count(), 1, "{} needs one main file", r.id);
            if r.kind == MediaKind::Video {
                assert!(r.defaults.frames.is_some());
            }
        }
    }

    #[test]
    fn shared_files_count_once() {
        let f = |p: &str, size| RecipeFile {
            role: FileRole::Vae,
            repo: "a/b".into(),
            path: p.into(),
            size,
        };
        assert_eq!(unique_size(&[f("x", 2), f("x", 2), f("y", 3)]), 5);
    }

    #[test]
    fn reads_picture_sizes() {
        let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0, 0, 0, 13];
        png.extend(b"IHDR");
        png.extend(640u32.to_be_bytes());
        png.extend(480u32.to_be_bytes());
        assert_eq!(dimensions(&png), Some((640, 480)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend([0; 8]);
        webp.extend([0x3F, 0x03, 0x00, 0xDF, 0x01, 0x00]); // 832-1, 480-1
        assert_eq!(dimensions(&webp), Some((832, 480)));
        assert_eq!(image_extension(&png).unwrap(), "png");
        assert!(image_extension(b"GIF89a").is_err());
    }
}
