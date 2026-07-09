//! Permission requests, question dialogs, text input, and input widget helpers.

use crossterm::event::{self, KeyCode};

use super::util::{char_column_to_byte_index, normalize_single_line_text, single_line_textarea, textarea_input_from_key_event};
use super::{PendingPermission, PendingQuestion, SessionView};

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
}
