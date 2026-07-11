//! Status bar — the single-line footer at the bottom of every view,
//! plus the info line directly above it (model, thinking, agent).

use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::tui::SessionView;

pub struct StatusBar<'a> {
    view: &'a SessionView,
}

impl<'a> StatusBar<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    pub fn render(&self, frame: &mut Frame, info_area: Rect, status_area: Rect) {
        let info = Paragraph::new(self.view.info_line()).style(self.view.theme.muted_style());
        frame.render_widget(info, info_area);

        let footer = Paragraph::new(self.view.status_line()).style(self.view.theme.footer_style());
        frame.render_widget(footer, status_area);
    }
}
