//! Token estimation utilities for context window management.
//!
//! Uses a character-based heuristic: ~3.5 chars per token for code/mixed text.
//! This is intentionally simple — a full tokenizer would require model-specific
//! vocabularies and is not needed for compaction decisions.

const CHARS_PER_TOKEN: f64 = 3.5;

pub fn estimate(text: &str) -> usize {
    (text.chars().count() as f64 / CHARS_PER_TOKEN).ceil() as usize
}

pub fn estimate_messages(messages: &[crate::core::provider::Message]) -> usize {
    estimate(&serde_json::to_string(messages).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_estimates_zero() {
        assert_eq!(estimate(""), 0);
    }

    #[test]
    fn short_text() {
        let tokens = estimate("hello world");
        assert!(tokens >= 2 && tokens <= 5);
    }

    #[test]
    fn longer_text_scales() {
        let short = estimate("hi");
        let long = estimate("this is a much longer piece of text with many words in it");
        assert!(long > short);
    }

    #[test]
    fn code_snippet() {
        let code = "fn main() { println!(\"hello\"); }";
        let tokens = estimate(code);
        assert!(tokens >= 8 && tokens <= 15);
    }
}
