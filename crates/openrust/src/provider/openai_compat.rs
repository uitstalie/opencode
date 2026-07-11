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

use crate::core::config::{ModelConfig, ResolvedProvider};
use crate::core::provider::{
    ChunkStream, LlmProvider, Message, MessageContent, RequestOptions, StreamChunk, Usage,
};
use std::collections::HashMap;

use super::{merge_options_into, retry_with_backoff};

pub struct OpenAICompatProvider {
    name: String,
    api_key: String,
    base_url: String,
    client: Client,
    models: HashMap<String, ModelConfig>,
    provider_options: Option<Value>,
    headers: HashMap<String, String>,
}

impl OpenAICompatProvider {
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
                    "content": serialize_content(&m.content),
                });
                if let Some(name) = &m.name {
                    message["name"] = serde_json::json!(name);
                }
                if let Some(tool_call_id) = &m.tool_call_id {
                    message["tool_call_id"] = serde_json::json!(tool_call_id);
                }
                if let Some(tool_calls) = &m.tool_calls {
                    message["tool_calls"] = serde_json::json!(tool_calls);
                    if m.role == "assistant" && m.content.to_text_lossy().is_empty() {
                        message["content"] = serde_json::Value::Null;
                    }
                }
                message
            }).collect::<Vec<_>>(),
            "stream": true,
            "stream_options": serde_json::json!({ "include_usage": true }),
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }

        // Deep-merge config options (provider-level first, then model-level overrides).
        if let Some(ref opts) = self.provider_options {
            merge_options_into(&mut body, opts);
        }
        if let Some(model_cfg) = self.models.get(&options.model)
            && let Some(ref opts) = model_cfg.options
        {
            merge_options_into(&mut body, opts);
        }

        if let Some(temp) = options.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(top_p) = options.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }

        // All provider-specific behaviour comes from ModelConfig, not hardcoded
        // prefix matching. Defaults match standard OpenAI format.
        let model_cfg = self.models.get(&options.model);
        let max_tokens_key = model_cfg
            .and_then(|c| c.max_tokens_key.as_deref())
            .unwrap_or("max_tokens");
        let system_role = model_cfg
            .and_then(|c| c.system_role.as_deref())
            .unwrap_or("system");
        let send_effort = model_cfg
            .and_then(|c| c.reasoning_send_effort)
            .unwrap_or(true);
        let reasoning_opts = model_cfg.and_then(|c| c.reasoning_options.as_ref());

        if let Some(max_tok) = options.max_tokens {
            body[max_tokens_key] = serde_json::json!(max_tok);
        }

        // When reasoning is on, merge provider-specific body fields from
        // `reasoning_options`, then optionally send the effort level.
        if options.reasoning_effort.is_some() {
            if let Some(opts) = reasoning_opts {
                merge_options_into(&mut body, opts);
            }
            if send_effort
                && let Some(ref effort) = options.reasoning_effort
            {
                body["reasoning_effort"] = serde_json::json!(effort);
            }
        }

        if let Some(ref tc) = options.tool_choice {
            body["tool_choice"] = tc.clone();
        }
        if let Some(ref system) = options.system
            && let Some(arr) = body["messages"].as_array_mut()
        {
            arr.insert(
                0,
                serde_json::json!({
                    "role": system_role,
                    "content": system,
                }),
            );
        }

        tracing::debug!("POST {} (model={})", url, options.model);

        // Merge provider-level headers with model-level overrides.
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
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("Content-Type", "application/json");
                for (key, value) in headers {
                    req = req.header(key, value);
                }
                req.json(body).send().await
            }
        })
        .await?;

        let stream = response.bytes_stream();

        let chunk_stream = async_stream::stream! {
            // Tracks active tool calls by (streaming index, call id).
            let mut call_ids: Vec<(usize, String)> = Vec::new();
            // Args received before the id-bearing delta for an index —
            // flushed once the id arrives (some providers send args first).
            let mut pending_args: Vec<(usize, String)> = Vec::new();
            let mut buffer = String::new();
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

                // Parse SSE lines
                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer = buffer[pos + 1..].to_string();

                    if line.is_empty() || line.starts_with(':') { continue; }
                    // SSE spec: data: optionally followed by a single space.
                    let Some(data) = line.strip_prefix("data:") else { continue; };
                    let data = data.strip_prefix(' ').unwrap_or(data);
                    if data == "[DONE]" {
                        for (_, id) in &call_ids {
                            yield Ok(StreamChunk::ToolCallEnd {
                                id: id.clone(),
                            });
                        }
                        call_ids.clear();
                        if !finish_emitted {
                            yield Ok(StreamChunk::Finish { usage: None, reason: None });
                        }
                        break;
                    }

                    let parsed: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            tracing::warn!("Failed to parse SSE: {}", e);
                            continue;
                        }
                    };

                    let Some(choices) = parsed["choices"].as_array() else {
                        continue;
                    };

                    for choice in choices {
                        let delta = &choice["delta"];

                        // Tool calls
                        if let Some(tool_calls) = delta["tool_calls"].as_array() {
                            for tc in tool_calls {
                                let index = tc["index"].as_u64().unwrap_or(0) as usize;
                                let id = tc["id"].as_str().unwrap_or("").to_string();
                                let fn_name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                                let args = tc["function"]["arguments"].as_str().unwrap_or("");

                                if !id.is_empty() {
                                    call_ids.push((index, id.clone()));
                                    yield Ok(StreamChunk::ToolCallStart {
                                        id: id.clone(),
                                        name: fn_name,
                                    });
                                    // Flush any args that arrived before the id.
                                    let flush: Vec<String> = pending_args
                                        .iter()
                                        .filter(|(idx, _)| *idx == index)
                                        .map(|(_, a)| a.clone())
                                        .collect();
                                    pending_args.retain(|(idx, _)| *idx != index);
                                    for a in flush {
                                        yield Ok(StreamChunk::ToolCallDelta {
                                            id: id.clone(),
                                            args: a,
                                        });
                                    }
                                }

                                if !args.is_empty() {
                                    let routed_id = call_ids
                                        .iter()
                                        .rev()
                                        .find(|(idx, _)| *idx == index)
                                        .map(|(_, id)| id.clone());
                                    match routed_id {
                                        Some(rid) => yield Ok(StreamChunk::ToolCallDelta {
                                            id: rid,
                                            args: args.to_string(),
                                        }),
                                        None => pending_args.push((index, args.to_string())),
                                    }
                                }
                            }
                        }

                        // Text content
                        if let Some(content) = delta["content"].as_str()
                            && !content.is_empty() {
                                yield Ok(StreamChunk::TextDelta(content.to_string()));
                            }

                        // Reasoning content (DeepSeek uses reasoning_content, o1 uses reasoning)
                        for key in &["reasoning_content", "reasoning"] {
                            if let Some(reasoning) = delta[key].as_str()
                                && !reasoning.is_empty() {
                                    yield Ok(StreamChunk::ReasoningDelta(reasoning.to_string()));
                                }
                            }

                        // Finish reason
                        if let Some(reason) = choice["finish_reason"].as_str() {
                            let usage = parsed["usage"].as_object().map(|u| {
                                let cache_hit = u
                                    .get("prompt_cache_hit_tokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or_else(|| {
                                        u.get("prompt_tokens_details")
                                            .and_then(|d| d.get("cached_tokens"))
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0)
                                    });
                                let reasoning = u
                                    .get("completion_tokens_details")
                                    .and_then(|d| d.get("reasoning_tokens"))
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                Usage {
                                    prompt_tokens: u.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                    completion_tokens: u.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                    total_tokens: u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                                    prompt_cache_hit_tokens: cache_hit,
                                    reasoning_tokens: reasoning,
                                }
                            });
                            for (_, id) in &call_ids {
                                yield Ok(StreamChunk::ToolCallEnd {
                                    id: id.clone(),
                                });
                            }
                            call_ids.clear();
                            finish_emitted = true;
                            yield Ok(StreamChunk::Finish {
                                usage,
                                reason: Some(reason.to_string()),
                            });
                        }
                    }
                }
            }

            // Flush any pending tool calls (connection-drop safety net)
            for (_, id) in &call_ids {
                yield Ok(StreamChunk::ToolCallEnd {
                    id: id.clone(),
                });
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

/// Create an OpenAI-compatible provider from config
pub fn create(cfg: &ResolvedProvider) -> Option<OpenAICompatProvider> {
    let api_key = cfg.api_key.clone()?;
    let base_url = cfg
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());

    Some(OpenAICompatProvider::new(
        cfg.name.clone(),
        api_key,
        base_url,
        cfg.models.clone(),
        cfg.options.clone(),
        cfg.headers.clone(),
    ))
}

