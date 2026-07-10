//! Rm tool — safe file & directory deletion.
//!
//! Safety: delegates system-path checks to `core::paths::check_protected`.
//! Project-scope permission goes through `Tool::execute_checked`.

use crate::core::paths;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, resolve_path};
use crate::{require_str, try_tool};
use serde_json::Value;

pub struct RmTool;

#[async_trait::async_trait]
impl Tool for RmTool {
    fn name(&self) -> &'static str {
        "rm"
    }
    fn description(&self) -> &'static str {
        "Delete a file or directory. Directories require recursive: true. System paths are refused. Relative paths resolve from cwd."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "Path to delete relative to cwd, or absolute path" },
                "recursive": { "type": "boolean", "description": "Required for directories (default false)" },
                "force": { "type": "boolean", "description": "Skip 'not found' errors (default false)" }
            },
            "required": ["target"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let target = require_str!(p, "target");
        let recursive = p.bool_or("recursive", false);
        let force = p.bool_or("force", false);
        let path = resolve_path(ctx, target);

        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                if force {
                    return ToolResult::text(format!("{} does not exist (skipped).", target));
                }
                return ToolResult::error(format!(
                    "{}: not found. Use force: true to skip.",
                    target
                ));
            }
        };

        if let Err(reason) = paths::check_protected(&canonical) {
            return ToolResult::error(format!("Refusing to delete {}: {}", target, reason));
        }

        if canonical.is_dir() {
            if !recursive {
                return ToolResult::error(format!(
                    "{} is a directory. Add recursive: true to delete.",
                    canonical.display()
                ));
            }
            try_tool!(std::fs::remove_dir_all(&canonical), |e| format!(
                "Delete dir: {}",
                e
            ));
            ToolResult::text(format!("Deleted directory {}", canonical.display()))
        } else {
            if let Some(store) = &ctx.undo_store
                && let Ok(existing) = std::fs::read_to_string(&canonical) {
                    store.save_snapshot(&canonical, &existing);
                }
            try_tool!(std::fs::remove_file(&canonical), |e| format!(
                "Delete: {}",
                e
            ));
            ToolResult::text(format!("Deleted {}", canonical.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn ctx() -> ToolContext {
        ToolContext::new(std::env::current_dir().unwrap())
    }

    #[tokio::test]
    async fn delete_file() {
        let tmp = "/tmp/openrust_rm_file.txt";
        std::fs::write(tmp, "data").unwrap();
        // Scope the context to /tmp so the target is in-project (this tests rm logic, not permission).
        let ctx = ToolContext::new(std::path::PathBuf::from("/tmp"));
        let r = RmTool
            .execute_checked(
                ToolParams::new(serde_json::json!({
                    "target": tmp
                })),
                &ctx,
            )
            .await;
        assert!(r.into_text().contains("Deleted"));
        assert!(!Path::new(tmp).exists());
    }

    #[tokio::test]
    async fn refuses_root() {
        let root = if cfg!(windows) { r"C:\" } else { "/" };
        let r = RmTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "target": root
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("Refusing"));
    }

    #[tokio::test]
    async fn dir_without_recursive() {
        let tmp = "/tmp/openrust_rm_dir";
        let _ = std::fs::create_dir_all(tmp);
        let r = RmTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "target": tmp
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("recursive"));
        assert!(Path::new(tmp).exists());
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn dir_with_recursive() {
        let tmp = "/tmp/openrust_rm_dir_rec";
        let _ = std::fs::create_dir_all(tmp);
        let r = RmTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "target": tmp, "recursive": true
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("Deleted directory"));
        assert!(!Path::new(tmp).exists());
    }

    #[tokio::test]
    async fn force_missing() {
        let r = RmTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "target": "/tmp/openrust_nonexistent_xyz", "force": true
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("skipped"));
    }

    #[tokio::test]
    async fn outside_project_denied() {
        let r = RmTool
            .execute_checked(
                ToolParams::new(serde_json::json!({
                    "target": "/tmp/some_file"
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("outside the project scope"));
    }
}
