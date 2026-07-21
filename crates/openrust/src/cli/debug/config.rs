use clap::Subcommand;
use std::path::PathBuf;

use crate::core::config::Config;

#[derive(Subcommand)]
pub enum Cmd {
    /// Show parsed configuration (secrets masked)
    Show,
    /// Show config file paths
    Path,
    /// Set provider options
    Set(SetArgs),
}

#[derive(clap::Args)]
pub struct SetArgs {
    /// Provider name
    provider: String,

    /// Set API key
    #[arg(long)]
    api_key: Option<String>,

    /// Set base URL
    #[arg(long)]
    base_url: Option<String>,

    /// Add a model with variants (comma-separated), e.g. "gpt-5.5:low,medium,high,xhigh"
    #[arg(long)]
    add_model: Option<String>,
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;

    match cmd {
        Cmd::Show => {
            let config = Config::load(&cwd)?;
            show_config(&config);
            Ok(())
        }
        Cmd::Path => {
            show_paths();
            Ok(())
        }
        Cmd::Set(args) => {
            let global_path = Config::global_config_path();

            // Read raw JSON to preserve unknown fields (permission, agent, etc.)
            let mut raw: serde_json::Value = if global_path.exists() {
                let content = std::fs::read_to_string(&global_path)?;
                let stripped = crate::core::config::strip_jsonc_comments(&content);
                match serde_json::from_str(&stripped) {
                    Ok(v) => v,
                    Err(e) => {
                        anyhow::bail!(
                            "Failed to parse global config at {}: {}. Fix the file manually or delete it.",
                            global_path.display(),
                            e
                        );
                    }
                }
            } else {
                serde_json::json!({})
            };

            // Ensure provider entry exists
            if raw.get("provider").is_none() {
                raw["provider"] = serde_json::json!({});
            }
            if raw["provider"].get(&args.provider).is_none() {
                raw["provider"][&args.provider] = serde_json::json!({});
            }

            let p = &mut raw["provider"][&args.provider];

            if let Some(key) = &args.api_key {
                // Save to encrypted vault instead of config.json
                crate::core::vault::Vault::save(&args.provider, key)?;
                println!(
                    "🔐 Saved encrypted API key for '{}' to vault.",
                    args.provider
                );
            }
            if let Some(url) = &args.base_url {
                p["base_url"] = serde_json::json!(url);
                println!("Set base_url for provider '{}' → {}", args.provider, url);
            }
            if let Some(model_spec) = &args.add_model {
                let parts: Vec<&str> = model_spec.splitn(2, ':').collect();
                let model_id = parts[0];
                let variants: Vec<&str> = parts
                    .get(1)
                    .map(|v| v.split(',').collect())
                    .unwrap_or_default();
                let mut model = serde_json::json!({ "name": model_id });
                if !variants.is_empty() {
                    let vmap: serde_json::Map<String, serde_json::Value> = variants
                        .iter()
                        .map(|v| (v.to_string(), serde_json::json!({})))
                        .collect();
                    model["variants"] = serde_json::json!(vmap);
                }
                p["models"][model_id] = model;
                println!(
                    "Added model '{}' to provider '{}'.",
                    model_id, args.provider
                );
            }
            if args.api_key.is_none() && args.base_url.is_none() && args.add_model.is_none() {
                // Show current state
                let config = Config::load(&cwd)?;
                let vault = crate::core::vault::Vault::load();
                println!(
                    "Usage: openrust debug config set <provider> --api-key <key> --base-url <url>"
                );
                if let Some(resolved) = config.get_provider(&args.provider) {
                    let key_status = match vault.get(&args.provider) {
                        Some(k) => format!("🔐 (vault) {}", super::mask_secret(&k)),
                        None => "(not set)".to_string(),
                    };
                    println!("  api_key:  {}", key_status);
                    println!(
                        "  base_url: {}",
                        resolved.base_url.as_deref().unwrap_or("(not set)")
                    );
                }
                return Ok(());
            }

            // Save atomically: write to temp file, then rename
            if let Some(parent) = global_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let json = serde_json::to_string_pretty(&raw)?;
            let temp_path = global_path.with_extension("tmp");
            std::fs::write(&temp_path, &json)?;
            std::fs::rename(&temp_path, &global_path)?;
            println!("Saved to {}", global_path.display());
            Ok(())
        }
    }
}

