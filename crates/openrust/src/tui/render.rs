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
    pub(super) background: Color,
    pub(super) panel: Color,
    pub(super) sidebar_bg: Color,
    pub(super) footer_bg: Color,
    pub(super) border: Color,
    pub(super) active_border: Color,
    pub(super) text: Color,
    pub(super) muted: Color,
    pub(super) user: Color,
    pub(super) assistant: Color,
    pub(super) success: Color,
    pub(super) warning: Color,
    pub(super) thinking: Color,
    pub(super) tool: Color,
    pub(super) dialog: Color,
    pub(super) dialog_selected: Color,
    pub(super) overlay: Color,
    pub(super) message_bg: Color,
    pub(super) heading: Color,
    pub(super) code: Color,
    pub(super) link: Color,
    pub(super) blockquote: Color,
    pub(super) list_marker: Color,
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
            message_bg: Color::Rgb(24, 24, 30),
            heading: Color::Rgb(157, 124, 216),
            code: Color::Rgb(127, 216, 143),
            link: Color::Rgb(250, 178, 131),
            blockquote: Color::Rgb(229, 192, 123),
            list_marker: Color::Rgb(86, 182, 194),
        }
    }

    /// Construct from individual color values (used by theme loader).
    #[allow(clippy::too_many_arguments)]
    pub fn from_colors(
        background: Color, panel: Color, sidebar_bg: Color, footer_bg: Color,
        border: Color, active_border: Color,
        text: Color, muted: Color, user: Color, assistant: Color,
        success: Color, warning: Color, thinking: Color, tool: Color,
        dialog: Color, dialog_selected: Color, overlay: Color,
        message_bg: Color, heading: Color, code: Color, link: Color,
        blockquote: Color, list_marker: Color,
    ) -> Self {
        Self {
            background, panel, sidebar_bg, footer_bg,
            border, active_border,
            text, muted, user, assistant,
            success, warning, thinking, tool,
            dialog, dialog_selected, overlay,
            message_bg, heading, code, link,
            blockquote, list_marker,
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

    pub fn message_bg_style(&self) -> Style {
        Style::default().bg(self.message_bg)
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

    pub fn heading_style(&self) -> Style {
        Style::default().fg(self.heading).add_modifier(Modifier::BOLD)
    }

    pub fn code_style(&self) -> Style {
        Style::default().fg(self.code)
    }

    pub(super) fn link_color(&self) -> Color {
        self.link
    }

    pub fn blockquote_style(&self) -> Style {
        Style::default().fg(self.blockquote)
    }

    pub fn list_marker_style(&self) -> Style {
        Style::default().fg(self.list_marker)
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
