pub mod agent;
pub mod config;
pub mod e2e;
pub mod permission;
pub mod prompt;
pub mod provider;
pub mod session;
pub mod task;
pub mod tool;
pub mod tui;
pub mod vault;

/// Mask a secret key for display, showing only the last 4 characters.
/// Safe for UTF-8 strings.
pub fn mask_secret(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() > 8 {
        let suffix: String = chars[chars.len() - 4..].iter().collect();
        format!("****{}", suffix)
    } else {
        "****".to_string()
    }
}
