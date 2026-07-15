//! Session conversation panel — renders the message history with
//! scroll state, selection highlight, and per-message backgrounds.

use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
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

        view.render.area_top.set(area.y);
        view.render.area_height.set(area.height);

        // Scan rows and render message blocks sequentially.
        // A "block" is a contiguous run of non-empty lines (one message).
        // Empty lines serve as transparent separators between blocks.
        let mut y = area.y;
        let mut i = 0;
        while i < rows.len() && y < area.y + area.height {
            // Skip empty separator lines.
            while i < rows.len() && rows[i].text.is_empty() {
                i += 1;
                y += 1;
            }
            if i >= rows.len() || y >= area.y + area.height {
                break;
            }
            // Collect this message block's non-empty lines.
            let block_start = i;
            while i < rows.len() && !rows[i].text.is_empty() {
                i += 1;
            }
            let block_end = i;
            let block_h = (block_end - block_start) as u16;
            let visible_h = block_h.min(area.y + area.height - y);
            if visible_h == 0 {
                break;
            }

            let block_area = Rect { x: area.x, y, width: area.width, height: visible_h };

            let lines: Vec<Line<'static>> = (block_start..block_start + visible_h as usize)
                .map(|ri| {
                    let row = &rows[ri];
                    let abs_index = ri + scroll_offset;
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

            let msg = Paragraph::new(lines)
                .style(view.theme.message_bg_style())
                .wrap(Wrap { trim: false });
            frame.render_widget(msg, block_area);

            y += visible_h;
        }
    }
}
