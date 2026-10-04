//! The BrainWashed relay: lets a device reach its home computer when they
//! are on different networks, with no router configuration.
//!
//! The host keeps one outgoing WebSocket open to the relay (`/host/connect`)
//! and proves it holds the secret key for its public key. Devices then send
//! the same requests they would send on the local network to
//! `/h/<host key>/pair`, `/h/<host key>/rpc` or `/h/<host key>/hello`, and
//! the relay passes them down the WebSocket and streams the answers back.
//!
//! Everything a device sends after pairing is already end-to-end encrypted
//! to the host's key, so the relay only ever sees ciphertext. It never
//! stores anything.

pub mod proto;

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Path, State,
    },
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use futures_util::{SinkExt, StreamExt};
use proto::{FromHost, ToHost};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// How long a host has to answer the challenge.
const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a device waits for the host to start answering.
const HEAD_TIMEOUT: Duration = Duration::from_secs(30);
/// Keeps idle connections open through proxies and lets hosts notice a dead
/// relay.
pub const PING_INTERVAL: Duration = Duration::from_secs(20);
/// Requests in flight per host. More get 429.
const MAX_PENDING: usize = 64;
/// Devices send whole conversations with each chat request.
const MAX_BODY: usize = 8 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct Relay {
    hosts: Arc<Mutex<HashMap<String, Arc<HostConn>>>>,
}

struct HostConn {
    conn_id: u64,
    to_host: mpsc::UnboundedSender<ToHost>,
    pending: Mutex<HashMap<String, mpsc::UnboundedSender<Reply>>>,
}

enum Reply {
    Head { status: u16, content_type: String },
    Chunk(Vec<u8>),
    End,
}

static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

impl Relay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/", get(index))
            .route("/host/connect", get(host_connect))
            .route("/h/{key}/hello", get(device_hello))
            .route("/h/{key}/pair", post(device_pair))
            .route("/h/{key}/rpc", post(device_rpc))
            .layer(DefaultBodyLimit::max(MAX_BODY))
            .with_state(self.clone())
    }

    /// Number of hosts connected right now.
    pub fn host_count(&self) -> usize {
        self.hosts.lock().unwrap().len()
    }

    pub fn is_connected(&self, host_key: &str) -> bool {
        self.hosts.lock().unwrap().contains_key(host_key)
    }
}

/// Serves the relay on `listener` until the process ends.
pub async fn serve(listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, Relay::new().router()).await
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

async fn index(State(relay): State<Relay>) -> Json<serde_json::Value> {
    Json(json!({ "app": "brainwashed-relay", "hosts": relay.host_count() }))
}

// ----- hosts -----

async fn host_connect(State(relay): State<Relay>, ws: WebSocketUpgrade) -> Response {
    ws.max_message_size(MAX_BODY * 2)
        .on_upgrade(move |socket| handle_host(relay, socket))
}

/// The host's public key if it answers the challenge correctly.
async fn authenticate(socket: &mut WebSocket) -> Option<String> {
    let challenge: [u8; 32] = rand_bytes();
    let relay_secret = SecretKey::generate(&mut OsRng);
    let hello = ToHost::Challenge {
        challenge: STANDARD.encode(challenge),
        relay_key: STANDARD.encode(relay_secret.public_key().as_bytes()),
    };
    socket
        .send(Message::Text(serde_json::to_string(&hello).ok()?.into()))
        .await
        .ok()?;
    let reply = tokio::time::timeout(AUTH_TIMEOUT, socket.next())
        .await
        .ok()??
        .ok()?;
    let Message::Text(text) = reply else {
        return None;
    };
    let FromHost::Auth { host_key, n, c } = serde_json::from_str(&text).ok()? else {
        return None;
    };
    let key_bytes: [u8; 32] = URL_SAFE_NO_PAD.decode(&host_key).ok()?.try_into().ok()?;
    let nonce: [u8; 24] = STANDARD.decode(&n).ok()?.try_into().ok()?;
    let sealed = STANDARD.decode(&c).ok()?;
    let opened = SalsaBox::new(&PublicKey::from(key_bytes), &relay_secret)
        .decrypt(&nonce.into(), sealed.as_slice())
        .ok()?;
    (opened == challenge).then_some(host_key)
}

fn rand_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    let mut filled = 0;
    while filled < N {
        // A nonce is 24 random bytes from the OS generator.
        let chunk = SalsaBox::generate_nonce(&mut OsRng);
        let take = (N - filled).min(chunk.len());
        out[filled..filled + take].copy_from_slice(&chunk[..take]);
        filled += take;
    }
    out
}

