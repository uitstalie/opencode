//! Permission requests, question dialogs, text input, and input widget helpers.

use crossterm::event::{self, KeyCode};
use ratatui::{style::Modifier, text::{Line, Span}, widgets::{Block, Borders, Paragraph, Wrap}};
use tui_textarea::TextArea;

use super::util::{char_column_to_byte_index, home_input_hint, normalize_single_line_text, single_line_textarea, textarea_input_from_key_event};
use super::{PendingPermission, PendingQuestion, PendingTextInput, SessionView, ViewMode};

impl SessionView {
    pub(super) fn poll_permission_request(&mut self) -> bool {
        if self.ui.pending_permission.is_some() {
            return false;
        }
        let request = {
            let Some(job) = &self.prompt_job else {
                return false;
            };
            match job.permission_receiver.try_recv() {
                Ok(request) => request,
                Err(_) => return false,
            }
        };
        self.ui.pending_permission = Some(PendingPermission {
            responder: request.responder,
            tool: request.tool,
            detail: request.detail,
            allow: false,
        });
        self.status = "permission: awaiting your decision".to_string();
        true
    }

    pub(super) fn handle_permission_key(&mut self, key: event::KeyEvent) {
        let decision = {
            let Some(permission) = self.ui.pending_permission.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Char('a') | KeyCode::Char('y') => Some(true),
                KeyCode::Char('d') | KeyCode::Char('n') | KeyCode::Esc => Some(false),
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    permission.allow = !permission.allow;
                    None
                }
                KeyCode::Enter => Some(permission.allow),
                _ => None,
            }
        };
        if let Some(allow) = decision {
            if let Some(permission) = self.ui.pending_permission.take() {
                let _ = permission.responder.send(allow);
            }
            self.status = if allow {
                "permission: allowed".to_string()
            } else {
                "permission: denied".to_string()
            };
        }
    }

    pub(super) fn handle_text_input_key(&mut self, key: event::KeyEvent) {
        enum Outcome {
            None,
            Cancel,
            Submit(String, fn(&mut SessionView, &str)),
        }

        let outcome = {
            let Some(input) = self.ui.pending_text_input.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Esc => Outcome::Cancel,
                KeyCode::Enter => Outcome::Submit(input.value.clone(), input.submit),
                _ => {
                    if input.editor.input(textarea_input_from_key_event(key)) {
                        input.sync_value();
                    }
                    Outcome::None
                }
            }
        };

        match outcome {
            Outcome::None => {}
            Outcome::Cancel => {
                self.ui.pending_text_input = None;
                self.note("input cancelled".to_string());
            }
            Outcome::Submit(value, submit) => {
                self.ui.pending_text_input = None;
                submit(self, &value);
            }
        }
    }

    pub(super) fn insert_pending_text_input(&mut self, text: &str) {
        let Some(input) = self.ui.pending_text_input.as_mut() else {
            return;
        };
        input.editor.insert_str(normalize_single_line_text(text));
        input.sync_value();
    }

    pub(super) fn insert_input_text(&mut self, text: &str) {
        self.input_editor.insert_str(normalize_single_line_text(text));
        self.sync_input_state();
        self.sync_slash_help();
    }

    pub(super) fn clear_input(&mut self) {
        self.input_editor = single_line_textarea("", false);
        self.sync_input_state();
    }

    pub(super) fn set_input_text(&mut self, value: &str) {
        self.input_editor = single_line_textarea(value, false);
        self.sync_input_state();
    }

    pub(super) fn sync_input_state(&mut self) {
        self.input = self.input_editor.lines().join("\n");
        self.cursor_index = char_column_to_byte_index(&self.input, self.input_editor.cursor().1);
    }

    pub(super) fn permission_widget(&self, permission: &PendingPermission) -> Paragraph<'static> {
        let theme = &self.theme;
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

    pub(super) fn text_input_widget(&self, input: &PendingTextInput) -> TextArea<'static> {
        let mut textarea = input.editor.clone();
        textarea.set_style(self.theme.dialog_selected_style());
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text("输入后 Enter 保存 · Esc 取消");
        textarea.set_placeholder_style(self.theme.muted_style());
        textarea.set_block(
            Block::default()
                .title(format!(" {} ", input.title))
                .title_alignment(ratatui::layout::Alignment::Center)
                .title_style(self.theme.title_style())
                .borders(Borders::ALL)
                .border_style(self.theme.dialog_border_style()),
        );
        textarea
    }

    pub(super) fn input_widget(&self, title: &str) -> TextArea<'static> {
        let mut textarea = self.input_editor.clone();
        textarea.set_style(self.theme.input_style());
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_placeholder_text(if self.view_mode == ViewMode::Home {
            home_input_hint()
        } else {
            "输入消息后 Enter 发送"
        });
        textarea.set_placeholder_style(self.theme.muted_style());
        textarea.set_block(
            Block::default()
                .title(format!(" {} ", title))
                .title_style(self.theme.title_style())
                .borders(Borders::ALL)
                .border_style(self.theme.input_border_style(self.ai_running)),
        );
        textarea
    }

    pub(super) fn poll_ask_request(&mut self) -> bool {
        if self.ui.pending_question.is_some() {
            return false;
        }
        let request = {
            let Some(job) = &self.prompt_job else {
                return false;
            };
            match job.ask_receiver.try_recv() {
                Ok(request) => request,
                Err(_) => return false,
            }
        };
        self.ui.pending_question = PendingQuestion::from_request(request);
        if self.ui.pending_question.is_some() {
            self.status = "question: awaiting your answer".to_string();
            return true;
        }
        false
    }

    pub(super) fn handle_question_key(&mut self, key: event::KeyEvent) {
        enum Outcome {
            None,
            Cancel,
            Done(Vec<String>),
        }
        let outcome = {
            let Some(q) = self.ui.pending_question.as_mut() else {
                return;
            };
            if q.typing.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        q.typing = None;
                        Outcome::None
                    }
                    KeyCode::Enter => match q.commit_custom() {
                        Some(answers) => Outcome::Done(answers),
                        None => Outcome::None,
                    },
                    KeyCode::Backspace => {
                        if let Some(buffer) = q.typing.as_mut() {
                            buffer.pop();
                        }
                        Outcome::None
                    }
                    KeyCode::Char(ch) => {
                        if let Some(buffer) = q.typing.as_mut() {
                            buffer.push(ch);
                        }
                        Outcome::None
                    }
                    _ => Outcome::None,
                }
            } else {
                match key.code {
                    KeyCode::Esc => Outcome::Cancel,
                    KeyCode::Up => {
                        q.previous();
                        Outcome::None
                    }
                    KeyCode::Down => {
                        q.next();
                        Outcome::None
                    }
                    KeyCode::Char(' ') => {
                        q.toggle_pick();
                        Outcome::None
                    }
                    KeyCode::Enter => match q.confirm() {
                        Some(answers) => Outcome::Done(answers),
                        None => Outcome::None,
                    },
                    _ => Outcome::None,
                }
            }
        };
        match outcome {
            Outcome::None => {}
            Outcome::Cancel => {
                if let Some(q) = self.ui.pending_question.take() {
                    let _ = q.responder.send(vec!["(cancelled)".to_string()]);
                }
                self.note("question cancelled".to_string());
            }
            Outcome::Done(answers) => {
                if let Some(q) = self.ui.pending_question.take() {
                    let _ = q.responder.send(answers);
                }
                self.status = "answer sent".to_string();
            }
        }
    }

    pub(super) fn question_widget(&self, q: &PendingQuestion) -> Paragraph<'static> {
        let theme = &self.theme;
        let item = q.item();
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
        if let Some(buffer) = &q.typing {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  > {}", buffer),
                theme.dialog_selected_style(),
            )));
        }
        lines.push(Line::from(""));
        let hint = if item.multiple {
            "↑/↓ 选择 · Space 多选 · Enter 确认 · Esc 取消"
        } else {
            "↑/↓ 选择 · Enter 确认 · Esc 取消"
        };
        lines.push(Line::from(Span::styled(hint, theme.muted_style())));

        let title = if item.header.is_empty() {
            format!(" Question {}/{} ", q.current + 1, q.items.len())
        } else {
            format!(" {} ({}/{}) ", item.header, q.current + 1, q.items.len())
        };
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
}
