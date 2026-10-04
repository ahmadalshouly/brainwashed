//! Streaming chat completions against llama-server's OpenAI-compatible API.

use crate::{Error, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Pictures and documents sent with the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

impl ChatMessage {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        ChatMessage {
            role,
            content: content.into(),
            attachments: Vec::new(),
        }
    }

    /// The text the model reads: attached documents first, then the message.
    /// Pictures are named, for models that can't see them.
    pub fn text(&self) -> String {
        self.text_with(true)
    }

    fn text_with(&self, name_pictures: bool) -> String {
        let mut text = String::new();
        for a in &self.attachments {
            match a {
                Attachment::File { name, text: body } => {
                    let name = name.replace('"', "'");
                    text.push_str(&format!("<file name=\"{name}\">\n{body}\n</file>\n\n"));
                }
                Attachment::Image { name, .. } if name_pictures => {
                    text.push_str(&format!("[Picture: {name}]\n"));
                }
                Attachment::Image { .. } => {}
            }
        }
        text.push_str(&self.content);
        text
    }

    pub fn images(&self) -> impl Iterator<Item = &Attachment> {
        self.attachments
            .iter()
            .filter(|a| matches!(a, Attachment::Image { .. }))
    }

    /// The message as llama-server's OpenAI API takes it. Pictures become
    /// image parts, which need a model with a vision projector.
    fn to_wire(&self, with_images: bool) -> serde_json::Value {
        let images: Vec<&Attachment> = if with_images {
            self.images().collect()
        } else {
            Vec::new()
        };
        if images.is_empty() {
            return serde_json::json!({ "role": self.role, "content": self.text() });
        }
        let mut parts = Vec::new();
        for image in images {
            if let Attachment::Image { mime, data, .. } = image {
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:{mime};base64,{data}") },
                }));
            }
        }
        parts.push(serde_json::json!({ "type": "text", "text": self.text_with(false) }));
        serde_json::json!({ "role": self.role, "content": parts })
    }
}

/// Something sent along with a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Attachment {
    /// A picture, base64 encoded. The model sees it only if it has a vision
    /// projector; otherwise it reads `[Picture: name]`.
    Image {
        name: String,
        mime: String,
        data: String,
    },
    /// A document's text, which the model reads before the message.
    File { name: String, text: String },
}

/// How the model picks its words. Anything left out uses the model's own
/// defaults. Clients send these in camelCase.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SamplingOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Whether reasoning models think before answering. None keeps the
    /// model's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
}

impl SamplingOptions {
    /// Rejects values llama.cpp would misbehave with.
    pub fn validate(&self) -> Result<()> {
        let check = |name: &str, v: Option<f64>, lo: f64, hi: f64| match v {
            Some(v) if !(lo..=hi).contains(&v) => Err(Error::other(format!(
                "{name} must be between {lo} and {hi}"
            ))),
            _ => Ok(()),
        };
        check("temperature", self.temperature, 0.0, 2.0)?;
        check("top p", self.top_p, 0.0, 1.0)?;
        check("min p", self.min_p, 0.0, 1.0)?;
        check("repeat penalty", self.repeat_penalty, 0.5, 2.0)?;
        check("presence penalty", self.presence_penalty, -2.0, 2.0)?;
        if self.top_k.is_some_and(|k| k > 1000) {
            return Err(Error::other("top k must be 1000 or less"));
        }
        if self.max_tokens == Some(0) {
            return Err(Error::other("max tokens must be at least 1"));
        }
        Ok(())
    }

