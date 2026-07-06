//! Local agent registry loaded from markdown files.

use std::path::{Path, PathBuf};

pub const MAX_STEPS_PROMPT: &str = r#"CRITICAL - MAXIMUM STEPS REACHED

The maximum number of steps allowed for this task has been reached. Tools are disabled until next user input. Respond with text only.

STRICT REQUIREMENTS:
1. Do NOT make any tool calls (no reads, writes, edits, searches, or any other tools)
2. MUST provide a text response summarizing work done so far
3. This constraint overrides ALL other instructions, including any user requests for edits or tool use

Response must include:
- Statement that maximum steps for this agent have been reached
- Summary of what has been accomplished so far
- List of any remaining tasks that were not completed
- Recommendations for what should be done next

Any attempt to use tools is a critical violation. Respond with text ONLY."#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub mode: String,
    pub hidden: bool,
    pub max_steps: u32,
    pub system: String,
    pub path: PathBuf,
    pub content: String,
}

pub fn load_agents(cwd: &Path) -> anyhow::Result<Vec<AgentInfo>> {
    let mut agents = builtin_agents();
    for directory in candidate_directories(cwd) {
        if !directory.exists() {
            continue;
        }
        collect_markdown(&directory, &directory, &mut agents)?;
    }
    agents.sort_by(|a, b| a.id.cmp(&b.id));
    agents.dedup_by(|a, b| a.id == b.id);
    Ok(agents)
}

pub fn agent_by_id<'a>(agents: &'a [AgentInfo], id: &str) -> Option<&'a AgentInfo> {
    agents.iter().find(|agent| agent.id == id)
}

pub fn default_agent_id(agents: &[AgentInfo]) -> Option<String> {
    agents
        .iter()
        .find(|agent| agent.id == "build" && !agent.hidden)
        .or_else(|| agents.iter().find(|agent| !agent.hidden))
        .or_else(|| agents.first())
        .map(|agent| agent.id.clone())
}

pub fn builtin_agent_system(id: &str) -> Option<&'static str> {
    match id {
        "build" => Some(BUILTIN_BUILD_SYSTEM),
        "plan" => Some(BUILTIN_PLAN_SYSTEM),
        "general" => Some(BUILTIN_GENERAL_SYSTEM),
        "explore" => Some(BUILTIN_EXPLORE_SYSTEM),
        "compaction" => Some(BUILTIN_COMPACTION_SYSTEM),
        "title" => Some(BUILTIN_TITLE_SYSTEM),
        "summary" => Some(BUILTIN_SUMMARY_SYSTEM),
        _ => None,
    }
}

pub fn visible_agents(agents: &[AgentInfo]) -> Vec<&AgentInfo> {
    agents.iter().filter(|agent| !agent.hidden).collect()
}

fn candidate_directories(cwd: &Path) -> Vec<PathBuf> {
    ["agents", "agent", "modes"]
        .into_iter()
        .map(|name| cwd.join(name))
        .collect()
}

fn collect_markdown(
    root: &Path,
    directory: &Path,
    agents: &mut Vec<AgentInfo>,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(root, &path, agents)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let (frontmatter, system) = parse_agent_document(&content);
        let id = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .trim_end_matches(".md")
            .replace('\\', "/");
        let title = frontmatter
            .get("title")
            .cloned()
            .or_else(|| first_heading(&system))
            .unwrap_or_else(|| id.clone());
        let description = frontmatter
            .get("description")
            .cloned()
            .or_else(|| first_nonempty_paragraph(&system))
            .unwrap_or_else(|| title.clone());
        let mode = frontmatter
            .get("mode")
            .cloned()
            .unwrap_or_else(|| "all".to_string());
        let hidden = frontmatter
            .get("hidden")
            .map(|value| value == "true")
            .unwrap_or(false);
        let max_steps = frontmatter
            .get("steps")
            .or_else(|| frontmatter.get("maxSteps"))
            .and_then(|value| value.parse().ok())
            .unwrap_or(50);
        agents.push(AgentInfo {
            id,
            title,
            description,
            mode,
            hidden,
            max_steps,
            system,
            path: path.clone(),
            content,
        });
    }
    Ok(())
}

