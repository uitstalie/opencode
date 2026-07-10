use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

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

#[derive(Clone, Debug)]
pub struct Theme {
    background: Color,
    panel: Color,
    sidebar_bg: Color,
    footer_bg: Color,
    border: Color,
    active_border: Color,
    text: Color,
    muted: Color,
    user: Color,
    assistant: Color,
    success: Color,
    warning: Color,
    thinking: Color,
    tool: Color,
    dialog: Color,
    dialog_selected: Color,
    overlay: Color,
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            background: Color::Rgb(0, 0, 0),
            panel: Color::Rgb(22, 22, 26),
            sidebar_bg: Color::Rgb(13, 13, 16),
            footer_bg: Color::Rgb(10, 10, 13),
            border: Color::Rgb(42, 42, 48),
            active_border: Color::Rgb(85, 85, 95),
            text: Color::Rgb(215, 215, 220),
            muted: Color::Rgb(130, 130, 140),
            user: Color::Rgb(204, 174, 100),
            assistant: Color::Rgb(110, 185, 165),
            success: Color::Rgb(95, 170, 105),
            warning: Color::Rgb(205, 175, 85),
            thinking: Color::Rgb(155, 125, 200),
            tool: Color::Rgb(120, 155, 195),
            dialog: Color::Rgb(28, 28, 34),
            dialog_selected: Color::Rgb(160, 180, 205),
            overlay: Color::Rgb(0, 0, 0),
        }
    }

    pub fn panel_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }

    pub fn input_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.panel)
    }

    pub fn sidebar_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.sidebar_bg)
    }

    pub fn footer_style(&self) -> Style {
        Style::default().fg(self.muted).bg(self.footer_bg)
    }

    pub fn title_style(&self) -> Style {
        Style::default().fg(self.text).add_modifier(Modifier::BOLD)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn input_border_style(&self, ai_running: bool) -> Style {
        Style::default().fg(if ai_running {
            self.warning
        } else {
            self.active_border
        })
    }

    pub fn brand_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn user_style(&self) -> Style {
        Style::default().fg(self.user)
    }

    pub fn assistant_style(&self) -> Style {
        Style::default().fg(self.assistant)
    }

    pub fn thinking_style(&self) -> Style {
        Style::default().fg(self.thinking)
    }

    pub fn tool_style(&self) -> Style {
        Style::default().fg(self.tool)
    }

    pub fn system_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn dialog_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.dialog)
    }

    pub fn dialog_selected_style(&self) -> Style {
        Style::default().fg(self.dialog_selected).bg(self.dialog)
    }

    pub fn dialog_border_style(&self) -> Style {
        Style::default().fg(self.active_border).bg(self.dialog)
    }

    pub fn overlay_style(&self) -> Style {
        Style::default().bg(self.overlay)
    }

    pub fn running_style(&self, ai_running: bool) -> Style {
        Style::default().fg(if ai_running {
            self.warning
        } else {
            self.success
        })
    }

    pub fn diff_insert_style(&self) -> Style {
        Style::default().fg(self.success)
    }

    pub fn diff_delete_style(&self) -> Style {
        Style::default().fg(Color::Rgb(200, 110, 110))
    }

    pub fn sidebar_dir_style(&self) -> Style {
        Style::default()
            .fg(self.active_border)
            .add_modifier(Modifier::BOLD)
    }

    pub fn sidebar_file_style(&self) -> Style {
        Style::default().fg(self.text)
    }
}

pub(super) fn display_message_lines(message: &DisplayMessage, theme: &Theme) -> Vec<Line<'static>> {
    let role = message.role();
    let role_style = match role {
        "user" => theme.user_style(),
        "assistant" => theme.assistant_style(),
        "thinking" | "thought" => theme.thinking_style(),
        "tool" => theme.tool_style(),
        _ => theme.system_style(),
    };
    let header_spans = if let Some(ref meta) = message.meta {
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
        let first_line = message.content().lines().next().unwrap_or("");
        lines.push(Line::from(vec![
            Span::from(format!("▶ {}", first_line)),
            Span::styled("  [click to expand]", theme.muted_style()),
        ]));
        lines.push(Line::from(""));
        return lines;
    }

    if role == "assistant" {
        lines.extend(super::markdown::render_markdown(message.content(), theme));
    } else {
        lines.extend(
            message
                .content()
                .lines()
                .map(|line| Line::from(line.to_string())),
        );
    }
    lines.push(Line::from(""));
    lines
}
