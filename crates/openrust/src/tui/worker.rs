use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crossterm::{cursor, event::{DisableBracketedPaste, DisableMouseCapture}, execute, terminal};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::core::session::SessionStore;
use crate::core::{
    agent, compaction, provider, provider::RequestOptions, provider::StreamChunk, provider::ToolDef,
    provider::ToolFunction,
};
use crate::tool::catalog;
use crate::tool::{AskRequest, PermissionRequest, ToolContext};

pub(super) struct PromptJob {
    pub(super) receiver: mpsc::Receiver<PromptEvent>,
    pub(super) ask_receiver: mpsc::Receiver<AskRequest>,
    pub(super) permission_receiver: mpsc::Receiver<PermissionRequest>,
    /// Sender held by the TUI; worker drains this each step to inject
    /// follow-up user messages queued while the turn is in flight.
    pub(super) followup_tx: mpsc::Sender<String>,
    /// Receiver for sub-agent progress strings (drained by the TUI pump).
    pub(super) progress_rx: mpsc::Receiver<String>,
}

#[derive(Debug)]
pub(super) enum PromptEvent {
    AssistantDelta(String),
    ThinkingDelta(String),
    ToolCallStart { id: String, name: String },
    ToolRunning { id: String },
    ToolBatch {
        assistant: String,
        tool_calls: Vec<serde_json::Value>,
        results: Vec<ToolBatchItem>,
    },
    Finish { prompt_tokens: u64, cache_hit_tokens: u64 },
    Error(String),
    /// User pressed ESC to abort the current turn.
    Aborted,
    /// Retry countdown: "retry 1/3 in 4s".
    RetryStatus(String),
}

#[derive(Debug, Clone)]
pub(super) struct ToolBatchItem {
    pub id: String,
    pub name: String,
    pub args: String,
    pub result: String,
}

pub(super) struct SessionRuntimeGuard {
    terminal: Option<Terminal<CrosstermBackend<std::io::Stdout>>>,
    raw_mode_enabled: bool,
    shutdown: Arc<AtomicBool>,
}

impl SessionRuntimeGuard {
    pub(super) fn new(
        terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            terminal: Some(terminal),
            raw_mode_enabled: true,
            shutdown,
        }
    }
}

