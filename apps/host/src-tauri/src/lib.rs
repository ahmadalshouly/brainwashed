mod commands;

use brainwashed_core::{Engine, EngineConfig};
use brainwashed_gateway::Gateway;
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
            let engine = Engine::new(EngineConfig::new(&data_dir, version))?;
            let gateway = Gateway::new(engine.clone(), &data_dir)?;
            if let Err(e) = gateway.set_relay_url(engine.settings().relay_url.as_deref()) {
                tracing::warn!("ignoring the saved relay address: {e}");
            }

            let mut gateway_events = gateway.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = gateway_events.recv().await {
                    let _ = handle.emit("gateway", event);
                }
            });

            if engine.settings().phone_access {
                let gw = gateway.clone();
                let port = engine.settings().phone_port;
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = gw.start(port).await {
                        tracing::warn!("could not start phone access on port {port}: {e}");
                    }
                });
            }
            app.manage(gateway);

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

            // Pick up skills edited in any text editor.
            let watching = engine.clone();
            tauri::async_runtime::spawn(async move {
                let _ = watching
                    .watch_skills(std::time::Duration::from_secs(2))
                    .await;
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
            commands::skills,
            commands::skill_source,
            commands::save_skill,
            commands::delete_skill,
            commands::set_skill_enabled,
            commands::chat,
            commands::phone_status,
            commands::set_phone_access,
            commands::set_relay_url,
            commands::create_pairing_offer,
            commands::paired_devices,
            commands::remove_device,
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
