//! The phone gateway: an HTTP server on the local network that paired phones
//! use to chat with the engine. Every request and response after pairing is
//! end-to-end encrypted with the phone's key, so nothing readable crosses the
//! network even though the transport is plain HTTP.
//!
//! It also serves the web chat (`apps/web`), so any browser on the network
//! can pair and chat without installing an app.

pub mod crypto;
pub mod devices;
pub mod relay;
mod server;
mod web;

pub use devices::Device;
pub use relay::RelayStatus;
pub use server::{Gateway, GatewayEvent, GatewayStatus, PairingOffer, DEFAULT_PORT};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
