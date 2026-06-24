use clap::Subcommand;

#[derive(Subcommand)]
pub enum Cmd {
    /// List configured providers
    List,
    /// Test a provider with a prompt
    Test {
        /// Provider name (e.g. "openai")
        name: String,
        /// Prompt to send
        #[arg(short, long, default_value = "hello")]
        prompt: String,
    },
    /// List available models for a provider
    Models {
        /// Provider name
        name: String,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::List => {
            println!("Providers: (not yet configured)");
            println!("  Configure via opencode.json or environment variables.");
            Ok(())
        }
        Cmd::Test { name, prompt } => {
            println!("Testing provider '{}' with prompt: {}", name, prompt);
            println!("  (OpenAI client not yet implemented)");
            Ok(())
        }
        Cmd::Models { name } => {
            println!("Models for provider '{}': (not yet implemented)", name);
            Ok(())
        }
    }
}
