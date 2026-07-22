//! Minimal interactive TUI for OpenRust.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::IsTerminal;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{EnableBracketedPaste, EnableMouseCapture, Event},
    execute,
    terminal,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tui_textarea::TextArea;

use crate::core::{
    config::Config,
    event::{EventPayload, PromptEvent, SessionEvent},
    provider::{self, Message, MessageContent},
    session::SessionStore,
};

mod components;
mod dialog;
mod dialogs;
mod diff;
mod highlight;
mod html;
mod input;
mod layout;
mod latex;
mod markdown;
mod render;
mod interaction;
mod key_route;
mod pending;
mod persist;
mod prompt_flow;
mod provider_ops;
mod session_ops;
mod session_render;
mod sidebar;
mod templates;
pub(super) mod theme;
mod types;
mod ui_bus;
mod util;
mod view;
mod widgets;
mod worker;

pub(in crate::tui) use types::*;
use ui_bus::{UiBus, UiEvent};
use util::*;

use dialog::{Dialog, DialogKind, slash_options};
use input::{load_script};
use render::{DisplayMessage, Theme, display_message_lines};
use worker::{PromptJob, SessionRuntimeGuard, spawn_prompt_worker};

pub fn run(script: Option<PathBuf>, prompt: Option<String>) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let config = Config::load(&cwd)?;

    // Refresh the models.dev catalog in the background when the cache is
    // stale. This session keeps using the current cache; the fresh catalog
    // takes effect on next launch. Failures fall back to stale cache or
    // static builtins.
    crate::core::models_dev::spawn_refresh_if_stale();

    // Validate config at startup — show clear errors before TUI initializes.
    let vault = crate::core::vault::Vault::load();
    for error in config.validate(&vault) {
        eprintln!("openrust config: {}", error);
    }

    let (provider_name, model) = config
        .resolve_provider_model()
        .unwrap_or_else(|| ("unconfigured".to_string(), "unconfigured".to_string()));
    let (llm, system_prompt) = config
        .get_provider(&provider_name)
        .and_then(|resolved| {
            provider::create_provider(&resolved).map(|llm| {
                let prompt = crate::system_prompt::SystemPrompt::from_config(
                    &config,
                    &resolved,
                )
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "system prompt build failed");
                    crate::system_prompt::SystemPrompt::fallback(&provider_name, &model)
                });
                (Arc::from(llm), prompt)
            })
        })
        .map(|(llm, prompt)| (Some(llm), prompt))
        .unwrap_or((None, crate::system_prompt::SystemPrompt::fallback(&provider_name, &model)));

    let script_lines = script
        .as_ref()
        .map(load_script)
        .transpose()?
        .unwrap_or_default();

    let mut session = SessionView::new(provider_name, model, system_prompt, llm, config, cwd);
    session.bootstrap(prompt, script_lines)?;
    session.run()
}

