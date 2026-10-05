//! A record of every OpenAI-compatible API reply, for the usage charts on the
//! admin API page: who asked (which key), which model answered, tokens in and
//! out, speed, and how long until the first word. Kept as JSON lines in
//! `gateway/api-usage.jsonl` for 90 days. No message text is stored.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Records older than this are dropped when the log is opened.
pub const KEEP_SECS: u64 = 90 * 24 * 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    /// Unix seconds when the request arrived.
    pub at: u64,
    pub key_id: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Generation speed as the model reported it.
    pub tokens_per_second: f64,
    /// From the request to the first word (or thought), in milliseconds.
    pub first_token_ms: Option<u64>,
    /// From the request to the end of the reply.
    pub total_ms: u64,
    pub stream: bool,
    /// Set when the reply failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct UsageLog {
    path: PathBuf,
    lock: Mutex<()>,
}

/// Totals for a set of records.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub requests: u64,
    pub errors: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Average generation speed, weighted by tokens written.
    pub tokens_per_second: f64,
    /// Average time to the first word, over replies that had one.
    pub first_token_ms: Option<f64>,
    #[serde(skip)]
    speed_weight: f64,
    #[serde(skip)]
    first_token_sum: f64,
    #[serde(skip)]
    first_token_n: u64,
}

