//! Access from anywhere with no server of your own. Runs Cloudflare's
//! `cloudflared` connector, which keeps an outgoing connection to Cloudflare
//! and gives this computer a public `https://` address that forwards to the
//! gateway. Works behind any home router, with no port forwarding.
//!
//! - **Quick tunnel:** free and needs no account. The address is random
//!   (`https://<words>.trycloudflare.com`) and changes every time it starts.
//! - **Named tunnel:** a tunnel on your own domain, made in the Cloudflare
//!   dashboard. Its token goes in the settings, and its address never changes.
//!
//! Cloudflare terminates TLS, so it serves the web page to browsers. Calls
//! after pairing stay end-to-end encrypted to this computer's key, so it can't
//! read chats or act as a paired device.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::AbortHandle;

/// The cloudflared release we download when none is installed, with the
/// SHA-256 of each file.
const CLOUDFLARED_VERSION: &str = "2026.9.3";
const CLOUDFLARED_ASSETS: &[(&str, &str, &str, &str)] = &[
    (
        "linux",
        "x86_64",
        "cloudflared-linux-amd64",
        "77e26d8d900e0b8469f416239d14b5f296525fdf79fee6f511ef55609e3fbac2",
    ),
    (
        "linux",
        "aarch64",
        "cloudflared-linux-arm64",
        "aaeb2d7d0da3614634c7e03ab13487a1522c2e79165ed2929cfe23d5e95b326d",
    ),
    (
        "macos",
        "x86_64",
        "cloudflared-darwin-amd64.tgz",
        "d1155d0837487f261183b15c1eab6c4ebcad9dc49b94675f1524c3564cea3977",
    ),
    (
        "macos",
        "aarch64",
        "cloudflared-darwin-arm64.tgz",
        "587c2cfb1c230fe36c7fa7727da78be459dae028cabe8c001291999350f07095",
    ),
    (
        "windows",
        "x86_64",
        "cloudflared-windows-amd64.exe",
        "f096265ec2fcbe9bb6e2d64268db167ced3fcbb83d894bdb9e2fcdb26f2ea7e2",
    ),
    // No Windows Arm build; x64 runs under emulation.
    (
        "windows",
        "aarch64",
        "cloudflared-windows-amd64.exe",
        "f096265ec2fcbe9bb6e2d64268db167ced3fcbb83d894bdb9e2fcdb26f2ea7e2",
    ),
];

const BINARY: &str = if cfg!(windows) {
    "cloudflared.exe"
} else {
    "cloudflared"
};
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// What kind of tunnel to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelSpec {
    Quick,
    Named { token: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    /// `quick` or `cloudflare`.
    pub kind: &'static str,
    /// The public address, once known. Named tunnels report none here; their
    /// address is the `public_url` setting.
    pub url: Option<String>,
    pub connected: bool,
    /// What's happening or what went wrong, while not connected.
    pub error: Option<String>,
}

/// Where to find or put cloudflared.
#[derive(Debug, Clone)]
pub struct TunnelOptions {
    /// Use this binary instead of looking for one.
    pub binary: Option<PathBuf>,
    /// Folder to download cloudflared into when it isn't installed.
    pub bin_dir: PathBuf,
}

