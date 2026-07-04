//! Minimal interactive TUI for OpenRust.

use std::io::{self, Write};
use std::path::PathBuf;
use std::io::IsTerminal;

use crossterm::{cursor, execute, terminal::{self, ClearType}};
use futures::StreamExt;

use crate::core::{config::Config, provider::{self, Message, RequestOptions, StreamChunk, ToolDef, ToolFunction}};
use crate::tool::catalog;

pub fn run(script: Option<PathBuf>, prompt: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;
    let provider_name = config
        .provider
        .keys()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No provider configured"))?
        .to_string();
    let resolved = config
        .get_provider(&provider_name)
        .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;
    let llm = provider::create_provider(&resolved)
        .ok_or_else(|| anyhow::anyhow!("Failed to create provider '{}'", provider_name))?;
    let model = config
        .resolve_provider_model()
        .map(|(_, model)| model)
        .ok_or_else(|| anyhow::anyhow!("No model configured"))?;
    let system = crate::core::system_prompt::SystemPrompt::from_config(&config, &resolved, config.mode.clone())?.render();

    let script_lines = script
        .as_ref()
        .map(|path| load_script(path))
        .transpose()?
        .unwrap_or_default();

    let mut session = SessionView::new(provider_name, model, system, llm);
    session.bootstrap(prompt, script_lines)?;
    session.run()
}

struct SessionView {
    provider_name: String,
    model: String,
    system: String,
    llm: Box<dyn provider::LlmProvider>,
    messages: Vec<Message>,
    transcript: Vec<String>,
    interactive: bool,
}

impl SessionView {
    fn new(provider_name: String, model: String, system: String, llm: Box<dyn provider::LlmProvider>) -> Self {
        let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
        Self {
            provider_name,
            model,
            system,
            llm,
            messages: Vec::new(),
            transcript: Vec::new(),
            interactive,
        }
    }

    fn bootstrap(&mut self, prompt: Option<String>, script_lines: Vec<String>) -> anyhow::Result<()> {
        if let Some(prompt) = prompt {
            self.enqueue(prompt);
        }

        for line in script_lines {
            self.enqueue(line);
        }

        Ok(())
    }

    fn run(&mut self) -> anyhow::Result<()> {
        if !self.interactive {
            return self.run_headless();
        }

        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

        let run_result = self.run_inner(&mut stdout);

        execute!(stdout, terminal::LeaveAlternateScreen, cursor::Show)?;
        terminal::disable_raw_mode()?;

        run_result
    }

    fn run_headless(&mut self) -> anyhow::Result<()> {
        let mut stdout = io::stdout();
        let pending = std::mem::take(&mut self.transcript);
        for prompt in pending {
            self.handle_prompt(&mut stdout, &prompt)?;
        }
        Ok(())
    }

    fn run_inner(&mut self, stdout: &mut io::Stdout) -> anyhow::Result<()> {
        let pending = std::mem::take(&mut self.transcript);
        for prompt in pending {
            self.handle_prompt(stdout, &prompt)?;
        }

        loop {
            self.render(stdout, Some("Interactive input is paused; use --prompt or --script for automation"))?;
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let input = input.trim().to_string();
            if input.is_empty() || input == "/quit" || input == "/q" {
                break;
            }
            self.handle_prompt(stdout, &input)?;
        }

        Ok(())
    }

    fn handle_prompt(&mut self, stdout: &mut io::Stdout, prompt: &str) -> anyhow::Result<()> {
        self.messages.push(Message { role: "user".to_string(), content: prompt.to_string() });
        self.render(stdout, Some("Connecting model..."))?;

        let rt = tokio::runtime::Runtime::new()?;
        let mut stream = rt.block_on(self.llm.chat(
            self.messages.clone(),
            catalog::TOOL_CATALOG.iter().map(|meta| ToolDef {
                r#type: "function".to_string(),
                function: ToolFunction {
                    name: meta.name.to_string(),
                    description: meta.description.to_string(),
                    parameters: serde_json::json!({"type": "object", "properties": {}, "required": []}),
                },
            }).collect(),
            RequestOptions {
                model: self.model.clone(),
                temperature: None,
                max_tokens: None,
                system: Some(self.system.clone()),
            },
        ))?;

        let mut assistant = String::new();
        while let Some(chunk) = rt.block_on(stream.next()) {
            match chunk? {
                StreamChunk::TextDelta(text) | StreamChunk::ReasoningDelta(text) => {
                    assistant.push_str(&text);
                    self.render(stdout, Some(&assistant))?;
                }
                StreamChunk::ToolCallStart { name, .. } => {
                    self.render(stdout, Some(&format!("tool call: {}", name)))?;
                }
                StreamChunk::ToolCallDelta { .. } => {}
                StreamChunk::ToolCallEnd { .. } => {}
                StreamChunk::Finish { .. } => {}
            }
        }

        self.messages.push(Message { role: "assistant".to_string(), content: assistant.clone() });
        self.render(stdout, Some(&assistant))?;
        Ok(())
    }

    fn enqueue(&mut self, prompt: String) {
        self.transcript.push(prompt);
    }

    fn render(&self, stdout: &mut io::Stdout, status: Option<&str>) -> anyhow::Result<()> {
        if self.interactive {
            execute!(stdout, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        }
        writeln!(stdout, "OpenRust TUI")?;
        writeln!(stdout, "provider: {}", self.provider_name)?;
        writeln!(stdout, "model: {}", self.model)?;
        writeln!(stdout, "")?;
        if let Some(status) = status {
            writeln!(stdout, "status: {}", status)?;
        }
        writeln!(stdout, "")?;
        writeln!(stdout, "history:")?;
        for msg in self.messages.iter().rev().take(12).rev() {
            writeln!(stdout, "- {}: {}", msg.role, msg.content)?;
        }
        stdout.flush()?;
        Ok(())
    }
}

fn load_script(path: &PathBuf) -> anyhow::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_loader_ignores_comments_and_blanks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("script.txt");
        std::fs::write(&path, "\n# comment\nhello\n\nworld\n").unwrap();

        let script = load_script(&path).unwrap();
        assert_eq!(script, vec!["hello".to_string(), "world".to_string()]);
    }
}
