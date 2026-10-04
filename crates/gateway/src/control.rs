//! Lets the `brainwashed` command on this computer talk to a server that's
//! already running, e.g. one started at login: `brainwashed open` asks it for
//! a fresh admin link, `brainwashed status` asks how it's doing, and
//! `brainwashed stop` asks it to quit.
//!
//! Requests through a tunnel arrive from localhost too, so being local proves
//! nothing. Instead the caller must send the random token the server wrote to
//! `<data dir>/gateway/control.token`, which only this user can read.

use crate::devices::DeviceRole;
use crate::server::Gateway;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

fn authorized(gw: &Gateway, headers: &HeaderMap) -> bool {
    let Some(sent) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    constant_time_eq(sent.as_bytes(), gw.inner.control_token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn denied() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "not allowed" })),
    )
        .into_response()
}

pub(crate) async fn status(State(gw): State<Gateway>, headers: HeaderMap) -> Response {
    if !authorized(&gw, &headers) {
        return denied();
    }
    let engine = &gw.inner.engine;
    Json(json!({
        "version": engine.info().version,
        "hostName": engine.host_name(),
        "state": engine.state(),
        "access": gw.status(),
        "devices": gw.devices().len(),
    }))
    .into_response()
}

/// A one-time admin pairing link, to open in this computer's browser.
pub(crate) async fn admin_link(State(gw): State<Gateway>, headers: HeaderMap) -> Response {
    if !authorized(&gw, &headers) {
        return denied();
    }
    match gw.create_pairing_offer(DeviceRole::Admin) {
        Ok(offer) => {
            gw.audit(None, "createPairingOffer", Some("admin (terminal)"), None);
            Json(offer).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub(crate) async fn stop(State(gw): State<Gateway>, headers: HeaderMap) -> Response {
    if !authorized(&gw, &headers) {
        return denied();
    }
    gw.inner.stop_requested.notify_one();
    Json(json!({})).into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares_tokens() {
        assert!(super::constant_time_eq(b"abc", b"abc"));
        assert!(!super::constant_time_eq(b"abc", b"abd"));
        assert!(!super::constant_time_eq(b"abc", b"ab"));
    }
}