async fn handle_host(relay: Relay, mut socket: WebSocket) {
    let Some(host_key) = authenticate(&mut socket).await else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<ToHost>();
    let conn = Arc::new(HostConn {
        conn_id: NEXT_CONN.fetch_add(1, Ordering::Relaxed),
        to_host: tx,
        pending: Mutex::new(HashMap::new()),
    });
    // A newer connection from the same host replaces the old one, which
    // ends when its sender is dropped here.
    relay
        .hosts
        .lock()
        .unwrap()
        .insert(host_key.clone(), conn.clone());
    tracing::info!("host connected ({} online)", relay.host_count());
    let _ = conn.to_host.send(ToHost::Ready);

    let (mut sink, mut stream) = socket.split();
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(PING_INTERVAL);
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    let Some(msg) = msg else { break };
                    let text = serde_json::to_string(&msg).expect("message serializes");
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sink.send(Message::Close(None)).await;
    });

    while let Some(Ok(msg)) = stream.next().await {
        let Message::Text(text) = msg else { continue };
        let Ok(msg) = serde_json::from_str::<FromHost>(&text) else {
            continue;
        };
        let (id, reply) = match msg {
            FromHost::Auth { .. } => continue,
            FromHost::Head {
                id,
                status,
                content_type,
            } => (
                id,
                Reply::Head {
                    status,
                    content_type,
                },
            ),
            FromHost::Chunk { id, data } => match STANDARD.decode(data) {
                Ok(bytes) => (id, Reply::Chunk(bytes)),
                Err(_) => continue,
            },
            FromHost::End { id } => (id, Reply::End),
        };
        let mut pending = conn.pending.lock().unwrap();
        let end = matches!(reply, Reply::End);
        if let Some(tx) = pending.get(&id) {
            let _ = tx.send(reply);
        }
        if end {
            pending.remove(&id);
        }
    }

    // Gone: forget it (unless it was already replaced) and end every
    // request still waiting on it.
    {
        let mut hosts = relay.hosts.lock().unwrap();
        if hosts.get(&host_key).map(|c| c.conn_id) == Some(conn.conn_id) {
            hosts.remove(&host_key);
        }
    }
    conn.pending.lock().unwrap().clear();
    writer.abort();
    tracing::info!("host disconnected ({} online)", relay.host_count());
}

// ----- devices -----

async fn device_hello(State(relay): State<Relay>, Path(key): Path<String>) -> Response {
    forward(relay, key, "/hello", String::new()).await
}

async fn device_pair(
    State(relay): State<Relay>,
    Path(key): Path<String>,
    body: String,
) -> Response {
    forward(relay, key, "/pair", body).await
}

async fn device_rpc(State(relay): State<Relay>, Path(key): Path<String>, body: String) -> Response {
    forward(relay, key, "/rpc", body).await
}

/// Removes the request from the host's pending list when the device's
/// response is finished or dropped, and tells the host if it stopped early.
struct PendingGuard {
    conn: Arc<HostConn>,
    id: String,
    finished: bool,
}

impl PendingGuard {
    /// The request ended normally; nothing to cancel.
    fn finish(&mut self) {
        self.finished = true;
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.conn.pending.lock().unwrap().remove(&self.id);
        if !self.finished {
            let _ = self.conn.to_host.send(ToHost::Cancel {
                id: self.id.clone(),
            });
        }
    }
}

async fn forward(relay: Relay, key: String, path: &str, body: String) -> Response {
    let Some(conn) = relay.hosts.lock().unwrap().get(&key).cloned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The computer is offline: it may be asleep, or BrainWashed isn't running.",
        );
    };
    let id = URL_SAFE_NO_PAD.encode(rand_bytes::<16>());
    let (tx, mut rx) = mpsc::unbounded_channel();
    {
        let mut pending = conn.pending.lock().unwrap();
        if pending.len() >= MAX_PENDING {
            return error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many requests to this computer at once. Try again in a moment.",
            );
        }
        pending.insert(id.clone(), tx);
    }
    let mut guard = PendingGuard {
        conn: conn.clone(),
        id: id.clone(),
        finished: false,
    };
    let request = ToHost::Request {
        id,
        path: path.to_string(),
        body,
    };
    if conn.to_host.send(request).is_err() {
        guard.finish();
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "The computer just went offline.",
        );
    }

    let head = match tokio::time::timeout(HEAD_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Head {
            status,
            content_type,
        })) => (status, content_type),
        Ok(_) => {
            guard.finish();
            return error(StatusCode::BAD_GATEWAY, "The computer stopped answering.");
        }
        Err(_) => {
            return error(
                StatusCode::GATEWAY_TIMEOUT,
                "The computer took too long to answer.",
            );
        }
    };

    let stream = futures_util::stream::unfold((rx, guard), |(mut rx, mut guard)| async move {
        match rx.recv().await {
            Some(Reply::Chunk(bytes)) => Some((Ok::<_, std::io::Error>(bytes), (rx, guard))),
            Some(Reply::End) => {
                guard.finish();
                None
            }
            // The host disconnected mid-answer; the device sees a cut-off reply.
            Some(Reply::Head { .. }) | None => {
                guard.finish();
                None
            }
        }
    });
    Response::builder()
        .status(StatusCode::from_u16(head.0).unwrap_or(StatusCode::BAD_GATEWAY))
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&head.1)
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}
