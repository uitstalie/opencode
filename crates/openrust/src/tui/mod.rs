//! Minimal interactive TUI for OpenRust.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::IsTerminal;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal,
};
use futures::StreamExt;
use ratatui::{Frame, Terminal, backend::CrosstermBackend, style::Modifier, text::{Line, Span}, widgets::{Block, Borders, Clear, Paragraph, Wrap}};

use crate::core::{
    agent,
    config::{Config, ModelConfig, ProviderConfig},
    provider::{self, Message, MessageContent, RequestOptions, StreamChunk},
    session::SessionStore,
    token,
    vault::Vault,
};
use crate::tool::AskRequest;

mod dialog;
mod diff;
mod highlight;
mod input;
mod markdown;
mod render;
mod interaction;
mod prompt_flow;
mod session_render;
mod sidebar;
mod worker;

use dialog::{Dialog, DialogKind, DialogOption, slash_options};
use input::{input_width, is_exit_command, load_script, next_char_boundary, prev_char_boundary, should_exit};
use render::{DisplayMessage, Theme, centered_rect, display_message_lines, main_layout};
use worker::{PromptEvent, PromptJob, SessionRuntimeGuard, spawn_prompt_worker};

pub fn run(script: Option<PathBuf>, prompt: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;

    // Validate config at startup — show clear errors before TUI initializes.
    for error in config.validate() {
        eprintln!("openrust config: {}", error);
    }

    let (provider_name, model) = config
        .resolve_provider_model()
        .unwrap_or_else(|| ("unconfigured".to_string(), "unconfigured".to_string()));
    let (llm, system) = config
        .get_provider(&provider_name)
        .and_then(|resolved| {
            provider::create_provider(&resolved).map(|llm| {
                let system = crate::core::system_prompt::SystemPrompt::from_config(
                    &config,
                    &resolved,
                    config.mode.clone(),
                )
                .ok()
                .map(|prompt| prompt.render())
                .unwrap_or_default();
                (Arc::from(llm), system)
            })
        })
        .map(|(llm, system)| (Some(llm), system))
        .unwrap_or((None, String::new()));

    let script_lines = script
        .as_ref()
        .map(|path| load_script(path))
        .transpose()?
        .unwrap_or_default();

    let mut session = SessionView::new(provider_name, model, system, llm, config, cwd);
    session.bootstrap(prompt, script_lines)?;
    session.run()
}

/// A single sub-question awaiting an answer in the interactive prompt.
struct QuestionItem {
    header: String,
    question: String,
    options: Vec<(String, String)>,
    multiple: bool,
}

/// Interactive state for a pending `question` tool call. The worker thread is
/// blocked on `responder` until every sub-question is answered (or cancelled).
struct PendingQuestion {
    responder: std::sync::mpsc::Sender<Vec<String>>,
    items: Vec<QuestionItem>,
    current: usize,
    selected: usize,
    picked: Vec<usize>,
    answers: Vec<String>,
    typing: Option<String>,
}

