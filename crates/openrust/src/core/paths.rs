//! System path utilities — protected paths, home directory, scope checks.
//!
//! Linux-focused for now; Windows support can be added later via cfg.

use std::path::{Path, PathBuf};

// ── Protected system paths (Linux) ──────────────────

/// Protected prefixes that should never be deleted.
pub const SYSTEM_PROTECTED: &[&str] = &[
    "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64",
    "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys", "/usr", "/var",
];

/// Protected user-level directories (~/.config, etc.)
pub const USER_PROTECTED: &[&str] = &[
    ".config", ".local", ".ssh", ".gnupg", ".cache",
];

/// Protected project-level dotdirs (use git/other tools to manage)
pub const PROJECT_PROTECTED: &[&str] = &[
    ".git", ".svn", ".hg",
];

// ── Home directory ──────────────────────────────────

pub fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

// ── Protection check ────────────────────────────────

/// Check if a canonical path is protected. Returns the reason if it is.
pub fn is_protected_path(canonical: &Path) -> Option<&'static str> {
    let s = canonical.to_string_lossy();

    // Exact match against system prefixes
    for prefix in SYSTEM_PROTECTED {
        if s.as_ref() == *prefix {
            return Some("protected system path");
        }
    }

    // Home-level protected dirs
    if let Some(home) = home_dir() {
        let home_str = home.to_string_lossy();
        if s.starts_with(home_str.as_ref()) {
            let rel = s[home_str.len()..].trim_start_matches('/');
            for dir in USER_PROTECTED {
                if rel == *dir || rel.starts_with(&format!("{}/", dir)) {
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

// ── Project scope ───────────────────────────────────

/// Check whether a path is within the project root.
pub fn is_within_project(target: &str, project_root: &Path) -> bool {
    let Ok(target_canon) = Path::new(target).canonicalize() else { return false };
    let Ok(proot_canon) = project_root.canonicalize() else { return false };

    target_canon.starts_with(&proot_canon) && target_canon != proot_canon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_protected() {
        assert!(is_protected_path(Path::new("/")).is_some());
    }

    #[test]
    fn etc_is_protected() {
        assert!(is_protected_path(Path::new("/etc")).is_some());
    }

    #[test]
    fn home_ssh_is_protected() {
        if let Some(home) = home_dir() {
            let ssh = home.join(".ssh");
            assert!(is_protected_path(&ssh).is_some());
        }
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
