//! Sidebar panel — workspace path and TODO list in the sidebar column.

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

    /// Render the workspace path into `workspace_area` if present, and the
    /// TODO panel into `todo_area` if present.
    pub fn render(
        &self,
        frame: &mut Frame,
        workspace_area: Option<Rect>,
        todo_area: Option<Rect>,
    ) {
        let view = self.view;
        if let Some(area) = workspace_area {
            let path = Line::from(Span::styled(
                view.cwd.display().to_string(),
                view.theme.sidebar_style(),
            ));
            let workspace = Paragraph::new(vec![path])
                .style(view.theme.sidebar_style())
                .block(
                    Block::default()
                        .title(" Workspace ")
                        .title_style(view.theme.title_style())
                        .borders(Borders::ALL)
                        .border_style(view.theme.border_style()),
                )
                .wrap(Wrap { trim: false });
            frame.render_widget(workspace, area);
        }

        if let Some(area) = todo_area {
            self.render_todo(frame, area);
        }
    }

    fn render_todo(&self, frame: &mut Frame, area: Rect) {
        let view = self.view;
        let tasks = view.cached_tasks.borrow();
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
                    _ => ("[ ]", view.theme.sidebar_style()),
                };
                Line::from(Span::styled(format!("{mark} {}", t.title), style))
            })
            .collect();
        let todo = Paragraph::new(todo_lines)
            .style(view.theme.sidebar_style())
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
