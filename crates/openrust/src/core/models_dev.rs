//! models.dev dynamic provider catalog.
//!
//! Mirrors dev-ai-release's ModelsDev service (`packages/core/src/models-dev.ts`):
//! - Source: `GET https://models.dev/api.json` (`OPENRUST_MODELS_URL` override)
//! - Cache: `~/.cache/openrust/models.json`, 5-minute TTL, atomic tmp+rename write
//! - `OPENRUST_DISABLE_MODELS_FETCH` disables network access (static builtins only)
//! - `OPENRUST_MODELS_PATH` overrides the cache file path
//!
//! The catalog is converted into `ProviderConfig` entries at config load time.
//! Priority: user config > dynamic catalog > static builtins. The static
//! builtins remain as the offline/first-launch fallback.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::core::config::{ModelConfig, ModelLimit, ProviderConfig};
use crate::core::platform::PlatformPaths;

const DEFAULT_SOURCE: &str = "https://models.dev";
const TTL: Duration = Duration::from_secs(300);

// ── Wire schema (lenient: unknown fields ignored) ──

#[derive(Debug, Clone, Deserialize)]
pub struct DevProvider {
    pub api: Option<String>,
    pub npm: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, DevModel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevModel {
    #[serde(default)]
    pub reasoning: bool,
    pub limit: Option<DevLimit>,
    pub modalities: Option<DevModalities>,
    pub status: Option<String>,
    #[serde(default)]
    pub reasoning_options: Option<Vec<DevReasoningOption>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevLimit {
    pub context: Option<u64>,
    pub output: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DevModalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum DevReasoningOption {
    #[serde(rename = "toggle")]
    Toggle,
    #[serde(rename = "effort")]
    Effort {
        #[serde(default)]
        #[allow(dead_code)]
        values: Option<Vec<Option<String>>>,
    },
    #[serde(rename = "budget_tokens")]
    BudgetTokens {
        #[allow(dead_code)]
        min: Option<f64>,
        #[allow(dead_code)]
        max: Option<f64>,
    },
}

pub type Catalog = HashMap<String, DevProvider>;

// ── Cache ──

pub fn cache_path() -> PathBuf {
    if let Ok(p) = std::env::var("OPENRUST_MODELS_PATH") {
        return PathBuf::from(p);
    }
    PlatformPaths::detect().cache_dir().join("models.json")
}

fn source_url() -> String {
    std::env::var("OPENRUST_MODELS_URL").unwrap_or_else(|_| DEFAULT_SOURCE.into())
}

fn fetch_disabled() -> bool {
    std::env::var("OPENRUST_DISABLE_MODELS_FETCH").is_ok()
}

/// Load the cached catalog from disk. Returns `None` when missing or corrupt
/// (a corrupt file is removed so the next fetch rewrites it).
pub fn load_cached() -> Option<Catalog> {
    load_cached_from(&cache_path())
}

fn load_cached_from(path: &Path) -> Option<Catalog> {
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&content) {
        Ok(catalog) => Some(catalog),
        Err(e) => {
            tracing::warn!(error = %e, "models.dev cache corrupt, removing");
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

fn is_stale(path: &Path, now: SystemTime) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(mtime) = meta.modified() else {
        return true;
    };
    now.duration_since(mtime).map(|age| age >= TTL).unwrap_or(false)
}

/// Fetch the catalog and atomically replace the cache file.
async fn fetch_and_cache(path: &Path) -> anyhow::Result<()> {
    let url = format!("{}/api.json", source_url().trim_end_matches('/'));
    let client = crate::provider::build_http_client();
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..=2u32 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(200 * 2u64.pow(attempt))).await;
        }
        let result = tokio::time::timeout(Duration::from_secs(10), client.get(&url).send()).await;
        let text = match result {
            Ok(Ok(resp)) if resp.status().is_success() => match resp.text().await {
                Ok(t) => t,
                Err(e) => {
                    last_err = Some(e.into());
                    continue;
                }
            },
            Ok(Ok(resp)) => {
                last_err = Some(anyhow::anyhow!("HTTP {}", resp.status()));
                continue;
            }
            Ok(Err(e)) => {
                last_err = Some(e.into());
                continue;
            }
            Err(_) => {
                last_err = Some(anyhow::anyhow!("request timed out"));
                continue;
            }
        };
        // Validate before touching the cache.
        if serde_json::from_str::<Catalog>(&text).is_err() {
            return Err(anyhow::anyhow!("models.dev response failed to parse"));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&tmp, &text)?;
        std::fs::rename(&tmp, path)?;
        tracing::info!("models.dev catalog refreshed");
        return Ok(());
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("fetch failed")))
}

