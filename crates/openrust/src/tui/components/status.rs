//! Status bar — the single-line footer at the bottom of every view.

use ratatui::{Frame, layout::Rect, widgets::Paragraph};

use crate::tui::SessionView;

pub struct StatusBar<'a> {
    view: &'a SessionView,
}

impl<'a> StatusBar<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let footer = Paragraph::new(self.view.status_line()).style(self.view.theme.footer_style());
        frame.render_widget(footer, area);
    }
}
