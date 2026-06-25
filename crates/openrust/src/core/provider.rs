//! LLM Provider abstraction.
//!
//! Provider-neutral interface for chat completion. Each provider
//! (OpenAI, Anthropic, DeepSeek, etc.) implements this trait.

use async_trait::async_trait;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::core::config::ResolvedProvider;

// ── Types ──────────────────────────────────────────

/// A chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// Tool definition passed to the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub r#type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Streaming response chunk
#[derive(Debug, Clone)]
pub enum StreamChunk {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, args: String },
    ToolCallEnd { id: String },
    Finish { usage: Option<Usage> },
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub system: Option<String>,
}

// ── Trait ──────────────────────────────────────────

/// A stream of LLM response chunks
pub type ChunkStream = Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Send a chat completion request, returning a stream of chunks
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
        options: RequestOptions,
    ) -> anyhow::Result<ChunkStream>;

    /// List available models for this provider
    fn list_models(&self) -> Vec<String>;

    /// Provider name (e.g. "openai", "deepseek")
    fn name(&self) -> &str;
}

// ── Factory ────────────────────────────────────────

/// Create a provider from config
pub fn create_provider(cfg: &ResolvedProvider) -> Option<Box<dyn LlmProvider>> {
    crate::provider::openai_compat::create(cfg)
        .map(|p| Box::new(p) as Box<dyn LlmProvider>)
}
