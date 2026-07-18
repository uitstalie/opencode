//! Session compaction — auto-triggered context compression.
//!
//! When conversation tokens exceed a threshold (context window minus buffer),
//! automatically compacts older messages into a structured summary using the
//! compaction agent, keeping only recent turns verbatim.

use crate::core::token;

/// Reserved tokens for the model's output, preventing context overflow.
pub const DEFAULT_COMPACTION_BUFFER: usize = 20_000;

/// How many tokens to preserve as recent verbatim context.
pub const DEFAULT_KEEP_TOKENS: usize = 8_000;

/// Structured summary template — mirrors the TypeScript `SUMMARY_TEMPLATE`.
pub const SUMMARY_TEMPLATE: &str = r#"Output exactly the Markdown structure shown inside <template> and keep the section order unchanged. Do not include the <template> tags in your response.
<template>
## Goal
- [single-sentence task summary]

## Constraints & Preferences
- [user constraints, preferences, specs, or "(none)"]

## Progress
### Done
- [completed work or "(none)"]

### In Progress
- [current work or "(none)"]

### Blocked
- [blockers or "(none)"]

## Key Decisions
- [decision and why, or "(none)"]

## Next Steps
- [ordered next actions or "(none)"]

## Critical Context
- [important technical facts, errors, open questions, or "(none)"]

## Relevant Files
- [file or directory path: why it matters, or "(none)"]
</template>

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact file paths, commands, error strings, and identifiers when known.
- Do not mention the summary process or that context was compacted."#;

/// Settings that control compaction behaviour.
#[derive(Debug, Clone)]
pub struct CompactionSettings {
    pub auto: bool,
    pub buffer: usize,
    pub keep_tokens: usize,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            auto: true,
            buffer: DEFAULT_COMPACTION_BUFFER,
            keep_tokens: DEFAULT_KEEP_TOKENS,
        }
    }
}

/// Estimate total tokens for a request (system + messages + tools JSON).
pub fn estimate_request(
    system: &str,
    messages: &[crate::core::provider::Message],
    tools: &[crate::core::provider::ToolDef],
) -> usize {
    let combined = format!(
        "{{\"system\":{},\"messages\":{},\"tools\":{}}}",
        serde_json::to_string(system).unwrap_or_default(),
        serde_json::to_string(messages).unwrap_or_default(),
        serde_json::to_string(tools).unwrap_or_default(),
    );
    token::estimate(&combined)
}

/// Check whether the request would overflow the model's context window.
/// Returns `true` when compaction is needed.
pub fn is_overflow(
    tokens: usize,
    context_window: usize,
    max_output: usize,
    settings: &CompactionSettings,
) -> bool {
    if !settings.auto || context_window == 0 {
        return false;
    }
    tokens >= context_window.saturating_sub(max_output.max(settings.buffer))
}

/// Serialize flattened messages into a compact text representation for the
/// compaction LLM prompt. Keeps the total under `limit` tokens, splitting
/// at message boundaries from the end.
pub fn select_recent(
    messages: &[crate::core::provider::Message],
    limit_tokens: usize,
) -> (String, String) {
    let serialized: Vec<String> = messages
        .iter()
        .map(|m| {
            let role = &m.role;
            let content = m.content.as_text();
            format!("[{role}]: {content}")
        })
        .collect();

    let mut total = 0usize;
    let mut split = serialized.len();

    for (i, msg) in serialized.iter().enumerate().rev() {
        let next = total + token::estimate(msg);
        if next > limit_tokens {
            split = i + 1;
            break;
        }
        total = next;
        split = i;
    }

    let head = serialized[..split].join("\n\n");
    let recent = serialized[split..].join("\n\n");
    (head, recent)
}

/// Build the full prompt for the compaction LLM call.
pub fn build_compaction_prompt(previous_summary: Option<&str>, context: &str) -> String {
    let instruction = if let Some(prev) = previous_summary {
        format!(
            "Update the anchored summary below using the conversation history above.\n\
             Preserve still-true details, remove stale details, and merge in the new facts.\n\
             <previous-summary>\n{prev}\n</previous-summary>"
        )
    } else {
        "Create a new anchored summary from the conversation history.".to_string()
    };

    format!("{instruction}\n\n{template}\n\n{context}", template = SUMMARY_TEMPLATE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::provider::{Message, MessageContent};

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: MessageContent::text(content),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn empty_overflows_nothing() {
        let s = CompactionSettings::default();
        assert!(!is_overflow(0, 128_000, 16_000, &s));
    }

    #[test]
    fn overflows_when_near_limit() {
        let s = CompactionSettings::default();
        assert!(is_overflow(110_000, 128_000, 16_000, &s));
    }

    #[test]
    fn no_overflow_when_auto_disabled() {
        let s = CompactionSettings { auto: false, ..Default::default() };
        assert!(!is_overflow(200_000, 128_000, 16_000, &s));
    }

    #[test]
    fn select_recent_splits_correctly() {
        let msgs: Vec<Message> = (0..40)
            .map(|i| msg("user", &format!("message {i} with extra padding to ensure it consumes tokens properly")))
            .collect();
        let (head, recent) = select_recent(&msgs, 300);
        assert!(!head.is_empty(), "head should not be empty with many messages and small token limit");
        assert!(!recent.is_empty(), "recent should not be empty");
        let head_lines: Vec<&str> = head.lines().collect();
        let recent_lines: Vec<&str> = recent.lines().collect();
        assert!(recent_lines.len() <= head_lines.len(),
            "recent ({}) should have <= head ({}) lines",
            recent_lines.len(), head_lines.len());
    }

    #[test]
    fn build_prompt_includes_template() {
        let prompt = build_compaction_prompt(None, "some context");
        assert!(prompt.contains("## Goal"));
        assert!(prompt.contains("some context"));
        assert!(!prompt.contains("<previous-summary>"));
    }

    #[test]
    fn build_prompt_with_previous_summary() {
        let prompt = build_compaction_prompt(Some("old summary"), "new context");
        assert!(prompt.contains("old summary"));
        assert!(prompt.contains("new context"));
        assert!(prompt.contains("<previous-summary>"));
    }

    #[test]
    fn estimate_request_scales_with_input() {
        let small = estimate_request("hi", &[msg("user", "hello")], &[]);
        let large = estimate_request(
            "long system prompt with many words",
            &[
                msg("user", "a much longer message with significantly more content"),
                msg("assistant", "and a similarly long response back to the user"),
            ],
            &[],
        );
        assert!(large > small);
    }
}
