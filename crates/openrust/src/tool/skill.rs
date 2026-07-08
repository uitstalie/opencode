//! Skill tool — load a skill's SKILL.md instructions by name.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::core::platform::PlatformPaths;
use crate::require_str;
use crate::tool::{Tool, ToolContext, ToolParams, ToolResult};
use serde_json::Value;

pub struct SkillTool;

#[async_trait::async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "skill"
    }
    fn description(&self) -> &'static str {
        "Load a specialized skill by name. Returns the skill's SKILL.md instructions for you to follow. Use when the task matches an available skill."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "The name of the skill to load" }
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, p: ToolParams, ctx: &ToolContext) -> ToolResult {
        let name = require_str!(p, "name");

        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return ToolResult::error(format!(
                "skill '{}' contains invalid path characters",
                name
            ));
        }

        let mut found: Option<(PathBuf, String, String)> = None; // (path, body, frontmatter_errors)

        for dir in skill_directories(ctx) {
            let path = dir.join(name).join("SKILL.md");
            if !path.exists() {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let (body, fm_errors) = parse_frontmatter(&content);
                    found = Some((path, body, fm_errors));
                    break;
                }
                Err(err) => {
                    return ToolResult::error(format!(
                        "skill '{}' at {}: {}",
                        name,
                        path.display(),
                        err
                    ));
                }
            }
        }

        let (skill_path, body, fm_errors) = match found {
            Some(f) => f,
            None => {
                let available = list_skill_names(ctx);
                if available.is_empty() {
                    return ToolResult::error(format!(
                        "skill '{}' not found; no skills are installed",
                        name
                    ));
                }
                return ToolResult::error(format!(
                    "skill '{}' not found. Available skills: {}",
                    name,
                    available.join(", ")
                ));
            }
        };

        let mut content = body;
        if !fm_errors.is_empty() {
            content = format!(
                "[warning: frontmatter issues in {}: {}]\n\n{}",
                skill_path.display(),
                fm_errors,
                content
            );
        }

        let mut metadata = HashMap::new();
        metadata.insert("name".to_string(), serde_json::json!(name));
        metadata.insert("path".to_string(), serde_json::json!(skill_path.to_string_lossy().as_ref()));
        metadata.insert("frontmatter_ok".to_string(), serde_json::json!(fm_errors.is_empty()));
        ToolResult::Structured { content, metadata }
    }
}

pub fn skill_directories(ctx: &ToolContext) -> Vec<PathBuf> {
    let mut dirs = vec![
        ctx.cwd.join(".openrust").join("skills"),
        ctx.cwd.join(".opencode").join("skills"),
        ctx.cwd.join("skills"),
    ];
    dirs.push(PlatformPaths::detect().config_dir().join("skills"));
    dirs
}

pub fn list_skill_names(ctx: &ToolContext) -> Vec<String> {
    list_skills(ctx)
        .into_iter()
        .map(|s| s.name)
        .collect()
}

pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

pub fn list_skills(ctx: &ToolContext) -> Vec<SkillEntry> {
    list_skills_impl(&skill_directories(ctx))
}

pub fn list_skills_for_cwd(cwd: &Path) -> Vec<SkillEntry> {
    let dirs = vec![
        cwd.join(".openrust").join("skills"),
        cwd.join(".opencode").join("skills"),
        cwd.join("skills"),
        PlatformPaths::detect().config_dir().join("skills"),
    ];
    list_skills_impl(&dirs)
}

