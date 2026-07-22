//! System prompt rendering.

use crate::core::config::{Config, ResolvedProvider};

pub struct SystemPrompt {
    pub provider: String,
    pub model: String,
    pub cwd: String,
    pub home: String,
    pub platform: String,
    pub config_path: String,
    pub global_rules: String,
    pub agents_md: String,
    pub project_rules: String,
    shell_kind: crate::tool::shell::ShellKind,
    skills: Vec<crate::tool::skill::SkillEntry>,
}

impl SystemPrompt {
    pub fn render(&self) -> String {
        let sections = [
            render_section("constraint", &render_constraint()),
            render_section("identity", &render_identity(&self.provider, &self.model)),
            render_section("environment", &render_environment(self)),
            render_section(
                "instructions",
                &render_instructions(
                    &self.config_path,
                    &self.global_rules,
                    &self.agents_md,
                    &self.project_rules,
                    &self.cwd,
                ),
            ),
            render_section("memory", &render_memory()),
            render_section("capabilities", &render_capabilities(self.shell_kind, &self.skills)),
            render_section("style", &render_style()),
        ];
        sections.join("\n\n")
    }

    pub fn from_config(
        config: &Config,
        provider: &ResolvedProvider,
    ) -> anyhow::Result<Self> {
        let cwd_path = std::env::current_dir().unwrap_or_default();
        let cwd = cwd_path.display().to_string();
        let home = crate::core::platform::PlatformPaths::detect()
            .home
            .display()
            .to_string();
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let shell_kind = crate::tool::shell::detect_shell_kind();
        let skills = crate::tool::skill::list_skills_for_cwd(&cwd_path);

        Ok(Self {
            provider: provider.name.clone(),
            model: config
                .resolve_provider_model()
                .map(|(_, model)| model)
                .ok_or_else(|| anyhow::anyhow!("No model configured"))?,
            cwd,
            home,
            platform,
            config_path: Config::global_config_path().display().to_string(),
            global_rules: load_global_rules(),
            agents_md: load_agents_md(&cwd_path),
            project_rules: load_project_rules(&cwd_path),
            shell_kind,
            skills,
        })
    }

    pub fn fallback(provider: &str, model: &str) -> Self {
        let cwd_path = std::env::current_dir().unwrap_or_default();
        let cwd = cwd_path.display().to_string();
        let home = crate::core::platform::PlatformPaths::detect()
            .home
            .display()
            .to_string();
        let shell_kind = crate::tool::shell::detect_shell_kind();
        let skills = crate::tool::skill::list_skills_for_cwd(&cwd_path);
        Self {
            provider: provider.to_string(),
            model: model.to_string(),
            cwd,
            home,
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            config_path: Config::global_config_path().display().to_string(),
            global_rules: load_global_rules(),
            agents_md: load_agents_md(&cwd_path),
            project_rules: load_project_rules(&cwd_path),
            shell_kind,
            skills,
        }
    }
}

fn render_section(name: &str, content: &str) -> String {
    format!("<{name}>\n{content}\n</{name}>")
}

fn render_constraint() -> String {
    [
        "## Concrete Discipline",
        "- Keep changes scoped to the requested task.",
        "- Do not overwrite unrelated work.",
        "- Do not preemptively add documentation for changes.",
    ]
    .join("\n")
}

fn render_identity(provider: &str, model: &str) -> String {
    [
        format!("provider={provider}"),
        format!("model={model}"),
    ]
    .join("\n")
}

fn render_environment(prompt: &SystemPrompt) -> String {
    [
        format!("model: {}", prompt.model),
        format!("provider: {}", prompt.provider),
        format!("cwd: {}", prompt.cwd),
        format!("home: {}", prompt.home),
        format!("platform: {}", prompt.platform),
    ]
    .join("\n")
}

/// Static `<memory>` index — only locations, no content. The agent uses
/// `memory_read` to retrieve actual entries on demand.
fn render_memory() -> String {
    [
        "Memory locations (use memory_read to query):",
        "- project: .openrust/memory/*.md (scope=project)",
        "- user: ~/.config/openrust/memory/*.md (scope=user)",
        "- dreaming: ~/.config/openrust/memory/dreaming/ (scope=dreaming)",
    ]
    .join("\n")
}

