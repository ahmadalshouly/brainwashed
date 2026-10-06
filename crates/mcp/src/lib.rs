//! A small client for the Model Context Protocol (MCP), so models can use the
//! tools of MCP servers. A server is a command this computer starts, which
//! speaks JSON-RPC over its stdin and stdout, or an address it talks to over
//! HTTP (Streamable HTTP, falling back to the older HTTP+SSE transport).
//!
//! Only tools are used: the client lists them and calls them. It answers the
//! server's pings and declares no other capabilities, so servers don't ask it
//! for sampling, roots or elicitation.

mod http;
mod sse;
mod stdio;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

/// The protocol version this client asks for. Servers may answer with an
/// older one they support; the client goes along with it.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// How to reach a server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Transport {
    /// A command this computer runs, e.g. `npx -y @modelcontextprotocol/server-filesystem ~/Documents`.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        /// Added to the host's own environment.
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// Folder to run it in. None runs it in the host's folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    /// A server reached over HTTP, e.g. `https://example.org/mcp`.
    Http {
        url: String,
        /// Sent with every request, e.g. `Authorization: Bearer …`.
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Failed(String),
    #[error("the server didn't answer within {0} seconds")]
    Timeout(u64),
    #[error("{0}")]
    Closed(String),
    /// The server answered with a JSON-RPC error.
    #[error("{message}")]
    Rpc { code: i64, message: String },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What the client tells servers about itself, and how long it waits.
#[derive(Debug, Clone)]
pub struct Options {
    pub client_name: String,
    pub client_version: String,
    /// For HTTP servers. Should have no overall timeout, since answers can
    /// stream for as long as a tool runs.
    pub http: reqwest::Client,
    /// How long starting and initializing may take. Commands run with `npx`
    /// or `uvx` download the server the first time, which can take a while.
    pub start_timeout: Duration,
}

impl Options {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Options {
            client_name: client_name.into(),
            client_version: client_version.into(),
            http: reqwest::Client::new(),
            start_timeout: Duration::from_secs(120),
        }
    }
}

/// A tool a server offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema of the arguments.
    #[serde(default = "empty_schema")]
    pub input_schema: Value,
}

fn empty_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

/// What a tool returned.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallResult {
    /// Text, images, resources… as the server sent them.
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default)]
    pub structured_content: Option<Value>,
    /// The tool ran but failed; `content` says why.
    #[serde(default)]
    pub is_error: bool,
}

impl CallResult {
    /// The result as text a model can read. Pictures and other binary
    /// content are named, not included.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        for item in &self.content {
            let part = match item["type"].as_str() {
                Some("text") => item["text"].as_str().unwrap_or_default().to_string(),
                Some("image") => format!(
                    "[A picture ({}) the tool returned]",
                    item["mimeType"].as_str().unwrap_or("image")
                ),
                Some("audio") => "[Audio the tool returned]".to_string(),
                Some("resource_link") => format!(
                    "[Link: {} {}]",
                    item["name"].as_str().unwrap_or_default(),
                    item["uri"].as_str().unwrap_or_default()
                ),
                Some("resource") => {
                    let r = &item["resource"];
                    match r["text"].as_str() {
                        Some(text) => text.to_string(),
                        None => format!("[File {}]", r["uri"].as_str().unwrap_or_default()),
                    }
                }
                _ => item.to_string(),
            };
            parts.push(part);
        }
        if parts.iter().all(|p| p.trim().is_empty()) {
            if let Some(s) = &self.structured_content {
                return s.to_string();
            }
        }
        parts.join("\n")
    }
}

/// What the server said about itself when it started.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub protocol_version: String,
    /// How to use the server, for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// A connection to one server. Cheap to clone; the connection closes when
/// [`Client::close`] is called or the last clone is dropped.
#[derive(Clone)]
pub struct Client {
    peer: Arc<Peer>,
    info: ServerInfo,
}

