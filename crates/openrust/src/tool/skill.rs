//! Skill tool — load a skill's SKILL.md instructions by name.

use std::path::PathBuf;

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

        for dir in skill_directories(ctx) {
            let path = dir.join(name).join("SKILL.md");
            if !path.exists() {
                continue;
            }
            return match std::fs::read_to_string(&path) {
                Ok(content) => ToolResult::text(strip_frontmatter(&content)),
                Err(err) => ToolResult::error(format!("skill '{}': {}", name, err)),
            };
        }

        let available = list_skills(ctx);
        if available.is_empty() {
            return ToolResult::error(format!(
                "skill '{}' not found; no skills are installed",
                name
            ));
        }
        ToolResult::error(format!(
            "skill '{}' not found. Available skills: {}",
            name,
            available.join(", ")
        ))
    }
}

fn skill_directories(ctx: &ToolContext) -> Vec<PathBuf> {
    let mut dirs = vec![
        ctx.cwd.join(".opencode").join("skills"),
        ctx.cwd.join("skills"),
    ];
    dirs.push(PlatformPaths::detect().config_dir().join("skills"));
    dirs
}

fn list_skills(ctx: &ToolContext) -> Vec<String> {
    let mut names = Vec::new();
    for dir in skill_directories(ctx) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("SKILL.md").exists() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if !names.iter().any(|existing| existing == name) {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }
    names.sort();
    names
}

/// Strip a leading `---` YAML frontmatter block, returning the body.
fn strip_frontmatter(content: &str) -> String {
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return content.to_string();
    };
    let Some(end) = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n")) else {
        return content.to_string();
    };
    rest[end + 5..].trim_start_matches(['\r', '\n']).to_string()
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
}
