//! TodoWrite tool — maintain the session todo list, persisted to the session store.

use crate::core::session::Task;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use serde_json::Value;

pub struct TodoWriteTool;

#[async_trait::async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &'static str {
        "todowrite"
    }
    fn description(&self) -> &'static str {
        "Create and update the session todo list. Always pass the full list; it replaces the previous one. Use for multi-step work and keep exactly one item in_progress at a time."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The full, updated todo list",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string", "description": "Brief description of the task" },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"],
                                "description": "Current status of the task"
                            },
                            "priority": {
                                "type": "string",
                                "enum": ["high", "medium", "low"],
                                "description": "Priority level of the task"
                            }
                        },
                        "required": ["content", "status", "priority"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let (Some(store), Some(session_id)) = (ctx.store.as_ref(), ctx.session_id.as_ref()) else {
            return ToolResult::error("todowrite: session store is unavailable in this context");
        };
        let Some(items) = p.raw_value().get("todos").and_then(|v| v.as_array()) else {
            return ToolResult::error("todowrite: missing required parameter: todos");
        };

        let now = now_string();

        let existing = store.list_tasks(session_id).unwrap_or_default();
        let title_to_id: std::collections::HashMap<&str, &str> = existing
            .iter()
            .map(|t| (t.title.as_str(), t.id.as_str()))
            .collect();
        let mut next_id = existing
            .iter()
            .filter_map(|t| t.id.parse::<usize>().ok())
            .max()
            .unwrap_or(0)
            + 1;

        let tasks: Vec<Task> = items
            .iter()
            .map(|item| {
                let title = item
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let id = title_to_id
                    .get(title.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        let s = format!("{}", next_id);
                        next_id += 1;
                        s
                    });
                Task {
                    id,
                    agent: None,
                    title,
                    status: item
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending")
                        .to_string(),
                    priority: item
                        .get("priority")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                }
            })
            .collect();

        if let Err(err) = store.replace_tasks(session_id, &tasks) {
            return ToolResult::error(format!("todowrite: {}", err));
        }

        let rendered = tasks
            .iter()
            .map(|task| {
                let mark = match task.status.as_str() {
                    "completed" => "[x]",
                    "in_progress" => "[~]",
                    "cancelled" => "[-]",
                    _ => "[ ]",
                };
                let priority = task.priority.as_deref().unwrap_or("");
                format!("{} {} ({}, {})", mark, task.title, task.status, priority)
            })
            .collect::<Vec<_>>()
            .join("\n");

        ToolResult::text(format!(
            "Updated todo list ({} items):\n{}",
            tasks.len(),
            rendered
        ))
    }
}

fn now_string() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:09}", now.as_secs(), now.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session::SessionStore;

    fn store_ctx() -> (SessionStore, ToolContext, String) {
        let dir = std::env::temp_dir().join(format!("openrust-todo-{}", now_string()));
        let store = SessionStore::open_at(&dir).unwrap();
        let session_id = "todo-session".to_string();
        store.ensure_session(&session_id).unwrap();
        let mut ctx = ToolContext::new(dir);
        ctx.store = Some(store.clone());
        ctx.session_id = Some(session_id.clone());
        (store, ctx, session_id)
    }

    #[tokio::test]
    async fn writes_and_persists_todos() {
        let (store, ctx, session_id) = store_ctx();
        let result = TodoWriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "todos": [
                        { "content": "first", "status": "in_progress", "priority": "high" },
                        { "content": "second", "status": "pending", "priority": "low" }
                    ]
                })),
                &ctx,
            )
            .await;
        let text = result.into_text();
        assert!(text.contains("first"));
        assert!(text.contains("[~]"));

        let tasks = store.list_tasks(&session_id).unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(
            tasks
                .iter()
                .any(|t| t.title == "first" && t.status == "in_progress")
        );
    }

    #[tokio::test]
    async fn replaces_previous_list() {
        let (store, ctx, session_id) = store_ctx();
        TodoWriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "todos": [
                        { "content": "a", "status": "pending", "priority": "low" },
                        { "content": "b", "status": "pending", "priority": "low" },
                        { "content": "c", "status": "pending", "priority": "low" }
                    ]
                })),
                &ctx,
            )
            .await;
        TodoWriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "todos": [ { "content": "only", "status": "completed", "priority": "high" } ]
                })),
                &ctx,
            )
            .await;
        let tasks = store.list_tasks(&session_id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "only");
    }

    #[tokio::test]
    async fn errors_without_store() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let result = TodoWriteTool
            .execute(ToolParams::new(serde_json::json!({ "todos": [] })), &ctx)
            .await;
        assert!(matches!(result, ToolResult::Error(_)));
    }
}