fn show_config(config: &Config) {
    let vault = crate::core::vault::Vault::load();

    println!("Model: {}", config.model.as_deref().unwrap_or("(not set)"));
    println!();
    for (name, _cfg) in &config.provider {
        let resolved = config.get_provider(name);
        let base_url = resolved
            .as_ref()
            .and_then(|r| r.base_url.as_deref())
            .unwrap_or("(default)")
            .to_string();
        let config_api_key = resolved.as_ref().and_then(|r| r.api_key.clone());
        let key_status = match vault.get(name) {
            Some(k) => format!("🔐 (vault) {}", super::mask_secret(&k)),
            None => config_api_key
                .map(|v| format!("(config) {}", super::mask_secret(&v)))
                .unwrap_or_else(|| "(not set)".to_string()),
        };
        println!("[{}]", name);
        println!("  base_url: {}", base_url);
        println!("  api_key:  {}", key_status);
        if !_cfg.models.is_empty() {
            println!("  models:");
            for (id, m) in &_cfg.models {
                let display = m.name.as_deref().unwrap_or(id);
                if let Some(variants) = &m.variants {
                    let vnames: Vec<&str> = variants.keys().map(|s| s.as_str()).collect();
                    println!("    {}  (variants: {})", display, vnames.join(", "));
                } else {
                    println!("    {}", display);
                }
            }
        }
        println!();
    }
}

fn show_paths() {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let paths = crate::core::platform::PlatformPaths::detect();
    for line in render_paths(&cwd, &paths) {
        println!("{}", line);
    }
}

fn render_paths(
    cwd: &std::path::Path,
    paths: &crate::core::platform::PlatformPaths,
) -> [String; 7] {
    [
        format!(
            "Project config:  {}",
            cwd.join(".openrust").join("config.jsonc").display()
        ),
        format!("Global config:   {}", paths.global_config_path().display()),
        format!(
            "Global rules:    {}",
            paths.config_dir().join("rules").join("*.md").display()
        ),
        format!(
            "Project rules:   {}",
            cwd.join(".openrust").join("rules").join("*.md").display()
        ),
        format!("Vault (enc):     {}", paths.credentials_path().display()),
        format!("Undo store:      {}", paths.undo_dir().display()),
        format!("Sessions DB:     {}", paths.sessions_db_path().display()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::platform::{PlatformKind, PlatformPaths, PlatformScope};

    #[test]
    fn render_paths_includes_all_expected_labels() {
        let cwd = PathBuf::from("/work/app");
        let paths = PlatformPaths {
            kind: PlatformKind::Fedora,
            home: PathBuf::from("/home/test"),
            config: PlatformScope {
                dir: PathBuf::from("/home/test/.config/openrust"),
            },
            data: PlatformScope {
                dir: PathBuf::from("/home/test/.local/share/openrust"),
            },
            cache: PlatformScope {
                dir: PathBuf::from("/home/test/.cache/openrust"),
            },
        };

        let lines = render_paths(&cwd, &paths);

        assert!(lines[0].contains("Project config:") && lines[0].ends_with("config.jsonc"));
        assert!(lines[1].contains("Global config:") && lines[1].ends_with("config.json"));
        assert!(lines[2].contains("Global rules:") && lines[2].ends_with("*.md"));
        assert!(lines[3].contains("Project rules:") && lines[3].ends_with("*.md"));
        assert!(
            lines[4].contains("Vault (enc):") && lines[4].ends_with("credentials.enc")
        );
        assert!(lines[5].contains("Undo store:") && lines[5].ends_with("undo"));
        assert!(lines[6].contains("Sessions DB:") && lines[6].ends_with("sessions.db"));
    }
}
