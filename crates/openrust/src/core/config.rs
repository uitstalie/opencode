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

    /// Model for background agents (title, summary, memory-extract).
    /// Falls back to `model` when unset.
    #[serde(default)]
    pub background_model: Option<String>,

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

impl ProviderConfig {
    /// Deep-merge another provider config into self (other wins on conflict).
    fn merge_with(&mut self, other: ProviderConfig) {
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
        if other.base_url.is_some() {
            self.base_url = other.base_url;
        }
        if other.protocol.is_some() {
            self.protocol = other.protocol;
        }
        for (name, model) in other.models {
            self.models
                .entry(name)
                .and_modify(|existing| existing.merge_with(model.clone()))
                .or_insert(model);
        }
        if other.options.is_some() {
            self.options = other.options;
        }
        for (k, v) in other.headers {
            self.headers.insert(k, v);
        }
    }
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

    /// Static body fields always merged into the API request.
    #[serde(default)]
    pub options: Option<serde_json::Value>,

    /// Body fields merged when `reasoning_effort` is set at runtime.
    /// Free-form JSON — covers any provider-specific reasoning activation
    /// fields (e.g. `{"thinking":{"type":"enabled"}}`).
    #[serde(default)]
    pub reasoning_options: Option<serde_json::Value>,

    /// Whether to send `reasoning_effort` value in the body (default true).
    #[serde(default)]
    pub reasoning_send_effort: Option<bool>,

    /// Override the body key for max tokens (default `"max_tokens"`).
    #[serde(default)]
    pub max_tokens_key: Option<String>,