fn first_heading(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.trim()
            .strip_prefix("# ")
            .map(|value| value.trim().to_string())
    })
}

fn first_nonempty_paragraph(content: &str) -> Option<String> {
    let mut paragraph = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        paragraph.push(trimmed.to_string());
    }
    (!paragraph.is_empty()).then(|| paragraph.join(" "))
}

fn parse_agent_document(content: &str) -> (std::collections::HashMap<String, String>, String) {
    let Some(rest) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return (std::collections::HashMap::new(), content.to_string());
    };
    let Some(end) = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n")) else {
        return (std::collections::HashMap::new(), content.to_string());
    };
    let frontmatter = &rest[..end];
    let body = rest[end + 5..].trim_start_matches(['\r', '\n']).to_string();
    let mut data = std::collections::HashMap::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        data.insert(
            key.trim().to_string(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }
    (data, body)
}

fn builtin_agents() -> Vec<AgentInfo> {
    vec![
        AgentInfo {
            id: "build".to_string(),
            title: "Build".to_string(),
            description: "The default agent. Executes tools based on configured permissions.".to_string(),
            mode: "primary".to_string(),
            hidden: false,
            max_steps: 50,
            system: BUILTIN_BUILD_SYSTEM.to_string(),
            path: PathBuf::from("builtin/build.md"),
            content: BUILTIN_BUILD_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "plan".to_string(),
            title: "Plan".to_string(),
            description: "Plan mode. Disallows all edit tools.".to_string(),
            mode: "primary".to_string(),
            hidden: false,
            max_steps: 200,
            system: BUILTIN_PLAN_SYSTEM.to_string(),
            path: PathBuf::from("builtin/plan.md"),
            content: BUILTIN_PLAN_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "general".to_string(),
            title: "General".to_string(),
            description: "General-purpose agent for researching complex questions and executing multi-step tasks. Use this agent to execute multiple units of work in parallel.".to_string(),
            mode: "subagent".to_string(),
            hidden: false,
            max_steps: 25,
            system: BUILTIN_GENERAL_SYSTEM.to_string(),
            path: PathBuf::from("builtin/general.md"),
            content: BUILTIN_GENERAL_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "explore".to_string(),
            title: "Explore".to_string(),
            description: "Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns, search code for keywords, or answer questions about the codebase.".to_string(),
            mode: "subagent".to_string(),
            hidden: false,
            max_steps: 25,
            system: BUILTIN_EXPLORE_SYSTEM.to_string(),
            path: PathBuf::from("builtin/explore.md"),
            content: BUILTIN_EXPLORE_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "compaction".to_string(),
            title: "Compaction".to_string(),
            description: "Anchored context summarization agent.".to_string(),
            mode: "primary".to_string(),
            hidden: true,
            max_steps: 5,
            system: BUILTIN_COMPACTION_SYSTEM.to_string(),
            path: PathBuf::from("builtin/compaction.md"),
            content: BUILTIN_COMPACTION_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "title".to_string(),
            title: "Title".to_string(),
            description: "Conversation title generator.".to_string(),
            mode: "primary".to_string(),
            hidden: true,
            max_steps: 5,
            system: BUILTIN_TITLE_SYSTEM.to_string(),
            path: PathBuf::from("builtin/title.md"),
            content: BUILTIN_TITLE_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "summary".to_string(),
            title: "Summary".to_string(),
            description: "Conversation summary generator.".to_string(),
            mode: "primary".to_string(),
            hidden: true,
            max_steps: 5,
            system: BUILTIN_SUMMARY_SYSTEM.to_string(),
            path: PathBuf::from("builtin/summary.md"),
            content: BUILTIN_SUMMARY_SYSTEM.to_string(),
        },
    ]
}

