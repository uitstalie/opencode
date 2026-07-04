use clap::Subcommand;

use crate::core::{config::Config, system_prompt::SystemPrompt};

#[derive(Subcommand)]
pub enum Cmd {
    /// Show the rendered system prompt
    Show {
        /// Rendering mode
        #[arg(long, default_value = "build")]
        mode: String,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Show { mode } => {
            let cwd = std::env::current_dir()?;
            let config = Config::load(&cwd)?;
            let provider_name = config
                .provider
                .keys()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No provider configured"))?;
            let provider = config
                .get_provider(provider_name)
                .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;

            let prompt = SystemPrompt::from_config(&config, &provider, Some(mode)).render();
            println!("{}", prompt);
            Ok(())
        }
    }
}
