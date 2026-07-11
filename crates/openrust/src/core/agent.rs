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
    pub tools: String,
    pub hidden: bool,
    pub max_steps: u32,
    pub system: String,
    pub path: PathBuf,
    pub content: String,
}

pub fn load_agents(cwd: &Path) -> anyhow::Result<Vec<AgentInfo>> {
    use std::collections::HashMap;
    let mut map: HashMap<String, AgentInfo> = HashMap::new();

    for agent in builtin_agents() {
        map.insert(agent.id.clone(), agent);
    }

    let global_dir = crate::core::platform::PlatformPaths::detect()
        .config_dir()
        .join("agents");
    apply_agent_dir(&mut map, &global_dir);

    let project_dir = cwd.join(".openrust").join("agents");
    apply_agent_dir(&mut map, &project_dir);

    let mut result: Vec<_> = map.into_values().collect();
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
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
        "memory-extract" => Some(BUILTIN_MEMORY_EXTRACT_SYSTEM),
        "dreaming" => Some(BUILTIN_DREAMING_SYSTEM),
        _ => None,
    }
}

pub fn visible_agents(agents: &[AgentInfo]) -> Vec<&AgentInfo> {
    agents.iter().filter(|agent| !agent.hidden).collect()
}

struct ParsedAgent {
    id: String,
    frontmatter: std::collections::HashMap<String, String>,
    system: String,
    path: PathBuf,
    content: String,
}

fn apply_agent_dir(map: &mut std::collections::HashMap<String, AgentInfo>, dir: &Path) {
    for parsed in parse_agent_dir(dir) {
        match map.entry(parsed.id.clone()) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                overlay_agent(e.get_mut(), &parsed);
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(parsed_to_agent(&parsed));
            }
        }
    }
}

fn parse_agent_dir(dir: &Path) -> Vec<ParsedAgent> {
    let mut result = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return result;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (frontmatter, system) = parse_agent_document(&content);
        result.push(ParsedAgent {
            id: id.to_string(),
            frontmatter,
            system,
            path,
            content,
        });
    }
    result
}

fn overlay_agent(base: &mut AgentInfo, parsed: &ParsedAgent) {
    if let Some(v) = parsed.frontmatter.get("title") {
        base.title = v.clone();
    }
    if let Some(v) = parsed.frontmatter.get("description") {
        base.description = v.clone();
    }
    if let Some(v) = parsed.frontmatter.get("tools") {
        base.tools = v.clone();
    }
    if let Some(v) = parsed.frontmatter.get("steps")
        && let Ok(n) = v.parse::<u32>() {
            base.max_steps = n;
        }
    if let Some(v) = parsed.frontmatter.get("hidden") {
        base.hidden = v == "true";
    }
    if !parsed.system.trim().is_empty() {
        base.system = parsed.system.clone();
    }
    base.path = parsed.path.clone();
    base.content = parsed.content.clone();
}