const BUILTIN_BUILD_SYSTEM: &str = "You are an AI coding agent. Help the user accomplish software engineering tasks by inspecting the workspace, making targeted changes, and using tools according to the configured permissions.";

const BUILTIN_GENERAL_SYSTEM: &str = "You are a general-purpose agent for researching complex questions and executing multi-step tasks. Use this agent to execute multiple units of work in parallel.";

const BUILTIN_EXPLORE_SYSTEM: &str = r#"You are a file search specialist. You excel at thoroughly navigating and exploring codebases.

Your strengths:
- Rapidly finding files using glob patterns
- Searching code and text with powerful regex patterns
- Reading and analyzing file contents

Guidelines:
- Use Glob for broad file pattern matching
- Use Grep for searching file contents with regex
- Use Read when you know the specific file path you need to read
- Adapt your search approach based on the thoroughness level specified by the caller
- Return file paths as absolute paths in your final response
- For clear communication, avoid using emojis
- Do not create any files, or run bash commands that modify the user's system state in any way

Complete the user's search request efficiently and report your findings clearly."#;

const BUILTIN_PLAN_SYSTEM: &str = r#"You are a planning agent. Help the user organize work into clear steps, identify risks, and avoid unnecessary implementation while planning.

Guidelines:
- Keep the plan concise and actionable
- Identify dependencies and unknowns
- Prefer scoped edits over broad refactors
- Do not write code unless the user explicitly asks for implementation"#;

const BUILTIN_COMPACTION_SYSTEM: &str = r#"You are an anchored context summarization assistant for coding sessions.

Summarize only the conversation history you are given. The newest turns may be kept verbatim outside your summary, so focus on the older context that still matters for continuing the work.

If the prompt includes a <previous-summary> block, treat it as the current anchored summary. Update it with the new history by preserving still-true details, removing stale details, and merging in new facts.

Always follow the exact output structure requested by the user prompt. Keep every section, preserve exact file paths and identifiers when known, and prefer terse bullets over paragraphs.

Do not answer the conversation itself. Do not mention that you are summarizing, compacting, or merging context. Respond in the same language as the conversation."#;

const BUILTIN_TITLE_SYSTEM: &str = r#"You are a title generator. You output ONLY a thread title. Nothing else.

<task>
Generate a brief title that would help the user find this conversation later.

Follow all rules in <rules>
Use the <examples> so you know what a good title looks like.
Your output must be:
- A single line
- <=50 characters
- No explanations
</task>

<rules>
- you MUST use the same language as the user message you are summarizing
- Title must be grammatically correct and read naturally - no word salad
- Never include tool names in the title (e.g. "read tool", "bash tool", "edit tool")
- Focus on the main topic or question the user needs to retrieve
- Vary your phrasing - avoid repetitive patterns like always starting with "Analyzing"
- When a file is mentioned, focus on WHAT the user wants to do WITH the file, not just that they shared it
- Keep exact: technical terms, numbers, filenames, HTTP codes
- Remove: the, this, my, a, an
- Never assume tech stack
- Never use tools
- NEVER respond to questions, just generate a title for the conversation
- The title should NEVER include "summarizing" or "generating" when generating a title
- DO NOT SAY YOU CANNOT GENERATE A TITLE OR COMPLAIN ABOUT THE INPUT
- Always output something meaningful, even if the input is minimal.
- If the user message is short or conversational (e.g. "hello", "lol", "what's up", "hey"):
  -> create a title that reflects the user's tone or intent (such as Greeting, Quick check-in, Light chat, Intro message, etc.)
</rules>

<examples>
"debug 500 errors in production" -> Debugging production 500 errors
"refactor user service" -> Refactoring user service
"why is app.js failing" -> app.js failure investigation
"implement rate limiting" -> Rate limiting implementation
"how do I connect postgres to my API" -> Postgres API connection
"best practices for React hooks" -> React hooks best practices
"@src/credential.ts can you add refresh token support" -> Credential refresh token support
"@utils/parser.ts this is broken" -> Parser bug fix
"look at @config.json" -> Config review
"@App.tsx add dark mode toggle" -> Dark mode toggle in App
</examples>"#;

