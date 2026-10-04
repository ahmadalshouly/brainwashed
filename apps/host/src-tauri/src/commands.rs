//! Tauri commands the UI calls. Errors are returned as display strings.

use brainwashed_core::{
    CatalogItem, ChatEvent, ChatMessage, Engine, EngineState, Hardware, HostInfo, InstalledModel,
    SamplingOptions, Settings, SkillList, UpdateInfo,
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
pub async fn check_for_update(engine: State<'_, Engine>) -> CmdResult<Option<UpdateInfo>> {
    Ok(engine.check_for_update().await)
}

#[tauri::command]
pub fn update_checks_enabled(engine: State<Engine>) -> bool {
    engine.settings().check_for_updates
}

#[tauri::command]
pub fn set_update_checks(engine: State<Engine>, enabled: bool) -> CmdResult<()> {
    let mut settings = engine.settings();
    settings.check_for_updates = enabled;
    engine.update_settings(settings).map_err(err)
}

/// Opens a release page in the browser. Only this project's releases.
#[tauri::command]
pub fn open_release_page(url: String) -> CmdResult<()> {
    if !url.starts_with("https://github.com/ahmadalshouly/brainwashed/releases") {
        return Err("not a BrainWashed release page".into());
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(err)
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

/// Sets the relay for access away from home (None turns it off) and
/// remembers it.
#[tauri::command]
pub fn set_relay_url(
    engine: State<'_, Engine>,
    gateway: State<'_, Gateway>,
    url: Option<String>,
) -> CmdResult<GatewayStatus> {
    gateway.set_relay_url(url.as_deref()).map_err(err)?;
    let mut settings = engine.settings();
    settings.relay_url = gateway.status().relay.map(|r| r.url);
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
