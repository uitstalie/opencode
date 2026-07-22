//! Google Gemini API provider (native protocol).
//!
//! Speaks Gemini's `streamGenerateContent` format:
//! - Messages → `contents` with `role` (user/model) and `parts`
//! - System → top-level `systemInstruction`
//! - Tool calls → `functionCall` parts (complete, not streamed incrementally)
//! - Tool results → `functionResponse` parts inside user messages
//! - Generation params nested in `generationConfig`

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

use crate::core::config::{ModelConfig, ResolvedProvider};
use crate::core::provider::{
    ChunkStream, LlmProvider, Message, RequestOptions, StreamChunk, Usage,
};

use super::{merge_options_into, normalize_options, retry_with_backoff, sse_data_lines};

pub struct GeminiProvider {
    base: super::ProviderBase,
}

impl GeminiProvider {
    pub fn new(
        name: String,
        api_key: String,
        base_url: String,
        models: HashMap<String, ModelConfig>,
        provider_options: Option<Value>,
        headers: HashMap<String, String>,
    ) -> Self {
        Self {
            base: super::ProviderBase::new(name, api_key, base_url, models, provider_options, headers),
        }
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[crate::core::provider::ToolDef],
        options: RequestOptions,
    ) -> anyhow::Result<ChunkStream> {
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.base.base_url, options.model
        );

