//! Tauri commands the UI calls. Errors are returned as display strings.

use brainwashed_core::{
    CatalogItem, ChatMessage, Delta, Engine, EngineState, Hardware, HostInfo, InstalledModel,
    SamplingOptions, Settings,
};
use std::path::PathBuf;
use tauri::{ipc::Channel, State};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub fn host_info(engine: State<Engine>) -> HostInfo {
    engine.info()
}

#[tauri::command]
pub fn engine_state(engine: State<Engine>) -> EngineState {
    engine.state()
}

#[tauri::command]
pub fn hardware(engine: State<Engine>) -> Hardware {
    engine.hardware().clone()
}

#[tauri::command]
pub fn catalog(engine: State<Engine>) -> Vec<CatalogItem> {
    engine.catalog()
}

#[tauri::command]
pub fn models(engine: State<Engine>) -> Vec<InstalledModel> {
    engine.models()
}

#[tauri::command]
pub async fn download_model(
    engine: State<'_, Engine>,
    repo: String,
    quant: Option<String>,
) -> CmdResult<InstalledModel> {
    engine
        .download_model(&repo, quant.as_deref())
        .await
        .map_err(err)
}

#[tauri::command]
pub fn import_model(engine: State<Engine>, path: PathBuf) -> CmdResult<InstalledModel> {
    engine.import_model(&path).map_err(err)
}

#[tauri::command]
pub async fn delete_model(engine: State<'_, Engine>, id: String) -> CmdResult<()> {
    engine.delete_model(&id).await.map_err(err)
}

#[tauri::command]
pub async fn load_model(engine: State<'_, Engine>, id: String) -> CmdResult<()> {
    engine.load_model(&id).await.map_err(err)
}

#[tauri::command]
pub async fn unload_model(engine: State<'_, Engine>) -> CmdResult<()> {
    engine.unload().await.map_err(err)
}

#[tauri::command]
pub fn get_settings(engine: State<Engine>) -> Settings {
    engine.settings()
}

#[tauri::command]
pub fn update_settings(engine: State<Engine>, settings: Settings) -> CmdResult<()> {
    engine.update_settings(settings).map_err(err)
}

/// Streams the reply through `on_delta` and returns the full answer.
#[tauri::command]
pub async fn chat(
    engine: State<'_, Engine>,
    messages: Vec<ChatMessage>,
    on_delta: Channel<Delta>,
) -> CmdResult<String> {
    engine
        .chat(&messages, &SamplingOptions::default(), |d| {
            let _ = on_delta.send(d);
        })
        .await
        .map_err(err)
}
