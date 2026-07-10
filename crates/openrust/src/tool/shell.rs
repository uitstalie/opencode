//! Shell selection and tool prompt text.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Bash,
    Pwsh,
    Powershell,
    Cmd,
}

pub fn detect_shell_kind() -> ShellKind {
    if cfg!(windows) {
        return detect_windows_shell();
    }

    ShellKind::Bash
}

pub fn shell_command(kind: ShellKind) -> (&'static str, &'static [&'static str]) {
    match kind {
        ShellKind::Bash => ("bash", &["-lc"]),
        ShellKind::Pwsh => ("pwsh", &["-NoLogo", "-NoProfile", "-Command"]),
        ShellKind::Powershell => ("powershell", &["-NoLogo", "-NoProfile", "-Command"]),
        ShellKind::Cmd => ("cmd", &["/C"]),
    }
}

pub fn shell_tool_prompt(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::Bash => {
            r#"Use `bash` on Linux/macOS. Prefer concise shell commands.
- Use `bash -lc` style command execution.
- Prefer `rg`, `fd`, `sed`, `awk`, `git`, and standard Unix tools.
- Combine short sequences into one command when it improves clarity.
- Keep commands safe and scoped to the requested task."#
        }
        ShellKind::Pwsh => {
            r#"Use `pwsh` on modern Windows (PowerShell Core).
- Prefer PowerShell Core cmdlets and syntax.
- Use `Get-ChildItem`, `Select-String`, `Set-Content`, `Get-Content`, `Test-Path`.
- Prefer `pwsh -NoLogo -NoProfile -Command` execution.
- Keep commands explicit; avoid shell-specific assumptions from Unix."#
        }
        ShellKind::Powershell => {
            r#"Use `powershell` (Windows PowerShell 5.1).
- Prefer Windows PowerShell cmdlets and syntax.
- Use `Get-ChildItem`, `Select-String`, `Set-Content`, `Get-Content`, `Test-Path`.
- Prefer `powershell -NoLogo -NoProfile -Command` execution.
- Avoid Unix-only tools unless they are installed and explicitly available."#
        }
        ShellKind::Cmd => {
            r#"Use `cmd` only when Windows shell compatibility is required.
- Prefer `dir`, `type`, `copy`, `move`, `del`, `where`, `for` when needed.
- Prefer `cmd /C` execution.
- Avoid relying on PowerShell-only features or Unix tools."#
        }
    }
}

pub fn shell_environment_hint(kind: ShellKind) -> &'static str {
    match kind {
        ShellKind::Bash => "bash",
        ShellKind::Pwsh => "pwsh",
        ShellKind::Powershell => "powershell",
        ShellKind::Cmd => "cmd",
    }
}

fn detect_windows_shell() -> ShellKind {
    if exists_in_path("pwsh") {
        return ShellKind::Pwsh;
    }

    if exists_in_path("powershell") {
        return ShellKind::Powershell;
    }

    ShellKind::Cmd
}

fn exists_in_path(command: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    let candidates: Vec<PathBuf> = if cfg!(windows) {
        vec![PathBuf::from(command), PathBuf::from(format!("{}.exe", command))]
    } else {
        vec![PathBuf::from(command)]
    };

    std::env::split_paths(&paths).any(|dir| {
        candidates.iter().any(|c| dir.join(c).exists())
    })
}

pub fn shell_binary_path(kind: ShellKind) -> PathBuf {
    let (bin, _) = shell_command(kind);
    PathBuf::from(bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_are_non_empty() {
        assert!(!shell_tool_prompt(ShellKind::Bash).is_empty());
        assert!(!shell_tool_prompt(ShellKind::Pwsh).is_empty());
        assert!(!shell_tool_prompt(ShellKind::Powershell).is_empty());
        assert!(!shell_tool_prompt(ShellKind::Cmd).is_empty());
    }

    #[test]
    fn shell_commands_match_kinds() {
        assert_eq!(shell_command(ShellKind::Bash).0, "bash");
        assert_eq!(shell_command(ShellKind::Pwsh).0, "pwsh");
        assert_eq!(shell_command(ShellKind::Powershell).0, "powershell");
        assert_eq!(shell_command(ShellKind::Cmd).0, "cmd");
    }

    #[test]
    fn shell_environment_hint_matches_kind() {
        assert_eq!(shell_environment_hint(ShellKind::Bash), "bash");
        assert_eq!(shell_environment_hint(ShellKind::Pwsh), "pwsh");
        assert_eq!(shell_environment_hint(ShellKind::Powershell), "powershell");
        assert_eq!(shell_environment_hint(ShellKind::Cmd), "cmd");
    }
}
