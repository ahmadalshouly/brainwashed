//! Lets paired devices find this computer after its address changes.
//!
//! A free Cloudflare quick tunnel gets a new random address every time it
//! starts. While the gateway runs, this posts the current public address to an
//! address book (a tiny web service, `services/address-book`) under an id
//! derived from a secret only this computer has. Pairing codes carry the
//! lookup address, so a device whose saved address stops answering asks the
//! address book where the computer is now, instead of needing a new code.
//!
//! The address book only learns addresses. Calls stay end-to-end encrypted to
//! this computer's key, so a wrong answer can't make a device talk to anyone
//! else. A new secret (`Gateway::new_address`) makes old lookups dead ends.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio::task::AbortHandle;

/// The BrainWashed address book, used unless the `address_book` setting names
/// another. None until one is deployed.
pub const DEFAULT_ADDRESS_BOOK: Option<&str> = None;

/// Post the address again this often even when it hasn't changed, so the
/// address book doesn't forget this computer (it forgets after 30 days).
const REFRESH: Duration = Duration::from_secs(12 * 3600);
/// How often to check whether the public address changed.
const CHECK: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(600);

/// This computer's secret for the address book, and the public id it gives.
#[derive(Clone)]
pub(crate) struct AddressKey {
    key: String,
    pub id: String,
}

impl AddressKey {
    fn from_key(key: String) -> Self {
        let id = URL_SAFE_NO_PAD.encode(Sha256::digest(key.as_bytes()));
        AddressKey { key, id }
    }

    pub fn load_or_create(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(key) if key.trim().len() >= 43 => Ok(Self::from_key(key.trim().to_string())),
            Ok(_) => Self::create(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::create(path),
            Err(e) => Err(e),
        }
    }

    /// A new secret, replacing the saved one.
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let key = format!(
            "{}{}{}",
            crate::crypto::random_token(),
            crate::crypto::random_token(),
            crate::crypto::random_token()
        );
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::crypto::write_private(path, key.as_bytes())?;
        Ok(Self::from_key(key))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressBookStatus {
    /// The address book, e.g. `https://book.example`.
    pub book: String,
    /// Where devices look up this computer's address (JSON).
    pub lookup: String,
    /// A link that always opens this computer's current address, for
    /// browsers.
    pub link: String,
    /// The address the address book has for this computer, once posted.
    pub published: Option<String>,
    /// What went wrong posting it, if anything.
    pub error: Option<String>,
}

impl AddressBookStatus {
    pub fn new(book: &str, id: &str) -> Self {
        AddressBookStatus {
            book: book.to_string(),
            lookup: format!("{book}/a/{id}"),
            link: format!("{book}/go/{id}"),
            published: None,
            error: None,
        }
    }
}

/// The address book to use from the `address_book` setting: None uses the
/// default, `off` (or empty) turns it off.
pub fn book_url(setting: Option<&str>) -> Result<Option<String>, String> {
    match setting.map(str::trim) {
        None => Ok(DEFAULT_ADDRESS_BOOK.map(str::to_string)),
        Some("") | Some("off") => Ok(None),
        Some(url) => crate::relay::normalize_url(url).map(Some).map_err(|e| {
            e.replace("a relay address", "an address book")
                .replace("relay address", "address book")
        }),
    }
}

/// Keeps the address book up to date while it lives.
pub(crate) struct Publisher {
    task: AbortHandle,
    status: Arc<Mutex<AddressBookStatus>>,
    wake: Arc<Notify>,
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Reads the address devices should use now, if any.
pub(crate) type CurrentAddress = Arc<dyn Fn() -> Option<String> + Send + Sync>;

impl Publisher {
    pub fn start(book: String, key: AddressKey, current: CurrentAddress) -> Self {
        let status = Arc::new(Mutex::new(AddressBookStatus::new(&book, &key.id)));
        let wake = Arc::new(Notify::new());
        let task =
            tokio::spawn(run(book, key, current, status.clone(), wake.clone())).abort_handle();
        Publisher { task, status, wake }
    }

    pub fn status(&self) -> AddressBookStatus {
        self.status.lock().unwrap().clone()
    }

    /// Checks the address now rather than at the next tick.
    pub fn poke(&self) {
        self.wake.notify_one();
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("BrainWashed/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

async fn send(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    body: serde_json::Value,
) -> Result<(), String> {
    let res = client
        .request(method, url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("couldn't reach the address book: {e}"))?;
    if res.status().is_success() {
        return Ok(());
    }
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    let reason = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("it answered {status}"));
    Err(format!("the address book refused the address: {reason}"))
}

async fn run(
    book: String,
    key: AddressKey,
    current: CurrentAddress,
    status: Arc<Mutex<AddressBookStatus>>,
    wake: Arc<Notify>,
) {
    let client = client();
    let entry = format!("{book}/a/{}", key.id);
    let mut posted: Option<(String, Instant)> = None;
    let mut backoff = Duration::from_secs(15);
    loop {
        let due = current().filter(|url| match &posted {
            Some((last, at)) => last != url || at.elapsed() >= REFRESH,
            None => true,
        });
        let mut wait = CHECK;
        if let Some(url) = due {
            let body = serde_json::json!({ "key": key.key, "url": url });
            match send(&client, reqwest::Method::PUT, &entry, body).await {
                Ok(()) => {
                    tracing::info!("told the address book this computer is at {url}");
                    let mut s = status.lock().unwrap();
                    s.published = Some(url.clone());
                    s.error = None;
                    posted = Some((url, Instant::now()));
                    backoff = Duration::from_secs(15);
                }
                Err(e) => {
                    tracing::warn!("{e}");
                    status.lock().unwrap().error = Some(e);
                    wait = backoff;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = wake.notified() => {}
        }
    }
}

/// Removes this computer's entry, after it picked a new secret. Best effort:
/// the address book forgets it after 30 days anyway.
pub(crate) async fn forget(book: &str, key: &AddressKey) {
    let body = serde_json::json!({ "key": key.key });
    let url = format!("{book}/a/{}", key.id);
    if let Err(e) = send(&client(), reqwest::Method::DELETE, &url, body).await {
        tracing::warn!("{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_id_is_the_hash_of_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("address.key");
        let a = AddressKey::load_or_create(&path).unwrap();
        assert_eq!(a.id.len(), 43);
        assert!(a.key.len() >= 43);
        assert_eq!(AddressKey::load_or_create(&path).unwrap().id, a.id);
        let b = AddressKey::create(&path).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(AddressKey::load_or_create(&path).unwrap().id, b.id);
        // Same as the address book computes it: base64url(SHA-256(secret)).
        assert_eq!(
            AddressKey::from_key("abc".into()).id,
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
    }

    #[test]
    fn reads_the_setting() {
        assert_eq!(
            book_url(None).unwrap(),
            DEFAULT_ADDRESS_BOOK.map(String::from)
        );
        assert_eq!(book_url(Some("off")).unwrap(), None);
        assert_eq!(book_url(Some(" ")).unwrap(), None);
        assert_eq!(
            book_url(Some("https://book.example/")).unwrap().as_deref(),
            Some("https://book.example")
        );
        assert!(book_url(Some("ftp://x")).is_err());
    }
}
