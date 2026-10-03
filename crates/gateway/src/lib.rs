//! The phone gateway: an HTTP server on the local network that paired phones
//! use to chat with the engine. Every request and response after pairing is
//! end-to-end encrypted with the phone's key, so nothing readable crosses the
//! network even though the transport is plain HTTP.

pub mod crypto;
pub mod devices;
mod server;

pub use devices::Device;
pub use server::{Gateway, GatewayEvent, GatewayStatus, PairingOffer, DEFAULT_PORT};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
