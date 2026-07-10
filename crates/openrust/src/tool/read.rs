//! Read file tool.

use crate::tool::{Tool, ToolContext, ToolParams, ToolResult, resolve_path};
use crate::try_tool;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// File extensions that are treated as binary / non-text.
pub const BINARY_EXTENSIONS: &[&str] = &[
    "3gp", "7z", "aac", "apk", "avi", "bin", "bmp", "bz2", "class", "db", "deb", "dll", "dmg",
    "doc", "docx", "ear", "eot", "exe", "flac", "flv", "gif", "gz", "ico", "iso", "jar", "jpeg",
    "jpg", "lz", "lz4", "lzma", "m4a", "mkv", "mov", "mp3", "mp4", "mpeg", "mpg", "o", "obj",
    "ogg", "otf", "parquet", "pdf", "pkg", "png", "ppt", "pptx", "psd", "rar", "rpm", "so",
    "sqlite", "swf", "tar", "ttf", "wasm", "wav", "webm", "webp", "woff", "woff2",
    "xls", "xlsx", "xz", "zip", "zst",
];

/// File extensions limited to images (may warrant special handling).
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "bmp", "gif", "ico", "jpeg", "jpg", "png", "svg", "webp",
];

/// Max file size for reading text (10 MB).
pub const MAX_READ_SIZE: u64 = 10 * 1024 * 1024;

pub struct ReadTool;

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }
    fn description(&self) -> &'static str {
        "Read a file or part of it with line numbers. Use offset/limit to read a slice of a large file; directories list their entries. Binary files are rejected."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to read relative to cwd, or absolute path" },
                "filePath": { "type": "string", "description": "Legacy alias for path" },
                "offset": { "type": "integer", "description": "Line to start at (1-indexed, default 1)" },
                "limit": { "type": "integer", "description": "Max lines to read (default 2000)" }
            },
            "required": []
        })
    }

    async fn execute(&self, p: ToolParams, _ctx: &ToolContext) -> ToolResult {
        let path = p
            .opt_str("path")
            .or_else(|| p.opt_str("filePath"))
            .unwrap_or("");
        if path.is_empty() {
            return ToolResult::error("Missing required parameter: path");
        }
        let offset = p.u64_or("offset", 1) as usize;
        let limit = p.u64_or("limit", 2000) as usize;
        let resolved = resolve_path(_ctx, path);

        let canonical = match resolved.canonicalize() {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("Cannot resolve path: {}", e)),
        };

        if is_binary_path(&canonical) {
            return ToolResult::error(format!(
                "Cannot read binary file: {} (image/binary/archive). Use external tools for these formats.",
                canonical.display()
            ));
        }

        if is_image_path(&canonical) {
            return read_image(&canonical);
        }

        if Path::new(&canonical).is_dir() {
            return read_directory(&canonical, offset, limit);
        }

        let metadata = try_tool!(std::fs::metadata(&canonical), |e| format!("Cannot stat {}: {}", canonical.display(), e));
        if metadata.len() > MAX_READ_SIZE {
            return ToolResult::error(format!(
                "File too large: {} bytes (max {} bytes). Use bash tools for large files.",
                metadata.len(),
                MAX_READ_SIZE
            ));
        }

        let content = try_tool!(std::fs::read_to_string(&canonical), |e| format!("Cannot read {}: {}", canonical.display(), e));

        if is_binary_content(&content) {
            return ToolResult::error(format!(
                "Cannot read binary file: {} (contains binary data). Use external tools.",
                canonical.display()
            ));
        }

        let lines: Vec<&str> = content.lines().collect();
        if offset == 0 || offset > lines.len() {
            return ToolResult::error(format!(
                "File has {} lines, offset {} out of range.",
                lines.len(),
                offset
            ));
        }
        let start = offset - 1;
        let end = (start + limit).min(lines.len());
        let selected: Vec<String> = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:6}: {}", start + i + 1, l))
            .collect();
        let mut out = selected.join("\n");
        if end < lines.len() {
            out.push_str(&format!("\n... ({} lines remaining)", lines.len() - end));
        }
        let file_size = metadata.len();
        let mut meta_map = HashMap::new();
        meta_map.insert("path".to_string(), serde_json::json!(path));
        meta_map.insert("lines".to_string(), serde_json::json!(lines.len()));
        meta_map.insert("bytes".to_string(), serde_json::json!(file_size));
        ToolResult::Structured { content: out, metadata: meta_map }
    }
}