impl PendingQuestion {
    /// Build from an `AskRequest`. Returns `None` if the payload has no valid questions.
    fn from_request(request: AskRequest) -> Option<Self> {
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

    fn item(&self) -> &QuestionItem {
        &self.items[self.current]
    }

    /// Total selectable rows: options plus the trailing "type your own" entry.
    fn row_count(&self) -> usize {
        self.item().options.len() + 1
    }

    fn custom_index(&self) -> usize {
        self.item().options.len()
    }

    fn next(&mut self) {
        let count = self.row_count();
        self.selected = (self.selected + 1) % count;
    }

    fn previous(&mut self) {
        let count = self.row_count();
        self.selected = if self.selected == 0 {
            count - 1
        } else {
            self.selected - 1
        };
    }

    fn toggle_pick(&mut self) {
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

    /// Advance to the next sub-question, recording `answer`. Returns `true` when
    /// all questions are answered (caller should send `answers` and clear state).
    fn record(&mut self, answer: String) -> bool {
        self.answers.push(answer);
        self.current += 1;
        self.selected = 0;
        self.picked.clear();
        self.typing = None;
        self.current >= self.items.len()
    }

    /// Handle Enter on the current selection. Returns finalized answers when done.
    fn confirm(&mut self) -> Option<Vec<String>> {
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

    /// Commit a typed custom answer. Returns finalized answers when done.
    fn commit_custom(&mut self) -> Option<Vec<String>> {
        let text = self.typing.take().unwrap_or_default();
        if self.record(text) {
            return Some(std::mem::take(&mut self.answers));
        }
        None
    }
}

/// Interactive state for a pending permission confirmation. The worker thread is
/// blocked on `responder` until the user chooses allow/deny.
struct PendingPermission {
    responder: std::sync::mpsc::Sender<bool>,
    tool: String,
    detail: String,
    allow: bool,
}

struct PendingTextInput {
    title: String,
    description: String,
    value: String,
    secret: bool,
    submit: fn(&mut SessionView, &str),
}

#[derive(Clone)]
struct SessionRenderLine {
    line: Line<'static>,
    text: String,
    tool_message_index: Option<usize>,
}

#[derive(Default)]
struct ConnectDraft {
    provider: String,
    base_url: String,
    model: String,
    wire_model: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ViewMode {
    Home,
    Session,
}

struct SessionView {
    provider_name: String,
    model: String,
    system: String,
    llm: Option<Arc<dyn provider::LlmProvider>>,
    config: Config,
    cwd: PathBuf,
    session_id: String,
    store: Option<SessionStore>,
    messages: Vec<Message>,
    display: Vec<DisplayMessage>,
    transcript: Vec<String>,
    pending_prompts: VecDeque<String>,
    interactive: bool,
    view_mode: ViewMode,
    input: String,
    cursor_index: usize,
    session_scroll: usize,
    status: String,
    ai_running: bool,
    cache_hits: usize,
    cache_total: usize,
    theme: Theme,
    thinking_mode: ThinkingMode,
    reasoning_effort: Option<String>,
    dialog: Option<Dialog>,
    toast: Option<String>,
    toast_deadline: Option<Instant>,
    pending_question: Option<PendingQuestion>,
    pending_permission: Option<PendingPermission>,
    pending_text_input: Option<PendingTextInput>,
    connect_draft: Option<ConnectDraft>,
    sidebar: Option<sidebar::FileTree>,
    sidebar_visible: bool,
    diff_visible: bool,
    last_diff: Option<(String, String, String)>,
    prompt_job: Option<PromptJob>,
    shutdown: Arc<AtomicBool>,
    assistant_preview: String,
    thinking_preview: String,
    session_render_lines: RefCell<Vec<SessionRenderLine>>,
    session_selection: Option<(usize, usize)>,
    mouse_down_row: Option<usize>,
    mouse_dragging: bool,
    session_area_top: Cell<u16>,
    session_area_height: Cell<u16>,
    dialog_area: Cell<Option<ratatui::layout::Rect>>,
}

impl SessionView {
    fn new(
        provider_name: String,
        model: String,
        system: String,
        llm: Option<Arc<dyn provider::LlmProvider>>,
        config: Config,
        cwd: PathBuf,
    ) -> Self {
        let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
        let session_id = format!("session-{}", now_micros());
        let db_path = crate::core::platform::PlatformPaths::detect().sessions_db_path();
        tracing::info!(path = %db_path.display(), "SessionStore opening");
        let (store, status) = match SessionStore::open() {
            Ok(s) => {
                let count = s.list_sessions().map(|v| v.len()).unwrap_or(0);
                tracing::info!(count, "SessionStore opened");
                (Some(s), String::new())
            }
            Err(e) => {
                let msg = format!("DB locked: {e}");
                tracing::error!(err = %e, "SessionStore open failed");
                (None, msg)
            }
        };
        if let Some(store) = &store {
            let _ = store.ensure_session(&session_id, None);
            let cleanup_age = 30 * 24 * 60 * 60;
            match store.cleanup_old_sessions(cleanup_age) {
                Ok(0) => {}
                Ok(n) => tracing::info!(count = n, max_age_days = 30, "cleaned old sessions"),
                Err(e) => tracing::error!(err = %e, "cleanup_old_sessions failed"),
            }
        }
        Self {
            provider_name,
            model,
            system,
            llm,
            config,
            cwd,
            session_id,
            store,
            messages: Vec::new(),
            display: Vec::new(),
            transcript: Vec::new(),
            pending_prompts: VecDeque::new(),
            interactive,
            view_mode: ViewMode::Home,
            input: String::new(),
            cursor_index: 0,
            session_scroll: 0,
            status,
            ai_running: false,
            cache_hits: 0,
            cache_total: 0,
            theme: Theme::dark(),
            thinking_mode: ThinkingMode::Show,
            reasoning_effort: None,
            dialog: None,
            toast: None,
            toast_deadline: None,
            pending_question: None,
            pending_permission: None,
            pending_text_input: None,
            connect_draft: None,
            sidebar: None,
            sidebar_visible: false,
            diff_visible: false,
            last_diff: None,
            prompt_job: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            assistant_preview: String::new(),
            thinking_preview: String::new(),
            session_render_lines: RefCell::new(Vec::new()),
            session_selection: None,
            mouse_down_row: None,
            mouse_dragging: false,
            session_area_top: Cell::new(0),
            session_area_height: Cell::new(24),
            dialog_area: Cell::new(None),
        }
    }

    fn bootstrap(
        &mut self,
        prompt: Option<String>,
        script_lines: Vec<String>,
    ) -> anyhow::Result<()> {
        let should_start_session = prompt.is_some() || !script_lines.is_empty();
        if let Some(prompt) = prompt {
            self.enqueue(prompt);
        }

        for line in script_lines {
            self.enqueue(line);
        }

        if should_start_session {
            self.view_mode = ViewMode::Session;
        }

        Ok(())
    }

    fn run(&mut self) -> anyhow::Result<()> {
        if !self.interactive {
            return self.run_headless();
        }

        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            EnableMouseCapture
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        let run_result = self.run_inner(&mut terminal);
        let guard = SessionRuntimeGuard::new(terminal, Arc::clone(&self.shutdown));
        drop(guard);

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

    fn run_inner(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        let pending = std::mem::take(&mut self.transcript);
        for prompt in pending {
            self.handle_interactive_prompt(terminal, &prompt)?;
        }

        loop {
            self.pump_prompt_job(terminal)?;
            self.poll_ask_request();
            self.poll_permission_request();
            if self.sidebar_visible {
                if let Some(tree) = &mut self.sidebar {
                    tree.poll_refresh();
                }
            }
            self.maybe_start_next_prompt(terminal)?;
            self.status = self.default_status_message();
            if let Some(deadline) = self.toast_deadline {
                if Instant::now() >= deadline {
                    self.toast = None;
                    self.toast_deadline = None;
                }
            }
            self.render_terminal(terminal)?;
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }

            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if self.pending_permission.is_some() {
                        self.handle_permission_key(key);
                        continue;
                    }
                    if self.pending_question.is_some() {
                        self.handle_question_key(key);
                        continue;
                    }
                    if self.pending_text_input.is_some() {
                        self.handle_text_input_key(key);
                        continue;
                    }
                    if self.handle_global_copy_key(key)? {
                        continue;
                    }
                    if should_exit(&key) {
                        break;
                    }

                    if self.view_mode == ViewMode::Home
                        && self.dialog.is_none()
                        && self.handle_home_key(terminal, key)?
                    {
                        continue;
                    }

                    match key.code {
                        KeyCode::Esc if self.dialog.is_some() => {
                            self.dialog = None;
                        }
                        KeyCode::Up if self.dialog.is_some() => {
                            if let Some(dialog) = &mut self.dialog {
                                dialog.previous();
                            }
                        }
                        KeyCode::Down if self.dialog.is_some() => {
                            if let Some(dialog) = &mut self.dialog {
                                dialog.next();
                            }
                        }
                        KeyCode::Up if self.view_mode == ViewMode::Session => {
                            self.scroll_session_up(3);
                        }
                        KeyCode::Down if self.view_mode == ViewMode::Session => {
                            self.scroll_session_down(3);
                        }
                        KeyCode::Enter => {
                            if self.dialog.is_some() {
                                let is_slash = matches!(
                                    self.dialog.as_ref().map(|d| &d.kind),
                                    Some(DialogKind::SlashHelp)
                                );
                                self.submit_dialog_selection();
                                if !is_slash {
                                    continue;
                                }
                            }
                            let input = self.input.trim().to_string();
                            self.input.clear();
                            self.cursor_index = 0;
                            if input.is_empty() {
                                continue;
                            }
                            if is_exit_command(&input) {
                                return Ok(());
                            }
                            if self.handle_slash_command(&input) {
                                continue;
                            }
                            self.enqueue_or_run_prompt(terminal, input)?;
                        }
                        KeyCode::Backspace => {
                            if self.cursor_index > 0 {
                                let index = prev_char_boundary(&self.input, self.cursor_index);
                                self.input.drain(index..self.cursor_index);
                                self.cursor_index = index;
                            }
                            self.sync_slash_help();
                        }
                        KeyCode::Delete => {
                            if self.cursor_index < self.input.len() {
                                let next = next_char_boundary(&self.input, self.cursor_index);
                                self.input.drain(self.cursor_index..next);
                            }
                            self.sync_slash_help();
                        }
                        KeyCode::Left => {
                            self.cursor_index = prev_char_boundary(&self.input, self.cursor_index);
                        }
                        KeyCode::Right => {
                            self.cursor_index = next_char_boundary(&self.input, self.cursor_index);
                        }
                        KeyCode::Home => {
                            self.cursor_index = 0;
                        }
                        KeyCode::End => {
                            self.cursor_index = self.input.len();
                        }
                        KeyCode::PageUp => {
                            self.scroll_session_up(8);
                        }
                        KeyCode::PageDown => {
                            self.scroll_session_down(8);
                        }
                        KeyCode::Char('e')
                            if key.modifiers.contains(KeyModifiers::CONTROL) =>
                        {
                            self.toggle_tool_collapse();
                        }
                        KeyCode::Char(ch) => {
                            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                                self.input.insert(self.cursor_index, ch);
                                self.cursor_index += ch.len_utf8();
                                self.sync_slash_help();
                            }
                        }
                        KeyCode::Tab => {
                            self.cycle_agent();
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => self.handle_mouse_event(mouse, terminal)?,
                _ => {}
            }
        }

        Ok(())
    }

    fn open_thinking_dialog(&mut self, mode: ThinkingModeCommand) {
        self.dialog = Some(Dialog::new(
            DialogKind::Thinking,
            "Thinking",
            "选择 thinking 内容是否显示在会话流中。",
            vec![
                DialogOption::new(
                    "show",
                    "Show",
                    "显示 thinking 内容，使用独立 thinking theme。",
                ),
                DialogOption::new("hide", "Hide", "隐藏 thinking 内容，只保留普通输出。"),
                DialogOption::new("toggle", "Toggle", "在 show / hide 之间切换。"),
            ],
            match mode {
                ThinkingModeCommand::Show => 0,
                ThinkingModeCommand::Hide => 1,
                ThinkingModeCommand::Toggle => 2,
            },
        ));
        self.status = "dialog: thinking".to_string();
    }

    fn open_session_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("new") => {
                self.create_session();
            }
            Some("switch") | Some("use") => {
                if let Some(target) = args.get(1) {
                    self.switch_session(target);
                } else {
                    self.note("usage: /session switch <id|number>".to_string());
                }
            }
            Some("delete") | Some("rm") => {
                if let Some(target) = args.get(1) {
                    self.delete_session(target);
                } else {
                    self.note("usage: /session delete <id|number>".to_string());
                }
            }
            _ => {
                let Some(store) = self.store.clone() else {
                    tracing::warn!("open_session_dialog: store is None");
                    self.note("session store unavailable".to_string());
                    return;
                };
                let Ok(sessions) = store.list_sessions() else {
                    tracing::error!("open_session_dialog: list_sessions failed");
                    self.note("failed to list sessions".to_string());
                    return;
                };
                tracing::info!(count = sessions.len(), "open_session_dialog");
                let mut options = vec![DialogOption::new(
                    "__new__",
                    "New session",
                    "创建一个新的空会话。",
                )];
                options.extend(
                    sessions
                        .iter()
                        .take(20)
                        .enumerate()
                        .map(|(index, session)| {
                            let title = session.title.as_deref().unwrap_or(&session.id);
                            let agent = session.agent.as_deref().unwrap_or("(no agent)");
                            let marker = if session.id == self.session_id {
                                "● "
                            } else {
                                ""
                            };
                            let summary_preview = session.summary.as_deref()
                                .map(|s| {
                                    let short = &s[..s.len().min(30)];
                                    format!(" · {}", short)
                                })
                                .unwrap_or_default();
                            let mut desc = format!(
                                "{} msg · {} · {}",
                                session.message_count, agent, summary_preview
                            );
                            if desc.len() > 60 {
                                desc.truncate(57);
                                desc.push_str("...");
                            }
                            DialogOption::new(
                                session.id.clone(),
                                format!("{}{}. {}", marker, index + 1, title),
                                desc,
                            )
                        }),
                );
                self.dialog = Some(Dialog::new(
                    DialogKind::Session,
                    "Sessions",
                    "选择要切换的会话。",
                    options,
                    0,
                ));
                self.status = "dialog: sessions".to_string();
            }
        }
    }

    fn open_agent_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("use") => {
                if let Some(agent_id) = args.get(1) {
                    self.switch_agent(agent_id);
                } else {
                    self.note("usage: /agent use <id|number>".to_string());
                }
            }
            _ => {
                let Some(agents) = agent::load_agents(&self.cwd).ok() else {
                    self.note("agent registry unavailable".to_string());
                    return;
                };
                let mut options = vec![DialogOption::new(
                    "__default__",
                    "Default agent",
                    "Clear the session agent and use the default workflow.",
                )];
                options.extend(agents.iter().enumerate().map(|(index, info)| {
                    let active =
                        if self.current_session_agent().as_deref() == Some(info.id.as_str()) {
                            "● "
                        } else {
                            ""
                        };
                    DialogOption::new(
                        info.id.clone(),
                        format!("{}{}. {}", active, index + 1, info.title),
                        info.description.clone(),
                    )
                }));
                self.dialog = Some(Dialog::new(
                    DialogKind::Agent,
                    "Agents",
                    "选择当前会话使用的 agent。",
                    options,
                    0,
                ));
                self.status = "dialog: agents".to_string();
            }
        }
    }

