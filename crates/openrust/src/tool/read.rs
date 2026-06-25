//! Read file tool.

use serde_json::Value;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};

pub struct ReadTool;

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str { "read" }
    fn description(&self) -> &'static str {
        "Read a file (like cat but with line numbers, offset, and limit). Use for any file content inspection."
    }
    fn parameters(&self) -> Value { serde_json::json!({
        "type": "object",
        "properties": {
            "filePath": { "type": "string", "description": "Absolute path to the file" },
            "offset": { "type": "integer", "description": "Line to start at (1-indexed, default 1)" },
            "limit": { "type": "integer", "description": "Max lines to read (default 2000)" }
        },
        "required": ["filePath"]
    })}

    async fn execute(&self, p: ToolParams, _ctx: &ToolContext) -> ToolResult {
        let path = require_str!(p, "filePath");
        let offset = p.u64_or("offset", 1) as usize;
        let limit = p.u64_or("limit", 2000) as usize;

        let content = try_tool!(std::fs::read_to_string(path), |e| format!("Cannot read {}", e));

        let lines: Vec<&str> = content.lines().collect();
        if offset == 0 || offset > lines.len() {
            return ToolResult::error(format!("File has {} lines, offset {} out of range.", lines.len(), offset));
        }
        let start = offset - 1;
        let end = (start + limit).min(lines.len());
        let selected: Vec<String> = lines[start..end].iter().enumerate()
            .map(|(i, l)| format!("{:6}: {}", start + i + 1, l)).collect();
        let mut out = selected.join("\n");
        if end < lines.len() { out.push_str(&format!("\n... ({} lines remaining)", lines.len() - end)); }
        ToolResult::text(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext { ToolContext::new(std::env::current_dir().unwrap()) }

    #[tokio::test]
    async fn reads_self() {
        let r = ReadTool.execute(ToolParams::new(serde_json::json!({"filePath": file!()})), &ctx()).await;
        assert!(r.into_text().contains("ReadTool"));
    }

    #[tokio::test]
    async fn nonexistent_file() {
        let r = ReadTool.execute(ToolParams::new(serde_json::json!({"filePath": "/no/such"})), &ctx()).await;
        assert!(r.into_text().contains("Cannot read"));
    }

    #[tokio::test]
    async fn with_offset_limit() {
        let r = ReadTool.execute(ToolParams::new(serde_json::json!({
            "filePath": file!(), "offset": 1, "limit": 3
        })), &ctx()).await;
        assert!(r.into_text().contains("1:"));
    }
}
