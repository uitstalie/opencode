//! Session management, agent switching, compaction, and summary generation.

use std::sync::Arc;

use super::render;
use super::util::now_micros;
use super::{SessionView, ViewMode};
use crate::core::agent;
use crate::core::compaction;
use crate::core::provider::{self, Message, MessageContent};

/// Lazily-initialized shared tokio runtime for fire-and-forget background
/// threads (title, summary, compaction). Avoids creating a new `Runtime`
/// per thread.
fn shared_runtime() -> Option<&'static tokio::runtime::Runtime> {
    static RUNTIME: std::sync::OnceLock<Option<tokio::runtime::Runtime>> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()
    }).as_ref()
}

impl SessionView {
    pub(super) fn create_session(&mut self) {
        self.session_id = format!("session-{}", now_micros());
        self.messages.clear();
        self.display.clear();
        self.render.message_cache.borrow_mut().clear();
        self.session_scroll = 0;
        self.view_mode = ViewMode::Session;
        self.reload_agents();
        self.session_agent = self.default_agent_id();
        if let Some(store) = &self.store {
            let _ = store.ensure_session(&self.session_id);
            let _ = store.set_session_agent(&self.session_id, self.session_agent.clone());
        }
        self.note(format!("session: created {}", self.session_id));
    }

    pub(super) fn switch_agent(&mut self, target: &str) {
        let Some(store) = &self.store else {
            self.note("session store unavailable".to_string());
            return;
        };
        let agent_id = if target == "__default__" {
            None
        } else {
            let resolved = target
                .parse::<usize>()
                .ok()
                .and_then(|index| {
                    self.agents
                        .get(index.saturating_sub(1))
                        .map(|agent| agent.id.clone())
                })
                .unwrap_or_else(|| target.to_string());
            Some(resolved)
        };
        match store.set_session_agent(&self.session_id, agent_id.clone()) {
            Ok(()) => {
                self.session_agent = agent_id.clone();
                let label = agent_id.as_deref().unwrap_or("default");
                self.note(format!("agent: {}", label));
            }
            Err(err) => self.note_error(format!("failed to set agent: {}", err)),
        }
    }

    pub(super) fn cycle_agent(&mut self) {
        let Some(store) = &self.store else {
            self.note("session store unavailable".to_string());
            return;
        };
        let agents = agent::visible_agents(&self.agents);
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
        let tools = agents[next].tools.clone();
        match store.set_session_agent(&self.session_id, agent_id.clone()) {
            Ok(()) => {
                self.session_agent = agent_id.clone();
                self.note(format!(
                    "agent: {} (tools: {})",
                    agent_id.as_deref().unwrap_or("default"),
                    tools
                ));
            }
            Err(err) => self.note_error(format!("failed to set agent: {}", err)),
        }
    }

    pub(super) fn current_session_agent(&self) -> Option<String> {
        self.session_agent.clone().or_else(|| self.default_agent_id())
    }

    /// Resolve the current session's agent from the cached `self.agents`.
    fn current_agent_info(&self) -> Option<&agent::AgentInfo> {
        let agent_id = self
            .store
            .as_ref()
            .and_then(|s| s.get_session_agent(&self.session_id).ok().flatten())
            .or_else(|| agent::default_agent_id(&self.agents))?;
        self.agents.iter().find(|a| a.id == agent_id)
    }

    pub(super) fn effective_system(&self) -> String {
        let mut system = self.system_prompt.render();
        if let Some(info) = self.current_agent_info()
            && !info.system.is_empty() {
                system.push_str("\n\n");
                system.push_str(&info.system);
            }
        system
    }

    pub(super) fn ensure_runtime_ready(&mut self) -> anyhow::Result<()> {
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
        let system_prompt = crate::system_prompt::SystemPrompt::from_config(
            &self.config,
            &resolved,
        )?;

        self.provider_name = provider_name;
        self.model = model;
        self.system_prompt = system_prompt;
        self.llm = Some(Arc::from(llm));
        Ok(())
    }

    /// Reload agents from disk (used on session create/switch).
    fn reload_agents(&mut self) {
        self.agents = agent::load_agents(&self.cwd).unwrap_or_default();
    }