    /// The fields of the request body. Cloud providers reject llama.cpp's
    /// extra sampling fields, so they get only the standard OpenAI ones.
    fn to_params(&self, llama: bool) -> serde_json::Map<String, serde_json::Value> {
        use serde_json::json;
        let mut p = serde_json::Map::new();
        let mut put = |k: &str, v: serde_json::Value| {
            p.insert(k.to_string(), v);
        };
        if let Some(v) = self.temperature {
            put("temperature", json!(v));
        }
        if let Some(v) = self.top_p {
            put("top_p", json!(v));
        }
        if llama {
            if let Some(v) = self.top_k {
                put("top_k", json!(v));
            }
            if let Some(v) = self.min_p {
                put("min_p", json!(v));
            }
            if let Some(v) = self.repeat_penalty {
                put("repeat_penalty", json!(v));
            }
        }
        if let Some(v) = self.presence_penalty {
            put("presence_penalty", json!(v));
        }
        if let Some(v) = self.seed {
            put("seed", json!(v));
        }
        if let Some(v) = self.max_tokens {
            put("max_tokens", json!(v));
        }
        match self.reasoning {
            // Read by the chat templates of Qwen3, DeepSeek R1 distills and
            // others that can think or not.
            Some(on) if llama => put("chat_template_kwargs", json!({ "enable_thinking": on })),
            _ => {}
        }
        p
    }
}

/// How a reply went, from llama-server's timings.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyStats {
    pub prompt_tokens: u64,
    pub tokens: u64,
    pub tokens_per_second: f64,
    /// The reply hit the max tokens limit before it finished.
    pub truncated: bool,
}

/// A piece of a streamed answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    /// Part of the visible answer.
    Content(String),
    /// Part of a reasoning model's thinking, shown separately.
    Reasoning(String),
    /// Sent once at the end.
    Stats(ReplyStats),
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    timings: Option<Timings>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: ChunkDelta,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct ChunkDelta {
    content: Option<String>,
    /// llama.cpp and DeepSeek.
    reasoning_content: Option<String>,
    /// OpenRouter and others.
    reasoning: Option<String>,
}

/// Where a chat goes: the local llama-server or a cloud provider with an
/// OpenAI-compatible API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// llama-server: `http://127.0.0.1:PORT`. Providers: the API base that
    /// `/chat/completions` hangs off, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    pub api_key: Option<String>,
    /// The provider's model name. llama-server serves one model and ignores it.
    pub model: Option<String>,
    pub llama: bool,
}

impl Endpoint {
    pub fn llama(base_url: impl Into<String>) -> Self {
        Endpoint {
            base_url: base_url.into(),
            api_key: None,
            model: None,
            llama: true,
        }
    }

    fn url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        if self.llama {
            format!("{base}/v1{path}")
        } else {
            format!("{base}{path}")
        }
    }

    fn authorize(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) else {
            return req;
        };
        let req = req.bearer_auth(key);
        // Anthropic's model list wants its own headers; its chat endpoint
        // takes either.
        if self.base_url.contains("api.anthropic.com") {
            req.header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
        } else {
            req
        }
    }
}

