//! Modal layer — overlay rendering for dialogs, questions, permissions,
//! text inputs, and diff viewers.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use tui_textarea::TextArea;

use crate::tui::dialog::{Dialog, DialogKind};
use crate::tui::layout::{centered_rect, modal_rect};
use crate::tui::util::textarea_display_col;
use crate::tui::{
    PendingPermission, PendingQuestion, PendingTextInput, SessionView,
};

pub struct ModalLayer<'a> {
    view: &'a SessionView,
}

impl<'a> ModalLayer<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    /// Render the modal overlay on top of the full frame area.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let view = self.view;
        if !modal_active(view) && !view.diff_visible {
            return;
        }

        frame.render_widget(Block::default().style(view.theme.overlay_style()), area);

        if let Some(dialog) = &view.ui.dialog {
            if dialog.kind == DialogKind::SlashHelp {
                view.render.dialog_area.set(None);
                return;
            }
            let dialog_area = dialog_rect(dialog, area);
            view.render.dialog_area.set(Some(dialog_area));
            frame.render_widget(Clear, dialog_area);
            render_dialog_panel(view, frame, dialog_area, dialog);
            return;
        }
        if let Some(question) = &view.ui.pending_question {
            let rect = centered_rect(72, 60, area);
            view.render.dialog_area.set(Some(rect));
            frame.render_widget(Clear, rect);
            frame.render_widget(question_widget(view, question), rect);
            return;
        }
        if let Some(permission) = &view.ui.pending_permission {
            let rect = centered_rect(60, 32, area);
            view.render.dialog_area.set(Some(rect));
            frame.render_widget(Clear, rect);
            frame.render_widget(permission_widget(view, permission), rect);
            return;
        }
        if let Some(input) = &view.ui.pending_text_input {
            let rect = centered_rect(64, 28, area);
            view.render.dialog_area.set(Some(rect));
            frame.render_widget(Clear, rect);
            let inner = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(3)])
                .margin(1)
                .split(rect);
            frame.render_widget(
                Paragraph::new(input.description.as_str())
                    .style(view.theme.muted_style())
                    .wrap(Wrap { trim: false }),
                inner[0],
            );
            let textarea = text_input_widget(view, input);
            frame.render_widget(&textarea, inner[1]);
            let (row, col) = input.editor.cursor();
            let display_col = textarea_display_col(input.editor.lines(), row, col);
            frame.set_cursor_position((
                inner[1].x + 1 + display_col as u16,
                inner[1].y + 1 + row as u16,
            ));
            return;
        }
        if view.diff_visible
            && let Some((title, before, after)) = &view.last_diff
        {
            let rect = centered_rect(80, 70, area);
            view.render.dialog_area.set(Some(rect));
            frame.render_widget(Clear, rect);
            let widget = Paragraph::new(crate::tui::diff::render_diff(before, after, &view.theme))
                .style(view.theme.dialog_style())
                .block(
                    Block::default()
                        .title(format!(" diff: {} ", title))
                        .title_alignment(ratatui::layout::Alignment::Center)
                        .title_style(view.theme.title_style())
                        .borders(Borders::ALL)
                        .border_style(view.theme.dialog_border_style()),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(widget, rect);
        }
    }
}

fn modal_active(view: &SessionView) -> bool {
    view.ui
        .dialog
        .as_ref()
        .is_some_and(|d| d.kind != DialogKind::SlashHelp)
        || view.ui.pending_question.is_some()
        || view.ui.pending_permission.is_some()
        || view.ui.pending_text_input.is_some()
}

fn dialog_rect(dialog: &Dialog, area: Rect) -> Rect {
    match dialog.kind {
        DialogKind::Provider
        | DialogKind::ProviderModel
        | DialogKind::ConnectProtocol
        | DialogKind::Model => modal_rect(62, 16, 4, area),
        DialogKind::Agent | DialogKind::Task => modal_rect(60, 15, 4, area),
        DialogKind::SlashHelp => modal_rect(56, 12, 3, area),
        DialogKind::Thinking | DialogKind::ReasoningEffort | DialogKind::Theme => modal_rect(52, 12, 4, area),
        DialogKind::ModelConfigLoop | DialogKind::ReasoningToggle => modal_rect(60, 14, 4, area),
        DialogKind::Session | DialogKind::SessionDelete => modal_rect(58, 24, 4, area),
    }
}

fn render_dialog_panel(view: &SessionView, frame: &mut Frame, area: Rect, dialog: &Dialog) {
    frame.render_widget(
        Block::default()
            .style(view.theme.dialog_style())
            .borders(Borders::ALL)
            .border_style(view.theme.dialog_border_style()),
        area,
    );

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .margin(1)
        .split(area);

    let header = Paragraph::new(dialog.title())
        .alignment(ratatui::layout::Alignment::Center)
        .style(view.theme.title_style());
    frame.render_widget(header, inner[0]);

    let description = Paragraph::new(dialog.description())
        .alignment(ratatui::layout::Alignment::Center)
        .style(view.theme.muted_style())
        .wrap(Wrap { trim: false });
    frame.render_widget(description, inner[1]);

    let max_visible = (inner[2].height as usize).div_ceil(3).max(1);
    let options = Paragraph::new(dialog.option_lines(&view.theme, max_visible))
        .style(view.theme.dialog_style())
        .wrap(Wrap { trim: false });
    frame.render_widget(options, inner[2]);

    let footer = Paragraph::new(dialog.footer_hint())
        .alignment(ratatui::layout::Alignment::Center)
        .style(view.theme.muted_style());
    frame.render_widget(footer, inner[3]);
}

