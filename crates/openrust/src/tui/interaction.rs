use std::io;

use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend};
use ratatui::{style::Modifier, text::{Line, Span}};

use super::{SessionRenderLine, SessionView, ThinkingMode, display_message_lines};

impl SessionView {
    pub(super) fn handle_global_copy_key(&mut self, key: event::KeyEvent) -> anyhow::Result<bool> {
        if key.code != KeyCode::Char('c') || !key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(false);
        }

        if key.modifiers.contains(KeyModifiers::SHIFT) {
            if self.copy_current_input()? {
                return Ok(true);
            }
        }

        if self.copy_selected_session_text()? {
            return Ok(true);
        }

        if self.copy_current_input()? {
            return Ok(true);
        }

        Ok(false)
    }

    pub(super) fn handle_mouse_event(
        &mut self,
        mouse: MouseEvent,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_session_up(3),
            MouseEventKind::ScrollDown => self.scroll_session_down(3),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.handle_dialog_mouse(mouse.column, mouse.row)? {
                    return Ok(());
                }
                if self.handle_session_mouse_down(mouse.column, mouse.row)? {
                    return Ok(());
                }
                self.mouse_down_row = None;
                self.mouse_dragging = false;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.session_area_contains(mouse.row) {
                    self.mouse_dragging = true;
                    self.update_session_selection(mouse.row, false)?;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.mouse_dragging {
                    self.update_session_selection(mouse.row, true)?;
                    self.mouse_dragging = false;
                    self.mouse_down_row = None;
                    return Ok(());
                }
                if self.handle_dialog_mouse(mouse.column, mouse.row)? {
                    return Ok(());
                }
                self.handle_session_click(mouse.column, mouse.row, terminal)?;
                self.mouse_down_row = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn copy_current_input(&mut self) -> anyhow::Result<bool> {
        let text = if let Some(input) = &self.pending_text_input {
            input.value.clone()
        } else if !self.input.is_empty() {
            self.input.clone()
        } else {
            String::new()
        };

        if text.is_empty() {
            return Ok(false);
        }

        self.copy_to_clipboard(text)?;
        self.note("copied input".to_string());
        Ok(true)
    }

    fn copy_selected_session_text(&mut self) -> anyhow::Result<bool> {
        let Some((start, end)) = self.session_selection else {
            return Ok(false);
        };
        let lines = self.session_render_lines.borrow();
        if lines.is_empty() {
            return Ok(false);
        }
        let (from, to) = if start <= end { (start, end) } else { (end, start) };
        let text = lines[from..=to]
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            return Ok(false);
        }
        drop(lines);
        self.copy_to_clipboard(text)?;
        self.note("copied selection".to_string());
        Ok(true)
    }

    fn copy_to_clipboard(&self, text: String) -> anyhow::Result<()> {
        let mut clipboard = arboard::Clipboard::new()?;
        clipboard.set_text(text)?;
        Ok(())
    }

    fn handle_dialog_mouse(&mut self, column: u16, row: u16) -> anyhow::Result<bool> {
        let Some(dialog) = &self.dialog else {
            return Ok(false);
        };
        let Some(area) = self.dialog_area.get() else {
            return Ok(false);
        };
        if !area.contains((column, row).into()) {
            return Ok(false);
        }
        if row <= area.y || row >= area.y.saturating_add(area.height.saturating_sub(1)) {
            return Ok(false);
        }

        let inner_y = area.y.saturating_add(1);
        let option_top = inner_y.saturating_add(3);
        let relative_row = row.saturating_sub(option_top);
        let max_visible = ((area.height.saturating_sub(6)) as usize / 3).max(1);
        let index = dialog.option_index_at(relative_row as usize, max_visible);
        if let Some(index) = index {
            if let Some(dialog) = &mut self.dialog {
                dialog.set_selected(index);
            }
            self.submit_dialog_selection();
            return Ok(true);
        }
        Ok(false)
    }

    fn handle_session_mouse_down(&mut self, column: u16, row: u16) -> anyhow::Result<bool> {
        if !self.session_area_contains(row) {
            return Ok(false);
        }
        let Some(index) = self.session_row_index_at(row) else {
            return Ok(false);
        };
        self.mouse_down_row = Some(index);
        self.session_selection = Some((index, index));
        if self.is_tool_row(index) {
            let _ = column;
        }
        Ok(true)
    }

    fn handle_session_click(
        &mut self,
        column: u16,
        row: u16,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        let Some(index) = self.session_row_index_at(row) else {
            self.session_selection = None;
            return Ok(());
        };

        if self.mouse_dragging {
            return Ok(());
        }

        if self.is_tool_row(index) {
            self.toggle_tool_at_index(index);
            self.session_selection = Some((index, index));
            self.render_terminal(terminal)?;
            return Ok(());
        }

        let _ = column;
        self.session_selection = Some((index, index));
        self.render_terminal(terminal)?;
        Ok(())
    }

    fn update_session_selection(&mut self, row: u16, finalize: bool) -> anyhow::Result<()> {
        let Some(index) = self.session_row_index_at(row) else {
            return Ok(());
        };
        let anchor = self.mouse_down_row.unwrap_or(index);
        self.session_selection = Some((anchor, index));
        if finalize && self.mouse_down_row.is_none() {
            self.session_selection = Some((index, index));
        }
        Ok(())
    }

    fn session_area_contains(&self, row: u16) -> bool {
        let top = self.session_area_top.get();
        let height = self.session_area_height.get();
        row >= top && row < top.saturating_add(height)
    }

    fn session_row_index_at(&self, row: u16) -> Option<usize> {
        let top = self.session_area_top.get().saturating_add(1);
        let height = self.session_area_height.get().saturating_sub(2);
        if row < top || row >= top.saturating_add(height) {
            return None;
        }
        let index = row.saturating_sub(top) as usize;
        self.session_render_lines.borrow().get(index).map(|_| index)
    }

    fn is_tool_row(&self, index: usize) -> bool {
        self.session_render_lines
            .borrow()
            .get(index)
            .and_then(|row| row.tool_message_index)
            .is_some()
    }

    fn toggle_tool_at_index(&mut self, index: usize) {
        let tool_message_index = {
            let rows = self.session_render_lines.borrow();
            rows.get(index).and_then(|row| row.tool_message_index)
        };
        let Some(tool_message_index) = tool_message_index else {
            return;
        };
        if let Some(message) = self.display.get_mut(tool_message_index) {
            message.collapsed = !message.collapsed;
        }
    }

    fn session_render_rows(&self, region_height: usize, region_width: usize) -> Vec<SessionRenderLine> {
        let visible_height = region_height.saturating_sub(2).max(1);
        let mut all_rows = Vec::new();

        for (message_index, message) in self.display.iter().enumerate() {
            for line in display_message_lines(message, &self.theme) {
                all_rows.push(SessionRenderLine {
                    text: Self::flatten_line(&line),
                    line,
                    tool_message_index: if message.role == "tool" {
                        Some(message_index)
                    } else {
                        None
                    },
                });
            }
        }

        if self.ai_running && !self.thinking_preview.trim().is_empty() {
            if self.thinking_mode == ThinkingMode::Show {
                all_rows.push(SessionRenderLine {
                    line: Line::from(vec![Span::styled(
                        "thinking".to_string(),
                        self.theme.thinking_style().add_modifier(Modifier::BOLD),
                    )]),
                    text: "thinking".to_string(),
                    tool_message_index: None,
                });
                for line in self.thinking_preview.lines() {
                    let line = Line::from(Span::styled(line.to_string(), self.theme.thinking_style()));
                    all_rows.push(SessionRenderLine {
                        text: Self::flatten_line(&line),
                        line,
                        tool_message_index: None,
                    });
                }
            }
        }

        if self.ai_running && !self.assistant_preview.trim().is_empty() {
            all_rows.push(SessionRenderLine {
                line: Line::from(vec![Span::styled(
                    "assistant".to_string(),
                    self.theme.assistant_style().add_modifier(Modifier::BOLD),
                )]),
                text: "assistant".to_string(),
                tool_message_index: None,
            });
            for line in self.assistant_preview.lines() {
                let line = Line::from(Span::styled(line.to_string(), self.theme.assistant_style()));
                all_rows.push(SessionRenderLine {
                    text: Self::flatten_line(&line),
                    line,
                    tool_message_index: None,
                });
            }
        }

        if all_rows.is_empty() {
            all_rows.push(SessionRenderLine {
                line: Line::from(Span::styled(
                    "No messages yet. Type in the input window and press Enter.".to_string(),
                    self.theme.muted_style(),
                )),
                text: "No messages yet. Type in the input window and press Enter.".to_string(),
                tool_message_index: None,
            });
        }

        let content_width = region_width.saturating_sub(2).max(1);
        let all_rows: Vec<SessionRenderLine> = all_rows
            .into_iter()
            .flat_map(|row| {
                Self::wrap_line_by_width(&row.line, content_width)
                    .into_iter()
                    .map(move |line| {
                        let text = Self::flatten_line(&line);
                        SessionRenderLine {
                            line,
                            text,
                            tool_message_index: row.tool_message_index,
                        }
                    })
            })
            .collect();

        let max_scroll = all_rows.len().saturating_sub(visible_height);
        let scroll = self.session_scroll.min(max_scroll);
        let start = all_rows.len().saturating_sub(visible_height + scroll);
        all_rows.into_iter().skip(start).take(visible_height).collect()
    }

    pub(super) fn session_render_lines_for_area(&self, region_height: usize, region_width: usize) {
        let rows = self.session_render_rows(region_height, region_width);
        *self.session_render_lines.borrow_mut() = rows;
    }

    fn flatten_line(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("")
    }

    fn wrap_line_by_width(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
        if width == 0 || line.width() <= width {
            return vec![line.clone()];
        }
        let mut result: Vec<Line<'static>> = Vec::new();
        let mut current_spans: Vec<Span<'static>> = Vec::new();
        let mut current_width = 0usize;
        for span in &line.spans {
            let span_style = span.style;
            let mut buf = String::new();
            for ch in span.content.chars() {
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if current_width + w > width && !buf.is_empty() {
                    current_spans.push(Span::styled(std::mem::take(&mut buf), span_style));
                    result.push(Line::from(std::mem::take(&mut current_spans)));
                    current_width = 0;
                }
                buf.push(ch);
                current_width += w;
            }
            if !buf.is_empty() {
                current_spans.push(Span::styled(buf, span_style));
            }
        }
        if !current_spans.is_empty() {
            result.push(Line::from(current_spans));
        }
        if result.is_empty() {
            vec![line.clone()]
        } else {
            result
        }
    }
}