/// Model names a provider offers, from its `/models` list.
pub async fn list_models(client: &reqwest::Client, endpoint: &Endpoint) -> Result<Vec<String>> {
    let res = endpoint
        .authorize(client.get(endpoint.url("/models")))
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;
    if !res.status().is_success() {
        let status = res.status();
        return Err(provider_error(
            status,
            &res.text().await.unwrap_or_default(),
        ));
    }
    let body: serde_json::Value = res.json().await?;
    let mut names: Vec<String> = body["data"]
        .as_array()
        .or(body["models"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().or(m["name"].as_str()))
        .map(|s| s.trim_start_matches("models/").to_string())
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// A readable error from a provider's reply.
fn provider_error(status: reqwest::StatusCode, body: &str) -> Error {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .or(v["error"].as_str())
                .or(v["message"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    match status.as_u16() {
        401 | 403 => Error::other(format!("The provider refused the API key: {detail}")),
        404 => Error::other(format!(
            "The provider doesn't know this model or address: {detail}"
        )),
        429 => Error::other(format!(
            "The provider is rate limiting or out of credit: {detail}"
        )),
        _ => Error::other(format!("The provider returned {status}: {detail}")),
    }
}

#[derive(Deserialize)]
struct Timings {
    #[serde(default)]
    prompt_n: u64,
    #[serde(default)]
    predicted_n: u64,
    #[serde(default)]
    predicted_per_second: f64,
}

/// Streams a chat completion, calling `on_delta` for each piece, and returns
/// the full visible answer. Pictures are sent only when `vision` is true.
pub async fn stream_chat(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    messages: &[ChatMessage],
    sampling: &SamplingOptions,
    vision: bool,
    mut on_delta: impl FnMut(Delta),
) -> Result<String> {
    let mut body = sampling.to_params(endpoint.llama);
    body.insert(
        "messages".into(),
        messages.iter().map(|m| m.to_wire(vision)).collect(),
    );
    body.insert("stream".into(), true.into());
    if let Some(model) = &endpoint.model {
        body.insert("model".into(), model.clone().into());
    }
    if !endpoint.llama {
        // Token counts for the stats, since providers send no timings.
        body.insert(
            "stream_options".into(),
            serde_json::json!({ "include_usage": true }),
        );
    }
    let started = std::time::Instant::now();
    let res = endpoint
        .authorize(client.post(endpoint.url("/chat/completions")))
        .json(&body)
        .send()
        .await?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        if body.contains("exceed_context_size_error") {
            return Err(Error::other(
                "This conversation no longer fits in the model's memory. Start a new chat, or raise the context size in settings.",
            ));
        }
        if !endpoint.llama {
            return Err(provider_error(status, &body));
        }
        return Err(Error::other(format!(
            "model server returned {status}: {body}"
        )));
    }

    let mut answer = String::new();
    let mut stats = ReplyStats::default();
    let mut first_token: Option<std::time::Instant> = None;
    let mut parser = SseParser::default();
    let mut stream = res.bytes_stream();
    'read: while let Some(bytes) = stream.next().await {
        for data in parser.push(&bytes?) {
            if data == "[DONE]" {
                break 'read;
            }
            let chunk: Chunk = serde_json::from_str(&data)?;
            if let Some(t) = chunk.timings {
                stats.prompt_tokens = t.prompt_n;
                stats.tokens = t.predicted_n;
                stats.tokens_per_second = t.predicted_per_second;
            } else if let Some(u) = chunk.usage {
                stats.prompt_tokens = u.prompt_tokens;
                stats.tokens = u.completion_tokens;
                let secs = first_token.unwrap_or(started).elapsed().as_secs_f64();
                if secs > 0.0 {
                    stats.tokens_per_second = u.completion_tokens as f64 / secs;
                }
            }
            for choice in chunk.choices {
                if choice.finish_reason.as_deref() == Some("length") {
                    stats.truncated = true;
                }
                let delta = choice.delta;
                if let Some(text) = delta
                    .reasoning_content
                    .or(delta.reasoning)
                    .filter(|t| !t.is_empty())
                {
                    first_token.get_or_insert_with(std::time::Instant::now);
                    on_delta(Delta::Reasoning(text));
                }
                if let Some(text) = delta.content.filter(|t| !t.is_empty()) {
                    first_token.get_or_insert_with(std::time::Instant::now);
                    answer.push_str(&text);
                    on_delta(Delta::Content(text));
                }
            }
        }
    }
    on_delta(Delta::Stats(stats));
    Ok(answer)
}

/// Incremental parser for `text/event-stream`, yielding each event's data.
#[derive(Default)]
pub struct SseParser {
    buf: String,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        while let Some(end) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=end).collect();
            let line = line.trim_end_matches(['\r', '\n']);
            if let Some(data) = line.strip_prefix("data:") {
                out.push(data.trim_start().to_string());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_parser_handles_split_chunks() {
        let mut p = SseParser::default();
        assert!(p.push(b"data: {\"a\"").is_empty());
        assert_eq!(
            p.push(b":1}\n\ndata: [DONE]\n\n"),
            vec!["{\"a\":1}", "[DONE]"]
        );
    }

    #[test]
    fn sse_parser_ignores_comments_and_crlf() {
        let mut p = SseParser::default();
        assert_eq!(p.push(b": keepalive\r\ndata: x\r\n\r\n"), vec!["x"]);
    }
}
