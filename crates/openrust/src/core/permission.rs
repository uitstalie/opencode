//! Permission evaluation — project-scope enforcement with `$PROJECT` expansion.
//!
//! Two-tier bash permission model:
//! - **Dangerous** commands (dd, mkfs, shutdown, …) always require explicit
//!   `Ask` per invocation. No config override possible.
//! - **Normal** commands go through scope-based path evaluation. Future config
//!   rules will add allow/deny/ask for specific command patterns.
//!
//! **Known limitation**: The shell tokenizer does not handle command
//! substitution (`$(...)` or backticks), subshells, variable indirection
//! (`$CMD`), or glob expansion. Commands using these features may bypass
//! dangerous-command detection. This is a documented limitation of the
//! hand-rolled tokenizer approach.

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

/// Commands that are inherently destructive — ALWAYS require `Ask`, even in
/// interactive mode, even if a future config rule tries to allow them.
///
/// Covers POSIX/Linux, Windows CMD, and PowerShell. The `.exe` / `.com`
/// suffix is stripped before matching, so `"shutdown"` catches both
/// `shutdown` and `shutdown.exe`.
const DANGEROUS_COMMANDS: &[&str] = &[
    // ── POSIX / Linux ────────────────────────────────
    "shutdown", "reboot", "halt", "poweroff", "init", "telinit",
    "dd", "mkfs", "fdisk", "parted",
    "grub-install", "update-grub",
    "iptables", "ip6tables", "ufw", "firewall-cmd", "nft", "ebtables",
    "systemctl", "service", "rcctl", "sv",
    "useradd", "userdel", "usermod", "passwd", "chage",
    "visudo", "sudoedit",
    "cryptsetup", "lvremove", "vgremove", "pvremove",
    "wipefs", "blkid",

    // ── Windows CMD / external executables ───────────
    "diskpart", "reg", "regedt32", "regedit",
    "sc", "net", "netsh",
    "cipher", "sfc", "bcdedit",
    "wbadmin", "vssadmin",
    "shutdown", "format", // shared name with POSIX; harmless to list twice

    // ── PowerShell cmdlets ───────────────────────────
    "Stop-Computer", "Restart-Computer",
    "Set-ExecutionPolicy",
    "Format-Volume", "Clear-Disk", "Remove-Partition", "Initialize-Disk",
    "Disable-WSMan", "Set-Service", "Stop-Service",
    "Remove-Item", "Clear-Content", "Clear-Item",
    "Set-ItemProperty", // when targeting registry
];

/// Strip `.exe` / `.com` suffix from a command name (Windows external commands).
fn strip_exe_suffix(cmd: &str) -> &str {
    for ext in [".exe", ".com", ".bat", ".cmd", ".ps1"] {
        if let Some(stripped) = cmd.strip_suffix(ext) {
            return stripped;
        }
    }
    cmd
}

/// True if `token` is a pipe / sequence boundary that separates sub-commands.
fn is_segment_boundary(token: &str) -> bool {
    matches!(token, "|" | ";" | "&&" | "||")
}

/// Check if *any* sub-command in the pipeline is dangerous.
/// Handles `sudo`/env prefixes, `.exe` suffix stripping, and pipe/`;`/`&&`/`||`
/// segment boundaries so that `echo foo | shutdown` is caught.
fn is_dangerous_command(command: &str) -> bool {
    let tokens = tokenize_command(command);
    let mut i = 0;
    loop {
        // Skip sudo / env assignments at the start of each segment
        while i < tokens.len() && (tokens[i] == "sudo" || tokens[i].contains('=')) {
            i += 1;
        }
        if i >= tokens.len() {
            break;
        }
        let cmd = strip_exe_suffix(&tokens[i]);
        let cmd_lower = cmd.to_ascii_lowercase();
        // Exact match (case-insensitive)
        if DANGEROUS_COMMANDS.iter().any(|c| c.eq_ignore_ascii_case(cmd)) {
            return true;
        }
        // mkfs.* family (mkfs.ext4, mkfs.ntfs, mkfs.vfat, …)
        if cmd_lower.starts_with("mkfs.") {
            return true;
        }
        // Advance past remaining tokens in this segment
        i += 1;
        while i < tokens.len() && !is_segment_boundary(&tokens[i]) {
            i += 1;
        }
        // Skip the boundary token itself
        if i < tokens.len() {
            i += 1;
        }
    }
    false
}

