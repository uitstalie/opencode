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
                serde_json::from_str(&stripped).unwrap_or(serde_json::json!({}))
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
                println!("🔐 Saved encrypted API key for '{}' to vault.", args.provider);
            }
            if let Some(url) = &args.base_url {
                p["base_url"] = serde_json::json!(url);
                println!("Set base_url for provider '{}' → {}", args.provider, url);
            }
            if let Some(model_spec) = &args.add_model {
                let parts: Vec<&str> = model_spec.splitn(2, ':').collect();
                let model_id = parts[0];
                let variants: Vec<&str> = parts.get(1).map(|v| v.split(',').collect()).unwrap_or_default();
                let mut model = serde_json::json!({ "name": model_id });
                if !variants.is_empty() {
                    let vmap: serde_json::Map<String, serde_json::Value> = variants
                        .iter()
                        .map(|v| (v.to_string(), serde_json::json!({})))
                        .collect();
                    model["variants"] = serde_json::json!(vmap);
                }
                p["models"][model_id] = model;
                println!("Added model '{}' to provider '{}'.", model_id, args.provider);
            }
            if args.api_key.is_none() && args.base_url.is_none() && args.add_model.is_none() {
                // Show current state
                let config = Config::load(&cwd)?;
                let vault = crate::core::vault::Vault::load();
                println!("Usage: opencode debug config set <provider> --api-key <key> --base-url <url>");
                if let Some(resolved) = config.get_provider(&args.provider) {
                    let key_status = match vault.get(&args.provider) {
                        Some(k) if k.len() > 8 => format!("🔐 (vault) ****{}", &k[k.len()-4..]),
                        Some(_) => "🔐 (vault) ****".to_string(),
                        None => "(not set)".to_string(),
                    };
                    println!("  api_key:  {}", key_status);
                    println!("  base_url: {}", resolved.base_url.as_deref().unwrap_or("(not set)"));
                }
                return Ok(());
            }

            // Save
            if let Some(parent) = global_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let json = serde_json::to_string_pretty(&raw)?;
            std::fs::write(&global_path, json)?;
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
        let base_url = resolved.as_ref().and_then(|r| r.base_url.as_deref()).unwrap_or("(default)");

        let key_status = match vault.get(name) {
            Some(k) if k.len() > 8 => format!("🔐 (vault) ****{}", &k[k.len() - 4..]),
            Some(_) => "🔐 (vault) ****".to_string(),
            None => {
                let env_key = format!("{}_API_KEY", name.to_uppercase().replace('-', "_"));
                match std::env::var(&env_key).or_else(|_| std::env::var("OPENAI_API_KEY")) {
                    Ok(v) if v.len() > 8 => format!("(env) ****{}", &v[v.len() - 4..]),
                    Ok(_) => "(env) ****".to_string(),
                    Err(_) => "(not set)".to_string(),
                }
            }
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
    let global = Config::global_config_path();
    let vault = Config::global_config_dir().join("credentials.enc");
    println!("Project config:  {}/opencode.json", cwd.display());
    println!("Global config:   {}", global.display());
    println!("Vault (enc):     {}", vault.display());
}