fn parsed_to_agent(parsed: &ParsedAgent) -> AgentInfo {
    AgentInfo {
        title: parsed
            .frontmatter
            .get("title")
            .cloned()
            .or_else(|| first_heading(&parsed.system))
            .unwrap_or_else(|| parsed.id.clone()),
        description: parsed
            .frontmatter
            .get("description")
            .cloned()
            .or_else(|| first_nonempty_paragraph(&parsed.system))
            .unwrap_or_else(|| parsed.id.clone()),
        tools: parsed
            .frontmatter
            .get("tools")
            .cloned()
            .unwrap_or_else(|| "all".to_string()),
        hidden: parsed
            .frontmatter
            .get("hidden")
            .map(|v| v == "true")
            .unwrap_or(false),
        max_steps: parsed
            .frontmatter
            .get("steps")
            .and_then(|v| v.parse().ok())
            .unwrap_or(50),
        system: parsed.system.clone(),
        id: parsed.id.clone(),
        path: parsed.path.clone(),
        content: parsed.content.clone(),
    }
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
    let Some((end, closing_len)) = rest
        .find("\n---\n")
        .map(|e| (e, 5usize))
        .or_else(|| rest.find("\r\n---\r\n").map(|e| (e, 7)))
    else {
        return (std::collections::HashMap::new(), content.to_string());
    };
    let frontmatter = &rest[..end];
    let body = rest[end + closing_len..]
        .trim_start_matches(['\r', '\n'])
        .to_string();
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
            tools: "all".to_string(),
            hidden: false,
            max_steps: 50,
            system: BUILTIN_BUILD_SYSTEM.to_string(),
            path: PathBuf::from("builtin/build.md"),
            content: BUILTIN_BUILD_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "plan".to_string(),
            title: "Plan".to_string(),
            description: "Planning agent with read-only tools.".to_string(),
            tools: "read_only".to_string(),
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
            tools: "all".to_string(),
            hidden: true,
            max_steps: 25,
            system: BUILTIN_GENERAL_SYSTEM.to_string(),
            path: PathBuf::from("builtin/general.md"),
            content: BUILTIN_GENERAL_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "explore".to_string(),
            title: "Explore".to_string(),
            description: "Fast agent specialized for exploring codebases. Use this when you need to quickly find files by patterns, search code for keywords, or answer questions about the codebase.".to_string(),
            tools: "read_only".to_string(),
            hidden: true,
            max_steps: 25,
            system: BUILTIN_EXPLORE_SYSTEM.to_string(),
            path: PathBuf::from("builtin/explore.md"),
            content: BUILTIN_EXPLORE_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "compaction".to_string(),
            title: "Compaction".to_string(),
            description: "Anchored context summarization agent.".to_string(),
            tools: "none".to_string(),
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
            tools: "none".to_string(),
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
            tools: "none".to_string(),
            hidden: true,
            max_steps: 5,
            system: BUILTIN_SUMMARY_SYSTEM.to_string(),
            path: PathBuf::from("builtin/summary.md"),
            content: BUILTIN_SUMMARY_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "memory-extract".to_string(),
            title: "Memory Extract".to_string(),
            description: "Background memory extraction agent.".to_string(),
            tools: "[memory_read, memory_record]".to_string(),
            hidden: true,
            max_steps: 15,
            system: BUILTIN_MEMORY_EXTRACT_SYSTEM.to_string(),
            path: PathBuf::from("builtin/memory-extract.md"),
            content: BUILTIN_MEMORY_EXTRACT_SYSTEM.to_string(),
        },
        AgentInfo {
            id: "dreaming".to_string(),
            title: "Dreaming".to_string(),
            description: "Cross-session pattern extraction.".to_string(),
            tools: "[memory_read, memory_record]".to_string(),
            hidden: true,
            max_steps: 20,
            system: BUILTIN_DREAMING_SYSTEM.to_string(),
            path: PathBuf::from("builtin/dreaming.md"),
            content: BUILTIN_DREAMING_SYSTEM.to_string(),
        },
    ]
}

const BUILTIN_BUILD_SYSTEM: &str = "You are an AI coding agent. Help the user accomplish software engineering tasks by inspecting the workspace, making targeted changes, and using tools according to the configured permissions.";

const BUILTIN_DREAMING_SYSTEM: &str = r#"You are a cross-session pattern extraction agent. You receive summaries from ALL sessions of a project and analyze the USER'S behavioral patterns — not project content.

## Your job

Find recurring patterns in how the user works across sessions:
- Language and communication style (e.g. prefers Chinese, terse commands, bullet lists)
- Workflow habits (e.g. always commits after each step, prefers config-driven solutions)
- Tool and technique preferences (e.g. dislikes mocks, prefers iterators over loops)
- Recurring constraints or rules the user enforces

## Rules

1. **Read first**: Call `memory_read(scope=user)` and `memory_read(scope=dreaming)` to see what's already recorded. Do not write duplicates.
2. **Quality over quantity**: Only record patterns that appear across MULTIPLE sessions. Single-session observations are not patterns.
3. **Write to dreaming scope**: Use `memory_record(scope=dreaming)` with tags like `#pattern`, `#style`, `#preference`, `#confirmed`.
4. **No new memory is OK**: If you don't find clear cross-session patterns, say "No new patterns" and stop.
5. **Upgrade confidence**: If an existing `#likely` pattern is strongly reinforced by new evidence, you may re-record it with `#confirmed`.

## What NOT to record

- Individual session content, bugs, or features (that's project memory's job)
- One-time preferences that didn't recur
- Anything already obvious from AGENTS.md or rules files"#;

const BUILTIN_GENERAL_SYSTEM: &str = "You are a general-purpose agent for researching complex questions and executing multi-step tasks.";

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

