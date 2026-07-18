//! Theme configuration — serializable color definitions, built-in presets,
//! and the runtime [`Theme`] consumed by all render code.
//!
//! `ThemeFile` is the JSON-serializable form (hex strings); `Theme` is the
//! runtime form with `ratatui::style::Color` fields plus the syntect theme
//! name used for code-block highlighting.

use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

// ── Runtime theme ────────────────────────────────────────────────

/// Runtime color palette. Every style used by the TUI derives from these
/// fields — no hardcoded colors outside this struct.
#[derive(Clone, Debug)]
pub struct Theme {
    pub(super) background: Color,
    pub(super) panel: Color,
    pub(super) sidebar_bg: Color,
    pub(super) footer_bg: Color,
    pub(super) border: Color,
    pub(super) active_border: Color,
    pub(super) text: Color,
    pub(super) muted: Color,
    pub(super) user: Color,
    pub(super) assistant: Color,
    /// Body-text color for assistant messages (streaming preview and
    /// rendered markdown share it, so finishing a stream does not shift the
    /// text color). Distinct from the brighter `assistant` header color.
    pub(super) assistant_text: Color,
    pub(super) success: Color,
    pub(super) warning: Color,
    pub(super) thinking: Color,
    pub(super) tool: Color,
    pub(super) dialog: Color,
    pub(super) dialog_selected: Color,
    pub(super) overlay: Color,
    pub(super) message_bg: Color,
    pub(super) heading: Color,
    pub(super) code: Color,
    pub(super) link: Color,
    pub(super) blockquote: Color,
    pub(super) list_marker: Color,
    pub(super) diff_delete: Color,
    pub(super) selection: Color,
    pub(super) selection_bg: Color,
    /// syntect theme name for code-block highlighting (e.g. "base16-ocean.dark").
    pub(super) syntect_theme: String,
}

impl Theme {
    /// Hard fallback when the built-in dark JSON fails to parse.
    /// Must stay in sync with `themes/dark.json`.
    pub fn dark() -> Self {
        Self {
            background: Color::Rgb(0, 0, 0),
            panel: Color::Rgb(22, 22, 26),
            sidebar_bg: Color::Rgb(13, 13, 16),
            footer_bg: Color::Rgb(10, 10, 13),
            border: Color::Rgb(42, 42, 48),
            active_border: Color::Rgb(85, 85, 95),
            text: Color::Rgb(215, 215, 220),
            muted: Color::Rgb(130, 130, 140),
            user: Color::Rgb(204, 174, 100),
            assistant: Color::Rgb(110, 185, 165),
            assistant_text: Color::Rgb(157, 201, 188),
            success: Color::Rgb(95, 170, 105),
            warning: Color::Rgb(205, 175, 85),
            thinking: Color::Rgb(155, 125, 200),
            tool: Color::Rgb(120, 155, 195),
            dialog: Color::Rgb(28, 28, 34),
            dialog_selected: Color::Rgb(160, 180, 205),
            overlay: Color::Rgb(0, 0, 0),
            message_bg: Color::Rgb(24, 24, 30),
            heading: Color::Rgb(157, 124, 216),
            code: Color::Rgb(127, 216, 143),
            link: Color::Rgb(250, 178, 131),
            blockquote: Color::Rgb(229, 192, 123),
            list_marker: Color::Rgb(86, 182, 194),
            diff_delete: Color::Rgb(200, 110, 110),
            selection: Color::Rgb(160, 180, 205),
            selection_bg: Color::Rgb(28, 28, 34),
            syntect_theme: "base16-ocean.dark".to_string(),
        }
    }