/// A running tunnel; dropping it stops cloudflared.
pub(crate) struct Tunnel {
    task: AbortHandle,
    status: Arc<Mutex<TunnelStatus>>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Tunnel {
    pub fn start(spec: TunnelSpec, port: u16, options: TunnelOptions) -> Self {
        let status = Arc::new(Mutex::new(TunnelStatus {
            kind: match spec {
                TunnelSpec::Quick => "quick",
                TunnelSpec::Named { .. } => "cloudflare",
            },
            url: None,
            connected: false,
            error: Some("starting".into()),
        }));
        let task = tokio::spawn(run(spec, port, options, status.clone())).abort_handle();
        Tunnel { task, status }
    }

    pub fn status(&self) -> TunnelStatus {
        self.status.lock().unwrap().clone()
    }
}

async fn run(
    spec: TunnelSpec,
    port: u16,
    options: TunnelOptions,
    status: Arc<Mutex<TunnelStatus>>,
) {
    let set = |f: &dyn Fn(&mut TunnelStatus)| f(&mut status.lock().unwrap());
    let mut backoff = Duration::from_secs(2);
    loop {
        let result =
            match ensure_cloudflared(&options, &|msg| set(&|s| s.error = Some(msg.to_string())))
                .await
            {
                Ok(exe) => run_once(&exe, &spec, port, &status).await,
                Err(e) => Err(e),
            };
        let was_connected = status.lock().unwrap().connected;
        set(&|s| {
            s.connected = false;
            s.url = None;
            s.error = Some(
                result
                    .clone()
                    .err()
                    .unwrap_or_else(|| "cloudflared stopped".into()),
            );
        });
        if was_connected {
            backoff = Duration::from_secs(2);
        }
        tracing::warn!(
            "tunnel stopped ({}); retrying in {}s",
            status.lock().unwrap().error.clone().unwrap_or_default(),
            backoff.as_secs()
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Runs cloudflared until it exits. Returns the last error it printed.
async fn run_once(
    exe: &Path,
    spec: &TunnelSpec,
    port: u16,
    status: &Arc<Mutex<TunnelStatus>>,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(["tunnel", "--no-autoupdate"]);
    match spec {
        TunnelSpec::Quick => {
            cmd.args(["--url", &format!("http://127.0.0.1:{port}")]);
        }
        TunnelSpec::Named { token } => {
            // Through the environment, so the token doesn't show in process lists.
            cmd.args(["run"]).env("TUNNEL_TOKEN", token);
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window for the child.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start cloudflared: {e}"))?;
    {
        let mut s = status.lock().unwrap();
        s.error = Some("connecting to Cloudflare".into());
    }
    let mut lines = BufReader::new(child.stderr.take().expect("piped")).lines();
    let mut last_error = None;
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!("cloudflared: {line}");
        let mut s = status.lock().unwrap();
        if let Some(url) = quick_tunnel_url(&line) {
            s.url = Some(url);
        }
        if is_connected_line(&line) && (s.url.is_some() || matches!(spec, TunnelSpec::Named { .. }))
        {
            if !s.connected {
                tracing::info!("tunnel connected {}", s.url.as_deref().unwrap_or_default());
            }
            s.connected = true;
            s.error = None;
        } else if let Some(err) = error_text(&line) {
            last_error = Some(err.clone());
            if !s.connected {
                s.error = Some(err);
            }
        }
    }
    let _ = child.wait().await;
    Err(last_error.unwrap_or_else(|| "cloudflared stopped".into()))
}

/// The `https://<name>.trycloudflare.com` address in a line of cloudflared's
/// output, if any.
pub fn quick_tunnel_url(line: &str) -> Option<String> {
    line.match_indices("https://").find_map(|(i, _)| {
        let rest = &line[i..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '|' | '"' | '\''))
            .unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches('/');
        let host = url.strip_prefix("https://")?;
        let label = host.strip_suffix(".trycloudflare.com")?;
        let ok = !label.is_empty()
            && label != "api"
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        ok.then(|| url.to_string())
    })
}

fn is_connected_line(line: &str) -> bool {
    line.contains("Registered tunnel connection")
}

/// cloudflared logs errors as `<time> ERR <message>`.
fn error_text(line: &str) -> Option<String> {
    let (_, msg) = line.split_once(" ERR ")?;
    let msg = msg.trim();
    (!msg.is_empty()).then(|| msg.chars().take(300).collect())
}

/// Finds cloudflared: the configured path, one on PATH, or our own copy,
/// downloading it the first time.
async fn ensure_cloudflared(
    options: &TunnelOptions,
    note: &(dyn Fn(&str) + Send + Sync),
) -> Result<PathBuf, String> {
    if let Some(path) = &options.binary {
        return Ok(path.clone());
    }
    let ours = options.bin_dir.join(BINARY);
    if ours.exists() {
        return Ok(ours);
    }
    if let Some(found) = find_on_path(BINARY) {
        return Ok(found);
    }

    let (asset, sha256) =
        asset_for(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
            format!(
                "cloudflared isn't available for {}/{}; install it and set its path in settings",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?;
    note("downloading cloudflared");
    tracing::info!("downloading cloudflared {CLOUDFLARED_VERSION}");
    let url = format!(
        "https://github.com/cloudflare/cloudflared/releases/download/{CLOUDFLARED_VERSION}/{asset}"
    );
    let downloads = options.bin_dir.join("downloads");
    let file = downloads.join(asset);
    let client = reqwest::Client::new();
    brainwashed_core::runtime::download::download(&client, &url, &file, Some(sha256), &|_, _| {})
        .await
        .map_err(|e| format!("couldn't download cloudflared: {e}"))?;
    if asset.ends_with(".tgz") {
        brainwashed_core::runtime::download::extract(&file, &downloads)
            .await
            .map_err(|e| format!("couldn't unpack cloudflared: {e}"))?;
        std::fs::rename(downloads.join(BINARY), &ours).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&file);
    } else {
        std::fs::rename(&file, &ours).map_err(|e| e.to_string())?;
    }
    make_executable(&ours).map_err(|e| e.to_string())?;
    Ok(ours)
}

fn asset_for(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    CLOUDFLARED_ASSETS
        .iter()
        .find(|(o, a, _, _)| *o == os && *a == arch)
        .map(|(_, _, name, sha)| (*name, *sha))
}

fn find_on_path(binary: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(binary))
        .find(|p| p.is_file())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_quick_tunnel_address() {
        let banner = "2026-10-04T12:00:00Z INF |  https://quiet-river-sun-moon.trycloudflare.com                       |";
        assert_eq!(
            quick_tunnel_url(banner).as_deref(),
            Some("https://quiet-river-sun-moon.trycloudflare.com")
        );
        let api = r#"2026-10-04T12:00:00Z ERR failed to request quick Tunnel: Post "https://api.trycloudflare.com/tunnel": dial tcp"#;
        assert_eq!(quick_tunnel_url(api), None);
        assert_eq!(
            quick_tunnel_url("https://evil.com/.trycloudflare.com"),
            None
        );
        assert_eq!(quick_tunnel_url("nothing here"), None);
    }

    #[test]
    fn reads_errors_and_connections() {
        assert_eq!(
            error_text("2026-10-04T12:00:00Z ERR Unable to reach the origin service").as_deref(),
            Some("Unable to reach the origin service")
        );
        assert_eq!(error_text("2026-10-04T12:00:00Z INF Starting tunnel"), None);
        assert!(is_connected_line(
            "2026-10-04T12:00:01Z INF Registered tunnel connection connIndex=0 location=fra08"
        ));
    }

    #[test]
    fn has_a_build_for_common_computers() {
        for (os, arch) in [
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("windows", "x86_64"),
        ] {
            assert!(asset_for(os, arch).is_some(), "{os}/{arch}");
        }
    }

    /// A stand-in cloudflared that prints what the real one prints.
    #[cfg(unix)]
    #[tokio::test]
    async fn reports_the_address_from_cloudflared() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("cloudflared");
        std::fs::write(
            &fake,
            "#!/bin/sh\n\
             echo 'INF Requesting new quick Tunnel on trycloudflare.com...' >&2\n\
             echo 'INF |  https://brave-test-tunnel.trycloudflare.com  |' >&2\n\
             echo 'INF Registered tunnel connection connIndex=0' >&2\n\
             sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let tunnel = Tunnel::start(
            TunnelSpec::Quick,
            1,
            TunnelOptions {
                binary: Some(fake),
                bin_dir: dir.path().into(),
            },
        );
        for _ in 0..200 {
            if tunnel.status().connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let status = tunnel.status();
        assert!(status.connected, "{status:?}");
        assert_eq!(
            status.url.as_deref(),
            Some("https://brave-test-tunnel.trycloudflare.com")
        );
    }
}
