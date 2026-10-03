use brainwashed_runtime::Backend;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_SYSTEM_PROMPT: &str =
    "You are BrainWashed, a helpful assistant running privately on the user's own computer. \
Answer clearly and concisely.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Name shown to paired phones. Defaults to the computer's hostname.
    pub host_name: Option<String>,
    pub backend: Backend,
    pub context_size: u32,
    pub gpu_layers: u32,
    /// Use this llama-server instead of downloading one.
    pub llama_server_path: Option<PathBuf>,
    /// Model loaded at startup.
    pub active_model: Option<String>,
    pub system_prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            host_name: None,
            backend: Backend::Auto,
            context_size: 8192,
            gpu_layers: 999,
            llama_server_path: None,
            active_model: None,
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
        }
    }
}
