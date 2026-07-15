use ratatui::{
    style::Modifier,
    text::{Line, Span},
};

use super::render::Theme;

pub(super) struct Dialog {
    pub(super) kind: DialogKind,
    title: String,
    description: String,
    options: Vec<DialogOption>,
    selected: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DialogKind {
    Thinking,
    Session,
    SessionDelete,
    Agent,
    Task,
    Provider,
    ProviderModel,
    ConnectProtocol,
    Model,
    ReasoningEffort,
    SlashHelp,
    Theme,
    ReasoningToggle,
    ModelConfigLoop,
}

impl Dialog {
    pub(super) fn new(
        kind: DialogKind,
        title: impl Into<String>,
        description: impl Into<String>,
        options: Vec<DialogOption>,
        selected: usize,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            description: description.into(),
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
        self.options
            .get(self.selected)
            .map(|option| option.value.as_str())
    }

    pub(super) fn set_selected(&mut self, selected: usize) {
        self.selected = selected.min(self.options.len().saturating_sub(1));
    }

    pub(super) fn title(&self) -> &str {
        &self.title
    }

    pub(super) fn description(&self) -> &str {
        &self.description
    }

    pub(super) fn footer_hint(&self) -> &'static str {
        "↑/↓ 选择 · Enter 确认 · Esc 关闭"
    }

    pub(super) fn option_count(&self) -> usize {
        self.options.len()
    }

    pub(super) fn compact_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        self.options
            .iter()
            .enumerate()
            .map(|(index, option)| {
                let selected = index == self.selected;
                let marker = if selected { "› " } else { "  " };
                if selected {
                    Line::from(vec![
                        Span::styled(
                            format!("{}{}", marker, option.value),
                            theme.dialog_selected_style().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  {}", option.description),
                            theme.muted_style(),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled(
                            format!("{}{}", marker, option.value),
                            theme.dialog_style(),
                        ),
                        Span::styled(
                            format!("  {}", option.description),
                            theme.muted_style(),
                        ),
                    ])
                }
            })
            .collect()
    }

    pub(super) fn option_lines(&self, theme: &Theme, max_visible: usize) -> Vec<Line<'static>> {
        let offset = self.compute_offset(max_visible);
        self.options
            .iter()
            .enumerate()
            .skip(offset)
            .take(max_visible)
            .flat_map(|(index, option)| {
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
                    Line::from(Span::styled(
                        format!("    {}", option.description),
                        theme.muted_style(),
                    )),
                    Line::from(""),
                ]
            })
            .collect()
    }

    pub(super) fn option_index_at(&self, row: usize, max_visible: usize) -> Option<usize> {
        let offset = self.compute_offset(max_visible);
        let index = offset + row / 3;
        if index >= self.options.len() {
            return None;
        }
        if index >= offset + max_visible {
            return None;
        }
        Some(index)
    }

    fn compute_offset(&self, max_visible: usize) -> usize {
        let mv = max_visible.max(1);
        if self.selected < mv {
            return 0;
        }
        let offset = self.selected.saturating_sub(mv / 2);
        offset.min(self.options.len().saturating_sub(mv))
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
        (
            "/connect",
            "connect",
            "Configure providers and verify credentials",
        ),
        (
            "/models",
            "models",
            "Switch registered models and reasoning effort",
        ),
        ("/compact", "compact", "Compact the current session"),
        ("/files", "files", "Toggle the file-tree sidebar"),
        ("/diff", "diff", "Show the last edit as a diff"),
        ("/reload", "reload", "Reload config, agents, skills, rules"),
        (
            "/init",
            "init",
            "Initialize project: check, investigate, scaffold docs",
        ),
        ("/dream", "dream", "Summarize sessions into dreaming memory"),
        ("/theme", "theme", "Switch color theme"),
        ("/exit", "exit", "Exit the TUI"),
    ];

    items
        .iter()
        .filter(|(value, _, _)| value.starts_with(input))
        .map(|(value, label, description)| DialogOption::new(*value, *label, *description))
        .collect()
}
