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

        // Check user skills first (project > global)
        let mut found: Option<(PathBuf, String, String)> = None;

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

        // Fallback: built-in skill
        if found.is_none()
            && let Some(body) = builtin_skill_body(name) {
                let mut metadata = HashMap::new();
                metadata.insert("name".to_string(), serde_json::json!(name));
                metadata.insert("path".to_string(), serde_json::json!(format!("builtin/{}", name)));
                metadata.insert("frontmatter_ok".to_string(), serde_json::json!(true));
                return ToolResult::Structured {
                    content: body.to_string(),
                    metadata,
                };
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

fn skill_dirs(cwd: &Path) -> Vec<PathBuf> {
    vec![
        cwd.join(".openrust").join("skills"),
        cwd.join(".opencode").join("skills"),
        cwd.join("skills"),
        PlatformPaths::detect().config_dir().join("skills"),
    ]
}

pub fn skill_directories(ctx: &ToolContext) -> Vec<PathBuf> {
    skill_dirs(&ctx.cwd)
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
    list_skills_impl(&skill_dirs(cwd))
}

fn list_skills_impl(dirs: &[PathBuf]) -> Vec<SkillEntry> {
    let mut map: HashMap<String, SkillEntry> = HashMap::new();

    // Layer 1: built-in skills (lowest priority)
    for skill in builtin_skills() {
        map.insert(skill.name.clone(), skill);
    }

    // Layer 2+: user skills. dirs = [project, global].
    // Iterate in reverse so project overwrites global overwrites built-in.
    for dir in dirs.iter().rev() {
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
                let description = std::fs::read_to_string(&skill_file)
                    .ok()
                    .map(|content| extract_description(&content))
                    .unwrap_or_default();
                map.insert(
                    name.to_string(),
                    SkillEntry {
                        name: name.to_string(),
                        description,
                        path: skill_file,
                    },
                );
            }
        }
    }

    let mut entries: Vec<_> = map.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

// ── Built-in skills ────────────────────────────────

fn builtin_skills() -> Vec<SkillEntry> {
    vec![
        SkillEntry {
            name: "create-agent".to_string(),
            description: BUILTIN_CREATE_AGENT_DESC.to_string(),
            path: PathBuf::from("builtin/create-agent"),
        },
        SkillEntry {
            name: "create-skills".to_string(),
            description: BUILTIN_CREATE_SKILLS_DESC.to_string(),
            path: PathBuf::from("builtin/create-skills"),
        },
    ]
}

fn builtin_skill_body(name: &str) -> Option<&'static str> {
    match name {
        "create-agent" => Some(BUILTIN_CREATE_AGENT_BODY),
        "create-skills" => Some(BUILTIN_CREATE_SKILLS_BODY),
        _ => None,
    }
}

const BUILTIN_CREATE_AGENT_DESC: &str =
    "Create or modify an agent definition file (.openrust/agents/*.md)";

const BUILTIN_CREATE_SKILLS_DESC: &str =
    "Create or modify a skill definition file (.openrust/skills/*/SKILL.md)";

const BUILTIN_CREATE_AGENT_BODY: &str = r#"# create-agent

Use this skill when creating or modifying agent definition files.

## File locations

- Project agent: `.openrust/agents/<name>.md`
- Global agent: `~/.config/openrust/agents/<name>.md`

Project agents override global agents with the same name. Both override built-in agents (build, plan, general, explore, compaction, title, summary).

## File format

```
---
title: Display Name
description: One-line description for the agent picker
mode: primary
steps: 50
hidden: false
---

Agent system prompt text here.
```

## Frontmatter fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `title` | no | First `#` heading in body | Display name in agent picker |
| `description` | no | First paragraph in body | Short description in picker |
| `mode` | no | `primary` | Tool availability (see below) |
| `steps` | no | `50` | Max tool-loop iterations per turn |
| `hidden` | no | `false` | Hide from agent picker |

## Mode values

| Mode | Tools available |
|------|----------------|
| `primary` | All tools (default) |
| `plan` | Read-only: no write, edit, rm, apply_patch, bash, undo_edit |
| `explore` | Same as plan (read-only) |
| `subagent` | All tools except task, question (when spawned as subagent) |

## Workflow

1. Ask what the agent should do and whether it should be project-level or global.
2. Determine the appropriate `mode` based on whether the agent needs write access.
3. Determine `steps` based on task complexity (simple: 10-25, complex: 50-200).
4. Write the file with valid frontmatter and a clear system prompt body.
5. The agent ID = filename without `.md` (case-sensitive). No subdirectories.

## Body format

The body is appended after the base system prompt, which already includes environment info (cwd, platform, model), tool list, project rules, and AGENTS.md. Write **only** what is unique about this agent.

**Simple agent** — one-line role statement:
```
You are a general-purpose agent for researching complex questions and executing multi-step tasks.
```

**Guided agent** — role + behavioral guidelines:
```
You are a planning agent. Help the user organize work into clear steps.

Guidelines:
- Keep the plan concise and actionable
- Identify dependencies and unknowns
- Do not write code unless the user explicitly asks
```

**Structured agent** — role + XML sections (for fixed-output tasks like title generation):
```
You are a title generator. Output ONLY a title.

<rules>
- Use the same language as the user message
- <= 50 characters
- No explanations
</rules>
```

