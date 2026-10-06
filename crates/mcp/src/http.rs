//! Servers reached over HTTP. Streamable HTTP first: every message is a POST
//! and answers come back as JSON or as a short event stream. Servers that
//! only speak the older HTTP+SSE transport get a long-lived event stream,
//! whose `endpoint` event says where to POST.

use crate::sse;
use crate::{Error, Link, Peer, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{StatusCode, Url};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

pub(crate) struct Http {
    client: reqwest::Client,
    url: Url,
    headers: HeaderMap,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    /// Older servers: where to POST, from the event stream's `endpoint` event.
    legacy: Mutex<Option<Url>>,
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}

pub(crate) fn open(
    url: &str,
    headers: &BTreeMap<String, String>,
    client: &reqwest::Client,
) -> Result<Arc<Peer>> {
    let url = Url::parse(url.trim())
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .ok_or_else(|| {
            Error::Failed("the server address must start with https:// or http://".into())
        })?;
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| Error::Failed(format!("`{name}` isn't a valid header name")))?;
        let mut value = HeaderValue::from_str(value.trim())
            .map_err(|_| Error::Failed(format!("the value of the {name} header isn't valid")))?;
        value.set_sensitive(true);
        map.insert(name, value);
    }
    Ok(Peer::new(Link::Http(Http {
        client: client.clone(),
        url,
        headers: map,
        session: Mutex::new(None),
        protocol: Mutex::new(None),
        legacy: Mutex::new(None),
        tasks: Mutex::new(Vec::new()),
    })))
}

/// Whether a failed first request means the server may speak the older
/// HTTP+SSE transport instead.
pub(crate) fn try_legacy(e: &Error) -> bool {
    matches!(e, Error::Failed(m) if m.starts_with(NOT_STREAMABLE))
}

const NOT_STREAMABLE: &str = "No MCP server answered POST requests at this address";

impl Http {
    pub(crate) fn set_protocol(&self, version: &str) {
        *self.protocol.lock().unwrap() = Some(version.to_string());
    }

