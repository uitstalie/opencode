//! Provider switching, model selection, and config persistence.

use std::sync::Arc;

use futures::StreamExt;

use super::SessionView;
use super::dialog::{Dialog, DialogKind, DialogOption};
use crate::core::{
    config::Config,
    provider::{self, Message, MessageContent, RequestOptions, StreamChunk},
};

impl SessionView {
    pub(super) fn switch_provider(&mut self, provider: &str) {
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

    /// Begin a guided provider switch: always show the API key prompt
    /// (empty submit = reuse existing key), then let the user pick a model.
    pub(super) fn begin_provider_switch(&mut self, provider: &str) {
        if !self.config.provider.contains_key(provider) {
            self.note(format!("provider not configured: {}", provider));
            return;
        }
        self.ui.pending_provider = Some(provider.to_string());

        let has_key = self
            .config
            .get_provider(provider)
            .is_some_and(|r| r.api_key.is_some());

        let description = if has_key {
            format!(
                "输入 {} 的新 API key，或直接回车复用已有 key。",
                provider,
            )
        } else {
            format!(
                "输入 {} 的 API key。",
                provider,
            )
        };

        self.ui.pending_text_input = Some(super::types::PendingTextInput {
            title: format!("{} · API key", provider),
            description,
            value: String::new(),
            editor: super::util::single_line_textarea("", true),
            submit: SessionView::save_switch_api_key,
        });
        self.status = format!("connect: API key for {}", provider);
    }

    /// Save the API key entered during guided switch, then show model dialog.
    fn save_switch_api_key(&mut self, value: &str) {
        let Some(provider) = self.ui.pending_provider.clone() else {
            self.note("provider switch state missing".to_string());
            return;
        };
        let key = value.trim();
        if !key.is_empty()
            && let Err(err) = crate::core::vault::Vault::save(&provider, key) {
                self.note(format!("failed to save API key: {}", err));
            }
        self.reload_config();
        self.open_provider_model_dialog();
    }

    /// Show a model-selection dialog scoped to the pending provider.
    fn open_provider_model_dialog(&mut self) {
        let Some(provider) = self.ui.pending_provider.clone() else {
            self.note("provider switch state missing".to_string());
            return;
        };
        let Some(cfg) = self.config.provider.get(&provider) else {
            self.note(format!("provider not configured: {}", provider));
            return;
        };
        let mut models: Vec<_> = cfg.models.keys().cloned().collect();
        models.sort();

        let current = self
            .config
            .model
            .as_deref()
            .filter(|m| m.starts_with(&format!("{}/", provider)));

        let mut options = models
            .into_iter()
            .map(|model| {
                let spec = format!("{}/{}", provider, model);
                let active = if Some(spec.as_str()) == current { "● " } else { "" };
                DialogOption::new(
                    spec.clone(),
                    format!("{}{}", active, model),
                    "切换到这个模型。",
                )
            })
            .collect::<Vec<_>>();
        options.push(DialogOption::new(
            "__reasoning__",
            "Thinking effort".to_string(),
            "设置 reasoning effort：low / medium / high / off。",
        ));

        self.ui.dialog = Some(Dialog::new(
            DialogKind::ProviderModel,
            format!("{} · model", provider),
            "选择模型，或设置 thinking effort。",
            options,
            0,
        ));
        self.ui.pending_text_input = None;
        self.status = format!("connect: model for {}", provider);
    }

    /// Finalize the guided switch: set the chosen model, then offer
    /// thinking-effort selection if the model supports reasoning.
    pub(super) fn switch_provider_model(&mut self, spec: &str) {
        self.config.model = Some(spec.to_string());
        match self.save_global_config() {
            Ok(()) => {
                self.reload_config();
                self.persist_session_model();
                self.note(format!(
                    "provider: {} · model: {}",
                    self.provider_name, self.model
                ));
            }
            Err(err) => self.note(format!("failed to save model: {}", err)),
        }
        self.ui.pending_provider = None;

        // Chain into thinking-effort selection if the model supports reasoning.
        let supports_reasoning = self
            .config
            .provider
            .get(&self.provider_name)
            .and_then(|p| p.models.get(&self.model))
            .is_some_and(|m| {
                m.reasoning_options.is_some() || m.reasoning_send_effort.unwrap_or(false)
            });
        if supports_reasoning {
            self.open_reasoning_dialog(None);
        }
    }

    pub(super) fn verify_provider(&mut self, provider_name: &str) {
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
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(err) => {
                self.note(format!("runtime error: {}", err));
                return;
            }
        };
        let result = rt.block_on(async {
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
                            top_p: None,
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
            });
        match result {
            Ok(()) => self.note(format!("provider verified: {}", provider_name)),
            Err(err) => self.note(format!("provider verify failed: {}", err)),
        }
    }

