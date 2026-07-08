//! HTML → styled text conversion for terminal display.
//!
//! Strips tags, decodes entities, and applies ratatui styles for
//! common inline elements (bold, italic, code, etc.). Block-level
//! HTML is reduced to plain text with line breaks.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::latex::{to_subscript, to_superscript};
use super::render::Theme;

/// Strip all HTML tags and decode entities. Block-level tags become newlines.
pub fn html_to_text(html: &str) -> String {
    let chars: Vec<char> = html.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            let (tag, is_closing, _, end) = parse_tag(&chars, i);
            if !is_closing && is_block_tag(&tag)
                && !result.is_empty() && !result.ends_with('\n') {
                    result.push('\n');
                }
            i = end;
        } else if chars[i] == '&' {
            let (decoded, next) = decode_entity(&chars, i);
            result.push_str(&decoded);
            i = next;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Parse inline HTML into styled spans. Known tags get styles,
/// unknown tags are stripped, entities are decoded.
pub fn html_to_spans(html: &str, theme: &Theme) -> Vec<Span<'static>> {
    let chars: Vec<char> = html.chars().collect();
    let mut pos = 0;
    parse_content(&chars, &mut pos, theme, Style::default(), None)
}

fn parse_content(
    chars: &[char],
    pos: &mut usize,
    theme: &Theme,
    style: Style,
    stop_tag: Option<&str>,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut text = String::new();

    while *pos < chars.len() {
        if chars[*pos] == '<' {
            if !text.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut text), style));
            }

            let tag_start = *pos;
            let (tag, is_closing, self_closing, end) = parse_tag(chars, *pos);
            *pos = end;

            if tag.is_empty() {
                continue;
            }

            if is_closing {
                if stop_tag == Some(tag.as_str()) {
                    return spans;
                }
            } else if self_closing || is_void_tag(&tag) {
                if tag == "br" {
                    spans.push(Span::raw("\n"));
                }
            } else {
                let child_style = merge_style(&tag, style, theme);
                let child_spans = parse_content(chars, pos, theme, child_style, Some(&tag));

                if tag == "sup" || tag == "sub" {
                    let inner: String = child_spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    let conv = if tag == "sup" {
                        to_superscript(&inner)
                    } else {
                        to_subscript(&inner)
                    };
                    let converted = conv.unwrap_or_else(|| {
                        let op = if tag == "sup" { '^' } else { '_' };
                        format!("{}({})", op, inner)
                    });
                    spans.push(Span::styled(converted, style));
                } else if tag == "a" {
                    let raw: String = chars[tag_start..end].iter().collect();
                    spans.extend(child_spans);
                    if let Some(href) = extract_href(&raw) {
                        spans.push(Span::styled(
                            format!(" ({})", href),
                            theme.muted_style(),
                        ));
                    }
                } else {
                    spans.extend(child_spans);
                }
            }
        } else if chars[*pos] == '&' {
            let (decoded, next) = decode_entity(chars, *pos);
            text.push_str(&decoded);
            *pos = next;
        } else {
            text.push(chars[*pos]);
            *pos += 1;
        }
    }

    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }

    spans
}

fn parse_tag(chars: &[char], start: usize) -> (String, bool, bool, usize) {
    let mut i = start + 1;

    if i < chars.len() && chars[i] == '!' {
        while i < chars.len() && chars[i] != '>' {
            i += 1;
        }
        if i < chars.len() {
            i += 1;
        }
        return (String::new(), false, true, i);
    }

    let is_closing = i < chars.len() && chars[i] == '/';
    if is_closing {
        i += 1;
    }

    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }

    let name_start = i;
    while i < chars.len() && chars[i].is_ascii_alphanumeric() {
        i += 1;
    }
    let name: String = chars[name_start..i]
        .iter()
        .map(|c| c.to_ascii_lowercase())
        .collect();

    let mut self_closing = false;
    while i < chars.len() && chars[i] != '>' {
        if chars[i] == '/' {
            self_closing = true;
        }
        i += 1;
    }
    if i < chars.len() {
        i += 1;
    }

    (name, is_closing, self_closing, i)
}

fn decode_entity(chars: &[char], start: usize) -> (String, usize) {
    let mut end = start + 1;
    while end < chars.len() && chars[end] != ';' && end - start < 12 {
        end += 1;
    }

    if end < chars.len() && chars[end] == ';' {
        let entity: String = chars[start + 1..end].iter().collect();
        let next = end + 1;

        if let Some(stripped) = entity.strip_prefix('#') {
            if let Some(hex) = stripped
                .strip_prefix('x')
                .or_else(|| stripped.strip_prefix('X'))
            {
                if let Ok(code) = u32::from_str_radix(hex, 16)
                    && let Some(c) = char::from_u32(code) {
                        return (c.to_string(), next);
                    }
            } else if let Ok(code) = stripped.parse::<u32>()
                && let Some(c) = char::from_u32(code) {
                    return (c.to_string(), next);
                }
        }

        if let Some(s) = named_entity(&entity) {
            return (s.to_string(), next);
        }
    }

    (chars[start].to_string(), start + 1)
}

