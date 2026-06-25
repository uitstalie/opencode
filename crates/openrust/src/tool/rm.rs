//! Rm tool — safe file & directory deletion.
//!
//! Safety: delegates system-path checks to `core::paths::is_protected_path`.
//! Project-scope permission goes through `Tool::execute_checked`.

use serde_json::Value;
use std::path::Path;
use crate::core::paths;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};

pub struct RmTool;

#[async_trait::async_trait]
impl Tool for RmTool {
    fn name(&self) -> &'static str { "rm" }
    fn description(&self) -> &'static str {
        "Delete a file or directory. Directories require recursive: true. System paths are refused."
    }
    fn parameters(&self) -> Value { serde_json::json!({
        "type": "object",
        "properties": {
            "target": { "type": "string", "description": "Absolute path to delete" },
            "recursive": { "type": "boolean", "description": "Required for directories (default false)" },
            "force": { "type": "boolean", "description": "Skip 'not found' errors (default false)" }
        },
        "required": ["target"]
    })}

    async fn execute(&self, p: ToolParams, _ctx: &ToolContext) -> ToolResult {
        let target = require_str!(p, "target");
        let recursive = p.bool_or("recursive", false);
        let force = p.bool_or("force", false);
        let path = Path::new(target);

        let canonical = match path.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                if force { return ToolResult::text(format!("{} does not exist (skipped).", target)); }
                return ToolResult::error(format!("{}: not found. Use force: true to skip.", target));
            }
        };

        if let Some(reason) = paths::is_protected_path(&canonical) {
            return ToolResult::error(format!("Refusing to delete {}: {}", target, reason));
        }

        if canonical.is_dir() {
            if !recursive {
                return ToolResult::error(format!(
                    "{} is a directory. Add recursive: true to delete.", target
                ));
            }
            try_tool!(std::fs::remove_dir_all(&canonical), |e| format!("Delete dir: {}", e));
            ToolResult::text(format!("Deleted directory {}", target))
        } else {
            try_tool!(std::fs::remove_file(&canonical), |e| format!("Delete: {}", e));
            ToolResult::text(format!("Deleted {}", target))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext { ToolContext::new(std::env::current_dir().unwrap()) }

    #[tokio::test]
    async fn delete_file() {
        let tmp = "/tmp/openrust_rm_file.txt";
        std::fs::write(tmp, "data").unwrap();
        // Bypass scope check for logic test — /tmp is outside cwd
        let mut ctx = ToolContext::new(std::env::current_dir().unwrap());
        ctx.interactive = true;
        let r = RmTool.execute_checked(ToolParams::new(serde_json::json!({
            "target": tmp
        })), &ctx).await;
        assert!(r.into_text().contains("Deleted"));
        assert!(!Path::new(tmp).exists());
    }

    #[tokio::test]
    async fn refuses_root() {
        let r = RmTool.execute(ToolParams::new(serde_json::json!({
            "target": "/"
        })), &ctx()).await;
        assert!(r.into_text().contains("Refusing"));
    }

    #[tokio::test]
    async fn dir_without_recursive() {
        let tmp = "/tmp/openrust_rm_dir";
        let _ = std::fs::create_dir_all(tmp);
        let r = RmTool.execute(ToolParams::new(serde_json::json!({
            "target": tmp
        })), &ctx()).await;
        assert!(r.into_text().contains("recursive"));
        assert!(Path::new(tmp).exists());
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn dir_with_recursive() {
        let tmp = "/tmp/openrust_rm_dir_rec";
        let _ = std::fs::create_dir_all(tmp);
        let r = RmTool.execute(ToolParams::new(serde_json::json!({
            "target": tmp, "recursive": true
        })), &ctx()).await;
        assert!(r.into_text().contains("Deleted directory"));
        assert!(!Path::new(tmp).exists());
    }

    #[tokio::test]
    async fn force_missing() {
        let r = RmTool.execute(ToolParams::new(serde_json::json!({
            "target": "/tmp/openrust_nonexistent_xyz", "force": true
        })), &ctx()).await;
        assert!(r.into_text().contains("skipped"));
    }

    #[tokio::test]
    async fn outside_project_denied() {
        let r = RmTool.execute_checked(ToolParams::new(serde_json::json!({
            "target": "/tmp/some_file"
        })), &ctx()).await;
        assert!(r.into_text().contains("outside the project scope"));
    }
}