impl Client {
    /// Starts or reaches the server and goes through MCP's handshake.
    pub async fn connect(transport: &Transport, options: &Options) -> Result<Client> {
        let timeout = options.start_timeout;
        let peer = match transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => stdio::spawn(command, args, env, cwd.as_deref())?,
            Transport::Http { url, headers } => http::open(url, headers, &options.http)?,
        };
        let info = match tokio::time::timeout(timeout, peer.handshake(options)).await {
            Ok(Ok(info)) => info,
            Ok(Err(e)) => {
                let e = peer.explain(e);
                peer.close().await;
                return Err(e);
            }
            Err(_) => {
                peer.close().await;
                return Err(peer.explain(Error::Timeout(timeout.as_secs())));
            }
        };
        Ok(Client { peer, info })
    }

    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    /// False once the server stopped or the connection broke.
    pub fn is_alive(&self) -> bool {
        self.peer.closed.lock().unwrap().is_none()
    }

    /// True when the server said its tools changed since they were last listed.
    pub fn tools_changed(&self) -> bool {
        self.peer.tools_changed.load(Ordering::SeqCst)
    }

    /// The last lines a command wrote to stderr, for working out why it failed.
    pub fn log(&self) -> Vec<String> {
        self.peer.log.lock().unwrap().iter().cloned().collect()
    }

    pub async fn list_tools(&self) -> Result<Vec<Tool>> {
        self.peer.tools_changed.store(false, Ordering::SeqCst);
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        // A server that keeps handing out cursors gets cut off.
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let page = self
                .peer
                .request("tools/list", params, Duration::from_secs(30), false)
                .await?;
            let list: Vec<Value> = page["tools"].as_array().cloned().unwrap_or_default();
            for t in list {
                match serde_json::from_value::<Tool>(t) {
                    Ok(t) if !t.name.is_empty() => tools.push(t),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("skipping a tool the MCP server described oddly: {e}"),
                }
            }
            cursor = page["nextCursor"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    /// Calls a tool. Dropping the future cancels the call on the server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> Result<CallResult> {
        let result = self
            .peer
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                timeout,
                true,
            )
            .await?;
        serde_json::from_value(result)
            .map_err(|e| Error::Failed(format!("the server's answer couldn't be read: {e}")))
    }

    /// Ends the connection: a command gets its stdin closed and a moment to
    /// exit before it is stopped.
    pub async fn close(&self) {
        self.peer.close().await;
    }
}

/// How messages reach the server.
enum Link {
    Stdio(stdio::Stdio),
    Http(http::Http),
}

/// One side of the JSON-RPC conversation, whatever carries it.
struct Peer {
    link: Link,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>,
    /// Why the connection ended, once it has.
    closed: Mutex<Option<String>>,
    tools_changed: AtomicBool,
    /// The last lines of a command's stderr.
    log: Mutex<VecDeque<String>>,
}

const LOG_LINES: usize = 30;

impl Peer {
    fn new(link: Link) -> Arc<Peer> {
        Arc::new(Peer {
            link,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            closed: Mutex::new(None),
            tools_changed: AtomicBool::new(false),
            log: Mutex::new(VecDeque::new()),
        })
    }

