use clap::Subcommand;

use crate::core::config::Config;
use crate::core::provider::{self, Message};
use crate::core::system_prompt::SystemPrompt;
use crate::tool::task::run_agent;
use crate::tool::ToolContext;

#[derive(Subcommand)]
pub enum Cmd {
    /// Run a full prompt → LLM → tool loop and print the final result
    Run { prompt: String },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let Cmd::Run { prompt } = cmd;
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;
    let provider_name = config
        .provider
        .keys()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No provider configured"))?
        .to_string();
    let resolved = config
        .get_provider(&provider_name)
        .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;
    let llm = provider::create_provider(&resolved)
        .ok_or_else(|| anyhow::anyhow!("Failed to create provider '{}'", provider_name))?;
    let model = config
        .resolve_provider_model()
        .map(|(_, model)| model)
        .ok_or_else(|| anyhow::anyhow!("No model configured"))?;
    let system = SystemPrompt::from_config(&config, &resolved, config.mode.clone())?.render();

    let ctx = ToolContext::new(cwd);
    let messages = vec![Message {
        role: "user".to_string(),
        content: prompt,
        name: None,
        tool_call_id: None,
        tool_calls: None,
    }];

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(run_agent(llm.as_ref(), &model, &system, None, messages, &ctx))?;
    println!("{}", result);
    Ok(())
}
