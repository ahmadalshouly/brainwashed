//! A record of what admins changed and which devices paired, kept as JSON
//! lines in `<data dir>/gateway/audit.jsonl`. When the file passes 1 MB it is
//! moved to `audit.1.jsonl`, replacing the one before.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// Unix seconds.
    pub at: u64,
    /// The device that acted, or None for this computer's terminal.
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    /// The call, e.g. `deleteModel`, or `pair` for a new device.
    pub action: String,
    /// What it acted on, e.g. a model id or skill name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    pub fn new(path: &Path) -> Self {
        AuditLog {
            path: path.to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    pub fn record(&self, entry: &AuditEntry) {
        let _guard = self.lock.lock().unwrap();
        if let Err(e) = self.append(entry) {
            tracing::warn!("couldn't write the audit log: {e}");
        }
    }

    fn append(&self, entry: &AuditEntry) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() > MAX_BYTES) {
            std::fs::rename(&self.path, self.path.with_extension("1.jsonl"))?;
        }
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&line)
    }

    /// The newest entries, newest first.
    pub fn recent(&self, limit: usize) -> Vec<AuditEntry> {
        let _guard = self.lock.lock().unwrap();
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        text.lines()
            .rev()
            .filter_map(|l| serde_json::from_str(l).ok())
            .take(limit)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::new(&dir.path().join("audit.jsonl"));
        for (i, action) in ["pair", "deleteModel", "saveSkill"].iter().enumerate() {
            log.record(&AuditEntry {
                at: i as u64,
                device_id: Some("d".into()),
                device_name: Some("Phone".into()),
                action: action.to_string(),
                target: None,
                error: None,
            });
        }
        let recent = log.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].action, "saveSkill");
        assert_eq!(recent[1].action, "deleteModel");
    }
}
