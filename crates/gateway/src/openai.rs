//! An OpenAI-compatible API, so any OpenAI client library or app can use the
//! models on this computer: `GET /v1/models` and `POST /v1/chat/completions`
//! (streaming or not). Requests carry an API key an admin created
//! (`Authorization: Bearer bw-…`). Chats get the same skills and admin chat
//! defaults as the web and phone apps.
//!
//! Unlike device calls, these aren't end-to-end encrypted: use them on the
//! local network or through the tunnel's HTTPS address. The relay doesn't
//! carry them.

// Handlers return a ready response as their error.
#![allow(clippy::result_large_err)]

use crate::api_keys::ApiKey;
use crate::devices::DeviceRole;
use crate::server::{now_secs, Gateway};
use crate::usage::UsageRecord;
use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use brainwashed_core::{
    Attachment, ChatEvent, ChatMessage, Engine, Role, SamplingOptions, LOCAL_MODEL,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// An OpenAI-style error: `{ "error": { "message", "type", "code" } }`.
fn error(status: StatusCode, kind: &str, code: Option<&str>, message: &str) -> Response {
    let body = json!({ "error": { "message": message, "type": kind, "code": code } });
    with_cors((status, Json(body)).into_response())
}

fn bad_request(message: &str) -> Response {
    error(
        StatusCode::BAD_REQUEST,
        "invalid_request_error",
        None,
        message,
    )
}

/// Lets web pages on other sites call the API. Requests carry a key, not
/// cookies, so this exposes nothing a page couldn't already reach with it.
fn with_cors(mut resp: Response) -> Response {
    let h = resp.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    resp
}

pub(crate) async fn preflight() -> Response {
    with_cors(StatusCode::NO_CONTENT.into_response())
}

fn authorize(gw: &Gateway, headers: &HeaderMap) -> Result<ApiKey, Response> {
    let secret = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or(v.strip_prefix("bearer ")))
        .map(str::trim);
    let Some(secret) = secret else {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            Some("missing_api_key"),
            "Send an API key as `Authorization: Bearer <key>`. An admin creates keys on the API page of the BrainWashed admin.",
        ));
    };
    gw.inner.api_keys.check(secret, now_secs()).ok_or_else(|| {
        error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            Some("invalid_api_key"),
            "This API key isn't valid. It may have been revoked.",
        )
    })
}

fn is_admin(key: &ApiKey) -> bool {
    key.role == DeviceRole::Admin
}

pub(crate) async fn models(State(gw): State<Gateway>, headers: HeaderMap) -> Response {
    let key = match authorize(&gw, &headers) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let data: Vec<Value> = gw
        .inner
        .engine
        .chat_models(is_admin(&key))
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id,
                "object": "model",
                "created": 0,
                "owned_by": m.provider.unwrap_or_else(|| "brainwashed".into()),
                "name": m.name,
            })
        })
        .collect();
    with_cors(Json(json!({ "object": "list", "data": data })).into_response())
}

#[derive(Deserialize)]
pub(crate) struct CompletionRequest {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<InMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    stream_options: Option<StreamOptions>,
    temperature: Option<f64>,
    top_p: Option<f64>,
    top_k: Option<u32>,
    min_p: Option<f64>,
    repeat_penalty: Option<f64>,
    presence_penalty: Option<f64>,
    seed: Option<i64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>,
    /// `none` turns thinking off; any other effort turns it on.
    reasoning_effort: Option<String>,
    /// BrainWashed's own switch for thinking, as in device chat options.
    reasoning: Option<bool>,
    n: Option<u32>,
}

#[derive(Deserialize)]
struct StreamOptions {
    #[serde(default)]
    include_usage: bool,
}

