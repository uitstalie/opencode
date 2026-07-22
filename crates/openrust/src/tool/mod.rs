//! Tool system — core types, registry, permission model, and LLM integration.
//!
//! ## Architecture
//! - Each tool implements `Tool`. The trait provides `name()`, `description()`,
//!   `parameters()` (JSON Schema), and `execute()`.
//! - `ToolParams` wraps `serde_json::Value` with typed accessors.
//! - `try_tool!` / `require_str!` macros provide early-return-on-error for
//!   tools, since `ToolResult` is not `Result` and doesn't support `?`.
//! - `Tool::to_llm_def()` converts a tool into an OpenAI-compatible
//!   `{ type: "function", function: { name, description, parameters } }`.
//! - `catalog::create_tool()` is the factory used at runtime; each tool
//!   implements `Tool`. The trait provides `name()`, `description()`,
//!   `parameters()` (JSON Schema), and `execute()`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub mod catalog;
pub use undo::UndoStore;

pub mod apply_patch;
pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod memory_read;
pub mod memory_record;
pub mod question;
pub mod read;
pub mod rm;
pub mod shell;
pub mod skill;
pub mod task;
pub mod todowrite;
pub mod undo;
pub mod undo_edit;
pub mod webfetch;
pub mod websearch;
pub mod write;

// ── Helpers: early-return macros for ToolResult ─────

/// Truncate a string to at most `max_bytes` bytes, backing off to the
/// nearest UTF-8 char boundary to avoid panics on multibyte content.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// `try_tool!(expression, |e| format!("...{}", e))` — early-return `ToolResult::Error` on Err.
#[macro_export]
macro_rules! try_tool {
    ($expr:expr, |$err:ident| $ctx:expr) => {
        match $expr {
            Ok(v) => v,
            Err($err) => return $crate::tool::ToolResult::error(format!("{}: {}", $ctx, $err)),
        }
    };
}

/// `require_str!(params, "key")` — extracts required string or early-returns error.
#[macro_export]
macro_rules! require_str {
    ($p:expr, $key:literal) => {
        match $p.require_str($key) {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

/// `try_opt!(expression, "message")` — early-return `ToolResult::Error` on None.
#[macro_export]
macro_rules! try_opt {
    ($expr:expr, $msg:literal) => {
        match $expr {
            Some(v) => v,
            None => return $crate::tool::ToolResult::error($msg),
        }
    };
}

// ── ToolParams: typed JSON parameter access ─────────

/// Wraps `serde_json::Value` with ergonomic typed accessors.
/// Every tool uses this instead of raw `params["key"].as_str()`.
#[derive(Debug, Clone)]
pub struct ToolParams {
    raw: serde_json::Value,
}

impl ToolParams {
    pub fn new(raw: serde_json::Value) -> Self {
        Self { raw }
    }

    /// Access the raw JSON for permission checks / debugging.
    pub fn raw_value(&self) -> &serde_json::Value {
        &self.raw
    }

    /// Require a string parameter. Returns `ToolResult::Error` if missing.
    pub fn require_str(&self, key: &str) -> Result<&str, ToolResult> {
        self.raw[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ToolResult::error(format!("Missing required parameter: {}", key)))
    }

    /// Optional string parameter.
    pub fn opt_str(&self, key: &str) -> Option<&str> {
        self.raw[key].as_str()
    }

    /// Optional u64 with default value.
    pub fn u64_or(&self, key: &str, default: u64) -> u64 {
        self.raw[key].as_u64().unwrap_or(default)
    }

    /// Optional bool parameter (defaults to false).
    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.raw[key].as_bool().unwrap_or(default)
    }
}

// ── Types ──────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ToolResult {
    Text(String),
    Image {
        mime_type: String,
        base64_data: String,
        description: String,
    },
    Structured {
        content: String,
        metadata: HashMap<String, serde_json::Value>,
    },
    Error(String),
}

impl ToolResult {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }
    pub fn error(s: impl Into<String>) -> Self {
        Self::Error(s.into())
    }
    pub fn into_text(self) -> String {
        match self {
            Self::Text(t) => t,
            Self::Image { description, .. } => description,
            Self::Structured { content, .. } => content,
            Self::Error(e) => e,
        }
    }
}

/// Interactive request types live in core (they travel on the session event
/// bus); re-exported here for tool authors' convenience.
pub use crate::core::event::{AskRequest, PermissionRequest};

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub interactive: bool,
    /// Project root for scope-based permission. Defaults to cwd if None.
    pub project_dir: Option<PathBuf>,
    pub undo_store: Option<Arc<UndoStore>>,
    /// Session identity for tools that persist state (todowrite).
    pub session_id: Option<String>,
    pub store: Option<crate::core::session::SessionStore>,
    /// Session event bus sender, tagged with this agent's session identity.
    /// Tools emit Ask/Permission/Progress events here. `None` disables
    /// interactive prompts and progress reporting.
    pub events: Option<crate::core::event::SessionEventSender>,
    /// LLM access for tools that run a nested agent loop (task).
    pub llm: Option<Arc<dyn crate::core::provider::LlmProvider>>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    /// Shared shutdown flag — checked by nested agent loops (task tool).
    pub shutdown: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Per-turn abort flag (ESC). Propagated into sub-agent loops so the
    /// user can interrupt a running sub-agent.
    pub abort: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Persist ephemeral sub-agent / background session histories for
    /// debugging (config `persist_agent_sessions`).
    pub persist_agent_sessions: bool,
    /// Tool presets for resolving agent `tools` specs (config overlay + builtins).
    pub presets: std::collections::HashMap<String, Vec<String>>,
}

