//! TodoWrite tool — maintain the session todo list as a state machine that guides work.

use crate::core::session::{Task, TaskSummary};
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use serde_json::Value;

pub struct TodoWriteTool;

#[async_trait::async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &'static str {
        "todowrite"
    }
    fn description(&self) -> &'static str {
        "Maintain the session todo list — a state machine that guides multi-step work. Always pass the FULL list (it replaces the previous one). Workflow: create all items as pending → mark the first as in_progress → work on it → mark it completed AND mark the next as in_progress → repeat. Keep AT MOST ONE in_progress at any time."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The full, updated todo list (replaces the previous one)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "content": { "type": "string", "description": "Brief description of the task" },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "cancelled"],
                                "description": "Current status. At most one item may be in_progress."
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

        let in_progress_count = items
            .iter()
            .filter(|item| {
                item.get("status").and_then(|v| v.as_str()) == Some("in_progress")
            })
            .count();
        if in_progress_count > 1 {
            return ToolResult::error(format!(
                "todowrite: at most one item may be in_progress (got {in_progress_count}). \
                 Complete the current one before starting the next."
            ));
        }

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

        ToolResult::text(render_todo_list(&tasks))
    }
}

fn now_string() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:09}", now.as_secs(), now.subsec_nanos())
}

fn status_mark(status: &str) -> &'static str {
    match status {
        "completed" => "[x]",
        "in_progress" => "[~]",
        "cancelled" => "[-]",
        _ => "[ ]",
    }
}

/// Render a full todo list with progress header for the tool result.
pub fn render_todo_list(tasks: &[Task]) -> String {
    let total = tasks.len();
    let completed = tasks
        .iter()
        .filter(|t| t.status == "completed")
        .count();
    let cancelled = tasks
        .iter()
        .filter(|t| t.status == "cancelled")
        .count();
    let active = total - completed - cancelled;

    let header = if active == 0 && total > 0 {
        format!("All done — {completed}/{total} completed")
    } else {
        format!("Progress: {completed}/{total} completed")
    };

    let body = tasks
        .iter()
        .map(|task| {
            let mark = status_mark(&task.status);
            let priority = task.priority.as_deref().unwrap_or("");
            format!("{mark} {} ({}, {})", task.title, task.status, priority)
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("{header}\n{body}")
}

/// Compact one-line reminder injected into conversation after each tool batch.
///
/// Format: `<todo-state>Progress: 2/5 · Working on: Write fix · Next: Run tests</todo-state>`
pub fn todo_reminder(tasks: &[TaskSummary]) -> String {
    if tasks.is_empty() {
        return String::new();
    }

    let total = tasks.len();
    let completed = tasks
        .iter()
        .filter(|t| t.status == "completed")
        .count();

    let mut parts = vec![format!("Progress: {completed}/{total}")];

    if let Some(t) = tasks.iter().find(|t| t.status == "in_progress") {
        parts.push(format!("Working on: {}", t.title));
    }
    if let Some(t) = tasks.iter().find(|t| t.status == "pending") {
        parts.push(format!("Next: {}", t.title));
    }

    format!("<todo-state>{}</todo-state>", parts.join(" · "))
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

    #[tokio::test]
    async fn rejects_multiple_in_progress() {
        let (_store, ctx, _session_id) = store_ctx();
        let result = TodoWriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "todos": [
                        { "content": "a", "status": "in_progress", "priority": "high" },
                        { "content": "b", "status": "in_progress", "priority": "high" }
                    ]
                })),
                &ctx,
            )
            .await;
        assert!(matches!(result, ToolResult::Error(_)));
        let err = result.into_text();
        assert!(err.contains("at most one"));
    }

    #[tokio::test]
    async fn result_shows_progress_count() {
        let (_store, ctx, _session_id) = store_ctx();
        let result = TodoWriteTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "todos": [
                        { "content": "done", "status": "completed", "priority": "high" },
                        { "content": "active", "status": "in_progress", "priority": "high" },
                        { "content": "later", "status": "pending", "priority": "low" }
                    ]
                })),
                &ctx,
            )
            .await;
        let text = result.into_text();
        assert!(text.contains("Progress: 1/3"));
    }

    #[test]
    fn todo_reminder_shows_current_and_next() {
        let tasks = vec![
            TaskSummary {
                id: "1".to_string(),
                agent: None,
                title: "done".to_string(),
                status: "completed".to_string(),
                created_at: String::new(),
                updated_at: String::new(),
            },
            TaskSummary {
                id: "2".to_string(),
                agent: None,
                title: "active".to_string(),
                status: "in_progress".to_string(),
                created_at: String::new(),
                updated_at: String::new(),
            },
            TaskSummary {
                id: "3".to_string(),
                agent: None,
                title: "later".to_string(),
                status: "pending".to_string(),
                created_at: String::new(),
                updated_at: String::new(),
            },
        ];
        let reminder = todo_reminder(&tasks);
        assert!(reminder.contains("Progress: 1/3"));
        assert!(reminder.contains("Working on: active"));
        assert!(reminder.contains("Next: later"));
        assert!(reminder.starts_with("<todo-state>"));
    }

    #[test]
    fn todo_reminder_empty_for_no_tasks() {
        assert!(todo_reminder(&[]).is_empty());
    }
}
