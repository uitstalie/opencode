use clap::Subcommand;

#[derive(Subcommand)]
pub enum Cmd {
    /// List all registered tools
    List,
    /// Run a tool with JSON params
    Run {
        /// Tool name
        name: String,
        /// JSON parameters
        #[arg(short, long)]
        params: String,
    },
    /// Show JSON Schema for a tool
    Schema {
        /// Tool name
        name: String,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    match cmd {
        Cmd::List => {
            println!("Tools: (not yet implemented)");
            println!("  bash, read, edit, write, glob, grep");
            println!("  webfetch, websearch, question, todowrite");
            println!("  skill, task, undo_edit, apply_patch");
            Ok(())
        }
        Cmd::Run { name, params } => {
            println!("Running tool '{}' with params: {}", name, params);
            println!("  (tool execution not yet implemented)");
            Ok(())
        }
        Cmd::Schema { name } => {
            println!("Schema for tool '{}': (not yet implemented)", name);
            Ok(())
        }
    }
}