/// Evaluate a bash command for permission.
///
/// Flow:
/// 1. Dangerous command → always `Ask` (interactive) / `Deny` (non-interactive)
/// 2. Extracted path hits a protected system path → always `Ask` / `Deny`
/// 3. Extracted path out of project scope → `Ask` / `Deny`
/// 4. Everything else → `Allow`
pub fn evaluate_bash(command: &str, project_root: &Path, interactive: bool) -> Decision {
    if is_dangerous_command(command) {
        return if interactive {
            Decision::Ask(format!("dangerous command: {}", command))
        } else {
            Decision::Deny(format!(
                "dangerous command denied (non-interactive): {}",
                command
            ))
        };
    }
    // Check extracted file paths
    for path in extract_command_paths(command) {
        // Protected path → always Ask (cannot be overridden by future config)
        let expanded = expand_project(&path, project_root);
        if crate::core::paths::check_protected(Path::new(&expanded)).is_err() {
            return if interactive {
                Decision::Ask(format!("protected path: {} ({})", path, command))
            } else {
                Decision::Deny(format!("protected path: {} ({})", path, command))
            };
        }
        // Normal scope check
        match evaluate("bash", &path, project_root, interactive) {
            Decision::Allow => {}
            other => return other,
        }
    }
    Decision::Allow
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
        if let Some(s) = params[key].as_str()
            && !s.is_empty() {
                return vec![s.to_string()];
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
        if (tokens[i] == ">" || tokens[i] == ">>")
            && i + 1 < tokens.len() {
                paths.push(tokens[i + 1].clone());
                i += 2;
                continue;
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

    // ── dangerous command detection ──────────────────

    #[test]
    fn dangerous_dd_always_asks() {
        assert!(matches!(
            evaluate_bash("dd if=/dev/zero of=/dev/sda", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn dangerous_dd_denies_non_interactive() {
        assert!(matches!(
            evaluate_bash("dd if=/dev/zero of=/dev/sda", &root(), false),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn dangerous_shutdown_asks() {
        assert!(matches!(
            evaluate_bash("shutdown -h now", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn dangerous_mkfs_family_asks() {
        assert!(matches!(
            evaluate_bash("mkfs.ext4 /dev/sda1", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn dangerous_sudo_prefix_still_dangerous() {
        assert!(matches!(
            evaluate_bash("sudo reboot", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn dangerous_iptables_asks() {
        assert!(matches!(
            evaluate_bash("iptables -F", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn normal_command_allowed_in_project() {
        // cargo build — no dangerous command, no paths → Allow
        assert_eq!(
            evaluate_bash("cargo build --release", &root(), true),
            Decision::Allow
        );
    }

    #[test]
    fn normal_rm_in_project_allowed() {
        // rm of a project file — not dangerous, path in project → Allow
        assert_eq!(
            evaluate_bash("rm target/debug/test", &root(), true),
            Decision::Allow
        );
    }

    #[test]
    fn normal_rm_outside_project_asks() {
        assert!(matches!(
            evaluate_bash("rm /tmp/external_file", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn is_dangerous_detects_systemctl() {
        assert!(is_dangerous_command("systemctl stop nginx"));
    }

    #[test]
    fn is_dangerous_not_triggered_by_normal_commands() {
        assert!(!is_dangerous_command("git commit -m msg"));
        assert!(!is_dangerous_command("cargo test"));
        assert!(!is_dangerous_command("rm -rf target/"));
    }

    // ── cross-shell dangerous command detection ──────

    #[test]
    fn dangerous_windows_diskpart() {
        assert!(is_dangerous_command("diskpart"));
        assert!(is_dangerous_command("diskpart.exe"));
    }

    #[test]
    fn dangerous_windows_reg() {
        assert!(is_dangerous_command("reg add HKLM\\Software\\Foo"));
        assert!(is_dangerous_command("reg.exe query HKLM\\Software"));
    }

    #[test]
    fn dangerous_windows_netsh() {
        assert!(is_dangerous_command("netsh firewall set opmode disable"));
    }

    #[test]
    fn dangerous_powershell_stop_computer() {
        assert!(is_dangerous_command("Stop-Computer -Force"));
    }

    #[test]
    fn dangerous_powershell_format_volume() {
        assert!(is_dangerous_command("Format-Volume -DriveLetter D -FileSystem NTFS"));
    }

    #[test]
    fn dangerous_powershell_set_execution_policy() {
        assert!(is_dangerous_command("Set-ExecutionPolicy Unrestricted"));
    }

    #[test]
    fn dangerous_via_pipe() {
        // `echo foo | shutdown` — shutdown is in a piped segment
        assert!(is_dangerous_command("echo foo | shutdown -h now"));
    }

    #[test]
    fn dangerous_via_semicolon() {
        assert!(is_dangerous_command("echo ok; reboot"));
    }

    #[test]
    fn dangerous_via_and_and() {
        assert!(is_dangerous_command("cd /tmp && mkfs.ext4 /dev/sda1"));
    }

    #[test]
    fn dangerous_exe_suffix_stripped() {
        assert!(is_dangerous_command("shutdown.exe /s /t 0"));
        assert!(is_dangerous_command("format.com D: /fs:NTFS"));
    }

    #[test]
    fn normal_powershell_not_dangerous() {
        assert!(!is_dangerous_command("Get-ChildItem -Path src"));
        assert!(!is_dangerous_command("Write-Host hello"));
    }

    // ── dangerous path detection in bash ─────────────

    #[test]
    fn bash_protected_path_always_asks() {
        // rm of a protected system path → Ask even though rm is not a "dangerous command"
        assert!(matches!(
            evaluate_bash("rm -rf /etc", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn bash_protected_path_denies_non_interactive() {
        assert!(matches!(
            evaluate_bash("rm -rf /etc", &root(), false),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn bash_write_to_protected_path_asks() {
        assert!(matches!(
            evaluate_bash("echo evil > /etc/passwd", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn bash_protected_dotgit_asks() {
        assert!(matches!(
            evaluate_bash("rm -rf .git", &root(), true),
            Decision::Ask(_)
        ));
    }

    #[test]
    fn normal_rm_in_scope_still_allowed() {
        // After adding protected path checks, normal in-project rm should still work
        assert_eq!(
            evaluate_bash("rm target/debug/test", &root(), true),
            Decision::Allow
        );
    }
}
