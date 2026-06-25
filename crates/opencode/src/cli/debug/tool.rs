use clap::Subcommand;
use crate::tool::{ToolContext, ToolParams, create_tool};

#[derive(Subcommand)]
pub enum Cmd {
    /// List all registered tools
    List,
    /// Run a tool with JSON params
    Run {
        name: String,
        #[arg(short, long)]
        params: String,
    },
    /// Show OpenAI-compatible tool definition for a tool
    Schema {
        name: String,
    },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let ctx = ToolContext { cwd, interactive: false, undo_store: None };

    match cmd {
        Cmd::List => {
            let tool_names = [
                "read", "write", "edit", "bash", "glob", "grep",
                "webfetch", "websearch", "undo_edit",
            ];
            println!("Available tools ({}):", tool_names.len());
            for name in tool_names {
                if let Some(tool) = create_tool(name, None) {
                    println!("  {:12} {}", name, tool.description());
                }
            }
            Ok(())
        }
        Cmd::Run { name, params } => {
            let tool = create_tool(&name, None)
                .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;

            let parsed: serde_json::Value = serde_json::from_str(&params)
                .map_err(|e| anyhow::anyhow!("Invalid JSON params: {}", e))?;

            let rt = tokio::runtime::Runtime::new()?;
            let result = rt.block_on(tool.execute(ToolParams::new(parsed), &ctx));

            println!("{}", result.into_text());
            Ok(())
        }
        Cmd::Schema { name } => {
            let tool = create_tool(&name, None)
                .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;

            let def = tool.to_llm_def();
            println!("{}", serde_json::to_string_pretty(&def)?);
            Ok(())
        }
    }
}
