use clap::Subcommand;
use futures::StreamExt;

use crate::core::config::Config;

#[derive(Subcommand)]
pub enum Cmd {
    /// List configured providers
    List,
    /// Test a provider with a prompt (streaming)
    Test {
        /// Provider name (e.g. "one_route", "deepseek")
        name: String,
        /// Prompt to send
        #[arg(short, long, default_value = "hello")]
        prompt: String,
        /// Model to use (default: from config)
        #[arg(short, long)]
        model: Option<String>,
        /// API key override
        #[arg(long)]
        api_key: Option<String>,
        /// Reasoning effort level (low/medium/high)
        #[arg(long)]
        reasoning: Option<String>,
    },
    /// List available models for a provider
    Models {
        /// Provider name
        name: String,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;

    match cmd {
        Cmd::List => {
            if config.provider.is_empty() {
                println!("No providers configured.");
                println!("Add provider config to .openrust/config.jsonc:");
                println!(
                    r#"  {{"provider": {{"deepseek": {{"baseURL": "https://api.deepseek.com/v1"}}}}}}"#
                );
            } else {
                println!("Configured providers:");
                for name in config.provider.keys() {
                    let resolved = config.get_provider(name);
                    let key_status = match resolved.and_then(|r| r.api_key) {
                        Some(k) => super::mask_secret(&k),
                        None => "(no API key)".to_string(),
                    };
                    println!("  {}  api_key: {}", name, key_status);
                }
            }
            println!();
            println!("Model: {}", config.model.as_deref().unwrap_or("(not set)"));
            Ok(())
        }
        Cmd::Test {
            name,
            prompt,
            model,
            api_key,
            reasoning,
        } => {
            let mut resolved = config
                .get_provider(&name)
                .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found in config", name))?;

            // CLI --api-key overrides config and vault
            if let Some(key) = api_key {
                resolved.api_key = Some(key);
            }

            resolved.api_key.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "No API key for provider '{}'. Add api_key to config or use --api-key.",
                    name
                )
            })?;

            let provider = crate::core::provider::create_provider(&resolved)
                .ok_or_else(|| anyhow::anyhow!("Failed to create provider '{}'", name))?;

            let model_id = model
                .or_else(|| config.resolve_provider_model().map(|(_, model)| model))
                .ok_or_else(|| anyhow::anyhow!("No model configured"))?;

            println!("Provider: {}", name);
            println!("Model:    {}", model_id);
            println!("Prompt:   {}", prompt);
            println!("──────────────────────────────────────────");

            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let messages = vec![crate::core::provider::Message::user(prompt.clone())];

                let mut stream = provider
                    .chat(
                        &messages,
                        &[],
                        crate::core::provider::RequestOptions {
                            model: model_id,
                            temperature: None,
                            max_tokens: None,
                            top_p: None,
                            system: None,
                            reasoning_effort: reasoning.clone(),
                            tool_choice: None,
                            cache_key: None,
                        },
                    )
                    .await?;

                while let Some(chunk) = stream.next().await {
                    match chunk? {
                        crate::core::provider::StreamChunk::TextDelta(text) => {
                            print!("{}", text);
                        }
                        crate::core::provider::StreamChunk::ReasoningDelta(text) => {
                            print!("\n[reasoning] {}", text);
                        }
                        crate::core::provider::StreamChunk::ToolCallStart {
                            name: tool_name,
                            ..
                        } => {
                            print!("\n[🔧 {}] ", tool_name);
                        }
                        crate::core::provider::StreamChunk::ToolCallDelta { args, .. } => {
                            print!("{}", args);
                        }
                        crate::core::provider::StreamChunk::ToolCallEnd { .. } => {}
                        crate::core::provider::StreamChunk::Finish { usage, .. } => {
                            if let Some(u) = usage {
                                println!("\n──────────────────────────────────────────");
                                println!(
                                    "Tokens: {} prompt + {} completion = {} total",
                                    u.prompt_tokens, u.completion_tokens, u.total_tokens
                                );
                            }
                        }
                    }
                }
                println!();
                Ok::<_, anyhow::Error>(())
            })?;

            Ok(())
        }
        Cmd::Models { name } => {
            let resolved = config
                .get_provider(&name)
                .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found in config", name))?;

            if resolved.models.is_empty() {
                println!("No models configured for provider '{}'.", name);
            } else {
                println!("Models for '{}':", name);
                for (model_id, model_cfg) in &resolved.models {
                    let display = model_cfg.name.as_deref().unwrap_or(model_id);
                    if let Some(variants) = &model_cfg.variants {
                        let vnames: Vec<&str> = variants.keys().map(|s| s.as_str()).collect();
                        println!("  {}  (variants: {})", display, vnames.join(", "));
                    } else {
                        println!("  {}", display);
                    }
                }
            }
            Ok(())
        }
    }
}