#[derive(Deserialize)]
struct InMessage {
    role: String,
    #[serde(default)]
    content: Option<Content>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Parts(Vec<Part>),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Part {
    Text {
        text: String,
    },
    ImageUrl {
        image_url: ImageUrl,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ImageUrl {
    url: String,
}

impl CompletionRequest {
    fn options(&self) -> SamplingOptions {
        let reasoning = self.reasoning.or_else(|| {
            self.reasoning_effort
                .as_deref()
                .map(|e| !e.eq_ignore_ascii_case("none"))
        });
        SamplingOptions {
            temperature: self.temperature,
            top_p: self.top_p,
            top_k: self.top_k,
            min_p: self.min_p,
            repeat_penalty: self.repeat_penalty,
            presence_penalty: self.presence_penalty,
            seed: self.seed,
            max_tokens: self.max_completion_tokens.or(self.max_tokens),
            reasoning,
        }
    }

    fn messages(&self) -> Result<Vec<ChatMessage>, String> {
        let mut out = Vec::with_capacity(self.messages.len());
        let mut pictures = 0;
        for m in &self.messages {
            let role = match m.role.as_str() {
                "system" | "developer" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                other => {
                    return Err(format!(
                    "messages with role `{other}` aren't supported; use system, user or assistant"
                ))
                }
            };
            let mut msg = ChatMessage::new(role, "");
            match &m.content {
                None => {}
                Some(Content::Text(t)) => msg.content = t.clone(),
                Some(Content::Parts(parts)) => {
                    for part in parts {
                        match part {
                            Part::Text { text } => {
                                if !msg.content.is_empty() {
                                    msg.content.push('\n');
                                }
                                msg.content.push_str(text);
                            }
                            Part::ImageUrl { image_url } => {
                                pictures += 1;
                                msg.attachments.push(picture(&image_url.url, pictures)?);
                            }
                            Part::Other => {
                                return Err(
                                    "only text and image_url content parts are supported".into()
                                )
                            }
                        }
                    }
                }
            }
            out.push(msg);
        }
        if !out.iter().any(|m| m.role == Role::User) {
            return Err("send at least one user message".into());
        }
        Ok(out)
    }
}

/// A picture sent as a `data:` URL.
fn picture(url: &str, n: usize) -> Result<Attachment, String> {
    let rest = url.strip_prefix("data:").ok_or(
        "pictures must be sent as data: URLs (data:image/png;base64,…); this computer doesn't fetch links",
    )?;
    let (meta, data) = rest.split_once(',').ok_or("bad data: URL")?;
    let mime = meta
        .strip_suffix(";base64")
        .ok_or("data: URLs must be base64 encoded")?;
    let ext = mime.strip_prefix("image/").unwrap_or("bin");
    Ok(Attachment::Image {
        name: format!("picture-{n}.{ext}"),
        mime: mime.to_string(),
        data: data.to_string(),
    })
}

/// `local` or `<provider>/<model>` for the engine, from what the client
/// asked for. The local model also answers to its own id and name, and to
/// no model at all.
fn resolve_model(
    engine: &Engine,
    requested: Option<&str>,
    admin: bool,
) -> Result<String, Response> {
    let models = engine.chat_models(admin);
    let requested = requested.map(str::trim).filter(|m| !m.is_empty());
    let Some(requested) = requested else {
        return Ok(LOCAL_MODEL.into());
    };
    if requested == LOCAL_MODEL || models.iter().any(|m| m.id == requested) {
        return Ok(requested.to_string());
    }
    if let Some(local) = models.iter().find(|m| m.id == LOCAL_MODEL) {
        let loaded = match engine.state() {
            brainwashed_core::EngineState::Ready { model } => Some(model),
            _ => None,
        };
        if local.name == requested || loaded.as_deref() == Some(requested) {
            return Ok(LOCAL_MODEL.into());
        }
    }
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    let hint = if ids.is_empty() {
        "No model is available yet: load one in the BrainWashed admin.".to_string()
    } else {
        format!("Use one of: {}.", ids.join(", "))
    };
    Err(error(
        StatusCode::NOT_FOUND,
        "invalid_request_error",
        Some("model_not_found"),
        &format!("The model `{requested}` isn't available. {hint}"),
    ))
}

fn engine_error(e: &brainwashed_core::Error) -> Response {
    match e {
        brainwashed_core::Error::Invalid(msg) => bad_request(msg),
        other => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            None,
            &other.to_string(),
        ),
    }
}

/// What a reply has done so far, for the usage log.
#[derive(Default)]
struct Progress {
    first_token_ms: Option<u64>,
    stats: Option<brainwashed_core::ReplyStats>,
}

impl Progress {
    fn see(&mut self, e: &ChatEvent, started: std::time::Instant) {
        match e {
            ChatEvent::Content { .. } | ChatEvent::Reasoning { .. } | ChatEvent::ToolCall(_) => {
                self.first_token_ms
                    .get_or_insert(started.elapsed().as_millis() as u64);
            }
            ChatEvent::Stats(s) => self.stats = Some(s.clone()),
            _ => {}
        }
    }
}

/// Writes a reply to the usage log when it ends, including when the caller
/// goes away and the reply is stopped.
struct Recorder {
    gw: Gateway,
    started: std::time::Instant,
    progress: std::sync::Arc<std::sync::Mutex<Progress>>,
    record: UsageRecord,
    finished: bool,
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let progress = self.progress.lock().unwrap();
        if let Some(s) = &progress.stats {
            self.record.prompt_tokens = s.prompt_tokens;
            self.record.completion_tokens = s.tokens;
            self.record.tokens_per_second = s.tokens_per_second;
        }
        self.record.first_token_ms = progress.first_token_ms;
        self.record.total_ms = self.started.elapsed().as_millis() as u64;
        if !self.finished {
            self.record.error = Some("the caller disconnected".into());
        }
        self.gw.inner.usage.record(&self.record);
    }
}

/// Stops the reply when the client goes away.
struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Piece {
    Event(ChatEvent),
    Done(Result<String, brainwashed_core::Error>),
}

pub(crate) async fn chat_completions(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let key = match authorize(&gw, &headers) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let req: CompletionRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return bad_request(&format!("bad request body: {e}")),
    };
    if req.n.is_some_and(|n| n != 1) {
        return bad_request("only n = 1 is supported");
    }
    let messages = match req.messages() {
        Ok(m) => m,
        Err(e) => return bad_request(&e),
    };
    let engine = gw.inner.engine.clone();
    let admin = is_admin(&key);
    let model = match resolve_model(&engine, req.model.as_deref(), admin) {
        Ok(m) => m,
        Err(resp) => return resp,
    };
    let options = req.options();
    // Checked here too, so a bad value is a 400 before any streaming starts.
    if let Err(e) = options
        .clone()
        .or(&engine.settings().chat_defaults)
        .validate()
    {
        return bad_request(&e.to_string());
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Piece>();
    let task_model = model.clone();
    let mut recorder = Recorder {
        gw: gw.clone(),
        started: std::time::Instant::now(),
        progress: Default::default(),
        record: UsageRecord {
            at: now_secs(),
            key_id: key.id.clone(),
            model: model.clone(),
            prompt_tokens: 0,
            completion_tokens: 0,
            tokens_per_second: 0.0,
            first_token_ms: None,
            total_ms: 0,
            stream: req.stream,
            error: None,
        },
        finished: false,
    };
    let task = tokio::spawn(async move {
        let events = tx.clone();
        let progress = recorder.progress.clone();
        let started = recorder.started;
        let result = engine
            .chat_with(
                &messages,
                &options,
                Some(&task_model),
                admin,
                false,
                move |e| {
                    progress.lock().unwrap().see(&e, started);
                    let _ = events.send(Piece::Event(e));
                },
            )
            .await;
        recorder.finished = true;
        if let Err(e) = &result {
            recorder.record.error = Some(e.to_string());
        }
        drop(recorder);
        let _ = tx.send(Piece::Done(result));
    });
    let guard = AbortOnDrop(task.abort_handle());

    let id = format!("chatcmpl-{}", crate::crypto::random_token());
    let created = now_secs();

    if !req.stream {
        let _guard = guard;
        let mut reasoning = String::new();
        let mut stats = None;
        let mut calls = Vec::new();
        while let Some(piece) = rx.recv().await {
            match piece {
                Piece::Event(ChatEvent::Reasoning { text }) => reasoning.push_str(&text),
                Piece::Event(ChatEvent::ToolCall(call)) => calls.push(call),
                Piece::Event(ChatEvent::Stats(s)) => stats = Some(s),
                Piece::Event(_) => {}
                Piece::Done(Err(e)) => return engine_error(&e),
                Piece::Done(Ok(mut answer)) => {
                    let (readable, as_text): (Vec<_>, Vec<_>) =
                        calls.into_iter().partition(|c| call_text(c).is_none());
                    for c in as_text {
                        answer.push_str(&format!("\n\n{}", call_text(&c).unwrap_or_default()));
                    }
                    let mut message = json!({ "role": "assistant", "content": answer });
                    if !reasoning.is_empty() {
                        message["reasoning_content"] = json!(reasoning);
                    }
                    let mut finish = finish_reason(stats.as_ref());
                    if !readable.is_empty() {
                        message["tool_calls"] = readable
                            .iter()
                            .enumerate()
                            .map(|(i, c)| wire_call(i, c, false))
                            .collect();
                        finish = "tool_calls";
                    }
                    let body = json!({
                        "id": id,
                        "object": "chat.completion",
                        "created": created,
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "message": message,
                            "finish_reason": finish,
                        }],
                        "usage": usage(stats.as_ref()),
                    });
                    return with_cors(Json(body).into_response());
                }
            }
        }
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            None,
            "the reply stopped unexpectedly",
        );
    }

    let include_usage = req.stream_options.is_some_and(|o| o.include_usage);
    let chunk = move |delta: Value, finish: Option<&str>| {
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        })
    };
    let stream = async_stream(rx, guard, chunk, include_usage);
    with_cors(
        Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-store, no-transform")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .unwrap(),
    )
}

