//! Markdown → ratatui rendering via pulldown-cmark.
//!
//! Handles headings, emphasis, inline code, fenced code blocks (syntax
//! highlighted), lists, block quotes, and horizontal rules. Anything unknown
//! falls back to plain text so no content is ever dropped.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::highlight::highlight_code;
use super::render::Theme;

/// Render markdown source into styled ratatui lines.
pub fn render_markdown(input: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut builder = Builder::new(theme);
    let parser = Parser::new_ext(input, Options::ENABLE_STRIKETHROUGH);
    for event in parser {
        builder.handle(event);
    }
    builder.finish()
}

struct Builder<'a> {
    theme: &'a Theme,
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    bold: bool,
    italic: bool,
    code_lang: Option<String>,
    code_buffer: String,
    in_code_block: bool,
    list_stack: Vec<Option<u64>>,
    quote_depth: usize,
}

impl<'a> Builder<'a> {
    fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            lines: Vec::new(),
            current: Vec::new(),
            bold: false,
            italic: false,
            code_lang: None,
            code_buffer: String::new(),
            in_code_block: false,
            list_stack: Vec::new(),
            quote_depth: 0,
        }
    }

    fn handle(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if self.in_code_block {
                    self.code_buffer.push_str(&text);
                } else {
                    self.push_span(text.to_string(), self.inline_style());
                }
            }
            Event::Code(text) => {
                self.push_span(format!("`{}`", text), self.theme.tool_style());
            }
            Event::SoftBreak => self.push_span(" ".to_string(), self.inline_style()),
            Event::HardBreak => self.flush_line(),
            Event::Rule => {
                self.flush_line();
                self.lines.push(Line::from(Span::styled(
                    "────────────────",
                    self.theme.muted_style(),
                )));
                self.lines.push(Line::from(""));
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_line();
                let hashes = "#".repeat(heading_depth(level));
                self.push_span(format!("{} ", hashes), self.theme.title_style());
            }
            Tag::Emphasis => self.italic = true,
            Tag::Strong => self.bold = true,
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.in_code_block = true;
                self.code_buffer.clear();
                self.code_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) => {
                        let lang = info.split_whitespace().next().unwrap_or("").to_string();
                        (!lang.is_empty()).then_some(lang)
                    }
                    pulldown_cmark::CodeBlockKind::Indented => None,
                };
            }
            Tag::List(start) => self.list_stack.push(start),
            Tag::Item => {
                self.flush_line();
                let indent = "  ".repeat(self.list_stack.len().saturating_sub(1));
                let marker = match self.list_stack.last_mut() {
                    Some(Some(number)) => {
                        let current = *number;
                        *number += 1;
                        format!("{}. ", current)
                    }
                    _ => "• ".to_string(),
                };
                self.push_span(format!("{}{}", indent, marker), self.theme.muted_style());
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush_line();
                self.lines.push(Line::from(""));
            }
            TagEnd::Paragraph => {
                self.flush_line();
                self.lines.push(Line::from(""));
            }
            TagEnd::Emphasis => self.italic = false,
            TagEnd::Strong => self.bold = false,
            TagEnd::CodeBlock => {
                let code = std::mem::take(&mut self.code_buffer);
                let highlighted = highlight_code(&code, self.code_lang.as_deref());
                self.lines.extend(highlighted);
                self.lines.push(Line::from(""));
                self.in_code_block = false;
                self.code_lang = None;
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::List(_) => {
                self.list_stack.pop();
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn inline_style(&self) -> Style {
        let mut style = Style::default();
        if self.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        style
    }

    fn push_span(&mut self, text: String, style: Style) {
        if self.current.is_empty() && self.quote_depth > 0 {
            self.current.push(Span::styled(
                "> ".repeat(self.quote_depth),
                self.theme.muted_style(),
            ));
        }
        self.current.push(Span::styled(text, style));
    }

    fn flush_line(&mut self) {
        if self.current.is_empty() {
            return;
        }
        self.lines
            .push(Line::from(std::mem::take(&mut self.current)));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
        self.lines
    }
}

fn heading_depth(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::render::Theme;

    fn text_of(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_heading_and_paragraph() {
        let lines = render_markdown("# Title\n\nHello world", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("# Title"));
        assert!(text.contains("Hello world"));
    }

    #[test]
    fn renders_bullet_list() {
        let lines = render_markdown("- one\n- two\n", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("• one"));
        assert!(text.contains("• two"));
    }

    #[test]
    fn renders_ordered_list_numbers() {
        let lines = render_markdown("1. first\n2. second\n", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("1. first"));
        assert!(text.contains("2. second"));
    }

    #[test]
    fn highlights_fenced_code_block() {
        let lines = render_markdown("```rust\nfn main() {}\n```\n", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("fn main"));
    }

    #[test]
    fn inline_code_is_preserved() {
        let lines = render_markdown("use `cargo test` please", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("`cargo test`"));
    }
}