        let mut system_text = String::new();
        let mut conv: Vec<&Message> = Vec::new();
        for msg in messages {
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
            "contents": lower_messages(&conv),
        });

        if !system_text.is_empty() {
            body["systemInstruction"] =
                serde_json::json!({"parts": [{"text": system_text}]});
        }

        // Inject per-message options (e.g. cache markers for proxy/relay services).
        if let Some(msg_opts) = self.base.models
            .get(&options.model)
            .and_then(|c| c.message_options.as_ref())
            && let Some(contents) = body["contents"].as_array_mut()
        {
            for msg in contents {
                merge_options_into(msg, msg_opts);
            }
        }

        if !tools.is_empty() {
            body["tools"] = serde_json::json!([{
                "functionDeclarations": tools.iter().map(|t| serde_json::json!({
                    "name": t.function.name,
                    "description": t.function.description,
                    "parameters": t.function.parameters,
                })).collect::<Vec<_>>()
            }]);
        }

        let mut gen_config = serde_json::Map::new();
        if let Some(max_tok) = options.max_tokens {
            gen_config.insert("maxOutputTokens".into(), serde_json::json!(max_tok));
        }
        if let Some(temp) = options.temperature {
            gen_config.insert("temperature".into(), serde_json::json!(temp));
        }
        if let Some(top_p) = options.top_p {
            gen_config.insert("topP".into(), serde_json::json!(top_p));
        }
        // Reasoning: skip default thinkingConfig if model config provides
        // custom reasoning_options (they'll be merged below).
        let reasoning_opts = self.base.models
            .get(&options.model)
            .and_then(|c| c.reasoning_options.as_ref());
        if options.reasoning_effort.is_some() && reasoning_opts.is_none() {
            gen_config.insert(
                "thinkingConfig".into(),
                serde_json::json!({"includeThoughts": true}),
            );
        }
        if !gen_config.is_empty() {
            body["generationConfig"] = Value::Object(gen_config);
        }

        if let Some(ref tc) = options.tool_choice {
            let mode = match tc.as_str() {
                Some("none") => "NONE",
                Some("required") => "ANY",
                Some("auto") => "AUTO",
                _ => "AUTO",
            };
            body["toolConfig"] = serde_json::json!({
                "functionCallingConfig": {"mode": mode}
            });
        }

        // Deep-merge config options.
        if let Some(ref opts) = self.base.provider_options {
            merge_options_into(&mut body, &normalize_options(opts));
        }
        if let Some(model_cfg) = self.base.models.get(&options.model)
            && let Some(ref opts) = model_cfg.options
        {
            merge_options_into(&mut body, &normalize_options(opts));
        }

        // Apply custom reasoning options (overrides default thinkingConfig).
        if options.reasoning_effort.is_some()
            && let Some(opts) = reasoning_opts
        {
            merge_options_into(&mut body, opts);
        }

        tracing::debug!("POST {} (model={})", url, options.model);

        let mut headers = self.base.headers.clone();
        if let Some(model_cfg) = self.base.models.get(&options.model) {
            for (k, v) in &model_cfg.headers {
                headers.insert(k.clone(), v.clone());
            }
        }

        let response = retry_with_backoff(1, || {
            let client = &self.base.client;
            let url = &url;
            let api_key = &self.base.api_key;
            let body = &body;
            let headers = &headers;
            async move {
                let mut req = client
                    .post(url)
                    .header("x-goog-api-key", api_key)
                    .header("content-type", "application/json");
                for (key, value) in headers {
                    req = req.header(key, value);
                }
                req.json(body).send().await
            }
        })
        .await?;

        let chunk_stream = async_stream::stream! {
            let mut finish_emitted = false;
            let mut tool_idx: usize = 0;

            for await result in sse_data_lines(response) {
                let data = match result {
                    Ok(d) => d,
                    Err(e) => {
                        yield Err(e);
                        break;
                    }
                };

                let parsed: Value = match serde_json::from_str(&data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(candidates) = parsed["candidates"].as_array() {
                    for candidate in candidates {
                        let parts = &candidate["content"]["parts"];
                        if let Some(parts_arr) = parts.as_array() {
                            for part in parts_arr {
                                // Thinking text
                                if part["thought"].as_bool() == Some(true) {
                                    if let Some(text) = part["text"].as_str() {
                                        yield Ok(StreamChunk::ReasoningDelta(text.to_string()));
                                    }
                                    continue;
                                }
                                // Regular text
                                if let Some(text) = part["text"].as_str() {
                                    yield Ok(StreamChunk::TextDelta(text.to_string()));
                                }
                                // Tool call (complete in one chunk)
                                if let Some(fc) = part.get("functionCall") {
                                    let id = format!("tool_{}", tool_idx);
                                    tool_idx += 1;
                                    let name = fc["name"].as_str().unwrap_or("").to_string();
                                    let args = serde_json::to_string(&fc["args"])
                                        .unwrap_or_default();
                                    yield Ok(StreamChunk::ToolCallStart {
                                        id: id.clone(),
                                        name,
                                    });
                                    yield Ok(StreamChunk::ToolCallDelta {
                                        id: id.clone(),
                                        args,
                                    });
                                    yield Ok(StreamChunk::ToolCallEnd { id });
                                }
                            }
                        }

                        // Finish reason
                        if let Some(reason) = candidate["finishReason"].as_str() {
                            let mapped = match reason {
                                "STOP" => None, // handled below — may have tool calls
                                "MAX_TOKENS" => Some("length"),
                                "SAFETY" | "RECITATION" | "BLOCKLIST" |
                                "PROHIBITED_CONTENT" | "SPII" | "IMAGE_SAFETY" => {
                                    Some("content_filter")
                                }
                                "MALFORMED_FUNCTION_CALL" => Some("error"),
                                _ => Some(reason),
                            };
                            if let Some(r) = mapped {
                                let usage = parse_gemini_usage(&parsed);
                                finish_emitted = true;
                                yield Ok(StreamChunk::Finish {
                                    usage,
                                    reason: Some(r.to_string()),
                                });
                            }
                        }
                    }
                }

                // Usage metadata can appear without finishReason (e.g. last chunk).
                if !finish_emitted
                    && parsed.get("usageMetadata").is_some()
                    && parsed.get("candidates").and_then(|c| c.as_array())
                        .is_none_or(|c| c.is_empty())
                {
                    let usage = parse_gemini_usage(&parsed);
                    finish_emitted = true;
                    yield Ok(StreamChunk::Finish {
                        usage,
                        reason: Some("stop".to_string()),
                    });
                }
            }

            if !finish_emitted {
                yield Ok(StreamChunk::Finish { usage: None, reason: None });
            }
        };

        Ok(Box::pin(chunk_stream))
    }

    fn list_models(&self) -> Vec<String> {
        self.base.models.keys().cloned().collect()
    }

    fn supports_images(&self, model: &str) -> bool {
        self.base.models
            .get(model)
            .and_then(|c| c.image_input)
            .unwrap_or(false)
    }

    fn name(&self) -> &str {
        &self.base.name
    }
}

