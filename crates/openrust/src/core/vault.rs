//! Encrypted credentials store.
//!
//! Stores provider API keys in the platform-specific OpenRust config directory.
//! Keys are encrypted with AES-256-GCM bound to the machine-id.
//! File permissions are set to 0600 (owner read/write only).
//!
//! On first run, migrates any `api_key` fields from config.json into the
//! encrypted store and removes them from the config.

use std::collections::HashMap;

use crate::core::crypto;

/// In-memory cache of decrypted credentials (never persisted to disk in plaintext).
pub struct Vault {
    /// provider_name → api_key plaintext
    keys: HashMap<String, String>,
}

impl Vault {
    /// Load credentials from the encrypted file.
    /// Returns empty store if the file doesn't exist or can't be decrypted.
    pub fn load() -> Self {
        let path = Self::path();
        if !path.exists() {
            return Self {
                keys: HashMap::new(),
            };
        }

        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => {
                return Self {
                    keys: HashMap::new(),
                };
            }
        };

        let parsed: serde_json::Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => {
                return Self {
                    keys: HashMap::new(),
                };
            }
        };

        let mut keys = HashMap::new();
        if let Some(keys_obj) = parsed.get("keys").and_then(|k| k.as_object()) {
            for (provider, encrypted_val) in keys_obj {
                if let Some(encrypted) = encrypted_val.as_str() {
                    if let Some(plaintext) = crypto::decrypt_api_key(encrypted) {
                        keys.insert(provider.clone(), plaintext);
                    } else {
                        tracing::warn!(
                            "Could not decrypt API key for '{}' — wrong machine? Re-set with: openrust debug config set {} --api-key <key>",
                            provider,
                            provider
                        );
                    }
                }
            }
        }

        Self { keys }
    }

    /// Save a provider's API key to the encrypted store.
    pub fn save(provider: &str, api_key: &str) -> anyhow::Result<()> {
        let encrypted = crypto::encrypt_api_key(api_key)
            .ok_or_else(|| anyhow::anyhow!("Failed to encrypt API key"))?;

        // Load existing (to preserve other providers' keys)
        let mut store = Self::load_from_file()?;
        store.insert(provider.to_string(), encrypted);

        let json = serde_json::json!({
            "version": 1,
            "algorithm": "AES-256-GCM",
            "keys": store,
        });

        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;

        // Set permissions: 0600 (owner read/write only)
        Self::set_permissions(&path);

        // Remove api_key from config.json if present
        Self::strip_api_key_from_config(provider);

        Ok(())
    }

    /// Get the plaintext API key for a provider.
    pub fn get(&self, provider: &str) -> Option<&str> {
        self.keys.get(provider).map(|s| s.as_str())
    }

    /// Migrate any plaintext api_key from config.json into the encrypted store.
    /// Called once during config load. Returns list of migrated provider names.
    pub fn migrate_from_config(config_providers: &HashMap<String, String>) -> Vec<String> {
        let mut migrated = Vec::new();

        for (provider, api_key) in config_providers {
            if api_key.is_empty() {
                continue;
            }
            // Don't overwrite existing encrypted credentials
            if Self::exists_in_store(provider) {
                continue;
            }
            match Self::save(provider, api_key) {
                Ok(()) => migrated.push(provider.clone()),
                Err(e) => tracing::warn!("Failed to migrate API key for '{}': {}", provider, e),
            }
        }

        migrated
    }

    // ── Internal ──────────────────────────────────────

    pub fn path() -> std::path::PathBuf {
        crate::core::platform::PlatformPaths::detect().credentials_path()
    }

    fn load_from_file() -> anyhow::Result<HashMap<String, String>> {
        let path = Self::path();
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let data = std::fs::read_to_string(&path)?;
        let parsed: serde_json::Value = serde_json::from_str(&data)?;

        let mut keys = HashMap::new();
        if let Some(keys_obj) = parsed.get("keys").and_then(|k| k.as_object()) {
            for (provider, val) in keys_obj {
                if let Some(encrypted) = val.as_str() {
                    keys.insert(provider.clone(), encrypted.to_string());
                }
            }
        }
        Ok(keys)
    }

    fn exists_in_store(provider: &str) -> bool {
        Self::load_from_file()
            .map(|m| m.contains_key(provider))
            .unwrap_or(false)
    }

    /// Remove api_key field for a provider from config.json (mitigation after migration)
    fn strip_api_key_from_config(provider: &str) {
        use crate::core::config::Config;
        let path = Config::global_config_path();
        if !path.exists() {
            return;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(mut raw) = serde_json::from_str::<serde_json::Value>(&content) else {
            return;
        };

        if let Some(p) = raw.get_mut("provider").and_then(|pv| pv.get_mut(provider))
            && let Some(obj) = p.as_object_mut()
                && obj.remove("api_key").is_some()
                    && let Ok(json) = serde_json::to_string_pretty(&raw) {
                        let _ = std::fs::write(&path, json);
                        tracing::info!(
                            "Removed plaintext api_key from config.json for '{}'",
                            provider
                        );
                    }
    }

    #[cfg(unix)]
    fn set_permissions(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }

    #[cfg(not(unix))]
    fn set_permissions(_path: &std::path::Path) {
        // Windows: ACL-based, skipped for now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_get_returns_none_for_missing_provider() {
        let vault = Vault {
            keys: HashMap::new(),
        };
        assert!(vault.get("nonexistent").is_none());
    }

    #[test]
    fn vault_get_returns_stored_key() {
        let mut keys = HashMap::new();
        keys.insert("test-provider".to_string(), "secret-key-123".to_string());
        let vault = Vault { keys };
        assert_eq!(vault.get("test-provider"), Some("secret-key-123"));
    }

    #[test]
    fn vault_keys_are_stored_separately() {
        let mut keys = HashMap::new();
        keys.insert("provider-a".to_string(), "key-a".to_string());
        keys.insert("provider-b".to_string(), "key-b".to_string());
        let vault = Vault { keys };
        assert_eq!(vault.get("provider-a"), Some("key-a"));
        assert_eq!(vault.get("provider-b"), Some("key-b"));
        assert!(vault.get("provider-c").is_none());
    }
}
