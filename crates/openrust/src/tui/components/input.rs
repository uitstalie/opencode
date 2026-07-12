//! Input panel — renders the input textarea and pins the hardware cursor.
//!
//! Slash-help popup rendering has been extracted to [`SlashHelpPopup`]
//! and is handled as a separate Float layer by the view system.

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders},
};
use tui_textarea::TextArea;

use crate::tui::dialog::DialogKind;
use crate::tui::util::{home_input_hint, textarea_display_col};
use crate::tui::{SessionView, ViewMode};

pub struct InputPanel<'a> {
    view: &'a SessionView,
}

impl<'a> InputPanel<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    /// Render the input textarea and place the hardware cursor.
    /// `title` distinguishes Home ("Prompt") from Session ("Input").
    pub fn render(&self, frame: &mut Frame, area: Rect, title: &str) {
        let input = self.input_widget(title);
        frame.render_widget(&input, area);
        self.place_cursor(frame, area);
    }

    fn place_cursor(&self, frame: &mut Frame, area: Rect) {
        let view = self.view;
        let modal_blocks_cursor = view.ui.pending_question.is_some()
            || view.ui.pending_permission.is_some()
            || view.ui.pending_text_input.is_some()
            || view
                .ui
                .dialog
                .as_ref()
                .is_some_and(|d| d.kind != DialogKind::SlashHelp);
        if modal_blocks_cursor {
            return;
        }
        let (row, col) = view.input_editor.cursor();
        let display_col = textarea_display_col(view.input_editor.lines(), row, col);
        frame.set_cursor_position((area.x + 1 + display_col as u16, area.y + 1 + row as u16));
    }

    fn input_widget(&self, title: &str) -> TextArea<'static> {
        let view = self.view;
        let mut textarea = view.input_editor.clone();
        textarea.set_style(view.theme.input_style());
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.set_cursor_style(ratatui::style::Style::default());
        textarea.set_placeholder_text(if view.view_mode == ViewMode::Home {
            home_input_hint()
        } else {
            "输入消息后 Enter 发送"
        });
        textarea.set_placeholder_style(view.theme.muted_style());
        textarea.set_block(
            Block::default()
                .title(format!(" {} ", title))
                .title_style(view.theme.title_style())
                .borders(Borders::ALL)
                .border_style(view.theme.input_border_style(view.ai_running)),
        );
        textarea
    }
}
