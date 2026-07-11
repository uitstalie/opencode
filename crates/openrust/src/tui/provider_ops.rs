//! Provider switching, model selection, and config persistence.

use std::sync::Arc;

use futures::StreamExt;

use super::SessionView;
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
                self.note(format!("model: {}/{}", self.provider_name, self.model));
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
        self.note(format!(
            "model thinking effort: {}",
            self.reasoning_effort.as_deref().unwrap_or("off")
        ));
    }

    pub(super) fn reload_config(&mut self) {
        if let Ok(config) = Config::load(&self.cwd) {
            if let Some((provider_name, model)) = config.resolve_provider_model()
                && let Some(resolved) = config.get_provider(&provider_name)
                    && let Some(llm) = provider::create_provider(&resolved) {
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
            self.config = config;
        }
    }

    pub(super) fn save_global_config(&self) -> anyhow::Result<()> {
        self.config.save_global()
    }

}
