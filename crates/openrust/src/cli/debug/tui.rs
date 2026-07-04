use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum Cmd {
    /// Print a scripted TUI scenario and run it
    Replay {
        /// Scenario script path
        #[arg(long)]
        script: PathBuf,

        /// Optional seed prompt to prepend
        #[arg(long)]
        prompt: Option<String>,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::Replay { script, prompt } => {
            println!("replay script: {}", script.display());
            crate::tui::run(Some(script), prompt)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_command_accepts_script_path() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("scenario.txt");
        std::fs::write(&script, "hello").unwrap();

        let cmd = Cmd::Replay { script, prompt: Some("seed".to_string()) };
        match cmd {
            Cmd::Replay { script, prompt } => {
                assert!(script.ends_with("scenario.txt"));
                assert_eq!(prompt.as_deref(), Some("seed"));
            }
        }
    }
}