fn finish_reason(stats: Option<&brainwashed_core::ReplyStats>) -> &'static str {
    if stats.is_some_and(|s| s.truncated) {
        "length"
    } else {
        "stop"
    }
}

/// Tool calls API clients get as text: ones that couldn't be read, and the
/// model asking the person something (API clients didn't offer that tool).
fn call_text(call: &brainwashed_core::runtime::toolcalls::ToolCall) -> Option<String> {
    if call.name.is_empty() {
        return Some(call.raw.clone().unwrap_or_default());
    }
    call.ask_user().map(|a| a.text())
}

/// A tool call the model made, in OpenAI's shape. Streamed calls carry
/// their index and come whole, in one chunk.
fn wire_call(
    i: usize,
    call: &brainwashed_core::runtime::toolcalls::ToolCall,
    streamed: bool,
) -> Value {
    let mut v = json!({
        "id": call.id,
        "type": "function",
        "function": { "name": call.name, "arguments": call.arguments.to_string() },
    });
    if streamed {
        v["index"] = json!(i);
    }
    v
}

fn usage(stats: Option<&brainwashed_core::ReplyStats>) -> Value {
    let (prompt, completion) = stats.map_or((0, 0), |s| (s.prompt_tokens, s.tokens));
    json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": prompt + completion,
    })
}

