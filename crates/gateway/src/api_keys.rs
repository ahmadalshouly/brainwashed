//! Keys for the OpenAI-compatible API (`/v1`). Only a SHA-256 hash of each
//! key is stored; the key itself is shown once, when an admin creates it.

use crate::devices::DeviceRole;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use crypto_box::aead::rand_core::RngCore;
use crypto_box::aead::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Every key starts with this, so people (and secret scanners) can tell what
/// it is.
pub const KEY_PREFIX: &str = "bw-";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    /// What the key may use: admins' keys also reach cloud models that
    /// aren't shared with members.
    pub role: DeviceRole,
    /// The first characters of the key, to tell keys apart.
    pub hint: String,
    /// Unix seconds.
    pub created_at: u64,
    pub last_used: Option<u64>,
    /// SHA-256 of the key, hex. Never sent to clients.
    #[serde(skip_serializing, default)]
    hash: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    #[serde(flatten)]
    key: ApiKey,
    hash: String,
}

pub struct ApiKeyStore {
    path: PathBuf,
    keys: RwLock<Vec<ApiKey>>,
}

fn hash(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl ApiKeyStore {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let stored: Vec<Stored> = match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let keys = stored
            .into_iter()
            .map(|s| ApiKey {
                hash: s.hash,
                ..s.key
            })
            .collect();
        Ok(ApiKeyStore {
            path: path.to_path_buf(),
            keys: RwLock::new(keys),
        })
    }

    pub fn list(&self) -> Vec<ApiKey> {
        self.keys.read().unwrap().clone()
    }

    /// Makes a new key and returns it with the secret, which isn't kept.
    pub fn create(
        &self,
        name: &str,
        role: DeviceRole,
        now: u64,
    ) -> std::io::Result<(ApiKey, String)> {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let secret = format!("{KEY_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes));
        let mut id = [0u8; 9];
        OsRng.fill_bytes(&mut id);
        let name = name.trim();
        let key = ApiKey {
            id: URL_SAFE_NO_PAD.encode(id),
            name: if name.is_empty() {
                "API key".into()
            } else {
                name.chars().take(64).collect()
            },
            role,
            hint: format!("{}…", &secret[..KEY_PREFIX.len() + 4]),
            created_at: now,
            last_used: None,
            hash: hash(&secret),
        };
        let mut keys = self.keys.write().unwrap();
        keys.push(key.clone());
        self.save(&keys)?;
        Ok((key, secret))
    }

    pub fn revoke(&self, id: &str) -> std::io::Result<bool> {
        let mut keys = self.keys.write().unwrap();
        let before = keys.len();
        keys.retain(|k| k.id != id);
        let removed = keys.len() != before;
        if removed {
            self.save(&keys)?;
        }
        Ok(removed)
    }

    /// The key a request presented, if it's one of ours. Records its use in
    /// memory; persisted with the next change.
    pub fn check(&self, secret: &str, now: u64) -> Option<ApiKey> {
        if !secret.starts_with(KEY_PREFIX) {
            return None;
        }
        let wanted = hash(secret);
        let mut keys = self.keys.write().unwrap();
        // Comparing hashes, so the time taken says nothing about the key.
        let key = keys.iter_mut().find(|k| k.hash == wanted)?;
        key.last_used = Some(now);
        Some(key.clone())
    }

    fn save(&self, keys: &[ApiKey]) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let stored: Vec<Stored> = keys
            .iter()
            .map(|k| Stored {
                key: k.clone(),
                hash: k.hash.clone(),
            })
            .collect();
        crate::crypto::write_private(&self.path, &serde_json::to_vec_pretty(&stored)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_checked_by_hash_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-keys.json");
        let store = ApiKeyStore::open(&path).unwrap();
        let (key, secret) = store
            .create("  Laptop scripts ", DeviceRole::Member, 10)
            .unwrap();
        assert!(secret.starts_with("bw-") && secret.len() > 40);
        assert_eq!(key.name, "Laptop scripts");
        assert!(secret.starts_with(key.hint.trim_end_matches('…')));

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains(&secret));
        assert!(!serde_json::to_string(&key).unwrap().contains("hash"));

        let store = ApiKeyStore::open(&path).unwrap();
        assert_eq!(store.check(&secret, 20).unwrap().last_used, Some(20));
        assert!(store.check("bw-wrong", 20).is_none());
        assert!(store.check(&secret[1..], 20).is_none());

        assert!(store.revoke(&key.id).unwrap());
        assert!(store.check(&secret, 30).is_none());
        assert!(ApiKeyStore::open(&path).unwrap().list().is_empty());
    }
}