    async fn handshake(self: &Arc<Self>, options: &Options) -> Result<ServerInfo> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": options.client_name, "version": options.client_version },
        });
        let init = match self
            .request("initialize", params.clone(), options.start_timeout, false)
            .await
        {
            Err(e) if http::try_legacy(&e) => {
                if let Link::Http(h) = &self.link {
                    h.open_legacy(self).await?;
                }
                self.request("initialize", params, options.start_timeout, false)
                    .await?
            }
            other => other?,
        };
        let protocol = init["protocolVersion"]
            .as_str()
            .unwrap_or(PROTOCOL_VERSION)
            .to_string();
        if let Link::Http(h) = &self.link {
            h.set_protocol(&protocol);
        }
        self.notify("notifications/initialized", json!({})).await?;
        Ok(ServerInfo {
            name: init["serverInfo"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            version: init["serverInfo"]["version"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            protocol_version: protocol,
            instructions: init["instructions"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string),
        })
    }

    async fn send(self: &Arc<Self>, message: Value) -> Result<()> {
        if let Some(why) = self.closed.lock().unwrap().clone() {
            return Err(Error::Closed(why));
        }
        match &self.link {
            Link::Stdio(s) => s.send(&message),
            Link::Http(h) => h.send(self, message).await,
        }
    }

    async fn notify(self: &Arc<Self>, method: &str, params: Value) -> Result<()> {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    /// Sends a request and waits for its answer. With `cancel`, giving up
    /// (a timeout, or the future being dropped) tells the server to stop.
    async fn request(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: bool,
    ) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut waiting = Waiting {
            peer: self.clone(),
            id,
            cancel,
            done: false,
        };
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let answer = tokio::time::timeout(timeout, async {
            self.send(message).await?;
            rx.await
                .unwrap_or_else(|_| Err(Error::Closed("the server stopped".into())))
        })
        .await;
        match answer {
            Ok(result) => {
                waiting.done = true;
                result
            }
            Err(_) => Err(Error::Timeout(timeout.as_secs())),
        }
    }

    /// Handles what the server sent: answers to our requests, its own
    /// requests (pings), and notifications.
    fn incoming(self: &Arc<Self>, message: Value) {
        let message = match message {
            Value::Array(batch) => {
                for m in batch {
                    self.incoming(m);
                }
                return;
            }
            Value::Object(m) => m,
            _ => return,
        };
        let id = message.get("id").filter(|v| !v.is_null()).cloned();
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            match id {
                Some(id) => {
                    let reply = match method {
                        "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                        "roots/list" => {
                            json!({ "jsonrpc": "2.0", "id": id, "result": { "roots": [] } })
                        }
                        _ => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": format!("BrainWashed doesn't support {method}") },
                        }),
                    };
                    let peer = self.clone();
                    tokio::spawn(async move {
                        let _ = peer.send(reply).await;
                    });
                }
                None if method == "notifications/tools/list_changed" => {
                    self.tools_changed.store(true, Ordering::SeqCst);
                }
                None => {}
            }
            return;
        }
        let Some(id) = id.as_ref().and_then(Value::as_u64) else {
            return;
        };
        let Some(tx) = self.pending.lock().unwrap().remove(&id) else {
            return;
        };
        let result = match message.get("error") {
            Some(e) => Err(Error::Rpc {
                code: e["code"].as_i64().unwrap_or(0),
                message: e["message"]
                    .as_str()
                    .unwrap_or("the server reported an error")
                    .to_string(),
            }),
            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        };
        let _ = tx.send(result);
    }

    /// Fails a request that can no longer be answered, e.g. because the
    /// stream that would carry its answer ended.
    fn fail(&self, id: u64, error: Error) {
        if let Some(tx) = self.pending.lock().unwrap().remove(&id) {
            let _ = tx.send(Err(error));
        }
    }

    /// Marks the connection as ended and fails everything still waiting.
    fn ended(&self, why: &str) {
        let why = {
            let mut closed = self.closed.lock().unwrap();
            if closed.is_some() {
                return;
            }
            let why = self.with_log(why);
            *closed = Some(why.clone());
            why
        };
        let pending: Vec<_> = self.pending.lock().unwrap().drain().collect();
        for (_, tx) in pending {
            let _ = tx.send(Err(Error::Closed(why.clone())));
        }
    }

    async fn close(&self) {
        self.ended("the connection was closed");
        match &self.link {
            Link::Stdio(s) => s.close().await,
            Link::Http(h) => h.close().await,
        }
    }

    fn remember(&self, line: &str) {
        let line = line.trim_end();
        if line.trim().is_empty() {
            return;
        }
        let mut log = self.log.lock().unwrap();
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(line.chars().take(500).collect());
    }

    /// `why`, followed by what the command last wrote to stderr.
    fn with_log(&self, why: &str) -> String {
        let log = self.log.lock().unwrap();
        if log.is_empty() {
            return why.to_string();
        }
        let tail: Vec<&str> = log.iter().rev().take(8).rev().map(String::as_str).collect();
        format!("{why}. It said:\n{}", tail.join("\n"))
    }

    /// Adds the command's last words to a failure, when it left some.
    fn explain(&self, e: Error) -> Error {
        match e {
            Error::Closed(why) => Error::Closed(why),
            Error::Timeout(secs) => Error::Failed(self.with_log(&format!(
                "the server didn't finish starting within {secs} seconds"
            ))),
            other => other,
        }
    }
}

/// A request waiting for its answer. Dropped early, it forgets the request
/// and, for tool calls, tells the server to stop working on it.
struct Waiting {
    peer: Arc<Peer>,
    id: u64,
    cancel: bool,
    done: bool,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let still_waiting = self.peer.pending.lock().unwrap().remove(&self.id).is_some();
        if self.done || !self.cancel || !still_waiting {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let peer = self.peer.clone();
        let id = self.id;
        rt.spawn(async move {
            let _ = peer
                .notify(
                    "notifications/cancelled",
                    json!({ "requestId": id, "reason": "The person stopped the reply." }),
                )
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_as_text() {
        let r: CallResult = serde_json::from_value(json!({
            "content": [
                { "type": "text", "text": "Sunny, 21°C" },
                { "type": "image", "data": "AAAA", "mimeType": "image/png" },
                { "type": "resource", "resource": { "uri": "file:///a.txt", "text": "hello" } },
            ],
            "isError": false,
        }))
        .unwrap();
        assert_eq!(
            r.text(),
            "Sunny, 21°C\n[A picture (image/png) the tool returned]\nhello"
        );

        let structured: CallResult =
            serde_json::from_value(json!({ "content": [], "structuredContent": { "t": 21 } }))
                .unwrap();
        assert_eq!(structured.text(), "{\"t\":21}");
    }

    #[test]
    fn transports_round_trip() {
        let t: Transport = serde_json::from_value(json!({
            "type": "stdio",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-everything"],
        }))
        .unwrap();
        assert!(
            matches!(&t, Transport::Stdio { command, env, .. } if command == "npx" && env.is_empty())
        );
        let h: Transport =
            serde_json::from_value(json!({ "type": "http", "url": "https://example.org/mcp" }))
                .unwrap();
        assert_eq!(
            serde_json::to_value(&h).unwrap(),
            json!({ "type": "http", "url": "https://example.org/mcp", "headers": {} })
        );
    }
}
