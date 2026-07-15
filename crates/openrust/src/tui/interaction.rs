use std::io;

use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::CrosstermBackend};
use ratatui::{style::Style, style::Modifier, text::{Line, Span}};

use super::{SessionRenderLine, SessionView, ThinkingMode, ToolState, display_message_lines};

impl SessionView {
    pub(super) fn handle_global_copy_key(&mut self, key: event::KeyEvent) -> anyhow::Result<bool> {
        if key.code != KeyCode::Char('c') || !key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(false);
        }

        if key.modifiers.contains(KeyModifiers::SHIFT)
            && self.copy_current_input()? {
                return Ok(true);
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
                self.render.mouse_down_row = None;
                self.render.mouse_dragging = false;
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.session_area_contains(mouse.row) {
                    self.render.mouse_dragging = true;
                    self.update_session_selection(mouse.row, false)?;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.render.mouse_dragging {
                    self.update_session_selection(mouse.row, true)?;
                    self.render.mouse_dragging = false;
                    self.render.mouse_down_row = None;
                    return Ok(());
                }
                if self.handle_dialog_mouse(mouse.column, mouse.row)? {
                    return Ok(());
                }
                self.handle_session_click(mouse.column, mouse.row, terminal)?;
                self.render.mouse_down_row = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn copy_current_input(&mut self) -> anyhow::Result<bool> {
        let text = if let Some(input) = &self.ui.pending_text_input {
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
        let Some((start, end)) = self.render.selection else {
            return Ok(false);
        };
        let lines = self.render.all_lines.borrow();
        if lines.is_empty() {
            return Ok(false);
        }
        let (from, to) = if start <= end { (start, end) } else { (end, start) };
        let from = from.min(lines.len() - 1);
        let to = to.min(lines.len() - 1);
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
        let Some(dialog) = &self.ui.dialog else {
            return Ok(false);
        };
        let Some(area) = self.render.dialog_area.get() else {
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
            if let Some(dialog) = &mut self.ui.dialog {
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
        self.render.mouse_down_row = Some(index);
        self.render.selection = Some((index, index));
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
            self.render.selection = None;
            return Ok(());
        };

        if self.render.mouse_dragging {
            return Ok(());
        }

        if self.is_tool_row(index) {
            self.toggle_tool_at_index(index);
            self.render.selection = Some((index, index));
            self.render_terminal(terminal)?;
            return Ok(());
        }

        let _ = column;
        self.render.selection = Some((index, index));
        self.render_terminal(terminal)?;
        Ok(())
    }

    fn update_session_selection(&mut self, row: u16, finalize: bool) -> anyhow::Result<()> {
        let Some(index) = self.session_row_index_at(row) else {
            return Ok(());
        };
        let anchor = self.render.mouse_down_row.unwrap_or(index);
        self.render.selection = Some((anchor, index));
        if finalize && self.render.mouse_down_row.is_none() {
            self.render.selection = Some((index, index));
        }
        Ok(())
    }

    fn session_area_contains(&self, row: u16) -> bool {
        let top = self.render.area_top.get();
        let height = self.render.area_height.get();
        row >= top && row < top.saturating_add(height)
    }

    fn session_row_index_at(&self, row: u16) -> Option<usize> {
        let top = self.render.area_top.get().saturating_add(1);
        let height = self.render.area_height.get().saturating_sub(2);
        if row < top || row >= top.saturating_add(height) {
            return None;
        }
        let visible_index = row.saturating_sub(top) as usize;
        let absolute_index = visible_index + self.render.scroll_offset.get();
        let all_lines = self.render.all_lines.borrow();
        (absolute_index < all_lines.len()).then_some(absolute_index)
    }

    fn is_tool_row(&self, index: usize) -> bool {
        self.render.all_lines
            .borrow()
            .get(index)
            .and_then(|row| row.tool_message_index)
            .is_some()
    }

    fn toggle_tool_at_index(&mut self, index: usize) {
        let tool_message_index = {
            let rows = self.render.all_lines.borrow();
            rows.get(index).and_then(|row| row.tool_message_index)
        };
        let Some(tool_message_index) = tool_message_index else {
            return;
        };
        if let Some(message) = self.display.get_mut(tool_message_index) {
            message.collapsed = !message.collapsed;
        }
    }

    fn session_render_rows(&self, _region_height: usize, region_width: usize) -> Vec<SessionRenderLine> {
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
                    message_index,
                });
            }
        }

        let mut live_idx = self.display.len();

        if self.ai_running && !self.thinking_preview.trim().is_empty()
            && self.thinking_mode == ThinkingMode::Show {
                let header_spans = if let Some(start) = self.thinking_start {
                    vec![
                        Span::styled(format!("{} ", spinner_frame()), self.theme.thinking_style()),
                        Span::styled("thinking", self.theme.thinking_style().add_modifier(Modifier::BOLD)),
                        Span::styled(format!(" · {}", format_elapsed(start)), self.theme.muted_style()),
                    ]
                } else {
                    let dur = self.thought_duration
                        .map(format_duration)
                        .unwrap_or_default();
                    vec![
                        Span::styled("thought", self.theme.thinking_style().add_modifier(Modifier::BOLD)),
                        Span::styled(format!(": {dur}"), self.theme.muted_style()),
                    ]
                };
                all_rows.push(SessionRenderLine {
                    line: Line::from(header_spans),
                    text: "thinking".to_string(),
                    tool_message_index: None,
                    message_index: live_idx,
                });
                for line in self.thinking_preview.lines() {
                    let rendered = super::latex::latex_to_unicode(line);
                    let line = Line::from(Span::styled(rendered, self.theme.thinking_style()));
                    all_rows.push(SessionRenderLine {
                        text: Self::flatten_line(&line),
                        line,
                        tool_message_index: None,
                        message_index: live_idx,
                    });
                }
                live_idx += 1;
            }

        if self.ai_running && !self.assistant_preview.trim().is_empty() {
            all_rows.push(SessionRenderLine {
                line: Line::from(vec![Span::styled(
                    "assistant".to_string(),
                    self.theme.assistant_style().add_modifier(Modifier::BOLD),
                )]),
                text: "assistant".to_string(),
                tool_message_index: None,
                message_index: live_idx,
            });
            for line in self.assistant_preview.lines() {
                let rendered = super::latex::latex_to_unicode(line);
                let line = Line::from(Span::styled(rendered, self.theme.assistant_style()));
                all_rows.push(SessionRenderLine {
                    text: Self::flatten_line(&line),
                    line,
                    tool_message_index: None,
                    message_index: live_idx,
                });
            }
            live_idx += 1;
        }

        // Live tool-call cards: created/running, before results land.
        for (tool_idx, tool) in self.pending_tool_calls.iter().enumerate() {
            let frame = spinner_frame();
            let label = match tool.state {
                ToolState::Created => "created",
                ToolState::Running => "running",
            };
            let elapsed = format_elapsed(tool.started_at);
            let tool_msg_idx = live_idx + tool_idx;
            all_rows.push(SessionRenderLine {
                line: Line::from(Span::styled(
                    "tool".to_string(),
                    self.theme.tool_style().add_modifier(Modifier::BOLD),
                )),
                text: "tool".to_string(),
                tool_message_index: None,
                message_index: tool_msg_idx,
            });
            let mut spans = vec![
                Span::styled(format!("{frame} "), self.theme.tool_style()),
                Span::styled(tool.name.clone(), self.theme.tool_style()),
            ];
            if !tool.input.is_empty() {
                let input = if tool.input.len() > 120 {
                    format!("  {}", &tool.input[..117])
                } else {
                    format!("  {}", tool.input)
                };
                spans.push(Span::styled(input, self.theme.muted_style()));
            }
            spans.push(Span::styled(format!("  [{label} · {elapsed}]"), self.theme.muted_style()));
            let text = spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>().join("");
            all_rows.push(SessionRenderLine {
                line: Line::from(spans),
                text,
                tool_message_index: None,
                message_index: tool_msg_idx,
            });
            all_rows.push(SessionRenderLine {
                line: Line::from(""),
                text: String::new(),
                tool_message_index: None,
                message_index: tool_msg_idx,
            });
        }

        if all_rows.is_empty() {
            all_rows.push(SessionRenderLine {
                line: Line::from(Span::styled(
                    "No messages yet. Type in the input window and press Enter.".to_string(),
                    self.theme.muted_style(),
                )),
                text: "No messages yet. Type in the input window and press Enter.".to_string(),
                tool_message_index: None,
                message_index: 0,
            });
        }

        let content_width = region_width.saturating_sub(2).max(1);
        let all_rows: Vec<SessionRenderLine> = all_rows
            .into_iter()
            .flat_map(|row| {
                let msg_idx = row.message_index;
                let tool_idx = row.tool_message_index;
                Self::wrap_line_by_width(&row.line, content_width)
                    .into_iter()
                    .map(move |line| {
                        let text = Self::flatten_line(&line);
                        SessionRenderLine {
                            line,
                            text,
                            tool_message_index: tool_idx,
                            message_index: msg_idx,
                        }
                    })
            })
            .collect();

        all_rows
    }

