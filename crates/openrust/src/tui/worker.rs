use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

use crossterm::{cursor, event::DisableMouseCapture, execute, terminal};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::core::session::SessionStore;
use crate::core::{
    provider, provider::RequestOptions, provider::StreamChunk, provider::ToolDef,
    provider::ToolFunction,
};
use crate::tool::catalog;
use crate::tool::{AskRequest, PermissionRequest, ToolContext};

pub(super) struct PromptJob {
    pub(super) receiver: mpsc::Receiver<PromptEvent>,
    pub(super) ask_receiver: mpsc::Receiver<AskRequest>,
    pub(super) permission_receiver: mpsc::Receiver<PermissionRequest>,
}

pub(super) enum PromptEvent {
    AssistantDelta(String),
    ThinkingDelta(String),
    ToolCall {
        id: String,
        name: String,
    },
    ToolComplete {
        id: String,
        name: String,
        assistant: String,
        args: String,
        result: String,
    },
    Finish,
    Error(String),
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
) -> PromptJob {
    let (tx, rx) = mpsc::channel();
    let (ask_tx, ask_rx) = mpsc::channel::<AskRequest>();
    let (permission_tx, permission_rx) = mpsc::channel::<PermissionRequest>();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                let _ = tx.send(PromptEvent::Error(err.to_string()));
                return;
            }
        };

        let result = rt.block_on(async {
            let tool_defs: Vec<provider::ToolDef> = catalog::TOOL_CATALOG
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

            loop {
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
                        },
                    )
                    .await?;

                let mut assistant_text = String::new();
                let mut thinking_text = String::new();
                let mut pending_tools: Vec<(String, String, String)> = Vec::new();
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
                            let _ = tx.send(PromptEvent::ToolCall { id, name });
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
                            let Some((_, name, args)) = pending_tools
                                .iter()
                                .find(|(call_id, _, _)| call_id == &id)
                                .cloned()
                            else {
                                continue;
                            };
                            let tool_output = crate::tool::run_tool(&name, &args, &tool_ctx).await;
                            let _ = tx.send(PromptEvent::ToolComplete {
                                id: id.clone(),
                                name: name.clone(),
                                assistant: assistant_text.clone(),
                                args: args.clone(),
                                result: tool_output.clone(),
                            });
                            history.push(provider::Message {
                                role: "assistant".to_string(),
                                content: assistant_text.clone(),
                                name: None,
                                tool_call_id: None,
                                tool_calls: Some(vec![provider::ToolCall {
                                    id: id.clone(),
                                    kind: "function".to_string(),
                                    function: provider::ToolCallFunction {
                                        name: name.clone(),
                                        arguments: args.clone(),
                                    },
                                }]),
                            });
                            history.push(provider::Message {
                                role: "tool".to_string(),
                                content: tool_output,
                                name: Some(name),
                                tool_call_id: Some(id),
                                tool_calls: None,
                            });
                            assistant_text.clear();
                            thinking_text.clear();
                            pending_tools.clear();
                            break;
                        }
                        StreamChunk::Finish { .. } => {
                            finish_seen = true;
                        }
                    }
                }

                if !assistant_text.trim().is_empty() {
                    history.push(provider::Message {
                        role: "assistant".to_string(),
                        content: assistant_text,
                        name: None,
                        tool_call_id: None,
                        tool_calls: None,
                    });
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
            let _ = tx.send(PromptEvent::Error(err.to_string()));
        }
    });

    PromptJob {
        receiver: rx,
        ask_receiver: ask_rx,
        permission_receiver: permission_rx,
    }
}