    /// Hot-reload all file-based resources: config, agents, rules, AGENTS.md, skills.
    /// Triggered by `/reload` command.
    pub(super) fn reload_resources(&mut self) {
        if let Ok(config) = crate::core::config::Config::load(&self.cwd) {
            self.config = config;
        }
        self.reload_agents();

        if let Some((provider_name, model)) = self.config.resolve_provider_model()
            && let Some(resolved) = self.config.get_provider(&provider_name)
                && let Ok(prompt) =
                    crate::system_prompt::SystemPrompt::from_config(&self.config, &resolved)
                {
                    self.system_prompt = prompt;
                    self.provider_name = provider_name;
                    self.model = model;
                    if let Some(new_llm) = provider::create_provider(&resolved) {
                        self.llm = Some(Arc::from(new_llm));
                    }
                }

        let agent_count = self.agents.len();
        self.note(format!("reloaded config, agents ({agent_count}), rules, skills"));
    }

    pub(super) fn current_agent_max_steps(&self) -> u32 {
        self.current_agent_info().map_or(50, |a| a.max_steps)
    }

    pub(super) fn current_agent_tools(&self) -> String {
        self.current_agent_info()
            .map_or("all".to_string(), |a| a.tools.clone())
    }

    pub(super) fn current_context_window(&self) -> u64 {
        self.config.resolve_context_window()
    }

    pub(super) fn default_agent_id(&self) -> Option<String> {
        agent::default_agent_id(&self.agents)
    }

