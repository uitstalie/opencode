//! ApplyPatch tool — apply an patch-style format with add/update/delete hunks.
//!
//! Patch format:
//! ```text
//! *** Begin Patch
//! *** Add File: path
//! +new content line
//! *** Update File: path
//! @@ optional context
//!  unchanged line
//! -removed line
//! +added line
//! *** Delete File: path
//! *** End Patch
//! ```

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, resolve_path};
use serde_json::Value;

pub struct ApplyPatchTool;

#[async_trait::async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &'static str {
        "apply_patch"
    }
    fn description(&self) -> &'static str {
        "Apply one patch containing add, update, and delete file operations. Operations apply sequentially; if a later operation fails, earlier operations remain applied and the failure reports them. Relative paths resolve from cwd."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "patchText": { "type": "string", "description": "The full patch text describing add, update, and delete operations" }
            },
            "required": ["patchText"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let patch_text = p.opt_str("patchText").unwrap_or("");
        if patch_text.trim().is_empty() {
            return ToolResult::error("patchText is required");
        }

        let hunks = match parse_patch(patch_text) {
            Ok(hunks) => hunks,
            Err(err) => {
                return ToolResult::error(format!("apply_patch verification failed: {}", err));
            }
        };
        if hunks.is_empty() {
            return ToolResult::error("patch rejected: empty patch");
        }
        if hunks.iter().any(|hunk| {
            matches!(
                hunk,
                Hunk::Update {
                    move_path: Some(_),
                    ..
                }
            )
        }) {
            return ToolResult::error("apply_patch moves are not supported yet");
        }

        let mut applied: Vec<String> = Vec::new();
        for hunk in &hunks {
            let resolved = resolve_path(ctx, hunk_path(hunk));
            if let Err(reason) = crate::core::paths::check_protected(&resolved) {
                return ToolResult::error(format!(
                    "Refusing to modify {}: {}",
                    resolved.display(),
                    reason
                ));
            }
            match apply_hunk(ctx, hunk) {
                Ok(label) => applied.push(label),
                Err(err) => {
                    let prefix = if applied.is_empty() {
                        format!("Unable to apply patch: {}", err)
                    } else {
                        format!(
                            "Patch partially applied before failing: {}. Applied: {}",
                            err,
                            applied.join(", ")
                        )
                    };
                    return ToolResult::error(prefix);
                }
            }
        }

        let mut lines = vec!["Applied patch sequentially:".to_string()];
        lines.extend(applied);
        ToolResult::text(lines.join("\n"))
    }
}

// ── Patch model ────────────────────────────────────

fn hunk_path(hunk: &Hunk) -> &str {
    match hunk {
        Hunk::Add { path, .. } | Hunk::Delete { path } | Hunk::Update { path, .. } => path,
    }
}

enum Hunk {
    Add {
        path: String,
        contents: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_path: Option<String>,
        chunks: Vec<UpdateChunk>,
    },
}

struct UpdateChunk {
    old_lines: Vec<String>,
    new_lines: Vec<String>,
    change_context: Option<String>,
    end_of_file: bool,
}

fn parse_patch(patch_text: &str) -> Result<Vec<Hunk>, String> {
    let lines: Vec<&str> = patch_text.trim().split('\n').collect();
    let begin = lines
        .iter()
        .position(|line| line.trim() == "*** Begin Patch");
    let end = lines.iter().position(|line| line.trim() == "*** End Patch");
    let (Some(begin), Some(end)) = (begin, end) else {
        return Err("missing Begin/End markers".to_string());
    };
    if begin >= end {
        return Err("missing Begin/End markers".to_string());
    }

    let mut hunks = Vec::new();
    let mut index = begin + 1;
    while index < end {
        let line = lines[index];
        if let Some(rest) = line.strip_prefix("*** Add File:") {
            let path = rest.trim().to_string();
            if path.is_empty() {
                return Err("invalid add file path".to_string());
            }
            let (contents, next) = parse_add(&lines, index + 1)?;
            hunks.push(Hunk::Add { path, contents });
            index = next;
            continue;
        }
        if let Some(rest) = line.strip_prefix("*** Delete File:") {
            let path = rest.trim().to_string();
            if path.is_empty() {
                return Err("invalid delete file path".to_string());
            }
            hunks.push(Hunk::Delete { path });
            index += 1;
            continue;
        }
        if let Some(rest) = line.strip_prefix("*** Update File:") {
            let path = rest.trim().to_string();
            if path.is_empty() {
                return Err("invalid update file path".to_string());
            }
            let mut next = index + 1;
            let mut move_path = None;
            if let Some(move_line) = lines.get(next).and_then(|l| l.strip_prefix("*** Move to:")) {
                let target = move_line.trim().to_string();
                if target.is_empty() {
                    return Err("invalid move file path".to_string());
                }
                move_path = Some(target);
                next += 1;
            }
            let (chunks, after) = parse_update(&lines, next)?;
            if chunks.is_empty() {
                return Err(format!(
                    "invalid update hunk for {}: expected at least one @@ chunk",
                    path
                ));
            }
            hunks.push(Hunk::Update {
                path,
                move_path,
                chunks,
            });
            index = after;
            continue;
        }
        return Err(format!("invalid patch line: {}", line));
    }
    Ok(hunks)
}