Do NOT repeat in the body: environment info, tool capabilities, project instructions. These are already injected.

## Overlay semantics

When overriding a built-in agent (e.g. creating `.openrust/agents/build.md`):
- Frontmatter fields you specify override the built-in values.
- Frontmatter fields you omit are inherited from the built-in.
- The markdown body replaces the built-in system prompt (if non-empty).
"#;

const BUILTIN_CREATE_SKILLS_BODY: &str = r#"# create-skills

Use this skill when creating or modifying skill definition files.

## Directory structure

- Project skill: `.openrust/skills/<skill-name>/SKILL.md`
- Global skill: `~/.config/openrust/skills/<skill-name>/SKILL.md`

Project skills override global skills with the same name.

## File format

```
---
description: One-line description shown in system prompt
---

Skill body text. This content is returned when the skill tool
is invoked with the skill name.
```

## Frontmatter fields

| Field | Required | Description |
|-------|----------|-------------|
| `description` | yes | Shown in the system prompt capabilities section so the LLM knows when to use this skill |

## Rules

1. Skill name = directory name (case-sensitive). The file MUST be named `SKILL.md`.
2. Each skill lives in its own subdirectory: `<skill-name>/SKILL.md`.
3. The `description` frontmatter field is required.
4. The body (after frontmatter) is the skill's instruction content — write clear, actionable steps.
5. When modifying an existing skill, preserve the description unless the user asks to change it.

## Body format

The body is injected into the conversation when the skill tool is invoked. It should be self-contained instructions the LLM can follow immediately.

**Good skill body** — concrete, scoped, actionable:
```
# deploy-checklist

Run through this checklist before deploying:

1. Run `npm run typecheck` — fix any errors.
2. Run `npm test` — all tests must pass.
3. Check `git status` for uncommitted changes.
4. Verify the build: `npm run build`.

If any step fails, stop and report the error. Do not attempt to fix it.
```

**Bad skill body** — vague, no actions:
```
This skill helps with deployment. Make sure everything is ready before you deploy.
```

Principles:
- Start with a `#` heading naming the skill.
- Use numbered steps for sequential workflows.
- Specify exact commands when possible.
- State what to do on failure (stop, report, retry?).
- Keep it under ~50 lines. If longer, split into multiple skills.

## Workflow

1. Ask what the skill should do and whether it should be project-level or global.
2. Create the directory: `mkdir -p .openrust/skills/<skill-name>` (or global equivalent).
3. Write the `SKILL.md` file with frontmatter and body.
4. The skill name should be lowercase-hyphenated (e.g. `deploy-checklist`, `code-review`).
"#;

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
        let skill = root.join(".openrust").join("skills").join("demo");
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
        let skill = root.join(".openrust").join("skills").join("present");
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

        let project_dir = root.join(".openrust").join("skills").join("dual");
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
        let skill = root.join(".openrust").join("skills").join("demo");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: demo\ndescription: My cool skill\n---\nBody",
        )
        .unwrap();

        let ctx = ToolContext::new(root);
        let skills = list_skills(&ctx);
        let demo = skills.iter().find(|s| s.name == "demo").unwrap();
        assert_eq!(demo.description, "My cool skill");
    }

    // ── built-in skill tests ──────────────────────────

    #[test]
    fn builtin_skills_are_registered() {
        let names: Vec<_> = builtin_skills().into_iter().map(|s| s.name).collect();
        assert!(names.contains(&"create-agent".to_string()));
        assert!(names.contains(&"create-skills".to_string()));
    }

    #[tokio::test]
    async fn builtin_create_agent_loads_without_filesystem() {
        let root = temp_dir();
        let ctx = ToolContext::new(root);
        let result = SkillTool
            .execute(ToolParams::new(serde_json::json!({ "name": "create-agent" })), &ctx)
            .await;
        let text = result.into_text();
        assert!(text.contains("create-agent"));
        assert!(text.contains("mode: primary"));
        assert!(text.contains("Frontmatter fields"));
    }

    #[tokio::test]
    async fn builtin_create_skills_loads_without_filesystem() {
        let root = temp_dir();
        let ctx = ToolContext::new(root);
        let result = SkillTool
            .execute(ToolParams::new(serde_json::json!({ "name": "create-skills" })), &ctx)
            .await;
        let text = result.into_text();
        assert!(text.contains("SKILL.md"));
        assert!(text.contains("description"));
    }

    #[test]
    fn builtin_skills_appear_in_list() {
        let root = temp_dir();
        let ctx = ToolContext::new(root);
        let names = list_skill_names(&ctx);
        assert!(names.contains(&"create-agent".to_string()));
        assert!(names.contains(&"create-skills".to_string()));
    }

    #[tokio::test]
    async fn user_skill_overrides_builtin() {
        let root = temp_dir();
        let skill = root.join(".openrust").join("skills").join("create-agent");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "Custom override").unwrap();

        let ctx = ToolContext::new(root);
        let result = SkillTool
            .execute(ToolParams::new(serde_json::json!({ "name": "create-agent" })), &ctx)
            .await;
        assert!(result.into_text().contains("Custom override"));
    }
}
