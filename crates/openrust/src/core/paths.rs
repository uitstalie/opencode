//! System path utilities — protected paths, scope checks, and cross-platform
//! separator helpers.

use std::path::{Path, PathBuf};

// ── Separator utilities ────────────────────────────

pub const SEPARATORS: &[char] = &['/', '\\'];

/// Trim leading `/` or `\` separators.
pub fn trim_leading(s: &str) -> &str {
    s.trim_start_matches(SEPARATORS)
}

/// Trim trailing `/` or `\` separators.
pub fn trim_trailing(s: &str) -> &str {
    s.trim_end_matches(SEPARATORS)
}

/// True when `rel` exactly equals `component` or starts with `component/` or `component\`.
pub fn has_component(rel: &str, component: &str) -> bool {
    rel.strip_prefix(component)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(SEPARATORS))
}

// ── Protected system paths (Linux) ──────────────────

/// Protected prefixes that should never be deleted.
pub const SYSTEM_PROTECTED: &[&str] = &[
    "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc", "/root",
    "/run", "/sbin", "/srv", "/sys", "/usr", "/var",
];

/// Protected user-level directories (~/.config, etc.)
pub const USER_PROTECTED: &[&str] = &[".config", ".local", ".ssh", ".gnupg", ".cache"];

/// Protected project-level dotdirs (use git/other tools to manage)
pub const PROJECT_PROTECTED: &[&str] = &[".git", ".svn", ".hg"];

// ── Home directory ──────────────────────────────────

pub fn home_dir() -> Option<PathBuf> {
    crate::core::platform::home_dir()
}

// ── Protection check ────────────────────────────────

/// Check if a canonical path is protected. Returns the reason if it is.
pub fn is_protected_path(canonical: &Path) -> Option<&'static str> {
    let raw = canonical.to_string_lossy();
    let s = raw.as_ref();

    // Windows: strip \\?\ prefix from canonical paths
    let s = s.strip_prefix("\\\\?\\").unwrap_or(s);

    // Exact match against system prefixes
    for prefix in SYSTEM_PROTECTED {
        if s == *prefix {
            return Some("protected system path");
        }
    }

    // Windows: drive root (e.g. C:\) or Windows system directory
    #[cfg(windows)]
    {
        if is_drive_root(s) || s.eq_ignore_ascii_case("C:\\Windows") {
            return Some("protected system path");
        }
    }

    // Home-level protected dirs
    if let Some(home) = home_dir() {
        let home_str = home.to_string_lossy();
        if s.starts_with(home_str.as_ref()) {
            let rel = trim_leading(&s[home_str.len()..]);
            for dir in USER_PROTECTED {
                if rel == *dir || has_component(rel, dir) {
                    return Some("protected user directory");
                }
            }
            for dir in PROJECT_PROTECTED {
                if rel == *dir {
                    return Some("protected project directory (use git to manage)");
                }
            }
        }
    }

    None
}

#[cfg(windows)]
fn is_drive_root(s: &str) -> bool {
    s.len() == 3
        && s.as_bytes()[1] == b':'
        && s.as_bytes()[2] == b'\\'
        && s.as_bytes()[0].is_ascii_alphabetic()
}

// ── Project scope ───────────────────────────────────

/// Check whether a path is within the project root.
pub fn is_within_project(target: &str, project_root: &Path) -> bool {
    let path = Path::new(target);
    let Ok(proot_canon) = project_root.canonicalize() else {
        return false;
    };

    // Existing path — canonicalize and compare directly.
    if let Ok(target_canon) = path.canonicalize() {
        return target_canon.starts_with(&proot_canon) && target_canon != proot_canon;
    }

    // Non-existent path (new file) — canonicalize the parent and check membership.
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(parent_canon) = parent.canonicalize() else {
        return false;
    };
    parent_canon.starts_with(&proot_canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_protected() {
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(is_protected_path(Path::new(root)).is_some());
    }

    #[test]
    fn etc_is_protected() {
        if cfg!(windows) {
            assert!(is_protected_path(Path::new("C:\\Windows")).is_some());
        } else {
            assert!(is_protected_path(Path::new("/etc")).is_some());
        }
    }

    #[test]
    fn home_ssh_is_protected() {
        if let Some(home) = home_dir() {
            let ssh = home.join(".ssh");
            assert!(is_protected_path(&ssh).is_some());
        }
    }

    #[test]
    fn detect_platform_paths_have_required_dirs() {
        let paths = crate::core::platform::PlatformPaths::detect();
        assert!(paths.global_config_path().ends_with("config.json"));
        assert!(paths.credentials_path().ends_with("credentials.enc"));
        assert!(paths.sessions_db_path().ends_with("sessions.db"));
    }

    #[test]
    fn tmp_is_not_protected() {
        assert!(is_protected_path(Path::new("/tmp")).is_none());
    }

    #[test]
    fn tmp_subdir_not_protected() {
        assert!(is_protected_path(Path::new("/tmp/test")).is_none());
    }

    #[test]
    fn project_subdir_is_in_scope() {
        let cwd = std::env::current_dir().unwrap();
        let child = cwd.join("src");
        assert!(is_within_project(&child.to_string_lossy(), &cwd));
    }

    #[test]
    fn outside_is_not_in_scope() {
        let cwd = std::env::current_dir().unwrap();
        assert!(!is_within_project("/tmp", &cwd));
    }
}
