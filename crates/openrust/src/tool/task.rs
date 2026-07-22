//! Task tool — delegate a prompt to a sub-agent that runs its own tool loop.

use futures::StreamExt;

use crate::core::agent;
use crate::core::provider::{
    self, LlmProvider, Message, MessageContent, RequestOptions, StreamChunk, ToolDef, ToolFunction,
};
use crate::core::session::{AgentKind, SessionStore};
use crate::require_str;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, catalog, run_tool};
use serde_json::Value;

/// Generate a session id for an ephemeral agent session (`sub-*` / `bg-*`).
pub fn agent_session_id(kind: AgentKind) -> String {
    let prefix = match kind {
        AgentKind::SubAgent => "sub",
        AgentKind::Background => "bg",
        AgentKind::Main => "session",
    };
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros();
    format!("{prefix}-{micros}")
}

/// Create and register an ephemeral agent session for history persistence.
/// Returns `None` when persistence is disabled or no store is available.
pub fn create_agent_session(
    store: Option<&SessionStore>,
    persist_enabled: bool,
    kind: AgentKind,
    parent: Option<&str>,
) -> Option<String> {
    let store = store.filter(|_| persist_enabled && kind != AgentKind::Main)?;
    let id = agent_session_id(kind);
    store
        .ensure_session_kind(&id, kind, parent.map(str::to_string))
        .ok()?;
    Some(id)
}

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
        let Some(found) = agent::agent_by_id(&agents, subagent_type) else {
            let available = agents
                .iter()
                .map(|a| a.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return ToolResult::error(format!(
                "task: unknown subagent_type '{}'. Available: {}",
                subagent_type, available
            ));
        };

        // Sub-agent gets its own ephemeral session anchoring its context and
        // history (persisted for debugging when enabled). No LLM (prevents
        // recursive task), no store in ctx (keeps the parent's TODO list
        // isolated). Interactivity is inherited: the sub-agent's permission /
        // question requests ride the bus with its SubAgent tag and are shown
        // on the main UI; abort propagates through the shared flag.
        let sub_id = agent_session_id(AgentKind::SubAgent);
        let persist = match (&ctx.store, ctx.persist_agent_sessions) {
            (Some(store), true) => {
                let _ = store.ensure_session_kind(
                    &sub_id,
                    AgentKind::SubAgent,
                    ctx.session_id.clone(),
                );
                Some((store.clone(), sub_id.clone()))
            }
            _ => None,
        };
        let sub_ctx = ToolContext {
            llm: None,
            interactive: ctx.interactive,
            store: None,
            session_id: None,
            abort: ctx.abort.clone(),
            events: ctx
                .events
                .as_ref()
                .map(|e| e.child(sub_id.clone(), AgentKind::SubAgent)),
            ..ctx.clone()
        };

        let initial = vec![Message::user(prompt)];

        match run_agent(
            llm.as_ref(),
            model,
            &found.system,
            &found.tools,
            found.max_steps,
            ctx.reasoning_effort.as_deref(),
            initial,
            &sub_ctx,
            persist,
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
///
/// `persist` optionally anchors the run to an ephemeral session: committed
/// messages are appended to `(store, session_id)` so sub-agent / background
/// histories can be inspected later.
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
    persist: Option<(SessionStore, String)>,
) -> anyhow::Result<String> {
    let allowed = catalog::resolve_tool_names(tool_spec, &tool_ctx.presets, true);
    let tool_defs: Vec<ToolDef> = allowed
        .iter()
        // Background agents never wait on the user: the question tool is
        // removed entirely so the model cannot even call it.
        .filter(|meta| tool_ctx.interactive || meta.name != "question")
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
        if let Some(events) = &tool_ctx.events {
            events.progress(msg);
        }
    };
    // Persistence is batched: messages accumulate in `pending_persist` and
    // are flushed at step boundaries (between LLM calls), keeping sync sled
    // writes out of the latency-sensitive streaming path and cutting write
    // amplification from O(messages) to O(steps).
    let mut pending_persist: Vec<PersistedMsg> = Vec::new();
    for message in &history {
        buffer_persisted(
            &mut pending_persist,
            &message.role,
            &message.content.as_text(),
            None,
            None,
            None,
        );
    }
    flush_persisted(&persist, &mut pending_persist);
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

        // Retryable + abortable LLM call with countdown via progress channel.
        const MAX_RETRIES: u32 = 3;
        let mut stream: Option<provider::ChunkStream> = None;
        'retry: for attempt in 0..=MAX_RETRIES {
            if is_cancelled() { return Ok(last_assistant); }

            let chat = llm.chat(
                &history,
                &tool_defs,
                RequestOptions {
                    model: model.to_string(),
                    temperature: None,
                    max_tokens: None,
                    top_p: None,
                    system: Some(system.to_string()),
                    reasoning_effort: reasoning_effort.map(str::to_string),
                    tool_choice: None,
                    cache_key: tool_ctx
                        .session_id
                        .as_deref()
                        .map(str::to_string),
                },
            );
            tokio::pin!(chat);

            loop {
                tokio::select! {
                    result = &mut chat => {
                        match result {
                            Ok(s) => {
                                stream = Some(s);
                                break 'retry;
                            }
                            Err(e) => {
                                if attempt < MAX_RETRIES && is_retriable_error(&e) {
                                    let delay_secs = 2u64.pow(attempt + 1).min(8);
                                    let detail = short_error(&e);
                                    let mut remaining_ms = delay_secs * 1000;
                                    while remaining_ms > 0 {
                                        let secs = remaining_ms.div_ceil(1000);
                                        send_progress(format!(
                                            "sub-agent retry {}/{} in {}s — {}",
                                            attempt + 1, MAX_RETRIES, secs, detail,
                                        ));
                                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                        if is_cancelled() { return Ok(last_assistant); }
                                        remaining_ms = remaining_ms.saturating_sub(500);
                                    }
                                    break; // inner → retry outer
                                }
                                return Err(e);
                            }
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                        if is_cancelled() { return Ok(last_assistant); }
                    }
                }
            }
        }
        let Some(mut stream) = stream else {
            return Err(anyhow::anyhow!("retry loop exhausted without setting stream"));
        };

        let mut assistant_text = String::new();
        let mut pending: Vec<(String, String, String)> = Vec::new();
        let mut executed_calls: Vec<provider::ToolCall> = Vec::new();
        let mut tool_outputs: Vec<(String, String)> = Vec::new();
        let mut finish_seen = false;

        loop {
            tokio::select! {
                chunk = stream.next() => {
                    let Some(chunk) = chunk else { break; };
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
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                    if is_cancelled() { return Ok(last_assistant); }
                }
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
            buffer_persisted(
                &mut pending_persist,
                "assistant",
                &assistant_text,
                None,
                None,
                serde_json::to_value(&executed_calls).ok(),
            );
            history.push(Message {
                role: "assistant".to_string(),
                content: MessageContent::text(assistant_text.clone()),
                name: None,
                tool_call_id: None,
                tool_calls: Some(executed_calls),
            });
            for (call_id, output) in &tool_outputs {
                buffer_persisted(
                    &mut pending_persist,
                    "tool",
                    output,
                    None,
                    Some(call_id.clone()),
                    None,
                );
                history.push(Message::tool(output.clone(), call_id.clone()));
            }
            flush_persisted(&persist, &mut pending_persist);
            continue;
        }

        if !assistant_text.trim().is_empty() {
            last_assistant = assistant_text.clone();
            buffer_persisted(
                &mut pending_persist,
                "assistant",
                &assistant_text,
                None,
                None,
                None,
            );
            history.push(Message::assistant(assistant_text));
        }
        flush_persisted(&persist, &mut pending_persist);

        if pending.is_empty() && finish_seen {
            return Ok(last_assistant);
        }
    }
}