    /// Override the role for system messages (default `"system"`).
    #[serde(default)]
    pub system_role: Option<String>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

impl ModelConfig {
    /// Deep-merge another model config into self (other wins on conflict).
    fn merge_with(&mut self, other: ModelConfig) {
        if other.name.is_some() {
            self.name = other.name;
        }
        if other.variants.is_some() {
            self.variants = other.variants;
        }
        if other.limit.is_some() {
            self.limit = other.limit;
        }
        if other.options.is_some() {
            self.options = other.options;
        }
        if other.reasoning_options.is_some() {
            self.reasoning_options = other.reasoning_options;
        }
        if other.reasoning_send_effort.is_some() {
            self.reasoning_send_effort = other.reasoning_send_effort;
        }
        if other.max_tokens_key.is_some() {
            self.max_tokens_key = other.max_tokens_key;
        }
        if other.system_role.is_some() {
            self.system_role = other.system_role;
        }
        for (k, v) in other.headers {
            self.headers.insert(k, v);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    #[serde(default)]
    pub context: Option<u64>,

    #[serde(default)]
    pub output: Option<u64>,
}

/// Built-in provider definitions for well-known APIs.
///
/// These are complete `ProviderConfig` entries: base_url, protocol, models,
/// reasoning fields — everything except the API key (which comes from the
/// vault via `/connect key <provider> <key>`).
///
/// At load time, each built-in is inserted via `entry().or_insert()`, so a
/// user-defined provider with the same name **completely replaces** the
/// built-in — no field-level merge.
fn builtin_providers() -> HashMap<String, ProviderConfig> {
    let mut map: HashMap<String, ProviderConfig> = HashMap::new();

    // ── DeepSeek ──────────────────────────────────────
    let thinking_on = serde_json::json!({"thinking": {"type": "enabled"}});
    map.insert(
        "deepseek".into(),
        ProviderConfig {
            base_url: Some("https://api.deepseek.com/v1".into()),
            protocol: Some("openai".into()),
            models: [
                (
                    "deepseek-chat".into(),
                    ModelConfig {
                        name: Some("deepseek-chat".into()),
                        limit: Some(ModelLimit { context: Some(64_000), output: Some(8_192) }),
                        reasoning_options: Some(thinking_on.clone()),
                        ..Default::default()
                    },
                ),
                (
                    "deepseek-reasoner".into(),
                    ModelConfig {
                        name: Some("deepseek-reasoner".into()),
                        limit: Some(ModelLimit { context: Some(64_000), output: Some(32_768) }),
                        reasoning_options: Some(thinking_on.clone()),
                        ..Default::default()
                    },
                ),
                (
                    "deepseek-v4-pro".into(),
                    ModelConfig {
                        name: Some("deepseek-v4-pro".into()),
                        limit: Some(ModelLimit { context: Some(128_000), output: Some(8_192) }),
                        reasoning_options: Some(thinking_on),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
    );

    // ── GLM (Zhipu AI) ────────────────────────────────
    let glm_thinking = serde_json::json!({"thinking": {"type": "enabled", "clear_thinking": false}});
    map.insert(
        "glm".into(),
        ProviderConfig {
            base_url: Some("https://open.bigmodel.cn/api/paas/v4".into()),
            protocol: Some("openai".into()),
            models: [
                (
                    "glm-4.6".into(),
                    ModelConfig {
                        name: Some("glm-4.6".into()),
                        limit: Some(ModelLimit { context: Some(128_000), output: Some(16_384) }),
                        reasoning_options: Some(glm_thinking.clone()),
                        reasoning_send_effort: Some(false),
                        ..Default::default()
                    },
                ),
                (
                    "glm-4-plus".into(),
                    ModelConfig {
                        name: Some("glm-4-plus".into()),
                        limit: Some(ModelLimit { context: Some(128_000), output: Some(4_096) }),
                        reasoning_options: Some(glm_thinking),
                        reasoning_send_effort: Some(false),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
    );

    // ── Zhipu AI Coding Plan ──────────────────────────
    let coding_thinking = serde_json::json!({"thinking": {"type": "enabled", "clear_thinking": false}});
    map.insert(
        "zhipuai-coding-plan".into(),
        ProviderConfig {
            base_url: Some("https://open.bigmodel.cn/api/paas/v4".into()),
            protocol: Some("openai".into()),
            models: [
                (
                    "glm-5.1".into(),
                    ModelConfig {
                        name: Some("glm-5.1".into()),
                        limit: Some(ModelLimit { context: Some(128_000), output: Some(16_384) }),
                        reasoning_options: Some(coding_thinking),
                        reasoning_send_effort: Some(false),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
    );

    // ── OpenAI ────────────────────────────────────────
    let oai_reasoning = |name: &str, ctx: u64, out: u64| ModelConfig {
        name: Some(name.into()),
        limit: Some(ModelLimit { context: Some(ctx), output: Some(out) }),
        max_tokens_key: Some("max_completion_tokens".into()),
        system_role: Some("developer".into()),
        ..Default::default()
    };
    map.insert(
        "openai".into(),
        ProviderConfig {
            base_url: Some("https://api.openai.com/v1".into()),
            protocol: Some("openai".into()),
            models: [
                (
                    "gpt-4o".into(),
                    ModelConfig {
                        name: Some("gpt-4o".into()),
                        limit: Some(ModelLimit { context: Some(128_000), output: Some(16_384) }),
                        ..Default::default()
                    },
                ),
                (
                    "gpt-4o-mini".into(),
                    ModelConfig {
                        name: Some("gpt-4o-mini".into()),
                        limit: Some(ModelLimit { context: Some(128_000), output: Some(16_384) }),
                        ..Default::default()
                    },
                ),
                ("o1".into(), oai_reasoning("o1", 200_000, 100_000)),
                ("o1-mini".into(), oai_reasoning("o1-mini", 128_000, 65_536)),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
    );

    // ── Anthropic ─────────────────────────────────────
    map.insert(
        "anthropic".into(),
        ProviderConfig {
            base_url: Some("https://api.anthropic.com".into()),
            protocol: Some("anthropic".into()),
            models: [
                (
                    "claude-sonnet-4-5-20250514".into(),
                    ModelConfig {
                        name: Some("claude-sonnet-4-5-20250514".into()),
                        limit: Some(ModelLimit { context: Some(200_000), output: Some(16_384) }),
                        ..Default::default()
                    },
                ),
                (
                    "claude-haiku-4-5-20251001".into(),
                    ModelConfig {
                        name: Some("claude-haiku-4-5-20251001".into()),
                        limit: Some(ModelLimit { context: Some(200_000), output: Some(8_192) }),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
    );

    // ── Gemini ────────────────────────────────────────
    map.insert(
        "gemini".into(),
        ProviderConfig {
            base_url: Some("https://generativelanguage.googleapis.com/v1beta".into()),
            protocol: Some("gemini".into()),
            models: [
                (
                    "gemini-2.5-pro".into(),
                    ModelConfig {
                        name: Some("gemini-2.5-pro".into()),
                        limit: Some(ModelLimit { context: Some(1_048_576), output: Some(65_536) }),
                        ..Default::default()
                    },
                ),
                (
                    "gemini-2.5-flash".into(),
                    ModelConfig {
                        name: Some("gemini-2.5-flash".into()),
                        limit: Some(ModelLimit { context: Some(1_048_576), output: Some(65_536) }),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        },
    );

    map
}

impl Config {
    /// Load config from project and global paths.
    /// Also runs migration: any plaintext api_key in config files is moved
    /// to the encrypted vault (credentials.enc) and removed from config.
    pub fn load(project_dir: &Path) -> anyhow::Result<Self> {
        let mut config = Config::default();

        // Load global config (~/.config/openrust/config.json)
        let global_path = Self::global_config_path();
        if global_path.exists() {
            let c = Self::load_file(&global_path)?;
            Self::migrate_api_keys(&c, Some(&global_path));
            config.merge(c);
        }

        // Load project config — first match wins:
        //   .openrust/config.jsonc → .openrust/config.json → openrust.json (legacy)
        let project_base = project_dir.join(".openrust");
        let candidates = [
            project_base.join("config.jsonc"),
            project_base.join("config.json"),
            project_dir.join("openrust.json"),
        ];
        for project_path in &candidates {
            if project_path.exists() {
                let c = Self::load_file(project_path)?;
                Self::migrate_api_keys(&c, Some(project_path));
                config.merge(c);
                break;
            }
        }

        // Fill in built-in providers for any name not already configured.
        // User-defined providers completely replace built-ins of the same name.
        for (name, provider) in builtin_providers() {
            config.provider.entry(name).or_insert(provider);
        }

        Ok(config)
    }

    /// Path to the global config (separate from TS opencode)
    pub fn global_config_path() -> PathBuf {
        crate::core::platform::PlatformPaths::detect().global_config_path()
    }

    /// Migrate plaintext api_keys from ProviderConfig to the encrypted vault.
    /// Strips migrated keys from the source file (global or project).
    fn migrate_api_keys(config: &Config, source_path: Option<&Path>) {
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
        // Strip migrated keys from the source file
        if let Some(path) = source_path {
            strip_api_keys_from_file(path, &migrated);
        }
    }

    /// Load config from a single file
    pub fn load_file(path: &Path) -> anyhow::Result<Config> {
        let content = std::fs::read_to_string(path)?;
        // Strip JSONC comments (simple // and /* */)
        let stripped = strip_jsonc_comments(&content);
        Ok(serde_json::from_str(&stripped)?)
    }

    fn merge(&mut self, other: Config) {
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.background_model.is_some() {
            self.background_model = other.background_model;
        }
        for (name, incoming) in other.provider {
            self.provider
                .entry(name)
                .and_modify(|existing| existing.merge_with(incoming.clone()))
                .or_insert(incoming);
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

    /// Like `resolve_provider_model` but prefers `background_model` over `model`.
    pub fn resolve_background_provider_model(&self) -> Option<(String, String)> {
        let model_spec = self.background_model.as_deref().or(self.model.as_deref())?;
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
    pub fn validate(&self, vault: &crate::core::vault::Vault) -> Vec<String> {
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

    /// Save config to a file, preserving unknown top-level fields from the
    /// existing file. Only `model`, `provider`, and `presets` are written;
    /// any other keys present on disk are kept verbatim.
    pub fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut raw: serde_json::Value = if path.exists() {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            let stripped = strip_jsonc_comments(&content);
            serde_json::from_str(&stripped)
                .unwrap_or(serde_json::Value::Object(Default::default()))
        } else {
            serde_json::Value::Object(Default::default())
        };

        let serialized = serde_json::to_value(self)?;
        if let (Some(raw_obj), Some(ser_obj)) = (raw.as_object_mut(), serialized.as_object()) {
            for (key, value) in ser_obj {
                raw_obj.insert(key.clone(), value.clone());
            }
        }

        std::fs::write(path, serde_json::to_string_pretty(&raw)?)?;
        Ok(())
    }

    /// Convenience: save to the global config path.
    pub fn save_global(&self) -> anyhow::Result<()> {
        self.save_to_file(&Self::global_config_path())
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

/// Remove plaintext `api_key` fields for the given providers from a config file.
fn strip_api_keys_from_file(path: &Path, providers: &[String]) {
    if !path.exists() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let stripped = strip_jsonc_comments(&content);
    let Ok(mut raw) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return;
    };

    let mut changed = false;
    for provider in providers {
        if let Some(p) = raw.get_mut("provider").and_then(|pv| pv.get_mut(provider))
            && let Some(obj) = p.as_object_mut()
                && obj.remove("api_key").is_some()
        {
            changed = true;
        }
    }

    if changed
        && let Ok(json) = serde_json::to_string_pretty(&raw)
    {
        let _ = std::fs::write(path, json);
    }
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
    fn provider_deep_merge_preserves_models_from_both_layers() {
        let mut global = Config {
            model: None,
            background_model: None,
            provider: HashMap::from([(
                "shared".to_string(),
                ProviderConfig {
                    base_url: Some("https://global.example/v1".to_string()),
                    models: HashMap::from([
                        ("global-model".to_string(), ModelConfig::default()),
                    ]),
                    ..Default::default()
                },
            )]),
            presets: HashMap::new(),
        };

        let project = Config {
            model: Some("shared/project-model".to_string()),
            background_model: None,
            provider: HashMap::from([(
                "shared".to_string(),
                ProviderConfig {
                    base_url: Some("https://project.example/v1".to_string()),
                    models: HashMap::from([
                        ("project-model".to_string(), ModelConfig::default()),
                    ]),
                    ..Default::default()
                },
            )]),
            presets: HashMap::new(),
        };

        global.merge(project);

        let provider = global.get_provider("shared").unwrap();
        // Project base_url overrides global
        assert_eq!(
            provider.base_url.as_deref(),
            Some("https://project.example/v1")
        );
        // Deep merge: both models preserved
        assert!(provider.models.contains_key("global-model"));
        assert!(provider.models.contains_key("project-model"));
        assert_eq!(global.model.as_deref(), Some("shared/project-model"));
    }

    #[test]
    fn project_config_preserves_unknown_top_level_fields() {
        let dir = tempdir().unwrap();
        let home_dir = dir.path().join("home");
        let global_dir = home_dir.join(".config").join("openrust");
        let global_path = global_dir.join("config.json");

        fs::create_dir_all(&global_dir).unwrap();
        fs::write(
            &global_path,
            r#"{
  "model": "test-provider/test-model",
  "provider": {
    "test-provider": {
      "api_key": "test-key",
      "base_url": "https://example/v1",
      "models": {"test-model": {}}
    }
  },
  "unknown_field": { "nested": "value" },
  "another_unknown": 42
}"#,
        )
        .unwrap();

        let mut config = Config::load(dir.path()).unwrap();
        config.model = Some("test-provider/test-model".to_string());

        // Save should preserve unknown fields
        config.save_to_file(&global_path).unwrap();

        let saved = fs::read_to_string(&global_path).unwrap();
        assert!(saved.contains("unknown_field"));
        assert!(saved.contains("another_unknown"));
        assert!(saved.contains("nested"));
    }

    #[test]
    fn provider_api_key_falls_back_to_config_value() {
        let config = Config {
            model: Some("config-only-provider/deepseek-v4-pro".to_string()),
            background_model: None,
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
