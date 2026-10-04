//! Remote access: keeps an outgoing WebSocket open to a relay
//! (`crates/relay`) so devices away from home can reach this computer with no
//! router configuration. Requests arriving through the relay go to the same
//! handlers as local ones, and their bodies are already end-to-end encrypted,
//! so the relay can't read or forge anything.

use crate::crypto::{self, HostKeys};
use axum::body::Body;
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use brainwashed_relay::proto::{FromHost, ToHost, FORWARDED_PATHS};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

/// No message (pings included) for this long means the relay is gone.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelayStatus {
    pub url: String,
    pub connected: bool,
    /// Why the last attempt failed, while not connected.
    pub error: Option<String>,
}

/// Checks a relay address typed by the user and returns it without a
/// trailing slash.
pub fn normalize_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or("The relay address must start with https://")?;
    if rest.is_empty() || rest.contains(['?', '#', ' ']) {
        return Err("That doesn't look like a relay address.".into());
    }
    Ok(url.to_string())
}

/// Where a device reaches this host through the relay.
pub fn device_base(relay_url: &str, keys: &HostKeys) -> String {
    format!("{relay_url}/h/{}", keys.public_key_b64url())
}

fn websocket_url(relay_url: &str) -> String {
    let url = if let Some(rest) = relay_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else {
        format!("ws://{}", relay_url.trim_start_matches("http://"))
    };
    format!("{url}/host/connect")
}

/// A running connection loop; dropping it disconnects.
pub(crate) struct RelayClient {
    task: AbortHandle,
    status: Arc<Mutex<RelayStatus>>,
}

impl Drop for RelayClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl RelayClient {
    pub fn start(url: String, keys: HostKeys, router: Router) -> Self {
        let status = Arc::new(Mutex::new(RelayStatus {
            url: url.clone(),
            connected: false,
            error: None,
        }));
        let task = tokio::spawn(run(url, keys, router, status.clone())).abort_handle();
        RelayClient { task, status }
    }

    pub fn status(&self) -> RelayStatus {
        self.status.lock().unwrap().clone()
    }
}

