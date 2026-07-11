//! Task tool — delegate a prompt to a sub-agent that runs its own tool loop.

use futures::StreamExt;

use crate::core::agent;
use crate::core::provider::{
    self, LlmProvider, Message, MessageContent, RequestOptions, StreamChunk, ToolDef, ToolFunction,
};
use crate::require_str;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, catalog, run_tool};
use serde_json::Value;

pub struct TaskTool;

#[async_trait::async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &'static str {
        "task"
    }
    fn description(&self) -> &'static str {
        "Delegate a self-contained task to a sub-agent. The sub-agent runs its own tool loop with the selected agent's instructions and returns a final result. Use for multi-step research or work that can run autonomously."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "A short (3-5 word) description of the task" },
                "prompt": { "type": "string", "description": "The task for the sub-agent to perform" },
                "subagent_type": { "type": "string", "description": "The agent id to run (e.g. general, explore, plan)" }
            },
            "required": ["prompt", "subagent_type"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let prompt = require_str!(p, "prompt");
        let subagent_type = require_str!(p, "subagent_type");

        let (Some(llm), Some(model)) = (ctx.llm.as_ref(), ctx.model.as_ref()) else {
            return ToolResult::error("task: no LLM provider is available in this context");
        };

        let agents = agent::load_agents(&ctx.cwd).unwrap_or_default();
        let (system, tool_spec, max_steps) = match agent::agent_by_id(&agents, subagent_type) {
            Some(found) => (found.system.clone(), found.tools.clone(), found.max_steps),
            None => match agent::builtin_agent_system(subagent_type) {
                Some(system) => (system.to_string(), "none".to_string(), 25u32),
                None => {
                    let available = agents
                        .iter()
                        .map(|a| a.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return ToolResult::error(format!(
                        "task: unknown subagent_type '{}'. Available: {}",
                        subagent_type, available
                    ));
                }
            },
        };

        // Sub-agent context: no LLM (prevents recursive task), no interactive
        // channels, no session store (prevents overwriting parent's TODO list).
        let sub_ctx = ToolContext {
            llm: None,
            ask_tx: None,
            permission_tx: None,
            interactive: false,
            store: None,
            session_id: None,
            abort: ctx.abort.clone(),
            progress_tx: ctx.progress_tx.clone(),
            ..ctx.clone()
        };

        let initial = vec![Message::user(prompt)];

        match run_agent(
            llm.as_ref(),
            model,
            &system,
            &tool_spec,
            max_steps,
            ctx.reasoning_effort.as_deref(),
            initial,
            &sub_ctx,
        )
        .await
        {
            Ok(output) if output.trim().is_empty() => ToolResult::text(format!(
                "[{} sub-agent finished with no textual output]",
                subagent_type
            )),
            Ok(output) => ToolResult::text(output),
            Err(err) => ToolResult::error(format!("task: sub-agent failed: {}", err)),
        }
    }
}

