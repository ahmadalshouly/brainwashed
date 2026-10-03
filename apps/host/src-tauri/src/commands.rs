//! Tauri commands the UI calls. Errors are returned as display strings.

use brainwashed_core::{
    CatalogItem, ChatEvent, ChatMessage, Engine, EngineState, Hardware, HostInfo, InstalledModel,
    SamplingOptions, Settings, SkillList,
};
use brainwashed_gateway::{Device, Gateway, GatewayStatus, PairingOffer};
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

#[tauri::command]
pub fn skills(engine: State<Engine>) -> SkillList {
    engine.skills()
}

#[tauri::command]
pub fn skill_source(engine: State<Engine>, name: String) -> CmdResult<String> {
    engine.skill_source(&name).map_err(err)
}

#[tauri::command]
pub fn save_skill(
    engine: State<Engine>,
    source: String,
    previous_name: Option<String>,
) -> CmdResult<String> {
    engine
        .save_skill(&source, previous_name.as_deref())
        .map_err(err)
}

#[tauri::command]
pub fn delete_skill(engine: State<Engine>, name: String) -> CmdResult<()> {
    engine.delete_skill(&name).map_err(err)
}

#[tauri::command]
pub fn set_skill_enabled(engine: State<Engine>, name: String, enabled: bool) -> CmdResult<()> {
    engine.set_skill_enabled(&name, enabled).map_err(err)
}

/// Streams the reply through `on_event` and returns the full answer.
#[tauri::command]
pub async fn chat(
    engine: State<'_, Engine>,
    messages: Vec<ChatMessage>,
    on_event: Channel<ChatEvent>,
) -> CmdResult<String> {
    engine
        .chat(&messages, &SamplingOptions::default(), |e| {
            let _ = on_event.send(e);
        })
        .await
        .map_err(err)
}

#[tauri::command]
pub fn phone_status(gateway: State<Gateway>) -> GatewayStatus {
    gateway.status()
}

/// Turns phone access on or off and remembers the choice.
#[tauri::command]
pub async fn set_phone_access(
    engine: State<'_, Engine>,
    gateway: State<'_, Gateway>,
    enabled: bool,
) -> CmdResult<GatewayStatus> {
    let mut settings = engine.settings();
    if enabled {
        gateway
            .start(settings.phone_port)
            .await
            .map_err(|e| format!("Could not open port {}: {e}", settings.phone_port))?;
    } else {
        gateway.stop();
    }
    settings.phone_access = enabled;
    engine.update_settings(settings).map_err(err)?;
    Ok(gateway.status())
}

#[tauri::command]
pub fn create_pairing_offer(gateway: State<Gateway>) -> CmdResult<PairingOffer> {
    gateway.create_pairing_offer().map_err(err)
}

#[tauri::command]
pub fn paired_devices(gateway: State<Gateway>) -> Vec<Device> {
    gateway.devices()
}

#[tauri::command]
pub fn remove_device(gateway: State<Gateway>, id: String) -> CmdResult<()> {
    gateway.remove_device(&id).map_err(err)
}
