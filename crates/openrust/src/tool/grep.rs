//! Grep tool — regex content search across files.

use crate::require_str;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use regex::Regex;
use serde_json::Value;

pub struct GrepTool;

#[async_trait::async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents with regex. Use include to filter by extension (e.g. '*.rs')."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern" },
                "path": { "type": "string", "description": "Directory or file (default: cwd)" },
                "include": { "type": "string", "description": "File filter, e.g. '*.rs' or '*.{ts,tsx}'" }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let pat = require_str!(p, "pattern");
        let cwd_str = ctx.cwd.to_string_lossy().to_string();
        let base = p.opt_str("path").unwrap_or(&cwd_str).to_string();
        let include = p.opt_str("include").map(|s| s.to_string());

        let re = match Regex::new(pat) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("Invalid regex: {}", e)),
        };

        let results = tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            let base_path = std::path::Path::new(&base);

            if base_path.is_file() {
                search_file(base_path, &re, &mut results);
            } else {
                for entry in walkdir::WalkDir::new(base_path)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|e| {
                        let n = e.file_name().to_string_lossy();
                        !n.starts_with('.') && n != "target" && n != "node_modules"
                    })
                    .filter_map(|e| e.ok())
                {
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    if let Some(ref inc) = include
                        && !match_ext(inc, &entry.file_name().to_string_lossy()) {
                            continue;
                        }
                    search_file(entry.path(), &re, &mut results);
                }
            }
            results
        })
        .await
        .unwrap_or_default();

        if results.is_empty() {
            return ToolResult::text("No matches found.");
        }
        let total = results.len();
        if total > 200 {
            let mut truncated = results;
            truncated.truncate(200);
            truncated.push(format!("... and {} more", total - 200));
            ToolResult::text(truncated.join("\n"))
        } else {
            ToolResult::text(results.join("\n"))
        }
    }
}

fn search_file(path: &std::path::Path, re: &Regex, out: &mut Vec<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };
    for (i, line) in content.lines().enumerate() {
        if re.is_match(line) {
            let display = if line.len() > 200 {
                format!("{}...", crate::tool::truncate_str(line, 200))
            } else {
                line.to_string()
            };
            out.push(format!("{}:{}: {}", path.display(), i + 1, display));
        }
    }
}

fn match_ext(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "*.*" {
        return true;
    }
    let pat = pattern.strip_prefix("./").unwrap_or(pattern);
    if let Some(ext) = pat.strip_prefix("*.") {
        return name.ends_with(&format!(".{}", ext));
    }
    glob::Pattern::new(pat)
        .map(|p| p.matches(name))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext {
        ToolContext::new(std::env::current_dir().unwrap())
    }

    #[tokio::test]
    async fn finds_struct() {
        let cwd = std::env::current_dir().unwrap();
        let r = GrepTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "pattern": "struct GrepTool", "path": cwd.to_string_lossy(), "include": "*.rs"
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("grep.rs"));
    }

    #[tokio::test]
    async fn no_match() {
        let tmp = "/tmp/openrust_grep_nomatch_test";
        let _ = std::fs::create_dir_all(tmp);
        let r = GrepTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "pattern": "XYZZY_NOMATCH_12345", "path": tmp
                })),
                &ctx(),
            )
            .await;
        assert_eq!(r.into_text(), "No matches found.");
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn invalid_regex() {
        let r = GrepTool
            .execute(
                ToolParams::new(serde_json::json!({"pattern": "[bad"})),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("Invalid regex"));
    }
}
