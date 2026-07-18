//! Memory store — pure `.md` file storage with process-wide write serialization.
//!
//! Each memory entry is one line in a `.md` file:
//! ```text
//! - [2026-07-11] some conclusion #tag1 #tag2
//! ```
//!
//! Files are organised by scope:
//! - `project`  → `<cwd>/.openrust/memory/{category}.md`
//! - `user`     → `~/.config/openrust/memory/{category}.md`
//! - `dreaming` → `~/.config/openrust/memory/dreaming/{hash}.md`

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Serialises all memory writes within the process.  Held only during the
/// sync read-check-write cycle (no `.await` crosses the guard).
static WRITE_LOCK: Mutex<()> = Mutex::new(());

// ── Types ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    User,
    Dreaming,
}

impl Scope {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "project" => Some(Self::Project),
            "user" => Some(Self::User),
            "dreaming" => Some(Self::Dreaming),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Dreaming => "dreaming",
        }
    }
}

/// Valid category names per scope.  Returns `&[]` for dreaming (no categories).
pub fn valid_categories(scope: Scope) -> &'static [&'static str] {
    match scope {
        Scope::Project => &["progress", "TODO", "tech", "conclusion"],
        Scope::User => &["preferences", "constraints", "patterns", "style"],
        Scope::Dreaming => &[],
    }
}

/// Check that `category` is valid for the given scope.
pub fn is_valid_category(scope: Scope, category: &str) -> bool {
    valid_categories(scope).contains(&category)
}

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub date: String,
    pub content: String,
    pub tags: Vec<String>,
}

// ── Path resolution ────────────────────────────────

/// Resolve the `.md` file path for a scope + optional category.
///
/// - Project:  `<cwd>/.openrust/memory/{category}.md`
/// - User:     `~/.config/openrust/memory/{category}.md`
/// - Dreaming: `~/.config/openrust/memory/dreaming/{sha256(cwd)[:12]}.md`
///
/// For project/user, `category` must be `Some`.  For dreaming it is ignored.
pub fn resolve_path(scope: Scope, category: Option<&str>, cwd: &Path) -> PathBuf {
    match scope {
        Scope::Project => {
            let cat = category.unwrap_or("tech");
            cwd.join(".openrust").join("memory").join(format!("{cat}.md"))
        }
        Scope::User => {
            let cat = category.unwrap_or("preferences");
            config_dir().join("memory").join(format!("{cat}.md"))
        }
        Scope::Dreaming => {
            let hash = dreaming_hash(cwd);
            config_dir().join("memory").join("dreaming").join(format!("{hash}.md"))
        }
    }
}

/// All `.md` files for a scope (for overview reads).
pub fn files_for_scope(scope: Scope, cwd: &Path) -> Vec<PathBuf> {
    match scope {
        Scope::Project => list_md_files(&cwd.join(".openrust").join("memory")),
        Scope::User => list_md_files(&config_dir().join("memory")),
        Scope::Dreaming => {
            let hash = dreaming_hash(cwd);
            vec![config_dir()
                .join("memory")
                .join("dreaming")
                .join(format!("{hash}.md"))]
        }
    }
}

fn list_md_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .map(|e| e.path())
        .collect()
}

fn config_dir() -> PathBuf {
    crate::core::platform::PlatformPaths::detect()
        .config_dir()
        .clone()
}

fn dreaming_hash(cwd: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(cwd.to_string_lossy().as_bytes());
    hex::encode(&hasher.finalize()[..6]) // 12 hex chars
}

// ── Parsing ────────────────────────────────────────

/// Parse a `.md` file into entries.  Returns empty vec if file is missing.
pub fn parse_file(path: &Path) -> Vec<MemoryEntry> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    parse_entries(&text)
}

/// Parse raw text into entries.  Each line: `- [YYYY-MM-DD] content #tag1 #tag2`
fn parse_entries(text: &str) -> Vec<MemoryEntry> {
    text.lines()
        .filter_map(parse_line)
        .collect()
}

fn parse_line(line: &str) -> Option<MemoryEntry> {
    let rest = line.trim().strip_prefix("- ")?;
    // Extract optional [date]
    let (date, rest) = if let Some(after_bracket) = rest.strip_prefix('[') {
        let close = after_bracket.find(']')?;
        let date = &after_bracket[..close];
        (date.to_string(), &after_bracket[close + 1..])
    } else {
        (String::new(), rest)
    };

    let rest = rest.trim_start();

    // Split tags (tokens starting with #)
    let mut content_parts = Vec::new();
    let mut tags = Vec::new();
    for token in rest.split_whitespace() {
        if let Some(tag) = token.strip_prefix('#') {
            if !tag.is_empty() {
                tags.push(tag.to_string());
            }
        } else {
            content_parts.push(token);
        }
    }

    Some(MemoryEntry {
        date,
        content: content_parts.join(" "),
        tags,
    })
}

