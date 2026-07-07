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
}

#[derive(Debug)]
pub(super) enum PromptEvent {
    AssistantDelta(String),
    ThinkingDelta(String),
    ToolCall {
        _id: String,
        name: String,
    },
    ToolBatch {
        assistant: String,
        tool_calls: Vec<serde_json::Value>,
        results: Vec<ToolBatchItem>,
    },
    Finish,
    Error(String),
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

pub(super) fn spawn_prompt_worker(
    llm: Arc<dyn provider::LlmProvider>,
    messages: Vec<provider::Message>,
    model: String,
    system: String,
    reasoning_effort: Option<String>,
    cwd: std::path::PathBuf,
    shutdown: Arc<AtomicBool>,
    session_id: Option<String>,
    store: Option<SessionStore>,
    interactive: bool,
    max_steps: u32,
    agent_mode: String,
    context_window: u64,
) -> PromptJob {
    let (tx, rx) = mpsc::channel();
    let (ask_tx, ask_rx) = mpsc::channel::<AskRequest>();
    let (permission_tx, permission_rx) = mpsc::channel::<PermissionRequest>();
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
            let allowed = catalog::tools_for_mode(&agent_mode, false);
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
                ..ToolContext::new(std::path::PathBuf::new())
            };

            let mut step_count: u32 = 0;
            let mut current_total_tokens: u64 = 0;
            let compaction_settings = compaction::CompactionSettings::default();

            loop {
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
                            None,
                            &format!("Conversation history to compact:\n\n{}", older.join("\n")),
                        );
                        let sys = agent::builtin_agent_system("compaction")
                            .unwrap_or("Summarize conversation history.")
                            .to_string();

                        let summary = crate::tool::task::run_agent(
                            llm.as_ref(),
                            &model,
                            &sys,
                            "subagent",
                            5,
                            None,
                            vec![provider::Message::user(compact_prompt)],
                            &tool_ctx,
                        )
                        .await
                        .unwrap_or_else(|_| "compaction failed".to_string());

                        if !summary.trim().is_empty() {
                            let _ = store_clone.as_ref().unwrap().append_compaction(
                                session_id_clone.as_deref().unwrap_or("unknown"),
                                summary.trim().to_string(),
                                recent.join("\n"),
                            );
                            history.drain(..compact_cutoff);
                            history.insert(
                                0,
                                provider::Message {
                                    role: "system".to_string(),
                                    content: provider::MessageContent::text(
                                        "[compaction checkpoint]",
                                    ),
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

                let mut stream = llm
                    .chat(
                        history.clone(),
                        tool_defs.clone(),
                        RequestOptions {
                            model: model.clone(),
                            temperature: None,
                            max_tokens: None,
                            system: Some(system.clone()),
                            reasoning_effort: reasoning_effort.clone(),
                            tool_choice: if is_last_step {
                                Some("none".to_string())
                            } else {
                                None
                            },
                        },
                    )
                    .await?;

                let mut assistant_text = String::new();
                let mut thinking_text = String::new();
                let mut pending_tools: Vec<(String, String, String)> = Vec::new();
                let mut executed_tools: Vec<provider::ToolCall> = Vec::new();
                let mut tool_outputs: Vec<(String, String, String, String, bool)> = Vec::new();
                let mut finish_seen = false;

                while let Some(chunk) = stream.next().await {
                    if shutdown.load(Ordering::SeqCst) {
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
                            let _ = tx.send(PromptEvent::ToolCall { _id: id, name });
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
                            let tool_output = crate::tool::run_tool(&name, &args, &tool_ctx).await;
                            let has_image = matches!(&tool_output, crate::tool::ToolResult::Image { .. });
                            let tool_text = tool_output.into_text();
                            executed_tools.push(provider::ToolCall {
                                id: call_id.clone(),
                                kind: "function".to_string(),
                                function: provider::ToolCallFunction {
                                    name: name.clone(),
                                    arguments: args.clone(),
                                },
                            });
                            tool_outputs.push((call_id, name, args, tool_text, has_image));
                        }
                        StreamChunk::Finish { usage } => {
                            finish_seen = true;
                            if let Some(u) = usage {
                                current_total_tokens = u.total_tokens;
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
                        .map(|(call_id, name, args, tool_text, _)| ToolBatchItem {
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
                        tool_calls: Some(executed_tools.drain(..).collect()),
                    };
                    history.push(assistant);

                    for (call_id, name, _args, tool_text, has_image) in &tool_outputs {
                        if *has_image && llm.supports_images(&model) {
                            history.push(provider::Message::user_with_images(
                                format!("(image read by {} tool)", name),
                                vec![],
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
                    tool_outputs.clear();
                    pending_tools.clear();
                    finish_seen = false;
                }

                if !assistant_text.trim().is_empty() {
                    history.push(provider::Message::assistant(assistant_text));
                }

                if is_last_step {
                    let _ = tx.send(PromptEvent::Finish);
                    return Ok(());
                }

                if pending_tools.is_empty() && finish_seen {
                    let _ = tx.send(PromptEvent::Finish);
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
    }
}
