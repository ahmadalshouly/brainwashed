use serde::Serialize;

/// Mirrors `HostInfo` in `packages/api/src/types.ts`.
#[derive(Debug, Serialize)]
pub struct HostInfo {
    name: String,
    version: String,
    model: Option<String>,
}

#[tauri::command]
fn host_info(app: tauri::AppHandle) -> HostInfo {
    HostInfo {
        name: hostname(),
        version: app.package_info().version.to_string(),
        model: None,
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "This computer".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![host_info])
        .run(tauri::generate_context!())
        .expect("error while running BrainWashed host");
}
