//! Toast — transient notification popup rendered at bottom-center.

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::tui::SessionView;

pub struct Toast<'a> {
    view: &'a SessionView,
}

impl<'a> Toast<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    /// Render the toast if one is active. No-op otherwise.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let view = self.view;
        let Some(toast) = &view.ui.toast else {
            return;
        };
        let toast_area = super::super::layout::toast_rect(area);
        let text = crate::tui::util::strip_terminal_controls(toast.as_str()).into_owned();
        let widget = Paragraph::new(text)
            .style(view.theme.system_style())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(view.theme.border_style())
                    .style(view.theme.panel_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, toast_area);
        frame.render_widget(widget, toast_area);
    }
}
