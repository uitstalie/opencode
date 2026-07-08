//! Permission evaluation — project-scope enforcement with `$PROJECT` expansion.
//!
//! Config-driven allow/deny rules are not yet part of the schema, so this module
//! centralizes the project-boundary policy: tools that can write, delete, or
//! execute outside the project must stay within the project root unless the user
//! confirms interactively.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
    Ask(String),
}

/// Tools that can mutate the filesystem or run commands outside the project.
pub fn is_scope_restricted(tool: &str) -> bool {
    matches!(tool, "rm" | "write" | "edit" | "bash" | "apply_patch" | "undo_edit")
}

/// Expand a leading `$PROJECT` / `${PROJECT}` token to the project root.
pub fn expand_project(path: &str, project_root: &Path) -> String {
    let root = project_root.to_string_lossy();
    if let Some(rest) = path.strip_prefix("${PROJECT}") {
        return format!("{}{}", root, rest);
    }
    if let Some(rest) = path.strip_prefix("$PROJECT") {
        return format!("{}{}", root, rest);
    }
    path.to_string()
}

/// The scope-relevant target paths from raw tool params.
/// Returns multiple paths for multi-file tools (apply_patch, bash).
pub fn target_paths(params: &serde_json::Value) -> Vec<String> {
    // Single-path tools: target / filePath / path / workdir
    for key in &["target", "filePath", "path", "workdir"] {
        if let Some(s) = params[key].as_str() {
            if !s.is_empty() {
                return vec![s.to_string()];
            }
        }
    }
    // bash: extract file paths from the command content
    if let Some(cmd) = params["command"].as_str() {
        let paths = extract_command_paths(cmd);
        if !paths.is_empty() {
            return paths;
        }
        // No paths found in command — fall through to empty (Allow).
        // workdir was already checked above if present.
        return Vec::new();
    }
    // apply_patch stores all paths inside patchText — extract every one.
    if let Some(patch) = params["patchText"].as_str() {
        let mut paths = Vec::new();
        for line in patch.lines() {
            let trimmed = line.trim();
            for marker in &["*** Add File: ", "*** Update File: ", "*** Delete File: "] {
                if let Some(rest) = trimmed.strip_prefix(marker) {
                    let p = rest.trim().to_string();
                    if !p.is_empty() {
                        paths.push(p);
                    }
                }
            }
        }
        return paths;
    }
    Vec::new()
}

/// Commands that modify the filesystem — their non-flag arguments are treated
/// as file paths for scope evaluation.
const PATH_COMMANDS: &[&str] = &[
    // POSIX
    "rm", "rmdir", "mv", "cp", "mkdir", "touch", "ln", "chmod", "chown", "chattr",
    "dd", "tee", "truncate", "shred",
    // Windows / PowerShell
    "del", "erase", "rd", "md", "ren", "rename", "copy", "move", "attrib",
    "icacls", "takeown",
    "Remove-Item", "Copy-Item", "Move-Item", "Set-Content", "New-Item",
    "Out-File", "Add-Content", "Clear-Content",
];

/// Extract file-system paths from a shell command string.
///
/// Handles:
/// - Redirects: `> file`, `>> file`
/// - Known destructive/write commands: non-flag args treated as paths
/// - `sudo` prefix is stripped
///
/// Returns paths that should be checked against project scope.
fn extract_command_paths(command: &str) -> Vec<String> {
    let tokens = tokenize_command(command);
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut paths = Vec::new();
    let mut i = 0;

    // Strip leading sudo / env assignments
    while i < tokens.len() && (tokens[i] == "sudo" || tokens[i].contains('=')) {
        i += 1;
    }

    // Walk tokens looking for: (a) redirect operators, (b) known path commands
    while i < tokens.len() {
        // Redirect: > or >> followed by a filename
        if tokens[i] == ">" || tokens[i] == ">>" {
            if i + 1 < tokens.len() {
                paths.push(tokens[i + 1].clone());
                i += 2;
                continue;
            }
        }
        // Redirect appended to previous token: echo>file
        if let Some(rest) = tokens[i].strip_prefix(">").or_else(|| tokens[i].strip_prefix(">>")) {
            if !rest.is_empty() {
                paths.push(rest.trim_start().to_string());
            } else if i + 1 < tokens.len() {
                paths.push(tokens[i + 1].clone());
                i += 1;
            }
        }

        // Check if this token is a known path command
        let cmd_lower = tokens[i].to_ascii_lowercase();
        let is_path_cmd = PATH_COMMANDS.iter().any(|c| {
            *c == tokens[i] || c.to_ascii_lowercase() == cmd_lower
        });

        if is_path_cmd {
            // Collect non-flag arguments after the command
            let mut j = i + 1;
            while j < tokens.len() {
                if tokens[j].starts_with('-') || tokens[j] == ">" || tokens[j] == ">>" {
                    // Flag or redirect — skip (redirect handled above)
                    if tokens[j] == ">" || tokens[j] == ">>" {
                        j += 2; // skip redirect + target
                        continue;
                    }
                    j += 1;
                    continue;
                }
                // Check if next token is itself a command (pipe boundary)
                if tokens[j] == "|" || tokens[j] == ";" || tokens[j] == "&&" || tokens[j] == "||" {
                    break;
                }
                // This looks like a path argument
                paths.push(tokens[j].clone());
                j += 1;
            }
            i = j;
            continue;
        }

        i += 1;
    }

    paths
}