    pub(crate) async fn send(&self, peer: &Arc<Peer>, message: Value) -> Result<()> {
        let request = message
            .get("method")
            .and_then(|_| message.get("id"))
            .and_then(Value::as_u64);
        let initialize = message["method"] == "initialize";

        let legacy = self.legacy.lock().unwrap().clone();
        if let Some(post) = legacy {
            let res = self
                .client
                .post(post)
                .headers(self.headers.clone())
                .json(&message)
                .send()
                .await
                .map_err(|e| unreachable(&self.url, &e))?;
            let status = res.status();
            if !status.is_success() {
                return Err(status_error(status, &res.text().await.unwrap_or_default()));
            }
            return Ok(());
        }

        let session = self.session.lock().unwrap().clone();
        let mut req = self
            .client
            .post(self.url.clone())
            .headers(self.headers.clone())
            .header(ACCEPT, "application/json, text/event-stream")
            .json(&message);
        if let Some(s) = &session {
            req = req.header("mcp-session-id", s);
        }
        if let Some(v) = self.protocol.lock().unwrap().clone() {
            req = req.header("mcp-protocol-version", v);
        }
        let res = req.send().await.map_err(|e| unreachable(&self.url, &e))?;
        if let Some(s) = res
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *self.session.lock().unwrap() = Some(s.to_string());
        }
        let status = res.status();
        if !status.is_success() {
            let body = res.text().await.unwrap_or_default();
            if initialize
                && matches!(
                    status,
                    StatusCode::BAD_REQUEST
                        | StatusCode::NOT_FOUND
                        | StatusCode::METHOD_NOT_ALLOWED
                )
            {
                return Err(Error::Failed(format!("{NOT_STREAMABLE} ({status})")));
            }
            if status == StatusCode::NOT_FOUND && session.is_some() {
                peer.ended("the server ended the session");
            }
            return Err(status_error(status, &body));
        }
        if status == StatusCode::ACCEPTED || status == StatusCode::NO_CONTENT {
            if let Some(id) = request {
                peer.fail(
                    id,
                    Error::Failed("the server didn't answer the request".into()),
                );
            }
            return Ok(());
        }
        let stream = res
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        if stream {
            // The answer comes as events, possibly after notifications and
            // requests of the server's own.
            let peer = peer.clone();
            let task = tokio::spawn(async move {
                let mut parser = sse::Parser::default();
                let mut body = res.bytes_stream();
                while let Some(Ok(bytes)) = body.next().await {
                    for event in parser.push(&bytes) {
                        if event.event == "message" {
                            if let Ok(message) = serde_json::from_str(&event.data) {
                                peer.incoming(message);
                            }
                        }
                    }
                }
                if let Some(id) = request {
                    peer.fail(
                        id,
                        Error::Failed("the server ended its answer without a result".into()),
                    );
                }
            });
            self.keep(task.abort_handle());
            return Ok(());
        }
        let body = res.bytes().await.map_err(|e| unreachable(&self.url, &e))?;
        if body.iter().all(u8::is_ascii_whitespace) {
            if let Some(id) = request {
                peer.fail(id, Error::Failed("the server sent an empty answer".into()));
            }
            return Ok(());
        }
        let message: Value = serde_json::from_slice(&body).map_err(|_| {
            Error::Failed(format!(
                "{} doesn't look like an MCP server: it didn't answer with JSON",
                self.url
            ))
        })?;
        peer.incoming(message);
        Ok(())
    }

    /// Opens the event stream of the older HTTP+SSE transport and waits for
    /// the address to POST to.
    pub(crate) async fn open_legacy(&self, peer: &Arc<Peer>) -> Result<()> {
        let res = self
            .client
            .get(self.url.clone())
            .headers(self.headers.clone())
            .header(ACCEPT, "text/event-stream")
            .send()
            .await
            .map_err(|e| unreachable(&self.url, &e))?;
        let status = res.status();
        if !status.is_success() {
            return Err(status_error(status, &res.text().await.unwrap_or_default()));
        }
        let is_stream = res
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        if !is_stream {
            return Err(Error::Failed(format!(
                "No MCP server answered at {}. Check the address: it often ends in /mcp, or /sse for older servers.",
                self.url
            )));
        }
        let (tx, rx) = tokio::sync::oneshot::channel::<Url>();
        let base = self.url.clone();
        let weak: Weak<Peer> = Arc::downgrade(peer);
        let task = tokio::spawn(async move {
            let mut tx = Some(tx);
            let mut parser = sse::Parser::default();
            let mut body = res.bytes_stream();
            while let Some(Ok(bytes)) = body.next().await {
                let Some(peer) = weak.upgrade() else { return };
                for event in parser.push(&bytes) {
                    match event.event.as_str() {
                        "endpoint" => {
                            if let (Some(tx), Ok(url)) = (tx.take(), base.join(event.data.trim())) {
                                let _ = tx.send(url);
                            }
                        }
                        "message" => {
                            if let Ok(message) = serde_json::from_str(&event.data) {
                                peer.incoming(message);
                            }
                        }
                        _ => {}
                    }
                }
            }
            if let Some(peer) = weak.upgrade() {
                peer.ended("the server closed the connection");
            }
        });
        self.keep(task.abort_handle());
        let post = tokio::time::timeout(Duration::from_secs(30), rx)
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| {
                Error::Failed(format!(
                    "{} opened an event stream but never said where to send messages",
                    self.url
                ))
            })?;
        *self.legacy.lock().unwrap() = Some(post);
        Ok(())
    }

    fn keep(&self, task: tokio::task::AbortHandle) {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|t| !t.is_finished());
        tasks.push(task);
    }

    pub(crate) async fn close(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        let session = self.session.lock().unwrap().take();
        if let Some(session) = session {
            // Lets the server free the session; it times out otherwise.
            let _ = self
                .client
                .delete(self.url.clone())
                .headers(self.headers.clone())
                .header("mcp-session-id", session)
                .timeout(Duration::from_secs(3))
                .send()
                .await;
        }
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

fn unreachable(url: &Url, e: &reqwest::Error) -> Error {
    let mut why = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        why = s.to_string();
        source = s.source();
    }
    Error::Failed(format!("couldn't reach {url}: {why}"))
}

fn status_error(status: StatusCode, body: &str) -> Error {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .or(v["error"].as_str())
                .or(v["message"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.trim().chars().take(200).collect());
    let detail = if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    };
    Error::Failed(match status.as_u16() {
        401 | 403 => format!(
            "the server refused access ({status}){detail}. If it needs a token, add it as a header, e.g. Authorization: Bearer …"
        ),
        _ => format!("the server returned {status}{detail}"),
    })
}
