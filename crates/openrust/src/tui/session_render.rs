use std::io;
use std::io::Write;

use crossterm::{cursor, execute, terminal};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
};

use super::dialog::DialogKind;
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
        let mut layers = self.build_layers(frame.area());
        layers.sort_by_key(|l| l.z());

        for layer in &layers {
            layer.render(self, frame);
        }
    }
}