fn named_entity(name: &str) -> Option<&'static str> {
    let sym = match name {
        "amp" => "&", "lt" => "<", "gt" => ">", "quot" => "\"", "apos" => "'",
        "nbsp" => " ", "copy" => "©", "reg" => "®", "trade" => "™",
        "mdash" => "—", "ndash" => "–", "hellip" => "…", "laquo" => "«",
        "raquo" => "»", "times" => "×", "divide" => "÷", "deg" => "°",
        "euro" => "€", "pound" => "£", "yen" => "¥", "cent" => "¢",
        "para" => "¶", "sect" => "§", "bull" => "•", "middot" => "·",
        "larr" => "←", "rarr" => "→", "uarr" => "↑", "darr" => "↓",
        "harr" => "↔", "crarr" => "↵", "infin" => "∞", "ne" => "≠",
        "le" => "≤", "ge" => "≥", "plusmn" => "±", "frac12" => "½",
        "frac14" => "¼", "frac34" => "¾", "sup2" => "²", "sup3" => "³",
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ",
        "pi" => "π", "Sigma" => "Σ", "sum" => "∑", "int" => "∫",
        "radic" => "√", "prop" => "∝", "part" => "∂", "nabla" => "∇",
        "forall" => "∀", "exist" => "∃", "empty" => "∅", "isin" => "∈",
        "notin" => "∉", "sub" => "⊂", "sup" => "⊃", "cap" => "∩",
        "cup" => "∪", "and" => "∧", "or" => "∨", "lceil" => "⌈",
        "rceil" => "⌉", "lfloor" => "⌊", "rfloor" => "⌋", "lang" => "⟨",
        "rang" => "⟩", "loz" => "◊", "spades" => "♠", "clubs" => "♣",
        "hearts" => "♥", "diams" => "♦",
        _ => return None,
    };
    Some(sym)
}

fn extract_href(tag: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let href_pos = lower.find("href")?;
    let after = &tag[href_pos + 4..];
    let after = after.trim_start().strip_prefix('=')?.trim_start();

    if let Some(s) = after.strip_prefix('"') {
        Some(s[..s.find('"')?].to_string())
    } else if let Some(s) = after.strip_prefix('\'') {
        Some(s[..s.find('\'')?].to_string())
    } else {
        let end = after
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(after.len());
        Some(after[..end].to_string())
    }
}

fn merge_style(tag: &str, current: Style, theme: &Theme) -> Style {
    match tag {
        "b" | "strong" => current.add_modifier(Modifier::BOLD),
        "i" | "em" => current.add_modifier(Modifier::ITALIC),
        "code" | "kbd" | "samp" | "tt" => theme.tool_style(),
        "s" | "del" | "strike" => current.add_modifier(Modifier::CROSSED_OUT),
        "u" | "ins" => current.add_modifier(Modifier::UNDERLINED),
        "small" => current.add_modifier(Modifier::DIM),
        "mark" => current.add_modifier(Modifier::REVERSED),
        _ => current,
    }
}

fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "br" | "hr" | "img" | "input" | "meta" | "link" | "area"
            | "base" | "col" | "embed" | "source" | "track" | "wbr"
    )
}

fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div" | "br" | "hr" | "li" | "tr" | "h1" | "h2" | "h3"
            | "h4" | "h5" | "h6" | "ul" | "ol" | "table" | "thead"
            | "tbody" | "blockquote" | "pre" | "section" | "article"
            | "header" | "footer" | "nav" | "aside" | "figure"
            | "figcaption" | "details" | "summary"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_simple_tags() {
        assert_eq!(html_to_text("<b>hello</b>"), "hello");
        assert_eq!(html_to_text("<p>world</p>"), "world");
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(html_to_text("a &amp; b"), "a & b");
        assert_eq!(html_to_text("&lt;tag&gt;"), "<tag>");
        assert_eq!(html_to_text("&#65;"), "A");
        assert_eq!(html_to_text("&#x41;"), "A");
        assert_eq!(html_to_text("&copy; 2024"), "© 2024");
    }

    #[test]
    fn block_tags_add_newlines() {
        let result = html_to_text("<p>one</p><p>two</p>");
        assert_eq!(result, "one\ntwo");
    }

    #[test]
    fn nested_tags() {
        assert_eq!(html_to_text("<b>bold <i>both</i></b>"), "bold both");
    }

    #[test]
    fn self_closing_br() {
        assert_eq!(html_to_text("a<br>b"), "a\nb");
        assert_eq!(html_to_text("a<br/>b"), "a\nb");
    }

    #[test]
    fn unknown_tag_stripped() {
        assert_eq!(html_to_text("<foo>bar</foo>"), "bar");
    }

    #[test]
    fn comment_skipped() {
        assert_eq!(html_to_text("a<!-- comment -->b"), "ab");
    }
}