/// Format an entry as a single line.
fn format_entry(content: &str, tags: &[String]) -> String {
    let tag_str = if tags.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            tags.iter()
                .map(|t| format!("#{t}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    // Skip the date prefix when content already carries one — callers
    // sometimes include it, and `[date] [date]` is never intended.
    if has_date_prefix(content) {
        return format!("- {content}{tag_str}");
    }
    let date = today();
    format!("- [{date}] {content}{tag_str}")
}

/// True when content starts with a `[YYYY-MM-DD]` date prefix.
fn has_date_prefix(content: &str) -> bool {
    let b = content.as_bytes();
    b.len() >= 12
        && b[0] == b'['
        && b[1..5].iter().all(|c| c.is_ascii_digit())
        && b[5] == b'-'
        && b[6..8].iter().all(|c| c.is_ascii_digit())
        && b[8] == b'-'
        && b[9..11].iter().all(|c| c.is_ascii_digit())
        && b[11] == b']'
}

// ── Write ──────────────────────────────────────────

/// Append a new entry to the target file with dedup check.
///
/// Returns `Ok(true)` if written, `Ok(false)` if an exact duplicate
/// (case-insensitive full content) already exists.
pub fn append_entry(
    path: &Path,
    content: &str,
    tags: &[String],
) -> std::io::Result<bool> {
    let _guard = WRITE_LOCK
        .lock()
        .expect("memory WRITE_LOCK poisoned");

    let normalized = content.trim().to_lowercase();
    if normalized.is_empty() {
        return Ok(false);
    }

    // Dedup: exact content match (case-insensitive)
    let existing = parse_file(path);
    if existing
        .iter()
        .any(|e| e.content.trim().to_lowercase() == normalized)
    {
        return Ok(false);
    }

    // Ensure parent dir exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let line = format_entry(&content.replace('\n', " "), tags);

    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&line);
    text.push('\n');

    // Atomic write: temp file + rename
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, path)?;

    Ok(true)
}

// ── Read ───────────────────────────────────────────

/// Read entries from a file, optionally filtering by search term.
pub fn read_entries(path: &Path, search: Option<&str>) -> Vec<MemoryEntry> {
    let mut entries = parse_file(path);
    if let Some(q) = search {
        let lower = q.to_lowercase();
        entries.retain(|e| e.content.to_lowercase().contains(&lower));
    }
    entries
}

/// Quick overview: scope → file → entry count.
pub fn overview(cwd: &Path) -> Vec<(Scope, String, usize)> {
    let mut out = Vec::new();
    for scope in [Scope::Project, Scope::User, Scope::Dreaming] {
        for path in files_for_scope(scope, cwd) {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let count = parse_file(&path).len();
            if count > 0 {
                out.push((scope, name, count));
            }
        }
    }
    out
}

// ── Date helper (no chrono) ────────────────────────

fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    epoch_to_ymd(secs)
}

/// Convert epoch seconds to `YYYY-MM-DD` (UTC).
fn epoch_to_ymd(epoch: u64) -> String {
    let days = (epoch / 86400) as i64;
    // 1970-01-01 = day 0
    let mut y = 1970i64;
    let mut remaining = days;

    loop {
        let dy = if is_leap(y) { 366 } else { 365 };
        if remaining < dy {
            break;
        }
        remaining -= dy;
        y += 1;
    }

    let month_days: [i64; 12] = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            m = i;
            break;
        }
        remaining -= md;
    }

    format!("{y:04}-{:02}-{:02}", m + 1, remaining + 1)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ── Tests ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_entry() {
        let entries = parse_entries("- [2026-07-11] hello world #tag1 #tag2");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].date, "2026-07-11");
        assert_eq!(entries[0].content, "hello world");
        assert_eq!(entries[0].tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn parse_no_tags() {
        let entries = parse_entries("- [2026-07-11] just text");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].tags.is_empty());
    }

    #[test]
    fn parse_no_date() {
        let entries = parse_entries("- some text without date");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].date.is_empty());
    }

    #[test]
    fn parse_skips_blank_and_non_entry_lines() {
        let entries = parse_entries(
            "# Title\n\n- [2026-07-11] real entry\nblank line\n  \n",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "real entry");
    }

    #[test]
    fn dedup_case_insensitive() {
        let dir = tempdir();
        let path = dir.join("test.md");

        assert!(append_entry(&path, "Hello World", &[]).unwrap());
        assert!(!append_entry(&path, "hello world", &[]).unwrap()); // dedup
        assert!(!append_entry(&path, "HELLO WORLD", &[]).unwrap()); // dedup
        assert!(append_entry(&path, "different content", &[]).unwrap());

        let entries = parse_file(&path);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn append_creates_parent_dirs() {
        let dir = tempdir();
        let path = dir.join("a").join("b").join("c.md");
        assert!(append_entry(&path, "test", &[]).unwrap());
        assert!(path.exists());
    }

    #[test]
    fn append_normalizes_newlines() {
        let dir = tempdir();
        let path = dir.join("test.md");
        assert!(append_entry(&path, "line1\nline2", &[]).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("line1 line2"));
        assert!(!text.contains("line1\nline2"));
    }

    #[test]
    fn read_with_search_filter() {
        let dir = tempdir();
        let path = dir.join("test.md");
        append_entry(&path, "rust rewrite", &[]).unwrap();
        append_entry(&path, "python project", &[]).unwrap();

        let all = read_entries(&path, None);
        assert_eq!(all.len(), 2);

        let filtered = read_entries(&path, Some("rust"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].content, "rust rewrite");
    }

    #[test]
    fn epoch_to_ymd_known_dates() {
        assert_eq!(epoch_to_ymd(0), "1970-01-01");
        assert_eq!(epoch_to_ymd(1_722_693_760), "2024-08-03");
        assert_eq!(epoch_to_ymd(1_782_624_000), "2026-06-28");
    }

    #[test]
    fn leap_year_detection() {
        assert!(is_leap(2000));
        assert!(is_leap(2024));
        assert!(!is_leap(1900));
        assert!(!is_leap(2023));
    }

    #[test]
    fn dreaming_hash_is_12_hex_chars() {
        let hash = dreaming_hash(Path::new("/home/user/project"));
        assert_eq!(hash.len(), 12);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "openrust-memory-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
