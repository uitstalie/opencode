//! File-tree sidebar with live refresh via `notify`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{RecursiveMode, Watcher};
use ratatui::text::{Line, Span};

use super::render::Theme;

const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 500;
const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".openrust",
    "dist",
    ".cache",
];

fn is_in_skip_dir(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            if let Some(name_str) = name.to_str() {
                if SKIP_DIRS.contains(&name_str) {
                    return true;
                }
            }
        }
    }
    false
}

pub struct FileTree {
    root: PathBuf,
    entries: Vec<TreeEntry>,
    events: Option<Receiver<PathBuf>>,
    // Held to keep the watcher alive; dropping it stops notifications.
    _watcher: Option<notify::RecommendedWatcher>,
}

struct TreeEntry {
    depth: usize,
    name: String,
    is_dir: bool,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let (events, watcher) = {
            let (tx, rx) = channel::<PathBuf>();
            let watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    for path in event.paths {
                        let _ = tx.send(path);
                    }
                }
            })
            .ok()
            .and_then(|mut watcher| {
                watcher.watch(&root, RecursiveMode::Recursive).ok()?;
                Some(watcher)
            });
            (watcher.is_some().then_some(rx), watcher)
        };
        let mut tree = Self {
            root,
            entries: Vec::new(),
            events,
            _watcher: watcher,
        };
        tree.rescan();
        tree
    }

    /// Drain filesystem events; returns true if a rescan was performed.
    pub fn poll_refresh(&mut self) -> bool {
        let Some(events) = &self.events else {
            return false;
        };
        let mut dirty = false;
        while let Ok(path) = events.try_recv() {
            if !is_in_skip_dir(&path) {
                dirty = true;
            }
        }
        if dirty {
            self.rescan();
        }
        dirty
    }

    pub fn rescan(&mut self) {
        self.entries = scan(&self.root);
    }

    pub fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        if self.entries.is_empty() {
            return vec![Line::from(Span::styled("(empty)", theme.muted_style()))];
        }
        self.entries
            .iter()
            .map(|entry| {
                let indent = "  ".repeat(entry.depth);
                if entry.is_dir {
                    Line::from(Span::styled(
                        format!("{}{}/", indent, entry.name),
                        theme.sidebar_dir_style(),
                    ))
                } else {
                    Line::from(Span::styled(
                        format!("{}{}", indent, entry.name),
                        theme.sidebar_file_style(),
                    ))
                }
            })
            .collect()
    }
}

/// Recursively collect a depth-limited, sorted file tree, skipping heavy dirs.
fn scan(root: &Path) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    walk(root, 0, &mut entries);
    entries
}

fn walk(dir: &Path, depth: usize, entries: &mut Vec<TreeEntry>) {
    if depth >= MAX_DEPTH || entries.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<(String, bool, PathBuf)> = read
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') && depth > 0 {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                return None;
            }
            Some((name, is_dir, entry.path()))
        })
        .collect();
    // Directories first, then files, each alphabetically.
    children.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    for (name, is_dir, path) in children {
        if entries.len() >= MAX_ENTRIES {
            return;
        }
        entries.push(TreeEntry {
            depth,
            name,
            is_dir,
        });
        if is_dir {
            walk(&path, depth + 1, entries);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_files_and_dirs_sorted() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("openrust-tree-{}", now));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("README.md"), "hi").unwrap();
        std::fs::write(root.join("src").join("main.rs"), "fn main() {}").unwrap();

        let entries = scan(&root);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"src"));
        assert!(names.contains(&"README.md"));
        assert!(names.contains(&"main.rs"));
        // Directory sorts before file at the same level.
        let src_pos = names.iter().position(|n| *n == "src").unwrap();
        let readme_pos = names.iter().position(|n| *n == "README.md").unwrap();
        assert!(src_pos < readme_pos);
    }

    #[test]
    fn skips_heavy_directories() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("openrust-tree-skip-{}", now));
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target").join("junk"), "x").unwrap();
        std::fs::write(root.join("keep.txt"), "x").unwrap();

        let entries = scan(&root);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"keep.txt"));
        assert!(!names.contains(&"target"));
    }
}
