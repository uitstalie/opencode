//! Configuration loading from opencode.json.
//!
//! Supports the existing opencode.json format:
//! ```json
//! {
//!   "model": "deepseek/deepseek-v4-pro",
//!   "provider": {
//!     "deepseek": { "baseURL": "...", "api_key": "sk-...", "models": {...} }
//!   }
//! }
//! ```
//!
//! API keys: config `api_key` field takes priority, then env `{NAME}_API_KEY`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Full opencode configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub model: Option<String>,

    #[serde(default)]
    pub mode: Option<String>,

    #[serde(default)]
    pub provider: HashMap<String, ProviderConfig>,
}

/// Per-provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(alias = "baseURL", default)]
    pub base_url: Option<String>,

    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,

    #[serde(default)]
    pub options: Option<serde_json::Value>,
}

/// Per-model configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub variants: Option<HashMap<String, serde_json::Value>>,

    #[serde(default)]
    pub limit: Option<ModelLimit>,

    #[serde(default)]
    pub options: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    #[serde(default)]
    pub context: Option<u64>,

    #[serde(default)]
    pub output: Option<u64>,
}

impl Config {
    /// Load config from project and global paths
    pub fn load(project_dir: &PathBuf) -> anyhow::Result<Self> {
        let mut config = Config::default();

        // Load global config
        if let Some(home) = dirs_fallback() {
            let global_path = PathBuf::from(&home)
                .join(".config")
                .join("opencode")
                .join("opencode.json");
            if global_path.exists() {
                if let Ok(c) = Self::load_file(&global_path) {
                    config.merge(c);
                }
            }
        }

        // Load project config
        let project_path = project_dir.join("opencode.json");
        if project_path.exists() {
            if let Ok(c) = Self::load_file(&project_path) {
                config.merge(c);
            }
        }

        Ok(config)
    }

    fn load_file(path: &PathBuf) -> anyhow::Result<Config> {
        let content = std::fs::read_to_string(path)?;
        // Strip JSONC comments (simple // and /* */)
        let stripped = strip_jsonc_comments(&content);
        Ok(serde_json::from_str(&stripped)?)
    }

    fn merge(&mut self, other: Config) {
        if other.model.is_some() {
            self.model = other.model;
        }
        if other.mode.is_some() {
            self.mode = other.mode;
        }
        for (k, v) in other.provider {
            self.provider.entry(k).or_insert(v);
        }
    }

    /// Resolve the effective model string (provider/model_id format)
    pub fn resolve_model(&self) -> Option<String> {
        self.model.clone()
    }

    /// Get provider config by name, resolving API key from env
    pub fn get_provider(&self, name: &str) -> Option<ResolvedProvider> {
        let cfg = self.provider.get(name)?;

        // API key: config value > provider-specific env > generic OPENAI_API_KEY
        let env_key = format!("{}_API_KEY", name.to_uppercase().replace('-', "_"));
        let api_key = cfg
            .api_key
            .clone()
            .or_else(|| std::env::var(&env_key).ok())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok());

        // base_url: config value > options.baseURL
        let base_url = cfg.base_url.clone().or_else(|| {
            cfg.options.as_ref()?.get("baseURL")?.as_str().map(|s| s.to_string())
        });

        Some(ResolvedProvider {
            name: name.to_string(),
            api_key,
            base_url,
            models: cfg.models.clone(),
            options: cfg.options.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub name: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub models: HashMap<String, ModelConfig>,
    pub options: Option<serde_json::Value>,
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

fn dirs_fallback() -> Option<String> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
}

/// Strip // line comments and /* */ block comments from JSONC
fn strip_jsonc_comments(input: &str) -> String {
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
                            if depth == 0 { break; }
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
                if c == '"' { break; }
                if c == '\\' {
                    if let Some(esc) = iter.next() {
                        result.push(esc);
                    }
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
        assert_eq!(parse_model_spec("deepseek/deepseek-chat"), ("deepseek", "deepseek-chat", None));
        assert_eq!(parse_model_spec("openai/gpt-5.5/high"), ("openai", "gpt-5.5", Some("high")));
        assert_eq!(parse_model_spec("gpt-4"), ("gpt-4", "gpt-4", None));
    }
}
