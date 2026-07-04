use clap::Subcommand;

use crate::core::permission::{evaluate, Decision};

#[derive(Subcommand)]
pub enum Cmd {
    /// Check whether a tool may operate on a path (allow | deny | ask)
    Check { tool: String, path: String },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    match cmd {
        Cmd::Check { tool, path } => {
            // interactive=true so an out-of-scope target surfaces as `ask`.
            let label = match evaluate(&tool, &path, &cwd, true) {
                Decision::Allow => "allow",
                Decision::Deny(_) => "deny",
                Decision::Ask(_) => "ask",
            };
            println!("{}", label);
            Ok(())
        }
    }
}