/// Spawn a background refresh when the cache is stale. Failures are logged
/// and the stale cache (or static builtins) keeps working.
pub fn spawn_refresh_if_stale() {
    if fetch_disabled() {
        return;
    }
    let path = cache_path();
    if !is_stale(&path, SystemTime::now()) {
        return;
    }
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::warn!(error = %e, "models.dev refresh: runtime creation failed");
                return;
            }
        };
        if let Err(e) = rt.block_on(fetch_and_cache(&path)) {
            tracing::warn!(error = %e, "models.dev refresh failed, using stale cache");
        }
    });
}

// ── Mapping to ProviderConfig ──

/// Map a models.dev provider to a `ProviderConfig`.
/// Returns `None` when the provider speaks an unsupported protocol.
pub fn to_provider_config(id: &str, p: &DevProvider) -> Option<ProviderConfig> {
    let protocol = protocol_for_npm(p.npm.as_deref()?)?;
    let models = p
        .models
        .iter()
        .filter(|(_, m)| is_chat_model(m))
        .map(|(mid, m)| (mid.clone(), to_model_config(id, protocol, m)))
        .collect();
    Some(ProviderConfig {
        base_url: p.api.clone(),
        protocol: Some(protocol.into()),
        models,
        ..Default::default()
    })
}

fn protocol_for_npm(npm: &str) -> Option<&'static str> {
    match npm {
        "@ai-sdk/anthropic" | "@ai-sdk/google-vertex/anthropic" => Some("anthropic"),
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => Some("gemini"),
        "@ai-sdk/openai" | "@ai-sdk/openai-compatible" | "@ai-sdk/azure" | "@ai-sdk/xai"
        | "@ai-sdk/mistral" => Some("openai"),
        _ => None,
    }
}

/// Keep only chat models: text output, positive context, not deprecated.
/// Filters embeddings, image/audio/video generation, and EOL models.
fn is_chat_model(m: &DevModel) -> bool {
    if m.status.as_deref() == Some("deprecated") {
        return false;
    }
    let Some(modalities) = &m.modalities else {
        return true;
    };
    if !modalities.output.is_empty() && !modalities.output.iter().any(|o| o == "text") {
        return false;
    }
    m.limit.as_ref().and_then(|l| l.context).unwrap_or(1) > 0
}

fn to_model_config(provider_id: &str, protocol: &str, m: &DevModel) -> ModelConfig {
    let mut cfg = ModelConfig {
        limit: m.limit.as_ref().map(|l| ModelLimit {
            input: None,
            context: l.context,
            output: l.output,
        }),
        image_input: m
            .modalities
            .as_ref()
            .map(|md| md.input.iter().any(|i| i == "image")),
        ..Default::default()
    };
    apply_reasoning(provider_id, protocol, m, &mut cfg);
    cfg
}

/// Reasoning wire config per protocol/provider, mirroring the subset of
/// dev-ai-release's transform.ts rules that rust providers consume.
fn apply_reasoning(provider_id: &str, protocol: &str, m: &DevModel, cfg: &mut ModelConfig) {
    if !m.reasoning {
        return;
    }
    let has_effort = m
        .reasoning_options
        .as_ref()
        .is_some_and(|opts| opts.iter().any(|o| matches!(o, DevReasoningOption::Effort { .. })));
    match protocol {
        // transform.ts: anthropic SDK → thinking enabled,
        // budget = min(16_000, output / 2 - 1).
        "anthropic" => {
            let output = m.limit.as_ref().and_then(|l| l.output).unwrap_or(32_768);
            let budget = (output / 2).saturating_sub(1).clamp(1_024, 16_000);
            cfg.reasoning_options = Some(
                serde_json::json!({"thinking": {"type": "enabled", "budget_tokens": budget}}),
            );
            cfg.reasoning_send_effort = Some(false);
        }
        // gemini.rs already injects thinkingConfig on the default path when
        // effort is set; send_effort only gates the /model thinking dialog.
        "gemini" => {
            cfg.reasoning_send_effort = Some(true);
        }
        _ => match provider_id {
            "deepseek" => {
                cfg.reasoning_options =
                    Some(serde_json::json!({"thinking": {"type": "enabled"}}));
                cfg.reasoning_send_effort = Some(has_effort);
            }
            "zhipuai" | "zhipuai-coding-plan" | "zai" | "zai-coding-plan" => {
                cfg.reasoning_options = Some(
                    serde_json::json!({"thinking": {"type": "enabled", "clear_thinking": false}}),
                );
                cfg.reasoning_send_effort = Some(has_effort);
            }
            "openai" | "azure" => {
                cfg.max_tokens_key = Some("max_completion_tokens".into());
                cfg.system_role = Some("developer".into());
                cfg.reasoning_send_effort = Some(true);
            }
            _ => {
                cfg.reasoning_send_effort = Some(has_effort);
            }
        },
    }
}