async fn run(url: String, keys: HostKeys, router: Router, status: Arc<Mutex<RelayStatus>>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let result = connect_once(&url, &keys, &router, &status).await;
        let was_connected = {
            let mut s = status.lock().unwrap();
            let was = s.connected;
            s.connected = false;
            s.error = Some(result.err().unwrap_or_else(|| "disconnected".into()));
            was
        };
        if was_connected {
            backoff = Duration::from_secs(1);
        }
        tracing::info!(
            "relay connection ended ({}); retrying in {}s",
            status.lock().unwrap().error.clone().unwrap_or_default(),
            backoff.as_secs()
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn tls_connector() -> Result<tokio_tungstenite::Connector, String> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(tokio_tungstenite::Connector::Rustls(Arc::new(config)))
}

async fn connect_once(
    url: &str,
    keys: &HostKeys,
    router: &Router,
    status: &Arc<Mutex<RelayStatus>>,
) -> Result<(), String> {
    let connect = tokio_tungstenite::connect_async_tls_with_config(
        websocket_url(url),
        None,
        false,
        Some(tls_connector()?),
    );
    let (mut socket, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect)
        .await
        .map_err(|_| "the relay didn't answer".to_string())?
        .map_err(|e| format!("couldn't reach the relay: {e}"))?;

    // Prove we hold the host key.
    let challenge = match next_message(&mut socket).await? {
        ToHost::Challenge {
            challenge,
            relay_key,
        } => {
            let relay_key =
                crypto::parse_public_key(&relay_key).map_err(|_| "bad relay key".to_string())?;
            let challenge = STANDARD
                .decode(challenge)
                .map_err(|_| "bad challenge".to_string())?;
            let env = keys.seal(&relay_key, &challenge);
            FromHost::Auth {
                host_key: keys.public_key_b64url(),
                n: env.n,
                c: env.c,
            }
        }
        _ => return Err("the relay didn't send a challenge".into()),
    };
    send(&mut socket, &challenge).await?;
    if next_message(&mut socket).await? != ToHost::Ready {
        return Err("the relay refused this computer".into());
    }
    {
        let mut s = status.lock().unwrap();
        s.connected = true;
        s.error = None;
    }
    tracing::info!("connected to relay {url}");

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<FromHost>();
    let running: Arc<Mutex<HashMap<String, AbortHandle>>> = Default::default();
    let result = loop {
        tokio::select! {
            out = out_rx.recv() => {
                let out = out.expect("sender kept below");
                if let Err(e) = send(&mut socket, &out).await {
                    break Err(e);
                }
            }
            msg = tokio::time::timeout(READ_TIMEOUT, socket.next()) => {
                let msg = match msg {
                    Err(_) => break Err("the relay stopped responding".into()),
                    Ok(None) => break Ok(()),
                    Ok(Some(Err(e))) => break Err(e.to_string()),
                    Ok(Some(Ok(m))) => m,
                };
                let Message::Text(text) = msg else { continue };
                match serde_json::from_str::<ToHost>(&text) {
                    Ok(ToHost::Request { id, path, body }) => {
                        let task = tokio::spawn(answer(
                            router.clone(),
                            id.clone(),
                            path,
                            body,
                            out_tx.clone(),
                        ));
                        let mut running = running.lock().unwrap();
                        running.retain(|_, t| !t.is_finished());
                        running.insert(id, task.abort_handle());
                    }
                    Ok(ToHost::Cancel { id }) => {
                        if let Some(task) = running.lock().unwrap().remove(&id) {
                            task.abort();
                        }
                    }
                    _ => {}
                }
            }
        }
    };
    for (_, task) in running.lock().unwrap().drain() {
        task.abort();
    }
    result
}

async fn next_message<S>(socket: &mut S) -> Result<ToHost, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = tokio::time::timeout(CONNECT_TIMEOUT, socket.next())
            .await
            .map_err(|_| "the relay didn't answer".to_string())?
            .ok_or("the relay closed the connection")?
            .map_err(|e| e.to_string())?;
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).map_err(|e| format!("bad relay message: {e}"));
        }
    }
}

async fn send<S>(socket: &mut S, msg: &FromHost) -> Result<(), String>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let text = serde_json::to_string(msg).expect("message serializes");
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(|e| e.to_string())
}

/// Runs one forwarded request through the gateway's own routes and streams
/// the answer back.
async fn answer(
    router: Router,
    id: String,
    path: String,
    body: String,
    out: mpsc::UnboundedSender<FromHost>,
) {
    let response = if FORWARDED_PATHS.contains(&path.as_str()) {
        let method = if path == "/hello" { "GET" } else { "POST" };
        let request = http::Request::builder()
            .method(method)
            .uri(&path)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .expect("request builds");
        router.oneshot(request).await.unwrap_or_else(|e| match e {})
    } else {
        http::Response::builder()
            .status(http::StatusCode::NOT_FOUND)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"error":"not available through the relay"}"#))
            .expect("response builds")
    };

    let content_type = response
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let _ = out.send(FromHost::Head {
        id: id.clone(),
        status: response.status().as_u16(),
        content_type,
    });
    let mut data = response.into_body().into_data_stream();
    while let Some(chunk) = data.next().await {
        let Ok(chunk) = chunk else { break };
        let _ = out.send(FromHost::Chunk {
            id: id.clone(),
            data: STANDARD.encode(&chunk),
        });
    }
    let _ = out.send(FromHost::End { id });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_relay_addresses() {
        assert_eq!(
            normalize_url(" https://relay.example.org/ ").unwrap(),
            "https://relay.example.org"
        );
        assert!(normalize_url("relay.example.org").is_err());
        assert!(normalize_url("https://").is_err());
        assert!(normalize_url("ftp://x").is_err());
        assert_eq!(
            websocket_url("https://relay.example.org"),
            "wss://relay.example.org/host/connect"
        );
        assert_eq!(
            websocket_url("http://127.0.0.1:9000"),
            "ws://127.0.0.1:9000/host/connect"
        );
    }
}
