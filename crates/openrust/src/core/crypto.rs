//! Machine-bound encryption for API keys.
//!
//! Uses AES-256-GCM with a key derived from the machine-id + a fixed pepper.
//! The encrypted blob can only be decrypted on the same machine.
//!
//! Format: base64(nonce[12] || ciphertext+tag)
//!
//! On Linux/WSL: reads `/etc/machine-id`.
//! On Windows: TODO — fallback to hostname + OS version hash.

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, KeyInit, Nonce};
use sha2::{Digest, Sha256};

/// Pepper for key derivation (different from any other tool)
const KEY_PEPPER: &[u8] = b"openrust-cred-v1";

/// Derive a 256-bit AES key from the machine-id.
fn derive_key() -> Option<[u8; 32]> {
    let machine_id = read_machine_id()?;
    let mut hasher = Sha256::new();
    hasher.update(machine_id.as_bytes());
    hasher.update(KEY_PEPPER);
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    Some(key)
}

/// Read the machine identifier.
/// Priority: `/etc/machine-id` → hostname fallback.
fn read_machine_id() -> Option<String> {
    if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
        let trimmed = id.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    // Fallback: hostname — less stable but works everywhere
    if let Ok(hostname) = std::process::Command::new("hostname").output() {
        let name = String::from_utf8_lossy(&hostname.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// Encrypt an API key. Returns base64-encoded ciphertext.
pub fn encrypt_api_key(plaintext: &str) -> Option<String> {
    let key = derive_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    let ciphertext = cipher.encrypt(&nonce, plaintext.as_bytes()).ok()?;

    // Pack: nonce (12 bytes) + ciphertext (variable)
    let mut packed = Vec::with_capacity(12 + ciphertext.len());
    packed.extend_from_slice(&nonce);
    packed.extend_from_slice(&ciphertext);

    use base64::Engine;
    Some(base64::engine::general_purpose::STANDARD.encode(&packed))
}

/// Decrypt an API key from a base64-encoded ciphertext.
/// Returns None if the key cannot be derived (wrong machine) or decryption fails.
pub fn decrypt_api_key(encrypted: &str) -> Option<String> {
    let key = derive_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;

    use base64::Engine;
    let packed = base64::engine::general_purpose::STANDARD
        .decode(encrypted)
        .ok()?;

    if packed.len() < 12 {
        return None;
    }

    let nonce = Nonce::from_slice(&packed[..12]);
    let ciphertext = &packed[12..];

    let plaintext = cipher.decrypt(nonce, ciphertext).ok()?;
    String::from_utf8(plaintext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let original = "sk-test-key-12345";
        let encrypted = encrypt_api_key(original).expect("encrypt should succeed");
        let decrypted = decrypt_api_key(&encrypted).expect("decrypt should succeed");
        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_same_key_different_ciphertext() {
        let key = "sk-abcdef";
        let enc1 = encrypt_api_key(key).unwrap();
        let enc2 = encrypt_api_key(key).unwrap();
        // Different nonces → different ciphertexts
        assert_ne!(enc1, enc2);
        // Both decrypt to the same value
        assert_eq!(decrypt_api_key(&enc1).unwrap(), key);
        assert_eq!(decrypt_api_key(&enc2).unwrap(), key);
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let encrypted = encrypt_api_key("my-secret").unwrap();
        let mut tampered = encrypted.clone();
        tampered.push('x');
        assert!(decrypt_api_key(&tampered).is_none());
    }
}