const BUILTIN_MEMORY_EXTRACT_SYSTEM: &str = r#"You are a background memory extraction agent. Your job is to review recent conversation messages and extract stable conclusions worth remembering.

## Rules

1. **Incremental input**: You receive only the messages since the last extraction, not the full conversation.
2. **Quality over quantity**: Only record stable conclusions, decisions, preferences, or patterns. NEVER record transient state, in-progress steps, single git commands, error traces, or implementation trivia (line numbers, variable names).
3. **Read before write**: Call memory_read first to see what already exists. Do not write duplicates.
4. **Two layers only**:
   - scope=project: progress, TODO, tech, conclusion (categories for the current project)
   - scope=user: preferences, constraints, patterns, style (cross-project user habits)
   - Do NOT write dreaming scope — that is a separate manual operation.
5. **No new memory is OK**: If the conversation contains nothing worth remembering, say "No new memory" and stop. Do not force writes.
6. **Tags**: Use tags to classify confidence and type: #confirmed #likely #decision #architecture #constraint #preference #pattern #style #issue

## What to record

- Technical decision with rationale → scope=project, category=tech
- Work milestone reached → scope=project, category=progress
- Non-transient pending task → scope=project, category=TODO
- Strategic project decision → scope=project, category=conclusion
- User preference seen → scope=user, category=preferences
- Hard constraint stated → scope=user, category=constraints
- Reusable workflow pattern → scope=user, category=patterns
- Style convention → scope=user, category=style

## What NOT to record

- Debug sessions, error traces, temporary workarounds
- Single-use commands, file paths, variable names
- In-progress steps that may change
- Anything already in AGENTS.md or rules (those are authoritative)"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_markdown_agents_from_known_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".openrust").join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("custom.md"), "# Build\nBuild agent.").unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "custom").unwrap();
        assert_eq!(agent.title, "Build");
        assert!(agent.description.contains("Build agent"));
        assert_eq!(agent.system.trim(), "# Build\nBuild agent.");
    }

    #[test]
    fn project_agent_overlays_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".openrust").join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("build.md"),
            "---\ntitle: Custom Build\ntools: read_only\nsteps: 10\n---\n# My Build\nCustom system.",
        )
        .unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "build").unwrap();
        assert_eq!(agent.title, "Custom Build");
        assert_eq!(agent.tools, "read_only");
        assert_eq!(agent.max_steps, 10);
        assert!(agent.system.contains("Custom system"));
    }

    #[test]
    fn overlay_preserves_builtin_fields_when_omitted() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".openrust").join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("build.md"), "New system only.").unwrap();

        let list = load_agents(dir.path()).unwrap();
        let agent = agent_by_id(&list, "build").unwrap();
        assert_eq!(agent.title, "Build");
        assert_eq!(agent.tools, "all");
        assert_eq!(agent.max_steps, 50);
        assert_eq!(agent.system.trim(), "New system only.");
    }

    #[test]
    fn agent_id_is_filename_stem() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".openrust").join("agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("MyAgent.md"), "# My Agent").unwrap();

        let list = load_agents(dir.path()).unwrap();
        assert!(agent_by_id(&list, "MyAgent").is_some());
        assert!(agent_by_id(&list, "myagent").is_none());
    }

    #[test]
    fn parses_frontmatter_title_description_and_system() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".openrust").join("agents");
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
            tools: "all".to_string(),
            hidden: false,
            max_steps: 50,
            system: String::new(),
            path: PathBuf::from(".openrust/agents/build.md"),
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
                tools: "all".to_string(),
                hidden: false,
                max_steps: 50,
                system: String::new(),
                path: PathBuf::from(".openrust/agents/review.md"),
                content: String::new(),
            },
            AgentInfo {
                id: "build".to_string(),
                title: "Build".to_string(),
                description: "Build agent".to_string(),
                tools: "all".to_string(),
                hidden: false,
                max_steps: 50,
                system: String::new(),
                path: PathBuf::from(".openrust/agents/build.md"),
                content: String::new(),
            },
        ];

        assert_eq!(default_agent_id(&agents).as_deref(), Some("build"));
    }

    #[test]
    fn parses_max_steps_from_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".openrust").join("agents");
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
    fn max_steps_defaults_to_50() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join(".openrust").join("agents");
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
