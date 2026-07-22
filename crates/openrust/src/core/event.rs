//! Unified session event bus.
//!
//! Every agent instance (main turn, sub-agent, background task) anchors its
//! context to a session and emits events tagged with that session id. The
//! frontend (TUI or headless) consumes a single `mpsc::Receiver<SessionEvent>`
//! and routes by `AgentKind` instead of polling half a dozen channels.

use std::sync::mpsc;

use crate::core::session::AgentKind;

/// Events emitted by an agent loop while streaming a turn.
#[derive(Debug)]
pub enum PromptEvent {
    AssistantDelta(String),
    ThinkingDelta(String),
    ToolCallStart { id: String, name: String },
    ToolRunning { id: String, args: String },
    ToolBatch {
        assistant: String,
        tool_calls: Vec<serde_json::Value>,
        results: Vec<ToolBatchItem>,
    },
    Finish { prompt_tokens: u64, cache_hit_tokens: u64 },
    /// Auto-compaction checkpoint was written to the session store;
    /// `drained` = number of old messages folded into the summary. The UI
    /// realigns its history from the store (see `compaction_window`).
    Compacted { drained: usize },
    Error(String),
    /// User pressed ESC to abort the current turn.
    Aborted,
    /// Retry countdown: "retry 1/3 in 4s".
    RetryStatus(String),
}

#[derive(Debug, Clone)]
pub struct ToolBatchItem {
    pub id: String,
    pub name: String,
    pub args: String,
    pub result: String,
}

/// A request from a tool (e.g. `question`) for interactive user input.
/// The tool emits this on the session bus, then blocks on `responder`
/// until the UI collects answers (one string per sub-question).
#[derive(Debug)]
pub struct AskRequest {
    pub questions: serde_json::Value,
    pub responder: mpsc::Sender<Vec<String>>,
}

/// A request to confirm a permission-gated tool invocation. The tool emits
/// this on the session bus and blocks on `responder` (true = allow).
#[derive(Debug)]
pub struct PermissionRequest {
    pub tool: String,
    pub detail: String,
    pub responder: mpsc::Sender<bool>,
}

/// Payload of a session bus event.
///
/// `Ask`/`Permission` are request/response interactions carried on the bus
/// for transport convenience — the reply travels back over the oneshot-style
/// `responder` inside the request, not over the bus.
#[derive(Debug)]
pub enum EventPayload {
    /// Streaming lifecycle of an agent turn.
    Prompt(PromptEvent),
    Ask(AskRequest),
    Permission(PermissionRequest),
    /// Short human-readable progress string (sub-agent steps, retries).
    Progress(String),
    /// Background task completion: Ok(result) or Err(message).
    Done { result: Result<String, String> },
}

/// A single event on the session bus.
#[derive(Debug)]
pub struct SessionEvent {
    pub session_id: String,
    pub kind: AgentKind,
    pub payload: EventPayload,
}

/// Sending end of the session bus, pre-tagged with a session identity.
/// Cheap to clone; clones share the underlying channel.
#[derive(Clone)]
pub struct SessionEventSender {
    session_id: String,
    kind: AgentKind,
    tx: mpsc::Sender<SessionEvent>,
}

impl SessionEventSender {
    pub fn new(
        session_id: impl Into<String>,
        kind: AgentKind,
        tx: mpsc::Sender<SessionEvent>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            kind,
            tx,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn kind(&self) -> AgentKind {
        self.kind
    }

    pub fn send(&self, payload: EventPayload) {
        let _ = self.tx.send(SessionEvent {
            session_id: self.session_id.clone(),
            kind: self.kind,
            payload,
        });
    }

    pub fn send_prompt(&self, event: PromptEvent) {
        self.send(EventPayload::Prompt(event));
    }

    pub fn progress(&self, msg: impl Into<String>) {
        self.send(EventPayload::Progress(msg.into()));
    }

    pub fn done(&self, result: Result<String, String>) {
        self.send(EventPayload::Done { result });
    }

    /// Sender for a child session (sub-agent / background task) sharing the
    /// same bus.
    pub fn child(&self, session_id: impl Into<String>, kind: AgentKind) -> Self {
        Self::new(session_id, kind, self.tx.clone())
    }
}

/// Create a bus channel pair.
pub fn session_bus() -> (mpsc::Sender<SessionEvent>, mpsc::Receiver<SessionEvent>) {
    mpsc::channel()
}
