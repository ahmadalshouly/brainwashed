//! MCP servers admins add, whose tools models can use in the chat. Each server
//! has an on/off switch, and only admins' chats use it unless the admin lets
//! members use it too. The host starts the servers, offers their tools to the
//! model, runs the calls the model makes and sends back the results.
//!
//! Servers are kept in `mcp.json` on this computer (readable only by its
//! owner), since their settings can hold tokens.

use crate::engine::Event;
use crate::{Engine, Error, Result};
use brainwashed_mcp as mcp;
use brainwashed_runtime::toolcalls::ToolCall;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub use mcp::Transport as McpTransport;

/// How long one tool call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// A server that failed to start is tried again after this long.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// How long a chat waits for servers that are still starting.
const START_WAIT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    /// Short name, letters, numbers, - or _.
    pub id: String,
    pub name: String,
    /// Off: not started, and its tools aren't offered.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Members' chats may use its tools too, not only admins'.
    #[serde(default)]
    pub members: bool,
    pub transport: McpTransport,
}

fn yes() -> bool {
    true
}

/// Whether a server is running. Mirrors `McpStatus` in `packages/api`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum McpStatus {
    Off,
    Starting,
    Ready,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
}

/// A server as admins see it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    #[serde(flatten)]
    pub server: McpServer,
    pub status: McpStatus,
    pub tools: Vec<McpToolInfo>,
    /// The name and version the server gave, e.g. `secure-filesystem-server 0.2.0`.
    pub server_info: Option<String>,
    /// The last lines a command server wrote to stderr.
    pub log: Vec<String>,
}

/// A tool's result, sent to clients after its call. Mirrors `ToolResult` in
/// `packages/api`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    /// The id of the call this answers.
    pub id: String,
    pub name: String,
    /// Name of the MCP server that ran it; empty if none did.
    pub server: String,
    /// What the model reads, cut to fit its context.
    pub content: String,
    pub is_error: bool,
}

/// One server and its connection.
struct Slot {
    server: McpServer,
    conn: tokio::sync::Mutex<Option<mcp::Client>>,
    status: RwLock<McpStatus>,
    tools: RwLock<Vec<mcp::Tool>>,
    info: RwLock<Option<mcp::ServerInfo>>,
    log: RwLock<Vec<String>>,
    tried: Mutex<Option<Instant>>,
}

impl Slot {
    fn new(server: McpServer) -> Arc<Slot> {
        let status = if server.enabled {
            McpStatus::Starting
        } else {
            McpStatus::Off
        };
        Arc::new(Slot {
            server,
            conn: tokio::sync::Mutex::new(None),
            status: RwLock::new(status),
            tools: RwLock::new(Vec::new()),
            info: RwLock::new(None),
            log: RwLock::new(Vec::new()),
            tried: Mutex::new(None),
        })
    }

    fn status(&self) -> McpStatus {
        self.status.read().unwrap().clone()
    }

    /// Connects if not connected, and lists the tools again if the server
    /// said they changed. Returns the client when the server is usable.
    async fn ensure(&self, options: &mcp::Options) -> Option<mcp::Client> {
        let mut conn = self.conn.lock().await;
        if let Some(client) = conn.as_ref() {
            if client.is_alive() {
                if client.tools_changed() {
                    if let Ok(tools) = client.list_tools().await {
                        *self.tools.write().unwrap() = tools;
                    }
                }
                return Some(client.clone());
            }
            *self.log.write().unwrap() = client.log();
            tracing::warn!("MCP server {} stopped; starting it again", self.server.id);
        }
        *conn = None;
        *self.tried.lock().unwrap() = Some(Instant::now());
        *self.status.write().unwrap() = McpStatus::Starting;
        let result = async {
            let client = mcp::Client::connect(&self.server.transport, options).await?;
            match client.list_tools().await {
                Ok(tools) => Ok((client, tools)),
                Err(e) => {
                    client.close().await;
                    Err(e)
                }
            }
        }
        .await;
        match result {
            Ok((client, tools)) => {
                tracing::info!(
                    "MCP server {} is ready with {} tools",
                    self.server.id,
                    tools.len()
                );
                *self.tools.write().unwrap() = tools;
                *self.info.write().unwrap() = Some(client.info().clone());
                *self.log.write().unwrap() = client.log();
                *self.status.write().unwrap() = McpStatus::Ready;
                *conn = Some(client.clone());
                Some(client)
            }
            Err(e) => {
                tracing::warn!("MCP server {} didn't start: {e}", self.server.id);
                *self.status.write().unwrap() = McpStatus::Error {
                    message: e.to_string(),
                };
                None
            }
        }
    }

