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
//! - `ToolRegistry` holds all tools; `standard_registry()` builds the default set.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub use undo::UndoStore;

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod read;
pub mod undo;
pub mod undo_edit;
pub mod webfetch;
pub mod websearch;
pub mod write;

// ── Helpers: early-return macros for ToolResult ─────

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

    /// Optional u64 parameter (with default).
    pub fn opt_u64(&self, key: &str) -> Option<u64> {
        self.raw[key].as_u64()
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
    Structured { content: String, metadata: HashMap<String, serde_json::Value> },
    Error(String),
}

impl ToolResult {
    pub fn text(s: impl Into<String>) -> Self { Self::Text(s.into()) }
    pub fn error(s: impl Into<String>) -> Self { Self::Error(s.into()) }
    pub fn into_text(self) -> String {
        match self { Self::Text(t) => t, Self::Structured { content, .. } => content, Self::Error(e) => e }
    }
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub interactive: bool,
    pub undo_store: Option<Arc<UndoStore>>,
}

#[derive(Debug, Clone)]
pub enum Permission { Allow, Deny(String) }

pub fn check_permission(tool_name: &str, _params: &serde_json::Value, _ctx: &ToolContext) -> Permission {
    // Phase 1A: allow all in debug mode. TUI will gate write/bash later.
    let _ = (tool_name, _params, _ctx);
    Permission::Allow
}

// ── Tool Trait ─────────────────────────────────────

#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> serde_json::Value;
    async fn execute(&self, params: ToolParams, ctx: &ToolContext) -> ToolResult;

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

// ── Registry ───────────────────────────────────────

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: HashMap::new() } }

    pub fn register(&mut self, tool: impl Tool + 'static) {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Build the full set of tool definitions for the LLM request.
    pub fn to_llm_defs(&self) -> Vec<serde_json::Value> {
        self.tools.values().map(|t| t.to_llm_def()).collect()
    }
}

/// Build the standard Phase 1A tool set.
pub fn standard_registry(undo_store: Option<Arc<UndoStore>>) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(read::ReadTool);
    reg.register(write::WriteTool);
    reg.register(edit::EditTool);
    reg.register(bash::BashTool);
    reg.register(glob::GlobTool);
    reg.register(grep::GrepTool);
    reg.register(webfetch::WebFetchTool);
    reg.register(websearch::WebSearchTool);
    reg.register(undo_edit::UndoEditTool { undo_store });
    reg
}

/// Factory: get a tool by name (for CLI debug usage).
pub fn create_tool(name: &str, undo_store: Option<Arc<UndoStore>>) -> Option<Box<dyn Tool>> {
    match name {
        "read" => Some(Box::new(read::ReadTool)),
        "write" => Some(Box::new(write::WriteTool)),
        "edit" => Some(Box::new(edit::EditTool)),
        "bash" => Some(Box::new(bash::BashTool)),
        "glob" => Some(Box::new(glob::GlobTool)),
        "grep" => Some(Box::new(grep::GrepTool)),
        "webfetch" => Some(Box::new(webfetch::WebFetchTool)),
        "websearch" => Some(Box::new(websearch::WebSearchTool)),
        "undo_edit" => Some(Box::new(undo_edit::UndoEditTool { undo_store })),
        _ => None,
    }
}
