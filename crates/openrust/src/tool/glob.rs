//! Glob tool — file pattern matching.

use crate::require_str;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use serde_json::Value;

pub struct GlobTool;

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }
    fn description(&self) -> &'static str {
        "Find files matching a glob pattern (e.g. '**/*.rs', 'src/**/*.ts')."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (supports **, *, ?, [sets])" },
                "path": { "type": "string", "description": "Base directory (default: cwd)" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let pattern = require_str!(p, "pattern");
        let cwd_str = ctx.cwd.to_string_lossy().to_string();
        let base = p.opt_str("path").unwrap_or(&cwd_str);

        let full = if std::path::Path::new(pattern).is_absolute() {
            pattern.to_string()
        } else {
            format!(
                "{}/{}",
                crate::core::paths::trim_trailing(base),
                crate::core::paths::trim_leading(pattern),
            )
        };

        let mut paths: Vec<String> = match glob::glob(&full) {
            Ok(iter) => iter
                .filter_map(|r| r.ok())
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            Err(e) => return ToolResult::error(format!("Invalid glob: {}", e)),
        };
        if paths.is_empty() {
            return ToolResult::text("No files matched.");
        }
        paths.sort();
        let total = paths.len();
        if total > 500 {
            paths.truncate(500);
        }
        let mut out = paths.join("\n");
        if total > 500 {
            out.push_str(&format!("\n... and {} more", total - 500));
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
    async fn finds_rs_files() {
        let cwd = std::env::current_dir().unwrap();
        let r = GlobTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "pattern": "src/**/*.rs", "path": cwd.to_string_lossy()
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains(".rs"));
    }

    #[tokio::test]
    async fn no_match() {
        let r = GlobTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "pattern": "zzz_nonexistent_*.xyz"
                })),
                &ctx(),
            )
            .await;
        assert_eq!(r.into_text(), "No files matched.");
    }
}
