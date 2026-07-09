//! Layout computation for all TUI view modes.
//!
//! Produces concrete `Rect` regions for each visual component.
//! The render entry point calls these functions, then passes the
//! resulting rects to individual component renderers.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub(super) struct SessionLayout {
    /// Main conversation area (already minus sidebar if present).
    pub session: Rect,
    /// Files tree sub-region inside the sidebar.
    pub sidebar_files: Option<Rect>,
    /// TODO panel sub-region inside the sidebar.
    pub sidebar_todo: Option<Rect>,
    /// Input box area.
    pub input: Rect,
    /// Bottom status line.
    pub status: Rect,
}

/// Regions for the Home view.
pub(super) struct HomeLayout {
    pub header: Rect,
    pub input: Rect,
    pub hint: Rect,
    pub status_message: Rect,
    pub status: Rect,
}

/// Compute the session layout: vertical split into session/input/status,
/// with an optional sidebar column carved from the session region.
pub(super) fn session_layout(
    area: Rect,
    sidebar_visible: bool,
    has_sidebar: bool,
    task_count: usize,
) -> SessionLayout {
    let main = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(5), Constraint::Length(1)])
        .split(area);

    let session_full = main[0];
    let input = main[1];
    let status = main[2];

    let (session, sidebar) = if sidebar_visible && has_sidebar {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(32), Constraint::Min(20)])
            .split(session_full);
        (chunks[1], Some(chunks[0]))
    } else {
        (session_full, None)
    };

    let (sidebar_files, sidebar_todo) = sidebar.map(|side| {
        if task_count == 0 {
            (Some(side), None)
        } else {
            let todo_height = (task_count as u16 + 2).min(side.height / 2).max(4);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(todo_height)])
                .split(side);
            (Some(chunks[0]), Some(chunks[1]))
        }
    }).unwrap_or((None, None));

    SessionLayout {
        session,
        sidebar_files,
        sidebar_todo,
        input,
        status,
    }
}

/// Compute the home layout: a centered box with sections.
pub(super) fn home_layout(area: Rect) -> HomeLayout {
    let centered = centered_rect(76, 72, area);

    let top_padding = centered.height.saturating_sub(15) / 2;
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_padding),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(centered);

    HomeLayout {
        header: sections[1],
        input: sections[2],
        hint: sections[3],
        status_message: sections[4],
        status: Layout::default()
            .constraints([Constraint::Length(1)])
            .split(area)[0],
    }
}

/// Center a rect inside `area` at the given percentage.
pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

/// Compute a modal rect anchored near the top-center of the screen.
pub(super) fn modal_rect(percent_x: u16, height: u16, top_offset: u16, area: Rect) -> Rect {
    let width = area.width.saturating_mul(percent_x).saturating_div(100).max(24);
    let height = height.min(area.height.saturating_sub(2)).max(6);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area
        .y
        .saturating_add(top_offset)
        .min(area.y + area.height.saturating_sub(height));
    Rect { x, y, width, height }
}

/// Compute the toast popup rect (bottom-center).
pub(super) fn toast_rect(area: Rect) -> Rect {
    let width = area.width.saturating_mul(60).saturating_div(100).max(20);
    let height = 3u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(4);
    Rect { x, y, width, height }
}
