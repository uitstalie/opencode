//! OpenAI-compatible provider.
//!
//! Works with any API that follows the OpenAI `/v1/chat/completions` format:
//! - OpenAI (api.openai.com)
//! - DeepSeek (api.deepseek.com)
//! - OneRoute (api.1route.dev)
//! - Any custom OpenAI-compatible endpoint

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::core::config::ResolvedProvider;
use crate::core::provider::{
    ChunkStream, LlmProvider, Message, RequestOptions, StreamChunk, Usage,
};

pub struct OpenAICompatProvider {
    name: String,
    api_key: String,
    base_url: String,
    client: Client,
    models: Vec<String>,
}

impl OpenAICompatProvider {
    pub fn new(name: String, api_key: String, base_url: String, models: Vec<String>) -> Self {
        Self {
            name,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::new(),
            models,
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAICompatProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<crate::core::provider::ToolDef>,
        options: RequestOptions,
    ) -> anyhow::Result<ChunkStream> {
        let url = format!("{}/chat/completions", self.base_url);

        let mut body = serde_json::json!({
            "model": options.model,
            "messages": messages.iter().map(|m| {
                let mut message = serde_json::json!({
                    "role": m.role,
                    "content": m.content,
                });
                if let Some(name) = &m.name {
                    message["name"] = serde_json::json!(name);
                }
                if let Some(tool_call_id) = &m.tool_call_id {
                    message["tool_call_id"] = serde_json::json!(tool_call_id);
                }
                if let Some(tool_calls) = &m.tool_calls {
                    message["tool_calls"] = serde_json::json!(tool_calls);
                }
                message
            }).collect::<Vec<_>>(),
            "stream": true,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }

        if let Some(temp) = options.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(max_tok) = options.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tok);
        }
        if let Some(effort) = options.reasoning_effort {
            body["reasoning_effort"] = serde_json::json!(effort);
        }
        if let Some(ref system) = options.system {
            // Insert system message at the beginning
            if let Some(arr) = body["messages"].as_array_mut() {
                arr.insert(
                    0,
                    serde_json::json!({
                        "role": "system",
                        "content": system,
                    }),
                );
            }
        }

        tracing::debug!("POST {} (model={})", url, options.model);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Provider {} returned HTTP {}: {}",
                self.name,
                status,
                text
            ));
        }

        let stream = response.bytes_stream();

        let chunk_stream = async_stream::stream! {
            let mut tool_call_id: Option<String> = None;
            let mut buffer = String::new();

            for await chunk in stream {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(anyhow::anyhow!("Stream error: {}", e));
                        break;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Parse SSE lines
                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer = buffer[pos + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') { continue; }
                    if !line.starts_with("data: ") { continue; }

                    let data = &line[6..];
                    if data == "[DONE]" {
                        yield Ok(StreamChunk::Finish { usage: None });
                        break;
                    }

                    let parsed: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("Failed to parse SSE: {}", e);
                            continue;
                        }
                    };

                    let choices = parsed["choices"].as_array();
                    if choices.is_none() { continue; }

                    for choice in choices.unwrap() {
                        let delta = &choice["delta"];

                        // Tool calls
                        if let Some(tool_calls) = delta["tool_calls"].as_array() {
                            for tc in tool_calls {
                                let id = tc["id"].as_str().unwrap_or("").to_string();
                                let fn_name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                                let args = tc["function"]["arguments"].as_str().unwrap_or("");

                                if !id.is_empty() && tool_call_id.as_deref() != Some(&id) {
                                    if tool_call_id.is_some() {
                                        yield Ok(StreamChunk::ToolCallEnd {
                                            id: tool_call_id.take().unwrap(),
                                        });
                                    }
                                    tool_call_id = Some(id.clone());
                                    yield Ok(StreamChunk::ToolCallStart {
                                        id: id.clone(),
                                        name: fn_name.clone(),
                                    });
                                }
                                if !args.is_empty() {
                                    yield Ok(StreamChunk::ToolCallDelta {
                                        id: tool_call_id.clone().unwrap_or_default(),
                                        args: args.to_string(),
                                    });
                                }
                            }
                        }

                        // Text content
                        if let Some(content) = delta["content"].as_str() {
                            if !content.is_empty() {
                                yield Ok(StreamChunk::TextDelta(content.to_string()));
                            }
                        }

                        // Reasoning content
                        if let Some(reasoning) = delta["reasoning_content"].as_str() {
                            if !reasoning.is_empty() {
                                yield Ok(StreamChunk::ReasoningDelta(reasoning.to_string()));
                            }
                        }

                        // Finish reason
                        if choice["finish_reason"].as_str().is_some() {
                            let usage = parsed["usage"].as_object().map(|u| Usage {
                                prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                completion_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                            });
                            if tool_call_id.is_some() {
                                yield Ok(StreamChunk::ToolCallEnd {
                                    id: tool_call_id.take().unwrap(),
                                });
                            }
                            yield Ok(StreamChunk::Finish { usage });
                        }
                    }
                }
            }

            // Flush any pending tool call
            if tool_call_id.is_some() {
                yield Ok(StreamChunk::ToolCallEnd {
                    id: tool_call_id.take().unwrap(),
                });
            }
        };

        Ok(Box::pin(chunk_stream))
    }

    fn list_models(&self) -> Vec<String> {
        self.models.clone()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Create an OpenAI-compatible provider from config
pub fn create(cfg: &ResolvedProvider) -> Option<OpenAICompatProvider> {
    let api_key = cfg.api_key.clone()?;
    let base_url = cfg
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    let models: Vec<String> = cfg.models.keys().cloned().collect();

    Some(OpenAICompatProvider::new(
        cfg.name.clone(),
        api_key,
        base_url,
        models,
    ))
}