    pub(super) fn set_session_agent(&mut self, agent: Option<String>) {
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
                self.session_agent = resolved.clone();
                let label = resolved.as_deref().unwrap_or("default");
                self.note(format!("agent: {}", label));
            }
            Err(err) => self.note_error(format!("failed to set agent: {}", err)),
        }
    }

    pub(super) fn compact_session(&mut self, args: Vec<String>) {
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
                let Some(rt) = shared_runtime() else {
                    tracing::error!("failed to create background runtime");
                    return;
                };
                let system = agent::builtin_agent_system("compaction")
                    .unwrap_or("Summarize this conversation history. Be concise.")
                    .to_string();
                let prompt = compaction::build_compaction_prompt(
                    None,
                    &format!("Conversation history to compact:\n\n{}", older_text),
                );
                let result = rt.block_on(crate::tool::task::run_agent(
                    llm_clone.as_ref(),
                    &model,
                    &system,
                    "none",
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
                        let _ = store_clone.flush();
                    }
                    _ => {
                        tracing::warn!("compaction LLM returned empty result, skipping checkpoint");
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

    pub(super) fn generate_title(&self) {
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
            let Some(rt) = shared_runtime() else {
                tracing::error!("failed to create background runtime");
                return;
            };
            let system = agent::builtin_agent_system("title")
                .unwrap_or("Generate a concise title.")
                .to_string();
            let result = rt.block_on(crate::tool::task::run_agent(
                llm_clone.as_ref(),
                &model,
                &system,
                "none",
                3,
                None,
                vec![provider::Message::user(user_text)],
                &crate::tool::ToolContext::new(cwd),
            ));
            if let Ok(title) = result {
                let title = title.trim().chars().take(50).collect::<String>();
                if !title.is_empty() {
                    let _ = store.set_title(&session_id, title);
                    let _ = store.flush();
                }
            }
        });
    }

    pub(super) fn generate_summary(&self) {
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
            let Some(rt) = shared_runtime() else {
                tracing::error!("failed to create background runtime");
                return;
            };
            let system = agent::builtin_agent_system("summary")
                .unwrap_or("Summarize what was done.")
                .to_string();
            let result = rt.block_on(crate::tool::task::run_agent(
                llm_clone.as_ref(),
                &model,
                &system,
                "none",
                3,
                None,
                vec![provider::Message::user(conversation)],
                &crate::tool::ToolContext::new(cwd),
            ));
            if let Ok(summary) = result {
                let summary = summary.trim().to_string();
                if !summary.is_empty() {
                    let _ = store.set_summary(&session_id, summary);
                    let _ = store.flush();
                }
            }
        });
    }

    pub(super) fn switch_session(&mut self, target: &str) {
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
        let Ok(Some(session)) = store.get_session(&id) else {
            self.note_error(format!("session not found: {}", target));
            return;
        };
        let Ok(history) = store.effective_messages(&id) else {
            self.note_error(format!("failed to load session: {}", id));
            return;
        };
        self.session_id = id.clone();
        self.session_scroll = 0;
        self.view_mode = ViewMode::Session;
        self.reload_agents();

        // Restore per-session model + reasoning effort overrides.
        self.restore_session_runtime(&session);

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
        self.render.message_cache.borrow_mut().clear();
        self.note(format!("session: switched to {}", id));
    }

    /// Restore per-session model and reasoning effort overrides.
    ///
    /// If the session has a model override, switch the runtime (provider,
    /// llm, system prompt) to that model. If not, fall back to the global
    /// config model. Reasoning effort is restored from the session record
    /// (or cleared if None).
    fn restore_session_runtime(&mut self, session: &crate::core::session::Session) {
        if let Some(model_spec) = &session.model {
            let (provider_name, model_name, _) =
                crate::core::config::parse_model_spec(model_spec);
            if let Some(resolved) = self.config.get_provider(provider_name)
                && let Some(llm) = provider::create_provider(&resolved) {
                    self.provider_name = provider_name.to_string();
                    self.model = resolved
                        .models
                        .get(model_name)
                        .and_then(|m| m.name.as_deref())
                        .unwrap_or(model_name)
                        .to_string();
                    self.llm = Some(Arc::from(llm));
                    if let Ok(prompt) =
                        crate::system_prompt::SystemPrompt::from_config(&self.config, &resolved)
                    {
                        self.system_prompt = prompt;
                    }
                }
        } else {
            // No override — restore from global config.
            if let Some((provider_name, model)) = self.config.resolve_provider_model()
                && let Some(resolved) = self.config.get_provider(&provider_name)
                    && let Some(llm) = provider::create_provider(&resolved) {
                        self.provider_name = provider_name;
                        self.model = model;
                        self.llm = Some(Arc::from(llm));
                        if let Ok(prompt) =
                            crate::system_prompt::SystemPrompt::from_config(
                                &self.config,
                                &resolved,
                            ) {
                            self.system_prompt = prompt;
                        }
                    }
        }
        self.reasoning_effort = session.reasoning_effort.clone();
    }

    pub(super) fn delete_session(&mut self, target: &str) {
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
            Err(e) => self.note_error(format!("delete failed: {}", e)),
        }
    }

    pub(super) fn scroll_session_up(&mut self, lines: usize) {
        self.session_scroll = self.session_scroll.saturating_add(lines);
    }

    pub(super) fn scroll_session_down(&mut self, lines: usize) {
        self.session_scroll = self.session_scroll.saturating_sub(lines);
    }

    /// Resolve the LLM provider+model for background agents.
    /// Prefers `background_model`; falls back to the main `model`.
    /// Reuses `self.llm` when the provider is the same.
    fn resolve_background_provider(&self) -> Option<(Arc<dyn provider::LlmProvider>, String)> {
        let bg_spec = self.config.background_model.as_deref();
        let main_spec = self.config.model.as_deref();

        // No background override → use main provider
        if bg_spec.is_none() || bg_spec == main_spec {
            return self
                .llm
                .as_ref()
                .map(|l| (Arc::clone(l), self.model.clone()));
        }

        // Background model set — resolve provider+model
        let (provider_name, wire_model) = self.config.resolve_background_provider_model()?;

        // Same provider, different model → reuse existing LLM with different model name
        if provider_name == self.provider_name {
            return self
                .llm
                .as_ref()
                .map(|l| (Arc::clone(l), wire_model));
        }

        // Different provider — create new instance
        let resolved = self.config.get_provider(&provider_name)?;
        let new_llm = provider::create_provider(&resolved)?;
        Some((Arc::from(new_llm), wire_model))
    }

    /// Fire-and-forget incremental memory extraction.
    ///
    /// Reads the watermark, collects messages since the last extraction,
    /// and spawns a background thread running the `memory-extract` agent.
    /// A 5 s delay avoids competing with title/summary LLM calls.
    pub(super) fn generate_memory(&self) {
        let Some(store) = self.store.clone() else { return; };
        let session_id = self.session_id.clone();

        let Some((llm, model)) = self.resolve_background_provider() else { return; };

        let watermark = store.get_memory_watermark(&session_id).unwrap_or(0);
        let all_msgs = store.get_messages(&session_id).unwrap_or_default();
        let new_msgs: Vec<_> = all_msgs.iter().filter(|m| m.seq > watermark).collect();

        // Need at least 3 new messages to be worth extracting
        if new_msgs.len() < 3 { return; }

        let last_seq = new_msgs.last().unwrap().seq;
        let conversation = new_msgs
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        let cwd = self.cwd.clone();
        let store_clone = store.clone();

        std::thread::spawn(move || {
            // Stagger to avoid competing with title/summary on the same LLM window
            std::thread::sleep(std::time::Duration::from_secs(5));

            let Some(rt) = shared_runtime() else {
                tracing::error!("failed to create background runtime");
                return;
            };
            let system = agent::builtin_agent_system("memory-extract")
                .unwrap_or("Extract stable memories from the conversation.")
                .to_string();
            let result = rt.block_on(crate::tool::task::run_agent(
                llm.as_ref(),
                &model,
                &system,
                "[memory_read, memory_record]",
                15,
                None,
                vec![provider::Message::user(conversation)],
                &crate::tool::ToolContext::new(cwd),
            ));

            // Advance watermark regardless of success/failure so we don't
            // re-process the same messages on every turn.
            let _ = store_clone.set_memory_watermark(&session_id, last_seq);
            let _ = store_clone.flush();

            if let Err(e) = result {
                tracing::warn!("memory extraction failed: {e}");
            }
        });
    }

    /// `/dream` — manually triggered cross-session pattern extraction.
    ///
    /// Collects all session summaries (or title + last messages as fallback),
    /// then spawns a background `dreaming` agent that writes patterns to
    /// `scope=dreaming`.
    pub(super) fn dream(&mut self) {
        let Some(store) = self.store.clone() else {
            self.note("session store unavailable".to_string());
            return;
        };
        let Some((llm, model)) = self.resolve_background_provider() else {
            self.note("no LLM available for dreaming".to_string());
            return;
        };

        // Build snapshot from all sessions.
        let sessions = store.list_sessions().unwrap_or_default();
        if sessions.is_empty() {
            self.note("no sessions to analyze".to_string());
            return;
        }

        let mut snapshot = String::new();
        for s in &sessions {
            snapshot.push_str(&format!("\n## Session: {}", s.title.as_deref().unwrap_or(&s.id)));
            if let Some(ref summary) = s.summary {
                snapshot.push_str(&format!("\nSummary: {summary}\n"));
            } else {
                // Fallback: title + last 3 user/assistant messages.
                match store.get_messages(&s.id) {
                    Ok(msgs) => {
                        let tail: Vec<_> = msgs
                            .iter()
                            .filter(|m| m.role == "user" || m.role == "assistant")
                            .rev()
                            .take(3)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect();
                        if !tail.is_empty() {
                            snapshot.push('\n');
                            for m in &tail {
                                snapshot.push_str(&format!("{}: {}\n", m.role, m.content));
                            }
                        }
                    }
                    Err(_) => { /* skip unreadable sessions */ }
                }
            }
        }

        if snapshot.trim().is_empty() {
            self.note("no session content to analyze".to_string());
            return;
        }

        let cwd = self.cwd.clone();
        let session_count = sessions.len();
        self.status = "dreaming...".to_string();
        self.note(format!(
            "dreaming: analyzing {} session(s) in background...",
            session_count
        ));

        std::thread::spawn(move || {
            let Some(rt) = shared_runtime() else {
                tracing::error!("failed to create background runtime");
                return;
            };
            let system = agent::builtin_agent_system("dreaming")
                .unwrap_or("Analyze cross-session patterns.")
                .to_string();
            let prompt = format!(
                "The following is a snapshot of ALL sessions for this project.\n\
                 Analyze the user's cross-session behavioral patterns and record\n\
                 any recurring patterns to dreaming memory.\n\n{snapshot}"
            );
            let result = rt.block_on(crate::tool::task::run_agent(
                llm.as_ref(),
                &model,
                &system,
                "[memory_read, memory_record]",
                20,
                None,
                vec![provider::Message::user(prompt)],
                &crate::tool::ToolContext::new(cwd),
            ));
            if let Err(e) = result {
                tracing::warn!("dreaming failed: {e}");
            }
        });
    }
}
