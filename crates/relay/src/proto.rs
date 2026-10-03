//! Messages on the WebSocket between a host and the relay. Each WebSocket
//! text message is one of these as JSON.
//!
//! 1. The relay sends [`ToHost::Challenge`]: random bytes and a one-off
//!    relay public key.
//! 2. The host answers [`FromHost::Auth`] with its public key and the
//!    challenge sealed (NaCl box) from its key to the relay's. Only the
//!    holder of the host's secret key can produce that, so nobody else can
//!    register under its key.
//! 3. The relay answers [`ToHost::Ready`], then forwards requests. The host
//!    answers each with one `Head`, any number of `Chunk`s and one `End`.

use serde::{Deserialize, Serialize};

/// Paths a device may reach through the relay. The web chat is not among
/// them: a page served through the relay could be altered by the relay.
pub const FORWARDED_PATHS: [&str; 3] = ["/hello", "/pair", "/rpc"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToHost {
    #[serde(rename_all = "camelCase")]
    Challenge {
        /// Base64, 32 bytes.
        challenge: String,
        /// Base64 X25519 public key, new for every connection.
        relay_key: String,
    },
    Ready,
    Request {
        id: String,
        /// One of [`FORWARDED_PATHS`].
        path: String,
        /// The device's request body; empty for `/hello`.
        body: String,
    },
    /// The device went away; the host may stop working on the request.
    Cancel {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FromHost {
    #[serde(rename_all = "camelCase")]
    Auth {
        /// The host's public key, base64url as in pairing links.
        host_key: String,
        n: String,
        c: String,
    },
    #[serde(rename_all = "camelCase")]
    Head {
        id: String,
        status: u16,
        content_type: String,
    },
    Chunk {
        id: String,
        /// Base64 bytes of the response body.
        data: String,
    },
    End {
        id: String,
    },
}