impl UsageTotals {
    fn add(&mut self, r: &UsageRecord) {
        self.requests += 1;
        if r.error.is_some() {
            self.errors += 1;
            return;
        }
        self.prompt_tokens += r.prompt_tokens;
        self.completion_tokens += r.completion_tokens;
        if r.tokens_per_second > 0.0 && r.completion_tokens > 0 {
            let w = r.completion_tokens as f64;
            self.tokens_per_second = (self.tokens_per_second * self.speed_weight
                + r.tokens_per_second * w)
                / (self.speed_weight + w);
            self.speed_weight += w;
        }
        if let Some(ms) = r.first_token_ms {
            self.first_token_sum += ms as f64;
            self.first_token_n += 1;
            self.first_token_ms = Some(self.first_token_sum / self.first_token_n as f64);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    /// Unix seconds at the start of the bucket.
    pub start: u64,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    /// A key id or a model id.
    pub id: String,
    #[serde(flatten)]
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub since: u64,
    pub until: u64,
    /// Seconds per bucket in `series`.
    pub bucket: u64,
    pub totals: UsageTotals,
    /// One entry per bucket from `since` to `until`, empty ones included.
    pub series: Vec<UsageBucket>,
    /// Busiest first.
    pub by_key: Vec<UsageGroup>,
    pub by_model: Vec<UsageGroup>,
}

impl UsageLog {
    /// Opens the log, dropping records older than [`KEEP_SECS`].
    pub fn open(path: &Path, now: u64) -> Self {
        let log = UsageLog {
            path: path.to_path_buf(),
            lock: Mutex::new(()),
        };
        let records = log.read();
        let cutoff = now.saturating_sub(KEEP_SECS);
        if records.first().is_some_and(|r| r.at < cutoff) {
            let kept: Vec<_> = records.into_iter().filter(|r| r.at >= cutoff).collect();
            let mut text = String::new();
            for r in &kept {
                text.push_str(&serde_json::to_string(r).unwrap_or_default());
                text.push('\n');
            }
            let _ = std::fs::write(path, text);
        }
        log
    }

    pub fn record(&self, r: &UsageRecord) {
        let _guard = self.lock.lock().unwrap();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let line = match serde_json::to_string(r) {
            Ok(l) => l + "\n",
            Err(_) => return,
        };
        let result = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
        if let Err(e) = result {
            tracing::warn!("couldn't record API usage: {e}");
        }
    }

    fn read(&self) -> Vec<UsageRecord> {
        let _guard = self.lock.lock().unwrap();
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Usage from `since` to `until`, in buckets of `bucket` seconds. Buckets
    /// start on whole hours or days in the viewer's time zone, `utc_offset`
    /// seconds ahead of UTC.
    pub fn summary(&self, since: u64, until: u64, bucket: u64, utc_offset: i64) -> UsageSummary {
        let bucket = bucket.max(60);
        let local = (since as i64 + utc_offset).max(0) as u64;
        let first = ((local / bucket * bucket) as i64 - utc_offset).max(0) as u64;
        let count = ((until.saturating_sub(first)) / bucket + 1) as usize;
        let mut series: Vec<UsageBucket> = (0..count)
            .map(|i| UsageBucket {
                start: first + i as u64 * bucket,
                totals: UsageTotals::default(),
            })
            .collect();
        let mut totals = UsageTotals::default();
        let mut by_key: BTreeMap<String, UsageTotals> = BTreeMap::new();
        let mut by_model: BTreeMap<String, UsageTotals> = BTreeMap::new();
        for r in self.read() {
            if r.at < since || r.at > until {
                continue;
            }
            totals.add(&r);
            by_key.entry(r.key_id.clone()).or_default().add(&r);
            by_model.entry(r.model.clone()).or_default().add(&r);
            let i = ((r.at - first) / bucket) as usize;
            if let Some(b) = series.get_mut(i) {
                b.totals.add(&r);
            }
        }
        let groups = |map: BTreeMap<String, UsageTotals>| {
            let mut v: Vec<UsageGroup> = map
                .into_iter()
                .map(|(id, totals)| UsageGroup { id, totals })
                .collect();
            v.sort_by(|a, b| {
                let size = |g: &UsageGroup| g.totals.prompt_tokens + g.totals.completion_tokens;
                size(b)
                    .cmp(&size(a))
                    .then(b.totals.requests.cmp(&a.totals.requests))
            });
            v
        };
        UsageSummary {
            since,
            until,
            bucket,
            totals,
            series,
            by_key: groups(by_key),
            by_model: groups(by_model),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(at: u64, key: &str, model: &str, out: u64, tps: f64) -> UsageRecord {
        UsageRecord {
            at,
            key_id: key.into(),
            model: model.into(),
            prompt_tokens: 10,
            completion_tokens: out,
            tokens_per_second: tps,
            first_token_ms: Some(100),
            total_ms: 1000,
            stream: false,
            error: None,
        }
    }

    #[test]
    fn sums_by_bucket_key_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.jsonl");
        let log = UsageLog::open(&path, 0);
        let h = 3600;
        log.record(&rec(10 * h + 5, "a", "local", 100, 10.0));
        log.record(&rec(10 * h + 9, "a", "local", 300, 30.0));
        log.record(&rec(12 * h, "b", "acme/fast", 50, 0.0));
        let mut failed = rec(12 * h + 1, "b", "acme/fast", 0, 0.0);
        failed.error = Some("boom".into());
        failed.first_token_ms = None;
        log.record(&failed);
        log.record(&rec(2 * h, "a", "local", 999, 1.0)); // before the range

        let s = log.summary(10 * h, 12 * h + 30, h, 0);
        assert_eq!(s.totals.requests, 4);
        assert_eq!(s.totals.errors, 1);
        assert_eq!(s.totals.completion_tokens, 450);
        assert_eq!(s.totals.prompt_tokens, 30);
        // Weighted by tokens: (100*10 + 300*30) / 400.
        assert!((s.totals.tokens_per_second - 25.0).abs() < 1e-9);
        assert_eq!(s.totals.first_token_ms, Some(100.0));
        assert_eq!(s.series.len(), 3);
        assert_eq!(s.series[0].totals.requests, 2);
        assert_eq!(s.series[1].totals.requests, 0);
        assert_eq!(s.series[2].totals.requests, 2);
        assert_eq!(s.by_key[0].id, "a");
        assert_eq!(s.by_model[1].id, "acme/fast");
        assert_eq!(s.by_model[1].totals.errors, 1);

        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["series"][0]["completionTokens"], 400);
        assert!(json["totals"].get("speedWeight").is_none());

        // Days start at local midnight: two hours ahead of UTC, the day
        // holding 06:00 UTC on day 1 starts at 22:00 UTC on day 0.
        let s = log.summary(30 * h, 40 * h, 24 * h, 2 * h as i64);
        assert_eq!(s.series[0].start, 22 * h);
    }

    #[test]
    fn old_records_are_dropped_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("u.jsonl");
        let log = UsageLog::open(&path, 0);
        log.record(&rec(1, "a", "local", 1, 1.0));
        log.record(&rec(KEEP_SECS + 100, "a", "local", 1, 1.0));
        let log = UsageLog::open(&path, KEEP_SECS + 200);
        assert_eq!(log.read().len(), 1);
    }
}
