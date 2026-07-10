//! Configuration loading from `.openrust/config.jsonc` (project) and
//! `~/.config/openrust/config.json` (global).
//!
//! Supports JSONC format (comments allowed):
//! ```jsonc
//! {
//!   "model": "deepseek/deepseek-v4-pro",
//!   "provider": {
//!     "deepseek": { "baseURL": "...", "api_key": "sk-...", "models": {...} }
//!   }
//! }
//! ```
//!
//! API keys: stored in encrypted vault (credentials.enc), not in config.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Full openrust configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub model: Option<String>,

    #[serde(default)]
    pub provider: HashMap<String, ProviderConfig>,

    /// Custom tool presets. Keys overlay builtins (all/read_only/no_write/
    /// no_internet/none) or define new ones; values are tool-name lists.
    #[serde(default)]
    pub presets: HashMap<String, Vec<String>>,
}

/// Per-provider configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub api_key: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", default, alias = "baseURL")]
    pub base_url: Option<String>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub models: HashMap<String, ModelConfig>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,

    /// Protocol: "openai" (default), "anthropic", or "gemini".
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub protocol: Option<String>,
}

/// Per-model configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelConfig {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub variants: Option<HashMap<String, serde_json::Value>>,

    #[serde(default)]
    pub limit: Option<ModelLimit>,

    #[serde(default)]
    pub options: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    #[serde(default)]
    pub context: Option<u64>,

    #[serde(default)]
    pub output: Option<u64>,
}

impl Config {
    /// Load config from project and global paths.
    /// Also runs migration: any plaintext api_key in config.json is moved
    /// to the encrypted vault (credentials.enc) and removed from config.
    pub fn load(project_dir: &Path) -> anyhow::Result<Self> {
        let mut config = Config::default();

        // Load global config (~/.config/openrust/config.json)
        let global_path = Self::global_config_path();
        if global_path.exists() {
            let c = Self::load_file(&global_path)?;
            Self::migrate_api_keys(&c);
            config.merge(c);
        }

        // Load project config (.openrust/config.jsonc)
        let project_path = project_dir.join(".openrust").join("config.jsonc");
        if project_path.exists() {
            let c = Self::load_file(&project_path)?;
            config.merge(c);
        }

        Ok(config)
    }

    /// Path to the global config (separate from TS opencode)
    pub fn global_config_path() -> PathBuf {
        crate::core::platform::PlatformPaths::detect().global_config_path()
    }

    /// Migrate plaintext api_keys from ProviderConfig to the encrypted vault.
    fn migrate_api_keys(config: &Config) {
        let mut to_migrate = HashMap::new();
        for (name, cfg) in &config.provider {
            if let Some(ref key) = cfg.api_key
                && !key.is_empty() {
                    to_migrate.insert(name.clone(), key.clone());
                }
        }
        if to_migrate.is_empty() {
            return;
        }
        let migrated = crate::core::vault::Vault::migrate_from_config(&to_migrate);
        for provider in &migrated {
            println!("🔐 Migrated API key for '{}' to encrypted vault.", provider);
        }
    }

    /// Load config from a single file
    pub fn load_file(path: &PathBuf) -> anyhow::Result<Config> {
        let content = std::fs::read_to_string(path)?;
        // Strip JSONC comments (simple // and /* */)
        let stripped = strip_jsonc_comments(&content);
        Ok(serde_json::from_str(&stripped)?)
    }

    fn merge(&mut self, other: Config) {
        if other.model.is_some() {
            self.model = other.model;
        }
        for (k, v) in other.provider {
            self.provider.insert(k, v);
        }
        for (k, v) in other.presets {
            self.presets.insert(k, v);
        }
    }

    /// Resolve the effective model string (provider/model_id format).
    pub fn resolve_model(&self) -> Option<String> {
        self.model.clone()
    }

    /// Resolve the configured provider and wire model name for API calls.
    pub fn resolve_provider_model(&self) -> Option<(String, String)> {
        let model_spec = self.model.as_deref()?;
        let (provider_name, model_name, variant) = parse_model_spec(model_spec);
        let provider = self.provider.get(provider_name)?;
        let wire_model = variant
            .or_else(|| {
                provider
                    .models
                    .get(model_name)
                    .and_then(|m| m.name.as_deref())
            })
            .unwrap_or(model_name);
        Some((provider_name.to_string(), wire_model.to_string()))
    }