fn parse_add(lines: &[&str], start: usize) -> Result<(String, usize), String> {
    let mut content = Vec::new();
    let mut index = start;
    while index < lines.len() && !lines[index].starts_with("***") {
        let Some(rest) = lines[index].strip_prefix('+') else {
            return Err(format!("invalid add file line: {}", lines[index]));
        };
        content.push(rest.to_string());
        index += 1;
    }
    Ok((content.join("\n"), index))
}

fn parse_update(lines: &[&str], start: usize) -> Result<(Vec<UpdateChunk>, usize), String> {
    let mut chunks = Vec::new();
    let mut index = start;
    while index < lines.len() && !lines[index].starts_with("***") {
        let Some(context_rest) = lines[index].strip_prefix("@@") else {
            return Err(format!("invalid update file line: {}", lines[index]));
        };
        let change_context = {
            let trimmed = context_rest.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();
        let mut end_of_file = false;
        index += 1;
        while index < lines.len() && !lines[index].starts_with("@@") {
            let line = lines[index];
            if line == "*** End of File" {
                end_of_file = true;
                index += 1;
                break;
            }
            if line.starts_with("***") {
                break;
            }
            if let Some(rest) = line.strip_prefix(' ') {
                old_lines.push(rest.to_string());
                new_lines.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix('-') {
                old_lines.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix('+') {
                new_lines.push(rest.to_string());
            } else if line.is_empty() {
                old_lines.push(String::new());
                new_lines.push(String::new());
            } else {
                return Err(format!("invalid update chunk line: {}", line));
            }
            index += 1;
        }
        chunks.push(UpdateChunk {
            old_lines,
            new_lines,
            change_context,
            end_of_file,
        });
    }
    Ok((chunks, index))
}

// ── Apply ──────────────────────────────────────────

fn apply_hunk(ctx: &ToolContext, hunk: &Hunk) -> Result<String, String> {
    match hunk {
        Hunk::Add { path, contents } => {
            let resolved = resolve_path(ctx, path);
            if let Some(parent) = resolved.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {}", path, e))?;
            }
            if let Some(store) = &ctx.undo_store
                && let Ok(existing) = std::fs::read_to_string(&resolved) {
                    store.save_snapshot(&resolved, &existing);
                }
            let body = if contents.ends_with('\n') || contents.is_empty() {
                contents.clone()
            } else {
                format!("{}\n", contents)
            };
            std::fs::write(&resolved, body).map_err(|e| format!("{}: {}", path, e))?;
            Ok(format!("A {}", path))
        }
        Hunk::Delete { path } => {
            let resolved = resolve_path(ctx, path);
            if let Some(store) = &ctx.undo_store
                && let Ok(existing) = std::fs::read_to_string(&resolved) {
                    store.save_snapshot(&resolved, &existing);
                }
            std::fs::remove_file(&resolved).map_err(|e| format!("{}: {}", path, e))?;
            Ok(format!("D {}", path))
        }
        Hunk::Update { path, chunks, .. } => {
            let resolved = resolve_path(ctx, path);
            let original =
                std::fs::read_to_string(&resolved).map_err(|e| format!("{}: {}", path, e))?;
            let updated = derive(path, chunks, &original)?;
            if let Some(store) = &ctx.undo_store {
                store.save_snapshot(&resolved, &original);
            }
            std::fs::write(&resolved, updated).map_err(|e| format!("{}: {}", path, e))?;
            Ok(format!("M {}", path))
        }
    }
}

fn derive(path: &str, chunks: &[UpdateChunk], original: &str) -> Result<String, String> {
    let mut lines: Vec<String> = original.split('\n').map(|s| s.to_string()).collect();
    if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        lines.pop();
    }

    let mut replacements = compute_replacements(&lines, path, chunks)?;
    replacements.sort_by_key(|item| item.0);

    let mut updated = lines.clone();
    for (start, remove, insert) in replacements.into_iter().rev() {
        let end = (start + remove).min(updated.len());
        updated.splice(start..end, insert);
    }
    if updated.last().map(|s| !s.is_empty()).unwrap_or(true) {
        updated.push(String::new());
    }
    Ok(updated.join("\n"))
}

fn compute_replacements(
    lines: &[String],
    path: &str,
    chunks: &[UpdateChunk],
) -> Result<Vec<(usize, usize, Vec<String>)>, String> {
    let mut replacements = Vec::new();
    let mut line_index = 0usize;
    for chunk in chunks {
        if let Some(context) = &chunk.change_context {
            let found = seek(lines, std::slice::from_ref(context), line_index, false);
            let Some(found) = found else {
                return Err(format!("Failed to find context '{}' in {}", context, path));
            };
            line_index = found + 1;
        }
        if chunk.old_lines.is_empty() {
            replacements.push((lines.len(), 0, chunk.new_lines.clone()));
            continue;
        }
        let mut old_lines = chunk.old_lines.clone();
        let mut new_lines = chunk.new_lines.clone();
        let mut found = seek(lines, &old_lines, line_index, chunk.end_of_file);
        if found.is_none() && old_lines.last().map(|s| s.is_empty()).unwrap_or(false) {
            old_lines.pop();
            if new_lines.last().map(|s| s.is_empty()).unwrap_or(false) {
                new_lines.pop();
            }
            found = seek(lines, &old_lines, line_index, chunk.end_of_file);
        }
        let Some(found) = found else {
            return Err(format!(
                "Failed to find expected lines in {}:\n{}",
                path,
                chunk.old_lines.join("\n")
            ));
        };
        let remove = old_lines.len();
        replacements.push((found, remove, new_lines));
        line_index = found + remove;
    }
    Ok(replacements)
}

fn seek(lines: &[String], pattern: &[String], start: usize, eof: bool) -> Option<usize> {
    if pattern.is_empty() || pattern.len() > lines.len() {
        return None;
    }
    let comparers: [fn(&str, &str) -> bool; 3] = [cmp_exact, cmp_rstrip, cmp_trim];
    for compare in comparers {
        if eof {
            let offset = lines.len() - pattern.len();
            if offset >= start && matches_at(lines, pattern, offset, compare) {
                return Some(offset);
            }
        }
        for offset in start..=(lines.len() - pattern.len()) {
            if matches_at(lines, pattern, offset, compare) {
                return Some(offset);
            }
        }
    }
    None
}

fn matches_at(
    lines: &[String],
    pattern: &[String],
    offset: usize,
    compare: fn(&str, &str) -> bool,
) -> bool {
    pattern
        .iter()
        .enumerate()
        .all(|(i, line)| compare(&lines[offset + i], line))
}

fn cmp_exact(left: &str, right: &str) -> bool {
    left == right
}
fn cmp_rstrip(left: &str, right: &str) -> bool {
    left.trim_end() == right.trim_end()
}
fn cmp_trim(left: &str, right: &str) -> bool {
    left.trim() == right.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(dir: &std::path::Path) -> ToolContext {
        ToolContext::new(dir.to_path_buf())
    }

    #[tokio::test]
    async fn adds_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let patch = "*** Begin Patch\n*** Add File: new.txt\n+hello\n+world\n*** End Patch";
        let r = ApplyPatchTool
            .execute(
                ToolParams::new(serde_json::json!({"patchText": patch})),
                &ctx(dir.path()),
            )
            .await;
        assert!(r.into_text().contains("A new.txt"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new.txt")).unwrap(),
            "hello\nworld\n"
        );
    }

    #[tokio::test]
    async fn updates_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha\nbeta\ngamma\n").unwrap();
        let patch = "*** Begin Patch\n*** Update File: f.txt\n@@\n alpha\n-beta\n+BETA\n gamma\n*** End Patch";
        let r = ApplyPatchTool
            .execute(
                ToolParams::new(serde_json::json!({"patchText": patch})),
                &ctx(dir.path()),
            )
            .await;
        assert!(r.into_text().contains("M f.txt"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "alpha\nBETA\ngamma\n"
        );
    }

    #[tokio::test]
    async fn deletes_a_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gone.txt"), "x\n").unwrap();
        let patch = "*** Begin Patch\n*** Delete File: gone.txt\n*** End Patch";
        let r = ApplyPatchTool
            .execute(
                ToolParams::new(serde_json::json!({"patchText": patch})),
                &ctx(dir.path()),
            )
            .await;
        assert!(r.into_text().contains("D gone.txt"));
        assert!(!dir.path().join("gone.txt").exists());
    }

    #[tokio::test]
    async fn rejects_missing_markers() {
        let dir = tempfile::tempdir().unwrap();
        let r = ApplyPatchTool
            .execute(
                ToolParams::new(serde_json::json!({"patchText": "not a patch"})),
                &ctx(dir.path()),
            )
            .await;
        assert!(r.into_text().contains("verification failed"));
    }

    #[tokio::test]
    async fn reports_missing_context() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "one\ntwo\n").unwrap();
        let patch = "*** Begin Patch\n*** Update File: f.txt\n@@\n-nope\n+yep\n*** End Patch";
        let r = ApplyPatchTool
            .execute(
                ToolParams::new(serde_json::json!({"patchText": patch})),
                &ctx(dir.path()),
            )
            .await;
        assert!(r.into_text().contains("Failed to find expected lines"));
    }

    #[tokio::test]
    async fn refuses_protected_path_in_patch() {
        let protected = if cfg!(windows) {
            r"C:\Windows\System32\fake_test.txt"
        } else {
            "/etc/fake_test_xyz"
        };
        let patch = format!(
            "*** Begin Patch\n*** Add File: {}\n+evil\n*** End Patch",
            protected
        );
        let dir = tempfile::tempdir().unwrap();
        let r = ApplyPatchTool
            .execute(
                ToolParams::new(serde_json::json!({"patchText": patch})),
                &ctx(dir.path()),
            )
            .await;
        assert!(r.into_text().contains("Refusing"));
    }
}
