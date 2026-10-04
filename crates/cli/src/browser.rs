//! Opens the admin page in its own window when a Chromium browser (Chrome,
//! Edge, Chromium, Brave) is installed, so it feels like an app rather than
//! another tab. Falls back to the default browser.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Returns false if no browser could be started.
pub fn open_app_window(url: &str) -> bool {
    if let Some(browser) = find_chromium() {
        let started = Command::new(&browser)
            .arg(format!("--app={url}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok();
        if started {
            return true;
        }
    }
    open::that_detached(url).is_ok()
}

fn find_chromium() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists());
    }
    if cfg!(windows) {
        let mut candidates = Vec::new();
        for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let base = Path::new(&base);
                candidates.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
                candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            }
        }
        return candidates.into_iter().find(|p| p.exists());
    }
    let path = std::env::var_os("PATH")?;
    [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "brave-browser",
    ]
    .iter()
    .flat_map(|name| std::env::split_paths(&path).map(move |dir| dir.join(name)))
    .find(|p| p.is_file())
}
