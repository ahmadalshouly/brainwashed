//! Serves the built web chat (`apps/web/dist`). Release builds embed the
//! files; debug builds read them from disk, so a web rebuild shows up without
//! recompiling once the folder exists. Without a build, `/` explains how to
//! make one.

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};

#[derive(rust_embed::Embed)]
#[folder = "../../apps/web/dist/"]
#[allow_missing = true]
struct Assets;

/// The page holds the device's secret key, so it only talks to this host and
/// can't be framed by another site.
const CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; \
                   style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'";

const NOT_BUILT: &str = "<!doctype html><meta charset=utf-8><title>BrainWashed</title>\
<p>The web chat isn't included in this build. Run <code>pnpm --filter @brainwashed/web build</code> \
and rebuild the host.</p>";

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Unknown paths outside assets/ get the app, which has a single page.
    let path = if path.is_empty() || (!path.starts_with("assets/") && Assets::get(path).is_none()) {
        "index.html"
    } else {
        path
    };
    let file = Assets::get(path);
    let mut res = match file {
        Some(f) => {
            let mime = f.metadata.mimetype().to_string();
            let mut res = Body::from(f.data.into_owned()).into_response();
            res.headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_str(&mime).unwrap());
            // Vite puts a content hash in every asset name.
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
            res
        }
        None if path == "index.html" => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            NOT_BUILT,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    };
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}
