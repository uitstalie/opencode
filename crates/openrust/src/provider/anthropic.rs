//! Anthropic Messages API provider (native protocol).
//!
//! Speaks Anthropic's `/v1/messages` format directly:
//! - System message → top-level `system` field
//! - Tool calls → `tool_use` content blocks
//! - Tool results → `tool_result` content blocks inside user messages
//! - Streaming: message_start → content_block_start/delta/stop → message_delta → message_stop

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;

use crate::core::config::{ModelConfig, ResolvedProvider};
use crate::core::provider::{
    ChunkStream, LlmProvider, Message, RequestOptions, StreamChunk, Usage,
};

use super::{merge_options_into, retry_with_backoff};

pub struct AnthropicProvider {
    name: String,
    api_key: String,
    base_url: String,
    client: Client,
    models: HashMap<String, ModelConfig>,
    provider_options: Option<Value>,
    headers: HashMap<String, String>,
}

impl AnthropicProvider {
    pub fn new(
        name: String,
        api_key: String,
        base_url: String,
        models: HashMap<String, ModelConfig>,
        provider_options: Option<Value>,
        headers: HashMap<String, String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            name,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
            models,
            provider_options,
            headers,
        }
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<crate::core::provider::ToolDef>,
        options: RequestOptions,
    ) -> anyhow::Result<ChunkStream> {
        let url = format!("{}/messages", self.base_url);

        // Split system messages from conversation messages and transform to Anthropic format.
        let mut system_text = String::new();
        let mut conv: Vec<&Message> = Vec::new();
        for msg in &messages {
            if msg.role == "system" {
                if !system_text.is_empty() {
                    system_text.push_str("\n\n");
                }
                system_text.push_str(&msg.content.to_text_lossy());
            } else {
                conv.push(msg);
            }
        }
        if let Some(ref sys) = options.system {
            if !system_text.is_empty() {
                system_text.push_str("\n\n");
            }
            system_text.push_str(sys);
        }

        let mut body = serde_json::json!({
            "model": options.model,
            "messages": lower_messages(&conv),
            "max_tokens": options.max_tokens.unwrap_or(16384),
            "stream": true,
        });

        if !system_text.is_empty() {
            body["system"] = serde_json::json!([{"type": "text", "text": system_text}]);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(
                tools.iter().map(|t| serde_json::json!({
                    "name": t.function.name,
                    "description": t.function.description,
                    "input_schema": t.function.parameters,
                })).collect::<Vec<_>>()
            );
        }
        if let Some(temp) = options.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(top_p) = options.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
        if let Some(ref tc) = options.tool_choice {
            body["tool_choice"] = tc.clone();
        }
        if let Some(ref effort) = options.reasoning_effort {
            let budget = match effort.as_str() {
                "low" => 8_000,
                "high" => 32_000,
                _ => 16_000,
            };
            body["thinking"] = serde_json::json!({"type": "enabled", "budget_tokens": budget});
        }

        // Deep-merge config options.
        if let Some(ref opts) = self.provider_options {
            merge_options_into(&mut body, opts);
        }
        if let Some(model_cfg) = self.models.get(&options.model)
            && let Some(ref opts) = model_cfg.options
        {
            merge_options_into(&mut body, opts);
        }

        tracing::debug!("POST {} (model={})", url, options.model);

        let mut headers = self.headers.clone();
        if let Some(model_cfg) = self.models.get(&options.model) {
            for (k, v) in &model_cfg.headers {
                headers.insert(k.clone(), v.clone());
            }
        }

        let response = retry_with_backoff(3, || {
            let client = &self.client;
            let url = &url;
            let api_key = &self.api_key;
            let body = &body;
            let headers = &headers;
            async move {
                let mut req = client
                    .post(url)
                    .header("x-api-key", api_key)
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json");
                for (key, value) in headers {
                    req = req.header(key, value);
                }
                req.json(body).send().await
            }
        })
        .await?;

        let stream = response.bytes_stream();
        let chunk_stream = async_stream::stream! {
            let mut buffer = String::new();
            // Tracks content block info by index: (kind, id_for_tool_use)
            let mut blocks: HashMap<usize, BlockInfo> = HashMap::new();
            let mut input_tokens: u64 = 0;
            let mut output_tokens: u64 = 0;
            let mut cache_read: u64 = 0;
            let mut cache_create: u64 = 0;
            let mut finish_emitted = false;

            for await chunk in stream {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(anyhow::anyhow!("Stream error: {}", e));
                        break;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer = buffer[pos + 1..].to_string();
                    if line.is_empty() || line.starts_with(':') { continue; }
                    let Some(data) = line.strip_prefix("data:") else { continue; };
                    let data = data.strip_prefix(' ').unwrap_or(data);
                    if data == "[DONE]" { break; }

                    let parsed: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    let event_type = parsed["type"].as_str().unwrap_or("");
                    match event_type {
                        "message_start" => {
                            if let Some(usage) = parsed["message"]["usage"].as_object() {
                                input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                cache_read = usage.get("cache_read_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                                cache_create = usage.get("cache_creation_input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
                            }
                        }
                        "content_block_start" => {
                            let index = parsed["index"].as_u64().unwrap_or(0) as usize;
                            let block = &parsed["content_block"];
                            let kind = block["type"].as_str().unwrap_or("text").to_string();
                            let id = block["id"].as_str().unwrap_or("").to_string();
                            let name = block["name"].as_str().unwrap_or("").to_string();
                            blocks.insert(index, BlockInfo { kind, id, name });

                            // Emit initial text if present.
                            if let Some(text) = block["text"].as_str()
                                && !text.is_empty()
                            {
                                yield Ok(StreamChunk::TextDelta(text.to_string()));
                            }
                            // Emit ToolCallStart for tool_use blocks.
                            if block["type"].as_str() == Some("tool_use") {
                                let bi = blocks.get(&index).unwrap();
                                yield Ok(StreamChunk::ToolCallStart {
                                    id: bi.id.clone(),
                                    name: bi.name.clone(),
                                });
                            }
                        }
                        "content_block_delta" => {
                            let index = parsed["index"].as_u64().unwrap_or(0) as usize;
                            let delta = &parsed["delta"];
                            match delta["type"].as_str() {
                                Some("text_delta") => {
                                    if let Some(text) = delta["text"].as_str() {
                                        yield Ok(StreamChunk::TextDelta(text.to_string()));
                                    }
                                }
                                Some("thinking_delta") => {
                                    if let Some(text) = delta["thinking"].as_str() {
                                        yield Ok(StreamChunk::ReasoningDelta(text.to_string()));
                                    }
                                }
                                Some("input_json_delta") => {
                                    if let Some(partial) = delta["partial_json"].as_str()
                                        && let Some(bi) = blocks.get(&index)
                                    {
                                        yield Ok(StreamChunk::ToolCallDelta {
                                            id: bi.id.clone(),
                                            args: partial.to_string(),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                        "content_block_stop" => {
                            let index = parsed["index"].as_u64().unwrap_or(0) as usize;
                            if let Some(bi) = blocks.get(&index)
                                && bi.kind == "tool_use"
                            {
                                yield Ok(StreamChunk::ToolCallEnd {
                                    id: bi.id.clone(),
                                });
                            }
                        }
                        "message_delta" => {
                            let stop_reason = parsed["delta"]["stop_reason"].as_str();
                            if let Some(usage) = parsed["usage"].as_object() {
                                output_tokens = usage.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(output_tokens);
                            }
                            let reason = stop_reason.map(|r| match r {
                                "end_turn" | "stop_sequence" | "pause_turn" => "stop",
                                "max_tokens" => "length",
                                "tool_use" => "tool_calls",
                                _ => r,
                            }).map(String::from);
                            finish_emitted = true;
                            yield Ok(StreamChunk::Finish {
                                usage: Some(Usage {
                                    prompt_tokens: input_tokens + cache_read + cache_create,
                                    completion_tokens: output_tokens,
                                    total_tokens: input_tokens + cache_read + cache_create + output_tokens,
                                    prompt_cache_hit_tokens: cache_read,
                                    reasoning_tokens: 0,
                                }),
                                reason,
                            });
                        }
                        "message_stop" if !finish_emitted => {
                            yield Ok(StreamChunk::Finish { usage: None, reason: None });
                        }
                        _ => {}
                    }
                }
            }

            if !finish_emitted {
                yield Ok(StreamChunk::Finish { usage: None, reason: None });
            }
        };

        Ok(Box::pin(chunk_stream))
    }

    fn list_models(&self) -> Vec<String> {
        self.models.keys().cloned().collect()
    }

    fn name(&self) -> &str {
        &self.name
    }
}

struct BlockInfo {
    kind: String,
    id: String,
    name: String,
}

/// Transform OpenAI-format messages into Anthropic content-block format.
/// Tool result messages (role: "tool") are merged into user messages as
/// tool_result content blocks.
fn lower_messages(messages: &[&Message]) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    // Accumulated tool_result blocks waiting to be flushed as a user message.
    let mut pending_tool_results: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "tool" => {
                let tool_use_id = msg.tool_call_id.clone().unwrap_or_default();
                let content = msg.content.to_text_lossy();
                pending_tool_results.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": content,
                }));
            }
            "user" => {
                if !pending_tool_results.is_empty() {
                    result.push(serde_json::json!({
                        "role": "user",
                        "content": pending_tool_results,
                    }));
                    pending_tool_results = Vec::new();
                }
                result.push(serde_json::json!({
                    "role": "user",
                    "content": [{"type": "text", "text": msg.content.to_text_lossy()}],
                }));
            }
            "assistant" => {
                if !pending_tool_results.is_empty() {
                    result.push(serde_json::json!({
                        "role": "user",
                        "content": pending_tool_results,
                    }));
                    pending_tool_results = Vec::new();
                }
                let mut blocks: Vec<Value> = Vec::new();
                let text = msg.content.to_text_lossy();
                if !text.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        let input: Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::json!({}));
                        blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.function.name,
                            "input": input,
                        }));
                    }
                }
                if !blocks.is_empty() {
                    result.push(serde_json::json!({"role": "assistant", "content": blocks}));
                }
            }
            _ => {}
        }
    }

    if !pending_tool_results.is_empty() {
        result.push(serde_json::json!({
            "role": "user",
            "content": pending_tool_results,
        }));
    }

    result
}

/// Create an Anthropic provider from config.
pub fn create(cfg: &ResolvedProvider) -> Option<AnthropicProvider> {
    let api_key = cfg.api_key.clone()?;
    let base_url = cfg
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());

    Some(AnthropicProvider::new(
        cfg.name.clone(),
        api_key,
        base_url,
        cfg.models.clone(),
        cfg.options.clone(),
        cfg.headers.clone(),
    ))
}
