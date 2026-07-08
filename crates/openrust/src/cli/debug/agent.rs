use clap::Subcommand;

use crate::core::agent;

#[derive(Subcommand)]
pub enum Cmd {
    /// List local markdown agents
    List,
    /// Show a local markdown agent
    Show {
        /// Agent id
        id: String,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let agents = agent::load_agents(&cwd)?;

    match cmd {
        Cmd::List => {
            print_list(&agents);
            Ok(())
        }
        Cmd::Show { id } => {
            let Some(info) = agent::agent_by_id(&agents, &id) else {
                anyhow::bail!("Agent '{}' not found", id);
            };
            println!("# {}", info.title);
            println!();
            println!("id: {}", info.id);
            println!("mode: {}", info.mode);
            println!("hidden: {}", info.hidden);
            println!("path: {}", info.path.display());
            println!();
            println!("--- system ---");
            println!("{}", info.system);
            println!();
            println!("--- source ---");
            println!("{}", info.content);
            Ok(())
        }
    }
}

fn print_list(agents: &[agent::AgentInfo]) {
    if agents.is_empty() {
        println!("No local agents found.");
        return;
    }

    for info in agents {
        println!(
            "{}  [{}] {}  hidden={}",
            info.id, info.mode, info.description, info.hidden
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_format_handles_empty_and_entries() {
        print_list(&[]);
        let agents = [agent::AgentInfo {
            id: "build".to_string(),
            title: "Build".to_string(),
            description: "Build agent".to_string(),
            mode: "all".to_string(),
            hidden: false,
            max_steps: 50,
            system: String::new(),
            path: std::path::PathBuf::from(".openrust/agents/build.md"),
            content: String::new(),
        }];
        print_list(&agents);
    }
}
