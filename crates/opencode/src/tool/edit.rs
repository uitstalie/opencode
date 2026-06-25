//! Edit file tool — exact `oldString` → `newString` replacement.

use serde_json::Value;
use std::path::Path;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};

pub struct EditTool;

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &'static str { "edit" }
    fn description(&self) -> &'static str {
        "Replace an exact string in a file. Fails if the string appears multiple times (use replaceAll to override)."
    }
    fn parameters(&self) -> Value { serde_json::json!({
        "type": "object",
        "properties": {
            "filePath": { "type": "string", "description": "Absolute path" },
            "oldString": { "type": "string", "description": "Exact text to replace" },
            "newString": { "type": "string", "description": "Replacement text (must differ from oldString)" },
            "replaceAll": { "type": "boolean", "description": "Replace all occurrences (default false)" }
        },
        "required": ["filePath", "oldString", "newString"]
    })}

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let path_str = require_str!(p, "filePath");
        let old = require_str!(p, "oldString");
        let new = require_str!(p, "newString");
        let replace_all = p.bool_or("replaceAll", false);

        if old == new { return ToolResult::error("oldString and newString must differ"); }

        let path = Path::new(path_str);
        let content = try_tool!(std::fs::read_to_string(path), |e| format!("Cannot read: {}", e));

        let matches: Vec<_> = content.match_indices(old).collect();
        if matches.is_empty() {
            return ToolResult::error(format!("oldString not found in {}. Check text and indentation.", path_str));
        }
        if !replace_all && matches.len() > 1 {
            return ToolResult::error(format!("Found {} matches. Add more context or set replaceAll: true.", matches.len()));
        }

        let undo_hash = ctx.undo_store.as_ref().and_then(|s| s.save_snapshot(path, &content));
        let modified = content.replace(old, new);
        try_tool!(std::fs::write(path, &modified), |e| format!("Write failed: {}", e));

        let count = if replace_all { matches.len() } else { 1 };
        let label = if count == 1 { "1 occurrence" } else { &format!("{} occurrences", count) };
        let mut msg = format!("Replaced {} in {}", label, path_str);
        if let Some(h) = undo_hash { msg.push_str(&format!("\nUndo hash: {}", h)); }
        ToolResult::text(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext { ToolContext { cwd: std::env::current_dir().unwrap(), interactive: false, undo_store: None } }

    #[tokio::test]
    async fn single_replace() {
        let tmp = "/tmp/opencode_edit_single.txt";
        std::fs::write(tmp, "hello\nfoo bar\n").unwrap();
        let r = EditTool.execute(ToolParams::new(serde_json::json!({
            "filePath": tmp, "oldString": "foo bar", "newString": "baz"
        })), &ctx()).await;
        assert!(r.into_text().contains("1 occurrence"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "hello\nbaz\n");
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn multiple_matches_rejected() {
        let tmp = "/tmp/opencode_edit_multi.txt";
        std::fs::write(tmp, "foo\nfoo\n").unwrap();
        let r = EditTool.execute(ToolParams::new(serde_json::json!({
            "filePath": tmp, "oldString": "foo", "newString": "bar"
        })), &ctx()).await;
        assert!(r.into_text().contains("2 matches"));
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn replace_all() {
        let tmp = "/tmp/opencode_edit_all.txt";
        std::fs::write(tmp, "foo\nfoo\n").unwrap();
        let r = EditTool.execute(ToolParams::new(serde_json::json!({
            "filePath": tmp, "oldString": "foo", "newString": "bar", "replaceAll": true
        })), &ctx()).await;
        assert!(r.into_text().contains("2 occurrences"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "bar\nbar\n");
        let _ = std::fs::remove_file(tmp);
    }
}
