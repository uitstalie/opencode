use crate::tool::{ToolContext, ToolParams, catalog, create_tool};
use clap::Subcommand;

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
    Schema { name: String },
}

pub fn run(cmd: Cmd) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let ctx = ToolContext::new(cwd);

    match cmd {
        Cmd::List => {
            let tool_names = catalog::tool_names();
            println!("Available tools ({}):", tool_names.len());
            for name in tool_names {
                if let Some(meta) = catalog::tool_meta(name) {
                    println!(
                        "  {:12} [{}] {}",
                        name,
                        format_category(meta.category),
                        meta.description
                    );
                }
            }
            println!();
            for (category, names) in catalog::registry_category_names() {
                println!("{:>8}: {}", format_category(category), names.join(", "));
            }
            Ok(())
        }
        Cmd::Run { name, params } => {
            let tool = create_tool(&name, None)
                .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?;

            let parsed: serde_json::Value = serde_json::from_str(&params)
                .map_err(|e| anyhow::anyhow!("Invalid JSON params: {}", e))?;

            let rt = tokio::runtime::Runtime::new()?;
            let result = rt.block_on(tool.execute_checked(ToolParams::new(parsed), &ctx));

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

fn format_category(category: catalog::ToolCategory) -> &'static str {
    match category {
        catalog::ToolCategory::Filesystem => "fs",
        catalog::ToolCategory::Shell => "shell",
        catalog::ToolCategory::Network => "net",
        catalog::ToolCategory::Interaction => "interaction",
        catalog::ToolCategory::Memory => "memory",
        catalog::ToolCategory::Undo => "undo",
    }
}
