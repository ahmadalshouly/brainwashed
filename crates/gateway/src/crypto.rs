//! Public-key encryption between the host and each paired phone.
//!
//! Every message is a NaCl `crypto_box` (X25519 + XSalsa20-Poly1305), so
//! phones can use any NaCl library (the app uses tweetnacl). The host's public
//! key reaches the phone through the pairing QR code, which is what stops a
//! machine in the middle from posing as the host.

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use crypto_box::aead::{Aead, AeadCore, OsRng};
use crypto_box::{PublicKey, SalsaBox, SecretKey};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("bad encoding")]
    Encoding,
    #[error("could not decrypt")]
    Decrypt,
}

/// An encrypted message: base64 nonce and ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub n: String,
    pub c: String,
}

#[derive(Clone)]
pub struct HostKeys {
    secret: SecretKey,
}

impl HostKeys {
    /// Loads the host's key, creating one the first time.
    pub fn load_or_create(path: &Path) -> std::io::Result<Self> {
        if let Ok(bytes) = std::fs::read(path) {
            if let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) {
                return Ok(HostKeys {
                    secret: SecretKey::from(arr),
                });
            }
        }
        let secret = SecretKey::generate(&mut OsRng);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(path, &secret.to_bytes())?;
        Ok(HostKeys { secret })
    }

    pub fn public_key(&self) -> PublicKey {
        self.secret.public_key()
    }

    pub fn public_key_b64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public_key().as_bytes())
    }

    /// Short id phones use to recognise this host.
    pub fn host_id(&self) -> String {
        self.public_key().as_bytes()[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    pub fn seal(&self, peer: &PublicKey, plaintext: &[u8]) -> Envelope {
        let cipher = SalsaBox::new(peer, &self.secret);
        let nonce = SalsaBox::generate_nonce(&mut OsRng);
        let c = cipher
            .encrypt(&nonce, plaintext)
            .expect("encryption with a valid key cannot fail");
        Envelope {
            n: STANDARD.encode(nonce),
            c: STANDARD.encode(c),
        }
    }

    pub fn open(&self, peer: &PublicKey, env: &Envelope) -> Result<Vec<u8>, CryptoError> {
        let nonce = STANDARD.decode(&env.n).map_err(|_| CryptoError::Encoding)?;
        if nonce.len() != 24 {
            return Err(CryptoError::Encoding);
        }
        let c = STANDARD.decode(&env.c).map_err(|_| CryptoError::Encoding)?;
        SalsaBox::new(peer, &self.secret)
            .decrypt(nonce.as_slice().into(), c.as_slice())
            .map_err(|_| CryptoError::Decrypt)
    }
}

pub fn parse_public_key(b64: &str) -> Result<PublicKey, CryptoError> {
    let bytes = STANDARD
        .decode(b64)
        .or_else(|_| URL_SAFE_NO_PAD.decode(b64))
        .map_err(|_| CryptoError::Encoding)?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| CryptoError::Encoding)?;
    Ok(PublicKey::from(arr))
}

pub fn random_token() -> String {
    let key = SecretKey::generate(&mut OsRng);
    URL_SAFE_NO_PAD.encode(&key.to_bytes()[..16])
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_between_two_parties() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostKeys::load_or_create(&dir.path().join("host.key")).unwrap();
        let phone = HostKeys::load_or_create(&dir.path().join("phone.key")).unwrap();

        let env = phone.seal(&host.public_key(), b"hello");
        assert_eq!(host.open(&phone.public_key(), &env).unwrap(), b"hello");

        // Anyone else can't read it.
        let other = HostKeys::load_or_create(&dir.path().join("other.key")).unwrap();
        assert!(other.open(&phone.public_key(), &env).is_err());
    }

    #[test]
    fn key_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("host.key");
        let a = HostKeys::load_or_create(&path).unwrap();
        let b = HostKeys::load_or_create(&path).unwrap();
        assert_eq!(a.public_key(), b.public_key());
        assert_eq!(a.host_id().len(), 16);
    }

    #[test]
    fn rejects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let host = HostKeys::load_or_create(&dir.path().join("h")).unwrap();
        let phone = HostKeys::load_or_create(&dir.path().join("p")).unwrap();
        let mut env = phone.seal(&host.public_key(), b"hello");
        env.c = STANDARD.encode(b"0123456789abcdefghijk");
        assert!(host.open(&phone.public_key(), &env).is_err());
    }
}
