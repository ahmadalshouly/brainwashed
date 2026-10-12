//! Pictures and videos: the `generate` device call, and the OpenAI-style
//! `/v1/images/generations` and `/v1/videos` endpoints.

// Handlers return a ready response as their error.
#![allow(clippy::result_large_err)]

use crate::openai::{authorize, bad_request, engine_error, error, with_cors};
use crate::replies::{self as reply_store, Reply};
use crate::server::{now_secs, reject, stream_frames, Gateway, StopOnDrop};
use crate::usage::UsageRecord;
use axum::{
    extract::{Path as UrlPath, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine as _;
use brainwashed_core::{GeneratedMedia, MediaEvent, MediaKind, MediaModel, MediaRequest};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// ----- device call -----

/// `generate { request, replyId? }`: one encrypted JSON frame per line,
/// `{"event": MediaEvent}` while it works, then `{"done": GeneratedMedia}` or
/// `{"error": "…"}`. With a `replyId` it can be resumed with `chatResume`
/// and stopped with `chatStop`, like a chat reply. Stopped, it ends with
/// `{"error": "Stopped.", "stopped": true}`.
pub(crate) fn generate(
    gw: Gateway,
    device_id: &str,
    device_key: crypto_box::PublicKey,
    params: Value,
) -> Response {
    let req: MediaRequest = match serde_json::from_value(params["request"].clone()) {
        Ok(r) => r,
        Err(e) => return reject(StatusCode::BAD_REQUEST, &format!("bad request: {e}")),
    };
    let reply = Reply::new();
    let resumable = match params["replyId"].as_str() {
        Some(id) if reply_store::valid_id(id) => {
            gw.inner.replies.insert(device_id, id, reply.clone());
            true
        }
        Some(_) => return reject(StatusCode::BAD_REQUEST, "bad replyId"),
        None => false,
    };
    let engine = gw.inner.engine.clone();
    let writer = reply.clone();
    tokio::spawn(async move {
        let events = writer.clone();
        let work = engine.generate(&req, move |e| events.push(json!({ "event": e }), false));
        let frame = tokio::select! {
            result = work => match result {
                Ok(media) => json!({ "done": media }),
                Err(e) => json!({ "error": e.to_string() }),
            },
            // Dropping the work stops sd-cli.
            _ = writer.stopped() => json!({ "error": "Stopped.", "stopped": true }),
        };
        writer.push(frame, true);
    });
    let frames = reply.clone().stream(0);
    if resumable {
        return stream_frames(gw, device_key, frames);
    }
    let guard = StopOnDrop(reply);
    stream_frames(
        gw,
        device_key,
        futures_util::StreamExt::map(frames, move |f| {
            let _ = &guard;
            f
        }),
    )
}

// ----- OpenAI-style API -----

/// The installed model asked for, or the first one of that kind.
fn pick_model(
    gw: &Gateway,
    kind: MediaKind,
    requested: Option<&str>,
) -> Result<MediaModel, Response> {
    let models: Vec<MediaModel> = gw
        .inner
        .engine
        .media_models()
        .into_iter()
        .filter(|m| m.kind == kind)
        .collect();
    let requested = requested.map(str::trim).filter(|m| !m.is_empty());
    let found = match requested {
        Some(r) => models.iter().find(|m| m.id == r || m.name == r),
        None => models.first(),
    };
    found.cloned().ok_or_else(|| {
        let what = if kind == MediaKind::Image {
            "picture"
        } else {
            "video"
        };
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        let hint = if ids.is_empty() {
            format!("No {what} model is installed yet: download one on the Models page of the BrainWashed admin.")
        } else {
            format!("Use one of: {}.", ids.join(", "))
        };
        error(
            StatusCode::NOT_FOUND,
            "invalid_request_error",
            Some("model_not_found"),
            &match requested {
                Some(r) => format!("The {what} model `{r}` isn't installed. {hint}"),
                None => hint,
            },
        )
    })
}

/// `1024x768` → (1024, 768). `auto` and nothing leave it to the model.
fn parse_size(size: Option<&str>) -> Result<(Option<u32>, Option<u32>), Response> {
    match size.map(str::trim) {
        None | Some("") | Some("auto") => Ok((None, None)),
        Some(s) => {
            let (w, h) = s
                .split_once(['x', 'X'])
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                .ok_or_else(|| bad_request("size must look like 1024x1024"))?;
            Ok((Some(w), Some(h)))
        }
    }
}

fn record(
    gw: &Gateway,
    key_id: &str,
    model: &str,
    started: std::time::Instant,
    error: Option<String>,
) {
    gw.inner.usage.record(&UsageRecord {
        at: now_secs(),
        key_id: key_id.to_string(),
        model: model.to_string(),
        prompt_tokens: 0,
        completion_tokens: 0,
        tokens_per_second: 0.0,
        first_token_ms: None,
        total_ms: started.elapsed().as_millis() as u64,
        stream: false,
        error,
    });
}

#[derive(Deserialize)]
pub(crate) struct ImageRequest {
    model: Option<String>,
    prompt: String,
    n: Option<u32>,
    size: Option<String>,
    /// `b64_json` (the default here) or `url`, which gets a data: URL.
    response_format: Option<String>,
    // BrainWashed extras, named as in sd-cli.
    negative_prompt: Option<String>,
    steps: Option<u32>,
    cfg_scale: Option<f32>,
    seed: Option<i64>,
}

/// `POST /v1/images/generations`. One picture per request; the reply waits
/// until it's done.
pub(crate) async fn images(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let key = match authorize(&gw, &headers) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let req: ImageRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return bad_request(&format!("bad request body: {e}")),
    };
    if req.n.is_some_and(|n| n != 1) {
        return bad_request("only n = 1 is supported");
    }
    let model = match pick_model(&gw, MediaKind::Image, req.model.as_deref()) {
        Ok(m) => m,
        Err(resp) => return resp,
    };
    let (width, height) = match parse_size(req.size.as_deref()) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let request = MediaRequest {
        model: model.id.clone(),
        prompt: req.prompt,
        negative_prompt: req.negative_prompt,
        width,
        height,
        steps: req.steps,
        cfg_scale: req.cfg_scale,
        seed: req.seed,
        ..Default::default()
    };
    let started = std::time::Instant::now();
    // The work runs in its own task so a client that hangs up stops it.
    let engine = gw.inner.engine.clone();
    let task = tokio::spawn(async move { engine.generate(&request, |_| {}).await });
    let _abort = AbortOnDrop(task.abort_handle());
    let result = match task.await {
        Ok(r) => r,
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                None,
                "stopped",
            )
        }
    };
    match result {
        Ok(media) => {
            record(&gw, &key.id, &model.id, started, None);
            let mut item = json!({ "b64_json": media.data, "revised_prompt": media.prompt, "seed": media.seed });
            if req.response_format.as_deref() == Some("url") {
                item["url"] = json!(format!("data:{};base64,{}", media.mime, media.data));
            }
            with_cors(
                Json(json!({
                    "created": now_secs(),
                    "data": [item],
                    "size": format!("{}x{}", media.width, media.height),
                    "output_format": "png",
                }))
                .into_response(),
            )
        }
        Err(e) => {
            record(&gw, &key.id, &model.id, started, Some(e.to_string()));
            engine_error(&e)
        }
    }
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Videos take minutes, so like OpenAI's video API they are jobs: create
/// one, ask how it's going, then fetch the file.
#[derive(Default)]
pub(crate) struct VideoJobs {
    jobs: Mutex<VecDeque<Arc<Mutex<Job>>>>,
}