impl Drop for SessionRuntimeGuard {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(mut terminal) = self.terminal.take() {
            let _ = execute!(
                terminal.backend_mut(),
                terminal::LeaveAlternateScreen,
                cursor::Show,
                DisableBracketedPaste,
                DisableMouseCapture
            );
        }
        if self.raw_mode_enabled {
            let _ = terminal::disable_raw_mode();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_prompt_worker(
    llm: Arc<dyn provider::LlmProvider>,
    messages: Vec<provider::Message>,
    model: String,
    system: String,
    reasoning_effort: Option<String>,
    cwd: std::path::PathBuf,
    shutdown: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    session_id: Option<String>,
    store: Option<SessionStore>,
    interactive: bool,
    max_steps: u32,
    tool_spec: String,
    presets: std::collections::HashMap<String, Vec<String>>,
    context_window: u64,
) -> PromptJob {
    let (tx, rx) = mpsc::channel();
    let (ask_tx, ask_rx) = mpsc::channel::<AskRequest>();
    let (permission_tx, permission_rx) = mpsc::channel::<PermissionRequest>();
    let (followup_tx, followup_rx) = mpsc::channel::<String>();
    let (progress_tx, progress_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                tracing::error!(error = %err, "tokio runtime creation failed");
                let _ = tx.send(PromptEvent::Error(err.to_string()));
                return;
            }
        };

        let result = rt.block_on(async {
            let allowed = catalog::resolve_tool_names(&tool_spec, &presets, false);
            let tool_defs: Vec<provider::ToolDef> = allowed
                .iter()
                .filter_map(|meta| catalog::create_tool(meta.name, None))
                .map(|tool| ToolDef {
                    r#type: "function".to_string(),
                    function: ToolFunction {
                        name: tool.name().to_string(),
                        description: tool.description().to_string(),
                        parameters: tool.parameters(),
                    },
                })
                .collect();
            let mut history = messages;
            let session_id_clone = session_id.clone();
            let store_clone = store.clone();
            let tool_ctx = ToolContext {
                cwd,
                interactive,
                session_id,
                store,
                ask_tx: if interactive { Some(ask_tx) } else { None },
                permission_tx: if interactive {
                    Some(permission_tx)
                } else {
                    None
                },
                llm: Some(Arc::clone(&llm)),
                model: Some(model.clone()),
                reasoning_effort: reasoning_effort.clone(),
                shutdown: Some(Arc::clone(&shutdown)),
                abort: Some(Arc::clone(&abort)),
                progress_tx: Some(progress_tx),
                presets: presets.clone(),
                ..ToolContext::new(std::path::PathBuf::new())
            };

            let mut step_count: u32 = 0;
            let mut current_total_tokens: u64 = 0;
            let mut total_prompt_tokens: u64 = 0;
            let mut total_cache_hit_tokens: u64 = 0;
            let mut last_compaction_summary: Option<String> = None;
            let compaction_settings = compaction::CompactionSettings::default();

            // Turn-start TODO injection: cross-turn persistence.
            // The supervisory list spans turns until all items are done.
            // Uses role "user" (not "system") because many providers reject
            // system messages in the middle of the conversation.
            if let (Some(store), Some(session_id)) =
                (store_clone.as_ref(), session_id_clone.as_ref())
                && let Ok(tasks) = store.list_tasks(session_id) {
                    let reminder = crate::tool::todowrite::todo_reminder(&tasks);
                    if !reminder.is_empty() {
                        history.push(provider::Message::user(reminder));
                    }
                }

            loop {
                if shutdown.load(Ordering::SeqCst) {
                    return Ok::<_, anyhow::Error>(());
                }
                if abort.load(Ordering::SeqCst) {
                    let _ = tx.send(PromptEvent::Aborted);
                    return Ok::<_, anyhow::Error>(());
                }
                step_count += 1;

                let is_last_step = step_count >= max_steps;
                if is_last_step {
                    history.push(provider::Message::assistant(
                        agent::MAX_STEPS_PROMPT.to_string(),
                    ));
                }

                // Token-based auto-compaction: when the last response reported
                // total_tokens near the context window, compact before the next call.
                if !is_last_step
                    && context_window > 0
                    && current_total_tokens > 0
                    && store_clone.is_some()
                    && compaction::is_overflow(
                        current_total_tokens as usize,
                        context_window as usize,
                        16_000,
                        &compaction_settings,
                    )
                {
                    let compact_cutoff = history.len().saturating_sub(8);
                    let older: Vec<String> = history[..compact_cutoff]
                        .iter()
                        .map(|m| format!("{}: {}", m.role, m.content.as_text()))
                        .collect();
                    let recent: Vec<String> = history[compact_cutoff..]
                        .iter()
                        .map(|m| format!("{}: {}", m.role, m.content.as_text()))
                        .collect();

                    if !older.is_empty() {
                        let compact_prompt = compaction::build_compaction_prompt(
                            last_compaction_summary.as_deref(),
                            &format!("Conversation history to compact:\n\n{}", older.join("\n")),
                        );
                        let sys = agent::builtin_agent_system("compaction")
                            .unwrap_or("Summarize conversation history.")
                            .to_string();

                        let summary = crate::tool::task::run_agent(
                            llm.as_ref(),
                            &model,
                            &sys,
                            "none",
                            5,
                            None,
                            vec![provider::Message::user(compact_prompt)],
                            &tool_ctx,
                        )
                        .await
                        .unwrap_or_else(|_| "compaction failed".to_string());

                        if !summary.trim().is_empty() {
                            if let Some(store) = store_clone.as_ref() {
                                let _ = store.append_compaction(
                                    session_id_clone.as_deref().unwrap_or("unknown"),
                                    summary.trim().to_string(),
                                    recent.join("\n"),
                                );
                                let _ = store.flush();
                            }
                            let summary_text = summary.trim().to_string();
                            last_compaction_summary = Some(summary_text.clone());
                            history.drain(..compact_cutoff);
                            history.insert(
                                0,
                                provider::Message {
                                    role: "system".to_string(),
                                    content: provider::MessageContent::text(summary_text),
                                    name: None,
                                    tool_call_id: None,
                                    tool_calls: None,
                                },
                            );
                            current_total_tokens = 0;
                        }
                    }
                }

                // Trim history to fit within context window before calling LLM.
                // Always keep the first 2 messages (system/user pair) and last 4.
                if context_window > 0 {
                    compaction::trim_history(&mut history, context_window, 24_000, 6);
                }

                // Retryable + abortable LLM call: on retriable errors, show
                // countdown via RetryStatus so the user sees what's happening.
                const MAX_RETRIES: u32 = 3;
                let mut stream: Option<provider::ChunkStream> = None;
                'retry: for attempt in 0..=MAX_RETRIES {
                    if shutdown.load(Ordering::SeqCst) {
                        return Ok::<_, anyhow::Error>(());
                    }
                    if abort.load(Ordering::SeqCst) {
                        let _ = tx.send(PromptEvent::Aborted);
                        return Ok::<_, anyhow::Error>(());
                    }

                    let chat = llm.chat(
                        history.clone(),
                        tool_defs.clone(),
                        RequestOptions {
                            model: model.clone(),
                            temperature: None,
                            max_tokens: None,
                            top_p: None,
                            system: Some(system.clone()),
                            reasoning_effort: reasoning_effort.clone(),
                            tool_choice: if is_last_step {
                                Some(serde_json::json!("none"))
                            } else {
                                None
                            },
                        },
                    );
                    tokio::pin!(chat);

                    loop {
                        tokio::select! {
                            result = &mut chat => {
                                match result {
                                    Ok(s) => {
                                        stream = Some(s);
                                        break 'retry;
                                    }
                                    Err(e) => {
                                        if attempt < MAX_RETRIES && is_retriable_error(&e) {
                                            let delay_secs = 2u64.pow(attempt + 1).min(8);
                                            let detail = short_error(&e);
                                            let mut remaining_ms = delay_secs * 1000;
                                            while remaining_ms > 0 {
                                                let secs = remaining_ms.div_ceil(1000);
                                                let _ = tx.send(PromptEvent::RetryStatus(
                                                    format!("retry {}/{} in {}s — {}",
                                                        attempt + 1, MAX_RETRIES, secs, detail),
                                                ));
                                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                                if shutdown.load(Ordering::SeqCst) {
                                                    return Ok::<_, anyhow::Error>(());
                                                }
                                                if abort.load(Ordering::SeqCst) {
                                                    let _ = tx.send(PromptEvent::Aborted);
                                                    return Ok::<_, anyhow::Error>(());
                                                }
                                                remaining_ms = remaining_ms.saturating_sub(500);
                                            }
                                            break; // inner → retry outer
                                        }
                                        return Err(e);
                                    }
                                }
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                                if shutdown.load(Ordering::SeqCst) {
                                    return Ok::<_, anyhow::Error>(());
                                }
                                if abort.load(Ordering::SeqCst) {
                                    let _ = tx.send(PromptEvent::Aborted);
                                    return Ok::<_, anyhow::Error>(());
                                }
                            }
                        }
                    }
                }
                let mut stream = stream.expect("retry loop sets stream or returns");

                let mut assistant_text = String::new();
                let mut thinking_text = String::new();
                let mut pending_tools: Vec<(String, String, String)> = Vec::new();
                let mut executed_tools: Vec<provider::ToolCall> = Vec::new();
                type ToolOutput = (String, String, String, String, bool, Option<(String, String)>);
                let mut tool_outputs: Vec<ToolOutput> = Vec::new();
                let mut finish_seen = false;

                loop {
                    tokio::select! {
                        chunk = stream.next() => {
                            let Some(chunk) = chunk else { break; };
                            if shutdown.load(Ordering::SeqCst) {
                                return Ok::<_, anyhow::Error>(());
                            }
                            if abort.load(Ordering::SeqCst) {
                                let _ = tx.send(PromptEvent::Aborted);
                                return Ok::<_, anyhow::Error>(());
                            }
                            match chunk? {
                                StreamChunk::TextDelta(text) => {
                                    assistant_text.push_str(&text);
                                    let _ = tx.send(PromptEvent::AssistantDelta(text));
                                }
                                StreamChunk::ReasoningDelta(text) => {
                                    thinking_text.push_str(&text);
                                    let _ = tx.send(PromptEvent::ThinkingDelta(text));
                                }
                                StreamChunk::ToolCallStart { id, name } => {
                                    pending_tools.push((id.clone(), name.clone(), String::new()));
                                    let _ = tx.send(PromptEvent::ToolCallStart { id, name });
                                }
                                StreamChunk::ToolCallDelta { id, args } => {
                                    if let Some((_, _, buffer)) = pending_tools
                                        .iter_mut()
                                        .rev()
                                        .find(|(call_id, _, _)| call_id == &id)
                                    {
                                        buffer.push_str(&args);
                                    }
                                }
                                StreamChunk::ToolCallEnd { id } => {
                                    let Some((call_id, name, args)) = pending_tools
                                        .iter()
                                        .find(|(cid, _, _)| cid == &id)
                                        .cloned()
                                    else {
                                        continue;
                                    };
                                    let _ = tx.send(PromptEvent::ToolRunning { id: call_id.clone() });
                                    let tool_output = crate::tool::run_tool(&name, &args, &tool_ctx).await;
                                    let has_image = matches!(&tool_output, crate::tool::ToolResult::Image { .. });
                                    let image_b64 = match &tool_output {
                                        crate::tool::ToolResult::Image { base64_data, mime_type, .. } => Some((base64_data.clone(), mime_type.clone())),
                                        _ => None,
                                    };
                                    let tool_text = tool_output.into_text();
                                    executed_tools.push(provider::ToolCall {
                                        id: call_id.clone(),
                                        kind: "function".to_string(),
                                        function: provider::ToolCallFunction {
                                            name: name.clone(),
                                            arguments: args.clone(),
                                        },
                                    });
                                    tool_outputs.push((call_id, name, args, tool_text, has_image, image_b64));
                                }
                                StreamChunk::Finish { usage, .. } => {
                                    finish_seen = true;
                                    if let Some(u) = &usage {
                                        current_total_tokens = u.total_tokens;
                                        total_prompt_tokens = total_prompt_tokens.saturating_add(u.prompt_tokens);
                                        total_cache_hit_tokens = total_cache_hit_tokens.saturating_add(u.prompt_cache_hit_tokens);
                                    }
                                }
                            }
                        }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                            if shutdown.load(Ordering::SeqCst) {
                                return Ok::<_, anyhow::Error>(());
                            }
                            if abort.load(Ordering::SeqCst) {
                                let _ = tx.send(PromptEvent::Aborted);
                                return Ok::<_, anyhow::Error>(());
                            }
                        }
                    }
                }

                if !executed_tools.is_empty() {
                    let captured_text = assistant_text.clone();
                    assistant_text.clear();
                    thinking_text.clear();

                    let tool_calls_json: Vec<serde_json::Value> = executed_tools
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": tc.kind,
                                "function": {
                                    "name": tc.function.name,
                                    "arguments": tc.function.arguments
                                }
                            })
                        })
                        .collect();

                    let results: Vec<ToolBatchItem> = tool_outputs
                        .iter()
                        .map(|(call_id, name, args, tool_text, _, _)| ToolBatchItem {
                            id: call_id.clone(),
                            name: name.clone(),
                            args: args.clone(),
                            result: tool_text.clone(),
                        })
                        .collect();

                    let _ = tx.send(PromptEvent::ToolBatch {
                        assistant: captured_text.clone(),
                        tool_calls: tool_calls_json,
                        results,
                    });

                    let assistant = provider::Message {
                        role: "assistant".to_string(),
                        content: provider::MessageContent::text(captured_text.clone()),
                        name: None,
                        tool_call_id: None,
                        tool_calls: Some(std::mem::take(&mut executed_tools)),
                    };
                    history.push(assistant);

                    for (call_id, name, _args, tool_text, has_image, image_b64) in &tool_outputs {
                        if let Some((b64, mime)) = image_b64
                            && *has_image && llm.supports_images(&model) {
                                history.push(provider::Message::user_with_images(
                                    format!("(image read by {} tool)", name),
                                    vec![provider::ContentPart::image_url(
                                        format!("data:{};base64,{}", mime, b64),
                                    )],
                                ));
                            }
                        history.push(provider::Message {
                            role: "tool".to_string(),
                            content: provider::MessageContent::text(tool_text.clone()),
                            name: Some(name.clone()),
                            tool_call_id: Some(call_id.clone()),
                            tool_calls: None,
                        });
                    }
                    // Inject TODO reminder only when the state changed
                    // (todowrite was called in this batch).
                    // Uses role "user" because many providers reject system
                    // messages in the middle of the conversation.
                    let todo_changed = tool_outputs
                        .iter()
                        .any(|(_, name, _, _, _, _)| name == "todowrite");
                    if todo_changed
                        && let (Some(store), Some(session_id)) =
                            (store_clone.as_ref(), session_id_clone.as_ref())
                            && let Ok(tasks) = store.list_tasks(session_id) {
                                let reminder =
                                    crate::tool::todowrite::todo_reminder(&tasks);
                                if !reminder.is_empty() {
                                    history.push(provider::Message::user(reminder));
                                }
                            }

                    tool_outputs.clear();
                    pending_tools.clear();
                    finish_seen = false;
                }

                if !assistant_text.trim().is_empty() {
                    history.push(provider::Message::assistant(assistant_text));
                }

                // Drain follow-up user messages queued mid-turn and inject them
                // so the next LLM round sees them alongside tool results.
                while let Ok(msg) = followup_rx.try_recv() {
                    history.push(provider::Message::user(msg));
                }

                if is_last_step {
                    let _ = tx.send(PromptEvent::Finish {
                        prompt_tokens: total_prompt_tokens,
                        cache_hit_tokens: total_cache_hit_tokens,
                    });
                    return Ok(());
                }

                if pending_tools.is_empty() && finish_seen {
                    let _ = tx.send(PromptEvent::Finish {
                        prompt_tokens: total_prompt_tokens,
                        cache_hit_tokens: total_cache_hit_tokens,
                    });
                    return Ok(());
                }

                if pending_tools.is_empty() {
                    continue;
                }
            }
        });

        if shutdown.load(Ordering::SeqCst) {
            return;
        }

        if let Err(err) = result {
            tracing::error!(error = %err, "prompt worker failed");
            let _ = tx.send(PromptEvent::Error(err.to_string()));
        }
    });

    PromptJob {
        receiver: rx,
        ask_receiver: ask_rx,
        permission_receiver: permission_rx,
        followup_tx,
        progress_rx,
    }
}

/// Whether an error from llm.chat() is worth retrying.
fn is_retriable_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    // Don't retry 4xx (except 429 rate-limit and 408 timeout).
    if msg.starts_with("HTTP 4") && !msg.starts_with("HTTP 429") && !msg.starts_with("HTTP 408") {
        return false;
    }
    true
}

/// Short summary of an error for the status line.
fn short_error(err: &anyhow::Error) -> String {
    let msg = err.to_string();
    let first_line = msg.lines().next().unwrap_or(&msg);
    if first_line.len() > 50 {
        format!("{}…", &first_line[..47])
    } else {
        first_line.to_string()
    }
}
