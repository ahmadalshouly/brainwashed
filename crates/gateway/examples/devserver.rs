//! Runs the engine and gateway on their own, for testing
//! phone clients. Also runs a relay on loopback and connects the gateway to
//! it, as if the client were away from home. Prints one JSON line with a
//! pairing link, then serves.
//!
//! ```sh
//! cargo run -p brainwashed-gateway --example devserver -- /tmp/bw-data [port]
//! ```
//! Set BRAINWASHED_TEST_LLAMA_SERVER and BRAINWASHED_TEST_MODEL to also load a model.
//! Set BRAINWASHED_TEST_PUBLIC_URL and BRAINWASHED_TEST_ADDRESS_BOOK to post a
//! public address to an address book (services/address-book).

use brainwashed_core::{Engine, EngineConfig};
use brainwashed_gateway::{DeviceRole, Gateway};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let data_dir = PathBuf::from(args.next().expect("usage: devserver <data-dir> [port]"));
    let port: u16 = args.next().map(|p| p.parse().expect("port")).unwrap_or(0);

    let engine = Engine::new(EngineConfig::new(&data_dir, env!("CARGO_PKG_VERSION"))).unwrap();
    if let (Ok(server), Ok(model)) = (
        std::env::var("BRAINWASHED_TEST_LLAMA_SERVER"),
        std::env::var("BRAINWASHED_TEST_MODEL"),
    ) {
        let mut s = engine.settings();
        s.llama_server_path = Some(server.into());
        s.gpu_layers = 0;
        s.context_size = 4096;
        engine.update_settings(s).unwrap();
        let m = engine.import_model(std::path::Path::new(&model)).unwrap();
        engine.load_model(&m.id).await.unwrap();
    }

    let book = std::env::var("BRAINWASHED_TEST_ADDRESS_BOOK").ok();
    if let Some(book) = book.clone() {
        let mut s = engine.settings();
        s.remote_access = brainwashed_core::RemoteAccess::Off;
        s.public_url = std::env::var("BRAINWASHED_TEST_PUBLIC_URL").ok();
        s.address_book = Some(book);
        engine.update_settings(s).unwrap();
    }

    let relay = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("http://{}", relay.local_addr().unwrap());
    tokio::spawn(brainwashed_relay::serve(relay));

    let gateway = Gateway::new(engine.clone(), &data_dir).unwrap();
    if book.is_some() {
        gateway.apply_settings().unwrap();
    }
    gateway.set_relay_url(Some(&relay_url)).unwrap();
    let addr = gateway.start(port).await.unwrap();
    while !gateway.status().relay.is_some_and(|r| r.connected) {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let offer = gateway.create_pairing_offer(DeviceRole::Admin).unwrap();
    println!(
        "{}",
        serde_json::json!({ "url": offer.url, "port": addr.port() })
    );

    tokio::signal::ctrl_c().await.ok();
    engine.unload().await.ok();
}