    pub fn panel_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }

    pub fn input_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.panel)
    }

    pub fn sidebar_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.sidebar_bg)
    }

    pub fn footer_style(&self) -> Style {
        Style::default().fg(self.muted).bg(self.footer_bg)
    }

    pub fn title_style(&self) -> Style {
        Style::default().fg(self.text).add_modifier(Modifier::BOLD)
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn input_border_style(&self, ai_running: bool) -> Style {
        Style::default().fg(if ai_running {
            self.warning
        } else {
            self.active_border
        })
    }

    pub fn brand_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn message_bg_style(&self) -> Style {
        Style::default().bg(self.message_bg)
    }

    pub fn user_style(&self) -> Style {
        Style::default().fg(self.user)
    }

    pub fn assistant_style(&self) -> Style {
        Style::default().fg(self.assistant)
    }

    pub fn assistant_text_style(&self) -> Style {
        Style::default().fg(self.assistant_text)
    }

    pub fn thinking_style(&self) -> Style {
        Style::default().fg(self.thinking)
    }

    pub fn tool_style(&self) -> Style {
        Style::default().fg(self.tool)
    }

    pub fn system_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn dialog_style(&self) -> Style {
        Style::default().fg(self.text).bg(self.dialog)
    }

    pub fn dialog_selected_style(&self) -> Style {
        Style::default().fg(self.dialog_selected).bg(self.dialog)
    }

    pub fn dialog_border_style(&self) -> Style {
        Style::default().fg(self.active_border).bg(self.dialog)
    }

    pub fn overlay_style(&self) -> Style {
        Style::default().bg(self.overlay)
    }

    pub fn running_style(&self, ai_running: bool) -> Style {
        Style::default().fg(if ai_running {
            self.warning
        } else {
            self.success
        })
    }

    pub fn diff_insert_style(&self) -> Style {
        Style::default().fg(self.success)
    }

    pub fn diff_delete_style(&self) -> Style {
        Style::default().fg(self.diff_delete)
    }

    /// Mouse text-selection highlight in the session panel.
    pub fn selection_style(&self) -> Style {
        Style::default()
            .fg(self.selection)
            .bg(self.selection_bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn heading_style(&self) -> Style {
        Style::default().fg(self.heading).add_modifier(Modifier::BOLD)
    }

    pub fn code_style(&self) -> Style {
        Style::default().fg(self.code)
    }

    pub(super) fn link_color(&self) -> Color {
        self.link
    }

    pub fn blockquote_style(&self) -> Style {
        Style::default().fg(self.blockquote)
    }

    pub fn list_marker_style(&self) -> Style {
        Style::default().fg(self.list_marker)
    }

    pub fn sidebar_dir_style(&self) -> Style {
        Style::default()
            .fg(self.active_border)
            .add_modifier(Modifier::BOLD)
    }

    pub fn sidebar_file_style(&self) -> Style {
        Style::default().fg(self.text)
    }

    /// syntect theme name for code-block highlighting.
    pub fn syntect_theme(&self) -> &str {
        &self.syntect_theme
    }
}

// ── Serializable form ────────────────────────────────────────────

/// JSON-serializable theme definition. Colors are hex strings (`"#RRGGBB"`);
/// empty fields inherit from the base theme during [`ThemeFile::merge`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub panel: String,
    #[serde(default)]
    pub sidebar_bg: String,
    #[serde(default)]
    pub footer_bg: String,
    #[serde(default)]
    pub border: String,
    #[serde(default)]
    pub active_border: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub muted: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub assistant: String,
    /// Assistant body-text color (empty = inherit `assistant`).
    #[serde(default)]
    pub assistant_text: String,
    #[serde(default)]
    pub success: String,
    #[serde(default)]
    pub warning: String,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub dialog: String,
    #[serde(default)]
    pub dialog_selected: String,
    #[serde(default)]
    pub overlay: String,
    #[serde(default)]
    pub message_bg: String,
    #[serde(default)]
    pub heading: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub blockquote: String,
    #[serde(default)]
    pub list_marker: String,
    #[serde(default)]
    pub diff_delete: String,
    #[serde(default)]
    pub selection: String,
    #[serde(default)]
    pub selection_bg: String,
    /// syntect theme name (empty = inherit base theme's).
    #[serde(default)]
    pub syntect_theme: String,
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn to_hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{:02X}{:02X}{:02X}", r, g, b),
        _ => "#000000".to_string(),
    }
}

impl ThemeFile {
    /// Merge `other` into self: non-empty fields from `other` overwrite.
    pub fn merge(&mut self, other: &ThemeFile) {
        macro_rules! merge_field {
            ($self:expr, $other:expr, $($field:ident),*) => {
                $(if !$other.$field.is_empty() { $self.$field = $other.$field.clone(); })*
            };
        }
        merge_field!(
            self, other,
            background, panel, sidebar_bg, footer_bg, border, active_border,
            text, muted, user, assistant, assistant_text, success, warning, thinking, tool,
            dialog, dialog_selected, overlay, message_bg,
            heading, code, link, blockquote, list_marker,
            diff_delete, selection, selection_bg, syntect_theme
        );
    }

    /// Convert to runtime `Theme`. Returns `None` if any color fails to parse.
    pub fn to_theme(&self) -> Option<Theme> {
        Some(Theme {
            background: parse_hex(&self.background)?,
            panel: parse_hex(&self.panel)?,
            sidebar_bg: parse_hex(&self.sidebar_bg)?,
            footer_bg: parse_hex(&self.footer_bg)?,
            border: parse_hex(&self.border)?,
            active_border: parse_hex(&self.active_border)?,
            text: parse_hex(&self.text)?,
            muted: parse_hex(&self.muted)?,
            user: parse_hex(&self.user)?,
            assistant: parse_hex(&self.assistant)?,
            assistant_text: if self.assistant_text.is_empty() {
                parse_hex(&self.assistant)?
            } else {
                parse_hex(&self.assistant_text)?
            },
            success: parse_hex(&self.success)?,
            warning: parse_hex(&self.warning)?,
            thinking: parse_hex(&self.thinking)?,
            tool: parse_hex(&self.tool)?,
            dialog: parse_hex(&self.dialog)?,
            dialog_selected: parse_hex(&self.dialog_selected)?,
            overlay: parse_hex(&self.overlay)?,
            message_bg: parse_hex(&self.message_bg)?,
            heading: parse_hex(&self.heading)?,
            code: parse_hex(&self.code)?,
            link: parse_hex(&self.link)?,
            blockquote: parse_hex(&self.blockquote)?,
            list_marker: parse_hex(&self.list_marker)?,
            diff_delete: parse_hex(&self.diff_delete)?,
            selection: parse_hex(&self.selection)?,
            selection_bg: parse_hex(&self.selection_bg)?,
            syntect_theme: if self.syntect_theme.is_empty() {
                "base16-ocean.dark".to_string()
            } else {
                self.syntect_theme.clone()
            },
        })
    }
}