    pub(super) fn session_render_lines_for_area(&self, region_height: usize, region_width: usize) {
        let all_rows = self.session_render_rows(region_height, region_width);
        let visible_height = region_height.max(1);

        let max_scroll = all_rows.len().saturating_sub(visible_height);
        let scroll = self.session_scroll.min(max_scroll);
        let start = all_rows.len().saturating_sub(visible_height + scroll);

        self.render.scroll_offset.set(start);
        *self.render.all_lines.borrow_mut() = all_rows;

        let visible: Vec<_> = self
            .render
            .all_lines
            .borrow()
            .iter()
            .skip(start)
            .take(visible_height)
            .cloned()
            .collect();
        *self.render.lines.borrow_mut() = visible;
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

        // Detect block-quote prefix ("> ", "> > ", etc.) so we can
        // re-insert it on every wrapped continuation line.
        let quote_prefix: Option<(String, Style)> = line
            .spans
            .first()
            .filter(|s| s.content.starts_with('>'))
            .map(|s| (s.content.to_string(), s.style));

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
                    // Re-insert block-quote prefix on continuation lines.
                    if let Some((ref prefix, style)) = quote_prefix {
                        let pw = unicode_width::UnicodeWidthStr::width(prefix.as_str());
                        current_spans.push(Span::styled(prefix.clone(), style));
                        current_width += pw;
                    }
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

fn spinner_frame() -> char {
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    FRAMES[(ms / 100) as usize % FRAMES.len()]
}

pub(super) fn format_elapsed(start: std::time::Instant) -> String {
    format_duration(start.elapsed())
}

pub(super) fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 1.0 {
        format!("{}ms", d.as_millis())
    } else if secs < 10.0 {
        format!("{:.1}s", secs)
    } else if secs < 60.0 {
        format!("{:.0}s", secs)
    } else {
        let m = (secs / 60.0) as u64;
        let s = (secs % 60.0) as u64;
        format!("{m}m {s}s")
    }
}
