//! Write file tool. Creates undo blob before overwriting.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, resolve_path};
use crate::{require_str, try_tool};
use serde_json::Value;

pub struct WriteTool;

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }
    fn description(&self) -> &'static str {
        "Write content to a file (create or overwrite). Creates an undo blob for existing files. Relative paths resolve from cwd."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to write relative to cwd, or absolute path" },
                "filePath": { "type": "string", "description": "Legacy alias for path" },
                "content": { "type": "string", "description": "Content to write" }
            },
            "required": ["path", "content"]
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
        let content = require_str!(p, "content");
        let path = resolve_path(ctx, path_str);

        if let Err(reason) = crate::core::paths::check_protected(&path) {
            return ToolResult::error(format!("Refusing to write {}: {}", path.display(), reason));
        }

        let undo_hash = ctx.undo_store.as_ref().and_then(|store| {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|existing| store.save_snapshot(&path, &existing))
        });

        if let Some(parent) = path.parent() {
            try_tool!(std::fs::create_dir_all(parent), |e| format!(
                "Cannot create dir: {}",
                e
            ));
        }
        try_tool!(std::fs::write(&path, content), |e| format!(
            "Write failed: {}",
            e
        ));

        let mut msg = format!("Wrote {} bytes to {}", content.len(), path.display());
        if let Some(h) = undo_hash {
            msg.push_str(&format!("\nUndo hash: {}", h));
        }
        ToolResult::text(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::UndoStore;
    use std::sync::Arc;

    fn ctx(undo: Option<Arc<UndoStore>>) -> ToolContext {
        ToolContext {
            undo_store: undo,
            ..ToolContext::new(std::env::current_dir().unwrap())
        }
    }

    #[tokio::test]
    async fn writes_new_file() {
        let tmp = "/tmp/openrust_test_write_new.txt";
        let _ = std::fs::remove_file(tmp);
        let r = WriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": tmp, "content": "hello"
                })),
                &ctx(None),
            )
            .await;
        assert!(r.into_text().contains("Wrote"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "hello");
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn writes_with_undo() {
        let store = Arc::new(UndoStore::new());
        let tmp = "/tmp/openrust_test_write_undo.txt";
        std::fs::write(tmp, "orig").unwrap();
        let r = WriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": tmp, "content": "mod"
                })),
                &ctx(Some(store)),
            )
            .await;
        assert!(r.into_text().contains("Undo hash"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "mod");
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn path_alias_resolves_relative_to_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let r = WriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "path": "nested/out.txt", "content": "hi"
                })),
                &ctx,
            )
            .await;
        assert!(r.into_text().contains("Wrote"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("nested/out.txt")).unwrap(),
            "hi"
        );
    }

    #[tokio::test]
    async fn missing_path_reports_error() {
        let r = WriteTool
            .execute(
                ToolParams::new(serde_json::json!({"content": "x"})),
                &ctx(None),
            )
            .await;
        assert!(r.into_text().contains("Missing required parameter"));
    }

    #[tokio::test]
    async fn refuses_protected_path() {
        let protected = if cfg!(windows) {
            r"C:\Windows\System32\drivers\etc\hosts"
        } else {
            "/etc/passwd"
        };
        let r = WriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": protected, "content": "hacked"
                })),
                &ctx(None),
            )
            .await;
        let text = r.into_text();
        assert!(
            text.contains("Refusing"),
            "expected Refusing, got: {}",
            text
        );
    }
}
