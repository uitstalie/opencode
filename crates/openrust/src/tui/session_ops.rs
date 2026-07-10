//! Session management, agent switching, compaction, and summary generation.

use std::sync::Arc;

use super::render;
use super::util::now_micros;
use super::{SessionView, ViewMode};
use crate::core::agent;
use crate::core::compaction;
use crate::core::provider::{self, Message, MessageContent};

impl SessionView {
    pub(super) fn create_session(&mut self) {
        self.session_id = format!("session-{}", now_micros());
        self.messages.clear();
        self.display.clear();
        self.session_scroll = 0;
        self.view_mode = ViewMode::Session;
        self.reload_agents();
        if let Some(store) = &self.store {
            let agent = self.default_agent_id();
            let _ = store.ensure_session(&self.session_id);
            let _ = store.set_session_agent(&self.session_id, agent.clone());
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
                let label = agent_id.as_deref().unwrap_or("default");
                self.note(format!("agent: {}", label));
            }
            Err(err) => self.note(format!("failed to set agent: {}", err)),
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
            Ok(()) => self.note(format!(
                "agent: {} (tools: {})",
                agent_id.as_deref().unwrap_or("default"),
                tools
            )),
            Err(err) => self.note(format!("failed to set agent: {}", err)),
        }
    }

    pub(super) fn current_session_agent(&self) -> Option<String> {
        self.store
            .as_ref()
            .and_then(|store| store.get_session_agent(&self.session_id).ok().flatten())
            .or_else(|| self.default_agent_id())
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
                let label = resolved.as_deref().unwrap_or("default");
                self.note(format!("agent: {}", label));
            }
            Err(err) => self.note(format!("failed to set agent: {}", err)),
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
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(_) => return,
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
        self.reload_agents();
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
            Err(e) => self.note(format!("delete failed: {}", e)),
        }
    }

    pub(super) fn scroll_session_up(&mut self, lines: usize) {
        self.session_scroll = self.session_scroll.saturating_add(lines);
    }

    pub(super) fn scroll_session_down(&mut self, lines: usize) {
        self.session_scroll = self.session_scroll.saturating_sub(lines);
    }
}
