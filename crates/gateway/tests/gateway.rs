use brainwashed_core::{Engine, EngineConfig};
use brainwashed_gateway::crypto::{parse_public_key, Envelope, HostKeys};
use brainwashed_gateway::Gateway;
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
    let offer = gw.create_pairing_offer().unwrap();
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
    assert_eq!(skills["ok"].as_array().unwrap().len(), 3);
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
    assert!(gw.create_pairing_offer().is_err());
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
