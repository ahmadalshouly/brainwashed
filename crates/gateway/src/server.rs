use crate::audit::{AuditEntry, AuditLog};
use crate::crypto::{self, Envelope, HostKeys};
use crate::devices::{Device, DeviceRole, DeviceStore};
use crate::relay::{self, RelayClient, RelayStatus};
use crate::tunnel::{Tunnel, TunnelOptions, TunnelSpec, TunnelStatus};
use crate::{Error, Result};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use brainwashed_core::{ChatEvent, ChatMessage, Engine, Event, RemoteAccess, SamplingOptions};
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
/// Largest encrypted call. Pictures and documents travel base64 encoded
/// inside the encrypted envelope, which is base64 encoded again.
pub const MAX_RPC_BODY: usize = 48 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOffer {
    /// App link: `brainwashed://pair?…`.
    pub url: String,
    /// The same pairing details as a link to the web app this gateway
    /// serves, carried in the URL fragment so they never reach the network.
    /// Phone cameras open it in the browser, and the app accepts it too, so
    /// it is what the QR code encodes. It uses the public address when there
    /// is one, so it works from anywhere, and the local network otherwise.
    pub web_url: String,
    /// The link on the local network, when this computer is on one.
    pub lan_url: Option<String>,
    /// The link through the public address, when there is one.
    pub public_url: Option<String>,
    /// The part after `#pair?`, to build a link to another address.
    pub query: String,
    pub role: DeviceRole,
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
    /// Remote access through a relay, when one is set.
    pub relay: Option<RelayStatus>,
    /// Remote access through a Cloudflare tunnel, when it's on.
    pub tunnel: Option<TunnelStatus>,
    /// Where devices reach this computer from anywhere: the configured
    /// public address, or the quick tunnel's address once it's connected.
    pub public_url: Option<String>,
}

/// A model download in progress or just finished, for the admin page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStatus {
    pub repo: String,
    pub done: u64,
    pub total: Option<u64>,
    pub finished: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GatewayEvent {
    DevicePaired { device: Device },
    DevicesChanged,
}

#[derive(Clone)]
pub struct Gateway {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) engine: Engine,
    pub(crate) keys: HostKeys,
    pub(crate) devices: DeviceStore,
    pub(crate) audit: AuditLog,
    data_dir: std::path::PathBuf,
    /// One-time pairing tokens: expiry (unix seconds) and the role they grant.
    offers: Mutex<HashMap<String, (u64, DeviceRole)>>,
    /// Nonces of recent requests with their arrival time (ms), oldest first.
    seen_nonces: Mutex<VecDeque<(u64, String)>>,
    running: Mutex<Option<Running>>,
    /// The relay address, and the live connection while the gateway runs.
    relay_url: Mutex<Option<String>>,
    relay: Mutex<Option<RelayClient>>,
    /// The tunnel to run while the gateway runs, and the running one.
    tunnel_spec: Mutex<Option<TunnelSpec>>,
    tunnel: Mutex<Option<Tunnel>>,
    cloudflared: Mutex<Option<std::path::PathBuf>>,
    /// A stable public address set by the user.
    public_url: Mutex<Option<String>>,
    pub(crate) downloads: Mutex<Vec<DownloadStatus>>,
    /// Lets the `brainwashed` command on this computer talk to a running
    /// server. Written to `gateway/control.token`, readable only by this user.
    pub(crate) control_token: String,
    /// Set when `brainwashed stop` asks the server to quit.
    pub(crate) stop_requested: tokio::sync::Notify,
    events: broadcast::Sender<GatewayEvent>,
}

struct Running {
    addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
}