    async fn stop(&self) {
        if let Some(client) = self.conn.lock().await.take() {
            client.close().await;
        }
    }

    /// Whether a chat should try to (re)start it now.
    fn worth_trying(&self) -> bool {
        match self.status() {
            McpStatus::Off => false,
            McpStatus::Error { .. } => self
                .tried
                .lock()
                .unwrap()
                .map_or(true, |t| t.elapsed() >= RETRY_AFTER),
            _ => true,
        }
    }
}

#[derive(Default)]
pub(crate) struct McpState {
    slots: Mutex<HashMap<String, Arc<Slot>>>,
}

/// A tool offered to the model in one chat.
#[derive(Clone)]
pub(crate) struct OfferedTool {
    /// The name the model sees; unique within the chat.
    pub name: String,
    tool: String,
    server: String,
    client: mcp::Client,
    slot: Arc<Slot>,
}

/// The tools of the servers a chat may use.
#[derive(Clone, Default)]
pub(crate) struct Toolbox {
    pub tools: Vec<OfferedTool>,
    /// In OpenAI's `tools` format.
    pub definitions: Vec<Value>,
}

impl Engine {
    fn mcp_file(&self) -> PathBuf {
        self.data_dir().join("mcp.json")
    }

    fn saved_mcp_servers(&self) -> Vec<McpServer> {
        crate::store::load(&self.mcp_file())
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn mcp_options(&self) -> mcp::Options {
        mcp::Options {
            http: self.inner.mcp_http.clone(),
            ..mcp::Options::new("BrainWashed", self.app_version())
        }
    }

    /// The slot of each saved server, creating any that are missing.
    fn mcp_slots(&self) -> Vec<Arc<Slot>> {
        let saved = self.saved_mcp_servers();
        let mut slots = self.inner.mcp.slots.lock().unwrap();
        saved
            .into_iter()
            .map(|s| {
                slots
                    .entry(s.id.clone())
                    .or_insert_with(|| Slot::new(s))
                    .clone()
            })
            .collect()
    }

    /// The servers admins added, with whether each is running and its tools.
    pub fn mcp_servers(&self) -> Vec<McpServerInfo> {
        self.mcp_slots()
            .into_iter()
            .map(|slot| McpServerInfo {
                server: slot.server.clone(),
                status: slot.status(),
                tools: slot
                    .tools
                    .read()
                    .unwrap()
                    .iter()
                    .map(|t| McpToolInfo {
                        name: t.name.clone(),
                        description: t
                            .description
                            .clone()
                            .or_else(|| t.title.clone())
                            .unwrap_or_default(),
                    })
                    .collect(),
                server_info: slot
                    .info
                    .read()
                    .unwrap()
                    .as_ref()
                    .map(|i| format!("{} {}", i.name, i.version).trim().to_string()),
                log: slot.log.read().unwrap().clone(),
            })
            .collect()
    }

    /// Starts every server that is switched on, in the background.
    pub fn start_mcp_servers(&self) {
        for slot in self.mcp_slots() {
            if slot.server.enabled {
                let options = self.mcp_options();
                tokio::spawn(async move {
                    slot.ensure(&options).await;
                });
            }
        }
    }

    /// Stops every server, e.g. before the host exits.
    pub async fn stop_mcp_servers(&self) {
        let slots: Vec<_> = self.inner.mcp.slots.lock().unwrap().drain().collect();
        for (_, slot) in slots {
            slot.stop().await;
        }
    }

    /// Adds a server or changes one, then (re)starts it in the background
    /// if it's on.
    pub async fn save_mcp_server(&self, server: McpServer) -> Result<McpServerInfo> {
        let server = validate(server)?;
        let mut all = self.saved_mcp_servers();
        match all.iter_mut().find(|s| s.id == server.id) {
            Some(s) => *s = server.clone(),
            None => all.push(server.clone()),
        }
        self.save_mcp_list(&all)?;
        self.replace_slot(&server.id).await;
        Ok(self.mcp_server(&server.id))
    }

    pub async fn delete_mcp_server(&self, id: &str) -> Result<()> {
        let mut all = self.saved_mcp_servers();
        let before = all.len();
        all.retain(|s| s.id != id);
        if all.len() == before {
            return Err(Error::Invalid(format!("no MCP server `{id}`")));
        }
        self.save_mcp_list(&all)?;
        self.replace_slot(id).await;
        Ok(())
    }

    /// Switches a server on (starting it) or off (stopping it).
    pub async fn set_mcp_server_enabled(&self, id: &str, enabled: bool) -> Result<McpServerInfo> {
        let mut all = self.saved_mcp_servers();
        let server = all
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::Invalid(format!("no MCP server `{id}`")))?;
        server.enabled = enabled;
        self.save_mcp_list(&all)?;
        self.replace_slot(id).await;
        Ok(self.mcp_server(id))
    }

