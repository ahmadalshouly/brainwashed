//! Runs and supervises a `llama-server` process.

use crate::{Error, Result};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

pub const SERVER_BINARY: &str = if cfg!(windows) {
    "llama-server.exe"
} else {
    "llama-server"
};

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub binary: PathBuf,
    pub model: PathBuf,
    pub context_size: u32,
    /// Layers to offload to the GPU. A large number means "all of them".
    pub gpu_layers: u32,
    pub port: Option<u16>,
}

pub struct LlamaServer {
    child: Child,
    port: u16,
    model: PathBuf,
    log: Arc<Mutex<VecDeque<String>>>,
}

const LOG_LINES: usize = 200;

impl LlamaServer {
    /// Starts llama-server and waits until the model is loaded.
    pub async fn start(
        opts: &ServerOptions,
        client: &reqwest::Client,
        timeout: Duration,
    ) -> Result<Self> {
        let port = match opts.port {
            Some(p) => p,
            None => free_port()?,
        };
        let bin_dir = opts.binary.parent().unwrap_or(Path::new("."));

        let mut cmd = Command::new(&opts.binary);
        cmd.arg("--model")
            .arg(&opts.model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string()])
            .args(["--ctx-size", &opts.context_size.to_string()])
            .args(["--n-gpu-layers", &opts.gpu_layers.to_string()])
            .arg("--jinja")
            .arg("--no-webui")
            .current_dir(bin_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Prebuilt releases ship their shared libraries next to the binary.
        if cfg!(target_os = "linux") {
            cmd.env("LD_LIBRARY_PATH", prepend_env("LD_LIBRARY_PATH", bin_dir));
        }
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::other(format!("could not start {}: {e}", opts.binary.display())))?;
        let log = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(out) = child.stdout.take() {
            tokio::spawn(capture(out, log.clone()));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(capture(err, log.clone()));
        }

        let mut server = LlamaServer {
            child,
            port,
            model: opts.model.clone(),
            log,
        };
        server.wait_ready(client, timeout).await?;
        Ok(server)
    }

    async fn wait_ready(&mut self, client: &reqwest::Client, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        let health = format!("{}/health", self.base_url());
        loop {
            if let Some(status) = self.child.try_wait()? {
                // Give the log readers a moment to drain the pipes.
                tokio::time::sleep(Duration::from_millis(100)).await;
                return Err(Error::other(format!(
                    "llama-server exited with {status} while loading the model:\n{}",
                    self.log_tail(20)
                )));
            }
            if let Ok(res) = client
                .get(&health)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                if res.status().is_success() {
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::other(format!(
                    "llama-server did not become ready in {}s:\n{}",
                    timeout.as_secs(),
                    self.log_tail(20)
                )));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn model(&self) -> &Path {
        &self.model
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn log_tail(&self, lines: usize) -> String {
        let log = self.log.lock().unwrap();
        let skip = log.len().saturating_sub(lines);
        log.iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub async fn stop(mut self) -> Result<()> {
        self.child.kill().await?;
        Ok(())
    }
}

async fn capture(stream: impl tokio::io::AsyncRead + Unpin, log: Arc<Mutex<VecDeque<String>>>) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(target: "llama_server", "{line}");
        let mut log = log.lock().unwrap();
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(line);
    }
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

fn prepend_env(var: &str, dir: &Path) -> std::ffi::OsString {
    let mut paths = vec![dir.to_path_buf()];
    if let Some(existing) = std::env::var_os(var) {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).unwrap_or_default()
}
