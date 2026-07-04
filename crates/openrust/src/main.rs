//! OpenRust — AI coding agent (Rust rewrite)
//!
//! Entry point. Parses CLI args and dispatches to TUI or debug subcommands.

#![allow(dead_code)] // Phase 0: many types defined but used in later phases

use clap::Parser;
use openrust::cli;

#[derive(Parser)]
#[command(name = "openrust", version, about = "AI coding agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Debug and test commands
    Debug {
        #[command(subcommand)]
        cmd: cli::DebugCmd,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "openrust=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Debug { cmd }) => cli::run_debug(cmd)?,
        None => {
            tracing::info!("No subcommand provided. Launching placeholder session shell.");
            println!("OpenRust session shell is not implemented yet.");
            println!("Use 'openrust debug --help' for session and prompt inspection.");
        }
    }

    Ok(())
}
