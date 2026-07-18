//! Status line and home view widget rendering.

use ratatui::{style::Modifier, text::{Line, Span}};

use super::{SessionView, ViewMode};
use crate::core::token;

impl SessionView {
    pub(super) fn home_provider_ready(&self) -> bool {
        self.config
            .get_provider(&self.provider_name)
            .and_then(|provider| provider.api_key)
            .is_some()
    }

    pub(super) fn home_status_message(&self) -> &'static str {
        if self.config.provider.is_empty() {
            return "Setup required: no provider configured. Type /connect to add one.";
        }
        if self.config.model.is_none() {
            return "Setup required: no model selected. Type /models to choose one.";
        }
        if !self.home_provider_ready() {
            return "Setup required: API key missing. Type /connect to configure it.";
        }
        "Home: type a prompt and press Enter. Esc exits."
    }

    pub(super) fn status_line(&self) -> Line<'static> {
        let running = if self.ai_running {
            "AI: running"
        } else {
            "AI: idle"
        };
        let cache_rate = if self.cache.prompt_count <= 1 {
            if self.cache.prompt_count == 0 {
                "cache: n/a".to_string()
            } else {
                "cache: priming".to_string()
            }
        } else {
            match self.cache.rate() {
                Some(pct) => format!("cache: {pct}%"),
                None => "cache: n/a".to_string(),
            }
        };
        let used = if self.cache.total > 0 {
            self.cache.total as u64
        } else {
            token::estimate_messages(&self.messages) as u64
        };
        let window = self.current_context_window();
        let window = window.max(1);
        let used_display = format_number(used);
        let window_display = format_number(window);
        let context = format!(
            "context: {} / {} tokens ({:.0}%)",
            used_display,
            window_display,
            used as f64 / window as f64 * 100.0
        );
        let task_count = if self.view_mode == ViewMode::Home {
            "tasks: 0".to_string()
        } else {
            self.store
                .as_ref()
                .and_then(|store| store.list_tasks(&self.session_id).ok())
                .map(|tasks| {
                    let done = tasks.iter().filter(|t| t.status == "completed").count();
                    format!("tasks: {}/{}", done, tasks.len())
                })
                .unwrap_or_else(|| "tasks: 0".to_string())
        };
        Line::from(vec![
            Span::styled(
                "OpenRust",
                self.theme.brand_style().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  |  "),
            Span::raw(task_count),
            Span::raw("  |  "),
            Span::raw(context),
            Span::raw("  |  "),
            Span::raw(cache_rate),
            Span::raw("  |  "),
            Span::styled(running, self.theme.running_style(self.ai_running)),
            Span::raw("  |  "),
            Span::raw(super::util::strip_terminal_controls(&self.status).into_owned()),
        ])
    }

    /// Info line shown directly below the input box:
    /// model · thinking effort · agent mode.
    pub(super) fn info_line(&self) -> Line<'static> {
        let thinking = self
            .reasoning_effort
            .as_deref()
            .unwrap_or("off");
        Line::from(vec![
            Span::raw(format!("model: {}/{}", self.provider_name, self.model)),
            Span::raw("  |  "),
            Span::raw(format!("thinking: {}", thinking)),
            Span::raw("  |  "),
            Span::raw(format!(
                "agent: {}",
                self.current_session_agent().as_deref().unwrap_or("default")
            )),
        ])
    }

    pub(super) fn default_status_message(&self) -> String {
        if self.ui.pending_permission.is_some()
            || self.ui.pending_question.is_some()
            || self.ui.pending_text_input.is_some()
            || self.ui.dialog.is_some()
            || self.ai_running
            || self.prompt_job.is_some()
        {
            return self.status.clone();
        }
        if self.view_mode == ViewMode::Home {
            return self.home_status_message().to_string();
        }
        "Enter 发送 · /exit /q /quit 退出".to_string()
    }
}

fn format_number(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(digit);
    }
    result
}