impl ToolContext {
    /// Convenience: create context with cwd as project root, non-interactive, no undo.
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            interactive: false,
            project_dir: None,
            undo_store: None,
            session_id: None,
            store: None,
            events: None,
            llm: None,
            model: None,
            reasoning_effort: None,
            shutdown: None,
            abort: None,
            persist_agent_sessions: false,
            presets: std::collections::HashMap::new(),
        }
    }

    /// The effective project root (project_dir or cwd).
    pub fn project_root(&self) -> &PathBuf {
        self.project_dir.as_ref().unwrap_or(&self.cwd)
    }
}

// ── Project scope ──────────────────────────────────

pub fn resolve_path(ctx: &ToolContext, path: &str) -> PathBuf {
    let candidate = std::path::Path::new(path);
    if candidate.is_absolute() {
        return candidate.to_path_buf();
    }
    ctx.cwd.join(candidate)
}

use crate::core::permission::Decision;

/// Check if a tool is allowed. Delegates to the core permission policy.
/// For bash, routes through `evaluate_bash` (dangerous check + scope).
/// For multi-path tools (apply_patch), checks every target.
pub fn check_permission(
    tool_name: &str,
    params: &serde_json::Value,
    ctx: &ToolContext,
) -> Decision {
    // bash: dedicated flow — dangerous command check + path scope check
    if tool_name == "bash"
        && let Some(cmd) = params["command"].as_str() {
            return crate::core::permission::evaluate_bash(
                cmd,
                ctx.project_root(),
                ctx.interactive,
            );
        }

    // Other tools: extract paths and scope-check each
    let targets = crate::core::permission::target_paths(params);
    for target in &targets {
        match crate::core::permission::evaluate(
            tool_name,
            target,
            ctx.project_root(),
            ctx.interactive,
        ) {
            Decision::Allow => continue,
            other => return other,
        }
    }
    Decision::Allow
}

/// Execute a tool by name against a context (parsing raw JSON args), returning
/// the full ToolResult. Shared by the TUI worker and the nested-agent runner.
pub(crate) async fn run_tool(name: &str, args: &str, ctx: &ToolContext) -> ToolResult {
    let Some(tool) = catalog::create_tool(name, ctx.undo_store.clone()) else {
        return ToolResult::error(format!("Unknown tool: {}", name));
    };
    let parsed: serde_json::Value = match serde_json::from_str(args) {
        Ok(v) => v,
        Err(e) => return ToolResult::error(format!("Invalid JSON arguments for '{}': {}", name, e)),
    };
    tool.execute_checked(ToolParams::new(parsed), ctx)
        .await
}

// ── Tool Trait ─────────────────────────────────────

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> serde_json::Value;
    async fn execute(&self, params: ToolParams, ctx: &ToolContext) -> ToolResult;

    /// Execute with permission check. The canonical entry point for tool invocation.
    async fn execute_checked(&self, params: ToolParams, ctx: &ToolContext) -> ToolResult {
        let raw = params.raw_value().clone();
        match check_permission(self.name(), &raw, ctx) {
            Decision::Allow => self.execute(params, ctx).await,
            Decision::Deny(reason) => ToolResult::error(reason),
            Decision::Ask(reason) => {
                // Only interactive agents may raise permission prompts;
                // background agents have no one to answer them.
                let Some(events) = ctx.events.as_ref().filter(|_| ctx.interactive) else {
                    return ToolResult::error(format!("{}: {} (denied)", self.name(), reason));
                };
                let (responder, decision_rx) = std::sync::mpsc::channel();
                events.send(crate::core::event::EventPayload::Permission(
                    PermissionRequest {
                        tool: self.name().to_string(),
                        detail: reason.clone(),
                        responder,
                    },
                ));
                match decision_rx.recv() {
                    Ok(true) => self.execute(params, ctx).await,
                    _ => ToolResult::error(format!("{}: denied by user", self.name())),
                }
            }
        }
    }

    /// Generate the OpenAI-compatible tool definition sent to the LLM.
    ///
    /// Format:
    /// ```json
    /// {
    ///   "type": "function",
    ///   "function": {
    ///     "name": "read",
    ///     "description": "Read a file from the filesystem...",
    ///     "parameters": { "type": "object", "properties": {...}, "required": [...] }
    ///   }
    /// }
    /// ```
    fn to_llm_def(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.parameters(),
            }
        })
    }
}

/// Factory: get a tool by name (for CLI debug usage).
pub fn create_tool(name: &str, undo_store: Option<Arc<UndoStore>>) -> Option<Box<dyn Tool>> {
    catalog::create_tool(name, undo_store)
}