fn question_widget(view: &SessionView, q: &PendingQuestion) -> Paragraph<'static> {
    let theme = &view.theme;
    let item = q.item();
    let title = if item.header.is_empty() {
        format!(" Question {}/{} ", q.current + 1, q.items.len())
    } else {
        format!(" {} ({}/{}) ", item.header, q.current + 1, q.items.len())
    };

    // Confirm sub-page: review the chosen options (incl. custom text).
    // Only Enter here submits; Esc/← returns to the selection page.
    if q.confirming {
        let mut lines = vec![
            Line::from(Span::styled(item.question.clone(), theme.muted_style())),
            Line::from(""),
            Line::from(Span::styled(
                "已选答案:".to_string(),
                theme.title_style(),
            )),
        ];
        for part in q.answer_parts() {
            lines.push(Line::from(Span::styled(
                format!("  • {part}"),
                theme.dialog_selected_style(),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Enter 提交答案 · Esc/← 返回修改",
            theme.muted_style(),
        )));
        return Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .title_style(theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(theme.dialog_border_style()),
            )
            .style(theme.dialog_style())
            .wrap(Wrap { trim: false });
    }

    let mut lines = vec![
        Line::from(Span::styled(item.question.clone(), theme.muted_style())),
        Line::from(""),
    ];
    for (index, (label, description)) in item.options.iter().enumerate() {
        let selected = index == q.selected && q.typing.is_none();
        let marker = if selected { "› " } else { "  " };
        let check = if item.multiple {
            if q.picked.contains(&index) {
                "[x] "
            } else {
                "[ ] "
            }
        } else {
            ""
        };
        let style = if selected {
            theme.dialog_selected_style().add_modifier(Modifier::BOLD)
        } else {
            theme.dialog_style()
        };
        lines.push(Line::from(Span::styled(
            format!("{}{}{}", marker, check, label),
            style,
        )));
        if !description.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("      {}", description),
                theme.muted_style(),
            )));
        }
    }
    let custom_selected = q.selected == q.custom_index() && q.typing.is_none();
    let custom_marker = if custom_selected { "› " } else { "  " };
    let custom_style = if custom_selected {
        theme.dialog_selected_style().add_modifier(Modifier::BOLD)
    } else {
        theme.dialog_style()
    };
    lines.push(Line::from(Span::styled(
        format!("{}✎ Type your own answer", custom_marker),
        custom_style,
    )));
    if item.multiple {
        let done_selected = q.selected == q.done_index() && q.typing.is_none();
        let done_marker = if done_selected { "› " } else { "  " };
        let done_style = if done_selected {
            theme.dialog_selected_style().add_modifier(Modifier::BOLD)
        } else {
            theme.dialog_style()
        };
        lines.push(Line::from(Span::styled(
            format!("{}✓ Done (确认选择)", done_marker),
            done_style,
        )));
    }
    if let Some(buffer) = &q.typing {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  > {}", buffer),
            theme.dialog_selected_style(),
        )));
    }
    lines.push(Line::from(""));
    let hint = if q.typing.is_some() {
        "Enter 确认输入 · Esc 返回选项"
    } else if item.multiple {
        "↑/↓ 移动 · Enter/Space 勾选 · Done 行确认 · Esc 取消"
    } else {
        "↑/↓ 移动 · Enter 选择 · Esc 取消"
    };
    lines.push(Line::from(Span::styled(hint, theme.muted_style())));

    Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .title_alignment(ratatui::layout::Alignment::Center)
                .title_style(theme.title_style())
                .borders(Borders::ALL)
                .border_style(theme.dialog_border_style()),
        )
        .style(theme.dialog_style())
        .wrap(Wrap { trim: false })
}

fn permission_widget(view: &SessionView, permission: &PendingPermission) -> Paragraph<'static> {
    let theme = &view.theme;
    let allow_style = if permission.allow {
        theme.dialog_selected_style().add_modifier(Modifier::BOLD)
    } else {
        theme.dialog_style()
    };
    let deny_style = if permission.allow {
        theme.dialog_style()
    } else {
        theme.dialog_selected_style().add_modifier(Modifier::BOLD)
    };
    let lines = vec![
        Line::from(Span::styled(
            format!("{}: {}", permission.tool, permission.detail),
            theme.muted_style(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [A] Allow  ", allow_style),
            Span::styled("  [D] Deny  ", deny_style),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "A/Y 允许 · D/N/Esc 拒绝 · ←/→ 切换 · Enter 确认",
            theme.muted_style(),
        )),
    ];
    Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Permission ")
                .title_alignment(ratatui::layout::Alignment::Center)
                .title_style(theme.title_style())
                .borders(Borders::ALL)
                .border_style(theme.dialog_border_style()),
        )
        .style(theme.dialog_style())
        .wrap(Wrap { trim: false })
}

fn text_input_widget(view: &SessionView, input: &PendingTextInput) -> TextArea<'static> {
    let mut textarea = input.editor.clone();
    textarea.set_style(view.theme.dialog_selected_style());
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.set_cursor_style(ratatui::style::Style::default());
    textarea.set_placeholder_text("输入后 Enter 保存 · Esc 取消");
    textarea.set_placeholder_style(view.theme.muted_style());
    textarea.set_block(
        Block::default()
            .title(format!(" {} ", input.title))
            .title_alignment(ratatui::layout::Alignment::Center)
            .title_style(view.theme.title_style())
            .borders(Borders::ALL)
            .border_style(view.theme.dialog_border_style()),
    );
    textarea
}