/// Jobs kept, finished ones dropped oldest first.
const MAX_JOBS: usize = 16;

struct Job {
    id: String,
    key_id: String,
    model: String,
    prompt: String,
    created_at: u64,
    completed_at: Option<u64>,
    /// 0 to 100.
    progress: u32,
    status: &'static str,
    error: Option<String>,
    result: Option<GeneratedMedia>,
    requested_size: Option<String>,
    task: Option<tokio::task::AbortHandle>,
}

impl Job {
    fn to_json(&self) -> Value {
        let mut v = json!({
            "id": self.id,
            "object": "video",
            "model": self.model,
            "status": self.status,
            "progress": self.progress,
            "created_at": self.created_at,
            "completed_at": self.completed_at,
            "prompt": self.prompt,
            "size": self.result.as_ref().map(|m| format!("{}x{}", m.width, m.height)).or(self.requested_size.clone()),
        });
        if let Some(m) = &self.result {
            let seconds = match (m.frames, m.fps) {
                (Some(f), Some(fps)) if fps > 0 => f as f64 / fps as f64,
                _ => 0.0,
            };
            v["seconds"] = json!(format!("{seconds:.1}"));
            v["seed"] = json!(m.seed);
            v["content_type"] = json!(m.mime);
        }
        if let Some(e) = &self.error {
            v["error"] = json!({ "message": e });
        }
        v
    }
}