    /// Stops a server and starts it again, e.g. after installing what it needs.
    pub async fn restart_mcp_server(&self, id: &str) -> Result<McpServerInfo> {
        if !self.saved_mcp_servers().iter().any(|s| s.id == id) {
            return Err(Error::Invalid(format!("no MCP server `{id}`")));
        }
        self.replace_slot(id).await;
        Ok(self.mcp_server(id))
    }

    fn mcp_server(&self, id: &str) -> McpServerInfo {
        self.mcp_servers()
            .into_iter()
            .find(|s| s.server.id == id)
            .expect("the server was just saved")
    }

    fn save_mcp_list(&self, servers: &[McpServer]) -> Result<()> {
        let path = self.mcp_file();
        crate::store::save(&path, &servers)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        self.emit(Event::ToolsChanged);
        Ok(())
    }

    /// Stops the running copy of a server and starts it from its saved
    /// settings, if it still exists and is on. Starting carries on in the
    /// background; its status says how it went.
    async fn replace_slot(&self, id: &str) {
        let old = self.inner.mcp.slots.lock().unwrap().remove(id);
        if let Some(old) = old {
            old.stop().await;
        }
        let slot = self.mcp_slots().into_iter().find(|s| s.server.id == id);
        if let Some(slot) = slot.filter(|s| s.server.enabled) {
            let options = self.mcp_options();
            tokio::spawn(async move {
                slot.ensure(&options).await;
            });
        }
    }

