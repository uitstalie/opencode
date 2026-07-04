//! Last-turn diff rendering via the `similar` crate.

use ratatui::text::{Line, Span};
use similar::{ChangeTag, TextDiff};

use super::render::Theme;

/// Render a line-level diff between `before` and `after` as colored lines.
pub fn render_diff(before: &str, after: &str, theme: &Theme) -> Vec<Line<'static>> {
    let diff = TextDiff::from_lines(before, after);
    diff.iter_all_changes()
        .map(|change| {
            let (sign, style) = match change.tag() {
                ChangeTag::Delete => ("-", theme.diff_delete_style()),
                ChangeTag::Insert => ("+", theme.diff_insert_style()),
                ChangeTag::Equal => (" ", theme.muted_style()),
            };
            let text = change.value().trim_end_matches('\n').to_string();
            Line::from(Span::styled(format!("{} {}", sign, text), style))
        })
        .collect()
}

/// A compact unified-diff string with `+`/`-`/` ` prefixes.
pub fn unified_diff(before: &str, after: &str) -> String {
    TextDiff::from_lines(before, after)
        .iter_all_changes()
        .map(|change| {
            let sign = match change.tag() {
                ChangeTag::Delete => "-",
                ChangeTag::Insert => "+",
                ChangeTag::Equal => " ",
            };
            format!("{} {}", sign, change.value().trim_end_matches('\n'))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::Theme;

    #[test]
    fn detects_insertions_and_deletions() {
        let out = unified_diff("a\nb\n", "a\nc\n");
        assert!(out.contains("- b"));
        assert!(out.contains("+ c"));
        assert!(out.contains("  a"));
    }

    #[test]
    fn render_diff_produces_a_line_per_change() {
        let lines = render_diff("x\n", "x\ny\n", &Theme::dark());
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn identical_inputs_have_no_changes() {
        let out = unified_diff("same\n", "same\n");
        assert!(!out.contains("+ same"));
        assert!(!out.contains("- same"));
    }
}
