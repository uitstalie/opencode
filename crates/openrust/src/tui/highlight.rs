//! Syntax highlighting for code blocks via syntect (pure Rust, fancy-regex).

use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme as SynTheme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<SynTheme> = OnceLock::new();

fn syntaxes() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static SynTheme {
    THEME.get_or_init(|| {
        let mut set = ThemeSet::load_defaults();
        set.themes
            .remove("base16-ocean.dark")
            .or_else(|| set.themes.values().next().cloned())
            .expect("syntect ships default themes")
    })
}

/// Highlight `code` for the given language token (e.g. "rust", "py", "json").
/// Falls back to plain text when the language is unknown.
pub fn highlight_code(code: &str, lang: Option<&str>) -> Vec<Line<'static>> {
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

    let mut highlighter = HighlightLines::new(syntax, theme());
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
        let lines = highlight_code("fn main() {\n    let x = 1;\n}\n", Some("rust"));
        assert_eq!(lines.len(), 3);
        // The first line should be tokenized into more than one styled span.
        assert!(lines[0].spans.len() > 1);
    }

    #[test]
    fn unknown_language_falls_back_to_plain() {
        let lines = highlight_code("just text\n", Some("nonesuch"));
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn handles_no_language() {
        let lines = highlight_code("plain\nlines\n", None);
        assert_eq!(lines.len(), 2);
    }
}