    /// The tools a chat may use: those of every running server that is on
    /// and, for members, shared with them. Servers still starting get a
    /// moment; ones that stopped are started again.
    pub(crate) async fn toolbox(&self, admin: bool) -> Toolbox {
        let slots: Vec<Arc<Slot>> = self
            .mcp_slots()
            .into_iter()
            .filter(|s| s.server.enabled && (admin || s.server.members) && s.worth_trying())
            .collect();
        let mut waits = Vec::new();
        for slot in slots {
            let options = self.mcp_options();
            // Spawned, so a slow start carries on for the next message.
            let task = tokio::spawn({
                let slot = slot.clone();
                async move { slot.ensure(&options).await }
            });
            waits.push(async move {
                let client = tokio::time::timeout(START_WAIT, task).await.ok()?.ok()??;
                Some((slot, client))
            });
        }
        let ready = futures_util::future::join_all(waits).await;

        let mut box_ = Toolbox::default();
        let mut taken: HashSet<String> = HashSet::from(["ask_user".to_string()]);
        let all: Vec<(Arc<Slot>, mcp::Client, mcp::Tool)> = ready
            .into_iter()
            .flatten()
            .flat_map(|(slot, client)| {
                let tools = slot.tools.read().unwrap().clone();
                tools
                    .into_iter()
                    .map(move |t| (slot.clone(), client.clone(), t))
            })
            .collect();
        // A tool keeps its own name unless another server's tool has it too.
        let mut counts: HashMap<String, usize> = HashMap::new();
        for (_, _, t) in &all {
            *counts.entry(clean_name(&t.name)).or_default() += 1;
        }
        for (slot, client, tool) in all {
            let plain = clean_name(&tool.name);
            let mut name = if counts[&plain] == 1 && !taken.contains(&plain) {
                plain
            } else {
                clean_name(&format!("{}_{}", slot.server.id, tool.name))
            };
            let mut n = 2;
            while taken.contains(&name) {
                name = format!("{}_{n}", name.chars().take(60).collect::<String>());
                n += 1;
            }
            taken.insert(name.clone());
            let description = tool
                .description
                .clone()
                .or_else(|| tool.title.clone())
                .unwrap_or_else(|| format!("{} from {}", tool.name, slot.server.name));
            box_.definitions.push(json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description.chars().take(2000).collect::<String>(),
                    "parameters": clean_schema(&tool.input_schema),
                }
            }));
            box_.tools.push(OfferedTool {
                name,
                tool: tool.name,
                server: slot.server.name.clone(),
                client,
                slot,
            });
        }
        box_
    }
}

impl Toolbox {
    pub(crate) fn find(&self, name: &str) -> Option<&OfferedTool> {
        self.tools.iter().find(|t| t.name == name)
    }

    /// Runs a call the model made and returns what to tell it. Results are
    /// cut to `max_chars`.
    pub(crate) async fn run(&self, call: &ToolCall, max_chars: usize) -> ToolResult {
        let fail = |server: &str, content: String| ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            server: server.to_string(),
            content,
            is_error: true,
        };
        if call.name.is_empty() {
            return fail(
                "",
                "The tool call couldn't be read. Write it again as valid JSON.".into(),
            );
        }
        let Some(tool) = self.find(&call.name) else {
            let names: Vec<&str> = self.tools.iter().map(|t| t.name.as_str()).collect();
            return fail(
                "",
                format!(
                    "There is no tool named {}. The tools are: {}.",
                    call.name,
                    names.join(", ")
                ),
            );
        };
        let arguments = match &call.arguments {
            Value::Object(_) => call.arguments.clone(),
            Value::Null => json!({}),
            _ => {
                return fail(
                    &tool.server,
                    "The arguments must be a JSON object, with a field per parameter.".into(),
                )
            }
        };
        let started = Instant::now();
        let result = tool
            .client
            .call_tool(&tool.tool, arguments, CALL_TIMEOUT)
            .await;
        tracing::info!(
            "tool {} of MCP server {} ran in {} ms",
            tool.tool,
            tool.slot.server.id,
            started.elapsed().as_millis()
        );
        let (content, is_error) = match result {
            Ok(r) => (r.text(), r.is_error),
            Err(e) => {
                if !tool.client.is_alive() {
                    *tool.slot.status.write().unwrap() = McpStatus::Error {
                        message: e.to_string(),
                    };
                }
                (format!("The tool failed: {e}"), true)
            }
        };
        ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            server: tool.server.clone(),
            content: cut(&content, max_chars),
            is_error,
        }
    }
}