/// Run a headless agent loop (no UI events): stream, execute tools locally, feed
/// results back, and repeat until the model finishes. Returns the final text.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent(
    llm: &dyn LlmProvider,
    model: &str,
    system: &str,
    tool_spec: &str,
    max_steps: u32,
    reasoning_effort: Option<&str>,
    initial: Vec<Message>,
    tool_ctx: &ToolContext,
) -> anyhow::Result<String> {
    let allowed = catalog::resolve_tool_names(tool_spec, &tool_ctx.presets, true);
    let tool_defs: Vec<ToolDef> = allowed
        .iter()
        .filter_map(|meta| catalog::create_tool(meta.name, None))
        .map(|tool| ToolDef {
            r#type: "function".to_string(),
            function: ToolFunction {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters(),
            },
        })
        .collect();

    let mut history = initial;
    let mut last_assistant = String::new();
    let mut step_count: u32 = 0;

    let send_progress = |msg: String| {
        if let Some(tx) = &tool_ctx.progress_tx {
            let _ = tx.send(msg);
        }
    };
    let is_cancelled = || {
        if let Some(flag) = &tool_ctx.shutdown
            && flag.load(std::sync::atomic::Ordering::SeqCst)
        {
            return true;
        }
        if let Some(flag) = &tool_ctx.abort
            && flag.load(std::sync::atomic::Ordering::SeqCst)
        {
            return true;
        }
        false
    };

    loop {
        if is_cancelled() {
            return Ok(last_assistant);
        }
        step_count += 1;
        if step_count > max_steps {
            return Ok(last_assistant);
        }
        send_progress(format!("sub-agent step {step_count}/{max_steps}"));

        let is_last_step = step_count >= max_steps;
        if is_last_step {
            history.push(Message::assistant(
                agent::MAX_STEPS_PROMPT.to_string(),
            ));
        }

        let mut stream = llm
            .chat(
                history.clone(),
                tool_defs.clone(),
                RequestOptions {
                    model: model.to_string(),
                    temperature: None,
                    max_tokens: None,
                    top_p: None,
                    system: Some(system.to_string()),
                    reasoning_effort: reasoning_effort.map(str::to_string),
                    tool_choice: None,
                },
            )
            .await?;

        let mut assistant_text = String::new();
        let mut pending: Vec<(String, String, String)> = Vec::new();
        let mut executed_calls: Vec<provider::ToolCall> = Vec::new();
        let mut tool_outputs: Vec<(String, String)> = Vec::new();
        let mut finish_seen = false;

        while let Some(chunk) = stream.next().await {
            if is_cancelled() {
                return Ok(last_assistant);
            }
            match chunk? {
                StreamChunk::TextDelta(text) => assistant_text.push_str(&text),
                StreamChunk::ReasoningDelta(_) => {}
                StreamChunk::ToolCallStart { id, name } => {
                    pending.push((id, name, String::new()));
                }
                StreamChunk::ToolCallDelta { id, args } => {
                    if let Some((_, _, buffer)) = pending
                        .iter_mut()
                        .rev()
                        .find(|(call_id, _, _)| call_id == &id)
                    {
                        buffer.push_str(&args);
                    }
                }
                StreamChunk::ToolCallEnd { id: _ } => {}
                StreamChunk::Finish { .. } => finish_seen = true,
            }
        }

        for (id, name, args) in &pending {
            if is_cancelled() {
                return Ok(last_assistant);
            }
            send_progress(format!("sub-agent: {name}"));
            let output = run_tool(name, args, tool_ctx).await.into_text();
            executed_calls.push(provider::ToolCall {
                id: id.clone(),
                kind: "function".to_string(),
                function: provider::ToolCallFunction {
                    name: name.clone(),
                    arguments: args.clone(),
                },
            });
            tool_outputs.push((id.clone(), output));
        }

        if !executed_calls.is_empty() {
            last_assistant = assistant_text.clone();
            history.push(Message {
                role: "assistant".to_string(),
                content: MessageContent::text(assistant_text.clone()),
                name: None,
                tool_call_id: None,
                tool_calls: Some(executed_calls),
            });
            for (call_id, output) in &tool_outputs {
                history.push(Message::tool(output.clone(), call_id.clone()));
            }
            continue;
        }

        if !assistant_text.trim().is_empty() {
            last_assistant = assistant_text.clone();
            history.push(Message::assistant(assistant_text));
        }

        if pending.is_empty() && finish_seen {
            return Ok(last_assistant);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct FakeProvider {
        calls: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        async fn chat(
            &self,
            messages: Vec<Message>,
            _tools: Vec<ToolDef>,
            _options: RequestOptions,
        ) -> anyhow::Result<provider::ChunkStream> {
            let mut calls = self.calls.lock().unwrap();
            *calls += 1;

            if *calls == 1 {
                return Ok(Box::pin(futures::stream::iter(vec![
                    Ok(StreamChunk::ToolCallStart {
                        id: "call-1".to_string(),
                        name: "write".to_string(),
                    }),
                    Ok(StreamChunk::ToolCallDelta {
                        id: "call-1".to_string(),
                        args: r#"{"path":"/tmp/openrust-denied/out.txt","content":"blocked"}"#
                            .to_string(),
                    }),
                    Ok(StreamChunk::ToolCallEnd {
                        id: "call-1".to_string(),
                    }),
                ])));
            }

            let denied = messages.iter().any(|message| {
                message.role == "tool" && message.content.as_text().contains("outside the project scope")
            });
            Ok(Box::pin(futures::stream::iter(vec![
                Ok(StreamChunk::TextDelta(if denied {
                    "permission denied observed".to_string()
                } else {
                    "permission result missing".to_string()
                })),
                Ok(StreamChunk::Finish { usage: None, reason: None }),
            ])))
        }

        fn list_models(&self) -> Vec<String> {
            vec!["fake-model".to_string()]
        }

        fn name(&self) -> &str {
            "fake"
        }
    }

    #[tokio::test]
    async fn errors_without_llm() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let result = TaskTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "description": "demo",
                    "prompt": "do something",
                    "subagent_type": "general"
                })),
                &ctx,
            )
            .await;
        assert!(matches!(result, ToolResult::Error(_)));
    }

    #[tokio::test]
    async fn subagent_tool_calls_use_permission_gate() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext {
            llm: Some(Arc::new(FakeProvider {
                calls: Mutex::new(0),
            })),
            model: Some("fake-model".to_string()),
            interactive: false,
            ..ToolContext::new(dir.path().to_path_buf())
        };

        let result = TaskTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "description": "permission check",
                    "prompt": "try a write",
                    "subagent_type": "general"
                })),
                &ctx,
            )
            .await;

        assert_eq!(result.into_text(), "permission denied observed");
        assert!(!std::path::Path::new("/tmp/openrust-denied/out.txt").exists());
    }
}
