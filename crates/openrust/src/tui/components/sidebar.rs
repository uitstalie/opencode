//! Sidebar panel — file tree and TODO list in the sidebar column.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::tui::SessionView;

pub struct SidebarPanel<'a> {
    view: &'a SessionView,
}

impl<'a> SidebarPanel<'a> {
    pub fn new(view: &'a SessionView) -> Self {
        Self { view }
    }

    /// Render the file tree into `files_area` if present, and the
    /// TODO panel into `todo_area` if present.
    pub fn render(
        &self,
        frame: &mut Frame,
        files_area: Option<Rect>,
        todo_area: Option<Rect>,
    ) {
        let view = self.view;
        if let Some(area) = files_area
            && let Some(tree) = &view.sidebar
        {
            let sidebar = Paragraph::new(tree.lines(&view.theme))
                .style(view.theme.panel_style())
                .block(
                    Block::default()
                        .title(" Files ")
                        .title_style(view.theme.title_style())
                        .borders(Borders::ALL)
                        .border_style(view.theme.border_style()),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(sidebar, area);
        }

        if let Some(area) = todo_area {
            self.render_todo(frame, area);
        }
    }

    fn render_todo(&self, frame: &mut Frame, area: Rect) {
        let view = self.view;
        let tasks: Vec<_> = view
            .store
            .as_ref()
            .and_then(|s| s.list_tasks(&view.session_id).ok())
            .unwrap_or_default();
        if tasks.is_empty() {
            return;
        }

        let completed = tasks.iter().filter(|t| t.status == "completed").count();
        let total = tasks.len();
        let todo_lines: Vec<Line<'static>> = tasks
            .iter()
            .map(|t| {
                let (mark, style) = match t.status.as_str() {
                    "completed" => ("[x]", view.theme.muted_style()),
                    "in_progress" => ("[~]", view.theme.assistant_style()),
                    "cancelled" => ("[-]", view.theme.muted_style()),
                    _ => ("[ ]", view.theme.panel_style()),
                };
                Line::from(Span::styled(format!("{mark} {}", t.title), style))
            })
            .collect();
        let todo = Paragraph::new(todo_lines)
            .style(view.theme.panel_style())
            .block(
                Block::default()
                    .title(format!(" TODO {completed}/{total} "))
                    .title_style(view.theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(view.theme.border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(todo, area);
    }
}
