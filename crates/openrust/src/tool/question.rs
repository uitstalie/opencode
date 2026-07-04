//! Question tool — ask the user structured questions via the interactive UI.
//!
//! The tool forwards questions over `ToolContext.ask_tx` and blocks until the
//! TUI collects answers (one string per sub-question). In non-interactive
//! contexts (`ask_tx` is `None`) it returns an error instead of hanging.

use crate::tool::{AskRequest, Tool, ToolContext, ToolParams, ToolResult};
use serde_json::Value;

pub struct QuestionTool;

#[async_trait::async_trait]
impl Tool for QuestionTool {
    fn name(&self) -> &'static str {
        "question"
    }
    fn description(&self) -> &'static str {
        "Ask the user one or more multiple-choice questions and wait for their answers. Use to gather preferences or resolve ambiguity. Each question shows selectable options; the user may also type a custom answer."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "Questions to ask",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": { "type": "string", "description": "The complete question" },
                            "header": { "type": "string", "description": "Very short label (max 30 chars)" },
                            "multiple": { "type": "boolean", "description": "Allow selecting more than one option" },
                            "options": {
                                "type": "array",
                                "description": "Available choices",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": { "type": "string", "description": "Display text" },
                                        "description": { "type": "string", "description": "Explanation of the choice" }
                                    },
                                    "required": ["label"]
                                }
                            }
                        },
                        "required": ["question", "header", "options"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let Some(ask_tx) = ctx.ask_tx.as_ref() else {
            return ToolResult::error(
                "question: interactive input is not available in this context",
            );
        };
        let Some(questions) = p
            .raw_value()
            .get("questions")
            .cloned()
            .filter(|value| value.as_array().is_some_and(|a| !a.is_empty()))
        else {
            return ToolResult::error("question: missing required parameter: questions");
        };

        let (responder, answers_rx) = std::sync::mpsc::channel();
        if ask_tx
            .send(AskRequest {
                questions: questions.clone(),
                responder,
            })
            .is_err()
        {
            return ToolResult::error("question: interactive UI is no longer available");
        }

        let answers = match answers_rx.recv() {
            Ok(answers) => answers,
            Err(_) => return ToolResult::error("question: no answer was received"),
        };

        let rendered = questions
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(index, question)| {
                let header = question.get("header").and_then(|v| v.as_str()).unwrap_or("");
                let label = if header.is_empty() {
                    format!("Q{}", index + 1)
                } else {
                    header.to_string()
                };
                let answer = answers.get(index).map(String::as_str).unwrap_or("(no answer)");
                format!("{}: {}", label, answer)
            })
            .collect::<Vec<_>>()
            .join("\n");

        ToolResult::text(format!("User answered:\n{}", rendered))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn errors_without_interactive_channel() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let result = QuestionTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "questions": [{
                        "question": "Pick one",
                        "header": "pick",
                        "options": [{ "label": "a", "description": "" }]
                    }]
                })),
                &ctx,
            )
            .await;
        assert!(matches!(result, ToolResult::Error(_)));
    }

    #[tokio::test]
    async fn round_trips_answer_through_channel() {
        let (ask_tx, ask_rx) = std::sync::mpsc::channel::<AskRequest>();
        // Simulated UI: receive the request and answer it.
        let ui = std::thread::spawn(move || {
            let request = ask_rx.recv().unwrap();
            request.responder.send(vec!["blue".to_string()]).unwrap();
        });

        let mut ctx = ToolContext::new(std::env::temp_dir());
        ctx.ask_tx = Some(ask_tx);
        let result = QuestionTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "questions": [{
                        "question": "Favorite color?",
                        "header": "color",
                        "options": [{ "label": "blue", "description": "" }]
                    }]
                })),
                &ctx,
            )
            .await;
        ui.join().unwrap();
        let text = result.into_text();
        assert!(text.contains("color: blue"));
    }
}
