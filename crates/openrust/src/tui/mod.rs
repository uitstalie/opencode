//! Minimal interactive TUI for OpenRust.

use std::collections::VecDeque;
use std::io::IsTerminal;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use crossterm::{
    cursor,
    event::{self, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    execute,
    terminal::{self, ClearType},
};
use futures::StreamExt;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::core::{
    agent,
    config::{Config, ModelConfig, ProviderConfig},
    provider::{self, Message, RequestOptions, StreamChunk},
    session::SessionStore,
    vault::Vault,
};
use crate::tool::AskRequest;

mod dialog;
mod input;
mod render;
mod worker;

use dialog::{slash_hint_dialog, slash_options, Dialog, DialogKind, DialogOption};
use input::{input_width, is_exit_command, load_script, next_char_boundary, parse_slash_command, prev_char_boundary, should_exit};
use render::{centered_rect, display_message_lines, main_layout, DisplayMessage, Theme};
use worker::{spawn_prompt_worker, PromptEvent, PromptJob, SessionRuntimeGuard};

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
    let system = crate::core::system_prompt::SystemPrompt::from_config(
        &config,
        &resolved,
        config.mode.clone(),
    )?
    .render();

    let script_lines = script
        .as_ref()
        .map(|path| load_script(path))
        .transpose()?
        .unwrap_or_default();

    let mut session = SessionView::new(provider_name, model, system, llm.into(), config, cwd);
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
                                let label = option.get("label").and_then(|v| v.as_str())?.to_string();
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
                let multiple = value.get("multiple").and_then(|v| v.as_bool()).unwrap_or(false);
                Some(QuestionItem { header, question, options, multiple })
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
        self.selected = if self.selected == 0 { count - 1 } else { self.selected - 1 };
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

struct SessionView {
    provider_name: String,
    model: String,
    system: String,
    llm: Arc<dyn provider::LlmProvider>,
    config: Config,
    cwd: PathBuf,
    session_id: String,
    store: Option<SessionStore>,
    messages: Vec<Message>,
    display: Vec<DisplayMessage>,
    transcript: Vec<String>,
    pending_prompts: VecDeque<String>,
    interactive: bool,
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
    pending_question: Option<PendingQuestion>,
    pending_permission: Option<PendingPermission>,
    prompt_job: Option<PromptJob>,
    shutdown: Arc<AtomicBool>,
    assistant_preview: String,
    thinking_preview: String,
}

impl SessionView {
    fn new(
        provider_name: String,
        model: String,
        system: String,
        llm: Arc<dyn provider::LlmProvider>,
        config: Config,
        cwd: PathBuf,
    ) -> Self {
        let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
        let session_id = format!("session-{}", now_micros());
        let store = SessionStore::open().ok();
        if let Some(store) = &store {
            let _ = store.ensure_session(&session_id, None);
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
            input: String::new(),
            cursor_index: 0,
            session_scroll: 0,
            status: "Ready".to_string(),
            ai_running: false,
            cache_hits: 0,
            cache_total: 0,
            theme: Theme::dark(),
            thinking_mode: ThinkingMode::Hide,
            reasoning_effort: None,
            dialog: None,
            pending_question: None,
            pending_permission: None,
            prompt_job: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            assistant_preview: String::new(),
            thinking_preview: String::new(),
        }
    }

    fn bootstrap(
        &mut self,
        prompt: Option<String>,
        script_lines: Vec<String>,
    ) -> anyhow::Result<()> {
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
        execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide, EnableMouseCapture)?;
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
            self.maybe_start_next_prompt(terminal)?;
            self.status = "Enter 发送 · /exit /q /quit 退出".to_string();
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
                        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            break;
                        }
                        self.handle_permission_key(key);
                        continue;
                    }
                    if self.pending_question.is_some() {
                        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            break;
                        }
                        self.handle_question_key(key);
                        continue;
                    }
                    if should_exit(&key) {
                        break;
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
                KeyCode::Enter => {
                    if self.dialog.is_some() {
                        self.submit_dialog_selection();
                        continue;
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
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => self.scroll_session_up(3),
                    MouseEventKind::ScrollDown => self.scroll_session_down(3),
                    _ => {}
                },
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_prompt(&mut self, stdout: &mut io::Stdout, prompt: &str) -> anyhow::Result<()> {
        self.messages.push(Message {
            role: "user".to_string(),
            content: prompt.to_string(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
        self.persist_message("user", prompt);
        self.display.push(render::DisplayMessage::new("user", prompt));
        self.status = "Connecting model...".to_string();
        self.ai_running = true;
        self.assistant_preview.clear();
        self.thinking_preview.clear();
        self.render(stdout, Some(&self.status))?;
        self.prompt_job = Some(spawn_prompt_worker(
            Arc::clone(&self.llm),
            self.messages.clone(),
            self.model.clone(),
            self.effective_system(),
            self.reasoning_effort.clone(),
            self.cwd.clone(),
            Arc::clone(&self.shutdown),
            Some(self.session_id.clone()),
            self.store.clone(),
            false,
        ));
        while self.prompt_job.is_some() {
            self.pump_prompt_job_for_stdout(stdout)?;
        }
        Ok(())
    }

    fn enqueue_or_run_prompt(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        prompt: String,
    ) -> anyhow::Result<()> {
        if self.ai_running || self.prompt_job.is_some() {
            self.pending_prompts.push_back(prompt);
            self.status = format!("queued: {} prompt(s)", self.pending_prompts.len());
            return Ok(());
        }

        self.start_prompt_job(terminal, prompt)
    }

    fn start_prompt_job(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        prompt: String,
    ) -> anyhow::Result<()> {
        self.handle_interactive_prompt(terminal, &prompt)
    }

    fn maybe_start_next_prompt(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        if self.ai_running || self.prompt_job.is_some() {
            return Ok(());
        }

        let Some(prompt) = self.pending_prompts.pop_front() else {
            return Ok(());
        };

        self.start_prompt_job(terminal, prompt)
    }

    fn pump_prompt_job_for_stdout(&mut self, stdout: &mut io::Stdout) -> anyhow::Result<()> {
        let Some(job) = &self.prompt_job else {
            return Ok(());
        };

        let mut events = Vec::new();
        loop {
            match job.receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(10));
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }

        let mut finished = false;
        let mut needs_render = false;
        for event in events {
            match event {
                PromptEvent::AssistantDelta(text) => {
                    self.assistant_preview.push_str(&text);
                    self.status = "AI running".to_string();
                    needs_render = true;
                }
                PromptEvent::ThinkingDelta(text) => {
                    self.thinking_preview.push_str(&text);
                    self.status = "AI thinking".to_string();
                    needs_render = true;
                }
                PromptEvent::ToolCall { id, name } => {
                    self.display
                        .push(render::DisplayMessage::new("tool", &format!("tool call: {} ({})", name, id)));
                    self.status = format!("tool call: {}", name);
                    self.persist_message_detail("assistant", "", None, None, Some(serde_json::json!([{
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": "" }
                    }])));
                    needs_render = true;
                }
                PromptEvent::ToolComplete { id, name, assistant, args, result } => {
                    if !assistant.trim().is_empty() {
                        self.persist_message_detail(
                            "assistant",
                            &assistant,
                            None,
                            None,
                            Some(serde_json::json!([{
                                "id": id,
                                "type": "function",
                                "function": { "name": name, "arguments": args }
                            }])),
                        );
                    }
                    self.persist_message_detail("tool", &result, Some(name.clone()), Some(id.clone()), None);
                    self.display.push(render::DisplayMessage::new(
                        "tool",
                        &format!("tool result: {}\n{}", name, result),
                    ));
                    self.status = format!("tool result: {}", name);
                    needs_render = true;
                }
                PromptEvent::Finish => {
                    let assistant = self.assistant_preview.trim().to_string();
                    if !assistant.is_empty() {
                        self.messages.push(Message {
                            role: "assistant".to_string(),
                            content: assistant.clone(),
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
                    self.thinking_preview.clear();
                    needs_render = true;
                    finished = true;
                }
                PromptEvent::Error(err) => {
                    self.note(format!("provider error: {}", err));
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.thinking_preview.clear();
                    needs_render = true;
                    finished = true;
                }
            }
            if finished {
                break;
            }
        }

        if needs_render {
            self.render(stdout, Some(&self.status))?;
        }

        Ok(())
    }

    fn handle_interactive_prompt(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        prompt: &str,
    ) -> anyhow::Result<()> {
        self.messages.push(Message {
            role: "user".to_string(),
            content: prompt.to_string(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
        self.persist_message("user", prompt);
        self.display.push(render::DisplayMessage::new("user", prompt));
        self.status = "Connecting model...".to_string();
        self.ai_running = true;
        self.render_terminal(terminal)?;

        self.prompt_job = Some(spawn_prompt_worker(
            Arc::clone(&self.llm),
            self.messages.clone(),
            self.model.clone(),
            self.effective_system(),
            self.reasoning_effort.clone(),
            self.cwd.clone(),
            Arc::clone(&self.shutdown),
            Some(self.session_id.clone()),
            self.store.clone(),
            true,
        ));
        Ok(())
    }

    fn enqueue(&mut self, prompt: String) {
        self.transcript.push(prompt);
    }

    fn handle_slash_command(&mut self, input: &str) -> bool {
        let Some(command) = parse_slash_command(input) else {
            return false;
        };
        match command {
            SlashCommand::Thinking(mode) => {
                self.open_thinking_dialog(mode);
                true
            }
            SlashCommand::Session(args) => {
                self.open_session_dialog(args);
                true
            }
            SlashCommand::Agent(args) => {
                self.open_agent_dialog(args);
                true
            }
            SlashCommand::Task(args) => {
                self.open_task_dialog(args);
                true
            }
            SlashCommand::Compact(args) => {
                self.compact_session(args);
                true
            }
            SlashCommand::Connect(args) => {
                self.open_connect_dialog(args);
                true
            }
            SlashCommand::Models(args) => {
                self.open_models_dialog(args);
                true
            }
        }
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
            _ => {
                let Some(store) = self.store.clone() else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                let Ok(sessions) = store.list_sessions() else {
                    self.note("failed to list sessions".to_string());
                    return;
                };
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
                            DialogOption::new(
                                session.id.clone(),
                                format!("{}{}. {}", marker, index + 1, title),
                                format!(
                                    "{} messages · {} · agent: {}",
                                    session.message_count, session.id, agent
                                ),
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
                    let active = if self.current_session_agent().as_deref() == Some(info.id.as_str()) {
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
                let status = args.get(2).cloned().unwrap_or_else(|| "pending".to_string());
                let Some(store) = &self.store else {
                    self.note("session store unavailable".to_string());
                    return;
                };
                let id = format!("task-{}", now_micros());
                let agent = self.current_session_agent();
                match store.upsert_task(&self.session_id, &id, agent, title.clone(), status.clone()) {
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
            Some("add") => self.add_provider(&args),
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
                Some("__new__") => self.note("use /task add <title> [status] to create a task".to_string()),
                Some(value) => self.note(format!("task selected: {}", value)),
                None => {}
            },
            DialogKind::Provider => match dialog.selected_value() {
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
            self.status = if allow { "permission: allowed".to_string() } else { "permission: denied".to_string() };
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
            Line::from(Span::styled(format!("{}: {}", permission.tool, permission.detail), theme.muted_style())),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [A] Allow  ", allow_style),
                Span::styled("  [D] Deny  ", deny_style),
            ]),
            Line::from(""),
            Line::from(Span::styled("A/Y 允许 · D/N/Esc 拒绝 · ←/→ 切换 · Enter 确认", theme.muted_style())),
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
            lines.push(Line::from(Span::styled(format!("{}{}{}", marker, check, label), style)));
            if !description.is_empty() {
                lines.push(Line::from(Span::styled(format!("      {}", description), theme.muted_style())));
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
            lines.push(Line::from(Span::styled(format!("  > {}", buffer), theme.dialog_selected_style())));
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
                .and_then(|index| agents.get(index.saturating_sub(1)).map(|agent| agent.id.clone()))
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
            Ok(()) => self.note(format!("agent: {}", agent_id.as_deref().unwrap_or("default"))),
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

    fn agent_system(&self, agent_id: &str) -> Option<String> {
        let agents = agent::load_agents(&self.cwd).ok()?;
        if let Some(agent) = agents.iter().find(|item| item.id == agent_id) {
            return Some(agent.system.clone());
        }
        agent::builtin_agent_system(agent_id).map(str::to_string)
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
        let Some(store) = &self.store else {
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
        let summary = older
            .iter()
            .map(|message| format!("{}: {}", message.role, message.content))
            .collect::<Vec<_>>()
            .join("\n");
        match store.append_compaction(
            &self.session_id,
            format!("compaction summary\n{}", summary),
            recent
                .iter()
                .map(|message| format!("{}: {}", message.role, message.content))
                .collect::<Vec<_>>()
                .join("\n"),
        ) {
            Ok(_) => {
                self.messages = store
                    .effective_messages(&self.session_id)
                    .unwrap_or_default()
                    .iter()
                    .filter(|message| message.role == "user" || message.role == "assistant" || message.role == "tool")
                    .map(|message| Message {
                        role: message.role.clone(),
                        content: message.content.clone(),
                        name: message.name.clone(),
                        tool_call_id: message.tool_call_id.clone(),
                        tool_calls: message
                            .tool_calls
                            .as_ref()
                            .and_then(|value| serde_json::from_value(value.clone()).ok()),
                    })
                    .collect();
                self.display.push(render::DisplayMessage::new(
                    "system",
                    &format!("compacted {} message(s) into checkpoint", older.len()),
                ));
                self.note(format!("compacted session history, kept {} recent message(s)", recent.len()));
            }
            Err(err) => self.note(format!("failed to compact session: {}", err)),
        }
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
        self.messages = history
            .iter()
            .filter(|message| message.role == "user" || message.role == "assistant" || message.role == "tool")
            .map(|message| Message {
                role: message.role.clone(),
                content: message.content.clone(),
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
            .filter(|message| message.role == "user" || message.role == "assistant" || message.role == "tool")
            .map(|message| render::DisplayMessage::new(&message.role, &message.content))
            .collect();
        self.note(format!("session: switched to {}", id));
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
                            content: "reply ok".to_string(),
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
                        self.llm = llm.into();
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
            let _ = store.append_message_detail(&self.session_id, role, content, None, None, None);
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
            let _ = store.append_message_detail(
                &self.session_id,
                role,
                content,
                name,
                tool_call_id,
                tool_calls,
            );
        }
    }

    fn note(&mut self, message: String) {
        self.status = message.lines().next().unwrap_or("Ready").to_string();
        self.display.push(render::DisplayMessage::new("system", &message));
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

    fn render(&self, stdout: &mut io::Stdout, status: Option<&str>) -> anyhow::Result<()> {
        if self.interactive {
            execute!(
                stdout,
                terminal::Clear(ClearType::All),
                cursor::MoveTo(0, 0)
            )?;
        }
        writeln!(stdout, "OpenRust TUI")?;
        writeln!(stdout, "provider: {}", self.provider_name)?;
        writeln!(stdout, "model: {}", self.model)?;
        writeln!(stdout, "")?;
        if let Some(status) = status {
            writeln!(stdout, "status: {}", status)?;
        }
        writeln!(stdout, "")?;
        if self.interactive {
            writeln!(stdout, "input: {}", self.input)?;
            writeln!(stdout, "")?;
        }
        writeln!(stdout, "history:")?;
        for msg in self.messages.iter().rev().take(12).rev() {
            writeln!(stdout, "- {}: {}", msg.role, msg.content)?;
        }
        stdout.flush()?;
        Ok(())
    }

    fn render_terminal(
        &self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> anyhow::Result<()> {
        terminal.draw(|frame| self.render_frame(frame))?;
        Ok(())
    }

    fn render_frame(&self, frame: &mut Frame) {
        let regions = main_layout().split(frame.area());

        let session = Paragraph::new(self.session_lines(regions.session.height as usize))
            .style(self.theme.panel_style())
            .block(
                Block::default()
                    .title(" Session ")
                    .title_style(self.theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(self.theme.border_style()),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(session, regions.session);

        let input = Paragraph::new(self.input.as_str())
            .style(self.theme.input_style())
            .block(
                Block::default()
                    .title(" Input ")
                    .title_style(self.theme.title_style())
                    .borders(Borders::ALL)
                    .border_style(self.theme.input_border_style(self.ai_running)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(input, regions.input);
        if self.interactive {
            let x = regions
                .input
                .x
                .saturating_add(1)
                .saturating_add(input_width(&self.input[..self.cursor_index]));
            let y = regions.input.y.saturating_add(1);
            frame.set_cursor_position((x, y));
        }

        if self.dialog.is_none() && self.input.starts_with('/') {
            frame.render_widget(slash_hint_dialog(&self.input, &self.theme), centered_rect(54, 34, frame.area()));
        }

        let footer = Paragraph::new(self.status_line()).style(self.theme.footer_style());
        frame.render_widget(footer, regions.status);

        if let Some(dialog) = &self.dialog {
            let area = centered_rect(70, 50, frame.area());
            frame.render_widget(Clear, area);
            frame.render_widget(dialog.widget(&self.theme), area);
        }

        if let Some(question) = &self.pending_question {
            let area = centered_rect(72, 60, frame.area());
            frame.render_widget(Clear, area);
            frame.render_widget(self.question_widget(question), area);
        }

        if let Some(permission) = &self.pending_permission {
            let area = centered_rect(60, 32, frame.area());
            frame.render_widget(Clear, area);
            frame.render_widget(self.permission_widget(permission), area);
        }
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
                PromptEvent::ToolCall { id, name } => {
                    self.display
                        .push(render::DisplayMessage::new("tool", &format!("tool call: {} ({})", name, id)));
                    self.status = format!("tool call: {}", name);
                    self.render_terminal(terminal)?;
                }
                PromptEvent::ToolComplete { id, name, assistant, args, result } => {
                    if !assistant.trim().is_empty() {
                        self.persist_message_detail(
                            "assistant",
                            &assistant,
                            None,
                            None,
                            Some(serde_json::json!([{
                                "id": id,
                                "type": "function",
                                "function": { "name": name, "arguments": args }
                            }])),
                        );
                    }
                    self.persist_message_detail("tool", &result, Some(name.clone()), Some(id.clone()), None);
                    self.display.push(render::DisplayMessage::new(
                        "tool",
                        &format!("tool result: {}\n{}", name, result),
                    ));
                    self.status = format!("tool result: {}", name);
                    self.render_terminal(terminal)?;
                }
                PromptEvent::Finish => {
                    let assistant = self.assistant_preview.trim().to_string();
                    if !assistant.is_empty() {
                        self.messages.push(Message {
                            role: "assistant".to_string(),
                            content: assistant.clone(),
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
                    self.thinking_preview.clear();
                    self.render_terminal(terminal)?;
                    finished = true;
                }
                PromptEvent::Error(err) => {
                    self.note(format!("provider error: {}", err));
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.thinking_preview.clear();
                    self.render_terminal(terminal)?;
                    finished = true;
                }
            }
            if finished {
                break;
            }
        }

        Ok(())
    }

    fn session_lines(&self, region_height: usize) -> Vec<Line<'static>> {
        let mut lines = self
            .display
            .iter()
            .flat_map(|message| display_message_lines(message, &self.theme))
            .collect::<Vec<_>>();

        if self.ai_running && !self.thinking_preview.trim().is_empty() {
            if self.thinking_mode == ThinkingMode::Show {
                lines.push(Line::from(vec![Span::styled(
                    "thinking",
                    self.theme.thinking_style().add_modifier(Modifier::BOLD),
                )]));
                lines.extend(self.thinking_preview.lines().map(|line| {
                    Line::from(Span::styled(line.to_string(), self.theme.thinking_style()))
                }));
            }
        }

        if self.ai_running && !self.assistant_preview.trim().is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "assistant",
                self.theme.assistant_style().add_modifier(Modifier::BOLD),
            )]));
            lines.extend(self.assistant_preview.lines().map(|line| Line::from(line.to_string())));
        }

        if lines.is_empty() {
            return vec![Line::from(Span::styled(
                "No messages yet. Type in the input window and press Enter.",
                self.theme.muted_style(),
            ))];
        }

        let visible_height = region_height.saturating_sub(2).max(1);
        let max_scroll = lines.len().saturating_sub(visible_height);
        let scroll = self.session_scroll.min(max_scroll);
        let start = lines.len().saturating_sub(visible_height + scroll);
        lines.into_iter().skip(start).take(visible_height).collect()
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
        let context = format!("context: {} msgs", self.messages.len());
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
            Span::raw(format!("agent: {}", self.current_session_agent().as_deref().unwrap_or("default"))),
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
}

fn now_micros() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
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
        assert!(should_exit(&crossterm::event::KeyEvent::new(
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
    fn char_boundaries_follow_utf8_edges() {
        let input = "a、b";
        assert_eq!(prev_char_boundary(input, input.len()), 4);
        assert_eq!(prev_char_boundary(input, 4), 1);
        assert_eq!(next_char_boundary(input, 1), 4);
        assert_eq!(next_char_boundary(input, 4), input.len());
    }
}
