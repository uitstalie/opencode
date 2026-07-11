//! Edit file tool — exact `oldString` → `newString` replacement.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, resolve_path};
use crate::{require_str, try_tool};
use serde_json::Value;

pub struct EditTool;

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &'static str {
        "edit"
    }
    fn description(&self) -> &'static str {
        "Replace an exact string in a file. Fails if the string appears multiple times (use replaceAll to override). Relative paths resolve from cwd."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to edit relative to cwd, or absolute path" },
                "filePath": { "type": "string", "description": "Legacy alias for path" },
                "oldString": { "type": "string", "description": "Exact text to replace" },
                "newString": { "type": "string", "description": "Replacement text (must differ from oldString)" },
                "replaceAll": { "type": "boolean", "description": "Replace all occurrences (default false)" }
            },
            "required": ["path", "oldString", "newString"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let path_str = p
            .opt_str("path")
            .or_else(|| p.opt_str("filePath"))
            .unwrap_or("");
        if path_str.is_empty() {
            return ToolResult::error("Missing required parameter: path");
        }
        let old = require_str!(p, "oldString");
        let new = require_str!(p, "newString");
        let replace_all = p.bool_or("replaceAll", false);

        if old == new {
            return ToolResult::error("oldString and newString must differ");
        }

        let path = resolve_path(ctx, path_str);

        if let Err(reason) = crate::core::paths::check_protected(&path) {
            return ToolResult::error(format!("Refusing to edit {}: {}", path.display(), reason));
        }

        let content = try_tool!(std::fs::read_to_string(&path), |e| format!(
            "Cannot read: {}",
            e
        ));

        let matches: Vec<_> = content.match_indices(old).collect();
        if matches.is_empty() {
            return ToolResult::error(format!(
                "oldString not found in {}. Check text and indentation.",
                path.display()
            ));
        }
        if !replace_all && matches.len() > 1 {
            return ToolResult::error(format!(
                "Found {} matches. Add more context or set replaceAll: true.",
                matches.len()
            ));
        }

        let undo_hash = ctx
            .undo_store
            .as_ref()
            .and_then(|s| s.save_snapshot(&path, &content));
        let modified = content.replace(old, new);
        try_tool!(std::fs::write(&path, &modified), |e| format!(
            "Write failed: {}",
            e
        ));

        let count = if replace_all { matches.len() } else { 1 };
        let label = if count == 1 {
            "1 occurrence"
        } else {
            &format!("{} occurrences", count)
        };
        let mut msg = format!("Replaced {} in {}", label, path.display());
        if let Some(h) = undo_hash {
            msg.push_str(&format!("\nUndo hash: {}", h));
        }
        ToolResult::text(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext {
        ToolContext::new(std::env::current_dir().unwrap())
    }

    #[tokio::test]
    async fn single_replace() {
        let tmp = "/tmp/openrust_edit_single.txt";
        std::fs::write(tmp, "hello\nfoo bar\n").unwrap();
        let r = EditTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": tmp, "oldString": "foo bar", "newString": "baz"
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("1 occurrence"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "hello\nbaz\n");
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn multiple_matches_rejected() {
        let tmp = "/tmp/openrust_edit_multi.txt";
        std::fs::write(tmp, "foo\nfoo\n").unwrap();
        let r = EditTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": tmp, "oldString": "foo", "newString": "bar"
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("2 matches"));
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn replace_all() {
        let tmp = "/tmp/openrust_edit_all.txt";
        std::fs::write(tmp, "foo\nfoo\n").unwrap();
        let r = EditTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": tmp, "oldString": "foo", "newString": "bar", "replaceAll": true
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("2 occurrences"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "bar\nbar\n");
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn path_alias_resolves_relative_to_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha\n").unwrap();
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let r = EditTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "path": "f.txt", "oldString": "alpha", "newString": "beta"
                })),
                &ctx,
            )
            .await;
        assert!(r.into_text().contains("1 occurrence"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "beta\n"
        );
    }
}
