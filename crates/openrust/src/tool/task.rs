//! Task tool — delegate a prompt to a sub-agent that runs its own tool loop.

use futures::StreamExt;

use crate::core::agent;
use crate::core::provider::{self, LlmProvider, Message, RequestOptions, StreamChunk, ToolDef, ToolFunction};
use crate::require_str;
use crate::tool::{catalog, run_tool, Tool, ToolContext, ToolParams, ToolResult};
use serde_json::Value;

/// Maximum provider turns for a single sub-agent run (guards against loops).
const MAX_TURNS: usize = 24;

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
            "required": ["description", "prompt", "subagent_type"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let prompt = require_str!(p, "prompt");
        let subagent_type = require_str!(p, "subagent_type");

        let (Some(llm), Some(model)) = (ctx.llm.as_ref(), ctx.model.as_ref()) else {
            return ToolResult::error("task: no LLM provider is available in this context");
        };

        let agents = agent::load_agents(&ctx.cwd).unwrap_or_default();
        let system = match agent::agent_by_id(&agents, subagent_type) {
            Some(found) => found.system.clone(),
            None => match agent::builtin_agent_system(subagent_type) {
                Some(system) => system.to_string(),
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

        // Sub-agent context: no LLM (prevents recursive task), no interactive channels.
        let sub_ctx = ToolContext {
            llm: None,
            ask_tx: None,
            permission_tx: None,
            interactive: false,
            ..ctx.clone()
        };

        let initial = vec![Message {
            role: "user".to_string(),
            content: prompt.to_string(),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];

        match run_agent(llm.as_ref(), model, &system, ctx.reasoning_effort.as_deref(), initial, &sub_ctx).await {
            Ok(output) if output.trim().is_empty() => {
                ToolResult::text(format!("[{} sub-agent finished with no textual output]", subagent_type))
            }
            Ok(output) => ToolResult::text(output),
            Err(err) => ToolResult::error(format!("task: sub-agent failed: {}", err)),
        }
    }
}

/// Run a headless agent loop (no UI events): stream, execute tools locally, feed
/// results back, and repeat until the model finishes. Returns the final text.
pub async fn run_agent(
    llm: &dyn LlmProvider,
    model: &str,
    system: &str,
    reasoning_effort: Option<&str>,
    initial: Vec<Message>,
    tool_ctx: &ToolContext,
) -> anyhow::Result<String> {
    let tool_defs: Vec<ToolDef> = catalog::TOOL_CATALOG
        .iter()
        .filter(|meta| meta.name != "task" && meta.name != "question")
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

    for _ in 0..MAX_TURNS {
        let mut stream = llm
            .chat(
                history.clone(),
                tool_defs.clone(),
                RequestOptions {
                    model: model.to_string(),
                    temperature: None,
                    max_tokens: None,
                    system: Some(system.to_string()),
                    reasoning_effort: reasoning_effort.map(str::to_string),
                },
            )
            .await?;

        let mut assistant_text = String::new();
        let mut pending: Vec<(String, String, String)> = Vec::new();
        let mut finish_seen = false;

        while let Some(chunk) = stream.next().await {
            match chunk? {
                StreamChunk::TextDelta(text) => assistant_text.push_str(&text),
                StreamChunk::ReasoningDelta(_) => {}
                StreamChunk::ToolCallStart { id, name } => {
                    pending.push((id, name, String::new()));
                }
                StreamChunk::ToolCallDelta { id, args } => {
                    if let Some((_, _, buffer)) = pending.iter_mut().rev().find(|(call_id, _, _)| call_id == &id) {
                        buffer.push_str(&args);
                    }
                }
                StreamChunk::ToolCallEnd { id } => {
                    let Some((_, name, args)) = pending.iter().find(|(call_id, _, _)| call_id == &id).cloned() else {
                        continue;
                    };
                    let output = run_tool(&name, &args, tool_ctx).await;
                    history.push(Message {
                        role: "assistant".to_string(),
                        content: assistant_text.clone(),
                        name: None,
                        tool_call_id: None,
                        tool_calls: Some(vec![provider::ToolCall {
                            id: id.clone(),
                            kind: "function".to_string(),
                            function: provider::ToolCallFunction {
                                name: name.clone(),
                                arguments: args.clone(),
                            },
                        }]),
                    });
                    history.push(Message {
                        role: "tool".to_string(),
                        content: output,
                        name: Some(name),
                        tool_call_id: Some(id),
                        tool_calls: None,
                    });
                    assistant_text.clear();
                    pending.clear();
                    break;
                }
                StreamChunk::Finish { .. } => finish_seen = true,
            }
        }

        if !assistant_text.trim().is_empty() {
            last_assistant = assistant_text.clone();
            history.push(Message {
                role: "assistant".to_string(),
                content: assistant_text,
                name: None,
                tool_call_id: None,
                tool_calls: None,
            });
        }

        if pending.is_empty() {
            if finish_seen || !last_assistant.is_empty() {
                return Ok(last_assistant);
            }
            return Ok(last_assistant);
        }
    }

    Ok(if last_assistant.is_empty() {
        "[task reached the maximum number of turns]".to_string()
    } else {
        last_assistant
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
