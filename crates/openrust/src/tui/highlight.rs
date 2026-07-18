//! Syntax highlighting for code blocks via syntect (pure Rust, fancy-regex).
//!
//! The syntect theme follows the active TUI theme (`Theme::syntect_theme`),
//! so code blocks stay readable when the user switches dark/light/hacker.

use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme as SynTheme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

const FALLBACK_THEME: &str = "base16-ocean.dark";

fn syntaxes() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

fn syntect_theme(name: &str) -> &'static SynTheme {
    let set = theme_set();
    set.themes
        .get(name)
        .or_else(|| set.themes.get(FALLBACK_THEME))
        .or_else(|| set.themes.values().next())
        .expect("syntect ships default themes")
}

/// Highlight `code` for the given language token (e.g. "rust", "py", "json").
/// `theme_name` is the syntect theme from the active TUI theme; unknown
/// names fall back to `base16-ocean.dark`. Unknown languages use plain text.
pub fn highlight_code(code: &str, lang: Option<&str>, theme_name: &str) -> Vec<Line<'static>> {
    let syntaxes = syntaxes();
    let syntax = lang
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .and_then(|l| {
            syntaxes
                .find_syntax_by_token(l)
                .or_else(|| syntaxes.find_syntax_by_extension(l))
                .or_else(|| syntaxes.find_syntax_by_name(l))
        })
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());

    let mut highlighter = HighlightLines::new(syntax, syntect_theme(theme_name));
    LinesWithEndings::from(code)
        .map(|line| {
            let Ok(ranges) = highlighter.highlight_line(line, syntaxes) else {
                return Line::from(line.trim_end_matches('\n').to_string());
            };
            let spans = ranges
                .into_iter()
                .map(|(style, text)| {
                    Span::styled(
                        text.trim_end_matches('\n').to_string(),
                        Style::default().fg(to_color(style.foreground)),
                    )
                })
                .collect::<Vec<_>>();
            Line::from(spans)
        })
        .collect()
}

fn to_color(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_into_multiple_spans() {
        let lines = highlight_code("fn main() {\n    let x = 1;\n}\n", Some("rust"), FALLBACK_THEME);
        assert_eq!(lines.len(), 3);
        // The first line should be tokenized into more than one styled span.
        assert!(lines[0].spans.len() > 1);
    }

    #[test]
    fn theme_name_selects_palette() {
        let dark = highlight_code("fn main() {}", Some("rust"), "base16-ocean.dark");
        let light = highlight_code("fn main() {}", Some("rust"), "InspiredGitHub");
        // Same code under two syntect themes must produce different colors.
        assert_ne!(
            dark[0].spans.iter().map(|s| s.style.fg).collect::<Vec<_>>(),
            light[0].spans.iter().map(|s| s.style.fg).collect::<Vec<_>>()
        );
    }

    #[test]
    fn builtin_tui_themes_resolve_to_syntect_themes() {
        for name in ["dark", "light", "hacker"] {
            let tf = crate::tui::theme::builtin(name).unwrap();
            let t = tf.to_theme().unwrap();
            let set = theme_set();
            assert!(
                set.themes.contains_key(t.syntect_theme()),
                "theme {name} points at unknown syntect theme {}",
                t.syntect_theme()
            );
        }
    }

    #[test]
    fn unknown_theme_falls_back() {
        let lines = highlight_code("let x = 1;", Some("rust"), "no-such-theme");
        assert!(!lines.is_empty());
    }

    #[test]
    fn unknown_language_falls_back_to_plain() {
        let lines = highlight_code("just text\n", Some("nonesuch"), FALLBACK_THEME);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn handles_no_language() {
        let lines = highlight_code("plain\nlines\n", None, FALLBACK_THEME);
        assert_eq!(lines.len(), 2);
    }
}