    fn open_task_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("add") => {
                let Some(title) = args.get(1) else {
                    self.note("usage: /task add <title> [status]".to_string());
                    return;
                };
                let status = args
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| "pending".to_string());
                let Some(store) = &self.store else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                let id = format!("task-{}", now_micros());
                let agent = self.current_session_agent();
                match store.upsert_task(&self.session_id, &id, agent, title.clone(), status.clone())
                {
                    Ok(task) => self.note(format!("task added: {} [{}]", task.title, task.status)),
                    Err(err) => self.note(format!("failed to add task: {}", err)),
                }
            }
            Some("done") => {
                let Some(id) = args.get(1) else {
                    self.note("usage: /task done <id>".to_string());
                    return;
                };
                let Some(store) = &self.store else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                match store.update_task_status(&self.session_id, id, "completed") {
                    Ok(Some(task)) => self.note(format!("task completed: {}", task.id)),
                    Ok(None) => self.note(format!("task not found: {}", id)),
                    Err(err) => self.note(format!("failed to update task: {}", err)),
                }
            }
            _ => {
                let Some(store) = &self.store else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                let Ok(tasks) = store.list_tasks(&self.session_id) else {
                    self.note("failed to list tasks".to_string());
                    return;
                };
                let mut options = vec![DialogOption::new(
                    "__new__",
                    "New task",
                    "Create a new task for the current session.",
                )];
                options.extend(tasks.iter().enumerate().map(|(index, task)| {
                    let agent = task.agent.as_deref().unwrap_or("(no agent)");
                    DialogOption::new(
                        task.id.clone(),
                        format!("{}. {}", index + 1, task.title),
                        format!("{} · agent: {}", task.status, agent),
                    )
                }));
                self.dialog = Some(Dialog::new(
                    DialogKind::Task,
                    "Tasks",
                    "查看或更新当前会话的任务。",
                    options,
                    0,
                ));
                self.status = "dialog: tasks".to_string();
            }
        }
    }

    fn open_connect_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("use") => {
                if let Some(provider) = args.get(1) {
                    self.switch_provider(provider);
                } else {
                    self.note("usage: /connect use <provider>".to_string());
                }
            }
            Some("add") if args.len() >= 4 => self.add_provider(&args),
            Some("add") => self.start_connect_wizard(),
            Some("key") => {
                let (Some(provider), Some(api_key)) = (args.get(1), args.get(2)) else {
                    self.note("usage: /connect key <provider> <api-key>".to_string());
                    return;
                };
                match Vault::save(provider, api_key) {
                    Ok(()) => self.note(format!(
                        "saved API key for {}; run /connect verify {}",
                        provider, provider
                    )),
                    Err(err) => self.note(format!("failed to save API key: {}", err)),
                }
                self.reload_config();
            }
            Some("verify") => {
                let provider = args
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| self.provider_name.clone());
                self.verify_provider(&provider);
            }
            _ => {
                let mut providers = self.config.provider.keys().cloned().collect::<Vec<_>>();
                providers.sort();
                let mut options = providers
                    .iter()
                    .map(|provider| {
                        let active = if provider == &self.provider_name {
                            "● "
                        } else {
                            ""
                        };
                        DialogOption::new(
                            provider.clone(),
                            format!("{}{}", active, provider),
                            "切换到这个 provider。",
                        )
                    })
                    .collect::<Vec<_>>();
                options.push(DialogOption::new(
                    "__add__",
                    "Add custom provider",
                    "交互式输入 provider、base URL、model 和 API key。",
                ));
                options.push(DialogOption::new(
                    "__verify__",
                    "Verify current provider",
                    "验证当前 provider、API key 和模型通路。",
                ));
                self.dialog = Some(Dialog::new(
                    DialogKind::Provider,
                    "Connect",
                    "选择 provider，或验证当前通路。自定义 provider 使用 /connect add。",
                    options,
                    0,
                ));
                self.status = "dialog: connect".to_string();
            }
        }
    }

    fn open_models_dialog(&mut self, args: Vec<String>) {
        match args.first().map(String::as_str) {
            Some("use") => {
                if let Some(spec) = args.get(1) {
                    self.switch_model(spec);
                } else {
                    self.note("usage: /models use <provider/model>".to_string());
                }
            }
            Some("thinking") | Some("effort") => {
                self.open_reasoning_dialog(args.get(1).map(String::as_str))
            }
            Some(spec) if spec.contains('/') => self.switch_model(spec),
            _ => {
                let current = self.config.model.as_deref().unwrap_or("");
                let mut options = self
                    .config
                    .provider
                    .iter()
                    .flat_map(|(provider, cfg)| {
                        let mut models = cfg.models.keys().cloned().collect::<Vec<_>>();
                        models.sort();
                        models
                            .into_iter()
                            .map(|model| {
                                let spec = format!("{}/{}", provider, model);
                                let active = if spec == current { "● " } else { "" };
                                DialogOption::new(
                                    spec.clone(),
                                    format!("{}{}", active, spec),
                                    "切换到这个已注册模型。",
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                options.push(DialogOption::new(
                    "__reasoning__",
                    "Thinking effort",
                    "设置模型 reasoning_effort：low / medium / high / off。",
                ));
                self.dialog = Some(Dialog::new(
                    DialogKind::Model,
                    "Models",
                    "选择已注册模型，或进入 thinking effort 设置。",
                    options,
                    0,
                ));
                self.status = "dialog: models".to_string();
            }
        }
    }

    fn open_reasoning_dialog(&mut self, selected: Option<&str>) {
        let selected = match selected.or(self.reasoning_effort.as_deref()) {
            Some("low") => 0,
            Some("medium") => 1,
            Some("high") => 2,
            _ => 3,
        };
        self.dialog = Some(Dialog::new(
            DialogKind::ReasoningEffort,
            "Thinking Effort",
            "选择发送给模型的 reasoning_effort。",
            vec![
                DialogOption::new("low", "Low", "较低思考强度。"),
                DialogOption::new("medium", "Medium", "默认思考强度。"),
                DialogOption::new("high", "High", "更强思考强度。"),
                DialogOption::new("off", "Off", "不发送 reasoning_effort 参数。"),
            ],
            selected,
        ));
        self.status = "dialog: thinking effort".to_string();
    }

    fn submit_dialog_selection(&mut self) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match dialog.kind {
            DialogKind::Thinking => {
                let mode = match dialog.selected_value() {
                    Some("show") => ThinkingModeCommand::Show,
                    Some("hide") => ThinkingModeCommand::Hide,
                    _ => ThinkingModeCommand::Toggle,
                };
                self.thinking_mode = mode.apply(self.thinking_mode);
                self.note(format!("thinking: {}", self.thinking_mode.label()));
            }
            DialogKind::Session => match dialog.selected_value() {
                Some("__new__") => self.create_session(),
                Some(value) => self.switch_session(value),
                None => {}
            },
            DialogKind::Agent => match dialog.selected_value() {
                Some("__default__") => self.set_session_agent(None),
                Some(value) => self.set_session_agent(Some(value.to_string())),
                None => {}
            },
            DialogKind::Task => match dialog.selected_value() {
                Some("__new__") => {
                    self.note("use /task add <title> [status] to create a task".to_string())
                }
                Some(value) => self.note(format!("task selected: {}", value)),
                None => {}
            },
            DialogKind::Provider => match dialog.selected_value() {
                Some("__add__") => self.start_connect_wizard(),
                Some("__verify__") => self.verify_provider(&self.provider_name.clone()),
                Some(value) => self.switch_provider(value),
                None => {}
            },
            DialogKind::Model => match dialog.selected_value() {
                Some("__reasoning__") => self.open_reasoning_dialog(None),
                Some(value) => self.switch_model(value),
                None => {}
            },
            DialogKind::ReasoningEffort => {
                if let Some(value) = dialog.selected_value() {
                    self.set_reasoning_effort(value);
                }
            }
            DialogKind::SlashHelp => {
                if let Some(value) = dialog.selected_value() {
                    self.input = value.to_string();
                    self.cursor_index = self.input.len();
                    self.dialog = None;
                }
            }
        }
    }

    fn poll_permission_request(&mut self) {
        if self.pending_permission.is_some() {
            return;
        }
        let request = {
            let Some(job) = &self.prompt_job else {
                return;
            };
            match job.permission_receiver.try_recv() {
                Ok(request) => request,
                Err(_) => return,
            }
        };
        self.pending_permission = Some(PendingPermission {
            responder: request.responder,
            tool: request.tool,
            detail: request.detail,
            allow: false,
        });
        self.status = "permission: awaiting your decision".to_string();
    }

    fn handle_permission_key(&mut self, key: event::KeyEvent) {
        let decision = {
            let Some(permission) = self.pending_permission.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Char('a') | KeyCode::Char('y') => Some(true),
                KeyCode::Char('d') | KeyCode::Char('n') | KeyCode::Esc => Some(false),
                KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                    permission.allow = !permission.allow;
                    None
                }
                KeyCode::Enter => Some(permission.allow),
                _ => None,
            }
        };
        if let Some(allow) = decision {
            if let Some(permission) = self.pending_permission.take() {
                let _ = permission.responder.send(allow);
            }
            self.status = if allow {
                "permission: allowed".to_string()
            } else {
                "permission: denied".to_string()
            };
        }
    }

    fn handle_text_input_key(&mut self, key: event::KeyEvent) {
        enum Outcome {
            None,
            Cancel,
            Submit(String, fn(&mut SessionView, &str)),
        }

        let outcome = {
            let Some(input) = self.pending_text_input.as_mut() else {
                return;
            };
            match key.code {
                KeyCode::Esc => Outcome::Cancel,
                KeyCode::Enter => Outcome::Submit(input.value.clone(), input.submit),
                KeyCode::Backspace => {
                    input.value.pop();
                    Outcome::None
                }
                KeyCode::Char(ch) => {
                    input.value.push(ch);
                    Outcome::None
                }
                _ => Outcome::None,
            }
        };

        match outcome {
            Outcome::None => {}
            Outcome::Cancel => {
                self.pending_text_input = None;
                self.note("input cancelled".to_string());
            }
            Outcome::Submit(value, submit) => {
                self.pending_text_input = None;
                submit(self, &value);
            }
        }
    }

    fn permission_widget(&self, permission: &PendingPermission) -> Paragraph<'static> {
        let theme = &self.theme;
        let allow_style = if permission.allow {
            theme.dialog_selected_style().add_modifier(Modifier::BOLD)
        } else {
            theme.dialog_style()
        };
        let deny_style = if permission.allow {
            theme.dialog_style()
        } else {
            theme.dialog_selected_style().add_modifier(Modifier::BOLD)
        };
        let lines = vec![
            Line::from(Span::styled(
                format!("{}: {}", permission.tool, permission.detail),
                theme.muted_style(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [A] Allow  ", allow_style),
                Span::styled("  [D] Deny  ", deny_style),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "A/Y 允许 · D/N/Esc 拒绝 · ←/→ 切换 · Enter 确认",
                theme.muted_style(),
            )),
        ];
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Permission ")
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .title_style(theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(theme.dialog_border_style()),
            )
            .style(theme.dialog_style())
            .wrap(Wrap { trim: false })
    }

    fn text_input_widget(&self, input: &PendingTextInput) -> Paragraph<'static> {
        let theme = &self.theme;
        let value = if input.secret {
            "*".repeat(input.value.chars().count())
        } else {
            input.value.clone()
        };
        let lines = vec![
            Line::from(Span::styled(input.description.clone(), theme.muted_style())),
            Line::from(""),
            Line::from(Span::styled(
                format!("> {}", value),
                theme.dialog_selected_style(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "输入后 Enter 保存 · Esc 取消",
                theme.muted_style(),
            )),
        ];
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(format!(" {} ", input.title))
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .title_style(theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(theme.dialog_border_style()),
            )
            .style(theme.dialog_style())
            .wrap(Wrap { trim: false })
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_visible = !self.sidebar_visible;
        if self.sidebar_visible && self.sidebar.is_none() {
            self.sidebar = Some(sidebar::FileTree::new(self.cwd.clone()));
        }
        self.note(if self.sidebar_visible {
            "sidebar: on".to_string()
        } else {
            "sidebar: off".to_string()
        });
    }

    fn toggle_diff(&mut self) {
        if self.last_diff.is_none() {
            self.note("no recent edit to diff".to_string());
            return;
        }
        self.diff_visible = !self.diff_visible;
    }

    fn toggle_tool_collapse(&mut self) {
        if let Some(msg) = self.display.iter_mut().rev().find(|m| m.role == "tool") {
            msg.collapsed = !msg.collapsed;
        }
    }

    /// Capture the last edit/write as a diff for the `/diff` viewer.
    fn capture_diff(&mut self, name: &str, args: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(args) else {
            return;
        };
        let path = value
            .get("filePath")
            .or_else(|| value.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        match name {
            "edit" => {
                let before = value
                    .get("oldString")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let after = value
                    .get("newString")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.last_diff = Some((format!("edit {}", path), before, after));
            }
            "write" => {
                let after = value
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.last_diff = Some((format!("write {}", path), String::new(), after));
            }
            _ => {}
        }
    }

    fn poll_ask_request(&mut self) {
        if self.pending_question.is_some() {
            return;
        }
        let request = {
            let Some(job) = &self.prompt_job else {
                return;
            };
            match job.ask_receiver.try_recv() {
                Ok(request) => request,
                Err(_) => return,
            }
        };
        self.pending_question = PendingQuestion::from_request(request);
        if self.pending_question.is_some() {
            self.status = "question: awaiting your answer".to_string();
        }
    }

    fn handle_question_key(&mut self, key: event::KeyEvent) {
        enum Outcome {
            None,
            Cancel,
            Done(Vec<String>),
        }
        let outcome = {
            let Some(q) = self.pending_question.as_mut() else {
                return;
            };
            if q.typing.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        q.typing = None;
                        Outcome::None
                    }
                    KeyCode::Enter => match q.commit_custom() {
                        Some(answers) => Outcome::Done(answers),
                        None => Outcome::None,
                    },
                    KeyCode::Backspace => {
                        if let Some(buffer) = q.typing.as_mut() {
                            buffer.pop();
                        }
                        Outcome::None
                    }
                    KeyCode::Char(ch) => {
                        if let Some(buffer) = q.typing.as_mut() {
                            buffer.push(ch);
                        }
                        Outcome::None
                    }
                    _ => Outcome::None,
                }
            } else {
                match key.code {
                    KeyCode::Esc => Outcome::Cancel,
                    KeyCode::Up => {
                        q.previous();
                        Outcome::None
                    }
                    KeyCode::Down => {
                        q.next();
                        Outcome::None
                    }
                    KeyCode::Char(' ') => {
                        q.toggle_pick();
                        Outcome::None
                    }
                    KeyCode::Enter => match q.confirm() {
                        Some(answers) => Outcome::Done(answers),
                        None => Outcome::None,
                    },
                    _ => Outcome::None,
                }
            }
        };
        match outcome {
            Outcome::None => {}
            Outcome::Cancel => {
                if let Some(q) = self.pending_question.take() {
                    let _ = q.responder.send(vec!["(cancelled)".to_string()]);
                }
                self.note("question cancelled".to_string());
            }
            Outcome::Done(answers) => {
                if let Some(q) = self.pending_question.take() {
                    let _ = q.responder.send(answers);
                }
                self.status = "answer sent".to_string();
            }
        }
    }

    fn question_widget(&self, q: &PendingQuestion) -> Paragraph<'static> {
        let theme = &self.theme;
        let item = q.item();
        let mut lines = vec![
            Line::from(Span::styled(item.question.clone(), theme.muted_style())),
            Line::from(""),
        ];
        for (index, (label, description)) in item.options.iter().enumerate() {
            let selected = index == q.selected && q.typing.is_none();
            let marker = if selected { "› " } else { "  " };
            let check = if item.multiple {
                if q.picked.contains(&index) {
                    "[x] "
                } else {
                    "[ ] "
                }
            } else {
                ""
            };
            let style = if selected {
                theme.dialog_selected_style().add_modifier(Modifier::BOLD)
            } else {
                theme.dialog_style()
            };
            lines.push(Line::from(Span::styled(
                format!("{}{}{}", marker, check, label),
                style,
            )));
            if !description.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("      {}", description),
                    theme.muted_style(),
                )));
            }
        }
        let custom_selected = q.selected == q.custom_index() && q.typing.is_none();
        let custom_marker = if custom_selected { "› " } else { "  " };
        let custom_style = if custom_selected {
            theme.dialog_selected_style().add_modifier(Modifier::BOLD)
        } else {
            theme.dialog_style()
        };
        lines.push(Line::from(Span::styled(
            format!("{}✎ Type your own answer", custom_marker),
            custom_style,
        )));
        if let Some(buffer) = &q.typing {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  > {}", buffer),
                theme.dialog_selected_style(),
            )));
        }
        lines.push(Line::from(""));
        let hint = if item.multiple {
            "↑/↓ 选择 · Space 多选 · Enter 确认 · Esc 取消"
        } else {
            "↑/↓ 选择 · Enter 确认 · Esc 取消"
        };
        lines.push(Line::from(Span::styled(hint, theme.muted_style())));

        let title = if item.header.is_empty() {
            format!(" Question {}/{} ", q.current + 1, q.items.len())
        } else {
            format!(" {} ({}/{}) ", item.header, q.current + 1, q.items.len())
        };
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .title_alignment(ratatui::layout::Alignment::Center)
                    .title_style(theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(theme.dialog_border_style()),
            )
            .style(theme.dialog_style())
            .wrap(Wrap { trim: false })
    }

    fn create_session(&mut self) {
        self.session_id = format!("session-{}", now_micros());
        self.messages.clear();
        self.display.clear();
        self.session_scroll = 0;
        self.view_mode = ViewMode::Session;
        if let Some(store) = &self.store {
            let agent = self.default_agent_id();
            let _ = store.ensure_session(&self.session_id, None);
            let _ = store.set_session_agent(&self.session_id, agent.clone());
        }
        self.note(format!("session: created {}", self.session_id));
    }

    fn switch_agent(&mut self, target: &str) {
        let Some(store) = &self.store else {
            self.note("session store unavailable".to_string());
            return;
        };
        let agent_id = if target == "__default__" {
            None
        } else {
            let agents = agent::load_agents(&self.cwd).unwrap_or_default();
            let resolved = target
                .parse::<usize>()
                .ok()
                .and_then(|index| {
                    agents
                        .get(index.saturating_sub(1))
                        .map(|agent| agent.id.clone())
                })
                .unwrap_or_else(|| target.to_string());
            Some(resolved)
        };
        match store.set_session_agent(&self.session_id, agent_id.clone()) {
            Ok(()) => {
                let label = agent_id.as_deref().unwrap_or("default");
                self.note(format!("agent: {}", label));
            }
            Err(err) => self.note(format!("failed to set agent: {}", err)),
        }
    }

    fn cycle_agent(&mut self) {
        let Some(store) = &self.store else {
            self.note("session store unavailable".to_string());
            return;
        };
        let loaded_agents = agent::load_agents(&self.cwd).unwrap_or_default();
        let agents = agent::visible_agents(&loaded_agents);
        if agents.is_empty() {
            self.note("no visible agents available".to_string());
            return;
        }
        let current = self.current_session_agent();
        let next = agents
            .iter()
            .position(|agent| current.as_deref() == Some(agent.id.as_str()))
            .map(|index| (index + 1) % agents.len())
            .unwrap_or(0);
        let agent_id = Some(agents[next].id.clone());
        match store.set_session_agent(&self.session_id, agent_id.clone()) {
            Ok(()) => self.note(format!(
                "agent: {}",
                agent_id.as_deref().unwrap_or("default")
            )),
            Err(err) => self.note(format!("failed to set agent: {}", err)),
        }
    }

    fn current_session_agent(&self) -> Option<String> {
        self.store
            .as_ref()
            .and_then(|store| store.get_session_agent(&self.session_id).ok().flatten())
            .or_else(|| self.default_agent_id())
    }

    fn effective_system(&self) -> String {
        let mut system = self.system.clone();
        let Some(agent_id) = self.current_session_agent() else {
            return system;
        };
        let Some(agent_system) = self.agent_system(&agent_id) else {
            return system;
        };
        system.push_str("\n\n");
        system.push_str(&agent_system);
        system
    }

    fn ensure_runtime_ready(&mut self) -> anyhow::Result<()> {
        if self.llm.is_some()
            && self.provider_name != "unconfigured"
            && self.model != "unconfigured"
        {
            return Ok(());
        }

        let (provider_name, model) = self
            .config
            .resolve_provider_model()
            .ok_or_else(|| anyhow::anyhow!("No provider configured"))?;
        let resolved = self
            .config
            .get_provider(&provider_name)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_name))?;
        let llm = provider::create_provider(&resolved)
            .ok_or_else(|| anyhow::anyhow!("Failed to create provider '{}'", provider_name))?;
        let system = crate::core::system_prompt::SystemPrompt::from_config(
            &self.config,
            &resolved,
            self.config.mode.clone(),
        )?
        .render();

        self.provider_name = provider_name;
        self.model = model;
        self.system = system;
        self.llm = Some(Arc::from(llm));
        Ok(())
    }

    fn agent_system(&self, agent_id: &str) -> Option<String> {
        let agents = agent::load_agents(&self.cwd).ok()?;
        if let Some(agent) = agents.iter().find(|item| item.id == agent_id) {
            return Some(agent.system.clone());
        }
        agent::builtin_agent_system(agent_id).map(str::to_string)
    }

    fn current_agent_max_steps(&self) -> u32 {
        let agent_id = self.current_session_agent();
        if let Some(id) = agent_id {
            if let Ok(agents) = agent::load_agents(&self.cwd) {
                if let Some(info) = agents.iter().find(|item| item.id == id) {
                    return info.max_steps;
                }
            }
        }
        50
    }

    fn current_agent_mode(&self) -> String {
        let agent_id = self.current_session_agent();
        if let Some(id) = agent_id {
            if let Ok(agents) = agent::load_agents(&self.cwd) {
                if let Some(info) = agents.iter().find(|item| item.id == id) {
                    return info.mode.clone();
                }
            }
        }
        "all".to_string()
    }

    fn current_context_window(&self) -> u64 {
        self.config.resolve_context_window()
    }

    fn default_agent_id(&self) -> Option<String> {
        let agents = agent::load_agents(&self.cwd).ok()?;
        agent::default_agent_id(&agents)
    }

    fn set_session_agent(&mut self, agent: Option<String>) {
        let Some(store) = &self.store else {
            self.note("session store unavailable".to_string());
            return;
        };
        let resolved = match agent.as_deref() {
            Some("__default__") | None => None,
            Some(value) => Some(value.to_string()),
        };
        match store.set_session_agent(&self.session_id, resolved.clone()) {
            Ok(()) => {
                let label = resolved.as_deref().unwrap_or("default");
                self.note(format!("agent: {}", label));
            }
            Err(err) => self.note(format!("failed to set agent: {}", err)),
        }
    }

    fn compact_session(&mut self, args: Vec<String>) {
        let keep = args
            .first()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(4)
            .max(2);
        let Some(store) = self.store.clone() else {
            self.note("session store unavailable".to_string());
            return;
        };
        let Ok(history) = store.effective_messages(&self.session_id) else {
            self.note("failed to load session history".to_string());
            return;
        };
        if history.len() <= keep {
            self.note("nothing to compact".to_string());
            return;
        }

        let cutoff = history.len().saturating_sub(keep);
        let older = &history[..cutoff];
        let recent = &history[cutoff..];

        if let Some(llm) = &self.llm {
            let model = self.model.clone();
            let llm_clone = Arc::clone(llm);
            let session_id = self.session_id.clone();
            let store_clone = store.clone();
            let recent_text = recent
                .iter()
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n");
            let older_text = older
                .iter()
                .map(|m| format!("{}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n");
            let cwd = self.cwd.clone();
            self.status = "compacting...".to_string();

            std::thread::spawn(move || {
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(_) => return,
                };
                let system = agent::builtin_agent_system("compaction")
                    .unwrap_or("Summarize this conversation history. Be concise.")
                    .to_string();
                let prompt = format!(
                    "Summarize the following conversation history, focusing on key decisions, \
                    files modified, and remaining tasks. Be terse.\n\n{}",
                    older_text
                );
                let result = rt.block_on(crate::tool::task::run_agent(
                    llm_clone.as_ref(),
                    &model,
                    &system,
                    "subagent",
                    5,
                    None,
                    vec![provider::Message::user(prompt)],
                    &crate::tool::ToolContext::new(cwd),
                ));
                match result {
                    Ok(summary) if !summary.trim().is_empty() => {
                        let _ = store_clone.append_compaction(
                            &session_id,
                            summary.trim().to_string(),
                            recent_text,
                        );
                    }
                    _ => {
                        let _ = store_clone.append_compaction(
                            &session_id,
                            format!("compaction summary\n{}", older_text),
                            recent_text,
                        );
                    }
                }
            });

            self.messages = store
                .effective_messages(&self.session_id)
                .unwrap_or_default()
                .iter()
                .filter(|m| {
                    m.role == "user"
                        || m.role == "assistant"
                        || m.role == "tool"
                })
                .map(|m| Message {
                    role: m.role.clone(),
                    content: MessageContent::text(m.content.clone()),
                    name: m.name.clone(),
                    tool_call_id: m.tool_call_id.clone(),
                    tool_calls: m
                        .tool_calls
                        .as_ref()
                        .and_then(|v| serde_json::from_value(v.clone()).ok()),
                })
                .collect();
            self.display.push(render::DisplayMessage::new(
                "system",
                &format!("compacting {} message(s) into checkpoint...", older.len()),
            ));
            self.note(format!(
                "compacting session history, keeping {} recent message(s)",
                recent.len()
            ));
            return;
        }

        self.note("no LLM available for AI compaction".to_string());
    }

    fn generate_title(&self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let session_id = self.session_id.clone();
        if store.get_session(&session_id).ok().flatten().and_then(|s| s.title).is_some() {
            return;
        }
        let Some(llm) = &self.llm else {
            return;
        };
        let model = self.model.clone();
        let llm_clone = Arc::clone(llm);
        let first_user = store
            .get_messages(&session_id)
            .ok()
            .and_then(|msgs| msgs.into_iter().find(|m| m.role == "user"))
            .map(|m| m.content);
        let Some(user_text) = first_user else {
            return;
        };
        let cwd = self.cwd.clone();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            let system = agent::builtin_agent_system("title")
                .unwrap_or("Generate a concise title.")
                .to_string();
            let result = rt.block_on(crate::tool::task::run_agent(
                llm_clone.as_ref(),
                &model,
                &system,
                "subagent",
                3,
                None,
                vec![provider::Message::user(user_text)],
                &crate::tool::ToolContext::new(cwd),
            ));
            if let Ok(title) = result {
                let title = title.trim().chars().take(50).collect::<String>();
                if !title.is_empty() {
                    let _ = store.set_title(&session_id, title);
                }
            }
        });
    }

    fn generate_summary(&self) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let session_id = self.session_id.clone();
        let Some(llm) = &self.llm else {
            return;
        };
        let model = self.model.clone();
        let llm_clone = Arc::clone(llm);
        let history = store.get_messages(&session_id).unwrap_or_default();
        if history.len() < 2 {
            return;
        }
        let conversation = history
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        let cwd = self.cwd.clone();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(_) => return,
            };
            let system = agent::builtin_agent_system("summary")
                .unwrap_or("Summarize what was done.")
                .to_string();
            let result = rt.block_on(crate::tool::task::run_agent(
                llm_clone.as_ref(),
                &model,
                &system,
                "subagent",
                3,
                None,
                vec![provider::Message::user(conversation)],
                &crate::tool::ToolContext::new(cwd),
            ));
            if let Ok(summary) = result {
                let summary = summary.trim().to_string();
                if !summary.is_empty() {
                    let _ = store.set_summary(&session_id, summary);
                }
            }
        });
    }

    fn switch_session(&mut self, target: &str) {
        let Some(store) = self.store.clone() else {
            self.note("session store unavailable".to_string());
            return;
        };
        let id = target
            .parse::<usize>()
            .ok()
            .and_then(|index| {
                store
                    .list_sessions()
                    .ok()?
                    .get(index.saturating_sub(1))
                    .map(|s| s.id.clone())
            })
            .unwrap_or_else(|| target.to_string());
        let Ok(Some(_)) = store.get_session(&id) else {
            self.note(format!("session not found: {}", target));
            return;
        };
        let Ok(history) = store.effective_messages(&id) else {
            self.note(format!("failed to load session: {}", id));
            return;
        };
        self.session_id = id.clone();
        self.session_scroll = 0;
        self.view_mode = ViewMode::Session;
        self.messages = history
            .iter()
            .filter(|message| {
                message.role == "user" || message.role == "assistant" || message.role == "tool"
            })
            .map(|message| Message {
                role: message.role.clone(),
                content: MessageContent::text(message.content.clone()),
                name: message.name.clone(),
                tool_call_id: message.tool_call_id.clone(),
                tool_calls: message
                    .tool_calls
                    .as_ref()
                    .and_then(|value| serde_json::from_value(value.clone()).ok()),
            })
            .collect();
        self.display = history
            .iter()
            .filter(|message| {
                message.role == "user" || message.role == "assistant" || message.role == "tool"
            })
            .map(|message| render::DisplayMessage::new(&message.role, &message.content))
            .collect();
        self.note(format!("session: switched to {}", id));
    }

    fn delete_session(&mut self, target: &str) {
        let Some(store) = self.store.clone() else {
            self.note("session store unavailable".to_string());
            return;
        };
        let id = target
            .parse::<usize>()
            .ok()
            .and_then(|index| {
                store
                    .list_sessions()
                    .ok()?
                    .get(index.saturating_sub(1))
                    .map(|s| s.id.clone())
            })
            .unwrap_or_else(|| target.to_string());
        if id == self.session_id {
            self.note("cannot delete the active session".to_string());
            return;
        }
        match store.delete_session(&id) {
            Ok(()) => self.note(format!("session deleted: {}", id)),
            Err(e) => self.note(format!("delete failed: {}", e)),
        }
    }

    fn scroll_session_up(&mut self, lines: usize) {
        self.session_scroll = self.session_scroll.saturating_add(lines);
    }

    fn scroll_session_down(&mut self, lines: usize) {
        self.session_scroll = self.session_scroll.saturating_sub(lines);
    }

    fn add_provider(&mut self, args: &[String]) {
        let (Some(provider), Some(base_url), Some(model)) = (args.get(1), args.get(2), args.get(3))
        else {
            self.note("usage: /connect add <provider> <base-url> <model> [wire-model]".to_string());
            return;
        };
        let wire = args.get(4).cloned().unwrap_or_else(|| model.clone());
        self.config.provider.insert(
            provider.clone(),
            ProviderConfig {
                api_key: None,
                base_url: Some(base_url.clone()),
                models: std::collections::HashMap::from([(
                    model.clone(),
                    ModelConfig {
                        name: Some(wire),
                        variants: None,
                        limit: None,
                        options: None,
                    },
                )]),
                options: None,
            },
        );
        self.config.model = Some(format!("{}/{}", provider, model));
        match self.save_global_config() {
            Ok(()) => {
                self.reload_config();
                self.note(format!(
                    "provider added: {}; run /connect key {} <api-key>",
                    provider, provider
                ));
            }
            Err(err) => self.note(format!("failed to save provider: {}", err)),
        }
    }

    fn start_connect_wizard(&mut self) {
        self.connect_draft = Some(ConnectDraft::default());
        self.pending_text_input = Some(PendingTextInput {
            title: "Connect · provider".to_string(),
            description: "输入 provider 名称，例如 deepseek、openai、one_route。".to_string(),
            value: String::new(),
            secret: false,
            submit: SessionView::save_connect_provider_name,
        });
        self.status = "connect: provider name".to_string();
    }

    fn save_connect_provider_name(&mut self, value: &str) {
        let provider = value.trim();
        if provider.is_empty() {
            self.note("provider name cannot be empty".to_string());
            return;
        }
        let draft = self.connect_draft.get_or_insert_with(ConnectDraft::default);
        draft.provider = provider.to_string();
        self.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} base URL", provider),
            description: "输入 OpenAI-compatible base URL，例如 https://api.deepseek.com/v1 。"
                .to_string(),
            value: String::new(),
            secret: false,
            submit: SessionView::save_connect_base_url,
        });
        self.status = format!("connect: base URL for {}", provider);
    }

    fn save_connect_base_url(&mut self, value: &str) {
        let base_url = value.trim();
        if base_url.is_empty() {
            self.note("base URL cannot be empty".to_string());
            return;
        }
        let Some(draft) = self.connect_draft.as_mut() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        draft.base_url = base_url.to_string();
        self.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} model", draft.provider),
            description: "输入配置中的 model 名称，例如 deepseek-chat。".to_string(),
            value: String::new(),
            secret: false,
            submit: SessionView::save_connect_model,
        });
        self.status = format!("connect: model for {}", draft.provider);
    }

    fn save_connect_model(&mut self, value: &str) {
        let model = value.trim();
        if model.is_empty() {
            self.note("model cannot be empty".to_string());
            return;
        }
        let Some(draft) = self.connect_draft.as_mut() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        draft.model = model.to_string();
        self.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} wire model", draft.provider),
            description: "输入实际发给 API 的模型名；若与上一步相同可直接回车留空。".to_string(),
            value: String::new(),
            secret: false,
            submit: SessionView::save_connect_wire_model,
        });
        self.status = format!("connect: wire model for {}", draft.provider);
    }

    fn save_connect_wire_model(&mut self, value: &str) {
        let Some(draft) = self.connect_draft.as_mut() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        let wire_model = value.trim();
        draft.wire_model = if wire_model.is_empty() {
            None
        } else {
            Some(wire_model.to_string())
        };
        self.pending_text_input = Some(PendingTextInput {
            title: format!("Connect · {} API key", draft.provider),
            description:
                "输入 API key；若暂时没有可直接回车跳过，之后再用 /connect key <provider> <api-key>。"
                    .to_string(),
            value: String::new(),
            secret: true,
            submit: SessionView::save_connect_api_key,
        });
        self.status = format!("connect: API key for {}", draft.provider);
    }

    fn save_connect_api_key(&mut self, value: &str) {
        let Some(draft) = self.connect_draft.take() else {
            self.note("connect wizard state missing".to_string());
            return;
        };
        self.config.provider.insert(
            draft.provider.clone(),
            ProviderConfig {
                api_key: None,
                base_url: Some(draft.base_url.clone()),
                models: std::collections::HashMap::from([(
                    draft.model.clone(),
                    ModelConfig {
                        name: draft.wire_model.clone(),
                        variants: None,
                        limit: None,
                        options: None,
                    },
                )]),
                options: None,
            },
        );
        self.config.model = Some(format!("{}/{}", draft.provider, draft.model));

        match self.save_global_config() {
            Ok(()) => {
                let api_key = value.trim();
                if !api_key.is_empty() {
                    if let Err(err) = Vault::save(&draft.provider, api_key) {
                        self.reload_config();
                        self.note(format!(
                            "provider added: {}, but failed to save API key: {}",
                            draft.provider, err
                        ));
                        return;
                    }
                }
                self.reload_config();
                self.note(format!(
                    "provider configured: {} · model: {}/{}",
                    draft.provider, draft.provider, draft.model
                ));
            }
            Err(err) => self.note(format!("failed to save provider: {}", err)),
        }
    }

    fn switch_provider(&mut self, provider: &str) {
        if !self.config.provider.contains_key(provider) {
            self.note(format!("provider not configured: {}", provider));
            return;
        }
        let Some(model) = self.config.provider[provider].models.keys().next().cloned() else {
            self.note(format!("provider has no registered models: {}", provider));
            return;
        };
        self.config.model = Some(format!("{}/{}", provider, model));
        match self.save_global_config() {
            Ok(()) => {
                self.reload_config();
                self.note(format!(
                    "provider: {} · model: {}",
                    self.provider_name, self.model
                ));
            }
            Err(err) => self.note(format!("failed to save provider switch: {}", err)),
        }
    }

    fn verify_provider(&mut self, provider_name: &str) {
        let Some(resolved) = self.config.get_provider(provider_name) else {
            self.note(format!("provider not configured: {}", provider_name));
            return;
        };
        if resolved.api_key.is_none() {
            self.note(format!("provider {} has no API key", provider_name));
            return;
        }
        let Some(provider) = provider::create_provider(&resolved) else {
            self.note(format!("failed to create provider: {}", provider_name));
            return;
        };
        let Some(model) = self
            .config
            .resolve_provider_model()
            .filter(|(name, _)| name == provider_name)
            .map(|(_, model)| model)
            .or_else(|| resolved.models.keys().next().cloned())
        else {
            self.note(format!("provider {} has no model", provider_name));
            return;
        };
        let result = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(async {
                let mut stream = provider
                    .chat(
                        vec![Message {
                            role: "user".to_string(),
                            content: MessageContent::text("reply ok"),
                            name: None,
                            tool_call_id: None,
                            tool_calls: None,
                        }],
                        vec![],
                        RequestOptions {
                            model,
                            temperature: None,
                            max_tokens: Some(16),
                            system: None,
                            reasoning_effort: self.reasoning_effort.clone(),
                            tool_choice: None,
                        },
                    )
                    .await?;
                while let Some(chunk) = stream.next().await {
                    if matches!(
                        chunk?,
                        StreamChunk::TextDelta(_) | StreamChunk::Finish { .. }
                    ) {
                        return Ok::<_, anyhow::Error>(());
                    }
                }
                Ok(())
            }),
            Err(err) => Err(anyhow::Error::from(err)),
        };
        match result {
            Ok(()) => self.note(format!("provider verified: {}", provider_name)),
            Err(err) => self.note(format!("provider verify failed: {}", err)),
        }
    }

    fn switch_model(&mut self, spec: &str) {
        let (provider, model, _) = crate::core::config::parse_model_spec(spec);
        if !self
            .config
            .provider
            .get(provider)
            .is_some_and(|p| p.models.contains_key(model))
        {
            self.note(format!("model not registered: {}", spec));
            return;
        }
        self.config.model = Some(format!("{}/{}", provider, model));
        match self.save_global_config() {
            Ok(()) => {
                self.reload_config();
                self.note(format!("model: {}/{}", self.provider_name, self.model));
            }
            Err(err) => self.note(format!("failed to save model: {}", err)),
        }
    }

    fn set_reasoning_effort(&mut self, effort: &str) {
        self.reasoning_effort = match effort {
            "off" | "none" => None,
            "low" | "medium" | "high" => Some(effort.to_string()),
            _ => {
                self.note("usage: /models thinking <low|medium|high|off>".to_string());
                return;
            }
        };
        self.note(format!(
            "model thinking effort: {}",
            self.reasoning_effort.as_deref().unwrap_or("off")
        ));
    }

    fn reload_config(&mut self) {
        if let Ok(config) = Config::load(&self.cwd) {
            if let Some((provider_name, model)) = config.resolve_provider_model() {
                if let Some(resolved) = config.get_provider(&provider_name) {
                    if let Some(llm) = provider::create_provider(&resolved) {
                        self.provider_name = provider_name;
                        self.model = model;
                        if let Ok(system) = crate::core::system_prompt::SystemPrompt::from_config(
                            &config,
                            &resolved,
                            config.mode.clone(),
                        ) {
                            self.system = system.render();
                        }
                        self.llm = Some(Arc::from(llm));
                    }
                }
            }
            self.config = config;
        }
    }

    fn save_global_config(&self) -> anyhow::Result<()> {
        let path = Config::global_config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(&self.config)?)?;
        Ok(())
    }

    fn persist_message(&self, role: &str, content: &str) {
        if let Some(store) = &self.store {
            if let Err(e) = store.append_message_detail(&self.session_id, role, content, None, None, None) {
                tracing::error!(err = %e, "persist_message failed");
            }
        } else {
            tracing::warn!("persist_message skipped: store is None");
        }
    }

    fn persist_message_detail(
        &self,
        role: &str,
        content: &str,
        name: Option<String>,
        tool_call_id: Option<String>,
        tool_calls: Option<serde_json::Value>,
    ) {
        if let Some(store) = &self.store {
            if let Err(e) = store.append_message_detail(
                &self.session_id, role, content, name, tool_call_id, tool_calls,
            ) {
                tracing::error!(err = %e, "persist_message_detail failed");
            }
        } else {
            tracing::warn!("persist_message_detail skipped: store is None");
        }
    }

    fn tool_display_preview(name: &str, args: &str, result: &str) -> String {
        let args_val = serde_json::from_str::<serde_json::Value>(args).ok();
        let path = args_val
            .as_ref()
            .and_then(|v| {
                v.get("path")
                    .or_else(|| v.get("file_path"))
                    .or_else(|| v.get("filePath"))
                    .or_else(|| v.get("url"))
                    .or_else(|| v.get("pattern"))
                    .or_else(|| v.get("query"))
                    .or_else(|| v.get("message"))
                    .and_then(|v| v.as_str())
            });
        if let Some(p) = path {
            return format!("{} {}", name, p);
        }
        let preview = result.lines().next().unwrap_or("");
        let preview = &preview[..preview.len().min(60)];
        format!("{}: {}", name, preview)
    }

    fn note(&mut self, message: String) {
        self.status = message.lines().next().unwrap_or("Ready").to_string();
        self.toast = Some(message);
        self.toast_deadline = Some(Instant::now() + Duration::from_secs(4));
    }

    fn sync_slash_help(&mut self) {
        if !self.input.starts_with('/') {
            if matches!(
                self.dialog.as_ref().map(|dialog| dialog.kind),
                Some(DialogKind::SlashHelp)
            ) {
                self.dialog = None;
            }
            return;
        }

        let options = slash_options(&self.input);
        if options.is_empty() {
            self.dialog = None;
            return;
        }

        let selected = options
            .iter()
            .position(|option| option.value().starts_with(&self.input))
            .unwrap_or(0);

        self.dialog = Some(Dialog::new(
            DialogKind::SlashHelp,
            "Slash Commands",
            "输入 / 时显示可用命令提示，Enter 可补全当前命令。",
            options,
            selected,
        ));
    }

    fn render_modal_layer(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        if !self.modal_active() && !self.diff_visible {
            return;
        }

        frame.render_widget(Block::default().style(self.theme.overlay_style()), area);

        if let Some(dialog) = &self.dialog {
            let dialog_area = self.dialog_area(dialog, area);
            self.dialog_area.set(Some(dialog_area));
            frame.render_widget(Clear, dialog_area);
            self.render_dialog_panel(frame, dialog_area, dialog);
            return;
        }
        if let Some(question) = &self.pending_question {
            self.dialog_area.set(Some(centered_rect(72, 60, area)));
            let dialog_area = centered_rect(72, 60, area);
            frame.render_widget(Clear, dialog_area);
            frame.render_widget(self.question_widget(question), dialog_area);
            return;
        }
        if let Some(permission) = &self.pending_permission {
            self.dialog_area.set(Some(centered_rect(60, 32, area)));
            let dialog_area = centered_rect(60, 32, area);
            frame.render_widget(Clear, dialog_area);
            frame.render_widget(self.permission_widget(permission), dialog_area);
            return;
        }
        if let Some(input) = &self.pending_text_input {
            self.dialog_area.set(Some(centered_rect(64, 28, area)));
            let dialog_area = centered_rect(64, 28, area);
            frame.render_widget(Clear, dialog_area);
            frame.render_widget(self.text_input_widget(input), dialog_area);
            return;
        }
        if self.diff_visible {
            if let Some((title, before, after)) = &self.last_diff {
                self.dialog_area.set(Some(centered_rect(80, 70, area)));
                let dialog_area = centered_rect(80, 70, area);
                frame.render_widget(Clear, dialog_area);
                let widget = Paragraph::new(diff::render_diff(before, after, &self.theme))
                    .style(self.theme.dialog_style())
                    .block(
                        Block::default()
                            .title(format!(" diff: {} ", title))
                            .title_alignment(ratatui::layout::Alignment::Center)
                            .title_style(self.theme.title_style())
                            .borders(Borders::ALL)
                            .border_style(self.theme.dialog_border_style()),
                    )
                    .wrap(Wrap { trim: false });
                frame.render_widget(widget, dialog_area);
            }
        }
    }

    fn render_dialog_panel(&self, frame: &mut Frame, area: ratatui::layout::Rect, dialog: &Dialog) {
        frame.render_widget(
            Block::default()
                .style(self.theme.dialog_style())
                .borders(Borders::ALL)
                .border_style(self.theme.dialog_border_style()),
            area,
        );

        let inner = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Vertical)
            .constraints([
                ratatui::layout::Constraint::Length(2),
                ratatui::layout::Constraint::Length(2),
                ratatui::layout::Constraint::Min(4),
                ratatui::layout::Constraint::Length(1),
            ])
            .margin(1)
            .split(area);

        let header = Paragraph::new(dialog.title())
            .alignment(ratatui::layout::Alignment::Center)
            .style(self.theme.title_style());
        frame.render_widget(header, inner[0]);

        let description = Paragraph::new(dialog.description())
            .alignment(ratatui::layout::Alignment::Center)
            .style(self.theme.muted_style())
            .wrap(Wrap { trim: false });
        frame.render_widget(description, inner[1]);

        let max_visible = ((inner[2].height as usize + 2) / 3).max(1);
        let options = Paragraph::new(dialog.option_lines(&self.theme, max_visible))
            .style(self.theme.dialog_style())
            .wrap(Wrap { trim: false });
        frame.render_widget(options, inner[2]);

        let footer = Paragraph::new(dialog.footer_hint())
            .alignment(ratatui::layout::Alignment::Center)
            .style(self.theme.muted_style());
        frame.render_widget(footer, inner[3]);
    }

    fn dialog_area(&self, dialog: &Dialog, area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        match dialog.kind {
            DialogKind::Provider | DialogKind::Model => render::modal_rect(62, 16, 4, area),
            DialogKind::Agent | DialogKind::Task => render::modal_rect(60, 15, 4, area),
            DialogKind::SlashHelp => render::modal_rect(56, 12, 3, area),
            DialogKind::Thinking | DialogKind::ReasoningEffort => {
                render::modal_rect(52, 12, 4, area)
            }
            DialogKind::Session => render::modal_rect(58, 24, 4, area),
        }
    }

    fn modal_active(&self) -> bool {
        self.dialog.is_some()
            || self.pending_question.is_some()
            || self.pending_permission.is_some()
            || self.pending_text_input.is_some()
    }

    fn pump_prompt_job(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        let Some(job) = &self.prompt_job else {
            return Ok(());
        };

        let mut events = Vec::new();
        loop {
            match job.receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }

        let mut finished = false;
        for event in events {
            match event {
                PromptEvent::AssistantDelta(text) => {
                    self.assistant_preview.push_str(&text);
                    self.status = "AI running".to_string();
                    self.render_terminal(terminal)?;
                }
                PromptEvent::ThinkingDelta(text) => {
                    self.thinking_preview.push_str(&text);
                    self.status = "AI thinking".to_string();
                    self.render_terminal(terminal)?;
                }
                PromptEvent::ToolCall { name, .. } => {
                    self.status = format!("tool call: {}", name);
                    self.render_terminal(terminal)?;
                }
                PromptEvent::ToolBatch {
                    assistant,
                    tool_calls,
                    results,
                } => {
                    self.persist_message_detail(
                        "assistant",
                        &assistant,
                        None,
                        None,
                        Some(serde_json::json!(tool_calls)),
                    );
                    self.messages.push(Message {
                        role: "assistant".to_string(),
                        content: MessageContent::text(assistant.clone()),
                        name: None,
                        tool_call_id: None,
                        tool_calls: Some(serde_json::from_value(serde_json::json!(tool_calls)).unwrap_or_default()),
                    });
                    self.display.push(render::DisplayMessage::new("assistant", &assistant));
                    self.assistant_preview.clear();
                    if self.thinking_mode == ThinkingMode::Show && !self.thinking_preview.trim().is_empty() {
                        self.display.push(render::DisplayMessage::new("thinking", &self.thinking_preview));
                    }
                    self.thinking_preview.clear();
                    for item in &results {
                        self.capture_diff(&item.name, &item.args);
                        self.persist_message_detail(
                            "tool",
                            &item.result,
                            Some(item.name.clone()),
                            Some(item.id.clone()),
                            None,
                        );
                        self.messages.push(Message {
                            role: "tool".to_string(),
                            content: MessageContent::text(item.result.clone()),
                            name: Some(item.name.clone()),
                            tool_call_id: Some(item.id.clone()),
                            tool_calls: None,
                        });
                        let display_text = match (item.name.as_str(), &self.last_diff) {
                            ("edit" | "write", Some((_, before, after))) => {
                                format!("{}:\n{}", item.name, diff::unified_diff(before, after))
                            }
                            _ => format!("{}:\n{}", item.name, item.result),
                        };
                        self.display.push(render::DisplayMessage::new_collapsed(
                            "tool",
                            &display_text,
                        ));
                    }
                    self.status = format!("tool results: {} tools", results.len());
                    self.render_terminal(terminal)?;
                }
                PromptEvent::Finish => {
                    let assistant = self.assistant_preview.trim().to_string();
                    if !assistant.is_empty() {
                        self.messages.push(Message {
                            role: "assistant".to_string(),
                            content: MessageContent::text(assistant.clone()),
                            name: None,
                            tool_call_id: None,
                            tool_calls: None,
                        });
                        self.persist_message("assistant", &assistant);
                        self.display
                            .push(render::DisplayMessage::new("assistant", &assistant));
                    }
                    self.cache_total = self.cache_total.saturating_add(1);
                    self.ai_running = false;
                    self.status = "Ready".to_string();
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    if self.thinking_mode == ThinkingMode::Show && !self.thinking_preview.trim().is_empty() {
                        self.display.push(render::DisplayMessage::new("thinking", &self.thinking_preview));
                    }
                    self.thinking_preview.clear();
                    self.render_terminal(terminal)?;
                    finished = true;
                    self.generate_summary();
                }
                PromptEvent::Error(err) => {
                    tracing::error!(error = %err, "prompt worker error");
                    self.note(format!("provider error: {}", err));
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.thinking_preview.clear();
                    self.render_terminal(terminal)?;
                    finished = true;
                    self.generate_summary();
                }
            }
            if finished {
                break;
            }
        }

        Ok(())
    }

    fn handle_home_key(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        key: event::KeyEvent,
    ) -> anyhow::Result<bool> {
        match key.code {
            KeyCode::Enter => {
                let input = self.input.trim().to_string();
                if input.is_empty() {
                    self.input.clear();
                    self.cursor_index = 0;
                    return Ok(true);
                }
                if is_exit_command(&input) {
                    return Ok(false);
                }
                self.input.clear();
                self.cursor_index = 0;
                if self.handle_slash_command(&input) {
                    return Ok(true);
                }
                self.create_session();
                self.enqueue_or_run_prompt(terminal, input)?;
                Ok(true)
            }
            KeyCode::Esc => Ok(false),
            _ => Ok(false),
        }
    }

    fn home_provider_ready(&self) -> bool {
        self.config
            .get_provider(&self.provider_name)
            .and_then(|provider| provider.api_key)
            .is_some()
    }

    fn home_status_message(&self) -> &'static str {
        if self.config.provider.is_empty() {
            return "Setup required: no provider configured. Type /connect to add one.";
        }
        if self.config.model.is_none() {
            return "Setup required: no model selected. Type /models to choose one.";
        }
        if !self.home_provider_ready() {
            return "Setup required: API key missing. Type /connect to configure it.";
        }
        "Home: type a prompt and press Enter. Esc exits."
    }

    fn status_line(&self) -> Line<'static> {
        let running = if self.ai_running {
            "AI: running"
        } else {
            "AI: idle"
        };
        let cache_rate = if self.cache_total == 0 {
            "cache: n/a".to_string()
        } else {
            format!("cache: {}%", self.cache_hits * 100 / self.cache_total)
        };
        let used = token::estimate_messages(&self.messages);
        let window = self.current_context_window();
        let context = format!(
            "context: {} / {} tokens ({:.0}%)",
            used,
            window,
            used as f64 / window as f64 * 100.0
        );
        let task_count = self
            .store
            .as_ref()
            .and_then(|store| store.list_tasks(&self.session_id).ok())
            .map(|tasks| format!("tasks: {}", tasks.len()))
            .unwrap_or_else(|| "tasks: n/a".to_string());
        Line::from(vec![
            Span::styled(
                "OpenRust",
                self.theme.brand_style().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(format!("model: {}/{}", self.provider_name, self.model)),
            Span::raw("  |  "),
            Span::raw(format!(
                "agent: {}",
                self.current_session_agent().as_deref().unwrap_or("default")
            )),
            Span::raw("  |  "),
            Span::raw(task_count),
            Span::raw("  |  "),
            Span::raw(context),
            Span::raw("  |  "),
            Span::raw(cache_rate),
            Span::raw("  |  "),
            Span::raw(format!("thinking: {}", self.thinking_mode.label())),
            Span::raw("  |  "),
            Span::styled(running, self.theme.running_style(self.ai_running)),
            Span::raw("  |  "),
            Span::raw(self.status.clone()),
        ])
    }

    fn default_status_message(&self) -> String {
        if self.pending_permission.is_some()
            || self.pending_question.is_some()
            || self.pending_text_input.is_some()
            || self.dialog.is_some()
            || self.ai_running
            || self.prompt_job.is_some()
        {
            return self.status.clone();
        }
        if self.view_mode == ViewMode::Home {
            return self.home_status_message().to_string();
        }
        "Enter 发送 · /exit /q /quit 退出".to_string()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThinkingMode {
    Show,
    Hide,
}

impl ThinkingMode {
    fn label(self) -> &'static str {
        match self {
            ThinkingMode::Show => "show",
            ThinkingMode::Hide => "hide",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThinkingModeCommand {
    Show,
    Hide,
    Toggle,
}

impl ThinkingModeCommand {
    fn apply(self, current: ThinkingMode) -> ThinkingMode {
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
enum SlashCommand {
    Thinking(ThinkingModeCommand),
    Session(Vec<String>),
    Agent(Vec<String>),
    Compact(Vec<String>),
    Task(Vec<String>),
    Connect(Vec<String>),
    Models(Vec<String>),
    Files,
    Diff,
}

fn now_micros() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
}

fn home_input_hint() -> &'static str {
    "输入消息后 Enter 开始 · /connect 配置 provider · /models 选择模型 · Esc 退出"
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

    #[test]
    fn exit_keys_are_detected() {
        assert!(!should_exit(&crossterm::event::KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE
        )));
        assert!(should_exit(&crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!should_exit(&crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn exit_commands_are_detected() {
        assert!(is_exit_command("/exit"));
        assert!(is_exit_command("exit"));
        assert!(is_exit_command("/quit"));
        assert!(is_exit_command("/q"));
        assert!(!is_exit_command("exiting soon"));
    }

    #[test]
    fn dark_theme_defines_distinct_status_colors() {
        let theme = Theme::dark();

        assert_ne!(theme.running_style(true), theme.running_style(false));
        assert_ne!(theme.user_style(), theme.assistant_style());
        assert_ne!(theme.border_style(), theme.input_border_style(false));
    }

    #[test]
    fn input_width_counts_visible_cells() {
        assert_eq!(input_width("abc"), 3);
        assert_eq!(input_width("、"), 2);
    }

    #[test]
    fn home_input_hint_lists_primary_actions() {
        let hint = home_input_hint();
        assert!(hint.contains("Enter"));
        assert!(hint.contains("/connect"));
        assert!(hint.contains("/models"));
        assert!(hint.contains("Esc 退出"));
    }

    #[test]
    fn connect_add_without_args_starts_wizard() {
        let mut view = SessionView::new(
            "unconfigured".to_string(),
            "unconfigured".to_string(),
            String::new(),
            None,
            Config::default(),
            std::env::current_dir().unwrap(),
        );

        view.open_connect_dialog(vec!["add".to_string()]);

        assert!(view.pending_text_input.is_some());
        assert!(view.connect_draft.is_some());
    }

    #[test]
    fn thinking_mode_labels_are_stable() {
        assert_eq!(ThinkingMode::Show.label(), "show");
        assert_eq!(ThinkingMode::Hide.label(), "hide");
    }

    #[test]
    fn home_status_message_reports_missing_setup() {
        let view = SessionView::new(
            "unconfigured".to_string(),
            "unconfigured".to_string(),
            String::new(),
            None,
            Config::default(),
            std::env::current_dir().unwrap(),
        );

        assert_eq!(
            view.home_status_message(),
            "Setup required: no provider configured. Type /connect to add one."
        );
    }

    #[test]
    fn default_status_message_prefers_home_guidance_when_idle() {
        let view = SessionView::new(
            "unconfigured".to_string(),
            "unconfigured".to_string(),
            String::new(),
            None,
            Config::default(),
            std::env::current_dir().unwrap(),
        );

        assert_eq!(
            view.default_status_message(),
            "Setup required: no provider configured. Type /connect to add one."
        );
    }

    #[test]
    fn char_boundaries_follow_utf8_edges() {
        let input = "a、b";
        assert_eq!(prev_char_boundary(input, input.len()), 4);
        assert_eq!(prev_char_boundary(input, 4), 1);
        assert_eq!(next_char_boundary(input, 1), 4);
        assert_eq!(next_char_boundary(input, 4), input.len());
    }
}
