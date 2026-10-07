//! Symmetric encryption for secrets at rest (env vars, OAuth tokens,
//! installation tokens). AES-256-GCM; stored as base64(nonce || ciphertext).
//!
//! devpush uses Fernet (AES-128-CBC + HMAC); Runway uses AES-256-GCM —
//! same threat model, no compat requirement since this is a new product.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct Crypto {
    cipher: Aes256Gcm,
}

impl Crypto {
    /// Key material: ENCRYPTION_KEY env var. Accepts a base64-encoded
    /// 32-byte key; any other string is hashed with SHA-256 to 32 bytes.
    pub fn new(key_material: &str) -> Result<Self> {
        let raw = base64::engine::general_purpose::STANDARD
            .decode(key_material)
            .ok()
            .filter(|b| b.len() == 32)
            .unwrap_or_else(|| Sha256::digest(key_material.as_bytes()).to_vec());
        if key_material.is_empty() {
            return Err(Error::Config(
                "ENCRYPTION_KEY is required to store secrets".into(),
            ));
        }
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&raw).map_err(|e| Error::Crypto(e.to_string()))?,
        })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
            .map_err(|e| Error::Crypto(e.to_string()))?;
        let mut blob = nonce_bytes.to_vec();
        blob.extend_from_slice(&ct);
        Ok(base64::engine::general_purpose::STANDARD.encode(blob))
    }

    pub fn decrypt(&self, encoded: &str) -> Result<String> {
        let blob = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        if blob.len() < 13 {
            return Err(Error::Crypto("ciphertext too short".into()));
        }
        let (nonce, ct) = blob.split_at(12);
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ct)
            .map_err(|e| Error::Crypto(e.to_string()))?;
        String::from_utf8(pt).map_err(|e| Error::Crypto(e.to_string()))
    }
}

/// sha256 hex digest — used for API key / deploy token lookup hashes.
pub fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = Crypto::new("test-key").unwrap();
        let ct = c.encrypt("secret value").unwrap();
        assert_ne!(ct, "secret value");
        assert_eq!(c.decrypt(&ct).unwrap(), "secret value");
    }

    #[test]
    fn wrong_key_fails() {
        let a = Crypto::new("key-a").unwrap();
        let b = Crypto::new("key-b").unwrap();
        let ct = a.encrypt("x").unwrap();
        assert!(b.decrypt(&ct).is_err());
    }

    #[test]
    fn base64_key_accepted() {
        let key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
        let c = Crypto::new(&key).unwrap();
        assert_eq!(c.decrypt(&c.encrypt("ok").unwrap()).unwrap(), "ok");
    }

    #[test]
    fn empty_key_rejected() {
        assert!(Crypto::new("").is_err());
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