impl Gateway {
    pub fn new(engine: Engine, data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("gateway");
        let gateway = Gateway {
            inner: Arc::new(Inner {
                keys: HostKeys::load_or_create(&dir.join("host.key"))?,
                devices: DeviceStore::open(&dir.join("devices.json"))?,
                audit: AuditLog::new(&dir.join("audit.jsonl")),
                data_dir: data_dir.to_path_buf(),
                engine,
                offers: Mutex::new(HashMap::new()),
                seen_nonces: Mutex::new(VecDeque::new()),
                running: Mutex::new(None),
                relay_url: Mutex::new(None),
                relay: Mutex::new(None),
                tunnel_spec: Mutex::new(None),
                tunnel: Mutex::new(None),
                cloudflared: Mutex::new(None),
                public_url: Mutex::new(None),
                downloads: Mutex::new(Vec::new()),
                control_token: crypto::random_token(),
                stop_requested: tokio::sync::Notify::new(),
                events: broadcast::channel(32).0,
            }),
        };
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(track_downloads(
                Arc::downgrade(&gateway.inner),
                gateway.inner.engine.subscribe(),
            ));
        }
        Ok(gateway)
    }

    /// The token the `brainwashed` command uses to reach this running server,
    /// stored where only this user can read it.
    pub fn control_token_path(data_dir: &Path) -> std::path::PathBuf {
        data_dir.join("gateway").join("control.token")
    }

    /// Applies the remote access settings (relay, tunnel, public address)
    /// from the engine's settings. Errors name the setting that's wrong; the
    /// others still apply.
    pub fn apply_settings(&self) -> Result<()> {
        let settings = self.inner.engine.settings();
        *self.inner.cloudflared.lock().unwrap() = settings.cloudflared_path.clone();
        let mut problems = Vec::new();
        if let Err(e) = self.set_relay_url(settings.relay_url.as_deref()) {
            problems.push(format!("relay: {e}"));
        }
        if let Err(e) = self.set_public_url(settings.public_url.as_deref()) {
            problems.push(format!("public address: {e}"));
        }
        let spec = match settings.remote_access {
            RemoteAccess::Off => None,
            RemoteAccess::Quick => Some(TunnelSpec::Quick),
            RemoteAccess::Cloudflare => {
                match settings.tunnel_token.filter(|t| !t.trim().is_empty()) {
                    Some(token) => Some(TunnelSpec::Named {
                        token: token.trim().to_string(),
                    }),
                    None => {
                        problems.push("Cloudflare tunnel: add the tunnel token".into());
                        None
                    }
                }
            }
        };
        self.set_tunnel(spec);
        if problems.is_empty() {
            Ok(())
        } else {
            Err(Error::Invalid(problems.join("; ")))
        }
    }

    /// Sets or clears the tunnel. While the gateway runs, it keeps it up.
    pub fn set_tunnel(&self, spec: Option<TunnelSpec>) {
        let changed = {
            let mut current = self.inner.tunnel_spec.lock().unwrap();
            let changed = *current != spec;
            *current = spec;
            changed
        };
        let stopped = self.inner.tunnel.lock().unwrap().is_none();
        if changed || stopped {
            self.restart_tunnel();
        }
    }

    fn restart_tunnel(&self) {
        let mut tunnel = self.inner.tunnel.lock().unwrap();
        tunnel.take();
        let port = self
            .inner
            .running
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.addr.port());
        if let (Some(port), Some(spec)) = (port, self.inner.tunnel_spec.lock().unwrap().clone()) {
            *tunnel = Some(Tunnel::start(
                spec,
                port,
                TunnelOptions {
                    binary: self.inner.cloudflared.lock().unwrap().clone(),
                    bin_dir: self.inner.data_dir.join("bin"),
                },
            ));
        }
    }

    /// Sets the stable address this computer is reachable at from anywhere.
    pub fn set_public_url(&self, url: Option<&str>) -> Result<()> {
        let url = url
            .filter(|u| !u.trim().is_empty())
            .map(relay::normalize_url)
            .transpose()
            .map_err(|e| Error::Invalid(e.replace("relay address", "address")))?;
        *self.inner.public_url.lock().unwrap() = url;
        Ok(())
    }

    /// Where devices reach this computer from anywhere, if anywhere.
    pub fn public_url(&self) -> Option<String> {
        let configured = self.inner.public_url.lock().unwrap().clone();
        configured.or_else(|| {
            self.tunnel_status()
                .filter(|s| s.connected)
                .and_then(|s| s.url)
        })
    }

    /// Resolves when `brainwashed stop` asks this server to quit.
    pub async fn stop_requested(&self) {
        self.inner.stop_requested.notified().await
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub(crate) fn audit(
        &self,
        device: Option<&Device>,
        action: &str,
        target: Option<&str>,
        error: Option<&str>,
    ) {
        self.inner.audit.record(&AuditEntry {
            at: now_secs(),
            device_id: device.map(|d| d.id.clone()),
            device_name: device.map(|d| d.name.clone()),
            action: action.to_string(),
            target: target.map(str::to_string),
            error: error.map(str::to_string),
        });
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
        crypto::write_private(
            &Self::control_token_path(&self.inner.data_dir),
            self.inner.control_token.as_bytes(),
        )?;
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
        self.restart_relay();
        self.restart_tunnel();
        Ok(addr)
    }

    pub fn stop(&self) {
        if let Some(r) = self.inner.running.lock().unwrap().take() {
            let _ = r.shutdown.send(());
            let _ = std::fs::remove_file(Self::control_token_path(&self.inner.data_dir));
        }
        self.inner.relay.lock().unwrap().take();
        self.inner.tunnel.lock().unwrap().take();
    }

    /// Sets or clears the relay that devices away from home connect
    /// through. While the gateway runs, the host stays connected to it.
    pub fn set_relay_url(&self, url: Option<&str>) -> Result<()> {
        let url = url
            .filter(|u| !u.trim().is_empty())
            .map(relay::normalize_url)
            .transpose()
            .map_err(Error::Invalid)?;
        *self.inner.relay_url.lock().unwrap() = url;
        self.restart_relay();
        Ok(())
    }

    fn restart_relay(&self) {
        let mut relay = self.inner.relay.lock().unwrap();
        relay.take();
        let running = self.inner.running.lock().unwrap().is_some();
        if let (true, Some(url)) = (running, self.inner.relay_url.lock().unwrap().clone()) {
            *relay = Some(RelayClient::start(
                url,
                self.inner.keys.clone(),
                self.router(),
            ));
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
            relay: self.relay_status(),
            tunnel: self.tunnel_status(),
            public_url: self.public_url(),
        }
    }

    fn tunnel_status(&self) -> Option<TunnelStatus> {
        let tunnel = self.inner.tunnel.lock().unwrap();
        tunnel.as_ref().map(|t| t.status())
    }

    fn relay_status(&self) -> Option<RelayStatus> {
        let url = self.inner.relay_url.lock().unwrap().clone()?;
        Some(
            self.inner
                .relay
                .lock()
                .unwrap()
                .as_ref()
                .map(|r| r.status())
                .unwrap_or(RelayStatus {
                    url,
                    connected: false,
                    error: None,
                }),
        )
    }

    /// A one-time pairing link for the QR code, granting `role` to the device
    /// that uses it. The gateway must be running.
    pub fn create_pairing_offer(&self, role: DeviceRole) -> Result<PairingOffer> {
        let port = self
            .status()
            .port
            .ok_or_else(|| Error::Invalid("the server isn't running".into()))?;
        let token = crypto::random_token();
        let expires_at = now_secs() + OFFER_TTL.as_secs();
        {
            let mut offers = self.inner.offers.lock().unwrap();
            let now = now_secs();
            offers.retain(|_, (exp, _)| *exp > now);
            offers.insert(token.clone(), (expires_at, role));
        }
        let addresses = lan_addresses();
        let addr_list = addresses
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut query = format!(
            "v={PROTOCOL_VERSION}&k={}&t={token}&a={addr_list}&p={port}&n={}",
            self.inner.keys.public_key_b64url(),
            percent_encode(&self.inner.engine.host_name()),
        );
        if let Some(url) = self.inner.relay_url.lock().unwrap().as_deref() {
            query.push_str(&format!("&r={}", percent_encode(url)));
        }
        let public = self.public_url();
        if let Some(url) = public.as_deref() {
            query.push_str(&format!("&u={}", percent_encode(url)));
        }
        let lan_url = addresses
            .first()
            .map(|a| format!("http://{a}:{port}/#pair?{query}"));
        let public_url = public.map(|u| format!("{u}/#pair?{query}"));
        let web_url = public_url
            .clone()
            .or_else(|| lan_url.clone())
            .unwrap_or_else(|| format!("http://localhost:{port}/#pair?{query}"));
        Ok(PairingOffer {
            url: format!("brainwashed://pair?{query}"),
            web_url,
            lan_url,
            public_url,
            query,
            role,
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

    pub(crate) fn devices_changed(&self) {
        let _ = self.inner.events.send(GatewayEvent::DevicesChanged);
    }

    pub(crate) fn router(&self) -> Router {
        Router::new()
            .route("/hello", get(hello))
            .route("/pair", post(pair))
            // Room for pictures and documents attached to a chat.
            .route("/rpc", post(rpc).layer(DefaultBodyLimit::max(MAX_RPC_BODY)))
            .route("/control/status", get(crate::control::status))
            .route("/control/admin-link", post(crate::control::admin_link))
            .route("/control/stop", post(crate::control::stop))
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

pub(crate) fn now_secs() -> u64 {
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

/// Private IPv4 addresses a phone on the same network could reach, most
/// likely first. The web link in the QR code uses the first one.
pub fn lan_addresses() -> Vec<IpAddr> {
    let candidates = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && i.is_oper_up())
        .map(|i| (i.name.clone(), i.ip()));
    rank_addresses(candidates)
}

/// Orders interface addresses so the home Wi-Fi or Ethernet address comes
/// first. Windows in particular lists virtual adapters (WSL, Hyper-V, Docker,
/// VirtualBox, VPNs) and self-assigned 169.254 addresses next to the real one.
fn rank_addresses(candidates: impl IntoIterator<Item = (String, IpAddr)>) -> Vec<IpAddr> {
    let mut ranked: Vec<(u8, IpAddr)> = candidates
        .into_iter()
        .filter_map(|(name, ip)| match ip {
            IpAddr::V4(v4) if v4.is_private() || v4.is_link_local() => {
                let range = match v4.octets() {
                    _ if v4.is_link_local() => 6,
                    [192, 168, ..] => 0,
                    [10, ..] => 1,
                    _ => 2,
                };
                // Virtual adapters rank after every real one except 169.254.
                let rank = if is_virtual_adapter(&name) {
                    3 + range.min(2)
                } else {
                    range
                };
                Some((rank, ip))
            }
            _ => None,
        })
        .collect();
    ranked.sort();
    let mut addrs: Vec<IpAddr> = Vec::new();
    for (_, ip) in ranked {
        if !addrs.contains(&ip) {
            addrs.push(ip);
        }
    }
    addrs
}

fn is_virtual_adapter(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "vethernet",
        "wsl",
        "hyper-v",
        "docker",
        "virtualbox",
        "vboxnet",
        "vmware",
        "vmnet",
        "br-",
        "veth",
        "virbr",
        "tailscale",
        "zerotier",
        "utun",
        "tun",
        "tap",
        "wireguard",
        "vpn",
    ]
    .iter()
    .any(|v| name.contains(v))
}

pub(crate) fn percent_encode(s: &str) -> String {
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
    let role = {
        let mut offers = gw.inner.offers.lock().unwrap();
        // Single use: the token is consumed even if it has expired.
        match offers.remove(&secret.token) {
            Some((exp, role)) if exp > now_secs() => Some(role),
            _ => None,
        }
    };
    let Some(role) = role else {
        return reject(
            StatusCode::UNAUTHORIZED,
            "this pairing code has expired or was already used; show a new QR code on the computer",
        );
    };

    let device = Device {
        id: crypto::random_token(),
        name: secret.device_name.chars().take(64).collect(),
        public_key: req.device_public_key.clone(),
        paired_at: now_secs(),
        last_seen: Some(now_secs()),
        role,
    };
    if let Err(e) = gw.inner.devices.add(device.clone()) {
        return reject(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let _ = gw.inner.events.send(GatewayEvent::DevicePaired {
        device: device.clone(),
    });
    tracing::info!("paired `{}` as {:?}", device.name, device.role);
    gw.audit(
        Some(&device),
        "pair",
        Some(if role == DeviceRole::Admin {
            "admin"
        } else {
            "member"
        }),
        None,
    );

    let body = json!({
        "deviceId": device.id,
        "hostId": gw.inner.keys.host_id(),
        "hostName": gw.inner.engine.host_name(),
        "role": device.role,
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
        let admin = device.role == DeviceRole::Admin;
        return chat(gw, device_key, admin, call.params);
    }
    let result = crate::admin::handle(&gw, &device, &call.method, call.params).await;
    let body = match result {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    };
    Json(gw.inner.keys.seal(&device_key, body.to_string().as_bytes())).into_response()
}

/// One encrypted JSON frame per line: `{"event":…}` while streaming, then
/// `{"done":"full answer"}` or `{"error":"…"}`.
fn chat(gw: Gateway, device_key: crypto_box::PublicKey, admin: bool, params: Value) -> Response {
    let messages: Vec<ChatMessage> = match serde_json::from_value(params["messages"].clone()) {
        Ok(m) => m,
        Err(e) => return reject(StatusCode::BAD_REQUEST, &format!("bad messages: {e}")),
    };
    let options: SamplingOptions = if params["options"].is_null() {
        SamplingOptions::default()
    } else {
        match serde_json::from_value(params["options"].clone()) {
            Ok(o) => o,
            Err(e) => return reject(StatusCode::BAD_REQUEST, &format!("bad options: {e}")),
        }
    };
    // `local` or `<provider>/<model>`; the local model when left out.
    let model = params["model"].as_str().map(str::to_string);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let engine = gw.inner.engine.clone();
    tokio::spawn(async move {
        let events = tx.clone();
        let result = engine
            .chat_with(
                &messages,
                &options,
                model.as_deref(),
                admin,
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

/// Keeps the latest progress of each model download for the admin page.
async fn track_downloads(inner: std::sync::Weak<Inner>, mut events: broadcast::Receiver<Event>) {
    loop {
        let event = match events.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let Some(inner) = inner.upgrade() else { return };
        let mut downloads = inner.downloads.lock().unwrap();
        match event {
            Event::DownloadProgress { repo, done, total } => {
                let d = download_entry(&mut downloads, &repo);
                d.done = done;
                d.total = total;
                d.finished = false;
                d.error = None;
            }
            Event::DownloadFinished { repo, .. } => {
                download_entry(&mut downloads, &repo).finished = true;
            }
            Event::DownloadFailed { repo, error } => {
                let d = download_entry(&mut downloads, &repo);
                d.finished = true;
                d.error = Some(error);
            }
            _ => {}
        }
    }
}

pub(crate) fn download_entry<'a>(
    downloads: &'a mut Vec<DownloadStatus>,
    repo: &str,
) -> &'a mut DownloadStatus {
    let i = match downloads.iter().position(|d| d.repo == repo) {
        Some(i) => i,
        None => {
            downloads.push(DownloadStatus {
                repo: repo.to_string(),
                done: 0,
                total: None,
                finished: false,
                error: None,
            });
            downloads.len() - 1
        }
    };
    &mut downloads[i]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn home_network_address_comes_first() {
        let ranked = rank_addresses([
            ("vEthernet (WSL)".to_string(), ip("172.25.48.1")),
            ("Ethernet 2".to_string(), ip("169.254.12.7")),
            ("Wi-Fi".to_string(), ip("192.168.1.42")),
            (
                "VirtualBox Host-Only Network".to_string(),
                ip("192.168.56.1"),
            ),
            ("Wi-Fi".to_string(), ip("fe80::1")),
            ("Wi-Fi".to_string(), ip("8.8.8.8")),
        ]);
        assert_eq!(
            ranked,
            [
                ip("192.168.1.42"),
                ip("192.168.56.1"),
                ip("172.25.48.1"),
                ip("169.254.12.7")
            ]
        );
    }

    #[test]
    fn keeps_office_style_networks() {
        let ranked = rank_addresses([
            ("eth0".to_string(), ip("172.20.0.5")),
            ("docker0".to_string(), ip("172.17.0.1")),
            ("wlan0".to_string(), ip("10.0.0.8")),
        ]);
        assert_eq!(ranked, [ip("10.0.0.8"), ip("172.20.0.5"), ip("172.17.0.1")]);
    }
}
