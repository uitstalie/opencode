use clap::Subcommand;

use crate::core::config::Config;
use crate::system_prompt::SystemPrompt;

#[derive(Subcommand)]
pub enum Cmd {
    /// Show the rendered system prompt
    Show,
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Show => {
            let cwd = std::env::current_dir()?;
            let config = Config::load(&cwd)?;
            let (provider_name, _model) = config
                .resolve_provider_model()
                .ok_or_else(|| anyhow::anyhow!("No model configured"))?;
            let provider = config
                .get_provider(&provider_name)
                .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;

            let prompt = SystemPrompt::from_config(&config, &provider)?.render();
            println!("{}", prompt);
            Ok(())
        }
    }
}
