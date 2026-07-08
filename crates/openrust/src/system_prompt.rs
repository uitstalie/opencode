//! System prompt rendering for Phase 1B.

use crate::core::config::{Config, ResolvedProvider};

pub struct SystemPrompt {
    pub mode: String,
    pub provider: String,
    pub model: String,
    pub cwd: String,
    pub home: String,
    pub platform: String,
    pub config_path: String,
    pub agents_md: String,
}

impl SystemPrompt {
    pub fn render(&self) -> String {
        let shell_kind = crate::tool::shell::detect_shell_kind();
        let cwd = std::path::Path::new(&self.cwd);
        let skills = crate::tool::skill::list_skills_for_cwd(cwd);
        let mut sections = Vec::new();
        sections.push(render_section("constraint", &render_constraint()));
        sections.push(render_section(
            "identity",
            &render_identity(&self.provider, &self.model, &self.mode),
        ));
        sections.push(render_section("environment", &render_environment(self)));
        sections.push(render_section(
            "instructions",
            &render_instructions(&self.config_path, &self.agents_md),
        ));
        sections.push(render_section(
            "capabilities",
            &render_capabilities(shell_kind, &skills),
        ));
        sections.push(render_section("style", &render_style()));
        sections.join("\n\n")
    }

    pub fn from_config(
        config: &Config,
        provider: &ResolvedProvider,
        mode: Option<String>,
    ) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .display()
            .to_string();
        let home = crate::core::platform::PlatformPaths::detect()
            .home
            .display()
            .to_string();
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);

        Ok(Self {
            mode: mode
                .or_else(|| config.mode.clone())
                .unwrap_or_else(|| "build".to_string()),
            provider: provider.name.clone(),
            model: config
                .resolve_provider_model()
                .map(|(_, model)| model)
                .ok_or_else(|| anyhow::anyhow!("No model configured"))?,
            cwd,
            home,
            platform,
            config_path: Config::global_config_path().display().to_string(),
            agents_md: load_agents_md(&std::env::current_dir().unwrap_or_default()),
        })
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

fn render_identity(provider: &str, model: &str, mode: &str) -> String {
    [
        format!("provider={provider}"),
        format!("model={model}"),
        String::new(),
        "## Mode".to_string(),
        format!("mode={mode}"),
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

fn render_instructions(path: &str, agents_md: &str) -> String {
    let mut parts = vec![format!("<!-- source: {} -->", path)];

    if !agents_md.trim().is_empty() {
        parts.push("<project-instructions>".to_string());
        parts.push(agents_md.trim().to_string());
        parts.push("</project-instructions>".to_string());
    }

    parts.join("\n\n")
}

/// Load AGENTS.md from the project root.
/// Returns empty string if not found.
fn load_agents_md(cwd: &std::path::Path) -> String {
    let path = cwd.join("AGENTS.md");
    std::fs::read_to_string(&path).unwrap_or_default()
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
    lines.push(
        "- agents are loaded from local markdown directories such as agents/, agent/, and modes/"
            .to_string(),
    );
    lines.push(String::new());
    lines.push("## Tasks".to_string());
    lines.push(
        "- task and todo state are session-scoped and persisted in the session database"
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

    lines.extend(crate::tool::catalog::TOOL_CATALOG.iter().map(|tool| {
        format!(
            "- {} [{}]: {}",
            tool.name,
            tool_category_label(tool.category),
            tool.prompt_hint
        )
    }));

    lines.join("\n")
}

fn tool_category_label(category: crate::tool::catalog::ToolCategory) -> &'static str {
    match category {
        crate::tool::catalog::ToolCategory::Filesystem => "fs",
        crate::tool::catalog::ToolCategory::Shell => "shell",
        crate::tool::catalog::ToolCategory::Network => "net",
        crate::tool::catalog::ToolCategory::Interaction => "interaction",
        crate::tool::catalog::ToolCategory::Undo => "undo",
    }
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
    fn render_keeps_fixed_sections_and_mode_block() {
        let prompt = SystemPrompt {
            mode: "build".to_string(),
            provider: "deepseek".to_string(),
            model: "deepseek/deepseek-v4-pro".to_string(),
            cwd: "/tmp/project".to_string(),
            home: "/home/test".to_string(),
            platform: "linux-x86_64".to_string(),
            config_path: "/home/test/.config/openrust/config.json".to_string(),
            agents_md: String::new(),
        }
        .render();

        assert!(prompt.contains("<constraint>"));
        assert!(prompt.contains("<identity>"));
        assert!(prompt.contains("<environment>"));
        assert!(prompt.contains("<instructions>"));
        assert!(prompt.contains("<capabilities>"));
        assert!(prompt.contains("<style>"));
        assert!(prompt.contains("mode=build"));
    }

    #[test]
    fn from_config_uses_explicit_config_model() {
        let config = Config {
            model: Some("deepseek/deepseek-v4-pro".to_string()),
            mode: None,
            provider: std::collections::HashMap::from([(
                "deepseek".to_string(),
                crate::core::config::ProviderConfig {
                    api_key: None,
                    base_url: None,
                    models: std::collections::HashMap::from([(
                        "deepseek-v4-pro".to_string(),
                        crate::core::config::ModelConfig {
                            name: Some("deepseek-v4-pro".to_string()),
                            variants: None,
                            limit: None,
                            options: None,
                        },
                    )]),
                    options: None,
                },
            )]),
        };
        let provider = ResolvedProvider {
            name: "deepseek".to_string(),
            api_key: None,
            base_url: None,
            models: Default::default(),
            options: None,
        };

        let prompt =
            SystemPrompt::from_config(&config, &provider, Some("plan".to_string())).unwrap();

        assert_eq!(prompt.mode, "plan");
        assert_eq!(prompt.provider, "deepseek");
        assert_eq!(prompt.model, "deepseek-v4-pro");
    }

    #[test]
    fn from_config_fails_without_model() {
        let config = Config::default();
        let provider = ResolvedProvider {
            name: "deepseek".to_string(),
            api_key: None,
            base_url: None,
            models: Default::default(),
            options: None,
        };

        assert!(SystemPrompt::from_config(&config, &provider, Some("plan".to_string())).is_err());
    }

    #[test]
    fn instructions_empty_when_no_agents_md() {
        let rendered = render_instructions("/path/to/config.json", "");
        assert!(rendered.contains("source:"));
        assert!(!rendered.contains("<project-instructions>"));
    }

    #[test]
    fn instructions_includes_agents_md() {
        let rendered = render_instructions(
            "/path/to/config.json",
            "# My Project\nBuild with cargo.\n",
        );
        assert!(rendered.contains("<project-instructions>"));
        assert!(rendered.contains("Build with cargo."));
        assert!(rendered.contains("</project-instructions>"));
    }
}