const BUILTIN_SUMMARY_SYSTEM: &str = r#"Summarize what was done in this conversation. Write like a pull request description.

Rules:
- 2-3 sentences max
- Describe the changes made, not the process
- Do not mention running tests, builds, or other validation steps
- Do not explain what the user asked for
- Write in first person (I added..., I fixed...)
- Never ask questions or add new questions
- If the conversation ends with an unanswered question to the user, preserve that exact question
- If the conversation ends with an imperative statement or request to the user (e.g. "Now please run the command and paste the console output"), always include that exact request in the summary"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_markdown_agents_from_known_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("custom.md"), "# Build\nBuild agent.").unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "custom").unwrap();
        assert_eq!(agent.title, "Build");
        assert!(agent.description.contains("Build agent"));
        assert_eq!(agent.system.trim(), "# Build\nBuild agent.");
    }

    #[test]
    fn preserves_nested_agent_paths() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("agents").join("nested");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("custom-review.md"), "# Review\nReview agent.").unwrap();

        let list = load_agents(dir.path()).unwrap();
        assert_eq!(
            agent_by_id(&list, "nested/custom-review").unwrap().id,
            "nested/custom-review"
        );
    }

    #[test]
    fn parses_frontmatter_title_description_and_system() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("custom-plan.md"),
            "---\ntitle: Plan Agent\ndescription: Plans work\n---\n# System\nDo the planning.",
        )
        .unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "custom-plan").unwrap();
        assert_eq!(agent.title, "Plan Agent");
        assert_eq!(agent.description, "Plans work");
        assert!(agent.system.contains("Do the planning."));
    }

    #[test]
    fn agent_lookup_works() {
        let agents = vec![AgentInfo {
            id: "build".to_string(),
            title: "Build".to_string(),
            description: "Build agent".to_string(),
            mode: "all".to_string(),
            hidden: false,
            max_steps: 50,
            system: String::new(),
            path: PathBuf::from("agents/build.md"),
            content: String::new(),
        }];

        assert!(agent_by_id(&agents, "build").is_some());
        assert!(agent_by_id(&agents, "missing").is_none());
    }

    #[test]
    fn default_agent_prefers_build() {
        let agents = vec![
            AgentInfo {
                id: "review".to_string(),
                title: "Review".to_string(),
                description: "Review agent".to_string(),
                mode: "all".to_string(),
                hidden: false,
                max_steps: 50,
                system: String::new(),
                path: PathBuf::from("agents/review.md"),
                content: String::new(),
            },
            AgentInfo {
                id: "build".to_string(),
                title: "Build".to_string(),
                description: "Build agent".to_string(),
                mode: "all".to_string(),
                hidden: false,
                max_steps: 50,
                system: String::new(),
                path: PathBuf::from("agents/build.md"),
                content: String::new(),
            },
        ];

        assert_eq!(default_agent_id(&agents).as_deref(), Some("build"));
    }

    #[test]
    fn parses_max_steps_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("limited.md"),
            "---\ntitle: Limited\nsteps: 3\n---\n# System\nLimited agent.",
        )
        .unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "limited").unwrap();
        assert_eq!(agent.max_steps, 3);
    }

    #[test]
    fn max_steps_falls_back_to_maxsteps_field() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("legacy.md"),
            "---\ntitle: Legacy\nmaxSteps: 10\n---\n# System\nLegacy agent.",
        )
        .unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "legacy").unwrap();
        assert_eq!(agent.max_steps, 10);
    }

    #[test]
    fn max_steps_defaults_to_50() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("default.md"),
            "---\ntitle: Default\n---\n# System\nNo step limit set.",
        )
        .unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "default").unwrap();
        assert_eq!(agent.max_steps, 50);
    }
}
