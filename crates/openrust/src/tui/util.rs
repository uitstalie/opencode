//! Utility functions extracted from the TUI root module.

use crossterm::event::{self, KeyCode, KeyModifiers};
use tui_textarea::{Input as TextAreaInput, Key as TextAreaKey, TextArea};

pub(super) fn now_micros() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
}

pub(super) fn single_line_textarea(value: &str, secret: bool) -> TextArea<'static> {
    let mut textarea = TextArea::default();
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    if !value.is_empty() {
        textarea.insert_str(normalize_single_line_text(value));
    }
    if secret {
        textarea.set_mask_char('*');
    }
    textarea
}

pub(super) fn normalize_single_line_text(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// Convert a textarea cursor (row, char-col) into a terminal display column,
/// accounting for wide characters (CJK, emoji) that occupy 2 cells.
pub(super) fn textarea_display_col(lines: &[String], row: usize, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;
    lines
        .get(row)
        .map(|line| line.chars().take(col).map(|c| c.width().unwrap_or(1)).sum())
        .unwrap_or(col)
}

pub(super) fn textarea_input_from_key_event(key: event::KeyEvent) -> TextAreaInput {
    let input_key = match key.code {
        KeyCode::Char(ch) => TextAreaKey::Char(ch),
        KeyCode::Backspace => TextAreaKey::Backspace,
        KeyCode::Enter => TextAreaKey::Enter,
        KeyCode::Left => TextAreaKey::Left,
        KeyCode::Right => TextAreaKey::Right,
        KeyCode::Up => TextAreaKey::Up,
        KeyCode::Down => TextAreaKey::Down,
        KeyCode::Tab => TextAreaKey::Tab,
        KeyCode::Delete => TextAreaKey::Delete,
        KeyCode::Home => TextAreaKey::Home,
        KeyCode::End => TextAreaKey::End,
        KeyCode::PageUp => TextAreaKey::PageUp,
        KeyCode::PageDown => TextAreaKey::PageDown,
        KeyCode::Esc => TextAreaKey::Esc,
        KeyCode::F(number) => TextAreaKey::F(number),
        _ => TextAreaKey::Null,
    };
    TextAreaInput {
        key: input_key,
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
    }
}

pub(super) fn char_column_to_byte_index(text: &str, column: usize) -> usize {
    text.char_indices()
        .nth(column)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

pub(super) fn read_line_span(result: &str) -> Option<(usize, usize)> {
    let mut numbers = result.lines().filter_map(|line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("...") {
            return None;
        }
        let colon = trimmed.find(':')?;
        trimmed[..colon].trim().parse::<usize>().ok()
    });
    let first = numbers.next()?;
    Some((first, numbers.next_back().unwrap_or(first)))
}

pub(super) fn home_input_hint() -> &'static str {
    "输入消息后 Enter 开始 · /connect 配置 provider · /models 选择模型 · Esc 退出"
}

/// Strip ANSI escape sequences and C0/C1 control characters from text before
/// it reaches the render pipeline. Tool output and provider errors may carry
/// color codes, `\r` progress rewrites, or cursor-move sequences; written
/// raw into a buffer cell they are interpreted by the real terminal and
/// corrupt the whole frame (rows shifted, leading columns eaten).
///
/// Keeps `\n` and `\t`. Borrows when the text is already clean.
pub(super) fn strip_terminal_controls(text: &str) -> std::borrow::Cow<'_, str> {
    let dirty = text
        .chars()
        .any(|c| c == '\x1b' || (c.is_control() && c != '\n' && c != '\t'));
    if !dirty {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.peek() {
                // CSI: ESC [ params… final byte in @..=~
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] … terminated by BEL or ST (ESC \)
                Some(']') => {
                    chars.next();
                    let mut prev_esc = false;
                    for c in chars.by_ref() {
                        if c == '\x07' || (prev_esc && c == '\\') {
                            break;
                        }
                        prev_esc = c == '\x1b';
                    }
                }
                // Lone ESC or other sequence: drop the ESC only.
                _ => {}
            }
            continue;
        }
        if c.is_control() && c != '\n' && c != '\t' {
            continue;
        }
        out.push(c);
    }
    std::borrow::Cow::Owned(out)
}