fn validate(mut server: McpServer) -> Result<McpServer> {
    server.id = server.id.trim().to_ascii_lowercase();
    server.name = server.name.trim().to_string();
    if server.id.is_empty()
        || server.id.len() > 32
        || !server
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::Invalid(
            "the server id must be up to 32 letters, numbers, - or _".into(),
        ));
    }
    if server.name.is_empty() {
        server.name = server.id.clone();
    }
    match &mut server.transport {
        McpTransport::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            *command = command.trim().to_string();
            if command.is_empty() {
                return Err(Error::Invalid(
                    "give the command that starts the server, e.g. npx".into(),
                ));
            }
            args.retain(|a| !a.is_empty());
            env.retain(|k, _| !k.trim().is_empty());
            if let Some(bad) = env.keys().find(|k| k.contains('=') || k.contains('\0')) {
                return Err(Error::Invalid(format!(
                    "`{bad}` isn't a valid environment variable name"
                )));
            }
            *cwd = cwd
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
        }
        McpTransport::Http { url, headers } => {
            *url = url.trim().to_string();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(Error::Invalid(
                    "the server address must start with https:// or http://".into(),
                ));
            }
            headers.retain(|k, _| !k.trim().is_empty());
        }
    }
    Ok(server)
}

/// A tool name as OpenAI's API accepts it: letters, digits, _ and -, up to
/// 64 characters.
fn clean_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    if clean.is_empty() {
        "tool".into()
    } else {
        clean
    }
}

/// The parameters schema as models and llama.cpp take it: always an object.
fn clean_schema(schema: &Value) -> Value {
    let mut schema = match schema {
        Value::Object(m) => m.clone(),
        _ => serde_json::Map::new(),
    };
    schema.remove("$schema");
    schema.entry("type").or_insert(json!("object"));
    if schema["type"] == "object" {
        schema.entry("properties").or_insert(json!({}));
    }
    Value::Object(schema)
}

/// `text` cut to about `max` characters, saying so.
fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((at, _)) => format!(
            "{}\n\n[Cut here: the full result is {} characters, too long for the model to read.]",
            &text[..at],
            text.chars().count()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_schemas_models_accept() {
        assert_eq!(clean_name("read file.v2"), "read_file_v2");
        assert_eq!(clean_name(&"x".repeat(80)).len(), 64);
        assert_eq!(
            clean_schema(
                &json!({ "$schema": "http://json-schema.org/draft-07/schema#", "type": "object", "properties": { "a": { "type": "string" } } })
            ),
            json!({ "type": "object", "properties": { "a": { "type": "string" } } })
        );
        assert_eq!(
            clean_schema(&Value::Null),
            json!({ "type": "object", "properties": {} })
        );
    }

    #[test]
    fn long_results_are_cut() {
        assert_eq!(cut("short", 10), "short");
        let long = cut(&"é".repeat(30), 10);
        assert!(long.starts_with(&"é".repeat(10)));
        assert!(long.contains("30 characters"));
    }

    #[test]
    fn checks_servers() {
        let ok = validate(McpServer {
            id: " Files ".into(),
            name: "".into(),
            enabled: true,
            members: false,
            transport: McpTransport::Stdio {
                command: " npx ".into(),
                args: vec!["-y".into(), "".into(), "pkg".into()],
                env: Default::default(),
                cwd: Some(" ".into()),
            },
        })
        .unwrap();
        assert_eq!(ok.id, "files");
        assert_eq!(ok.name, "files");
        assert!(matches!(
            &ok.transport,
            McpTransport::Stdio { command, args, cwd: None, .. } if command == "npx" && args.len() == 2
        ));
        let bad_url = McpServer {
            transport: McpTransport::Http {
                url: "example.org/mcp".into(),
                headers: Default::default(),
            },
            ..ok.clone()
        };
        assert!(validate(bad_url).is_err());
        assert!(validate(McpServer {
            id: "no spaces".into(),
            ..ok
        })
        .is_err());
    }
}
