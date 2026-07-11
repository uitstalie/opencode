use std::io;
use std::io::Write;

use crossterm::{cursor, execute, terminal};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
};

use super::components::{HomeView, InputPanel, ModalLayer, SessionPanel, SidebarPanel, StatusBar, Toast};
use super::dialog::DialogKind;
use super::layout;
use super::SessionView;

impl SessionView {
    pub(super) fn render(&self, stdout: &mut io::Stdout, status: Option<&str>) -> anyhow::Result<()> {
        if self.interactive {
            execute!(
                stdout,
                terminal::Clear(terminal::ClearType::All),
                cursor::MoveTo(0, 0)
            )?;
        }
        writeln!(stdout, "OpenRust TUI")?;
        writeln!(stdout, "provider: {}", self.provider_name)?;
        writeln!(stdout, "model: {}", self.model)?;
        writeln!(stdout)?;
        if let Some(status) = status {
            writeln!(stdout, "status: {}", status)?;
        }
        writeln!(stdout)?;
        if self.interactive {
            writeln!(stdout, "input: {}", self.input_editor.lines().join("\n"))?;
            writeln!(stdout)?;
        }
        writeln!(stdout, "history:")?;
        for msg in self.messages.iter().rev().take(12).rev() {
            writeln!(stdout, "- {}: {}", msg.role, msg.content)?;
        }
        stdout.flush()?;
        Ok(())
    }

    pub(super) fn render_terminal(
        &self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        terminal.draw(|frame| self.render_frame(frame))?;
        if self.ui.dialog.as_ref().is_some_and(|d| d.kind != DialogKind::SlashHelp)
            || self.ui.pending_question.is_some()
            || self.ui.pending_permission.is_some()
        {
            execute!(terminal.backend_mut(), cursor::Hide)?;
        }
        Ok(())
    }

    pub(super) fn render_frame(&self, frame: &mut Frame) {
        if self.view_mode == super::ViewMode::Home {
            HomeView::new(self).render(frame);
            return;
        }
        self.render_session_frame(frame);
    }

    fn render_session_frame(&self, frame: &mut Frame) {
        let task_count = self
            .store
            .as_ref()
            .and_then(|s| s.task_count(&self.session_id).ok())
            .unwrap_or(0);

        let layout = layout::session_layout(
            frame.area(),
            self.sidebar_visible,
            self.sidebar.is_some(),
            task_count,
        );

        SidebarPanel::new(self).render(frame, layout.sidebar_files, layout.sidebar_todo);
        SessionPanel::new(self).render(frame, layout.session);
        InputPanel::new(self).render(frame, layout.input, "Input");
        StatusBar::new(self).render(frame, layout.info, layout.status);
        Toast::new(self).render(frame, frame.area());
        ModalLayer::new(self).render(frame, frame.area());
    }
}
