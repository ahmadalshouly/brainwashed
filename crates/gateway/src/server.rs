use crate::crypto::{self, Envelope, HostKeys};
use crate::devices::{Device, DeviceStore};
use crate::{Error, Result};
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use brainwashed_core::{ChatEvent, ChatMessage, Engine, SamplingOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, oneshot};

pub const DEFAULT_PORT: u16 = 47860;
const PROTOCOL_VERSION: u32 = 1;
const OFFER_TTL: Duration = Duration::from_secs(10 * 60);
/// Requests whose clock differs from ours by more than this are refused.
const MAX_CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOffer {
    /// App link: `brainwashed://pair?…`.
    pub url: String,
    /// The same pairing details as a link to the web chat this gateway
    /// serves, carried in the URL fragment so they never reach the network.
    /// Phone cameras open it in the browser, and the app accepts it too, so
    /// it is what the QR code encodes.
    pub web_url: String,
    pub expires_at: u64,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub addresses: Vec<IpAddr>,
    pub host_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GatewayEvent {
    DevicePaired { device: Device },
    DevicesChanged,
}

#[derive(Clone)]
pub struct Gateway {
    inner: Arc<Inner>,
}

struct Inner {
    engine: Engine,
    keys: HostKeys,
    devices: DeviceStore,
    offers: Mutex<HashMap<String, u64>>,
    /// Nonces of recent requests with their arrival time (ms), oldest first.
    seen_nonces: Mutex<VecDeque<(u64, String)>>,
    running: Mutex<Option<Running>>,
    events: broadcast::Sender<GatewayEvent>,
}

struct Running {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
}

impl Gateway {
    pub fn new(engine: Engine, data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("gateway");
        Ok(Gateway {
            inner: Arc::new(Inner {
                keys: HostKeys::load_or_create(&dir.join("host.key"))?,
                devices: DeviceStore::open(&dir.join("devices.json"))?,
                engine,
                offers: Mutex::new(HashMap::new()),
                seen_nonces: Mutex::new(VecDeque::new()),
                running: Mutex::new(None),
                events: broadcast::channel(32).0,
            }),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<GatewayEvent> {
        self.inner.events.subscribe()
    }

    /// Starts listening on every interface. Port 0 picks a free port.
    pub async fn start(&self, port: u16) -> Result<SocketAddr> {
        if let Some(addr) = self.inner.running.lock().unwrap().as_ref().map(|r| r.addr) {
            return Ok(addr);
        }
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
        let addr = listener.local_addr()?;
        let (tx, rx) = oneshot::channel();
        let app = self.router();
        tokio::spawn(async move {
            let server = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(e) = server.await {
                tracing::error!("phone gateway stopped: {e}");
            }
        });
        tracing::info!("phone gateway listening on {addr}");
        *self.inner.running.lock().unwrap() = Some(Running { addr, shutdown: tx });
        Ok(addr)
    }

    pub fn stop(&self) {
        if let Some(r) = self.inner.running.lock().unwrap().take() {
            let _ = r.shutdown.send(());
        }
    }

    pub fn status(&self) -> GatewayStatus {
        let port = self
            .inner
            .running
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.addr.port());
        GatewayStatus {
            running: port.is_some(),
            port,
            addresses: lan_addresses(),
            host_id: self.inner.keys.host_id(),
        }
    }

    /// A one-time pairing link for the QR code. The gateway must be running.
    pub fn create_pairing_offer(&self) -> Result<PairingOffer> {
        let port = self
            .status()
            .port
            .ok_or_else(|| Error::Invalid("turn on phone access first".into()))?;
        let token = crypto::random_token();
        let expires_at = now_secs() + OFFER_TTL.as_secs();
        {
            let mut offers = self.inner.offers.lock().unwrap();
            let now = now_secs();
            offers.retain(|_, exp| *exp > now);
            offers.insert(token.clone(), expires_at);
        }
        let addresses = lan_addresses();
        let addr_list = addresses
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            "v={PROTOCOL_VERSION}&k={}&t={token}&a={addr_list}&p={port}&n={}",
            self.inner.keys.public_key_b64url(),
            percent_encode(&self.inner.engine.host_name()),
        );
        let web_host = addresses
            .first()
            .map(|a| a.to_string())
            .unwrap_or_else(|| "localhost".into());
        Ok(PairingOffer {
            url: format!("brainwashed://pair?{query}"),
            web_url: format!("http://{web_host}:{port}/#pair?{query}"),
            expires_at,
            addresses,
            port,
        })
    }

    pub fn devices(&self) -> Vec<Device> {
        self.inner.devices.list()
    }

    pub fn remove_device(&self, id: &str) -> Result<()> {
        if self.inner.devices.remove(id)? {
            let _ = self.inner.events.send(GatewayEvent::DevicesChanged);
        }
        Ok(())
    }

    fn router(&self) -> Router {
        Router::new()
            .route("/hello", get(hello))
            .route("/pair", post(pair))
            .route("/rpc", post(rpc))
            .fallback(get(crate::web::serve))
            .with_state(self.clone())
    }

    /// Rejects a nonce we have already seen, so a captured request can't be
    /// replayed.
    /// Nonces are kept for twice the allowed clock skew, longer than any
    /// request with an acceptable timestamp could be replayed.
    fn check_replay(&self, nonce: &str) -> bool {
        let now = now_ms();
        let mut seen = self.inner.seen_nonces.lock().unwrap();
        while seen
            .front()
            .is_some_and(|(t, _)| now.saturating_sub(*t) > 2 * MAX_CLOCK_SKEW_MS)
        {
            seen.pop_front();
        }
        if seen.iter().any(|(_, n)| n == nonce) {
            return false;
        }
        seen.push_back((now, nonce.to_string()));
        true
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Private IPv4 addresses a phone on the same network could reach.
pub fn lan_addresses() -> Vec<IpAddr> {
    let mut addrs: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| i.ip())
        .filter(|ip| match ip {
            IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
            IpAddr::V6(_) => false,
        })
        .collect();
    addrs.sort();
    addrs.dedup();
    addrs
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn reject(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

async fn hello(State(gw): State<Gateway>) -> Json<Value> {
    Json(json!({
        "app": "brainwashed",
        "protocol": PROTOCOL_VERSION,
        "hostId": gw.inner.keys.host_id(),
    }))
}

// ----- pairing -----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairRequest {
    device_public_key: String,
    #[serde(flatten)]
    envelope: Envelope,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairSecret {
    token: String,
    device_name: String,
}

async fn pair(State(gw): State<Gateway>, Json(req): Json<PairRequest>) -> Response {
    let Ok(device_key) = crypto::parse_public_key(&req.device_public_key) else {
        return reject(StatusCode::BAD_REQUEST, "bad device key");
    };
    let Ok(plain) = gw.inner.keys.open(&device_key, &req.envelope) else {
        return reject(
            StatusCode::UNAUTHORIZED,
            "could not decrypt pairing request",
        );
    };
    let Ok(secret) = serde_json::from_slice::<PairSecret>(&plain) else {
        return reject(StatusCode::BAD_REQUEST, "bad pairing request");
    };
    let valid = {
        let mut offers = gw.inner.offers.lock().unwrap();
        // Single use: the token is consumed even if it has expired.
        matches!(offers.remove(&secret.token), Some(exp) if exp > now_secs())
    };
    if !valid {
        return reject(
            StatusCode::UNAUTHORIZED,
            "this pairing code has expired or was already used; show a new QR code on the computer",
        );
    }

    let device = Device {
        id: crypto::random_token(),
        name: secret.device_name.chars().take(64).collect(),
        public_key: req.device_public_key.clone(),
        paired_at: now_secs(),
        last_seen: Some(now_secs()),
    };
    if let Err(e) = gw.inner.devices.add(device.clone()) {
        return reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let _ = gw.inner.events.send(GatewayEvent::DevicePaired {
        device: device.clone(),
    });
    tracing::info!("paired phone `{}`", device.name);

    let body = json!({
        "deviceId": device.id,
        "hostId": gw.inner.keys.host_id(),
        "hostName": gw.inner.engine.host_name(),
    });
    Json(gw.inner.keys.seal(&device_key, body.to_string().as_bytes())).into_response()
}

// ----- encrypted calls -----

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcRequest {
    device_id: String,
    #[serde(flatten)]
    envelope: Envelope,
}

#[derive(Deserialize)]
struct Call {
    /// Sender's clock, unix milliseconds.
    ts: u64,
    method: String,
    #[serde(default)]
    params: Value,
}

async fn rpc(State(gw): State<Gateway>, Json(req): Json<RpcRequest>) -> Response {
    let Some(device) = gw.inner.devices.get(&req.device_id) else {
        return reject(
            StatusCode::UNAUTHORIZED,
            "this phone is not paired with this computer",
        );
    };
    let Ok(device_key) = crypto::parse_public_key(&device.public_key) else {
        return reject(
            StatusCode::INTERNAL_SERVER_ERROR,
            "stored device key is invalid",
        );
    };
    let Ok(plain) = gw.inner.keys.open(&device_key, &req.envelope) else {
        return reject(StatusCode::UNAUTHORIZED, "could not decrypt request");
    };
    let Ok(call) = serde_json::from_slice::<Call>(&plain) else {
        return reject(StatusCode::BAD_REQUEST, "bad request");
    };
    if now_ms().abs_diff(call.ts) > MAX_CLOCK_SKEW_MS {
        return reject(
            StatusCode::UNAUTHORIZED,
            "the phone's clock is too far off from the computer's",
        );
    }
    if !gw.check_replay(&req.envelope.n) {
        return reject(StatusCode::UNAUTHORIZED, "request was already used");
    }
    gw.inner.devices.touch(&device.id, now_secs());

    if call.method == "chat" {
        return chat(gw, device_key, call.params);
    }
    let result = handle(&gw, &call.method, call.params).await;
    let body = match result {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    };
    Json(gw.inner.keys.seal(&device_key, body.to_string().as_bytes())).into_response()
}

async fn handle(gw: &Gateway, method: &str, params: Value) -> Result<Value, String> {
    let engine = &gw.inner.engine;
    let to_json = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
    match method {
        "info" => to_json(serde_json::to_value(engine.info())),
        "state" => to_json(serde_json::to_value(engine.state())),
        "models" => to_json(serde_json::to_value(engine.models())),
        "loadModel" => {
            let id = params["id"].as_str().ok_or("missing id")?.to_string();
            // Loading can take minutes; reply now and let the phone poll state.
            let engine = engine.clone();
            tokio::spawn(async move {
                if let Err(e) = engine.load_model(&id).await {
                    tracing::warn!("phone-requested load failed: {e}");
                }
            });
            Ok(Value::Null)
        }
        "skills" => to_json(serde_json::to_value(engine.skills().skills)),
        "setSkillEnabled" => {
            let name = params["name"].as_str().ok_or("missing name")?;
            let enabled = params["enabled"].as_bool().ok_or("missing enabled")?;
            engine
                .set_skill_enabled(name, enabled)
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        other => Err(format!("unknown method `{other}`")),
    }
}

/// One encrypted JSON frame per line: `{"event":…}` while streaming, then
/// `{"done":"full answer"}` or `{"error":"…"}`.
fn chat(gw: Gateway, device_key: crypto_box::PublicKey, params: Value) -> Response {
    let messages: Vec<ChatMessage> = match serde_json::from_value(params["messages"].clone()) {
        Ok(m) => m,
        Err(e) => return reject(StatusCode::BAD_REQUEST, &format!("bad messages: {e}")),
    };
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let engine = gw.inner.engine.clone();
    tokio::spawn(async move {
        let events = tx.clone();
        let result = engine
            .chat(
                &messages,
                &SamplingOptions::default(),
                move |e: ChatEvent| {
                    let _ = events.send(json!({ "event": e }));
                },
            )
            .await;
        let _ = tx.send(match result {
            Ok(answer) => json!({ "done": answer }),
            Err(e) => json!({ "error": e.to_string() }),
        });
    });

    let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
    let body = futures_util::StreamExt::map(stream, move |frame| {
        let env = gw
            .inner
            .keys
            .seal(&device_key, frame.to_string().as_bytes());
        let mut line = serde_json::to_vec(&env).expect("envelope serializes");
        line.push(b'\n');
        Ok::<_, std::convert::Infallible>(line)
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(body))
        .unwrap()
}
