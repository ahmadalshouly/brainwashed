//! Replies kept for a while after they finish, so a phone that lost its
//! connection mid-answer (the app went to the background, the network
//! changed) can pick up where it left off with `chatResume`.
//!
//! Each reply is a list of frames, the same `{"event"}` / `{"done"}` /
//! `{"error"}` values the chat stream sends. A reply is keyed by the device
//! that asked and an id the device chose, so one phone can't read another's.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// How long a finished reply stays available.
const KEEP_FOR: Duration = Duration::from_secs(15 * 60);
/// Most replies kept at once; the oldest finished ones go first.
const MAX_REPLIES: usize = 64;

#[derive(Default)]
struct Frames {
    list: Vec<Value>,
    finished_at: Option<Instant>,
}

/// One reply as it is being written.
pub(crate) struct Reply {
    frames: Mutex<Frames>,
    /// Bumped on every new frame so readers can wait for more.
    changed: watch::Sender<usize>,
    /// Set when the person pressed stop or, for a reply nobody can resume,
    /// when its reader went away.
    stop: tokio::sync::Notify,
}

impl Reply {
    pub(crate) fn new() -> Arc<Reply> {
        Arc::new(Reply {
            frames: Mutex::new(Frames::default()),
            changed: watch::channel(0).0,
            stop: tokio::sync::Notify::new(),
        })
    }

    /// Asks the writer to stop. Harmless once the reply finished.
    pub(crate) fn stop(&self) {
        self.stop.notify_one();
    }

    /// Resolves once [`Reply::stop`] was called, even if that was earlier.
    pub(crate) async fn stopped(&self) {
        self.stop.notified().await
    }

    /// Adds a frame. `last` marks the final `done` or `error` frame.
    pub(crate) fn push(&self, frame: Value, last: bool) {
        let len = {
            let mut f = self.frames.lock().unwrap();
            f.list.push(frame);
            if last {
                f.finished_at = Some(Instant::now());
            }
            f.list.len()
        };
        self.changed.send_replace(len);
    }

    fn finished_at(&self) -> Option<Instant> {
        self.frames.lock().unwrap().finished_at
    }

    /// Every frame from index `from` on, waiting for new ones until the reply ends.
    pub(crate) fn stream(self: Arc<Self>, from: usize) -> impl futures_util::Stream<Item = Value> {
        let rx = self.changed.subscribe();
        futures_util::stream::unfold((self, from, rx), |(reply, next, mut rx)| async move {
            loop {
                let (frame, finished) = {
                    let f = reply.frames.lock().unwrap();
                    (f.list.get(next).cloned(), f.finished_at.is_some())
                };
                if let Some(frame) = frame {
                    return Some((frame, (reply, next + 1, rx)));
                }
                // A frame pushed after the check above still wakes `changed`.
                if finished || rx.changed().await.is_err() {
                    return None;
                }
            }
        })
    }
}

/// Replies by device id and the id the device chose.
#[derive(Default)]
pub(crate) struct Replies {
    map: Mutex<HashMap<(String, String), Arc<Reply>>>,
}

/// Ids are short and plain so they can't be used to flood memory.
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Replies {
    pub(crate) fn insert(&self, device: &str, id: &str, reply: Arc<Reply>) {
        let mut map = self.map.lock().unwrap();
        map.retain(|_, r| r.finished_at().map_or(true, |t| t.elapsed() < KEEP_FOR));
        while map.len() >= MAX_REPLIES {
            let oldest = map
                .iter()
                .filter_map(|(k, r)| r.finished_at().map(|t| (t, k.clone())))
                .min_by_key(|(t, _)| *t)
                .map(|(_, k)| k);
            match oldest {
                Some(k) => map.remove(&k),
                // Every kept reply is still running; drop an arbitrary one.
                None => {
                    let k = map.keys().next().cloned().unwrap();
                    map.remove(&k)
                }
            };
        }
        map.insert((device.to_string(), id.to_string()), reply);
    }

    pub(crate) fn get(&self, device: &str, id: &str) -> Option<Arc<Reply>> {
        self.map
            .lock()
            .unwrap()
            .get(&(device.to_string(), id.to_string()))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde_json::json;

    #[tokio::test]
    async fn readers_get_missed_frames_then_live_ones() {
        let reply = Reply::new();
        reply.push(json!({"event": 1}), false);
        reply.push(json!({"event": 2}), false);
        let late = tokio::spawn(reply.clone().stream(1).collect::<Vec<_>>());
        tokio::time::sleep(Duration::from_millis(20)).await;
        reply.push(json!({"event": 3}), false);
        reply.push(json!({"done": "x"}), true);
        let got = late.await.unwrap();
        assert_eq!(
            got,
            vec![
                json!({"event": 2}),
                json!({"event": 3}),
                json!({"done": "x"})
            ]
        );
        // After it finished, a reader gets the rest at once and the stream ends.
        let all: Vec<_> = reply.clone().stream(0).collect().await;
        assert_eq!(all.len(), 4);
        let none: Vec<_> = reply.stream(4).collect().await;
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn a_stop_before_waiting_is_not_lost() {
        let reply = Reply::new();
        reply.stop();
        tokio::time::timeout(Duration::from_secs(1), reply.stopped())
            .await
            .unwrap();
    }

    #[test]
    fn ids_are_checked_and_scoped_to_a_device() {
        assert!(valid_id("r-1_A"));
        assert!(!valid_id(""));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(&"x".repeat(65)));
        let replies = Replies::default();
        replies.insert("phone", "r1", Reply::new());
        assert!(replies.get("phone", "r1").is_some());
        assert!(replies.get("other", "r1").is_none());
    }
}
