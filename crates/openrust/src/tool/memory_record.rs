//! memory_record tool — append a memory entry to a `.md` file.
//!
//! See `core::memory` for storage details.

use crate::core::memory;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use crate::require_str;
use crate::try_opt;
use crate::try_tool;

pub struct MemoryRecordTool;

#[async_trait::async_trait]
impl Tool for MemoryRecordTool {
    fn name(&self) -> &'static str {
        "memory_record"
    }

    fn description(&self) -> &'static str {
        "Record a memory entry. Memory is NOT injected into the system prompt \
         — use memory_read to retrieve it when needed. Only record stable \
         conclusions, decisions, preferences, or patterns. Never record \
         transient state or single-use operations."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The memory content (single line, newlines become spaces)."
                },
                "scope": {
                    "type": "string",
                    "enum": ["project", "user", "dreaming"],
                    "description": "project = current project; user = global preferences; dreaming = cross-session patterns."
                },
                "category": {
                    "type": "string",
                    "description": "Required for project/user. project: progress|TODO|tech|conclusion. user: preferences|constraints|patterns|style. Ignored for dreaming."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags: #confirmed #likely #decision #architecture #constraint #preference #pattern #style #issue"
                }
            },
            "required": ["content", "scope"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let content = require_str!(p, "content");
        let scope_str = require_str!(p, "scope");

        let scope = match memory::Scope::parse(scope_str) {
            Some(s) => s,
            None => {
                return ToolResult::error(format!(
                    "Invalid scope '{}': expected project|user|dreaming",
                    scope_str
                ))
            }
        };

        // category validation
        let category = p.opt_str("category");
        if scope != memory::Scope::Dreaming {
            let cat = try_opt!(
                category,
                "category is required for project/user scope \
                 (project: progress|TODO|tech|conclusion, \
                 user: preferences|constraints|patterns|style)"
            );
            if !memory::is_valid_category(scope, cat) {
                return ToolResult::error(format!(
                    "Invalid category '{}' for scope {}. Valid: {}",
                    cat,
                    scope.as_str(),
                    memory::valid_categories(scope).join("|")
                ));
            }
        }

        // tags
        let tags: Vec<String> = p.raw_value()["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let path = memory::resolve_path(scope, category, &ctx.cwd);

        let written = try_tool!(
            memory::append_entry(&path, content, &tags),
            |e| format!("Failed to write memory: {e}")
        );

        let location = path.to_string_lossy();
        if written {
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("scope".to_string(), serde_json::json!(scope.as_str()));
            if let Some(cat) = category {
                metadata.insert("category".to_string(), serde_json::json!(cat));
            }
            metadata.insert("status".to_string(), serde_json::json!("recorded"));

            ToolResult::Structured {
                content: format!("Memory recorded → {location}"),
                metadata,
            }
        } else {
            ToolResult::text("Duplicate memory — already exists, skipped.")
        }
    }
}
