use std::io;
use std::io::Write;

use crossterm::{cursor, execute, terminal};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use super::render::{self, centered_rect, main_layout};
use super::SessionView;

impl SessionView {
    pub(super) fn render(&self, stdout: &mut io::Stdout, status: Option<&str>) -> anyhow::Result<()> {
        if self.interactive {
            execute!(
                stdout,
                terminal::Clear(terminal::ClearType::All),
                cursor::MoveTo(0, 0)
            )?;
        }
        writeln!(stdout, "OpenRust TUI")?;
        writeln!(stdout, "provider: {}", self.provider_name)?;
        writeln!(stdout, "model: {}", self.model)?;
        writeln!(stdout)?;
        if let Some(status) = status {
            writeln!(stdout, "status: {}", status)?;
        }
        writeln!(stdout)?;
        if self.interactive {
            writeln!(stdout, "input: {}", self.input_editor.lines().join("\n"))?;
            writeln!(stdout)?;
        }
        writeln!(stdout, "history:")?;
        for msg in self.messages.iter().rev().take(12).rev() {
            writeln!(stdout, "- {}: {}", msg.role, msg.content)?;
        }
        stdout.flush()?;
        Ok(())
    }

    pub(super) fn render_terminal(
        &self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        terminal.draw(|frame| self.render_frame(frame))?;
        // Selection modals (dialog/question/permission) are list pickers — no
        // text cursor needed. Hide it so it doesn't linger at the last input
        // position underneath the overlay.
        if self.ui.dialog.is_some()
            || self.ui.pending_question.is_some()
            || self.ui.pending_permission.is_some()
        {
            execute!(terminal.backend_mut(), cursor::Hide)?;
        }
        Ok(())
    }

    pub(super) fn render_frame(&self, frame: &mut Frame) {
        let regions = main_layout().split(frame.area());
        if self.view_mode == super::ViewMode::Home {
            self.render_home_frame(frame, regions.status);
            return;
        }

        self.render_session_frame(frame, regions);
    }

    /// Pin the hardware cursor to the input textarea so it stays inside the
    /// input box instead of drifting into the session output while the AI is
    /// streaming. Skipped when any modal is active (the modal layer handles
    /// its own cursor, or the cursor is hidden for list-picker modals).
    fn place_input_cursor(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        if self.ui.dialog.is_some()
            || self.ui.pending_question.is_some()
            || self.ui.pending_permission.is_some()
            || self.ui.pending_text_input.is_some()
        {
            return;
        }
        let (row, col) = self.input_editor.cursor();
        let display_col = super::util::textarea_display_col(self.input_editor.lines(), row, col);
        frame.set_cursor_position((area.x + 1 + display_col as u16, area.y + 1 + row as u16));
    }

    pub(super) fn render_session_frame(&self, frame: &mut Frame, regions: render::LayoutRegions) {
        let session_area = if self.sidebar_visible && self.sidebar.is_some() {
            let (side, main) = render::split_sidebar(regions.session);
            if let Some(tree) = &self.sidebar {
                let sidebar = Paragraph::new(tree.lines(&self.theme))
                    .style(self.theme.panel_style())
                    .block(
                        Block::default()
                            .title(" Files ")
                            .title_style(self.theme.title_style())
                            .borders(Borders::ALL)
                            .border_style(self.theme.border_style()),
                    )
                    .wrap(Wrap { trim: false });
                frame.render_widget(sidebar, side);
            }
            main
        } else {
            regions.session
        };
        self.session_render_lines_for_area(session_area.height as usize, session_area.width as usize);
        let rows = self.render.lines.borrow();
        let lines: Vec<Line<'static>> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                if self.render.selection.is_some_and(|(start, end)| {
                    let (from, to) = if start <= end { (start, end) } else { (end, start) };
                    index >= from && index <= to
                }) {
                    Line::from(vec![Span::styled(
                        row.text.clone(),
                        self.theme.dialog_selected_style().add_modifier(Modifier::BOLD),
                    )])
                } else {
                    row.line.clone()
                }
            })
            .collect();
        self.render.area_top.set(session_area.y);
        self.render.area_height.set(session_area.height);
        let session = Paragraph::new(lines)
            .style(self.theme.panel_style())
            .block(
                Block::default()
                    .title(" Session ")
                    .title_style(self.theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(session, session_area);

        let input = self.input_widget("Input");
        frame.render_widget(&input, regions.input);
        self.place_input_cursor(frame, regions.input);

        let footer = Paragraph::new(self.status_line()).style(self.theme.footer_style());
        frame.render_widget(footer, regions.status);

        if let Some(toast) = &self.ui.toast {
            let toast_area = render::toast_rect(frame.area());
            let widget = Paragraph::new(toast.as_str())
                .style(self.theme.system_style())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(self.theme.border_style())
                        .style(self.theme.panel_style()),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(Clear, toast_area);
            frame.render_widget(widget, toast_area);
        }

        self.render_modal_layer(frame, frame.area());
    }

    pub(super) fn render_home_frame(&self, frame: &mut Frame, status_area: ratatui::layout::Rect) {
        let outer = centered_rect(76, 72, frame.area());

        let top_padding = outer.height.saturating_sub(15) / 2;
        let sections = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(top_padding),
                ratatui::layout::Constraint::Length(4),
                ratatui::layout::Constraint::Length(3),
                ratatui::layout::Constraint::Length(2),
                ratatui::layout::Constraint::Length(4),
                ratatui::layout::Constraint::Min(0),
            ])
            .split(outer);

        let header = Paragraph::new(vec![
            Line::from(Span::styled(
                "OpenRust",
                self.theme.brand_style().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "AI coding agent · Rust native runtime",
                self.theme.muted_style(),
            )),
            Line::from(Span::styled(
                format!(
                    "model {}/{} · agent {}",
                    self.provider_name,
                    self.model,
                    self.current_session_agent()
                        .unwrap_or_else(|| "default".to_string())
                ),
                self.theme.muted_style(),
            )),
        ])
        .alignment(ratatui::layout::Alignment::Center)
        .style(self.theme.panel_style());
        frame.render_widget(header, sections[1]);

        let prompt = self.input_widget("Prompt");
        frame.render_widget(&prompt, sections[2]);
        self.place_input_cursor(frame, sections[2]);

        let hint = Paragraph::new("输入消息后 Enter 开始 · /connect 配置 provider · /models 选择模型 · Esc 退出")
            .style(self.theme.muted_style())
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(hint, sections[3]);

        let status = Paragraph::new(self.home_status_message())
            .style(self.theme.muted_style())
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(status, sections[4]);

        let footer = Paragraph::new(self.status_line()).style(self.theme.footer_style());
        frame.render_widget(footer, status_area);

        if let Some(toast) = &self.ui.toast {
            let toast_area = render::toast_rect(frame.area());
            let widget = Paragraph::new(toast.as_str())
                .style(self.theme.system_style())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(self.theme.border_style())
                        .style(self.theme.panel_style()),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(Clear, toast_area);
            frame.render_widget(widget, toast_area);
        }

        self.render_modal_layer(frame, frame.area());
    }
}