struct SessionView {
    provider_name: String,
    model: String,
    system_prompt: crate::system_prompt::SystemPrompt,
    agents: Vec<crate::core::agent::AgentInfo>,
    llm: Option<Arc<dyn provider::LlmProvider>>,
    config: Config,
    cwd: PathBuf,
    session_id: String,
    store: Option<SessionStore>,
    /// Explicit agent selection for the active session; `None` means default.
    session_agent: Option<String>,
    messages: Vec<Message>,
    display: Vec<DisplayMessage>,
    transcript: Vec<String>,
    pending_prompts: VecDeque<String>,
    interactive: bool,
    view_mode: ViewMode,
    input: String,
    input_editor: TextArea<'static>,
    cursor_index: usize,
    session_scroll: usize,
    status: String,
    ai_running: bool,
    cache: CacheStats,
    theme: Theme,
    thinking_mode: ThinkingMode,
    reasoning_effort: Option<String>,
    ui: DialogState,
    sidebar: Option<sidebar::FileTree>,
    sidebar_visible: bool,
    diff_visible: bool,
    last_diff: Option<(String, String, String)>,
    prompt_job: Option<PromptJob>,
    /// Session event bus: workers, sub-agents, and background tasks all
    /// publish here. In interactive mode a forwarder thread moves events
    /// onto the UI bus; headless mode drains this directly.
    bus_tx: mpsc::Sender<SessionEvent>,
    bus_rx: Option<mpsc::Receiver<SessionEvent>>,
    /// In-flight tool calls awaiting results, shown live with a spinner.
    pending_tool_calls: Vec<PendingTool>,
    shutdown: Arc<AtomicBool>,
    /// Per-turn abort flag; set by ESC to interrupt the in-flight worker.
    abort: Arc<AtomicBool>,
    assistant_preview: String,
    thinking_preview: String,
    thinking_start: Option<Instant>,
    thought_duration: Option<Duration>,
    render: RenderState,
    /// Frame composition flag (SurfaceFlinger-style): handlers only mark the
    /// frame dirty; the actual draw happens on the next vsync Tick. When no
    /// handler dirtied the frame, the previous frame is reused (no draw).
    frame_dirty: bool,
    /// Cached task count (updated on todowrite events, avoids per-frame SQLite queries).
    task_count: std::cell::Cell<usize>,
    /// Cached task list for the TODO panel (updated on todowrite events).
    cached_tasks: std::cell::RefCell<Vec<crate::core::session::TaskSummary>>,
}

