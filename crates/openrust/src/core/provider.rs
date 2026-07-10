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

/// Content of a message — either a plain string or a multimodal array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl MessageContent {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn from_parts(parts: Vec<ContentPart>) -> Self {
        Self::Parts(parts)
    }

    pub fn as_text(&self) -> &str {
        match self {
            Self::Text(t) => t.as_str(),
            Self::Parts(parts) => {
                // No stable way to return &str from owned concatenation;
                // callers that need the full text should use to_text_lossy().
                parts
                    .iter()
                    .find_map(|p| p.text.as_deref())
                    .unwrap_or("")
            }
        }
    }

    pub fn to_text_lossy(&self) -> String {
        match self {
            Self::Text(t) => t.clone(),
            Self::Parts(parts) => parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<Vec<_>>()
                .join(""),
        }
    }
}

impl std::fmt::Display for MessageContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(t) => write!(f, "{}", t),
            Self::Parts(parts) => {
                for p in parts {
                    if let Some(t) = &p.text {
                        write!(f, "{}", t)?;
                    } else if p.image_url.is_some() {
                        write!(f, "[image]")?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// A single content part in a multimodal message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<ImageUrl>,
}

impl ContentPart {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            kind: "text".to_string(),
            text: Some(value.into()),
            image_url: None,
        }
    }

    pub fn image_url(url: impl Into<String>) -> Self {
        Self {
            kind: "image_url".to_string(),
            text: None,
            image_url: Some(ImageUrl { url: url.into(), detail: None }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: MessageContent::text(content),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: MessageContent::text(content),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn tool(content: impl Into<String>, tool_call_id: String) -> Self {
        Self {
            role: "tool".to_string(),
            content: MessageContent::text(content),
            name: None,
            tool_call_id: Some(tool_call_id),
            tool_calls: None,
        }
    }

    pub fn user_with_images(text: impl Into<String>, images: Vec<ContentPart>) -> Self {
        let mut parts = vec![ContentPart::text(text)];
        parts.extend(images);
        Self {
            role: "user".to_string(),
            content: MessageContent::from_parts(parts),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
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
    Finish { usage: Option<Usage>, reason: Option<String> },
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub prompt_cache_hit_tokens: u64,
    pub reasoning_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub model: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub max_tokens: Option<u32>,
    pub system: Option<String>,
    pub reasoning_effort: Option<String>,
    pub tool_choice: Option<serde_json::Value>,
}

impl RequestOptions {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            system: None,
            reasoning_effort: None,
            tool_choice: None,
        }
    }
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

    /// Whether this provider/model supports image input.
    fn supports_images(&self, model: &str) -> bool {
        model_supports_images(model)
    }
}

/// Heuristic: detect whether a model name suggests image/vision support.
pub fn model_supports_images(model: &str) -> bool {
    let lower = model.to_lowercase();
    lower.contains("gpt-4o")
        || lower.contains("gpt-4-turbo")
        || lower.contains("gpt-4-vision")
        || lower.contains("claude-3")
        || lower.contains("claude-3.5")
        || lower.contains("claude-4")
        || lower.contains("gemini")
        || lower.contains("deepseek-v4")
        || lower.contains("deepseek-chat")
        || lower.contains("deepseek-reasoner")
}

// ── Factory ────────────────────────────────────────

/// Detect the protocol for a provider: explicit config field, then base_url heuristic.
fn detect_protocol(cfg: &ResolvedProvider) -> &'static str {
    if let Some(ref p) = cfg.protocol {
        return match p.as_str() {
            "anthropic" => "anthropic",
            "gemini" | "google" => "gemini",
            _ => "openai",
        };
    }
    let base_url = cfg.base_url.as_deref().unwrap_or("");
    if base_url.contains("anthropic.com") {
        "anthropic"
    } else if base_url.contains("googleapis.com") {
        "gemini"
    } else {
        "openai"
    }
}

/// Create a provider from config, routing to the correct protocol implementation.
pub fn create_provider(cfg: &ResolvedProvider) -> Option<Box<dyn LlmProvider>> {
    match detect_protocol(cfg) {
        "anthropic" => crate::provider::anthropic::create(cfg)
            .map(|p| Box::new(p) as Box<dyn LlmProvider>),
        "gemini" => crate::provider::gemini::create(cfg)
            .map(|p| Box::new(p) as Box<dyn LlmProvider>),
        _ => crate::provider::openai_compat::create(cfg)
            .map(|p| Box::new(p) as Box<dyn LlmProvider>),
    }
}
