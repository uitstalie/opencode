use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyModifiers};
use super::{SlashCommand, ThinkingModeCommand};

pub(super) fn load_script(path: &PathBuf) -> anyhow::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.to_string())
        .collect())
}

pub(super) fn should_exit(key: &crossterm::event::KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

pub(super) fn is_exit_command(input: &str) -> bool {
    matches!(input.trim(), "/exit" | "exit" | "/quit" | "/q")
}

pub(super) fn parse_slash_command(input: &str) -> Option<SlashCommand> {
    let mut parts = input.split_whitespace();
    match parts.next()? {
        "/thinking" | "/think" => Some(SlashCommand::Thinking(match parts.next() {
            Some("show") | Some("on") => ThinkingModeCommand::Show,
            Some("hide") | Some("off") => ThinkingModeCommand::Hide,
            _ => ThinkingModeCommand::Toggle,
        })),
        "/session" => Some(SlashCommand::Session(parts.map(str::to_string).collect())),
        "/agent" => Some(SlashCommand::Agent(parts.map(str::to_string).collect())),
        "/compact" | "/compaction" => {
            Some(SlashCommand::Compact(parts.map(str::to_string).collect()))
        }
        "/task" => Some(SlashCommand::Task(parts.map(str::to_string).collect())),
        "/connect" => Some(SlashCommand::Connect(parts.map(str::to_string).collect())),
        "/models" | "/model" => Some(SlashCommand::Models(parts.map(str::to_string).collect())),
        "/files" | "/tree" => Some(SlashCommand::Files),
        "/diff" => Some(SlashCommand::Diff),
        "/reload" => Some(SlashCommand::Reload),
        "/dream" => Some(SlashCommand::Dream),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_commands_include_agent_and_task() {
        match parse_slash_command("/agent use build") {
            Some(SlashCommand::Agent(args)) => assert_eq!(args, vec!["use", "build"]),
            other => panic!("unexpected parse result: {:?}", other),
        }

        match parse_slash_command("/compact 4") {
            Some(SlashCommand::Compact(args)) => assert_eq!(args, vec!["4"]),
            other => panic!("unexpected parse result: {:?}", other),
        }

        match parse_slash_command("/task add write tests pending") {
            Some(SlashCommand::Task(args)) => {
                assert_eq!(args, vec!["add", "write", "tests", "pending"])
            }
            other => panic!("unexpected parse result: {:?}", other),
        }

        match parse_slash_command("/thinking show") {
            Some(SlashCommand::Thinking(ThinkingModeCommand::Show)) => {}
            other => panic!("unexpected parse result: {:?}", other),
        }
    }
}
