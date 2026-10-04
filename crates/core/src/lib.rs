//! The BrainWashed engine. The `brainwashed` command and the gateway it
//! serves talk to the model through [`Engine`].

mod engine;
mod settings;
mod skills;
mod store;
mod updates;

pub use brainwashed_runtime as runtime;
pub use brainwashed_runtime::chat::{Delta, SamplingOptions};
pub use brainwashed_runtime::{Backend, ChatMessage, Hardware, Role};
pub use brainwashed_skills as skill_format;
pub use engine::{CatalogItem, Engine, EngineConfig, EngineState, Event, HostInfo};
pub use settings::{RemoteAccess, Settings};
pub use skills::{ChatEvent, SkillInfo, SkillList, MAX_SKILL_CHARS};
pub use store::InstalledModel;
pub use updates::UpdateInfo;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Runtime(#[from] brainwashed_runtime::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
