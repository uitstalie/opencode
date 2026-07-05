//! Read file tool.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, resolve_path};
use crate::try_tool;
use serde_json::Value;
use std::path::Path;

pub struct ReadTool;

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }
    fn description(&self) -> &'static str {
        "Read a file or directory (like cat but with line numbers, offset, and limit). Use for any file content inspection."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to read relative to cwd, or absolute path" },
                "filePath": { "type": "string", "description": "Legacy alias for path" },
                "offset": { "type": "integer", "description": "Line to start at (1-indexed, default 1)" },
                "limit": { "type": "integer", "description": "Max lines to read (default 2000)" }
            },
            "required": []
        })
    }

    async fn execute(&self, p: ToolParams, _ctx: &ToolContext) -> ToolResult {
        let path = p
            .opt_str("path")
            .or_else(|| p.opt_str("filePath"))
            .unwrap_or("");
        if path.is_empty() {
            return ToolResult::error("Missing required parameter: path");
        }
        let offset = p.u64_or("offset", 1) as usize;
        let limit = p.u64_or("limit", 2000) as usize;
        let resolved = resolve_path(_ctx, path);

        if Path::new(&resolved).is_dir() {
            let mut entries = match std::fs::read_dir(&resolved) {
                Ok(read_dir) => read_dir
                    .filter_map(|entry| entry.ok())
                    .map(|entry| {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if entry.path().is_dir() {
                            format!("{}/", name)
                        } else {
                            name
                        }
                    })
                    .collect::<Vec<_>>(),
                Err(err) => return ToolResult::error(format!("Cannot read {}", err)),
            };
            entries.sort();
            if offset == 0 || offset > entries.len() {
                return ToolResult::error(format!(
                    "Directory has {} entries, offset {} out of range.",
                    entries.len(),
                    offset
                ));
            }
            let start = offset - 1;
            let end = (start + limit).min(entries.len());
            let mut out = entries[start..end]
                .iter()
                .enumerate()
                .map(|(i, entry)| format!("{:6}: {}", start + i + 1, entry))
                .collect::<Vec<_>>()
                .join("\n");
            if end < entries.len() {
                out.push_str(&format!(
                    "\n... ({} entries remaining)",
                    entries.len() - end
                ));
            }
            return ToolResult::text(out);
        }

        let content = try_tool!(std::fs::read_to_string(&resolved), |e| format!(
            "Cannot read {}",
            e
        ));

        let lines: Vec<&str> = content.lines().collect();
        if offset == 0 || offset > lines.len() {
            return ToolResult::error(format!(
                "File has {} lines, offset {} out of range.",
                lines.len(),
                offset
            ));
        }
        let start = offset - 1;
        let end = (start + limit).min(lines.len());
        let selected: Vec<String> = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:6}: {}", start + i + 1, l))
            .collect();
        let mut out = selected.join("\n");
        if end < lines.len() {
            out.push_str(&format!("\n... ({} lines remaining)", lines.len() - end));
        }
        ToolResult::text(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext {
        ToolContext::new(std::env::current_dir().unwrap())
    }

    #[tokio::test]
    async fn reads_self() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"filePath": file!()})),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("ReadTool"));
    }

    #[tokio::test]
    async fn nonexistent_file() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"filePath": "/no/such"})),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("Cannot read"));
    }

    #[tokio::test]
    async fn with_offset_limit() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": file!(), "offset": 1, "limit": 3
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("1:"));
    }

    #[tokio::test]
    async fn path_alias_resolves_relative_to_cwd() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"path": "src/tool/read.rs"})),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("ReadTool"));
    }

    #[tokio::test]
    async fn lists_directory_entries() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"path": "src/tool"})),
                &ctx(),
            )
            .await;
        let text = r.into_text();
        assert!(text.contains("read.rs"));
        assert!(text.contains(":"));
    }

    #[tokio::test]
    async fn missing_path_reports_error() {
        let r = ReadTool
            .execute(ToolParams::new(serde_json::json!({})), &ctx())
            .await;
        assert!(r.into_text().contains("Missing required parameter"));
    }
}