/// Buffered message awaiting batch persistence
/// (role, content, name, tool_call_id, tool_calls).
type PersistedMsg = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<serde_json::Value>,
);

/// Buffer a message for batch persistence at the next step boundary.
fn buffer_persisted(
    pending: &mut Vec<PersistedMsg>,
    role: &str,
    content: &str,
    name: Option<String>,
    tool_call_id: Option<String>,
    tool_calls: Option<serde_json::Value>,
) {
    pending.push((
        role.to_string(),
        content.to_string(),
        name,
        tool_call_id,
        tool_calls,
    ));
}

/// Write buffered messages to the session store in one batch. Call order is
/// preserved, so message seq numbers stay monotonic.
fn flush_persisted(persist: &Option<(SessionStore, String)>, pending: &mut Vec<PersistedMsg>) {
    let Some((store, session_id)) = persist else {
        pending.clear();
        return;
    };
    for (role, content, name, tool_call_id, tool_calls) in pending.drain(..) {
        let _ = store.append_message_detail(
            session_id,
            &role,
            &content,
            name,
            tool_call_id,
            tool_calls,
        );
    }
}

/// Whether an error from llm.chat() is worth retrying.
fn is_retriable_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    if msg.starts_with("HTTP 4") && !msg.starts_with("HTTP 429") && !msg.starts_with("HTTP 408") {
        return false;
    }
    true
}

/// Short summary of an error for the progress channel.
fn short_error(err: &anyhow::Error) -> String {
    let msg = err.to_string();
    let first_line = msg.lines().next().unwrap_or(&msg);
    if first_line.chars().count() > 50 {
        let truncated: String = first_line.chars().take(47).collect();
        format!("{truncated}…")
    } else {
        first_line.to_string()
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
            messages: &[Message],
            _tools: &[ToolDef],
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
    async fn run_agent_persists_ephemeral_session_history() {
        struct TextProvider;

        #[async_trait::async_trait]
        impl LlmProvider for TextProvider {
            async fn chat(
                &self,
                _messages: &[Message],
                _tools: &[ToolDef],
                _options: RequestOptions,
            ) -> anyhow::Result<provider::ChunkStream> {
                Ok(Box::pin(futures::stream::iter(vec![
                    Ok(StreamChunk::TextDelta("final answer".to_string())),
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

        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::open_at(dir.path()).unwrap();
        store
            .ensure_session_kind("sub-test", AgentKind::SubAgent, Some("session-main".to_string()))
            .unwrap();
        let ctx = ToolContext::new(dir.path().to_path_buf());

        let output = run_agent(
            &TextProvider,
            "fake-model",
            "system",
            "none",
            5,
            None,
            vec![Message::user("hello")],
            &ctx,
            Some((store.clone(), "sub-test".to_string())),
        )
        .await
        .unwrap();

        assert_eq!(output, "final answer");
        let messages = store.get_messages("sub-test").unwrap();
        assert!(
            messages
                .iter()
                .any(|m| m.role == "user" && m.content == "hello")
        );
        assert!(
            messages
                .iter()
                .any(|m| m.role == "assistant" && m.content == "final answer")
        );
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
