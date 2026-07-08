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
/// Returns multiple paths for multi-file tools (apply_patch).
pub fn target_paths(params: &serde_json::Value) -> Vec<String> {
    for key in &["target", "filePath", "path", "workdir"] {
        if let Some(s) = params[key].as_str() {
            if !s.is_empty() {
                return vec![s.to_string()];
            }
        }
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
}
