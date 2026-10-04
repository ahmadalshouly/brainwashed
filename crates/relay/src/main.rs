//! `brainwashed-relay [address]`: runs a relay. The address defaults to
//! `0.0.0.0:$PORT`, or `0.0.0.0:8080` without `PORT`. Put it behind a reverse
//! proxy that terminates TLS, so hosts and devices connect over HTTPS.

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let addr = std::env::args().nth(1).unwrap_or_else(|| {
        let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
        format!("0.0.0.0:{port}")
    });
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("BrainWashed relay listening on {}", listener.local_addr()?);
    tokio::select! {
        r = brainwashed_relay::serve(listener) => r,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}
