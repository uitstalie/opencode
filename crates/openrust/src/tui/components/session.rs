//! Session conversation panel — renders the message history with
//! scroll state, selection highlight, and live streaming previews.

use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::SessionView;

pub struct SessionPanel<'a> {
    view: &'a SessionView,
}

impl<'a> SessionPanel<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let view = self.view;
        view.session_render_lines_for_area(area.height as usize, area.width as usize);

        let scroll_offset = view.render.scroll_offset.get();
        let rows = view.render.lines.borrow();
        let lines: Vec<Line<'static>> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let abs_index = index + scroll_offset;
                if view.render.selection.is_some_and(|(start, end)| {
                    let (from, to) = if start <= end { (start, end) } else { (end, start) };
                    abs_index >= from && abs_index <= to
                }) {
                    Line::from(vec![Span::styled(
                        row.text.clone(),
                        view.theme
                            .dialog_selected_style()
                            .add_modifier(Modifier::BOLD),
                    )])
                } else {
                    row.line.clone()
                }
            })
            .collect();

        view.render.area_top.set(area.y);
        view.render.area_height.set(area.height);

        let session = Paragraph::new(lines)
            .style(view.theme.panel_style())
            .block(
                Block::default()
                    .title(" Session ")
                    .title_style(view.theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(view.theme.border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(session, area);
    }
}
