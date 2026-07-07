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
    event::{self, EnableBracketedPaste, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal,
};
use ratatui::{Terminal, backend::CrosstermBackend, style::Modifier, text::{Line, Span}};
use tui_textarea::TextArea;

use crate::core::{
    config::Config,
    provider::{self, Message, MessageContent},
    session::SessionStore,
    token,
};

mod dialog;
mod dialogs;
mod diff;
mod highlight;
mod html;
mod input;
mod latex;
mod markdown;
mod render;
mod interaction;
mod pending;
mod prompt_flow;
mod provider_ops;
mod session_ops;
mod session_render;
mod sidebar;
mod types;
mod util;
mod worker;

pub(in crate::tui) use types::*;
use util::*;

use dialog::{Dialog, DialogKind, slash_options};
use input::{is_exit_command, load_script, should_exit};
use render::{DisplayMessage, Theme, display_message_lines};
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
                let system = crate::system_prompt::SystemPrompt::from_config(
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
    shutdown: Arc<AtomicBool>,
    assistant_preview: String,
    thinking_preview: String,
    render: RenderState,
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
            input_editor: single_line_textarea("", false),
            cursor_index: 0,
            session_scroll: 0,
            status,
            ai_running: false,
            cache: CacheStats { hits: 0, total: 0, prompt_count: 0 },
            theme: Theme::dark(),
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
            },
            sidebar: None,
            sidebar_visible: false,
            diff_visible: false,
            last_diff: None,
            prompt_job: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            assistant_preview: String::new(),
            thinking_preview: String::new(),
            render: RenderState {
                lines: RefCell::new(Vec::new()),
                selection: None,
                mouse_down_row: None,
                mouse_dragging: false,
                area_top: Cell::new(0),
                area_height: Cell::new(24),
                dialog_area: Cell::new(None),
            },
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

        self.status = self.default_status_message();
        self.render_terminal(terminal)?;

        loop {
            let mut needs_render = self.pump_prompt_job()?;
            needs_render |= self.poll_ask_request();
            needs_render |= self.poll_permission_request();
            if self.sidebar_visible {
                if let Some(tree) = &mut self.sidebar {
                    needs_render |= tree.poll_refresh();
                }
            }
            self.maybe_start_next_prompt(terminal)?;
            self.status = self.default_status_message();
            if let Some(deadline) = self.ui.toast_deadline {
                if Instant::now() >= deadline {
                    self.ui.toast = None;
                    self.ui.toast_deadline = None;
                    needs_render = true;
                }
            }
            let poll_timeout = if self.prompt_job.is_some() || self.ai_running {
                Duration::from_millis(1)
            } else {
                Duration::from_millis(33)
            };
            if !event::poll(poll_timeout)? {
                if needs_render {
                    self.render_terminal(terminal)?;
                }
                continue;
            }

            let mut handled_input = false;
            let mut should_break = false;
            let mut events = vec![event::read()?];
            while event::poll(Duration::from_millis(0))? {
                events.push(event::read()?);
            }

            for event in events {
                match event {
                    Event::Key(key) => {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }
                        if self.ui.pending_permission.is_some() {
                            self.handle_permission_key(key);
                            handled_input = true;
                        } else if self.ui.pending_question.is_some() {
                            self.handle_question_key(key);
                            handled_input = true;
                        } else if self.ui.pending_text_input.is_some() {
                            self.handle_text_input_key(key);
                            handled_input = true;
                        } else if self.handle_global_copy_key(key)? {
                            handled_input = true;
                        } else if should_exit(&key) {
                            should_break = true;
                            break;
                        } else if self.view_mode == ViewMode::Home
                            && self.ui.dialog.is_none()
                            && self.handle_home_key(terminal, key)?
                        {
                            handled_input = true;
                        } else {
                            match key.code {
                                KeyCode::Esc if self.ui.dialog.is_some() => {
                                    self.ui.dialog = None;
                                    handled_input = true;
                                }
                                KeyCode::Up if self.ui.dialog.is_some() => {
                                    if let Some(dialog) = &mut self.ui.dialog {
                                        dialog.previous();
                                    }
                                    handled_input = true;
                                }
                                KeyCode::Down if self.ui.dialog.is_some() => {
                                    if let Some(dialog) = &mut self.ui.dialog {
                                        dialog.next();
                                    }
                                    handled_input = true;
                                }
                                KeyCode::Up if self.view_mode == ViewMode::Session => {
                                    self.scroll_session_up(3);
                                    handled_input = true;
                                }
                                KeyCode::Down if self.view_mode == ViewMode::Session => {
                                    self.scroll_session_down(3);
                                    handled_input = true;
                                }
                                KeyCode::Enter => {
                                    let mut skip_input = false;
                                    if self.ui.dialog.is_some() {
                                        let is_slash = matches!(
                                            self.ui.dialog.as_ref().map(|d| &d.kind),
                                            Some(DialogKind::SlashHelp)
                                        );
                                        self.submit_dialog_selection();
                                        handled_input = true;
                                        skip_input = !is_slash;
                                    }
                                    if !skip_input {
                                        let input = self.input.trim().to_string();
                                        self.clear_input();
                                        handled_input = true;
                                        if input.is_empty() {
                                            skip_input = true;
                                        }
                                        if !skip_input && is_exit_command(&input) {
                                            return Ok(());
                                        }
                                        if !skip_input && self.handle_slash_command(&input) {
                                            skip_input = true;
                                        }
                                        if !skip_input {
                                            self.enqueue_or_run_prompt(terminal, input)?;
                                        }
                                    }
                                }
                                KeyCode::Backspace => {
                                    if self.input_editor.input(textarea_input_from_key_event(key)) {
                                        self.sync_input_state();
                                        self.sync_slash_help();
                                        handled_input = true;
                                    }
                                }
                                KeyCode::Delete => {
                                    if self.input_editor.input(textarea_input_from_key_event(key)) {
                                        self.sync_input_state();
                                        self.sync_slash_help();
                                        handled_input = true;
                                    }
                                }
                                KeyCode::Left => {
                                    if self.input_editor.input(textarea_input_from_key_event(key)) {
                                        self.sync_input_state();
                                        handled_input = true;
                                    }
                                }
                                KeyCode::Right => {
                                    if self.input_editor.input(textarea_input_from_key_event(key)) {
                                        self.sync_input_state();
                                        handled_input = true;
                                    }
                                }
                                KeyCode::Home => {
                                    if self.input_editor.input(textarea_input_from_key_event(key)) {
                                        self.sync_input_state();
                                        handled_input = true;
                                    }
                                }
                                KeyCode::End => {
                                    if self.input_editor.input(textarea_input_from_key_event(key)) {
                                        self.sync_input_state();
                                        handled_input = true;
                                    }
                                }
                                KeyCode::PageUp => {
                                    self.scroll_session_up(8);
                                    handled_input = true;
                                }
                                KeyCode::PageDown => {
                                    self.scroll_session_down(8);
                                    handled_input = true;
                                }
                                KeyCode::Char('e')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    self.toggle_tool_collapse();
                                    handled_input = true;
                                }
                                KeyCode::Char(_) => {
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) {
                                        if self.input_editor.input(textarea_input_from_key_event(key)) {
                                            self.sync_input_state();
                                            self.sync_slash_help();
                                            handled_input = true;
                                        }
                                    }
                                }
                                KeyCode::Tab => {
                                    self.cycle_agent();
                                    handled_input = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    Event::Mouse(mouse) => {
                        if matches!(mouse.kind, crossterm::event::MouseEventKind::Moved) {
                            continue;
                        }
                        self.handle_mouse_event(mouse, terminal)?;
                        handled_input = true;
                    }
                    Event::Paste(text) => {
                        if self.ui.pending_text_input.is_some() {
                            self.insert_pending_text_input(&text);
                        } else {
                            self.insert_input_text(&text);
                        }
                        handled_input = true;
                    }
                    _ => {}
                }
            }

            if should_break {
                break;
            }

            if handled_input || needs_render {
                self.render_terminal(terminal)?;
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
            if name == "read" {
                if let Some((start, end)) = read_line_span(result) {
                    return format!("read {} L{}-{}", p, start, end);
                }
            }
            return format!("{} {}", name, p);
        }
        let preview = result.lines().next().unwrap_or("");
        let preview = &preview[..preview.len().min(60)];
        format!("{}: {}", name, preview)
    }

    fn note(&mut self, message: String) {
        self.status = message.lines().next().unwrap_or("Ready").to_string();
        self.ui.toast = Some(message);
        self.ui.toast_deadline = Some(Instant::now() + Duration::from_secs(4));
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

    fn pump_prompt_job(
        &mut self,
    ) -> anyhow::Result<bool> {
        let Some(job) = &self.prompt_job else {
            return Ok(false);
        };

        let mut events = Vec::new();
        loop {
            match job.receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }

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
                PromptEvent::ToolCall { name, .. } => {
                    self.status = format!("tool call: {}", name);
                    needs_render = true;
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
                    if self.thinking_mode == ThinkingMode::Show && !self.thinking_preview.trim().is_empty() {
                        self.display.push(render::DisplayMessage::new("thinking", &self.thinking_preview));
                    }
                    self.thinking_preview.clear();
                    if !assistant.is_empty() {
                        self.display.push(render::DisplayMessage::new("assistant", &assistant));
                    }
                    self.assistant_preview.clear();
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
                    needs_render = true;
                }
                PromptEvent::Finish { prompt_tokens, cache_hit_tokens } => {
                    if self.thinking_mode == ThinkingMode::Show && !self.thinking_preview.trim().is_empty() {
                        self.display.push(render::DisplayMessage::new("thinking", &self.thinking_preview));
                    }
                    self.thinking_preview.clear();
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
                    self.cache.prompt_count = self.cache.prompt_count.saturating_add(1);
                    self.cache.total = prompt_tokens as usize;
                    self.cache.hits = cache_hit_tokens as usize;
                    self.ai_running = false;
                    self.status = "Ready".to_string();
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    needs_render = true;
                    self.generate_summary();
                }
                PromptEvent::Error(err) => {
                    tracing::error!(error = %err, "prompt worker error");
                    self.note(format!("provider error: {}", err));
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.thinking_preview.clear();
                    needs_render = true;
                    self.generate_summary();
                }
            }
        }

        Ok(needs_render)
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
                    self.clear_input();
                    return Ok(true);
                }
                if is_exit_command(&input) {
                    return Ok(false);
                }
                self.clear_input();
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
        let cache_rate = if self.cache.prompt_count <= 1 {
            if self.cache.prompt_count == 0 {
                "cache: n/a".to_string()
            } else {
                "cache: priming".to_string()
            }
        } else {
            match self.cache.rate() {
                Some(pct) => format!("cache: {pct}%"),
                None => "cache: n/a".to_string(),
            }
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
        if self.ui.pending_permission.is_some()
            || self.ui.pending_question.is_some()
            || self.ui.pending_text_input.is_some()
            || self.ui.dialog.is_some()
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

#[cfg(test)]
mod tests {
    use super::*;
    use super::util::{home_input_hint, read_line_span};

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
            String::new(),
            None,
            Config::default(),
            std::env::current_dir().unwrap(),
        );

        view.open_connect_dialog(vec!["add".to_string()]);

        assert!(view.ui.pending_text_input.is_some());
        assert!(view.ui.connect_draft.is_some());
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
