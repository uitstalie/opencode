use clap::Subcommand;

use crate::core::permission::{Decision, evaluate};

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
            let label = decision_label(evaluate(&tool, &path, &cwd, true));
            println!("{}", label);
            Ok(())
        }
    }
}

fn decision_label(decision: Decision) -> &'static str {
    match decision {
        Decision::Allow => "allow",
        Decision::Deny(_) => "deny",
        Decision::Ask(_) => "ask",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_label_maps_allow() {
        assert_eq!(decision_label(Decision::Allow), "allow");
    }

    #[test]
    fn decision_label_maps_deny() {
        assert_eq!(decision_label(Decision::Deny("nope".to_string())), "deny");
    }

    #[test]
    fn decision_label_maps_ask() {
        assert_eq!(decision_label(Decision::Ask("check".to_string())), "ask");
    }
}
