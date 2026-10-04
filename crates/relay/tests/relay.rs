use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use brainwashed_relay::proto::{FromHost, ToHost};
use brainwashed_relay::Relay;
use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

async fn start() -> (Relay, String) {
    let relay = Relay::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = relay.router();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (relay, format!("127.0.0.1:{}", addr.port()))
}

/// Connects as a host whose public key is `claimed`, signing with `secret`.
async fn connect_as(addr: &str, claimed: &PublicKey, secret: &SecretKey) -> Option<ToHost> {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/host/connect"))
        .await
        .unwrap();
    let Some(Ok(Message::Text(text))) = ws.next().await else {
        panic!("no challenge")
    };
    let ToHost::Challenge {
        challenge,
        relay_key,
    } = serde_json::from_str(&text).unwrap()
    else {
        panic!("expected a challenge")
    };
    let relay_key: [u8; 32] = STANDARD.decode(relay_key).unwrap().try_into().unwrap();
    let nonce = SalsaBox::generate_nonce(&mut OsRng);
    let c = SalsaBox::new(&PublicKey::from(relay_key), secret)
        .encrypt(&nonce, STANDARD.decode(challenge).unwrap().as_slice())
        .unwrap();
    let auth = FromHost::Auth {
        host_key: URL_SAFE_NO_PAD.encode(claimed.as_bytes()),
        n: STANDARD.encode(nonce),
        c: STANDARD.encode(c),
    };
    ws.send(Message::Text(serde_json::to_string(&auth).unwrap().into()))
        .await
        .unwrap();
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(text))) => return Some(serde_json::from_str(&text).unwrap()),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

#[tokio::test]
async fn a_host_must_hold_its_key() {
    let (relay, addr) = start().await;
    let real = SecretKey::generate(&mut OsRng);
    let impostor = SecretKey::generate(&mut OsRng);
    assert_eq!(connect_as(&addr, &real.public_key(), &impostor).await, None);
    assert_eq!(relay.host_count(), 0);
}

#[tokio::test]
async fn unknown_hosts_are_offline() {
    let (_relay, addr) = start().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let res = client
        .post(format!("http://{addr}/h/nobody/rpc"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 503);
    assert!(res.text().await.unwrap().contains("offline"));
    let index = client.get(format!("http://{addr}/")).send().await.unwrap();
    assert_eq!(index.status(), 200);
}