fn render_instructions(
    path: &str,
    global_rules: &str,
    agents_md: &str,
    project_rules: &str,
    cwd: &str,
) -> String {
    let mut parts = vec![format!("<!-- source: {} -->", path)];

    // Onboarding hint: if AGENTS.md or .openrust/ is missing, nudge the user.
    let has_agents_md = !agents_md.trim().is_empty();
    let has_openrust_dir = std::path::Path::new(cwd).join(".openrust").exists();
    if !has_agents_md || !has_openrust_dir {
        let missing = match (!has_agents_md, !has_openrust_dir) {
            (true, true) => "AGENTS.md and .openrust/",
            (true, false) => "AGENTS.md",
            (false, true) => ".openrust/",
            _ => "",
        };
        parts.push(format!(
            "[onboarding] Project is missing {missing}. \
             Suggest the user run /init to complete project setup."
        ));
    }

    if !global_rules.trim().is_empty() {
        parts.push("<global-rules>".to_string());
        parts.push(global_rules.trim().to_string());
        parts.push("</global-rules>".to_string());
    }

    if !agents_md.trim().is_empty() {
        parts.push("<project-instructions>".to_string());
        parts.push(agents_md.trim().to_string());
        parts.push("</project-instructions>".to_string());
    }

    if !project_rules.trim().is_empty() {
        parts.push("<project-rules>".to_string());
        parts.push(project_rules.trim().to_string());
        parts.push("</project-rules>".to_string());
    }

    parts.join("\n\n")
}

/// Load AGENTS.md from the project root.
fn load_agents_md(cwd: &std::path::Path) -> String {
    let path = cwd.join("AGENTS.md");
    std::fs::read_to_string(&path).unwrap_or_default()
}

/// Load global rules from all `*.md` files in `~/.config/openrust/rules/`.
/// Files are sorted alphabetically for deterministic ordering.
fn load_global_rules() -> String {
    let dir = crate::core::platform::PlatformPaths::detect()
        .config_dir()
        .join("rules");
    load_rules_dir(&dir)
}

/// Load project rules from all `*.md` files in `.openrust/rules/`.
fn load_project_rules(cwd: &std::path::Path) -> String {
    let dir = cwd.join(".openrust").join("rules");
    load_rules_dir(&dir)
}

/// Read all `*.md` files from `dir`, sorted by filename, concatenated.
fn load_rules_dir(dir: &std::path::Path) -> String {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return String::new();
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter(|e| {
            e.path().extension().is_some_and(|ext| ext == "md")
        })
        .collect();
    files.sort_by_key(|e| e.file_name());

    let mut parts = Vec::new();
    for entry in &files {
        if let Ok(content) = std::fs::read_to_string(entry.path())
            && !content.trim().is_empty() {
                parts.push(content.trim().to_string());
            }
    }
    parts.join("\n\n")
}

