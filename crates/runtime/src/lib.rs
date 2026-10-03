//! Low-level runtime for BrainWashed: hardware detection, downloading the
//! llama.cpp server and GGUF models, running `llama-server`, and streaming
//! chat completions from it.

pub mod catalog;
pub mod chat;
pub mod download;
pub mod hardware;
pub mod llama;
pub mod release;

pub use chat::{ChatMessage, Role};
pub use hardware::{Backend, Hardware};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("archive error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub(crate) fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Reports `(bytes_done, bytes_total)` while something downloads.
pub type Progress<'a> = &'a (dyn Fn(u64, Option<u64>) + Send + Sync);