impl VideoJobs {
    fn add(&self, job: Arc<Mutex<Job>>) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs.push_back(job);
        while jobs.len() > MAX_JOBS {
            let Some(pos) = jobs
                .iter()
                .position(|j| j.lock().unwrap().completed_at.is_some())
            else {
                break;
            };
            jobs.remove(pos);
        }
    }

    fn get(&self, id: &str, key_id: &str) -> Option<Arc<Mutex<Job>>> {
        self.jobs
            .lock()
            .unwrap()
            .iter()
            .find(|j| {
                let j = j.lock().unwrap();
                j.id == id && j.key_id == key_id
            })
            .cloned()
    }

    fn remove(&self, id: &str) {
        self.jobs
            .lock()
            .unwrap()
            .retain(|j| j.lock().unwrap().id != id);
    }
}

#[derive(Deserialize)]
pub(crate) struct VideoRequest {
    model: Option<String>,
    prompt: String,
    /// `832x480`; left out uses the model's size.
    size: Option<String>,
    /// Length, as OpenAI sends it: "4" (a string) or 4.
    seconds: Option<Value>,
    // BrainWashed extras.
    negative_prompt: Option<String>,
    frames: Option<u32>,
    fps: Option<u32>,
    steps: Option<u32>,
    seed: Option<i64>,
    /// A picture to set in motion, as a data: URL or plain base64.
    image: Option<String>,
}

/// `POST /v1/videos`: starts a video and answers at once with its job.
pub(crate) async fn create_video(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let key = match authorize(&gw, &headers) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let req: VideoRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return bad_request(&format!("bad request body (send JSON): {e}")),
    };
    let model = match pick_model(&gw, MediaKind::Video, req.model.as_deref()) {
        Ok(m) => m,
        Err(resp) => return resp,
    };
    let (width, height) = match parse_size(req.size.as_deref()) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let seconds = req.seconds.as_ref().and_then(|s| match s {
        Value::String(s) => s.parse::<f64>().ok(),
        other => other.as_f64(),
    });
    let fps = req.fps.or(model.defaults.fps).unwrap_or(16);
    let frames = req
        .frames
        .or_else(|| seconds.map(|s| (s * fps as f64).round() as u32 + 1));
    let image = req.image.map(|i| match i.split_once(";base64,") {
        Some((_, data)) => data.to_string(),
        None => i,
    });
    let request = MediaRequest {
        model: model.id.clone(),
        prompt: req.prompt.clone(),
        negative_prompt: req.negative_prompt,
        width,
        height,
        steps: req.steps,
        seed: req.seed,
        frames,
        fps: Some(fps),
        image,
        ..Default::default()
    };
    let job = Arc::new(Mutex::new(Job {
        id: format!(
            "video_{}",
            crate::crypto::random_token().to_ascii_lowercase()
        ),
        key_id: key.id.clone(),
        model: model.id.clone(),
        prompt: req.prompt,
        created_at: now_secs(),
        completed_at: None,
        progress: 0,
        status: "queued",
        error: None,
        result: None,
        requested_size: req.size,
        task: None,
    }));
    let engine = gw.inner.engine.clone();
    let worker = job.clone();
    let gw2 = gw.clone();
    let key_id = key.id.clone();
    let model_id = model.id.clone();
    let task = tokio::spawn(async move {
        let started = std::time::Instant::now();
        let seen = worker.clone();
        let result = engine
            .generate(&request, move |e| {
                let mut j = seen.lock().unwrap();
                match e {
                    MediaEvent::Queued => j.status = "queued",
                    MediaEvent::Installing { .. } => j.status = "in_progress",
                    MediaEvent::Progress(p) => {
                        j.status = "in_progress";
                        // Leave the last bit for decoding and writing the file.
                        if let Some(pct) = (p.step * 95).checked_div(p.steps) {
                            j.progress = pct.min(95);
                        }
                    }
                }
            })
            .await;
        let mut j = worker.lock().unwrap();
        j.completed_at = Some(now_secs());
        match result {
            Ok(media) => {
                j.status = "completed";
                j.progress = 100;
                j.result = Some(media);
                record(&gw2, &key_id, &model_id, started, None);
            }
            Err(e) => {
                j.status = "failed";
                j.error = Some(e.to_string());
                record(&gw2, &key_id, &model_id, started, Some(e.to_string()));
            }
        }
    });
    job.lock().unwrap().task = Some(task.abort_handle());
    gw.inner.video_jobs.add(job.clone());
    let body = job.lock().unwrap().to_json();
    with_cors(Json(body).into_response())
}

