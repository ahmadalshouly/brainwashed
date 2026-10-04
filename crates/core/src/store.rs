//! Small JSON files in the data directory.

use crate::Result;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledModel {
    pub id: String,
    pub name: String,
    /// Hugging Face repo it came from, if downloaded.
    pub repo: Option<String>,
    pub path: PathBuf,
    pub size: u64,
    /// Vision projector, for models that can look at pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mmproj: Option<PathBuf>,
}

/// The llama.cpp build currently installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledRuntime {
    pub tag: String,
    pub platform: String,
    pub binary: PathBuf,
}

pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

/// Writes atomically so a crash never leaves a half-written file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Turns a file name into a stable id usable in URLs.
pub fn model_id(file_name: &str) -> String {
    let stem = file_name.strip_suffix(".gguf").unwrap_or(file_name);
    stem.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub fn dir_for_repo(models_dir: &Path, repo: &str) -> PathBuf {
    models_dir.join(repo.replace('/', "__"))
}
