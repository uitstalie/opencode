//! Input panel — renders the input textarea, the inline slash-help
//! popup floating above it, and pins the hardware cursor.

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
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

    /// Render the slash-help popup (if active), the input textarea,
    /// and place the hardware cursor.  `title` distinguishes Home
    /// ("Prompt") from Session ("Input").
    pub fn render(&self, frame: &mut Frame, area: Rect, title: &str) {
        self.render_slash_help(frame, area);

        let input = self.input_widget(title);
        frame.render_widget(&input, area);
        self.place_cursor(frame, area);
    }

    fn render_slash_help(&self, frame: &mut Frame, input_area: Rect) {
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
