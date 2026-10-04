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
}

impl ChatMessage {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        ChatMessage {
            role,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SamplingOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// A piece of a streamed answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "text", rename_all = "lowercase")]
pub enum Delta {
    /// Part of the visible answer.
    Content(String),
    /// Part of a reasoning model's thinking, shown separately.
    Reasoning(String),
}

#[derive(Serialize)]
struct Request<'a> {
    messages: &'a [ChatMessage],
    stream: bool,
    #[serde(flatten)]
    sampling: &'a SamplingOptions,
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: ChunkDelta,
}

#[derive(Deserialize, Default)]
struct ChunkDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
}

/// Streams a chat completion from `base_url` (e.g. `http://127.0.0.1:8080`),
/// calling `on_delta` for each piece, and returns the full visible answer.
pub async fn stream_chat(
    client: &reqwest::Client,
    base_url: &str,
    messages: &[ChatMessage],
    sampling: &SamplingOptions,
    mut on_delta: impl FnMut(Delta),
) -> Result<String> {
    let res = client
        .post(format!("{base_url}/v1/chat/completions"))
        .json(&Request {
            messages,
            stream: true,
            sampling,
        })
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
        return Err(Error::other(format!(
            "model server returned {status}: {body}"
        )));
    }

    let mut answer = String::new();
    let mut parser = SseParser::default();
    let mut stream = res.bytes_stream();
    while let Some(bytes) = stream.next().await {
        for data in parser.push(&bytes?) {
            if data == "[DONE]" {
                return Ok(answer);
            }
            let chunk: Chunk = serde_json::from_str(&data)?;
            for choice in chunk.choices {
                if let Some(text) = choice.delta.reasoning_content.filter(|t| !t.is_empty()) {
                    on_delta(Delta::Reasoning(text));
                }
                if let Some(text) = choice.delta.content.filter(|t| !t.is_empty()) {
                    answer.push_str(&text);
                    on_delta(Delta::Content(text));
                }
            }
        }
    }
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
