//! The gateway: the HTTP server that paired devices (browsers and the phone
//! apps) use to chat with the engine and, for admins, to manage it. Every
//! request and response after pairing is end-to-end encrypted with the
//! device's key, so nothing readable crosses the network even over plain HTTP,
//! a tunnel or a relay.
//!
//! It also serves an OpenAI-compatible API (`openai`) for scripts and other
//! apps, with keys admins create (`api_keys`).
//!
//! It also serves the web app (`apps/web`): chat for everyone, and the admin
//! pages for admins. Remote access runs through a Cloudflare tunnel
//! (`tunnel`), a stable public address, or a self-hosted relay (`relay`).
//! The address book (`address_book`) lets devices find a tunnel whose address
//! changed.

pub mod address_book;
mod admin;
pub mod api_keys;
pub mod audit;
mod control;
pub mod crypto;
pub mod devices;
mod openai;
pub mod relay;
mod replies;
mod server;
pub mod tunnel;
pub mod usage;
mod web;

pub use address_book::AddressBookStatus;
pub use api_keys::ApiKey;
pub use audit::AuditEntry;
pub use devices::{Device, DeviceRole};
pub use relay::RelayStatus;
pub use server::{
    DownloadStatus, Gateway, GatewayEvent, GatewayStatus, PairingOffer, DEFAULT_PORT,
};
pub use tunnel::{TunnelSpec, TunnelStatus};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
