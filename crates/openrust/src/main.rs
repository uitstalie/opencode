//! OpenRust — AI coding agent (Rust rewrite)
//!
//! Entry point. Parses CLI args and dispatches to TUI or debug subcommands.

#![allow(dead_code)] // Phase 0: many types defined but used in later phases

use clap::Parser;
use openrust::cli;
use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::prelude::*;

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
    let cwd = std::env::current_dir()?;
    let log_level = openrust::core::config::Config::load(&cwd)?.log_level;
    let _guard = init_logging(log_level.as_deref())?;

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

fn init_logging(log_level: Option<&str>) -> anyhow::Result<WorkerGuard> {
    let log_dir = openrust::core::platform::PlatformPaths::detect()
        .data_dir()
        .join("log");
    std::fs::create_dir_all(&log_dir)?;

    let file_appender = tracing_appender::rolling::daily(&log_dir, "openrust.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_new(log_level.unwrap_or("openrust=info"))
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("openrust=info")),
        )
        .with(file_layer)
        .init();

    Ok(guard)
}