impl SessionView {
    fn new(
        provider_name: String,
        model: String,
    system_prompt: crate::system_prompt::SystemPrompt,
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
            let _ = store.ensure_session(&session_id);
            let cleanup_age = 30 * 24 * 60 * 60;
            match store.cleanup_old_sessions(cleanup_age) {
                Ok(0) => {}
                Ok(n) => tracing::info!(count = n, max_age_days = 30, "cleaned old sessions"),
                Err(e) => tracing::error!(err = %e, "cleanup_old_sessions failed"),
            }
            // GC persisted ephemeral agent sessions (sub-agent / background
            // debug histories) after 7 days.
            let agent_cleanup_age = 7 * 24 * 60 * 60;
            match store.cleanup_agent_sessions(agent_cleanup_age) {
                Ok(0) => {}
                Ok(n) => tracing::info!(count = n, max_age_days = 7, "cleaned old agent sessions"),
                Err(e) => tracing::error!(err = %e, "cleanup_agent_sessions failed"),
            }
        }
        // Load initial task list for TODO panel
        let initial_tasks = store
            .as_ref()
            .and_then(|s| s.list_tasks(&session_id).ok())
            .unwrap_or_default();
        let initial_task_count = initial_tasks.len();
        let theme = theme::resolve(config.theme.as_ref());
        let (bus_tx, bus_rx) = crate::core::event::session_bus();
        Self {
            provider_name,
            model,
            system_prompt,
            agents: crate::core::agent::load_agents(&cwd).unwrap_or_default(),
            llm,
            config,
            cwd,
            session_id,
            store,
            session_agent: None,
            messages: Vec::new(),
            display: Vec::new(),
            transcript: Vec::new(),
            pending_prompts: VecDeque::new(),
            interactive,
            view_mode: ViewMode::Home,
            input: String::new(),
            input_editor: single_line_textarea("", false),
            cursor_index: 0,
            session_scroll: 0,
            status,
            ai_running: false,
            cache: CacheStats { hits: 0, total: 0, prompt_count: 0 },
            theme,
            thinking_mode: ThinkingMode::Show,
            reasoning_effort: None,
            ui: DialogState {
                dialog: None,
                toast: None,
                toast_deadline: None,
                pending_question: None,
                pending_permission: None,
                pending_text_input: None,
                connect_draft: None,
                pending_provider: None,
            },
            sidebar: None,
            sidebar_visible: false,
            diff_visible: false,
            last_diff: None,
            prompt_job: None,
            bus_tx,
            bus_rx: Some(bus_rx),
            pending_tool_calls: Vec::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            abort: Arc::new(AtomicBool::new(false)),
            assistant_preview: String::new(),
            thinking_preview: String::new(),
            thinking_start: None,
            thought_duration: None,
            render: RenderState {
                lines: RefCell::new(Vec::new()),
                all_lines: RefCell::new(Vec::new()),
                scroll_offset: Cell::new(0),
                selection: None,
                mouse_down_row: None,
                mouse_dragging: false,
                area_top: Cell::new(0),
                area_height: Cell::new(24),
                dialog_area: Cell::new(None),
                input_area: Cell::new(ratatui::layout::Rect::ZERO),
                message_cache: RefCell::new(std::collections::HashMap::new()),
                theme_version: Cell::new(0),
            },
            task_count: std::cell::Cell::new(initial_task_count),
            cached_tasks: std::cell::RefCell::new(initial_tasks),
            frame_dirty: false,
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
            EnableBracketedPaste,
            EnableMouseCapture
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        // Create the guard before running so a panic still restores the
        // terminal (previously it was created after run_inner returned).
        let mut guard = SessionRuntimeGuard::new(terminal, Arc::clone(&self.shutdown));
        let session_rx = self
            .bus_rx
            .take()
            .expect("session bus receiver already taken");
        let ui_bus = UiBus::spawn(session_rx, Arc::clone(&self.shutdown));
        let run_result = self.run_inner(guard.terminal_mut(), &ui_bus);
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
        ui_bus: &UiBus,
    ) -> anyhow::Result<()> {
        let pending = std::mem::take(&mut self.transcript);
        for prompt in pending {
            self.handle_interactive_prompt(terminal, &prompt)?;
        }

        self.status = self.default_status_message();
        self.render_terminal(terminal)?;

        // Event-driven main loop with SurfaceFlinger-style frame composition:
        // handlers only mark `frame_dirty`; the draw happens on the next
        // vsync Tick. Multiple dirty sources within one frame interval
        // coalesce into a single draw; a Tick with no dirty flag reuses the
        // previous frame (no draw at all).
        loop {
            ui_bus.set_fast_tick(self.ai_running || self.frame_dirty);
            let Some(first) = ui_bus.recv() else {
                break;
            };
            let mut batch = vec![first];
            batch.extend(ui_bus.drain());

            let mut should_exit = false;
            for event in batch {
                match event {
                    UiEvent::Session(session_event) => {
                        self.frame_dirty |= self.handle_session_event(*session_event);
                    }
                    UiEvent::Tick => {
                        if self.sidebar_visible
                            && let Some(tree) = &mut self.sidebar {
                                self.frame_dirty |= tree.poll_refresh();
                            }
                        if let Some(deadline) = self.ui.toast_deadline
                            && Instant::now() >= deadline {
                                self.ui.toast = None;
                                self.ui.toast_deadline = None;
                                self.frame_dirty = true;
                            }
                        // Streaming / spinner frames: animating state always
                        // composes a new frame at vsync cadence.
                        if self.ai_running {
                            self.frame_dirty = true;
                        }
                        if self.frame_dirty {
                            self.render_terminal(terminal)?;
                            self.frame_dirty = false;
                        }
                    }
                    UiEvent::Input(Event::Key(key)) => {
                        match self.route_key(key, terminal)? {
                            view::KeyFlow::Exit => {
                                should_exit = true;
                                break;
                            }
                            view::KeyFlow::Consumed => self.frame_dirty = true,
                            view::KeyFlow::Propagate => {}
                        }
                    }
                    UiEvent::Input(Event::Mouse(mouse)) => {
                        if matches!(mouse.kind, crossterm::event::MouseEventKind::Moved) {
                            continue;
                        }
                        self.handle_mouse_event(mouse, terminal)?;
                        self.frame_dirty = true;
                    }
                    UiEvent::Input(Event::Paste(text)) => {
                        if self.ui.pending_text_input.is_some() {
                            self.insert_pending_text_input(&text);
                        } else {
                            self.insert_input_text(&text);
                        }
                        self.frame_dirty = true;
                    }
                    UiEvent::Input(_) => {}
                }
            }

            if should_exit {
                break;
            }

            self.maybe_start_next_prompt(terminal)?;
            let status = self.default_status_message();
            if status != self.status {
                self.status = status;
                self.frame_dirty = true;
            }
        }

        Ok(())
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

    /// Whether the slash-help popup should render as a Float layer.
    pub(super) fn slash_help_active(&self) -> bool {
        self.ui.dialog.as_ref().is_some_and(|d| {
            d.kind == DialogKind::SlashHelp && d.option_count() > 0
        })
    }

    /// Whether a modal overlay (dialog/question/permission/text-input) should render.
    pub(super) fn overlay_active(&self) -> bool {
        self.ui
            .dialog
            .as_ref()
            .is_some_and(|d| d.kind != DialogKind::SlashHelp)
            || self.ui.pending_question.is_some()
            || self.ui.pending_permission.is_some()
            || self.ui.pending_text_input.is_some()
    }

    fn note(&mut self, message: String) {
        tracing::info!(session = %self.session_id, "{}", message);
        self.status = message.lines().next().unwrap_or("Ready").to_string();
        self.ui.toast = Some(message);
        self.ui.toast_deadline = Some(Instant::now() + Duration::from_secs(4));
    }

    fn note_error(&mut self, message: String) {
        tracing::error!(session = %self.session_id, "{}", message);
        self.status = message.lines().next().unwrap_or("Error").to_string();
        self.ui.toast = Some(message);
        self.ui.toast_deadline = Some(Instant::now() + Duration::from_secs(8));
    }

    fn abort_current_turn(&mut self) {
        if self.prompt_job.is_some() {
            self.abort.store(true, Ordering::SeqCst);
            self.note("aborting…".to_string());
        }
    }

    fn sync_slash_help(&mut self) {
        if !self.input.starts_with('/') {
            if matches!(
                self.ui.dialog.as_ref().map(|dialog| dialog.kind),
                Some(DialogKind::SlashHelp)
            ) {
                self.ui.dialog = None;
            }
            return;
        }

        let options = slash_options(&self.input);
        if options.is_empty() {
            self.ui.dialog = None;
            return;
        }

        let selected = options
            .iter()
            .position(|option| option.value().starts_with(&self.input))
            .unwrap_or(0);

        self.ui.dialog = Some(Dialog::new(
            DialogKind::SlashHelp,
            "Slash Commands",
            "输入 / 时显示可用命令提示，Enter 可补全当前命令。",
            options,
            selected,
        ));
    }

    /// Drain events from the session event bus (headless mode only; in
    /// interactive mode the UI bus forwarder owns the receiver).
    fn drain_session_events(&self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        let Some(bus_rx) = &self.bus_rx else {
            return events;
        };
        loop {
            match bus_rx.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        events
    }

    /// Handle thinking preview state changes.
    fn handle_thinking_preview(&mut self, show_thought: bool) {
        if let Some(start) = self.thinking_start.take() {
            self.thought_duration = Some(start.elapsed());
        }
        if show_thought && !self.thinking_preview.trim().is_empty() {
            let meta = self.thought_duration
                .map(interaction::format_duration)
                .unwrap_or_default();
            self.display.push(render::DisplayMessage::new_with_meta(
                "thought",
                &self.thinking_preview,
                meta,
            ));
        }
        self.thinking_preview.clear();
        self.thinking_start = None;
        self.thought_duration = None;
    }

    /// Save assistant message to history and display.
    fn save_assistant_message(&mut self, assistant: &str, tool_calls: Option<&[serde_json::Value]>) {
        if assistant.is_empty() {
            return;
        }
        let tool_calls_value = tool_calls.map(|tc| serde_json::json!(tc));
        self.persist_message_detail("assistant", assistant, None, None, tool_calls_value);
        self.messages.push(Message {
            role: "assistant".to_string(),
            content: MessageContent::text(assistant.to_string()),
            name: None,
            tool_call_id: None,
            tool_calls: tool_calls.map(|tc| {
                serde_json::from_value(serde_json::json!(tc)).unwrap_or_default()
            }),
        });
        self.display.push(render::DisplayMessage::new("assistant", assistant));
    }

    /// Save tool result to history and display.
    fn save_tool_result(&mut self, name: &str, id: &str, args: &str, result: &str) {
        self.persist_message_detail("tool", result, Some(name.to_string()), Some(id.to_string()), None);
        self.messages.push(Message {
            role: "tool".to_string(),
            content: MessageContent::text(result.to_string()),
            name: Some(name.to_string()),
            tool_call_id: Some(id.to_string()),
            tool_calls: None,
        });
        let display_text = match (name, &self.last_diff) {
            ("edit" | "write", Some((_, before, after))) => {
                format!("{}:\n{}", name, diff::unified_diff(before, after))
            }
            _ => {
                let input = types::tool_display_input(name, args);
                if input.is_empty() {
                    format!("{}:\n{}", name, result)
                } else {
                    format!("{} · {}:\n{}", name, input, result)
                }
            }
        };
        self.display.push(render::DisplayMessage::new_collapsed("tool", &display_text));
    }

    /// Handle one event from the session bus (routed by payload; the
    /// session tag distinguishes main / sub-agent / background agents).
    /// Returns true when a re-render is needed.
    fn handle_session_event(&mut self, event: SessionEvent) -> bool {
        let mut needs_render = false;
        match event.payload {
                EventPayload::Ask(request) => {
                    needs_render |= self.handle_ask_request(request);
                }
                EventPayload::Permission(request) => {
                    needs_render |= self.handle_permission_request(request);
                }
                EventPayload::Progress(msg) => {
                    if self.ai_running {
                        self.status = msg;
                        needs_render = true;
                    }
                }
                EventPayload::Done { result } => {
                    match result {
                        Ok(msg) if !msg.is_empty() => self.note(msg),
                        Err(err) => self.note_error(err),
                        _ => {}
                    }
                    needs_render = true;
                }
                EventPayload::Prompt(event) => match event {
                PromptEvent::AssistantDelta(text) => {
                    if let Some(start) = self.thinking_start.take() {
                        self.thought_duration = Some(start.elapsed());
                    }
                    self.assistant_preview.push_str(&text);
                    self.status = "AI running".to_string();
                    needs_render = true;
                }
                PromptEvent::ThinkingDelta(text) => {
                    if self.thinking_start.is_none() {
                        self.thinking_start = Some(Instant::now());
                    }
                    self.thinking_preview.push_str(&text);
                    self.status = "AI thinking".to_string();
                    needs_render = true;
                }
                PromptEvent::ToolCallStart { id, name } => {
                    self.pending_tool_calls.push(PendingTool {
                        id,
                        name: name.clone(),
                        input: String::new(),
                        state: ToolState::Created,
                        started_at: Instant::now(),
                    });
                    self.status = format!("tool call: {}", name);
                    needs_render = true;
                }
                PromptEvent::ToolRunning { id, args } => {
                    if let Some(tool) = self
                        .pending_tool_calls
                        .iter_mut()
                        .rev()
                        .find(|t| t.id == id)
                    {
                        tool.state = ToolState::Running;
                        tool.input = tool_display_input(&tool.name, &args);
                    }
                    needs_render = true;
                }
                PromptEvent::ToolBatch {
                    assistant,
                    tool_calls,
                    results,
                } => {
                    self.pending_tool_calls.clear();
                    self.handle_thinking_preview(self.thinking_mode == ThinkingMode::Show);
                    self.save_assistant_message(&assistant, Some(&tool_calls));
                    self.assistant_preview.clear();
                    // Update task count when todowrite was called
                    let todo_changed = results.iter().any(|item| item.name == "todowrite");
                    if todo_changed {
                        if let Some(store) = &self.store {
                            if let Ok(tasks) = store.list_tasks(&self.session_id) {
                                self.task_count.set(tasks.len());
                                *self.cached_tasks.borrow_mut() = tasks;
                            }
                        }
                    }
                    for item in &results {
                        self.capture_diff(&item.name, &item.args);
                        self.save_tool_result(&item.name, &item.id, &item.args, &item.result);
                    }
                    self.status = format!("tool results: {} tools", results.len());
                    needs_render = true;
                }
                PromptEvent::Finish { prompt_tokens, cache_hit_tokens } => {
                    self.handle_thinking_preview(self.thinking_mode == ThinkingMode::Show);
                    let assistant = self.assistant_preview.trim().to_string();
                    if !assistant.is_empty() {
                        self.save_assistant_message(&assistant, None);
                    }
                    self.cache.prompt_count = self.cache.prompt_count.saturating_add(1);
                    self.cache.total = prompt_tokens as usize;
                    self.cache.hits = cache_hit_tokens as usize;
                    self.ai_running = false;
                    self.status = "Ready".to_string();
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    needs_render = true;
                    self.generate_summary();
                    self.generate_memory();
                }
                PromptEvent::Error(err) => {
                    self.note_error(format!("provider error: {}", err));
                    self.pending_tool_calls.clear();
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.thinking_preview.clear();
                    self.thinking_start = None;
                    self.thought_duration = None;
                    needs_render = true;
                    self.generate_summary();
                }
                PromptEvent::Aborted => {
                    // Salvage whatever assistant text streamed so far, drop the
                    // queued prompts, then return to ready.
                    self.handle_thinking_preview(self.thinking_mode == ThinkingMode::Show);
                    let assistant = self.assistant_preview.trim().to_string();
                    if !assistant.is_empty() {
                        self.save_assistant_message(&assistant, None);
                    }
                    self.pending_tool_calls.clear();
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.pending_prompts.clear();
                    self.note("aborted".to_string());
                    needs_render = true;
                    self.generate_summary();
                }
                PromptEvent::RetryStatus(msg) => {
                    self.status = msg;
                    needs_render = true;
                }
                },
            }
        needs_render
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::input::{is_exit_command, should_exit};
    use super::util::{home_input_hint, read_line_span};
    use crossterm::event::{KeyCode, KeyModifiers};

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
            crate::system_prompt::SystemPrompt::fallback("unconfigured", "unconfigured"),
            None,
            Config::default(),
            std::env::current_dir().unwrap(),
        );

        view.open_connect_dialog(vec!["add".to_string()]);

        assert!(view.ui.pending_text_input.is_some());
        assert!(view.ui.connect_draft.is_some());

        let input = view.ui.pending_text_input.take().unwrap();
        (input.submit)(&mut view, "custom-provider");
        assert_eq!(
            view.ui.dialog.as_ref().map(|dialog| dialog.kind),
            Some(DialogKind::ConnectProtocol)
        );
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
            crate::system_prompt::SystemPrompt::fallback("unconfigured", "unconfigured"),
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
            crate::system_prompt::SystemPrompt::fallback("unconfigured", "unconfigured"),
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
    fn read_line_span_extracts_first_and_last_line_numbers() {
        assert_eq!(
            read_line_span("     1: foo\n     2: bar\n     3: baz"),
            Some((1, 3))
        );
        assert_eq!(
            read_line_span("   100: foo\n   101: bar\n... (50 lines remaining)"),
            Some((100, 101))
        );
        assert_eq!(read_line_span("Cannot read: nope"), None);
        assert_eq!(read_line_span("   42: only"), Some((42, 42)));
    }
}
