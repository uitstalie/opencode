//! Session display model: `DisplayMessage` and its line rendering.
//!
//! The runtime [`Theme`] lives in [`super::theme`]; it is re-exported here
//! so existing `render::Theme` imports keep working.

use ratatui::{
    style::Modifier,
    text::{Line, Span},
};

pub(super) use super::theme::Theme;

pub struct DisplayMessage {
    pub role: String,
    pub content: String,
    pub collapsed: bool,
    pub meta: Option<String>,
}

impl DisplayMessage {
    pub fn role(&self) -> &str {
        &self.role
    }

    pub fn content(&self) -> &str {
        &self.content
    }
}

impl DisplayMessage {
    pub fn new(role: &str, content: &str) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
            collapsed: false,
            meta: None,
        }
    }

    pub fn new_collapsed(role: &str, content: &str) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
            collapsed: true,
            meta: None,
        }
    }

    pub fn new_with_meta(role: &str, content: &str, meta: String) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
            collapsed: false,
            meta: Some(meta),
        }
    }
}

pub(super) fn display_message_lines(message: &DisplayMessage, theme: &Theme) -> Vec<Line<'static>> {
    let role = message.role();
    // Strip terminal control sequences at the pipeline entry: tool output and
    // provider errors may carry ANSI/CR bytes that would otherwise be
    // interpreted by the real terminal and corrupt the frame.
    let content = super::util::strip_terminal_controls(message.content());
    let meta = message
        .meta
        .as_deref()
        .map(|m| super::util::strip_terminal_controls(m));
    let role_style = match role {
        "user" => theme.user_style(),
        "assistant" => theme.assistant_style(),
        "thinking" | "thought" => theme.thinking_style(),
        "tool" => theme.tool_style(),
        _ => theme.system_style(),
    };
    let header_spans = if let Some(ref meta) = meta {
        vec![
            Span::styled(role.to_string(), role_style.add_modifier(Modifier::BOLD)),
            Span::styled(format!(": {meta}"), theme.muted_style()),
        ]
    } else {
        vec![Span::styled(
            role.to_string(),
            role_style.add_modifier(Modifier::BOLD),
        )]
    };
    let mut lines = vec![Line::from(header_spans)];

    if message.collapsed {
        let first_line = content.lines().next().unwrap_or("");
        lines.push(Line::from(vec![
            Span::from(format!("▶ {}", first_line)),
            Span::styled("  [click to expand]", theme.muted_style()),
        ]));
        lines.push(Line::from(""));
        return lines;
    }

    if role == "assistant" {
        lines.extend(super::markdown::render_markdown(&content, theme));
    } else {
        lines.extend(
            content
                .lines()
                .map(|line| Line::from(line.to_string())),
        );
    }
    lines.push(Line::from(""));
    lines
}
