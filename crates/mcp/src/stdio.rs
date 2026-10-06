//! Servers run as a command: one JSON-RPC message per line on its stdin and
//! stdout. What it writes to stderr is kept to explain failures.

use crate::{Error, Link, Peer, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio as Piped;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

pub(crate) struct Stdio {
    /// Lines for the command's stdin. Dropping it closes stdin.
    tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
    child: Mutex<Option<Child>>,
}

impl Stdio {
    pub(crate) fn send(&self, message: &Value) -> Result<()> {
        let line = serde_json::to_string(message).map_err(|e| Error::Failed(e.to_string()))?;
        let tx = self.tx.lock().unwrap();
        tx.as_ref()
            .and_then(|tx| tx.send(line).ok())
            .ok_or_else(|| Error::Closed("the server stopped".into()))
    }

    /// Closes stdin, which tells the server to exit, and stops it if it
    /// hasn't after a moment.
    pub(crate) async fn close(&self) {
        self.tx.lock().unwrap().take();
        let child = self.child.lock().unwrap().take();
        if let Some(mut child) = child {
            if tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        }
    }
}

pub(crate) fn spawn(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: Option<&str>,
) -> Result<Arc<Peer>> {
    let command = command.trim();
    if command.is_empty() {
        return Err(Error::Failed(
            "give the command that starts the server".into(),
        ));
    }
    let program = program(command, env.get("PATH").map(String::as_str));
    let mut cmd = Command::new(&program);
    cmd.args(args)
        .envs(env)
        .stdin(Piped::piped())
        .stdout(Piped::piped())
        .stderr(Piped::piped())
        .kill_on_drop(true);
    if let Some(dir) = cwd.map(str::trim).filter(|d| !d.is_empty()) {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        // No console window popping up for each server.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::Failed(format!(
            "couldn't find `{command}` on this computer. Install it, or give its full path."
        )),
        _ => Error::Failed(format!("couldn't start `{command}`: {e}")),
    })?;
    let stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let peer = Peer::new(Link::Stdio(Stdio {
        tx: Mutex::new(Some(tx)),
        child: Mutex::new(Some(child)),
    }));

    tokio::spawn(async move {
        let mut stdin = stdin;
        while let Some(mut line) = rx.recv().await {
            line.push('\n');
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                break;
            }
        }
        // Dropping stdin here closes it.
    });

    let weak: Weak<Peer> = Arc::downgrade(&peer);
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = lines.next_line().await;
            let Some(peer) = weak.upgrade() else { return };
            match line {
                Ok(Some(line)) => match serde_json::from_str::<Value>(&line) {
                    Ok(message) => peer.incoming(message),
                    // Some servers print banners on stdout; keep them as log.
                    Err(_) => peer.remember(&line),
                },
                Ok(None) | Err(_) => {
                    // Give stderr a moment to catch up, so the reason shows.
                    drop(peer);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    if let Some(peer) = weak.upgrade() {
                        peer.ended("the server stopped");
                    }
                    return;
                }
            }
        }
    });

    let weak: Weak<Peer> = Arc::downgrade(&peer);
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match weak.upgrade() {
                Some(peer) => peer.remember(&line),
                None => return,
            }
        }
    });

    Ok(peer)
}

/// The program to run. Windows only finds `.exe` files by itself, while
/// `npx`, `uvx` and friends are often `.cmd` scripts.
#[cfg(windows)]
fn program(command: &str, path: Option<&str>) -> PathBuf {
    let given = std::path::Path::new(command);
    if given.extension().is_some() || given.components().count() > 1 {
        return given.to_path_buf();
    }
    let path = path
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    for dir in std::env::split_paths(&path) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{command}{ext}"));
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    given.to_path_buf()
}

#[cfg(not(windows))]
fn program(command: &str, _path: Option<&str>) -> PathBuf {
    PathBuf::from(command)
}
