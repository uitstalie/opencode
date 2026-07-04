use ratatui::{
    layout::Alignment,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::render::Theme;

pub(super) struct Dialog {
    pub(super) kind: DialogKind,
    title: &'static str,
    description: &'static str,
    options: Vec<DialogOption>,
    selected: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DialogKind {
    Thinking,
    Session,
    Agent,
    Task,
    Provider,
    Model,
    ReasoningEffort,
    SlashHelp,
}

impl Dialog {
    pub(super) fn new(
        kind: DialogKind,
        title: &'static str,
        description: &'static str,
        options: Vec<DialogOption>,
        selected: usize,
    ) -> Self {
        Self {
            kind,
            title,
            description,
            selected: selected.min(options.len().saturating_sub(1)),
            options,
        }
    }

    pub(super) fn next(&mut self) {
        if self.options.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.options.len();
    }

    pub(super) fn previous(&mut self) {
        if self.options.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.options.len() - 1
        } else {
            self.selected - 1
        };
    }

    pub(super) fn selected_value(&self) -> Option<&str> {
        self.options.get(self.selected).map(|option| option.value.as_str())
    }

    pub(super) fn widget(&self, theme: &Theme) -> Paragraph<'static> {
        let mut lines = vec![
            Line::from(Span::styled(self.description, theme.muted_style())),
            Line::from(""),
        ];
        lines.extend(self.options.iter().enumerate().flat_map(|(index, option)| {
            let selected = index == self.selected;
            let marker = if selected { "› " } else { "  " };
            let title = if selected {
                Span::styled(
                    format!("{}{}", marker, option.label),
                    theme.dialog_selected_style().add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!("{}{}", marker, option.label), theme.dialog_style())
            };
            [
                Line::from(title),
                Line::from(Span::styled(format!("    {}", option.description), theme.muted_style())),
            ]
        }));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("↑/↓ 选择 · Enter 确认 · Esc 关闭", theme.muted_style())));

        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(format!(" {} ", self.title))
                    .title_alignment(Alignment::Center)
                    .title_style(theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(theme.dialog_border_style()),
            )
            .style(theme.dialog_style())
            .wrap(Wrap { trim: false })
    }
}

pub(super) struct DialogOption {
    value: String,
    label: String,
    description: String,
}

impl DialogOption {
    pub(super) fn new(
        value: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: description.into(),
        }
    }

    pub(super) fn value(&self) -> &str {
        &self.value
    }
}

pub(super) fn slash_options(input: &str) -> Vec<DialogOption> {
    let items = [
        ("/thinking", "thinking", "Toggle thinking visibility"),
        ("/session", "session", "List or switch sessions"),
        ("/agent", "agent", "List or switch agents"),
        ("/task", "task", "List or update tasks"),
        ("/connect", "connect", "Configure providers and verify credentials"),
        ("/models", "models", "Switch registered models and reasoning effort"),
        ("/exit", "exit", "Exit the TUI"),
    ];

    items
        .iter()
        .filter(|(value, _, _)| value.starts_with(input))
        .map(|(value, label, description)| DialogOption::new(*value, *label, *description))
        .collect()
}

pub(super) fn slash_hint_dialog(input: &str, theme: &Theme) -> Paragraph<'static> {
    let options = slash_options(input);
    let mut lines = vec![Line::from(Span::styled("Slash command hint", theme.title_style()))];
    lines.push(Line::from(""));
    for option in options {
        lines.push(Line::from(vec![
            Span::styled(format!("{}", option.value), theme.dialog_selected_style()),
            Span::raw("  "),
            Span::styled(option.description, theme.muted_style()),
        ]));
    }
    Paragraph::new(lines)
        .block(
            Block::default()
                .title(" / ")
                .title_alignment(Alignment::Center)
                .title_style(theme.title_style())
                .borders(Borders::ALL)
                .border_style(theme.dialog_border_style()),
        )
        .style(theme.dialog_style())
        .wrap(Wrap { trim: false })
}
