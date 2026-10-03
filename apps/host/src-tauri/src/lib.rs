mod commands;

use brainwashed_core::{Engine, EngineConfig};
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let version = app.package_info().version.to_string();
            let engine = Engine::new(EngineConfig::new(data_dir, version))?;

            // Forward engine events to the UI.
            let mut events = engine.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let _ = handle.emit("engine", event);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break,
                    }
                }
            });

            // Bring back the model that was loaded last time.
            let restoring = engine.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = restoring.restore().await {
                    tracing::warn!("could not restore the last model: {e}");
                }
            });

            app.manage(engine);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::host_info,
            commands::engine_state,
            commands::hardware,
            commands::catalog,
            commands::models,
            commands::download_model,
            commands::import_model,
            commands::delete_model,
            commands::load_model,
            commands::unload_model,
            commands::get_settings,
            commands::update_settings,
            commands::chat,
        ])
        .build(tauri::generate_context!())
        .expect("error while building BrainWashed host");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            // Don't leave llama-server running after the app quits.
            let engine = handle.state::<Engine>().inner().clone();
            let _ = tauri::async_runtime::block_on(engine.unload());
        }
    });
}