/// Server-sent events: a first chunk with the role, one per piece of the
/// answer (thinking as `reasoning_content`), a last one with the finish
/// reason, usage when asked for, then `[DONE]`.
fn async_stream(
    rx: tokio::sync::mpsc::UnboundedReceiver<Piece>,
    guard: AbortOnDrop,
    chunk: impl Fn(Value, Option<&str>) -> Value + Send + 'static,
    include_usage: bool,
) -> impl futures_util::Stream<Item = Result<String, std::convert::Infallible>> + Send {
    struct St<F> {
        rx: tokio::sync::mpsc::UnboundedReceiver<Piece>,
        _guard: AbortOnDrop,
        chunk: F,
        started: bool,
        stats: Option<brainwashed_core::ReplyStats>,
        calls: usize,
        finished: bool,
    }
    let state = St {
        rx,
        _guard: guard,
        chunk,
        started: false,
        stats: None,
        calls: 0,
        finished: false,
    };
    let sse = |v: &Value| format!("data: {v}\n\n");
    futures_util::stream::unfold(state, move |mut st| async move {
        if st.finished {
            return None;
        }
        if !st.started {
            st.started = true;
            let first = (st.chunk)(json!({ "role": "assistant", "content": "" }), None);
            return Some((Ok(sse(&first)), st));
        }
        loop {
            let out = match st.rx.recv().await {
                Some(Piece::Event(ChatEvent::Content { text })) => {
                    sse(&(st.chunk)(json!({ "content": text }), None))
                }
                Some(Piece::Event(ChatEvent::Reasoning { text })) => {
                    sse(&(st.chunk)(json!({ "reasoning_content": text }), None))
                }
                Some(Piece::Event(ChatEvent::ToolCall(call))) if call_text(&call).is_some() => {
                    let text = format!("\n\n{}", call_text(&call).unwrap_or_default());
                    sse(&(st.chunk)(json!({ "content": text }), None))
                }
                Some(Piece::Event(ChatEvent::ToolCall(call))) => {
                    let wire = wire_call(st.calls, &call, true);
                    st.calls += 1;
                    sse(&(st.chunk)(json!({ "tool_calls": [wire] }), None))
                }
                Some(Piece::Event(ChatEvent::Stats(s))) => {
                    st.stats = Some(s);
                    continue;
                }
                Some(Piece::Event(_)) => continue,
                Some(Piece::Done(Ok(_))) => {
                    st.finished = true;
                    let finish = if st.calls > 0 {
                        "tool_calls"
                    } else {
                        finish_reason(st.stats.as_ref())
                    };
                    let last = (st.chunk)(json!({}), Some(finish));
                    let mut out = sse(&last);
                    if include_usage {
                        let mut u = (st.chunk)(json!({}), None);
                        u["choices"] = json!([]);
                        u["usage"] = usage(st.stats.as_ref());
                        out.push_str(&sse(&u));
                    }
                    out + "data: [DONE]\n\n"
                }
                Some(Piece::Done(Err(e))) => {
                    st.finished = true;
                    let err =
                        json!({ "error": { "message": e.to_string(), "type": "server_error" } });
                    sse(&err) + "data: [DONE]\n\n"
                }
                None => return None,
            };
            return Some((Ok(out), st));
        }
    })
}