/// Serialize message content — plain string or multimodal content array.
fn serialize_content(content: &MessageContent) -> serde_json::Value {
    match content {
        MessageContent::Text(text) => serde_json::json!(text),
        MessageContent::Parts(parts) => serde_json::json!(parts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_adds_new_keys() {
        let mut target = serde_json::json!({"a": 1});
        merge_options_into(&mut target, &serde_json::json!({"b": 2}));
        assert_eq!(target, serde_json::json!({"a": 1, "b": 2}));
    }

    #[test]
    fn merge_scalar_replaces_scalar() {
        let mut target = serde_json::json!({"temperature": 0.5});
        merge_options_into(&mut target, &serde_json::json!({"temperature": 0.9}));
        assert_eq!(target["temperature"], 0.9);
    }

    #[test]
    fn merge_deep_combines_nested_objects() {
        let mut target = serde_json::json!({"thinking": {"type": "enabled"}});
        merge_options_into(&mut target, &serde_json::json!({"thinking": {"budget": 8000}}));
        assert_eq!(
            target["thinking"],
            serde_json::json!({"type": "enabled", "budget": 8000})
        );
    }

    #[test]
    fn merge_scalar_overrides_object() {
        let mut target = serde_json::json!({"store": {"k": "v"}});
        merge_options_into(&mut target, &serde_json::json!({"store": false}));
        assert_eq!(target["store"], false);
    }
}