fn render_capabilities(
    shell_kind: crate::tool::shell::ShellKind,
    skills: &[crate::tool::skill::SkillEntry],
) -> String {
    let mut lines = Vec::new();

    if !skills.is_empty() {
        lines.push("## Skills".to_string());
        for skill in skills {
            if skill.description.is_empty() {
                lines.push(format!("- {}", skill.name));
            } else {
                lines.push(format!("- {}: {}", skill.name, skill.description));
            }
        }
        lines.push(String::new());
    }

    lines.push("## Agents".to_string());
    lines.push("- agents are loaded from .openrust/agents/*.md".to_string());
    lines.push(String::new());
    lines.push("## Tasks".to_string());
    lines.push(
        "- todo list is a state machine: pending → in_progress → completed"
            .to_string(),
    );
    lines.push("- create a todo list BEFORE starting multi-step work".to_string());
    lines.push(
        "- keep exactly ONE item in_progress; mark it completed before advancing"
            .to_string(),
    );
    lines.push(
        "- current todo state is injected after each tool batch — follow it"
            .to_string(),
    );
    lines.push(String::new());
    lines.push("## Tools".to_string());
    lines.push(
        format!(
            "- shell: {}",
            crate::tool::shell::shell_environment_hint(shell_kind)
        ),
    );

    lines.join("\n")
}

    fn render_style() -> String {
    [
        "## Role".to_string(),
        "- 直接给结论，再给必要依据。".to_string(),
        "- 不做无关重构。".to_string(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_keeps_fixed_sections() {
        let prompt = SystemPrompt {
            provider: "deepseek".to_string(),
            model: "deepseek/deepseek-v4-pro".to_string(),
            cwd: "/tmp/project".to_string(),
            home: "/home/test".to_string(),
            platform: "linux-x86_64".to_string(),
            config_path: "/home/test/.config/openrust/config.json".to_string(),
            global_rules: String::new(),
            agents_md: String::new(),
            project_rules: String::new(),
            shell_kind: crate::tool::shell::ShellKind::Bash,
            skills: vec![],
        }
        .render();

        assert!(prompt.contains("<constraint>"));
        assert!(prompt.contains("<identity>"));
        assert!(prompt.contains("<environment>"));
        assert!(prompt.contains("<instructions>"));
        assert!(prompt.contains("<capabilities>"));
        assert!(prompt.contains("<style>"));
    }

    #[test]
    fn from_config_uses_explicit_config_model() {
        let config = Config {
            log_level: None,
            model: Some("deepseek/deepseek-v4-pro".to_string()),
            theme: None,
            search_engine: None,
            persist_agent_sessions: true,
            background_model: None,
            provider: std::collections::HashMap::from([(
                "deepseek".to_string(),
                crate::core::config::ProviderConfig {
                    api_key: None,
                    base_url: None,
                    models: std::collections::HashMap::from([(
                        "deepseek-v4-pro".to_string(),
                        crate::core::config::ModelConfig {
                            name: Some("deepseek-v4-pro".to_string()),
                            ..Default::default()
                        },
                    )]),
                    ..Default::default()
                },
            )]),
            presets: std::collections::HashMap::new(),
            user_providers: Default::default(),
        };
        let provider = ResolvedProvider {
            name: "deepseek".to_string(),
            ..Default::default()
        };

        let prompt =
            SystemPrompt::from_config(&config, &provider).unwrap();

        assert_eq!(prompt.provider, "deepseek");
        assert_eq!(prompt.model, "deepseek-v4-pro");
    }

    #[test]
    fn from_config_fails_without_model() {
        let config = Config::default();
        let provider = ResolvedProvider {
            name: "deepseek".to_string(),
            ..Default::default()
        };

        assert!(SystemPrompt::from_config(&config, &provider).is_err());
    }

    #[test]
    fn instructions_empty_when_no_content() {
        let rendered = render_instructions("/path/to/config.json", "", "", "", "/nonexistent");
        assert!(rendered.contains("source:"));
        assert!(rendered.contains("[onboarding]"));
        assert!(!rendered.contains("<global-rules>"));
        assert!(!rendered.contains("<project-instructions>"));
        assert!(!rendered.contains("<project-rules>"));
    }

    #[test]
    fn instructions_includes_agents_md() {
        let rendered = render_instructions(
            "/path/to/config.json",
            "",
            "# My Project\nBuild with cargo.\n",
            "",
            "/nonexistent",
        );
        assert!(rendered.contains("<project-instructions>"));
        assert!(rendered.contains("Build with cargo."));
        assert!(rendered.contains("</project-instructions>"));
    }

    #[test]
    fn instructions_includes_global_rules() {
        let rendered = render_instructions(
            "/path/to/config.json",
            "Always use tabs.\nNever commit secrets.",
            "",
            "",
            "/nonexistent",
        );
        assert!(rendered.contains("<global-rules>"));
        assert!(rendered.contains("Always use tabs."));
        assert!(rendered.contains("</global-rules>"));
    }

    #[test]
    fn instructions_includes_project_rules() {
        let rendered = render_instructions(
            "/path/to/config.json",
            "",
            "",
            "Use 4-space indent.\nPrefer iterators.",
            "/nonexistent",
        );
        assert!(rendered.contains("<project-rules>"));
        assert!(rendered.contains("Use 4-space indent."));
        assert!(rendered.contains("</project-rules>"));
    }

    #[test]
    fn instructions_layers_in_priority_order() {
        let rendered = render_instructions(
            "/path/to/config.json",
            "global rule",
            "agents md",
            "project rule",
            "/nonexistent",
        );
        let global_pos = rendered.find("<global-rules>").unwrap();
        let agents_pos = rendered.find("<project-instructions>").unwrap();
        let project_pos = rendered.find("<project-rules>").unwrap();
        assert!(global_pos < agents_pos);
        assert!(agents_pos < project_pos);
    }

    #[test]
    fn load_rules_dir_concatenates_sorted_md_files() {
        let dir = std::env::temp_dir().join(format!(
            "openrust-rules-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b_second.md"), "Second rule").unwrap();
        std::fs::write(dir.join("a_first.md"), "First rule").unwrap();
        std::fs::write(dir.join("c_ignore.txt"), "Not markdown").unwrap();

        let content = load_rules_dir(&dir);
        std::fs::remove_dir_all(&dir).ok();

        assert!(content.contains("First rule"));
        assert!(content.contains("Second rule"));
        assert!(!content.contains("Not markdown"));
        let first_pos = content.find("First rule").unwrap();
        let second_pos = content.find("Second rule").unwrap();
        assert!(first_pos < second_pos);
    }

    #[test]
    fn load_rules_dir_empty_when_no_dir() {
        let content = load_rules_dir(std::path::Path::new("/nonexistent/rules/dir"));
        assert!(content.is_empty());
    }
}