    /// Validate config at startup, returning a list of errors for missing keys/models.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let Some(model_spec) = self.model.as_deref() else {
            errors.push("No model configured. Set `model` in config to e.g. \"deepseek/deepseek-v4-pro\".".to_string());
            return errors;
        };
        let (provider_name, model_name, _) = parse_model_spec(model_spec);
        let Some(provider) = self.provider.get(provider_name) else {
            errors.push(format!(
                "Provider '{}' not found in config. Add it under `provider.{}`.",
                provider_name, provider_name
            ));
            return errors;
        };
        // Check API key — either in vault or config
        let vault = crate::core::vault::Vault::load();
        let has_key = vault.get(provider_name).is_some() || provider.api_key.is_some();
        if !has_key {
            let vault_path = crate::core::vault::Vault::path().display().to_string();
            errors.push(format!(
                "No API key for provider '{}'. Set `provider.{}.api_key` in config, \
                 or store it encrypted with: openrust debug vault set {} <key> \
                 (vault: {})",
                provider_name, provider_name, provider_name, vault_path
            ));
        }
        if !provider.models.contains_key(model_name) {
            errors.push(format!(
                "Model '{}' not found in provider '{}'. Check `provider.{}.models`.",
                model_name, provider_name, provider_name
            ));
        }
        errors
    }

    /// Resolve the model's context window size from config, falling back to
    /// an aggressive default for well-known models.
    pub fn resolve_context_window(&self) -> u64 {
        let model_spec = self.model.as_deref().unwrap_or("");
        let (provider_name, model_name, _) = parse_model_spec(model_spec);
        self.provider
            .get(provider_name)
            .and_then(|p| p.models.get(model_name))
            .and_then(|m| m.limit.as_ref())
            .and_then(|l| l.context)
            .unwrap_or_else(|| default_context_window(model_name))
    }

    /// Get provider config by name, resolving API key from:
    ///   1. Encrypted vault (credentials.enc)
    ///   2. Config file `api_key` field
    pub fn get_provider(&self, name: &str) -> Option<ResolvedProvider> {
        let cfg = self.provider.get(name)?;

        let vault = crate::core::vault::Vault::load();
        let api_key = vault
            .get(name)
            .map(|s| s.to_string())
            .or_else(|| cfg.api_key.clone());

        // base_url: config value > options.baseURL
        let base_url = cfg.base_url.clone().or_else(|| {
            cfg.options
                .as_ref()?
                .get("baseURL")?
                .as_str()
                .map(|s| s.to_string())
        });

        Some(ResolvedProvider {
            name: name.to_string(),
            api_key,
            base_url,
            models: cfg.models.clone(),
            options: cfg.options.clone(),
            headers: cfg.headers.clone(),
            protocol: cfg.protocol.clone(),
        })
    }

    /// Get the config directory (shared by config.json and credentials.enc)
    pub fn global_config_dir() -> PathBuf {
        crate::core::platform::PlatformPaths::detect()
            .config_dir()
            .clone()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedProvider {
    pub name: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub models: HashMap<String, ModelConfig>,
    pub options: Option<serde_json::Value>,
    pub headers: HashMap<String, String>,
    pub protocol: Option<String>,
}

/// Parse "provider/model" or "provider/model/variant" string
pub fn parse_model_spec(spec: &str) -> (&str, &str, Option<&str>) {
    let parts: Vec<&str> = spec.splitn(3, '/').collect();
    match parts.len() {
        1 => (parts[0], parts[0], None),
        2 => (parts[0], parts[1], None),
        _ => (parts[0], parts[1], Some(parts[2])),
    }
}

/// Default context window sizes for well-known models when config is absent.
pub fn default_context_window(model: &str) -> u64 {
    let lower = model.to_lowercase();
    if lower.contains("gpt-4") || lower.contains("gpt-4o") { return 128_000 }
    if lower.contains("claude-3") || lower.contains("claude-4") || lower.contains("sonnet") || lower.contains("opus") { return 200_000 }
    if lower.contains("deepseek-v3") || lower.contains("deepseek-v4") { return 128_000 }
    if lower.contains("deepseek-r1") || lower.contains("deepseek-reasoner") { return 128_000 }
    if lower.contains("gemini-2") { return 1_000_000 }
    if lower.contains("gemini") { return 32_000 }
    if lower.contains("qwen") { return 32_000 }
    if lower.contains("llama-3") || lower.contains("llama3") { return 128_000 }
    if lower.contains("mixtral") { return 32_000 }
    128_000 // default
}

/// Strip // line comments and /* */ block comments from JSONC
pub fn strip_jsonc_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '/' {
            match chars.peek() {
                Some('/') => {
                    // Line comment: skip until newline
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    // Block comment: skip until */
                    chars.next();
                    let mut depth = 1;
                    while let Some(c) = chars.next() {
                        if c == '/' && chars.peek() == Some(&'*') {
                            depth += 1;
                        } else if c == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                    }
                }
                _ => result.push(ch),
            }
        } else if ch == '"' {
            // Preserve string contents verbatim
            result.push(ch);
            let iter = chars.by_ref();
            while let Some(c) = iter.next() {
                result.push(c);
                if c == '"' {
                    break;
                }
                if c == '\\'
                    && let Some(esc) = iter.next() {
                        result.push(esc);
                    }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_strip_line_comment() {
        let input = "{\n  // comment\n  \"key\": \"val\"\n}";
        let stripped = strip_jsonc_comments(input);
        assert!(stripped.contains("\"key\""));
        assert!(!stripped.contains("comment"));
    }

    #[test]
    fn test_strip_block_comment() {
        let input = "{ /* block */ \"key\": \"val\" }";
        let stripped = strip_jsonc_comments(input);
        assert!(stripped.contains("\"key\""));
        assert!(!stripped.contains("block"));
    }

    #[test]
    fn test_strip_nested_block_comment() {
        let input = "{ /* outer /* inner */ still */ \"key\": 1 }";
        let stripped = strip_jsonc_comments(input);
        assert!(stripped.contains("\"key\""));
    }

    #[test]
    fn test_preserve_string_with_slashes() {
        let input = r#"{"url": "https://example.com/path"}"#;
        let stripped = strip_jsonc_comments(input);
        assert_eq!(stripped, input);
    }

    #[test]
    fn test_parse_model_spec() {
        assert_eq!(
            parse_model_spec("deepseek/deepseek-chat"),
            ("deepseek", "deepseek-chat", None)
        );
        assert_eq!(
            parse_model_spec("openai/gpt-5.5/high"),
            ("openai", "gpt-5.5", Some("high"))
        );
        assert_eq!(parse_model_spec("gpt-4"), ("gpt-4", "gpt-4", None));
    }

    #[test]
    fn global_config_dir_is_stable_path() {
        let dir = Config::global_config_dir();
        assert!(dir.ends_with("openrust"));
    }

    #[test]
    fn global_config_path_points_to_config_json() {
        let path = Config::global_config_path();
        assert!(path.ends_with("config.json"));
    }

    #[test]
    fn project_config_overlays_global_config() {
        let dir = tempdir().unwrap();
        let home_dir = dir.path().join("home");
        let global_dir = home_dir.join(".config").join("openrust");
        let global_path = global_dir.join("config.json");
        let project_dir = dir.path().join(".openrust");
        let project_path = project_dir.join("config.jsonc");

        fs::create_dir_all(&global_dir).unwrap();
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(
            &global_path,
            r#"{
  "model": "overlay-provider/global-model",
  "provider": {
    "overlay-provider": {
      "baseURL": "https://global.example/v1",
      "models": {"global-model": {"name": "global-model"}}
    }
  }
}"#,
        )
        .unwrap();
        fs::write(
            &project_path,
            r#"{
  "model": "overlay-provider/project-model",
  "provider": {
    "overlay-provider": {
      "baseURL": "https://project.example/v1",
      "api_key": "project-key",
      "models": {"project-model": {"name": "project-model"}}
    }
  }
}"#,
        )
        .unwrap();

        let config = Config::load(dir.path()).unwrap();
        let provider = config.get_provider("overlay-provider").unwrap();

        assert_eq!(
            config.model.as_deref(),
            Some("overlay-provider/project-model")
        );
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://project.example/v1")
        );
        assert_eq!(provider.api_key.as_deref(), Some("project-key"));
        assert!(provider.models.contains_key("project-model"));
    }

    #[test]
    fn provider_api_key_falls_back_to_config_value() {
        let config = Config {
            model: Some("config-only-provider/deepseek-v4-pro".to_string()),
            provider: HashMap::from([(
                "config-only-provider".to_string(),
                ProviderConfig {
                    api_key: Some("config-key".to_string()),
                    base_url: Some("https://example/v1".to_string()),
                    ..Default::default()
                },
            )]),
            presets: HashMap::new(),
        };

        let provider = config.get_provider("config-only-provider").unwrap();
        assert_eq!(provider.api_key.as_deref(), Some("config-key"));
    }

    #[test]
    fn resolve_provider_model_returns_none_when_config_missing() {
        let config = Config::default();
        assert!(config.resolve_provider_model().is_none());
    }
}
