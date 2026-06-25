//! Bash tool — shell command execution with timeout and cwd.

use serde_json::Value;
use std::process::Stdio;
use std::time::{Duration, Instant};
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::{require_str, try_tool};

pub struct BashTool;

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str { "bash" }
    fn description(&self) -> &'static str {
        "Execute a shell command. Use for git, build, and system operations. Respects timeout."
    }
    fn parameters(&self) -> Value { serde_json::json!({
        "type": "object",
        "properties": {
            "command": { "type": "string", "description": "Command to execute" },
            "description": { "type": "string", "description": "Short description (5-10 words)" },
            "timeout": { "type": "integer", "description": "Timeout in ms (default 120000)" },
            "workdir": { "type": "string", "description": "Working directory" }
        },
        "required": ["command"]
    })}

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let cmd_str = require_str!(p, "command");
        let timeout_ms = p.u64_or("timeout", 120_000);
        let cwd_str = ctx.cwd.to_string_lossy().to_string();
        let workdir = p.opt_str("workdir").unwrap_or(&cwd_str);

        let mut child = try_tool!(
            std::process::Command::new("bash")
                .arg("-c").arg(cmd_str)
                .current_dir(workdir)
                .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
                .env_remove("LD_PRELOAD").env_remove("LD_LIBRARY_PATH")
                .spawn(),
            |e| format!("Spawn failed: {}", e)
        );

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let out = child.wait_with_output().unwrap_or(std::process::Output {
                        status, stdout: vec![], stderr: vec![],
                    });
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let mut result = String::new();
                    if !stdout.is_empty() { result.push_str(&stdout); }
                    if !stderr.is_empty() {
                        if !result.is_empty() { result.push('\n'); }
                        result.push_str("[stderr]\n"); result.push_str(&stderr);
                    }
                    if result.is_empty() { result = format!("exit code {}", status.code().unwrap_or(-1)); }
                    return ToolResult::text(result.trim().to_string());
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill(); let _ = child.wait();
                        return ToolResult::error(format!("Timed out after {}ms", timeout_ms));
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => return ToolResult::error(format!("Process error: {}", e)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext { ToolContext { cwd: std::env::current_dir().unwrap(), interactive: false, undo_store: None } }

    #[tokio::test]
    async fn echo() {
        let r = BashTool.execute(ToolParams::new(serde_json::json!({
            "command": "echo hello", "timeout": 5000
        })), &ctx()).await;
        assert!(r.into_text().contains("hello"));
    }

    #[tokio::test]
    async fn timeout_kills() {
        let r = BashTool.execute(ToolParams::new(serde_json::json!({
            "command": "sleep 10", "timeout": 300
        })), &ctx()).await;
        assert!(r.into_text().contains("Timed out"));
    }

    #[tokio::test]
    async fn exit_code() {
        let r = BashTool.execute(ToolParams::new(serde_json::json!({
            "command": "exit 42", "timeout": 5000
        })), &ctx()).await;
        assert!(r.into_text().contains("42"));
    }
}