// ── Config integration ──

/// Overlay the cached models.dev catalog onto `config`.
///
/// `user_defined` lists providers present in user config files — those are
/// never touched. Static builtins are replaced (fresh data wins); the
/// builtin's base_url/protocol survive when the dynamic entry lacks them.
pub fn apply_catalog(
    providers: &mut HashMap<String, ProviderConfig>,
    user_defined: &HashSet<String>,
    catalog: &Catalog,
) {
    for (id, dev) in catalog {
        if user_defined.contains(id) {
            continue;
        }
        let Some(dynamic) = to_provider_config(id, dev) else {
            continue;
        };
        match providers.entry(id.clone()) {
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(dynamic);
            }
            std::collections::hash_map::Entry::Occupied(mut o) => {
                let existing = o.get_mut();
                let base_url = dynamic.base_url.or_else(|| existing.base_url.take());
                let protocol = dynamic.protocol.or_else(|| existing.protocol.take());
                *existing = ProviderConfig {
                    base_url,
                    protocol,
                    models: dynamic.models,
                    ..Default::default()
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> Catalog {
        let json = serde_json::json!({
            "kimi-for-coding": {
                "id": "kimi-for-coding",
                "api": "https://api.kimi.com/coding/v1",
                "npm": "@ai-sdk/anthropic",
                "env": ["KIMI_API_KEY"],
                "models": {
                    "k3": {
                        "id": "k3",
                        "reasoning": true,
                        "tool_call": true,
                        "limit": {"context": 1048576, "output": 131072},
                        "modalities": {"input": ["text", "image", "video"], "output": ["text"]},
                        "reasoning_options": [{"type": "toggle"}]
                    },
                    "kimi-k2-thinking": {
                        "id": "kimi-k2-thinking",
                        "reasoning": true,
                        "tool_call": true,
                        "limit": {"context": 262144, "output": 32768},
                        "modalities": {"input": ["text"], "output": ["text"]}
                    }
                }
            },
            "deepseek": {
                "id": "deepseek",
                "api": "https://api.deepseek.com",
                "npm": "@ai-sdk/openai-compatible",
                "models": {
                    "deepseek-v4-flash": {
                        "id": "deepseek-v4-flash",
                        "reasoning": true,
                        "limit": {"context": 1000000, "output": 384000},
                        "modalities": {"input": ["text"], "output": ["text"]},
                        "reasoning_options": [{"type": "toggle"}, {"type": "effort", "values": ["high", "max"]}]
                    },
                    "deepseek-chat": {
                        "id": "deepseek-chat",
                        "reasoning": false,
                        "limit": {"context": 1000000, "output": 384000},
                        "modalities": {"input": ["text"], "output": ["text"]}
                    }
                }
            },
            "mixed-nonchat": {
                "id": "mixed-nonchat",
                "npm": "@ai-sdk/openai-compatible",
                "models": {
                    "embed-1": {
                        "id": "embed-1",
                        "limit": {"context": 8191, "output": 3072},
                        "modalities": {"input": ["text"], "output": ["embedding"]}
                    },
                    "old-chat": {
                        "id": "old-chat",
                        "status": "deprecated",
                        "limit": {"context": 32000, "output": 4096},
                        "modalities": {"input": ["text"], "output": ["text"]}
                    },
                    "ok-chat": {
                        "id": "ok-chat",
                        "limit": {"context": 32000, "output": 4096},
                        "modalities": {"input": ["text", "image"], "output": ["text"]}
                    }
                }
            },
            "bedrock": {
                "id": "bedrock",
                "npm": "@ai-sdk/amazon-bedrock",
                "models": {}
            }
        });
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn protocol_whitelist() {
        assert_eq!(protocol_for_npm("@ai-sdk/anthropic"), Some("anthropic"));
        assert_eq!(protocol_for_npm("@ai-sdk/google"), Some("gemini"));
        assert_eq!(protocol_for_npm("@ai-sdk/openai-compatible"), Some("openai"));
        assert_eq!(protocol_for_npm("@openrouter/ai-sdk-provider"), None);
    }

    #[test]
    fn filters_non_chat_and_deprecated() {
        let catalog = sample_catalog();
        let cfg = to_provider_config("mixed-nonchat", &catalog["mixed-nonchat"]).unwrap();
        assert!(cfg.models.contains_key("ok-chat"));
        assert!(!cfg.models.contains_key("embed-1"));
        assert!(!cfg.models.contains_key("old-chat"));
        assert_eq!(cfg.models["ok-chat"].image_input, Some(true));
    }

    #[test]
    fn unsupported_protocol_skipped() {
        let catalog = sample_catalog();
        assert!(to_provider_config("bedrock", &catalog["bedrock"]).is_none());
    }

    #[test]
    fn anthropic_reasoning_budget_matches_transform() {
        let catalog = sample_catalog();
        let cfg = to_provider_config("kimi-for-coding", &catalog["kimi-for-coding"]).unwrap();
        assert_eq!(cfg.protocol.as_deref(), Some("anthropic"));
        let k3 = &cfg.models["k3"];
        // min(16_000, 131072 / 2 - 1) = 16_000
        assert_eq!(
            k3.reasoning_options
                .as_ref()
                .and_then(|v| v.pointer("/thinking/budget_tokens"))
                .and_then(|v| v.as_u64()),
            Some(16_000)
        );
        assert_eq!(k3.reasoning_send_effort, Some(false));
        assert_eq!(k3.image_input, Some(true));
        let thinking = &cfg.models["kimi-k2-thinking"];
        assert_eq!(thinking.image_input, Some(false));
    }

    #[test]
    fn openai_compatible_reasoning_rules() {
        let catalog = sample_catalog();
        let cfg = to_provider_config("deepseek", &catalog["deepseek"]).unwrap();
        let flash = &cfg.models["deepseek-v4-flash"];
        assert_eq!(
            flash.reasoning_options.as_ref().and_then(|v| v.pointer("/thinking/type")).and_then(|v| v.as_str()),
            Some("enabled")
        );
        // effort option present in catalog → send effort
        assert_eq!(flash.reasoning_send_effort, Some(true));
        // non-reasoning model gets no reasoning config
        let chat = &cfg.models["deepseek-chat"];
        assert!(chat.reasoning_options.is_none());
        assert!(chat.reasoning_send_effort.is_none());
    }

    #[test]
    fn apply_catalog_respects_priority() {
        let catalog = sample_catalog();
        let mut providers = HashMap::new();
        // static builtin present
        providers.insert(
            "kimi-for-coding".to_string(),
            ProviderConfig {
                base_url: Some("https://static-builtin.example".into()),
                protocol: Some("anthropic".into()),
                models: HashMap::from([("legacy".to_string(), ModelConfig::default())]),
                ..Default::default()
            },
        );
        let mut user_defined = HashSet::new();
        user_defined.insert("deepseek".to_string());
        providers.insert(
            "deepseek".to_string(),
            ProviderConfig {
                base_url: Some("https://user-defined.example".into()),
                ..Default::default()
            },
        );

        apply_catalog(&mut providers, &user_defined, &catalog);

        // static builtin replaced by dynamic models, dynamic base_url wins
        let kimi = &providers["kimi-for-coding"];
        assert_eq!(kimi.base_url.as_deref(), Some("https://api.kimi.com/coding/v1"));
        assert!(kimi.models.contains_key("k3"));
        assert!(!kimi.models.contains_key("legacy"));
        // user-defined untouched
        assert_eq!(
            providers["deepseek"].base_url.as_deref(),
            Some("https://user-defined.example")
        );
        assert!(providers["deepseek"].models.is_empty());
        // new provider added
        assert!(providers.contains_key("mixed-nonchat"));
    }

    #[test]
    fn stale_check_uses_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.json");
        assert!(is_stale(&path, SystemTime::now())); // missing → stale
        std::fs::write(&path, "{}").unwrap();
        assert!(!is_stale(&path, SystemTime::now())); // fresh
        let old = SystemTime::now() + TTL + Duration::from_secs(1);
        assert!(is_stale(&path, old)); // aged beyond TTL
    }

    #[test]
    fn corrupt_cache_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(load_cached_from(&path).is_none());
        assert!(!path.exists());
    }
}
