//! AES-GCM encryption for the provider API key, keyed off a per-install
//! random 32-byte key persisted at `install_key_file`.

use std::path::Path;

use aes_gcm::{
    aead::{Aead, OsRng},
    Aes256Gcm, Key, KeyInit, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;

use crate::error::{Error, Result};

/// 32-byte machine-bound install key. Generated on first start and stored
/// at `install_key_file` with mode 0600 (best-effort on non-Unix).
#[derive(Clone)]
pub struct InstallKey {
    bytes: [u8; 32],
}

impl InstallKey {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let raw = std::fs::read(path)?;
            if raw.len() != 32 {
                return Err(Error::Internal("install_key wrong length".into()));
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&raw);
            return Ok(Self { bytes });
        }
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(path, perms)?;
        }
        Ok(Self { bytes })
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }
}

/// Encrypt a UTF-8 string with the install key. Returns
/// `base64(nonce || ciphertext)`.
pub fn encrypt_value(key: &InstallKey, plaintext: &str) -> Result<String> {
    let k = Key::<Aes256Gcm>::from_slice(&key.bytes);
    let cipher = Aes256Gcm::new(k);
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| Error::Internal(format!("encrypt: {e}")))?;
    let mut buf = Vec::with_capacity(12 + ct.len());
    buf.extend_from_slice(&nonce_bytes);
    buf.extend_from_slice(&ct);
    Ok(B64.encode(buf))
}

pub fn decrypt_value(key: &InstallKey, blob: &str) -> Result<String> {
    let raw = B64
        .decode(blob.as_bytes())
        .map_err(|e| Error::Internal(format!("b64: {e}")))?;
    if raw.len() < 12 {
        return Err(Error::Internal("ciphertext too short".into()));
    }
    let (nonce_bytes, ct) = raw.split_at(12);
    let k = Key::<Aes256Gcm>::from_slice(&key.bytes);
    let cipher = Aes256Gcm::new(k);
    let nonce = Nonce::from_slice(nonce_bytes);
    let pt = cipher
        .decrypt(nonce, ct)
        .map_err(|e| Error::Internal(format!("decrypt: {e}")))?;
    String::from_utf8(pt).map_err(|e| Error::Internal(format!("utf8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let key = InstallKey::from_bytes([7u8; 32]);
        let blob = encrypt_value(&key, "sk-secret").unwrap();
        let back = decrypt_value(&key, &blob).unwrap();
        assert_eq!(back, "sk-secret");
    }

    #[test]
    fn different_keys_fail() {
        let k1 = InstallKey::from_bytes([1u8; 32]);
        let k2 = InstallKey::from_bytes([2u8; 32]);
        let blob = encrypt_value(&k1, "x").unwrap();
        assert!(decrypt_value(&k2, &blob).is_err());
    }

    #[test]
    fn install_key_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("install_key");
        let k1 = InstallKey::load_or_create(&p).unwrap();
        let k2 = InstallKey::load_or_create(&p).unwrap();
        assert_eq!(k1.bytes, k2.bytes);
    }
}