fn no_job(id: &str) -> Response {
    error(
        StatusCode::NOT_FOUND,
        "invalid_request_error",
        Some("not_found"),
        &format!("No video `{id}`. Videos are kept for a while after they finish."),
    )
}

/// `GET /v1/videos/{id}`.
pub(crate) async fn get_video(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> Response {
    let key = match authorize(&gw, &headers) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    match gw.inner.video_jobs.get(&id, &key.id) {
        Some(job) => with_cors(Json(job.lock().unwrap().to_json()).into_response()),
        None => no_job(&id),
    }
}

/// `DELETE /v1/videos/{id}`: stops it if it's still being made, and forgets it.
pub(crate) async fn delete_video(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> Response {
    let key = match authorize(&gw, &headers) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let Some(job) = gw.inner.video_jobs.get(&id, &key.id) else {
        return no_job(&id);
    };
    let mut j = job.lock().unwrap();
    if let Some(task) = j.task.take() {
        task.abort();
    }
    if j.completed_at.is_none() {
        j.status = "failed";
        j.error = Some("deleted".into());
    }
    let mut body = j.to_json();
    body["deleted"] = json!(true);
    drop(j);
    gw.inner.video_jobs.remove(&id);
    with_cors(Json(body).into_response())
}

/// `GET /v1/videos/{id}/content`: the finished video, an animated WebP.
pub(crate) async fn video_content(
    State(gw): State<Gateway>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> Response {
    let key = match authorize(&gw, &headers) {
        Ok(k) => k,
        Err(resp) => return resp,
    };
    let Some(job) = gw.inner.video_jobs.get(&id, &key.id) else {
        return no_job(&id);
    };
    let j = job.lock().unwrap();
    let Some(media) = &j.result else {
        return error(
            StatusCode::CONFLICT,
            "invalid_request_error",
            Some("video_not_ready"),
            &format!("The video is {}; ask again when it's completed.", j.status),
        );
    };
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&media.data) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            None,
            "bad video data",
        );
    };
    with_cors(
        (
            [
                (header::CONTENT_TYPE, media.mime.clone()),
                (
                    header::CONTENT_DISPOSITION,
                    format!("inline; filename=\"{}.webp\"", j.id),
                ),
            ],
            bytes,
        )
            .into_response(),
    )
}

/// What `/v1/models` lists besides chat models.
pub(crate) fn listed_models(gw: &Gateway) -> Vec<Value> {
    gw.inner
        .engine
        .media_models()
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id,
                "object": "model",
                "created": 0,
                "owned_by": "brainwashed",
                "name": m.name,
                "kind": m.kind,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn reads_sizes() {
        assert_eq!(
            parse_size(Some("832x480")).ok(),
            Some((Some(832), Some(480)))
        );
        assert_eq!(parse_size(Some("auto")).ok(), Some((None, None)));
        assert_eq!(parse_size(None).ok(), Some((None, None)));
        assert!(parse_size(Some("big")).is_err());
    }
}
