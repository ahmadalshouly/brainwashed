use brainwashed_core::{Engine, EngineConfig};
use brainwashed_gateway::crypto::{parse_public_key, Envelope, HostKeys};
use brainwashed_gateway::{DeviceRole, Gateway};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// A phone, as far as the protocol is concerned: its own NaCl key pair.
struct Phone {
    keys: HostKeys,
    _dir: tempfile::TempDir,
    host_key: crypto_box::PublicKey,
    base: String,
    device_id: Option<String>,
    http: reqwest::Client,
}

fn query(url: &str, key: &str) -> String {
    url.split(['?', '&'])
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
        .unwrap()
        .to_string()
}

impl Phone {
    fn from_offer(url: &str, port: u16) -> Phone {
        let dir = tempfile::tempdir().unwrap();
        Phone {
            keys: HostKeys::load_or_create(&dir.path().join("phone.key")).unwrap(),
            _dir: dir,
            host_key: parse_public_key(&query(url, "k")).unwrap(),
            base: format!("http://127.0.0.1:{port}"),
            device_id: None,
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    fn public_key_b64(&self) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(self.keys.public_key().as_bytes())
    }

    async fn pair(&mut self, token: &str) -> Result<Value, (u16, Value)> {
        let env = self.keys.seal(
            &self.host_key,
            json!({ "token": token, "deviceName": "Test Phone" })
                .to_string()
                .as_bytes(),
        );
        let res = self
            .http
            .post(format!("{}/pair", self.base))
            .json(&json!({ "devicePublicKey": self.public_key_b64(), "n": env.n, "c": env.c }))
            .send()
            .await
            .unwrap();
        let status = res.status().as_u16();
        let body: Value = res.json().await.unwrap();
        if status != 200 {
            return Err((status, body));
        }
        let env: Envelope = serde_json::from_value(body).unwrap();
        let reply: Value =
            serde_json::from_slice(&self.keys.open(&self.host_key, &env).unwrap()).unwrap();
        self.device_id = Some(reply["deviceId"].as_str().unwrap().to_string());
        Ok(reply)
    }

    fn sealed_call(&self, method: &str, params: Value, ts: u64) -> Value {
        let env = self.keys.seal(
            &self.host_key,
            json!({ "ts": ts, "method": method, "params": params })
                .to_string()
                .as_bytes(),
        );
        json!({ "deviceId": self.device_id, "n": env.n, "c": env.c })
    }

    async fn post(&self, body: &Value) -> (u16, String) {
        let res = self
            .http
            .post(format!("{}/rpc", self.base))
            .json(body)
            .send()
            .await
            .unwrap();
        (res.status().as_u16(), res.text().await.unwrap())
    }

    fn open_line(&self, line: &str) -> Value {
        let env: Envelope = serde_json::from_str(line).unwrap();
        serde_json::from_slice(&self.keys.open(&self.host_key, &env).unwrap()).unwrap()
    }

    async fn call(&self, method: &str, params: Value) -> Value {
        let (status, text) = self.post(&self.sealed_call(method, params, now_ms())).await;
        assert_eq!(status, 200, "{text}");
        self.open_line(&text)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

async fn setup() -> (Gateway, String, u16, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(EngineConfig::new(dir.path(), "0.1.0")).unwrap();
    let gw = Gateway::new(engine, dir.path()).unwrap();
    let port = gw.start(0).await.unwrap().port();
    let offer = gw.create_pairing_offer(DeviceRole::Admin).unwrap();
    (gw, offer.url, port, dir)
}

#[tokio::test]
async fn pairs_and_answers_encrypted_calls() {
    let (gw, url, port, _dir) = setup().await;
    let mut events = gw.subscribe();
    let mut phone = Phone::from_offer(&url, port);

    let reply = phone.pair(&query(&url, "t")).await.unwrap();
    assert_eq!(reply["hostId"], gw.status().host_id);
    assert_eq!(gw.devices().len(), 1);
    assert_eq!(gw.devices()[0].name, "Test Phone");
    assert!(matches!(
        events.try_recv().unwrap(),
        brainwashed_gateway::GatewayEvent::DevicePaired { .. }
    ));

    let info = phone.call("info", Value::Null).await;
    assert_eq!(info["ok"]["version"], "0.1.0");
    let skills = phone.call("skills", Value::Null).await;
    assert_eq!(skills["ok"].as_array().unwrap().len(), 2);
    let unknown = phone.call("nope", Value::Null).await;
    assert!(unknown["error"]
        .as_str()
        .unwrap()
        .contains("unknown method"));
}

#[tokio::test]
async fn pairing_codes_are_single_use() {
    let (_gw, url, port, _dir) = setup().await;
    let token = query(&url, "t");
    Phone::from_offer(&url, port).pair(&token).await.unwrap();
    let (status, body) = Phone::from_offer(&url, port)
        .pair(&token)
        .await
        .unwrap_err();
    assert_eq!(status, 401);
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("expired or was already used"));
}

#[tokio::test]
async fn wrong_host_key_cannot_pair() {
    let (_gw, url, port, _dir) = setup().await;
    let mut phone = Phone::from_offer(&url, port);
    // As if a different machine had answered at that address.
    phone.host_key = HostKeys::load_or_create(&phone._dir.path().join("x"))
        .unwrap()
        .public_key();
    let (status, _) = phone.pair(&query(&url, "t")).await.unwrap_err();
    assert_eq!(status, 401);
}

#[tokio::test]
async fn rejects_unpaired_replayed_and_stale_requests() {
    let (gw, url, port, _dir) = setup().await;
    let mut phone = Phone::from_offer(&url, port);
    phone.pair(&query(&url, "t")).await.unwrap();

    // Replay of a captured request.
    let call = phone.sealed_call("info", Value::Null, now_ms());
    assert_eq!(phone.post(&call).await.0, 200);
    assert_eq!(phone.post(&call).await.0, 401);

    // Clock far off (or a very old captured request).
    let stale = phone.sealed_call("info", Value::Null, now_ms() - 60 * 60 * 1000);
    assert_eq!(phone.post(&stale).await.0, 401);

    // Removed phones are locked out.
    gw.remove_device(phone.device_id.as_deref().unwrap())
        .unwrap();
    let call = phone.sealed_call("info", Value::Null, now_ms());
    assert_eq!(phone.post(&call).await.0, 401);
}

#[tokio::test]
async fn chat_streams_encrypted_frames() {
    let (_gw, url, port, _dir) = setup().await;
    let mut phone = Phone::from_offer(&url, port);
    phone.pair(&query(&url, "t")).await.unwrap();

    // No model is loaded, so the stream ends with an error frame.
    let call = phone.sealed_call(
        "chat",
        json!({ "messages": [{ "role": "user", "content": "hi" }] }),
        now_ms(),
    );
    let (status, text) = phone.post(&call).await;
    assert_eq!(status, 200);
    let frames: Vec<Value> = text.lines().map(|l| phone.open_line(l)).collect();
    assert!(frames.last().unwrap()["error"]
        .as_str()
        .unwrap()
        .contains("no model"));
}

#[tokio::test]
async fn offers_need_a_running_gateway_and_stop_works() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::new(EngineConfig::new(dir.path(), "0.1.0")).unwrap();
    let gw = Gateway::new(engine, dir.path()).unwrap();
    assert!(gw.create_pairing_offer(DeviceRole::Admin).is_err());
    let addr = gw.start(0).await.unwrap();
    assert!(gw.status().running);
    gw.stop();
    assert!(!gw.status().running);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let refused = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("http://127.0.0.1:{}/hello", addr.port()))
        .send()
        .await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn serves_the_web_chat_with_a_browser_pairing_link() {
    let (gw, url, port, _dir) = setup().await;
    let offer = gw.create_pairing_offer(DeviceRole::Admin).unwrap();
    // Same details as the app link, in the fragment so they stay in the browser.
    let (base, fragment) = offer.web_url.split_once("#pair?").unwrap();
    assert!(base.starts_with("http://") && base.ends_with(&format!(":{port}/")));
    assert_eq!(query(&format!("x?{fragment}"), "k"), query(&url, "k"));

    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for path in ["/", "/chat"] {
        let res = client
            .get(format!("http://127.0.0.1:{port}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        assert!(res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        let csp = res.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("connect-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
    }
    let missing = client
        .get(format!("http://127.0.0.1:{port}/assets/nope.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

async fn start_relay() -> (brainwashed_relay::Relay, String) {
    let relay = brainwashed_relay::Relay::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = relay.router();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (relay, url)
}

async fn wait_for_relay(gw: &Gateway) {
    for _ in 0..100 {
        if gw.status().relay.is_some_and(|r| r.connected) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("never connected to the relay: {:?}", gw.status().relay);
}

#[tokio::test]
async fn works_through_the_relay() {
    let (relay, relay_url) = start_relay().await;
    let (gw, _, _, _dir) = setup().await;
    gw.set_relay_url(Some(&relay_url)).unwrap();
    wait_for_relay(&gw).await;

    // The pairing link tells devices where the relay is.
    let url = gw.create_pairing_offer(DeviceRole::Admin).unwrap().url;
    let r = query(&url, "r").replace("%3A", ":").replace("%2F", "/");
    assert_eq!(r, relay_url);
    let host_key = query(&url, "k");
    assert!(relay.is_connected(&host_key));

    // Same requests as on the local network, under /h/<host key>.
    let mut phone = Phone::from_offer(&url, 0);
    phone.base = format!("{relay_url}/h/{host_key}");
    phone.pair(&query(&url, "t")).await.unwrap();
    let info = phone.call("info", Value::Null).await;
    assert_eq!(info["ok"]["version"], "0.1.0");

    // Replays are still refused: the host does the checking, not the relay.
    let call = phone.sealed_call("info", Value::Null, now_ms());
    assert_eq!(phone.post(&call).await.0, 200);
    assert_eq!(phone.post(&call).await.0, 401);

    // Streams come through frame by frame.
    let call = phone.sealed_call(
        "chat",
        json!({ "messages": [{ "role": "user", "content": "hi" }] }),
        now_ms(),
    );
    let (status, text) = phone.post(&call).await;
    assert_eq!(status, 200);
    let frames: Vec<Value> = text.lines().map(|l| phone.open_line(l)).collect();
    assert!(frames.last().unwrap()["error"]
        .as_str()
        .unwrap()
        .contains("no model"));

    // The web chat is only served on the local network.
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let page = client
        .get(format!("{relay_url}/h/{host_key}/"))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 404);

    // Turning access off disconnects from the relay.
    gw.stop();
    for _ in 0..100 {
        if !relay.is_connected(&host_key) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!relay.is_connected(&host_key));
    let (status, text) = phone
        .post(&phone.sealed_call("info", Value::Null, now_ms()))
        .await;
    assert_eq!(status, 503, "{text}");
    assert!(text.contains("offline"));
}

#[tokio::test]
async fn relay_settings_are_checked() {
    let (gw, _, _, _dir) = setup().await;
    assert!(gw.set_relay_url(Some("relay.example.org")).is_err());
    gw.set_relay_url(Some("https://relay.example.org/"))
        .unwrap();
    assert_eq!(gw.status().relay.unwrap().url, "https://relay.example.org");
    gw.set_relay_url(None).unwrap();
    assert!(gw.status().relay.is_none());
    assert!(!gw
        .create_pairing_offer(DeviceRole::Admin)
        .unwrap()
        .url
        .contains("&r="));
}

#[tokio::test]
async fn members_chat_and_admins_manage() {
    let (_gw, admin_url, port, dir) = setup().await;
    let mut admin = Phone::from_offer(&admin_url, port);
    let reply = admin.pair(&query(&admin_url, "t")).await.unwrap();
    assert_eq!(reply["role"], "admin");

    // An admin invites a member.
    let invite = admin
        .call("createPairingOffer", json!({ "role": "member" }))
        .await;
    let member_url = invite["ok"]["url"].as_str().unwrap().to_string();
    assert_eq!(invite["ok"]["role"], "member");
    let mut member = Phone::from_offer(&member_url, port);
    assert_eq!(
        member.pair(&query(&member_url, "t")).await.unwrap()["role"],
        "member"
    );

    // Members can chat and look, not change things.
    assert_eq!(
        member.call("whoami", Value::Null).await["ok"]["role"],
        "member"
    );
    assert!(member.call("models", Value::Null).await["ok"].is_array());
    // Anyone can attach documents, which the computer reads.
    let doc = member
        .call(
            "readDocument",
            json!({ "name": "notes.txt", "data": "YnV5IG1pbGsK" }),
        )
        .await;
    assert_eq!(doc["ok"]["text"], "buy milk", "{doc}");
    let bad = member
        .call("readDocument", json!({ "name": "a.bin", "data": "AAEC" }))
        .await;
    assert!(
        bad["error"].as_str().unwrap().contains("can't read"),
        "{bad}"
    );
    // Bigger than axum's default body limit.
    let big = member
        .call(
            "readDocument",
            json!({ "name": "big.txt", "data": "YWFh".repeat(1_000_000) }),
        )
        .await;
    assert_eq!(
        big["ok"]["truncated"],
        true,
        "{}",
        &big.to_string()[..200.min(big.to_string().len())]
    );
    for (method, params) in [
        ("settings", Value::Null),
        ("devices", Value::Null),
        ("deleteSkill", json!({ "name": "writing-assistant" })),
        ("loadModel", json!({ "id": "x" })),
    ] {
        let denied = member.call(method, params).await;
        assert!(
            denied["error"].as_str().unwrap().contains("Only admins"),
            "{method}: {denied}"
        );
    }

    // Admins manage skills, settings and devices.
    let source = admin
        .call("skillSource", json!({ "name": "writing-assistant" }))
        .await;
    assert!(source["ok"].as_str().unwrap().contains("name:"));
    let settings = admin
        .call(
            "updateSettings",
            json!({ "settings": { "host_name": "Office AI", "remote_access": "off" } }),
        )
        .await;
    assert_eq!(settings["ok"]["host_name"], "Office AI", "{settings}");
    assert_eq!(settings["ok"]["tunnel_token"], Value::Null);

    // Chat defaults set by an admin reach every device.
    let saved = admin
        .call(
            "updateSettings",
            json!({ "settings": { "chat_defaults": { "temperature": 0.3, "reasoning": false } } }),
        )
        .await;
    assert_eq!(saved["ok"]["chat_defaults"]["temperature"], 0.3, "{saved}");
    let defaults = member.call("chatDefaults", Value::Null).await;
    assert_eq!(
        defaults["ok"],
        json!({ "temperature": 0.3, "reasoning": false })
    );
    let bad = admin
        .call(
            "updateSettings",
            json!({ "settings": { "chat_defaults": { "temperature": 7 } } }),
        )
        .await;
    assert!(
        bad["error"].as_str().unwrap().contains("temperature"),
        "{bad}"
    );
    let devices = admin.call("devices", Value::Null).await;
    let devices = devices["ok"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    let member_id = devices.iter().find(|d| d["role"] == "member").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    admin
        .call("setDeviceRole", json!({ "id": member_id, "role": "admin" }))
        .await;
    assert!(member.call("settings", Value::Null).await["ok"].is_object());

    // The last admin can't be demoted.
    let me = admin.device_id.clone().unwrap();
    admin
        .call(
            "setDeviceRole",
            json!({ "id": member_id, "role": "member" }),
        )
        .await;
    let refused = admin
        .call("setDeviceRole", json!({ "id": me, "role": "member" }))
        .await;
    assert!(refused["error"]
        .as_str()
        .unwrap()
        .contains("at least one admin"));

    // Changes are in the audit log, newest first.
    let log = admin.call("auditLog", json!({ "limit": 50 })).await;
    let actions: Vec<&str> = log["ok"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions[0], "setDeviceRole");
    assert!(actions.contains(&"updateSettings"));
    assert!(actions.contains(&"pair"));
    drop(dir);
}

#[tokio::test]
async fn the_terminal_gets_admin_links_with_its_token() {
    let (gw, _url, port, dir) = setup().await;
    let token = std::fs::read_to_string(Gateway::control_token_path(dir.path())).unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let link = |auth: Option<String>| {
        let mut req = http.post(format!("http://127.0.0.1:{port}/control/admin-link"));
        if let Some(a) = auth {
            req = req.header("Authorization", a);
        }
        req.send()
    };
    assert_eq!(link(None).await.unwrap().status(), 401);
    assert_eq!(
        link(Some("Bearer wrong".into())).await.unwrap().status(),
        401
    );
    let offer: Value = link(Some(format!("Bearer {token}")))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(offer["role"], "admin");
    assert!(offer["query"].as_str().unwrap().contains("t="));

    gw.stop();
    assert!(!Gateway::control_token_path(dir.path()).exists());
}

#[tokio::test]
async fn pairing_links_carry_the_public_address() {
    let (gw, _url, _port, _dir) = setup().await;
    gw.set_public_url(Some("https://ai.example.org/")).unwrap();
    let offer = gw.create_pairing_offer(DeviceRole::Member).unwrap();
    assert_eq!(query(&offer.url, "u"), "https%3A%2F%2Fai.example.org");
    assert!(offer.web_url.starts_with("https://ai.example.org/#pair?"));
    assert_eq!(
        gw.status().public_url.as_deref(),
        Some("https://ai.example.org")
    );
    assert!(gw.set_public_url(Some("ai.example.org")).is_err());
}

/// A cloud provider with an OpenAI-compatible API that answers "Hello".
async fn fake_provider() -> String {
    use axum::{routing::post, Router};
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            (
                [("content-type", "text/event-stream")],
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: [DONE]\n\n",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/v1")
}

#[tokio::test]
async fn admins_connect_cloud_providers_and_members_use_them() {
    let (_gw, admin_url, port, _dir) = setup().await;
    let mut admin = Phone::from_offer(&admin_url, port);
    admin.pair(&query(&admin_url, "t")).await.unwrap();
    let invite = admin
        .call("createPairingOffer", json!({ "role": "member" }))
        .await;
    let member_url = invite["ok"]["url"].as_str().unwrap().to_string();
    let mut member = Phone::from_offer(&member_url, port);
    member.pair(&query(&member_url, "t")).await.unwrap();

    let base = fake_provider().await;
    let saved = admin
        .call(
            "saveProvider",
            json!({ "provider": {
                "id": "acme", "name": "Acme AI", "baseUrl": base,
                "apiKey": "sk-secret-1234", "models": ["fast", "big"], "members": true
            }}),
        )
        .await;
    assert_eq!(saved["ok"]["keySet"], true, "{saved}");
    assert_eq!(saved["ok"]["keyHint"], "…1234");
    assert!(!saved.to_string().contains("sk-secret"));
    let listed = admin.call("providers", Value::Null).await;
    assert!(!listed.to_string().contains("sk-secret"), "{listed}");

    // Members can't change providers but see their models.
    let denied = member.call("deleteProvider", json!({ "id": "acme" })).await;
    assert!(denied["error"].as_str().unwrap().contains("Only admins"));
    let models = member.call("chatModels", Value::Null).await;
    let ids: Vec<&str> = models["ok"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["acme/fast", "acme/big"]);
    assert_eq!(models["ok"][0]["cloud"], true);

    // A member chats with a cloud model; a model the admin didn't offer is refused.
    for (model, expect) in [("acme/fast", "done"), ("acme/secret-model", "error")] {
        let call = member.sealed_call(
            "chat",
            json!({ "model": model, "messages": [{ "role": "user", "content": "hi" }] }),
            now_ms(),
        );
        let (status, text) = member.post(&call).await;
        assert_eq!(status, 200);
        let last = member.open_line(text.lines().next_back().unwrap());
        assert!(last.get(expect).is_some(), "{model}: {last}");
        if expect == "done" {
            assert_eq!(last["done"], "Hello");
        }
    }

    // Saving without a key keeps the key.
    let resaved = admin
        .call(
            "saveProvider",
            json!({ "provider": { "id": "acme", "name": "Acme", "baseUrl": base, "models": ["fast"], "members": false }}),
        )
        .await;
    assert_eq!(resaved["ok"]["keySet"], true, "{resaved}");
    let models = member.call("chatModels", Value::Null).await;
    assert_eq!(models["ok"], json!([]));
}

#[tokio::test]
async fn a_phone_that_dropped_off_resumes_its_reply() {
    let (_gw, url, port, _dir) = setup().await;
    let mut phone = Phone::from_offer(&url, port);
    phone.pair(&query(&url, "t")).await.unwrap();
    let base = fake_provider().await;
    phone
        .call(
            "saveProvider",
            json!({ "provider": { "id": "acme", "name": "Acme", "baseUrl": base, "apiKey": "k", "models": ["fast"], "members": false }}),
        )
        .await;

    let call = phone.sealed_call(
        "chat",
        json!({ "replyId": "r-1", "model": "acme/fast", "messages": [{ "role": "user", "content": "hi" }] }),
        now_ms(),
    );
    let (_, text) = phone.post(&call).await;
    let frames: Vec<Value> = text.lines().map(|l| phone.open_line(l)).collect();
    assert_eq!(frames.last().unwrap()["done"], "Hello");

    // Having handled only the first frame, the phone gets the rest.
    let resume = |after: usize, id: &str| {
        phone.sealed_call(
            "chatResume",
            json!({ "replyId": id, "after": after }),
            now_ms(),
        )
    };
    let (status, text) = phone.post(&resume(1, "r-1")).await;
    assert_eq!(status, 200);
    let rest: Vec<Value> = text.lines().map(|l| phone.open_line(l)).collect();
    assert_eq!(rest, frames[1..].to_vec());

    // An unknown reply says so.
    let (_, text) = phone.post(&resume(0, "nope")).await;
    let gone = phone.open_line(text.lines().next().unwrap());
    assert!(gone["error"].as_str().unwrap().contains("no longer"));

    // Ids are checked.
    let bad = phone.sealed_call(
        "chat",
        json!({ "replyId": "../x", "messages": [{ "role": "user", "content": "hi" }] }),
        now_ms(),
    );
    assert_eq!(phone.post(&bad).await.0, 400);
}