impl From<&Theme> for ThemeFile {
    fn from(t: &Theme) -> Self {
        macro_rules! fields {
            ($t:expr, $($field:ident),*) => {
                ThemeFile {
                    name: String::new(),
                    syntect_theme: $t.syntect_theme.clone(),
                    $($field: to_hex($t.$field)),*
                }
            };
        }
        fields!(
            t,
            background, panel, sidebar_bg, footer_bg, border, active_border,
            text, muted, user, assistant, assistant_text, success, warning, thinking, tool,
            dialog, dialog_selected, overlay, message_bg,
            heading, code, link, blockquote, list_marker,
            diff_delete, selection, selection_bg
        )
    }
}

/// Built-in "dark" theme — the default.
pub fn builtin_dark() -> ThemeFile {
    serde_json::from_str(include_str!("themes/dark.json")).unwrap()
}

/// Built-in "light" theme.
pub fn builtin_light() -> ThemeFile {
    serde_json::from_str(include_str!("themes/light.json")).unwrap()
}

/// Built-in "hacker" theme — green phosphor on dark background.
pub fn builtin_hacker() -> ThemeFile {
    serde_json::from_str(include_str!("themes/hacker.json")).unwrap()
}

/// Look up a built-in theme by name ("dark", "light", or "hacker").
pub fn builtin(name: &str) -> Option<ThemeFile> {
    match name {
        "dark" => Some(builtin_dark()),
        "light" => Some(builtin_light()),
        "hacker" => Some(builtin_hacker()),
        _ => None,
    }
}

/// Resolve a theme from config JSON. Accepts:
/// - `"dark"` / `"light"` (built-in name)
/// - `{"name": "...", "background": "#000", ...}` (inline override)
///
/// Falls back to the built-in dark theme on any parsing error.
pub fn resolve(config_value: Option<&serde_json::Value>) -> Theme {
    let mut theme_file = builtin_dark();

    if let Some(value) = config_value {
        match value {
            serde_json::Value::String(name) => {
                if let Some(builtin) = builtin(name) {
                    theme_file = builtin;
                }
            }
            serde_json::Value::Object(_) => {
                if let Ok(overlay) = serde_json::from_value::<ThemeFile>(value.clone()) {
                    theme_file.merge(&overlay);
                }
            }
            _ => {}
        }
    }

    theme_file.to_theme().unwrap_or_else(Theme::dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_parses() {
        let t = builtin_dark().to_theme().unwrap();
        assert_eq!(t.background, Color::Rgb(0, 0, 0));
    }

    #[test]
    fn dark_fallback_synced_with_json() {
        let json = builtin_dark().to_theme().unwrap();
        let hard = Theme::dark();
        assert_eq!(json.background, hard.background);
        assert_eq!(json.diff_delete, hard.diff_delete);
        assert_eq!(json.selection, hard.selection);
        assert_eq!(json.selection_bg, hard.selection_bg);
        assert_eq!(json.syntect_theme, hard.syntect_theme);
    }

    #[test]
    fn builtin_themes_have_syntect_and_selection() {
        for name in ["dark", "light", "hacker"] {
            let t = builtin(name).unwrap().to_theme().unwrap();
            assert!(!t.syntect_theme.is_empty(), "{name} missing syntect_theme");
            let _ = t.selection_style();
            let _ = t.diff_delete_style();
        }
    }

    #[test]
    fn resolve_string_name() {
        let v = serde_json::json!("dark");
        let t = resolve(Some(&v));
        assert_eq!(t.background, Color::Rgb(0, 0, 0));
    }

    #[test]
    fn resolve_inline_override() {
        let v = serde_json::json!({"background": "#FF0000", "text": "#0000FF"});
        let t = resolve(Some(&v));
        assert_eq!(t.background, Color::Rgb(255, 0, 0));
        assert_eq!(t.text, Color::Rgb(0, 0, 255));
        // Unspecified fields keep dark defaults
        assert_eq!(t.panel, Color::Rgb(22, 22, 26));
    }

    #[test]
    fn inline_override_supports_new_fields() {
        let v = serde_json::json!({"diff_delete": "#FF0000", "syntect_theme": "solarized.light"});
        let t = resolve(Some(&v));
        assert_eq!(t.diff_delete, Color::Rgb(255, 0, 0));
        assert_eq!(t.syntect_theme(), "solarized.light");
    }
}
