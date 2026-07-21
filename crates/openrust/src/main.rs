//! OpenRust — AI coding agent (Rust rewrite)
//!
//! Entry point. Parses CLI args and dispatches to TUI or debug subcommands.

#![allow(dead_code)]

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

    install_panic_hook(&log_dir);

    Ok(guard)
}

/// Route panics (any thread) into the log file in addition to stderr, so a
/// crashed TUI session still leaves a trace. The record goes through tracing
/// and is also appended synchronously as a fallback in case the non-blocking
/// writer does not flush before teardown.
fn install_panic_hook(log_dir: &std::path::Path) {
    let log_dir = log_dir.to_path_buf();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic: {info}\n{backtrace}");

        // Append to the file the rolling appender is currently writing
        // (newest openrust.log.*), falling back to the unsuffixed name.
        let latest = std::fs::read_dir(&log_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("openrust.log."))
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
        let path = latest
            .map(|e| e.path())
            .unwrap_or_else(|| log_dir.join("openrust.log"));
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            use std::io::Write;
            let _ = writeln!(file, "ERROR panic: {info}\n{backtrace}");
        }

        default_hook(info);
    }));
}