/// Split a command string into tokens, handling single/double quotes.
fn tokenize_command(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in command.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            // Split on pipe/semicolon operators (but not inside quotes)
            '|' | ';' if !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                // Handle && and ||
                tokens.push(ch.to_string());
            }
            '&' if !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                tokens.push(ch.to_string());
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    // Merge && and ||
    let mut merged = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if i + 1 < tokens.len() && tokens[i] == "&" && tokens[i + 1] == "&" {
            merged.push("&&".to_string());
            i += 2;
        } else if i + 1 < tokens.len() && tokens[i] == "|" && tokens[i + 1] == "|" {
            merged.push("||".to_string());
            i += 2;
        } else if tokens[i] == "|" && i + 1 < tokens.len() && tokens[i + 1] == "|" {
            // || in the original (not merged)
            merged.push("||".to_string());
            i += 2;
        } else {
            merged.push(tokens[i].clone());
            i += 1;
        }
    }
    merged
}

/// Evaluate a tool invocation against project scope.
pub fn evaluate(tool: &str, target: &str, project_root: &Path, interactive: bool) -> Decision {
    if !is_scope_restricted(tool) {
        return Decision::Allow;
    }
    let target = expand_project(target, project_root);
    if target.is_empty() || crate::core::paths::is_within_project(&target, project_root) {
        return Decision::Allow;
    }
    if interactive {
        Decision::Ask(format!(
            "{} operates outside project scope: {}",
            tool, target
        ))
    } else {
        Decision::Deny(format!(
            "{}: '{}' is outside the project scope ({}).",
            tool,
            target,
            project_root.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        std::env::current_dir().unwrap()
    }

    #[test]
    fn non_restricted_tools_are_allowed() {
        assert_eq!(
            evaluate("read", "/etc/passwd", &root(), false),
            Decision::Allow
        );
    }

    #[test]
    fn in_project_paths_are_allowed() {
        assert_eq!(
            evaluate("write", "src/main.rs", &root(), true),
            Decision::Allow
        );
    }

    #[test]
    fn outside_project_denies_when_non_interactive() {
        assert!(matches!(
            evaluate("write", "/etc/hosts", &root(), false),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn outside_project_asks_when_interactive() {
        assert!(matches!(
            evaluate("bash", "/tmp/elsewhere", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn expands_project_token() {
        let expanded = expand_project("$PROJECT/src/main.rs", Path::new("/work/app"));
        assert_eq!(expanded, "/work/app/src/main.rs");
    }

    #[test]
    fn extract_rm_path() {
        let paths = extract_command_paths("rm -rf /tmp/foo");
        assert_eq!(paths, vec!["/tmp/foo"]);
    }

    #[test]
    fn extract_cp_paths() {
        let paths = extract_command_paths("cp src/main.rs /etc/cron.d/evil");
        assert_eq!(paths, vec!["src/main.rs", "/etc/cron.d/evil"]);
    }

    #[test]
    fn extract_redirect_path() {
        let paths = extract_command_paths("echo hello > /etc/passwd");
        assert_eq!(paths, vec!["/etc/passwd"]);
    }

    #[test]
    fn extract_redirect_append() {
        let paths = extract_command_paths("echo data >> /var/log/evil");
        assert_eq!(paths, vec!["/var/log/evil"]);
    }

    #[test]
    fn extract_pipe_command_paths() {
        let paths = extract_command_paths("cat foo | tee /etc/cron.d/x");
        assert_eq!(paths, vec!["/etc/cron.d/x"]);
    }

    #[test]
    fn extract_sudo_prefix() {
        let paths = extract_command_paths("sudo rm /etc/important");
        assert_eq!(paths, vec!["/etc/important"]);
    }

    #[test]
    fn extract_no_paths_for_safe_command() {
        let paths = extract_command_paths("cargo build --release");
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_no_paths_for_echo() {
        let paths = extract_command_paths("echo hello world");
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_quoted_path() {
        let paths = extract_command_paths("rm '/tmp/file with spaces'");
        assert_eq!(paths, vec!["/tmp/file with spaces"]);
    }

    #[test]
    fn extract_mv_paths() {
        let paths = extract_command_paths("mv old.txt /etc/new.txt");
        assert_eq!(paths, vec!["old.txt", "/etc/new.txt"]);
    }

    #[test]
    fn target_paths_bash_extracts_command() {
        let params = serde_json::json!({"command": "rm /etc/passwd"});
        let paths = target_paths(&params);
        assert_eq!(paths, vec!["/etc/passwd"]);
    }

    #[test]
    fn target_paths_bash_no_paths_returns_empty() {
        let params = serde_json::json!({"command": "cargo test"});
        let paths = target_paths(&params);
        assert!(paths.is_empty());
    }
}
