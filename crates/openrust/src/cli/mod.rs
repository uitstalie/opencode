pub mod debug;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum DebugCmd {
    /// LLM provider operations
    Provider {
        #[command(subcommand)]
        cmd: debug::provider::Cmd,
    },
    /// Tool testing
    Tool {
        #[command(subcommand)]
        cmd: debug::tool::Cmd,
    },
    /// Configuration
    Config {
        #[command(subcommand)]
        cmd: debug::config::Cmd,
    },
    /// Session storage and prompt rendering
    Session {
        #[command(subcommand)]
        cmd: debug::session::Cmd,
    },
    /// System prompt rendering
    Prompt {
        #[command(subcommand)]
        cmd: debug::prompt::Cmd,
    },
    /// Encrypted credentials vault
    Vault {
        #[command(subcommand)]
        cmd: debug::vault::Cmd,
    },
}

pub fn run_debug(cmd: DebugCmd) -> anyhow::Result<()> {
    match cmd {
        DebugCmd::Provider { cmd } => debug::provider::run(cmd),
        DebugCmd::Tool { cmd } => debug::tool::run(cmd),
        DebugCmd::Config { cmd } => debug::config::run(cmd),
        DebugCmd::Session { cmd } => debug::session::run(cmd),
        DebugCmd::Prompt { cmd } => debug::prompt::run(cmd),
        DebugCmd::Vault { cmd } => debug::vault::run(cmd),
    }
}