    pub(super) fn switch_model(&mut self, spec: &str) {
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
                self.persist_session_model();
                self.note(format!("model: {}/{}", self.provider_name, self.model));
                // Chain into thinking-effort selection if the model supports reasoning.
                let supports_reasoning = self
                    .config
                    .provider
                    .get(&self.provider_name)
                    .and_then(|p| p.models.get(&self.model))
                    .is_some_and(|m| {
                        m.reasoning_options.is_some() || m.reasoning_send_effort.unwrap_or(false)
                    });
                if supports_reasoning {
                    self.open_reasoning_dialog(None);
                }
            }
            Err(err) => self.note(format!("failed to save model: {}", err)),
        }
    }

    pub(super) fn set_reasoning_effort(&mut self, effort: &str) {
        self.reasoning_effort = match effort {
            "off" | "none" => None,
            "low" | "medium" | "high" => Some(effort.to_string()),
            _ => {
                self.note("usage: /models thinking <low|medium|high|off>".to_string());
                return;
            }
        };
        self.persist_session_reasoning();
        self.note(format!(
            "model thinking effort: {}",
            self.reasoning_effort.as_deref().unwrap_or("off")
        ));
    }

    pub(super) fn reload_config(&mut self) {
        let Ok(config) = Config::load(&self.cwd) else {
            return;
        };
        match config.resolve_provider_model() {
            Some((provider_name, model)) => {
                let resolved = match config.get_provider(&provider_name) {
                    Some(r) => r,
                    None => {
                        self.config = config;
                        self.note(format!("provider '{}' not found in config", provider_name));
                        return;
                    }
                };
                match provider::create_provider(&resolved) {
                    Some(llm) => {
                        self.provider_name = provider_name;
                        self.model = model;
                        if let Ok(prompt) = crate::system_prompt::SystemPrompt::from_config(
                            &config,
                            &resolved,
                        ) {
                            self.system_prompt = prompt;
                        }
                        self.llm = Some(Arc::from(llm));
                    }
                    None => {
                        let reason = if resolved.api_key.is_none() {
                            "no API key"
                        } else {
                            "unknown protocol or misconfigured provider"
                        };
                        self.note(format!(
                            "failed to create provider '{}': {} — use /connect to configure",
                            provider_name, reason
                        ));
                    }
                }
            }
            None => {
                self.note("no model configured — use /connect to set up a provider".to_string());
            }
        }
        self.config = config;
    }

    pub(super) fn save_global_config(&self) -> anyhow::Result<()> {
        self.config.save_global()
    }

    /// Persist current model to the active session record.
    fn persist_session_model(&self) {
        if let Some(store) = &self.store {
            let spec = format!("{}/{}", self.provider_name, self.model);
            let _ = store.set_session_model(&self.session_id, Some(spec));
        }
    }

    /// Persist current reasoning effort to the active session record.
    fn persist_session_reasoning(&self) {
        if let Some(store) = &self.store {
            let _ =
                store.set_session_reasoning(&self.session_id, self.reasoning_effort.clone());
        }
    }

}
