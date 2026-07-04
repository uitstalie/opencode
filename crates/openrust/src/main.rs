//! OpenRust — AI coding agent (Rust rewrite)
//!
//! Entry point. Parses CLI args and dispatches to TUI or debug subcommands.

#![allow(dead_code)] // Phase 0: many types defined but used in later phases

use clap::Parser;
use openrust::cli;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "openrust", version, about = "AI coding agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Launch the interactive TUI
    Tui {
        /// Preloaded prompt script (one prompt per non-empty line)
        #[arg(long)]
        script: Option<PathBuf>,

        /// Seed prompt to send immediately after launch
        #[arg(long)]
        prompt: Option<String>,
    },
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
        Some(Command::Tui { script, prompt }) => openrust::tui::run(script, prompt)?,
        Some(Command::Debug { cmd }) => cli::run_debug(cmd)?,
        None => {
            openrust::tui::run(None, None)?;
        }
    }

    Ok(())
}
