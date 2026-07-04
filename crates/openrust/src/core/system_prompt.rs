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
}

impl SystemPrompt {
    pub fn render(&self) -> String {
        let mut sections = Vec::new();
        sections.push(render_section("constraint", &render_constraint()));
        sections.push(render_section("identity", &render_identity(&self.provider, &self.model, &self.mode)));
        sections.push(render_section("environment", &render_environment(self)));
        sections.push(render_section("instructions", &render_instructions(&self.config_path)));
        sections.push(render_section("capabilities", &render_capabilities()));
        sections.push(render_section("style", &render_style()));
        sections.push(render_section("memory", &render_memory()));
        sections.push(render_section("nudge", "[CONSTRAINT NUDGE] memory_read(决策前) → compact_check"));
        sections.join("\n\n")
    }

    pub fn from_config(config: &Config, provider: &ResolvedProvider, mode: Option<String>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")).display().to_string();
        let home = crate::core::platform::PlatformPaths::detect()
            .home
            .display()
            .to_string();
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);

        Self {
            mode: mode.or_else(|| config.mode.clone()).unwrap_or_else(|| "build".to_string()),
            provider: provider.name.clone(),
            model: config.resolve_model().unwrap_or_else(|| "gpt-5.5".to_string()),
            cwd,
            home,
            platform,
            config_path: Config::global_config_path().display().to_string(),
        }
    }
}

fn render_section(name: &str, content: &str) -> String {
    format!("<{name}>\n{content}\n</{name}>")
}

fn render_constraint() -> String {
    [
        "## Tool Invocation Rules",
        "- [Memory] memory_record and memory_review must stay ordered.",
        "- [Compact] honor compact_check when requested.",
        "",
        "## Concrete Discipline",
        "- Keep changes scoped to the requested task.",
        "- Do not overwrite unrelated work.",
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

fn render_instructions(path: &str) -> String {
    [
        format!("<!-- source: {} -->", path),
        "使用简体中文回复；术语、命令、路径、变量名和代码标识符可保留英文。".to_string(),
        "修改源码后需编译、替换二进制并重启。".to_string(),
    ]
    .join("\n\n")
}

fn render_capabilities() -> String {
    [
        "## Skills".to_string(),
        "- customize-opencode: opencode 自身配置参考".to_string(),
        "- write-skills: SKILL.md 格式与陷阱".to_string(),
        String::new(),
        "## Tools".to_string(),
        "- debug: provider / tool / config / vault / session / prompt".to_string(),
    ]
    .join("\n")
}

fn render_style() -> String {
    [
        "## Role".to_string(),
        "- 直接给结论，再给必要依据。".to_string(),
        "- 不做无关重构。".to_string(),
    ]
    .join("\n")
}

fn render_memory() -> String {
    [
        "## Dreaming Guide".to_string(),
        "- memory_record 写入前先 memory_review。".to_string(),
        "- 避免记录流水账和临时调试过程。".to_string(),
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
        }
        .render();

        assert!(prompt.contains("<constraint>"));
        assert!(prompt.contains("<identity>"));
        assert!(prompt.contains("<environment>"));
        assert!(prompt.contains("<instructions>"));
        assert!(prompt.contains("<capabilities>"));
        assert!(prompt.contains("<style>"));
        assert!(prompt.contains("<memory>"));
        assert!(prompt.contains("<nudge>"));
        assert!(prompt.contains("mode=build"));
        assert!(prompt.contains("[CONSTRAINT NUDGE] memory_read(决策前) → compact_check"));
    }

    #[test]
    fn from_config_uses_config_mode_and_default_model() {
        let config = Config::default();
        let provider = ResolvedProvider {
            name: "deepseek".to_string(),
            api_key: None,
            base_url: None,
            models: Default::default(),
            options: None,
        };

        let prompt = SystemPrompt::from_config(&config, &provider, Some("plan".to_string()));

        assert_eq!(prompt.mode, "plan");
        assert_eq!(prompt.provider, "deepseek");
        assert_eq!(prompt.model, "gpt-5.5");
    }
}
