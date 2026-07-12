//! Home content — header, hint line, and status message for the Home view.
//!
//! Only renders the home-specific chrome. Input, status bar, toast,
//! and modal overlays are handled as separate layers by the view system.

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use crate::tui::SessionView;

pub struct HomeContent<'a> {
    view: &'a SessionView,
}

impl<'a> HomeContent<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    pub fn render(&self, frame: &mut Frame, header_area: Rect, hint_area: Rect, status_area: Rect) {
        let view = self.view;

        let header = Paragraph::new(vec![
            Line::from(Span::styled(
                "OpenRust",
                view.theme.brand_style().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "AI coding agent \u{00b7} Rust native runtime",
                view.theme.muted_style(),
            )),
            Line::from(Span::styled(
                format!(
                    "model {}/{} \u{00b7} agent {}",
                    view.provider_name,
                    view.model,
                    view.current_session_agent().unwrap_or_else(|| "default".to_string())
                ),
                view.theme.muted_style(),
            )),
        ])
        .alignment(Alignment::Center)
        .style(view.theme.panel_style());
        frame.render_widget(header, header_area);

        let hint = Paragraph::new(
            "\u{8f93}\u{5165}\u{6d88}\u{606f}\u{540e} Enter \u{5f00}\u{59cb} \u{00b7} /connect \u{914d}\u{7f6e} provider \u{00b7} /models \u{9009}\u{62e9}\u{6a21}\u{578b} \u{00b7} Esc \u{9000}\u{51fa}",
        )
        .style(view.theme.muted_style())
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false });
        frame.render_widget(hint, hint_area);

        let status = Paragraph::new(view.home_status_message())
            .style(view.theme.muted_style())
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(status, status_area);
    }
}