fn parse_gemini_usage(parsed: &Value) -> Option<Usage> {
    let u = parsed["usageMetadata"].as_object()?;
    let prompt = u.get("promptTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
    let candidates = u.get("candidatesTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
    let thoughts = u.get("thoughtsTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
    let cached = u.get("cachedContentTokenCount").and_then(|v| v.as_u64()).unwrap_or(0);
    Some(Usage {
        prompt_tokens: prompt,
        completion_tokens: candidates + thoughts,
        total_tokens: u.get("totalTokenCount").and_then(|v| v.as_u64())
            .unwrap_or(prompt + candidates + thoughts),
        prompt_cache_hit_tokens: cached,
        reasoning_tokens: thoughts,
    })
}

/// Transform OpenAI-format messages into Gemini `contents` format.
/// Tool result messages (role: "tool") become functionResponse parts.
fn lower_messages(messages: &[&Message]) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();
    let mut pending_fn_responses: Vec<Value> = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "tool" => {
                let name = msg.name.clone().unwrap_or_else(|| "tool".to_string());
                let content = msg.content.to_text_lossy();
                pending_fn_responses.push(serde_json::json!({
                    "functionResponse": {
                        "name": name,
                        "response": {"name": name, "content": content},
                    }
                }));
            }
            "user" => {
                if !pending_fn_responses.is_empty() {
                    result.push(serde_json::json!({
                        "role": "user",
                        "parts": pending_fn_responses,
                    }));
                    pending_fn_responses = Vec::new();
                }
                result.push(serde_json::json!({
                    "role": "user",
                    "parts": [{"text": msg.content.to_text_lossy()}],
                }));
            }
            "assistant" => {
                if !pending_fn_responses.is_empty() {
                    result.push(serde_json::json!({
                        "role": "user",
                        "parts": pending_fn_responses,
                    }));
                    pending_fn_responses = Vec::new();
                }
                let mut parts: Vec<Value> = Vec::new();
                let text = msg.content.to_text_lossy();
                if !text.is_empty() {
                    parts.push(serde_json::json!({"text": text}));
                }
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        let args: Value = serde_json::from_str(&tc.function.arguments)
                            .unwrap_or(serde_json::json!({}));
                        parts.push(serde_json::json!({
                            "functionCall": {"name": tc.function.name, "args": args}
                        }));
                    }
                }
                if !parts.is_empty() {
                    result.push(serde_json::json!({"role": "model", "parts": parts}));
                }
            }
            _ => {}
        }
    }

    if !pending_fn_responses.is_empty() {
        result.push(serde_json::json!({
            "role": "user",
            "parts": pending_fn_responses,
        }));
    }

    result
}

/// Create a Gemini provider from config.
pub fn create(cfg: &ResolvedProvider) -> Option<GeminiProvider> {
    let api_key = cfg.api_key.clone()?;
    let base_url = cfg
        .base_url
        .clone()
        .unwrap_or_else(|| {
            "https://generativelanguage.googleapis.com/v1beta".to_string()
        });

    Some(GeminiProvider::new(
        cfg.name.clone(),
        api_key,
        base_url,
        cfg.models.clone(),
        cfg.options.clone(),
        cfg.headers.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::provider::{MessageContent, ToolCall, ToolCallFunction};

    fn make_message(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: MessageContent::text(content),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn lower_messages_user_text() {
        let msg = make_message("user", "hello");
        let result = lower_messages(&[&msg]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[0]["parts"][0]["text"], "hello");
    }

    #[test]
    fn lower_messages_assistant_becomes_model() {
        let msg = make_message("assistant", "hi there");
        let result = lower_messages(&[&msg]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "model");
        assert_eq!(result[0]["parts"][0]["text"], "hi there");
    }

    #[test]
    fn lower_messages_tool_result_as_function_response() {
        let tool_msg = Message {
            role: "tool".to_string(),
            content: MessageContent::text("result"),
            name: Some("read".to_string()),
            tool_call_id: None,
            tool_calls: None,
        };
        let user_msg = make_message("user", "next");
        let result = lower_messages(&[&tool_msg, &user_msg]);
        assert_eq!(result.len(), 2);
        // First message should be the flushed function responses as user
        assert_eq!(result[0]["role"], "user");
        assert_eq!(result[0]["parts"][0]["functionResponse"]["name"], "read");
        // Second is the actual user message
        assert_eq!(result[1]["role"], "user");
    }

    #[test]
    fn lower_messages_assistant_with_function_call() {
        let msg = Message {
            role: "assistant".to_string(),
            content: MessageContent::text(""),
            name: None,
            tool_call_id: None,
            tool_calls: Some(vec![ToolCall {
                id: "call-1".to_string(),
                kind: "function".to_string(),
                function: ToolCallFunction {
                    name: "read".to_string(),
                    arguments: r#"{"path":"test.rs"}"#.to_string(),
                },
            }]),
        };
        let result = lower_messages(&[&msg]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["role"], "model");
        assert_eq!(result[0]["parts"][0]["functionCall"]["name"], "read");
    }

    #[test]
    fn lower_messages_empty_assistant_not_emitted() {
        let msg = Message {
            role: "assistant".to_string(),
            content: MessageContent::text(""),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        };
        let result = lower_messages(&[&msg]);
        assert!(result.is_empty());
    }
}
