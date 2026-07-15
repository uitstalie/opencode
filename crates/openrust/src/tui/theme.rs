//! Theme configuration — serializable color definitions and built-in presets.
//!
//! `ThemeFile` is the JSON-serializable form; `Theme` (in [`super::render`]) is
//! the runtime form with `ratatui::style::Color` fields.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// JSON-serializable theme definition. All colors are hex strings (`"#RRGGBB"`).
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
        for pair in [
            ("background", &mut self.background, &other.background),
            ("panel", &mut self.panel, &other.panel),
        ] {
            if !pair.2.is_empty() {
                *pair.1 = pair.2.clone();
            }
        }
        macro_rules! merge_field {
            ($self:expr, $other:expr, $($field:ident),*) => {
                $(if !$other.$field.is_empty() { $self.$field = $other.$field.clone(); })*
            };
        }
        merge_field!(
            self, other,
            sidebar_bg, footer_bg, border, active_border, text, muted,
            user, assistant, success, warning, thinking, tool,
            dialog, dialog_selected, overlay, message_bg,
            heading, code, link, blockquote, list_marker
        );
    }

    /// Convert to runtime `Theme`. Returns `None` if any color fails to parse.
    pub fn to_theme(&self) -> Option<super::render::Theme> {
        Some(super::render::Theme::from_colors(
            parse_hex(&self.background)?,
            parse_hex(&self.panel)?,
            parse_hex(&self.sidebar_bg)?,
            parse_hex(&self.footer_bg)?,
            parse_hex(&self.border)?,
            parse_hex(&self.active_border)?,
            parse_hex(&self.text)?,
            parse_hex(&self.muted)?,
            parse_hex(&self.user)?,
            parse_hex(&self.assistant)?,
            parse_hex(&self.success)?,
            parse_hex(&self.warning)?,
            parse_hex(&self.thinking)?,
            parse_hex(&self.tool)?,
            parse_hex(&self.dialog)?,
            parse_hex(&self.dialog_selected)?,
            parse_hex(&self.overlay)?,
            parse_hex(&self.message_bg)?,
            parse_hex(&self.heading)?,
            parse_hex(&self.code)?,
            parse_hex(&self.link)?,
            parse_hex(&self.blockquote)?,
            parse_hex(&self.list_marker)?,
        ))
    }
}

impl From<&super::render::Theme> for ThemeFile {
    fn from(t: &super::render::Theme) -> Self {
        macro_rules! fields {
            ($t:expr, $($field:ident),*) => {
                ThemeFile {
                    name: String::new(),
                    $($field: to_hex($t.$field)),*
                }
            };
        }
        fields!(
            t,
            background, panel, sidebar_bg, footer_bg, border, active_border,
            text, muted, user, assistant, success, warning, thinking, tool,
            dialog, dialog_selected, overlay, message_bg,
            heading, code, link, blockquote, list_marker
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
pub fn resolve(config_value: Option<&serde_json::Value>) -> super::render::Theme {
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

    theme_file
        .to_theme()
        .unwrap_or_else(super::render::Theme::dark)
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
}
