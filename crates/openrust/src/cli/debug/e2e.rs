use clap::Subcommand;

use crate::core::config::Config;
use crate::core::provider::{self, Message};
use crate::system_prompt::SystemPrompt;
use crate::tool::ToolContext;
use crate::tool::task::run_agent;

#[derive(Debug)]
struct E2eRunConfig {
    config: Config,
    provider_name: String,
    resolved: crate::core::config::ResolvedProvider,
    model: String,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run a full prompt → LLM → tool loop and print the final result
    Run { prompt: String },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let Cmd::Run { prompt } = cmd;
    let cwd = std::env::current_dir()?;
    let run_config = load_run_config(&cwd)?;
    let llm = create_run_provider(&run_config)?;
    let system = render_run_system(&run_config)?;

    let ctx = ToolContext::new(cwd);
    let messages = vec![Message::user(prompt)];

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(run_agent(
        llm.as_ref(),
        &run_config.model,
        &system,
        "all",
        50,
        None,
        messages,
        &ctx,
    ))?;
    println!("{}", result);
    Ok(())
}

fn load_run_config(cwd: &std::path::Path) -> anyhow::Result<E2eRunConfig> {
    resolve_run_config(Config::load(cwd)?)
}

fn resolve_run_config(config: Config) -> anyhow::Result<E2eRunConfig> {
    let (provider_name, model) = config
        .resolve_provider_model()
        .ok_or_else(|| anyhow::anyhow!("No model configured (set \"model\": \"provider/model\" in config)"))?;
    let resolved = config
        .get_provider(&provider_name)
        .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;

    Ok(E2eRunConfig {
        config,
        provider_name,
        resolved,
        model,
    })
}

fn create_run_provider(
    run_config: &E2eRunConfig,
) -> anyhow::Result<Box<dyn provider::LlmProvider>> {
    provider::create_provider(&run_config.resolved)
        .ok_or_else(|| anyhow::anyhow!("Failed to create provider '{}'", run_config.provider_name))
}

fn render_run_system(run_config: &E2eRunConfig) -> anyhow::Result<String> {
    Ok(SystemPrompt::from_config(
        &run_config.config,
        &run_config.resolved,
    )?
    .render())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ProviderConfig;
    use std::collections::HashMap;

    #[test]
    fn resolve_run_config_errors_without_provider() {
        let err = resolve_run_config(Config::default()).unwrap_err();
        assert!(err.to_string().contains("No model configured"));
    }

    #[test]
    fn load_run_config_surfaces_malformed_project_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".openrust")).unwrap();
        std::fs::write(dir.path().join(".openrust").join("config.jsonc"), "{ nope").unwrap();

        let err = load_run_config(dir.path()).unwrap_err();

        assert_ne!(err.to_string(), "No model configured");
    }

    #[test]
    fn resolve_run_config_errors_without_model() {
        let err = resolve_run_config(Config {
            log_level: None,
            model: None,
            theme: None,
            search_engine: None,
            background_model: None,
            provider: HashMap::from([(
                "deepseek".to_string(),
                ProviderConfig {
                    api_key: Some("test-key".to_string()),
                    base_url: Some("https://example.test/v1".to_string()),
                    ..Default::default()
                },
            )]),
            presets: HashMap::new(),
            user_providers: Default::default(),
        })
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "No model configured (set \"model\": \"provider/model\" in config)"
        );
    }

    #[test]
    fn create_run_provider_errors_without_api_key() {
        let run_config = resolve_run_config(Config {
            log_level: None,
            model: Some("missing-key-provider/deepseek-chat".to_string()),
            theme: None,
            search_engine: None,
            background_model: None,
            provider: HashMap::from([(
                "missing-key-provider".to_string(),
                ProviderConfig {
                    api_key: None,
                    base_url: Some("https://example.test/v1".to_string()),
                    ..Default::default()
                },
            )]),
            presets: HashMap::new(),
            user_providers: Default::default(),
        })
        .unwrap();

        let err = match create_run_provider(&run_config) {
            Ok(_) => panic!("provider creation should fail without api key"),
            Err(err) => err,
        };

        assert_eq!(
            err.to_string(),
            "Failed to create provider 'missing-key-provider'"
        );
    }

    #[test]
    fn render_run_system_surfaces_system_prompt_errors() {
        let run_config = E2eRunConfig {
            config: Config {
                log_level: None,
                model: None,
                theme: None,
                search_engine: None,
                background_model: None,
                provider: HashMap::new(),
                presets: HashMap::new(),
            user_providers: Default::default(),
            },
            provider_name: "deepseek".to_string(),
            resolved: crate::core::config::ResolvedProvider {
                name: "deepseek".to_string(),
                api_key: Some("test-key".to_string()),
                base_url: Some("https://example.test/v1".to_string()),
                ..Default::default()
            },
            model: "deepseek-chat".to_string(),
        };

        let err = render_run_system(&run_config).unwrap_err();

        assert_eq!(err.to_string(), "No model configured");
    }
}
