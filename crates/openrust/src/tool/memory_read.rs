//! memory_read tool — read memory entries from `.md` files.
//!
//! With no parameters, returns a lightweight overview (scope → file → count).
//! With parameters, returns matching entries.

use crate::core::memory;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};

pub struct MemoryReadTool;

#[async_trait::async_trait]
impl Tool for MemoryReadTool {
    fn name(&self) -> &'static str {
        "memory_read"
    }

    fn description(&self) -> &'static str {
        "Read memory entries. With no params, returns an overview of all \
         scopes and entry counts. Filter by scope/category/search to get \
         specific entries. Memory is the only way to access past conclusions, \
         decisions, and preferences — check it before important decisions."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": {
                    "type": "string",
                    "enum": ["project", "user", "dreaming"],
                    "description": "Filter to one scope. Omit to scan all scopes."
                },
                "category": {
                    "type": "string",
                    "description": "Filter within scope. project: progress|TODO|tech|conclusion. user: preferences|constraints|patterns|style."
                },
                "search": {
                    "type": "string",
                    "description": "Case-insensitive keyword filter on entry content."
                }
            }
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let scope_opt = p.opt_str("scope").and_then(memory::Scope::parse);
        let category = p.opt_str("category");
        let search = p.opt_str("search");

        // ── Overview mode: no params at all ──
        if scope_opt.is_none() && category.is_none() && search.is_none() {
            return render_overview(ctx);
        }

        // ── Detailed read ──
        let scopes: Vec<memory::Scope> = scope_opt
            .map(|s| vec![s])
            .unwrap_or_else(|| vec![memory::Scope::Project, memory::Scope::User, memory::Scope::Dreaming]);

        let mut sections = Vec::new();
        let mut total = 0usize;

        for scope in &scopes {
            let files = files_for_scope_with_category(*scope, category, &ctx.cwd);
            for path in files {
                let entries = memory::read_entries(&path, search);
                if entries.is_empty() {
                    continue;
                }
                let label = format!(
                    "{} ({})",
                    scope.as_str(),
                    path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                );
                let lines: Vec<String> = entries
                    .iter()
                    .map(format_entry)
                    .collect();
                total += entries.len();
                sections.push(format!("{label}:\n{}", lines.join("\n")));
            }
        }

        if sections.is_empty() {
            return ToolResult::text("No matching memory entries found.");
        }

        ToolResult::text(format!(
            "{}\n\n({} entries)",
            sections.join("\n\n"),
            total
        ))
    }
}

fn render_overview(ctx: &ToolContext) -> ToolResult {
    let data = memory::overview(&ctx.cwd);
    if data.is_empty() {
        return ToolResult::text("No memory entries recorded yet.");
    }

    let lines: Vec<String> = data
        .iter()
        .map(|(scope, name, count)| format!("- {}: {} → {} entries", scope.as_str(), name, count))
        .collect();

    ToolResult::text(format!("Memory overview:\n{}", lines.join("\n")))
}

fn files_for_scope_with_category(
    scope: memory::Scope,
    category: Option<&str>,
    cwd: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    if let Some(cat) = category {
        // Specific file
        let path = memory::resolve_path(scope, Some(cat), cwd);
        return vec![path];
    }
    memory::files_for_scope(scope, cwd)
}

fn format_entry(e: &memory::MemoryEntry) -> String {
    let tag_str = if e.tags.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            e.tags
                .iter()
                .map(|t| format!("#{t}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    if e.date.is_empty() {
        format!("  - {}{}", e.content, tag_str)
    } else {
        format!("  - [{}] {}{}", e.date, e.content, tag_str)
    }
}