fn list_skills_impl(dirs: &[PathBuf]) -> Vec<SkillEntry> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entries = Vec::new();

    for dir in dirs {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let skill_path = entry.path();
            let skill_file = skill_path.join("SKILL.md");
            if !skill_file.exists() {
                continue;
            }
            if let Some(name) = skill_path.file_name().and_then(|n| n.to_str()) {
                if seen.insert(name.to_string()) {
                    let description = std::fs::read_to_string(&skill_file)
                        .ok()
                        .map(|content| extract_description(&content))
                        .unwrap_or_default();
                    entries.push(SkillEntry {
                        name: name.to_string(),
                        description,
                        path: skill_file,
                    });
                }
            }
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// Extract the `description` field from a SKILL.md's frontmatter.
fn extract_description(content: &str) -> String {
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return String::new();
    };
    let end = match rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n")) {
        Some(o) => o,
        None => return String::new(),
    };
    for line in rest[..end].lines() {
        if let Some(value) = line.trim().strip_prefix("description:") {
            return value.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    String::new()
}

/// Parse and strip a leading `---` YAML frontmatter block.
/// Returns `(body, frontmatter_errors)`.
fn parse_frontmatter(content: &str) -> (String, String) {
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return (content.to_string(), String::new());
    };

    let end_offset = match rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n")) {
        Some(o) => o,
        None => {
            return (
                content.to_string(),
                String::from("missing closing '---' delimiter"),
            );
        }
    };

    let fm = &rest[..end_offset];

    // Light validation: frontmatter lines should be "key: value" or start with "-"
    let mut errors = Vec::new();
    for line in fm.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !trimmed.contains(':') && !trimmed.starts_with('-') {
            errors.push(format!("invalid YAML line: '{}'", trimmed));
        }
    }

    // Body starts after the closing `\n---\n` (5 chars) or `\r\n---\r\n` (7 chars)
    let body_start = if rest[end_offset..].starts_with("\n---\n") {
        end_offset + 5
    } else {
        end_offset + 7
    };
    let body = rest[body_start..]
        .trim_start_matches(['\r', '\n'])
        .to_string();

    let fm_errors = if errors.is_empty() {
        String::new()
    } else {
        errors.join("; ")
    };

    (body, fm_errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("openrust-skill-{}", now.as_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn loads_skill_body() {
        let root = temp_dir();
        let skill = root.join(".opencode").join("skills").join("demo");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: demo\n---\nStep one\nStep two\n",
        )
        .unwrap();

        let ctx = ToolContext::new(root);
        let result = SkillTool
            .execute(ToolParams::new(serde_json::json!({ "name": "demo" })), &ctx)
            .await;
        let text = result.into_text();
        assert!(text.contains("Step one"));
        assert!(!text.contains("name: demo"));
    }

    #[tokio::test]
    async fn missing_skill_reports_available() {
        let root = temp_dir();
        let skill = root.join("skills").join("present");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "body").unwrap();

        let ctx = ToolContext::new(root);
        let result = SkillTool
            .execute(
                ToolParams::new(serde_json::json!({ "name": "absent" })),
                &ctx,
            )
            .await;
        assert!(matches!(result, ToolResult::Error(_)));
        assert!(result.into_text().contains("present"));
    }

    #[tokio::test]
    async fn skill_from_skills_dir_overrides_global() {
        let root = temp_dir();

        let project_dir = root.join("skills").join("dual");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("SKILL.md"), "project content").unwrap();

        let ctx = ToolContext::new(root);
        let result = SkillTool
            .execute(
                ToolParams::new(serde_json::json!({ "name": "dual" })),
                &ctx,
            )
            .await;
        assert!(result.into_text().contains("project content"));
    }

    #[test]
    fn malformed_frontmatter_warns() {
        let (body, errors) = parse_frontmatter("---\nbad line\n---\nreal body\n");
        assert!(errors.contains("invalid YAML line"));
        assert!(body.contains("real body"));
    }

    #[test]
    fn missing_closing_delimiter_warns() {
        let (body, errors) = parse_frontmatter("---\nkey: value\nmore body\n");
        assert!(errors.contains("missing closing"));
        assert!(body.contains("---"));
    }

    #[test]
    fn extract_description_from_frontmatter() {
        let content = "---\nname: demo\ndescription: A skill for testing\n---\nBody";
        assert_eq!(extract_description(content), "A skill for testing");
    }

    #[test]
    fn extract_description_missing_returns_empty() {
        assert_eq!(extract_description("---\nname: demo\n---\nBody"), "");
        assert_eq!(extract_description("No frontmatter at all"), "");
    }

    #[tokio::test]
    async fn rejects_path_traversal_in_skill_name() {
        let root = temp_dir();
        let ctx = ToolContext::new(root);
        let result = SkillTool
            .execute(ToolParams::new(serde_json::json!({ "name": "../etc/passwd" })), &ctx)
            .await;
        assert!(matches!(result, ToolResult::Error(_)));
        assert!(result.into_text().contains("invalid path characters"));
    }

    #[tokio::test]
    async fn list_skills_includes_description() {
        let root = temp_dir();
        let skill = root.join(".opencode").join("skills").join("demo");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: demo\ndescription: My cool skill\n---\nBody",
        )
        .unwrap();

        let ctx = ToolContext::new(root);
        let skills = list_skills(&ctx);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "demo");
        assert_eq!(skills[0].description, "My cool skill");
    }
}
