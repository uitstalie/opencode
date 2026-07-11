//! UndoEdit tool — restore a file from undo blob.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, UndoStore};
use crate::{require_str, try_opt, try_tool};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

pub struct UndoEditTool {
    pub undo_store: Option<Arc<UndoStore>>,
}

#[async_trait::async_trait]
impl Tool for UndoEditTool {
    fn name(&self) -> &'static str {
        "undo_edit"
    }
    fn description(&self) -> &'static str {
        "Restore a file to its previous state using an undo hash from write/edit."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "filePath": { "type": "string", "description": "Absolute path to restore" },
                "path": { "type": "string", "description": "Alias for filePath" },
                "undoHash": { "type": "string", "description": "Undo hash from the prior write/edit" }
            },
            "required": ["filePath", "undoHash"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let path_str = match p.opt_str("filePath").or_else(|| p.opt_str("path")) {
            Some(s) => s,
            None => return ToolResult::error("undo_edit: missing required parameter: filePath"),
        };
        let hash = require_str!(p, "undoHash");
        let path = Path::new(path_str);

        if let Err(reason) = crate::core::paths::check_protected(path) {
            return ToolResult::error(format!("Refusing to restore {}: {}", path_str, reason));
        }

        let store = try_opt!(ctx.undo_store.as_ref(), "Undo store not available");
        let blob = match store.read_blob(hash) {
            Some(b) => b,
            None => {
                return ToolResult::error(format!(
                    "Blob not found: {}. May have been GC'd (24h TTL).",
                    hash
                ));
            }
        };

        let new_hash = std::fs::read_to_string(path)
            .ok()
            .and_then(|c| store.save_snapshot(path, &c));

        try_tool!(std::fs::write(path, &blob), |e| format!("Restore: {}", e));

        let mut msg = format!("Restored {} from undo blob.", path_str);
        if let Some(h) = new_hash {
            msg.push_str(&format!("\nUndo hash: {}", h));
        }
        ToolResult::text(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(store: Arc<UndoStore>) -> ToolContext {
        ToolContext {
            undo_store: Some(store),
            ..ToolContext::new(std::env::current_dir().unwrap())
        }
    }

    #[tokio::test]
    async fn restores_content() {
        let s = Arc::new(UndoStore::new());
        let tmp = "/tmp/openrust_undo_restore.txt";
        let orig = "original\n";
        std::fs::write(tmp, orig).unwrap();
        let hash = s.save_snapshot(Path::new(tmp), orig).unwrap();
        std::fs::write(tmp, "modified\n").unwrap();

        let r = UndoEditTool {
            undo_store: Some(s.clone()),
        }
        .execute(
            ToolParams::new(serde_json::json!({"filePath": tmp, "undoHash": hash})),
            &ctx(s),
        )
        .await;
        assert!(r.into_text().contains("Restored"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), orig);
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn chaining() {
        let s = Arc::new(UndoStore::new());
        let tmp = "/tmp/openrust_undo_chain.txt";
        let v1 = "v1\n";
        std::fs::write(tmp, v1).unwrap();
        let h1 = s.save_snapshot(Path::new(tmp), v1).unwrap();
        std::fs::write(tmp, "v2\n").unwrap();

        let r = UndoEditTool {
            undo_store: Some(s.clone()),
        }
        .execute(
            ToolParams::new(serde_json::json!({"filePath": tmp, "undoHash": h1})),
            &ctx(s.clone()),
        )
        .await;
        let text = r.into_text();
        assert!(text.contains("Restored") && text.contains("Undo hash:"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), v1);
        let _ = std::fs::remove_file(tmp);
    }
}