fn is_binary_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| BINARY_EXTENSIONS.iter().any(|&b| b.eq_ignore_ascii_case(ext)))
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|&b| b.eq_ignore_ascii_case(ext)))
}

fn is_binary_content(content: &str) -> bool {
    content.as_bytes().iter().take(8192).any(|&b| b == 0)
}

fn read_image(path: &Path) -> ToolResult {
    // Map extension to MIME type
    let mime_type = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| match e.to_lowercase().as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            "ico" => "image/x-icon",
            "svg" => "image/svg+xml",
            _ => "image/png",
        })
        .unwrap_or("image/png");

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => return ToolResult::error(format!("Cannot read image {}: {}", path.display(), e)),
    };

    if data.len() > MAX_READ_SIZE as usize {
        return ToolResult::error(format!(
            "Image too large: {} bytes (max {} bytes). Use external tools.",
            data.len(),
            MAX_READ_SIZE
        ));
    }

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);

    ToolResult::Image {
        mime_type: mime_type.to_string(),
        base64_data: b64,
        description: format!("(image: {} at {}, {} bytes)", mime_type, path.display(), data.len()),
    }
}

fn read_directory(resolved: &Path, offset: usize, limit: usize) -> ToolResult {
    let mut entries = match std::fs::read_dir(resolved) {
        Ok(read_dir) => read_dir
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() {
                    format!("{}/", name)
                } else {
                    name
                }
            })
            .collect::<Vec<_>>(),
        Err(err) => return ToolResult::error(format!("Cannot read {}", err)),
    };
    entries.sort();
    if offset == 0 || offset > entries.len() {
        return ToolResult::error(format!(
            "Directory has {} entries, offset {} out of range.",
            entries.len(),
            offset
        ));
    }
    let start = offset - 1;
    let end = (start + limit).min(entries.len());
    let mut out = entries[start..end]
        .iter()
        .enumerate()
        .map(|(i, entry)| format!("{:6}: {}", start + i + 1, entry))
        .collect::<Vec<_>>()
        .join("\n");
    if end < entries.len() {
        out.push_str(&format!(
            "\n... ({} entries remaining)",
            entries.len() - end
        ));
    }
    ToolResult::text(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> ToolContext {
        ToolContext::new(std::env::current_dir().unwrap())
    }

    #[tokio::test]
    async fn reads_self() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"filePath": file!()})),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("ReadTool"));
    }

    #[tokio::test]
    async fn nonexistent_file() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"filePath": "/no/such"})),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("Cannot"));
    }

    #[tokio::test]
    async fn with_offset_limit() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({
                    "filePath": file!(), "offset": 1, "limit": 3
                })),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("1:"));
    }

    #[tokio::test]
    async fn path_alias_resolves_relative_to_cwd() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"path": "src/tool/read.rs"})),
                &ctx(),
            )
            .await;
        assert!(r.into_text().contains("ReadTool"));
    }

    #[tokio::test]
    async fn lists_directory_entries() {
        let r = ReadTool
            .execute(
                ToolParams::new(serde_json::json!({"path": "src/tool"})),
                &ctx(),
            )
            .await;
        let text = r.into_text();
        assert!(text.contains("read.rs"));
        assert!(text.contains(":"));
    }

    #[tokio::test]
    async fn missing_path_reports_error() {
        let r = ReadTool
            .execute(ToolParams::new(serde_json::json!({})), &ctx())
            .await;
        assert!(r.into_text().contains("Missing required parameter"));
    }

    #[test]
    fn binary_detection_by_extension() {
        assert!(is_binary_path(Path::new("app.exe")));
        assert!(is_binary_path(Path::new("lib.so")));
        assert!(is_binary_path(Path::new("image.png")));
        assert!(!is_binary_path(Path::new("main.rs")));
        assert!(!is_binary_path(Path::new("README.md")));
    }

    #[test]
    fn binary_detection_by_content() {
        let mut data = vec![b'H', b'i', 0, b'!'];
        data.extend(vec![b' '; 8192]);
        let s = String::from_utf8_lossy(&data).to_string();
        assert!(is_binary_content(&s));
        assert!(!is_binary_content("Hello world"));
    }
}
