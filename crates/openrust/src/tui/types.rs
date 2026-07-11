//! Type definitions extracted from the TUI root module.
//!
//! All types are `pub(super)` — visible within the `tui` module tree
//! but not exposed to the rest of the crate.

use std::cell::{Cell, RefCell};
use std::time::Instant;

use tui_textarea::TextArea;

use crate::tool::AskRequest;
use super::dialog::Dialog;

pub(super) struct QuestionItem {
    pub(super) header: String,
    pub(super) question: String,
    pub(super) options: Vec<(String, String)>,
    pub(super) multiple: bool,
}

pub(super) struct PendingQuestion {
    pub(super) responder: std::sync::mpsc::Sender<Vec<String>>,
    pub(super) items: Vec<QuestionItem>,
    pub(super) current: usize,
    pub(super) selected: usize,
    pub(super) picked: Vec<usize>,
    pub(super) answers: Vec<String>,
    pub(super) typing: Option<String>,
}

impl PendingQuestion {
    /// Build from an `AskRequest`. Returns `None` if the payload has no valid questions.
    pub(super) fn from_request(request: AskRequest) -> Option<Self> {
        let items: Vec<QuestionItem> = request
            .questions
            .as_array()?
            .iter()
            .filter_map(|value| {
                let question = value.get("question").and_then(|v| v.as_str())?.to_string();
                let header = value
                    .get("header")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let options = value
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(|option| {
                                let label =
                                    option.get("label").and_then(|v| v.as_str())?.to_string();
                                let description = option
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                Some((label, description))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let multiple = value
                    .get("multiple")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Some(QuestionItem {
                    header,
                    question,
                    options,
                    multiple,
                })
            })
            .collect();
        if items.is_empty() {
            let _ = request.responder.send(Vec::new());
            return None;
        }
        Some(Self {
            responder: request.responder,
            items,
            current: 0,
            selected: 0,
            picked: Vec::new(),
            answers: Vec::new(),
            typing: None,
        })
    }

    pub(super) fn item(&self) -> &QuestionItem {
        &self.items[self.current]
    }

    pub(super) fn row_count(&self) -> usize {
        self.item().options.len() + 1
    }

    pub(super) fn custom_index(&self) -> usize {
        self.item().options.len()
    }

    pub(super) fn next(&mut self) {
        let count = self.row_count();
        self.selected = (self.selected + 1) % count;
    }

    pub(super) fn previous(&mut self) {
        let count = self.row_count();
        self.selected = if self.selected == 0 {
            count - 1
        } else {
            self.selected - 1
        };
    }

    pub(super) fn toggle_pick(&mut self) {
        if !self.item().multiple || self.selected >= self.item().options.len() {
            return;
        }
        match self.picked.iter().position(|&index| index == self.selected) {
            Some(existing) => {
                self.picked.remove(existing);
            }
            None => self.picked.push(self.selected),
        }
    }

    pub(super) fn record(&mut self, answer: String) -> bool {
        self.answers.push(answer);
        self.current += 1;
        self.selected = 0;
        self.picked.clear();
        self.typing = None;
        self.current >= self.items.len()
    }

    pub(super) fn confirm(&mut self) -> Option<Vec<String>> {
        if self.selected == self.custom_index() {
            self.typing = Some(String::new());
            return None;
        }
        let item = self.item();
        let answer = if item.multiple {
            if self.picked.is_empty() {
                item.options[self.selected].0.clone()
            } else {
                let mut picks = self.picked.clone();
                picks.sort_unstable();
                picks
                    .iter()
                    .map(|&index| item.options[index].0.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        } else {
            item.options[self.selected].0.clone()
        };
        if self.record(answer) {
            return Some(std::mem::take(&mut self.answers));
        }
        None
    }

    pub(super) fn commit_custom(&mut self) -> Option<Vec<String>> {
        let text = self.typing.take().unwrap_or_default();
        if self.record(text) {
            return Some(std::mem::take(&mut self.answers));
        }
        None
    }
}

pub(super) struct PendingPermission {
    pub(super) responder: std::sync::mpsc::Sender<bool>,
    pub(super) tool: String,
    pub(super) detail: String,
    pub(super) allow: bool,
}

pub(super) struct PendingTextInput {
    pub(super) title: String,
    pub(super) description: String,
    pub(super) value: String,
    pub(super) editor: TextArea<'static>,
    pub(super) submit: fn(&mut super::SessionView, &str),
}

impl PendingTextInput {
    pub(super) fn sync_value(&mut self) {
        self.value = self.editor.lines().join("\n");
    }
}

#[derive(Clone)]
pub(super) struct SessionRenderLine {
    pub(super) line: ratatui::text::Line<'static>,
    pub(super) text: String,
    pub(super) tool_message_index: Option<usize>,
}

#[derive(Default)]
pub(super) struct ConnectDraft {
    pub(super) provider: String,
    pub(super) base_url: String,
    pub(super) model: String,
    pub(super) wire_model: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ViewMode {
    Home,
    Session,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ThinkingMode {
    Show,
    Hide,
}

impl ThinkingMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            ThinkingMode::Show => "show",
            ThinkingMode::Hide => "hide",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ThinkingModeCommand {
    Show,
    Hide,
    Toggle,
}

impl ThinkingModeCommand {
    pub(super) fn apply(self, current: ThinkingMode) -> ThinkingMode {
        match self {
            ThinkingModeCommand::Show => ThinkingMode::Show,
            ThinkingModeCommand::Hide => ThinkingMode::Hide,
            ThinkingModeCommand::Toggle => match current {
                ThinkingMode::Show => ThinkingMode::Hide,
                ThinkingMode::Hide => ThinkingMode::Show,
            },
        }
    }
}

#[derive(Debug)]
pub(super) enum SlashCommand {
    Thinking(ThinkingModeCommand),
    Session(Vec<String>),
    Agent(Vec<String>),
    Compact(Vec<String>),
    Task(Vec<String>),
    Connect(Vec<String>),
    Models(Vec<String>),
    Files,
    Diff,
    Reload,
    Dream,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ToolState {
    Created,
    Running,
}

#[derive(Clone, Debug)]
pub(super) struct PendingTool {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) state: ToolState,
    pub(super) started_at: std::time::Instant,
}

pub(super) struct CacheStats {
    pub(super) hits: usize,
    pub(super) total: usize,
    pub(super) prompt_count: usize,
}

impl CacheStats {
    pub(super) fn rate(&self) -> Option<usize> {
        if self.total == 0 {
            return None;
        }
        Some(self.hits * 100 / self.total)
    }
}

pub(super) struct RenderState {
    pub(super) lines: RefCell<Vec<SessionRenderLine>>,
    pub(super) all_lines: RefCell<Vec<SessionRenderLine>>,
    pub(super) scroll_offset: Cell<usize>,
    pub(super) selection: Option<(usize, usize)>,
    pub(super) mouse_down_row: Option<usize>,
    pub(super) mouse_dragging: bool,
    pub(super) area_top: Cell<u16>,
    pub(super) area_height: Cell<u16>,
    pub(super) dialog_area: Cell<Option<ratatui::layout::Rect>>,
}

pub(super) struct DialogState {
    pub(super) dialog: Option<Dialog>,
    pub(super) toast: Option<String>,
    pub(super) toast_deadline: Option<Instant>,
    pub(super) pending_question: Option<PendingQuestion>,
    pub(super) pending_permission: Option<PendingPermission>,
    pub(super) pending_text_input: Option<PendingTextInput>,
    pub(super) connect_draft: Option<ConnectDraft>,
}
