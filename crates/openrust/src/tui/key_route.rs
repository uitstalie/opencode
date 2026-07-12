//! Z-order aware key routing.
//!
//! Replaces the monolithic 210-line match in `run_inner` with a
//! structured dispatcher that checks layers from highest to lowest
//! [`Z`] priority. Each layer group is a separate method, making the
//! routing order explicit and easy to modify.

use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

use super::dialog::DialogKind;
use super::input::{is_exit_command, should_exit};
use super::util::{textarea_input_from_key_event};
use super::view::KeyFlow;
use super::{SessionView, ViewMode};

type Term = Terminal<CrosstermBackend<io::Stdout>>;

impl SessionView {
    /// Route a key press through the z-order layer stack.
    ///
    /// Returns [`KeyFlow::Exit`] to break the event loop,
    /// [`KeyFlow::Consumed`] if a layer handled it,
    /// [`KeyFlow::Propagate`] if no layer claimed it.
    pub(super) fn route_key(
        &mut self,
        key: event::KeyEvent,
        terminal: &mut Term,
    ) -> anyhow::Result<KeyFlow> {
        if key.kind != event::KeyEventKind::Press {
            return Ok(KeyFlow::Propagate);
        }

        // ━━ Z::Modal (blocking) — consume ALL keys ━━
        if self.ui.pending_permission.is_some() {
            self.handle_permission_key(key);
            return Ok(KeyFlow::Consumed);
        }
        if self.ui.pending_question.is_some() {
            self.handle_question_key(key);
            return Ok(KeyFlow::Consumed);
        }
        if self.ui.pending_text_input.is_some() {
            self.handle_text_input_key(key);
            return Ok(KeyFlow::Consumed);
        }

        // ━━ Global keys — work below blocking overlay ━━
        if self.handle_global_copy_key(key)? {
            return Ok(KeyFlow::Consumed);
        }
        if should_exit(&key) {
            return Ok(KeyFlow::Exit);
        }

        // ━━ Esc (multi-purpose) ━━
        if key.code == KeyCode::Esc {
            // AI abort takes priority (matches previous behavior)
            if self.ai_running {
                self.abort_current_turn();
                return Ok(KeyFlow::Consumed);
            }
            if self.ui.dialog.is_some() {
                self.ui.dialog = None;
                return Ok(KeyFlow::Consumed);
            }
            return Ok(KeyFlow::Consumed);
        }

        // ━━ Dialog navigation (non-blocking modal) ━━
        if self.ui.dialog.is_some() {
            match key.code {
                KeyCode::Up => {
                    if let Some(d) = &mut self.ui.dialog {
                        d.previous();
                    }
                    return Ok(KeyFlow::Consumed);
                }
                KeyCode::Down => {
                    if let Some(d) = &mut self.ui.dialog {
                        d.next();
                    }
                    return Ok(KeyFlow::Consumed);
                }
                _ => {} // fall through
            }
        }

        // ━━ Enter (spans dialog submission + input) ━━
        if key.code == KeyCode::Enter {
            return self.route_enter(terminal);
        }

        // ━━ Tab (slash completion or agent cycling) ━━
        if key.code == KeyCode::Tab {
            let slash_value = self
                .ui
                .dialog
                .as_ref()
                .filter(|d| d.kind == DialogKind::SlashHelp)
                .and_then(|d| d.selected_value().map(str::to_string));
            if let Some(value) = slash_value {
                self.set_input_text(&value);
                self.sync_slash_help();
            } else {
                self.cycle_agent();
            }
            return Ok(KeyFlow::Consumed);
        }

        // ━━ Z::Base — scroll, input editing, misc ━━
        self.route_base_key(key)
    }

    /// Handle Enter across all modes (dialog submission + prompt dispatch).
    fn route_enter(&mut self, terminal: &mut Term) -> anyhow::Result<KeyFlow> {
        // Dialog submission
        if self.ui.dialog.is_some() {
            let is_slash = matches!(
                self.ui.dialog.as_ref().map(|d| &d.kind),
                Some(DialogKind::SlashHelp)
            );
            self.submit_dialog_selection();
            // Non-slash dialog: don't submit input
            if !is_slash {
                return Ok(KeyFlow::Consumed);
            }
            // Slash dialog: fall through to submit the completed input
        }

        let input = self.input.trim().to_string();
        if input.is_empty() {
            self.clear_input();
            return Ok(KeyFlow::Consumed);
        }
        if is_exit_command(&input) {
            return Ok(KeyFlow::Exit);
        }
        self.clear_input();

        // Home mode: create session before first prompt
        if self.view_mode == ViewMode::Home {
            self.create_session();
        }

        match self.handle_slash_command(&input) {
            super::SlashResult::Handled => {}
            super::SlashResult::Prompt(template) => {
                self.enqueue_or_run_prompt(terminal, template)?;
            }
            super::SlashResult::NotHandled => {
                self.enqueue_or_run_prompt(terminal, input)?;
            }
        }
        Ok(KeyFlow::Consumed)
    }

    /// Scroll, input editing, and miscellaneous base-layer keys.
    fn route_base_key(&mut self, key: event::KeyEvent) -> anyhow::Result<KeyFlow> {
        match key.code {
            // Scroll (session mode only)
            KeyCode::Up if self.view_mode == ViewMode::Session => {
                self.scroll_session_up(3);
                Ok(KeyFlow::Consumed)
            }
            KeyCode::Down if self.view_mode == ViewMode::Session => {
                self.scroll_session_down(3);
                Ok(KeyFlow::Consumed)
            }
            KeyCode::PageUp => {
                self.scroll_session_up(8);
                Ok(KeyFlow::Consumed)
            }
            KeyCode::PageDown => {
                self.scroll_session_down(8);
                Ok(KeyFlow::Consumed)
            }

            // Input editor
            KeyCode::Backspace | KeyCode::Delete => {
                if self.input_editor.input(textarea_input_from_key_event(key)) {
                    self.sync_input_state();
                    self.sync_slash_help();
                }
                Ok(KeyFlow::Consumed)
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                self.input_editor.input(textarea_input_from_key_event(key));
                self.sync_input_state();
                Ok(KeyFlow::Consumed)
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_tool_collapse();
                Ok(KeyFlow::Consumed)
            }
            KeyCode::Char(_) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.input_editor.input(textarea_input_from_key_event(key)) {
                    self.sync_input_state();
                    self.sync_slash_help();
                }
                Ok(KeyFlow::Consumed)
            }

            _ => Ok(KeyFlow::Propagate),
        }
    }
}
