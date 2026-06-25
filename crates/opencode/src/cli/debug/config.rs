use clap::Subcommand;

use crate::core::config::Config;

#[derive(Subcommand)]
pub enum Cmd {
    /// Validate opencode.json configuration
    Validate,
    /// Show parsed configuration (secrets masked)
    Show,
    /// Show config file paths
    Path,
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;

    match cmd {
        Cmd::Validate => {
            println!("Config validation: OK");
            println!("  Providers: {}", config.provider.len());
            println!("  Model: {}", config.model.as_deref().unwrap_or("(not set)"));
            Ok(())
        }
        Cmd::Show => {
            println!("Model: {}", config.model.as_deref().unwrap_or("(not set)"));
            println!();
            for (name, cfg) in &config.provider {
                let resolved = config.get_provider(name);
                let base_url = resolved.as_ref().and_then(|r| r.base_url.as_deref()).unwrap_or("(default)");
                let key_status = match &cfg.api_key {
                    Some(k) if k.len() > 8 => format!("****{}", &k[k.len()-4..]),
                    Some(_) => "****".to_string(),
                    None => {
                        let env_key = format!("{}_API_KEY", name.to_uppercase().replace('-', "_"));
                        match std::env::var(&env_key) {
                            Ok(v) if v.len() > 8 => format!("(env) ****{}", &v[v.len()-4..]),
                            Ok(_) => "(env) ****".to_string(),
                            Err(_) => "(not set)".to_string(),
                        }
                    }
                };
                println!("[{}]", name);
                println!("  base_url: {}", base_url);
                println!("  api_key:  {}", key_status);
                if !cfg.models.is_empty() {
                    println!("  models:");
                    for (id, m) in &cfg.models {
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
            Ok(())
        }
        Cmd::Path => {
            let cwd = std::env::current_dir()?;
            println!("Project config:  {}/opencode.json", cwd.display());
            println!("                  {}/.opencode/opencode.jsonc", cwd.display());
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| "~".to_string());
            println!("Global config:   {}/.config/opencode/opencode.json", home);
            Ok(())
        }
    }
}
