//! Slash-help popup — floating command list above the input area.
//!
//! Extracted from InputPanel to become an independent Float layer.
//! Renders only when a `DialogKind::SlashHelp` dialog is active.

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::tui::dialog::DialogKind;
use crate::tui::SessionView;

pub struct SlashHelpPopup<'a> {
    view: &'a SessionView,
}

impl<'a> SlashHelpPopup<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    /// Render the popup above `input_area` if a slash-help dialog is active.
    pub fn render(&self, frame: &mut Frame, input_area: Rect) {
        let view = self.view;
        let Some(dialog) = &view.ui.dialog else {
            return;
        };
        if dialog.kind != DialogKind::SlashHelp {
            return;
        }
        let count = dialog.option_count();
        if count == 0 {
            return;
        }

        let panel_height = (count as u16 + 2).min(input_area.y);
        let panel_area = Rect {
            x: input_area.x,
            y: input_area.y.saturating_sub(panel_height),
            width: input_area.width,
            height: panel_height,
        };
        frame.render_widget(Clear, panel_area);
        let lines = dialog.compact_lines(&view.theme);
        let panel = Paragraph::new(lines)
            .style(view.theme.panel_style())
            .block(
                Block::default()
                    .title(" Commands ")
                    .title_style(view.theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(view.theme.border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(panel, panel_area);
    }
}
