//! Shell tool — command execution with timeout and cwd.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, resolve_path};
use crate::{require_str, try_tool};
use serde_json::Value;
use std::process::Stdio;
use std::time::{Duration, Instant};

pub struct BashTool;

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn description(&self) -> &'static str {
        "Execute a shell command with timeout. Output is captured and returned."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Command to execute" },
                "timeout": { "type": "integer", "description": "Timeout in ms (default 120000)" },
                "workdir": { "type": "string", "description": "Working directory" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let cmd_str = require_str!(p, "command");
        let timeout_ms = p.u64_or("timeout", 120_000);
        let workdir = p
            .opt_str("workdir")
            .map(|value| resolve_path(ctx, value))
            .unwrap_or_else(|| ctx.cwd.clone());
        let shell_kind = crate::tool::shell::detect_shell_kind();
        let (shell_bin, shell_args) = crate::tool::shell::shell_command(shell_kind);

        let mut child = try_tool!(
            std::process::Command::new(shell_bin)
                .args(shell_args)
                .arg(cmd_str)
                .current_dir(&workdir)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env_remove("LD_PRELOAD")
                .env_remove("LD_LIBRARY_PATH")
                .spawn(),
            |e| format!("Spawn failed: {}", e)
        );

        // Drain stdout/stderr in separate threads to prevent pipe-buffer
        // deadlock when the child writes more than ~64 KB.
        let stdout_handle = {
            let pipe = child.stdout.take();
            std::thread::spawn(move || {
                use std::io::Read;
                let mut buf = String::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_string(&mut buf);
                }
                buf
            })
        };
        let stderr_handle = {
            let pipe = child.stderr.take();
            std::thread::spawn(move || {
                use std::io::Read;
                let mut buf = String::new();
                if let Some(mut p) = pipe {
                    let _ = p.read_to_string(&mut buf);
                }
                buf
            })
        };

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout = stdout_handle.join().unwrap_or_default();
                    let stderr = stderr_handle.join().unwrap_or_default();
                    let mut result = String::new();
                    if !stdout.is_empty() {
                        result.push_str(&stdout);
                    }
                    if !stderr.is_empty() {
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result.push_str("[stderr]\n");
                        result.push_str(&stderr);
                    }
                    if result.is_empty() {
                        result = format!("exit code {}", status.code().unwrap_or(-1));
                    }
                    return ToolResult::text(result.trim().to_string());
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
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
    fn ctx() -> ToolContext {
        ToolContext::new(std::env::current_dir().unwrap())
    }

    #[tokio::test]
    async fn echo() {
        let r = BashTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "command": "echo hello", "timeout": 5000
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("hello"));
    }

    #[tokio::test]
    async fn timeout_kills() {
        let sleep_cmd = match crate::tool::shell::detect_shell_kind() {
            crate::tool::shell::ShellKind::Cmd => "ping -n 11 127.0.0.1 > NUL",
            _ if cfg!(windows) => "Start-Sleep -Seconds 10",
            _ => "sleep 10",
        };
        let r = BashTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "command": sleep_cmd, "timeout": 300
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("Timed out"));
    }

    #[tokio::test]
    async fn exit_code() {
        let r = BashTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "command": "exit 42", "timeout": 5000
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("42"));
    }
}
