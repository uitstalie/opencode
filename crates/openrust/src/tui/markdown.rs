//! Markdown → ratatui rendering via pulldown-cmark.
//!
//! Full GFM feature support: headings, emphasis, strikethrough, inline
//! code, fenced code blocks (syntax highlighted), ordered/unordered/task
//! lists, tables, block quotes (incl. GFM admonitions), links, images,
//! footnotes, definition lists, and horizontal rules.

use pulldown_cmark::{
    Alignment, BlockQuoteKind, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use super::highlight::highlight_code;
use super::html;
use super::latex;
use super::render::Theme;

/// Render markdown source into styled ratatui lines.
pub fn render_markdown(input: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut builder = Builder::new(theme);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH;
    for event in Parser::new_ext(input, options) {
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
    strikethrough: bool,
    link_url: Option<String>,
    in_link: bool,
    code_lang: Option<String>,
    code_buffer: String,
    in_code_block: bool,
    list_stack: Vec<Option<u64>>,
    quote_depth: usize,
    table_alignments: Vec<Alignment>,
    table_grid: Vec<Vec<String>>,
    table_row: Vec<String>,
    table_cell_buf: String,
    in_table_cell: bool,
    html_buffer: String,
    in_html_block: bool,
    in_skip: bool,
}

impl<'a> Builder<'a> {
    fn new(theme: &'a Theme) -> Self {
        Self {
            theme,
            lines: Vec::new(),
            current: Vec::new(),
            bold: false,
            italic: false,
            strikethrough: false,
            link_url: None,
            in_link: false,
            code_lang: None,
            code_buffer: String::new(),
            in_code_block: false,
            list_stack: Vec::new(),
            quote_depth: 0,
            table_alignments: Vec::new(),
            table_grid: Vec::new(),
            table_row: Vec::new(),
            table_cell_buf: String::new(),
            in_table_cell: false,
            html_buffer: String::new(),
            in_html_block: false,
            in_skip: false,
        }
    }

    fn handle(&mut self, event: Event<'_>) {
        if self.in_skip {
            if let Event::End(TagEnd::MetadataBlock(_)) = event {
                self.in_skip = false;
            }
            return;
        }

        if self.in_html_block {
            match event {
                Event::End(TagEnd::HtmlBlock) => {
                    self.in_html_block = false;
                    let html_input = std::mem::take(&mut self.html_buffer);
                    let text = html::html_to_text(&html_input);
                    for line in text.lines() {
                        self.lines.push(Line::from(Span::styled(
                            line.to_string(),
                            self.theme.muted_style(),
                        )));
                    }
                    self.lines.push(Line::from(""));
                }
                Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                    self.html_buffer.push_str(&t);
                }
                _ => {}
            }
            return;
        }

        if self.in_table_cell {
            match event {
                Event::End(TagEnd::TableCell) => {
                    self.in_table_cell = false;
                    let cell = std::mem::take(&mut self.table_cell_buf);
                    self.table_row.push(cell.trim().to_string());
                }
                Event::Text(t) | Event::Code(t) => {
                    self.table_cell_buf.push_str(&t);
                }
                Event::SoftBreak => self.table_cell_buf.push(' '),
                _ => {}
            }
            return;
        }

        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if self.in_code_block {
                    self.code_buffer.push_str(&text);
                } else {
                    self.push_span(latex::latex_to_unicode(&text), self.inline_style());
                }
            }
            Event::Code(text) => {
                self.push_span(text.to_string(), self.theme.code_style());
            }
            Event::InlineMath(text) => {
                self.push_span(latex::latex_to_unicode(&text), self.theme.code_style());
            }
            Event::DisplayMath(text) => {
                self.flush_line();
                for line in latex::latex_to_unicode(&text).lines() {
                    self.lines.push(Line::from(Span::styled(
                        line.to_string(),
                        self.theme.code_style(),
                    )));
                }
                self.lines.push(Line::from(""));
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                if self.current.is_empty() && self.quote_depth > 0 {
                    self.current.push(Span::styled(
                        "> ".repeat(self.quote_depth),
                        self.theme.blockquote_style(),
                    ));
                }
                self.current
                    .extend(html::html_to_spans(&text, self.theme));
            }
            Event::FootnoteReference(label) => {
                self.push_span(format!("[^{}]", label), self.theme.muted_style());
            }
            Event::SoftBreak => {
                self.push_span(" ".to_string(), self.inline_style());
            }
            Event::HardBreak => self.flush_line(),
            Event::Rule => {
                self.flush_line();
                self.lines.push(Line::from(Span::styled(
                    "────────────────────────",
                    self.theme.muted_style(),
                )));
                self.lines.push(Line::from(""));
            }
            Event::TaskListMarker(checked) => {
                let marker = if checked { "[x] " } else { "[ ] " };
                self.push_span(marker.to_string(), self.theme.muted_style());
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_line();
                let hashes = "#".repeat(heading_depth(level));
                self.push_span(format!("{} ", hashes), self.theme.heading_style());
            }
            Tag::BlockQuote(kind) => {
                self.quote_depth += 1;
                if let Some(k) = kind {
                    self.flush_line();
                    self.lines.push(Line::from(Span::styled(
                        format!("[{}]", block_quote_kind_label(k)),
                        self.theme.assistant_style(),
                    )));
                }
            }
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.in_code_block = true;
                self.code_buffer.clear();
                self.code_lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let lang = info.split_whitespace().next().unwrap_or("").to_string();
                        (!lang.is_empty()).then_some(lang)
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Tag::HtmlBlock => {
                self.flush_line();
                self.in_html_block = true;
                self.html_buffer.clear();
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
                self.push_span(format!("{}{}", indent, marker), self.theme.list_marker_style());
            }
            Tag::FootnoteDefinition(label) => {
                self.flush_line();
                self.push_span(format!("[^{}]: ", label), self.theme.muted_style());
            }
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                self.flush_line();
            }
            Tag::Table(alignments) => {
                self.flush_line();
                self.table_alignments = alignments;
                self.table_grid.clear();
            }
            Tag::TableHead | Tag::TableRow => {
                self.table_row.clear();
            }
            Tag::TableCell => {
                self.in_table_cell = true;
                self.table_cell_buf.clear();
            }
            Tag::Emphasis => self.italic = true,
            Tag::Strong => self.bold = true,
            Tag::Strikethrough => self.strikethrough = true,
            Tag::Link { dest_url, .. } => {
                self.link_url = Some(dest_url.to_string());
                self.in_link = true;
            }
            Tag::Image { dest_url, .. } => {
                self.link_url = Some(dest_url.to_string());
            }
            Tag::MetadataBlock(_) => {
                self.in_skip = true;
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_line();
                self.lines.push(Line::from(""));
            }
            TagEnd::Heading(_) => {
                self.flush_line();
                self.lines.push(Line::from(""));
            }
            TagEnd::BlockQuote(_) => {
                self.flush_line();
                self.quote_depth = self.quote_depth.saturating_sub(1);
                self.lines.push(Line::from(""));
            }
            TagEnd::CodeBlock => {
                let code = std::mem::take(&mut self.code_buffer);
                let highlighted =
                    highlight_code(&code, self.code_lang.as_deref(), self.theme.syntect_theme());
                for line in highlighted {
                    let mut spans = vec![Span::styled("│ ", self.theme.muted_style())];
                    spans.extend(line.spans);
                    self.lines.push(Line::from(spans));
                }
                self.lines.push(Line::from(""));
                self.in_code_block = false;
                self.code_lang = None;
            }
            TagEnd::HtmlBlock => {}
            TagEnd::List(_) => {
                self.list_stack.pop();
                self.lines.push(Line::from(""));
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::FootnoteDefinition => {
                self.flush_line();
                self.lines.push(Line::from(""));
            }
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {
                self.flush_line();
            }
            TagEnd::Table => {
                let grid = std::mem::take(&mut self.table_grid);
                let alignments = std::mem::take(&mut self.table_alignments);
                self.lines
                    .extend(build_table(&grid, &alignments, self.theme));
                self.lines.push(Line::from(""));
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                self.table_grid.push(std::mem::take(&mut self.table_row));
            }
            TagEnd::TableCell => {
                self.in_table_cell = false;
                let cell = std::mem::take(&mut self.table_cell_buf);
                self.table_row.push(cell.trim().to_string());
            }
            TagEnd::Emphasis => self.italic = false,
            TagEnd::Strong => self.bold = false,
            TagEnd::Strikethrough => self.strikethrough = false,
            TagEnd::Link => {
                self.in_link = false;
                if let Some(url) = self.link_url.take() {
                    self.push_span(format!(" ({})", url), self.theme.muted_style());
                }
            }
            TagEnd::Image => {
                if let Some(url) = self.link_url.take() {
                    self.push_span(format!(" ({})", url), self.theme.muted_style());
                }
            }
            TagEnd::MetadataBlock(_) => {}
        }
    }

    fn inline_style(&self) -> Style {
        // Body text keeps the assistant body color so the streaming →
        // rendered transition does not shift the text color; emphasis
        // elements (links, code, quotes) keep their own accents.
        let mut style = self.theme.assistant_text_style();
        if self.bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strikethrough {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.in_link {
            style = style
                .add_modifier(Modifier::UNDERLINED)
                .fg(self.theme.link_color());
        }
        style
    }

    fn push_span(&mut self, text: String, style: Style) {
        if self.current.is_empty() && self.quote_depth > 0 {
            self.current.push(Span::styled(
                "> ".repeat(self.quote_depth),
                self.theme.blockquote_style(),
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

fn block_quote_kind_label(kind: BlockQuoteKind) -> &'static str {
    match kind {
        BlockQuoteKind::Note => "Note",
        BlockQuoteKind::Tip => "Tip",
        BlockQuoteKind::Important => "Important",
        BlockQuoteKind::Warning => "Warning",
        BlockQuoteKind::Caution => "Caution",
    }
}

fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn build_table(
    grid: &[Vec<String>],
    alignments: &[Alignment],
    theme: &Theme,
) -> Vec<Line<'static>> {
    if grid.is_empty() {
        return Vec::new();
    }
    let n_cols = alignments
        .len()
        .max(grid.iter().map(|r| r.len()).max().unwrap_or(0));
    if n_cols == 0 {
        return Vec::new();
    }

    let col_w: Vec<usize> = (0..n_cols)
        .map(|i| {
            grid.iter()
                .filter_map(|r| r.get(i))
                .map(|c| display_width(c))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let mut result = Vec::new();
    for (row_idx, row) in grid.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (i, max_w) in col_w.iter().enumerate().take(n_cols) {
            let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
            let cw = display_width(cell);
            let gap = max_w.saturating_sub(cw);
            let (lp, rp) = match alignments.get(i).copied() {
                Some(Alignment::Center) => (gap / 2, gap - gap / 2),
                Some(Alignment::Right) => (gap, 0),
                _ => (0, gap),
            };
            if i > 0 {
                spans.push(Span::styled(" │ ", theme.muted_style()));
            }
            let style = if row_idx == 0 {
                theme.title_style()
            } else {
                Style::default()
            };
            let mut content = String::with_capacity(*max_w);
            content.push_str(&" ".repeat(lp));
            content.push_str(cell);
            content.push_str(&" ".repeat(rp));
            spans.push(Span::styled(content, style));
        }
        result.push(Line::from(spans));

        if row_idx == 0 {
            let mut sep = String::new();
            for (i, max_w) in col_w.iter().enumerate().take(n_cols) {
                if i > 0 {
                    sep.push_str("─┼─");
                }
                sep.push_str(&"─".repeat(*max_w));
            }
            result.push(Line::from(Span::styled(sep, theme.muted_style())));
        }
    }
    result
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
    fn inline_code_without_backticks() {
        let lines = render_markdown("use `cargo test` please", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("cargo test"));
        assert!(!text.contains("`cargo test`"));
    }

    #[test]
    fn renders_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let lines = render_markdown(md, &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("A"));
        assert!(text.contains("B"));
        assert!(text.contains("1"));
        assert!(text.contains("2"));
    }

    #[test]
    fn renders_strikethrough() {
        let lines = render_markdown("~~deleted~~", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("deleted"));
    }

    #[test]
    fn renders_task_list() {
        let lines = render_markdown("- [x] done\n- [ ] todo\n", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("[x]"));
        assert!(text.contains("[ ]"));
    }

    #[test]
    fn renders_link_with_url() {
        let lines = render_markdown("[GitHub](https://github.com)", &Theme::dark());
        let text = text_of(&lines);
        assert!(text.contains("GitHub"));
        assert!(text.contains("https://github.com"));
    }
}
