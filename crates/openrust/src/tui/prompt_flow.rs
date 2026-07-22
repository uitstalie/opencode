use std::io;
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use super::{
    SessionView, SlashCommand, SlashResult, ThinkingMode, render, input::parse_slash_command,
    templates::init_template,
};

impl SessionView {
    pub(super) fn handle_prompt(&mut self, stdout: &mut io::Stdout, prompt: &str) -> anyhow::Result<()> {
        self.ensure_runtime_ready()?;
        self.messages.push(super::Message {
            role: "user".to_string(),
            content: super::MessageContent::text(prompt),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
        self.persist_message("user", prompt);
        self.generate_title();
        self.display.push(render::DisplayMessage::new("user", prompt));
        self.status = "Connecting model...".to_string();
        self.ai_running = true;
        self.assistant_preview.clear();
        self.thinking_preview.clear();
        self.thinking_start = None;
        self.thought_duration = None;
        self.render(stdout, Some(&self.status))?;
        tracing::debug!(
            provider = %self.provider_name,
            model = %self.model,
            context_window = self.current_context_window(),
            input_limit = ?self.config.resolve_input_tokens(),
            output_limit = ?self.config.resolve_output_tokens(),
            "starting prompt with model limits"
        );
        let Some(llm) = &self.llm else {
            self.note_error("LLM not initialized".to_string());
            return Ok(());
        };
        self.abort.store(false, std::sync::atomic::Ordering::SeqCst);
        self.prompt_job = Some(super::spawn_prompt_worker(
            Arc::clone(llm),
            self.messages.clone(),
            self.model.clone(),
            self.effective_system(),
            self.reasoning_effort.clone(),
            self.cwd.clone(),
            Arc::clone(&self.shutdown),
            self.abort.clone(),
            Some(self.session_id.clone()),
            self.store.clone(),
            false,
            self.current_agent_max_steps(),
            self.current_agent_tools(),
            self.config.presets.clone(),
            self.current_context_window(),
            self.config.resolve_output_tokens(),
            self.bus_tx.clone(),
            self.config.persist_agent_sessions,
        ));
        while self.prompt_job.is_some() {
            self.pump_prompt_job_for_stdout(stdout)?;
            // Avoid busy-polling at 100% CPU for the whole turn.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(())
    }

    pub(super) fn enqueue_or_run_prompt(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        prompt: String,
    ) -> anyhow::Result<()> {
        if self.ai_running || self.prompt_job.is_some() {
            self.enqueue_followup(prompt);
            return Ok(());
        }

        self.start_prompt_job(terminal, prompt)
    }

    /// Handle a user message submitted while the AI turn is in flight: show it
    /// immediately as a queued user message and forward it to the worker via
    /// the follow-up channel so it is injected at the next tool round.
    fn enqueue_followup(&mut self, prompt: String) {
        self.messages.push(super::Message {
            role: "user".to_string(),
            content: super::MessageContent::text(prompt.clone()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
        self.persist_message("user", &prompt);
        self.display.push(render::DisplayMessage::new("user", &prompt));
        if let Some(job) = &self.prompt_job {
            let _ = job.followup_tx.send(prompt);
            self.status = "queued for next round".to_string();
        } else {
            self.pending_prompts.push_back(prompt);
            self.status = format!("queued: {} prompt(s)", self.pending_prompts.len());
        }
    }

    pub(super) fn start_prompt_job(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        prompt: String,
    ) -> anyhow::Result<()> {
        if let Err(err) = self.handle_interactive_prompt(terminal, &prompt) {
            self.note_error(err.to_string());
            self.render_terminal(terminal)?;
        }
        Ok(())
    }

    pub(super) fn maybe_start_next_prompt(
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

    pub(super) fn pump_prompt_job_for_stdout(&mut self, stdout: &mut io::Stdout) -> anyhow::Result<()> {
        let events = self.drain_session_events();

        let mut finished = false;
        let mut needs_render = false;
        for event in events {
            let prompt_event = match event.payload {
                super::EventPayload::Prompt(e) => e,
                super::EventPayload::Ask(request) => {
                    // Headless is non-interactive; never leave a tool hanging.
                    let _ = request.responder.send(vec!["(non-interactive)".to_string()]);
                    continue;
                }
                super::EventPayload::Permission(request) => {
                    let _ = request.responder.send(false);
                    continue;
                }
                super::EventPayload::Progress(msg) => {
                    if self.ai_running {
                        self.status = msg;
                        needs_render = true;
                    }
                    continue;
                }
                super::EventPayload::Done { .. } => continue,
            };
            match prompt_event {
                super::PromptEvent::AssistantDelta(text) => {
                    if let Some(start) = self.thinking_start.take() {
                        self.thought_duration = Some(start.elapsed());
                    }
                    self.assistant_preview.push_str(&text);
                    self.status = "AI running".to_string();
                    needs_render = true;
                }
                super::PromptEvent::ThinkingDelta(text) => {
                    if self.thinking_start.is_none() {
                        self.thinking_start = Some(std::time::Instant::now());
                    }
                    self.thinking_preview.push_str(&text);
                    self.status = "AI thinking".to_string();
                    needs_render = true;
                }
                super::PromptEvent::ToolCallStart { id: _, name } => {
                    self.status = format!("tool call: {}", name);
                    needs_render = true;
                }
                super::PromptEvent::ToolRunning { id: _, args: _ } => {
                    self.status = "tool running".to_string();
                    needs_render = true;
                }
                super::PromptEvent::ToolBatch { assistant, tool_calls, results } => {
                    self.handle_thinking_preview(self.thinking_mode == ThinkingMode::Show);
                    self.save_assistant_message(&assistant, Some(&tool_calls));
                    self.assistant_preview.clear();
                    for item in &results {
                        self.persist_message_detail(
                            "tool",
                            &item.result,
                            Some(item.name.clone()),
                            Some(item.id.clone()),
                            None,
                        );
                        self.messages.push(super::Message {
                            role: "tool".to_string(),
                            content: super::MessageContent::text(item.result.clone()),
                            name: Some(item.name.clone()),
                            tool_call_id: Some(item.id.clone()),
                            tool_calls: None,
                        });
                        self.display.push(render::DisplayMessage::new_collapsed(
                            "tool",
                            &format!(
                                "{}\n{}",
                                Self::tool_display_preview(&item.name, &item.args, &item.result),
                                item.result
                            ),
                        ));
                    }
                    self.status = format!("tool results: {} tools", results.len());
                    needs_render = true;
                }
                super::PromptEvent::Finish { prompt_tokens, cache_hit_tokens } => {
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
                    finished = true;
                    self.generate_summary();
                    self.generate_memory();
                }
                super::PromptEvent::Error(err) => {
                    self.note_error(format!("provider error: {}", err));
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.thinking_preview.clear();
                    self.thinking_start = None;
                    self.thought_duration = None;
                    needs_render = true;
                    finished = true;
                    self.generate_summary();
                }
                super::PromptEvent::Aborted => {
                    self.ai_running = false;
                    self.prompt_job = None;
                    self.assistant_preview.clear();
                    self.thinking_preview.clear();
                    self.thinking_start = None;
                    self.thought_duration = None;
                    needs_render = true;
                    finished = true;
                }
                super::PromptEvent::RetryStatus(msg) => {
                    self.status = msg;
                    needs_render = true;
                }
                super::PromptEvent::Compacted { drained } => {
                    self.reload_compacted_history(drained);
                    needs_render = true;
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

    pub(super) fn handle_interactive_prompt(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        prompt: &str,
    ) -> anyhow::Result<()> {
        self.ensure_runtime_ready()?;
        self.messages.push(super::Message {
            role: "user".to_string(),
            content: super::MessageContent::text(prompt),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
        self.persist_message("user", prompt);
        self.generate_title();
        self.display.push(render::DisplayMessage::new("user", prompt));
        self.status = "Connecting model...".to_string();
        self.ai_running = true;
        self.assistant_preview.clear();
        self.thinking_preview.clear();
        self.thinking_start = None;
        self.thought_duration = None;
        self.render_terminal(terminal)?;

        let Some(llm) = &self.llm else {
            self.note_error("LLM not initialized".to_string());
            return Ok(());
        };
        self.abort.store(false, std::sync::atomic::Ordering::SeqCst);
        self.prompt_job = Some(super::spawn_prompt_worker(
            Arc::clone(llm),
            self.messages.clone(),
            self.model.clone(),
            self.effective_system(),
            self.reasoning_effort.clone(),
            self.cwd.clone(),
            Arc::clone(&self.shutdown),
            self.abort.clone(),
            Some(self.session_id.clone()),
            self.store.clone(),
            true,
            self.current_agent_max_steps(),
            self.current_agent_tools(),
            self.config.presets.clone(),
            self.current_context_window(),
            self.config.resolve_output_tokens(),
            self.bus_tx.clone(),
            self.config.persist_agent_sessions,
        ));
        Ok(())
    }

    pub(super) fn enqueue(&mut self, prompt: String) {
        self.transcript.push(prompt);
    }

    pub(super) fn handle_slash_command(&mut self, input: &str) -> SlashResult {
        let Some(command) = parse_slash_command(input) else {
            return SlashResult::NotHandled;
        };
        match command {
            SlashCommand::Thinking(mode) => {
                self.open_thinking_dialog(mode);
                SlashResult::Handled
            }
            SlashCommand::Session(args) => {
                self.open_session_dialog(args);
                SlashResult::Handled
            }
            SlashCommand::Agent(args) => {
                self.open_agent_dialog(args);
                SlashResult::Handled
            }
            SlashCommand::Task(args) => {
                self.open_task_dialog(args);
                SlashResult::Handled
            }
            SlashCommand::Compact(args) => {
                self.compact_session(args);
                SlashResult::Handled
            }
            SlashCommand::Connect(args) => {
                self.open_connect_dialog(args);
                SlashResult::Handled
            }
            SlashCommand::Models(args) => {
                self.open_models_dialog(args);
                SlashResult::Handled
            }
            SlashCommand::Files => {
                self.toggle_sidebar();
                SlashResult::Handled
            }
            SlashCommand::Diff => {
                self.toggle_diff();
                SlashResult::Handled
            }
            SlashCommand::Reload => {
                self.reload_resources();
                SlashResult::Handled
            }
            SlashCommand::Dream => {
                self.dream();
                SlashResult::Handled
            }
            SlashCommand::Theme(args) => {
                self.open_theme_dialog(args);
                SlashResult::Handled
            }
            SlashCommand::Init(args) => {
                SlashResult::Prompt(init_template(&args))
            }
        }
    }
}
