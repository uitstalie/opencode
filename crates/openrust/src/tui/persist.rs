//! Message persistence, diff capture, and tool display preview.

use super::util::read_line_span;
use super::SessionView;

impl SessionView {
    /// Capture the last edit/write as a diff for the `/diff` viewer.
    pub(super) fn capture_diff(&mut self, name: &str, args: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(args) else {
            return;
        };
        let path = value
            .get("filePath")
            .or_else(|| value.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match name {
            "edit" => {
                let before = value
                    .get("oldString")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let after = value
                    .get("newString")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.last_diff = Some((format!("edit {}", path), before, after));
            }
            "write" => {
                let after = value
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.last_diff = Some((format!("write {}", path), String::new(), after));
            }
            _ => {}
        }
    }

    pub(super) fn persist_message(&self, role: &str, content: &str) {
        if let Some(store) = &self.store {
            if let Err(e) = store.append_message_detail(&self.session_id, role, content, None, None, None) {
                tracing::error!(err = %e, "persist_message failed");
            }
        } else {
            tracing::warn!("persist_message skipped: store is None");
        }
    }

    pub(super) fn persist_message_detail(
        &self,
        role: &str,
        content: &str,
        name: Option<String>,
        tool_call_id: Option<String>,
        tool_calls: Option<serde_json::Value>,
    ) {
        if let Some(store) = &self.store {
            if let Err(e) = store.append_message_detail(
                &self.session_id, role, content, name, tool_call_id, tool_calls,
            ) {
                tracing::error!(err = %e, "persist_message_detail failed");
            }
        } else {
            tracing::warn!("persist_message_detail skipped: store is None");
        }
    }

    pub(super) fn tool_display_preview(name: &str, args: &str, result: &str) -> String {
        let args_val = serde_json::from_str::<serde_json::Value>(args).ok();
        let path = args_val
            .as_ref()
            .and_then(|v| {
                v.get("path")
                    .or_else(|| v.get("file_path"))
                    .or_else(|| v.get("filePath"))
                    .or_else(|| v.get("url"))
                    .or_else(|| v.get("pattern"))
                    .or_else(|| v.get("query"))
                    .or_else(|| v.get("message"))
                    .and_then(|v| v.as_str())
            });
        if let Some(p) = path {
            if name == "read" {
                if let Some((start, end)) = read_line_span(result) {
                    return format!("read {} L{}-{}", p, start, end);
                }
            }
            return format!("{} {}", name, p);
        }
        let preview = result.lines().next().unwrap_or("");
        let preview = crate::tool::truncate_str(preview, 60);
        format!("{}: {}", name, preview)
    }

}
