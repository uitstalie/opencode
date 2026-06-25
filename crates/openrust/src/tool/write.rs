//! Write file tool. Creates undo blob before overwriting.

use serde_json::Value;
use std::path::Path;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};

pub struct WriteTool;

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str { "write" }
    fn description(&self) -> &'static str {
        "Write content to a file (create or overwrite). Creates an undo blob for existing files."
    }
    fn parameters(&self) -> Value { serde_json::json!({
        "type": "object",
        "properties": {
            "filePath": { "type": "string", "description": "Absolute path to write" },
            "content": { "type": "string", "description": "Content to write" }
        },
        "required": ["filePath", "content"]
    })}

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let path_str = require_str!(p, "filePath");
        let content = require_str!(p, "content");
        let path = Path::new(path_str);

        let undo_hash = ctx.undo_store.as_ref().and_then(|store| {
            std::fs::read_to_string(path).ok()
                .and_then(|existing| store.save_snapshot(path, &existing))
        });

        if let Some(parent) = path.parent() {
            try_tool!(std::fs::create_dir_all(parent), |e| format!("Cannot create dir: {}", e));
        }
        try_tool!(std::fs::write(path, content), |e| format!("Write failed: {}", e));

        let mut msg = format!("Wrote {} bytes to {}", content.len(), path_str);
        if let Some(h) = undo_hash { msg.push_str(&format!("\nUndo hash: {}", h)); }
        ToolResult::text(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::tool::UndoStore;

    fn ctx(undo: Option<Arc<UndoStore>>) -> ToolContext {
        ToolContext {
            cwd: std::env::current_dir().unwrap(),
            interactive: false,
            project_dir: None,
            undo_store: undo,
        }
    }

    #[tokio::test]
    async fn writes_new_file() {
        let tmp = "/tmp/opencode_test_write_new.txt"; let _ = std::fs::remove_file(tmp);
        let r = WriteTool.execute(ToolParams::new(serde_json::json!({
            "filePath": tmp, "content": "hello"
        })), &ctx(None)).await;
        assert!(r.into_text().contains("Wrote"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "hello");
        let _ = std::fs::remove_file(tmp);
    }

    #[tokio::test]
    async fn writes_with_undo() {
        let store = Arc::new(UndoStore::new());
        let tmp = "/tmp/opencode_test_write_undo.txt";
        std::fs::write(tmp, "orig").unwrap();
        let r = WriteTool.execute(ToolParams::new(serde_json::json!({
            "filePath": tmp, "content": "mod"
        })), &ctx(Some(store))).await;
        assert!(r.into_text().contains("Undo hash"));
        assert_eq!(std::fs::read_to_string(tmp).unwrap(), "mod");
        let _ = std::fs::remove_file(tmp);
    }
}
