//! Tool catalog — central metadata registry for tool abstractions.

use std::sync::Arc;

use super::{
    apply_patch::ApplyPatchTool, bash::BashTool, edit::EditTool, glob::GlobTool, grep::GrepTool,
    question::QuestionTool, read::ReadTool, rm::RmTool, skill::SkillTool, task::TaskTool,
    todowrite::TodoWriteTool, undo::UndoStore, undo_edit::UndoEditTool, webfetch::WebFetchTool,
    websearch::WebSearchTool, write::WriteTool,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    Filesystem,
    Shell,
    Network,
    Interaction,
    Undo,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolMeta {
    pub name: &'static str,
    pub category: ToolCategory,
    pub description: &'static str,
    pub prompt_hint: &'static str,
}

pub const TOOL_CATALOG: &[ToolMeta] = &[
    ToolMeta {
        name: "read",
        category: ToolCategory::Filesystem,
        description: "Read a file or a slice of it, with line numbers.",
        prompt_hint: "Use offset/limit to read only the part you need.",
    },
    ToolMeta {
        name: "write",
        category: ToolCategory::Filesystem,
        description: "Write a file to the filesystem.",
        prompt_hint: "Use for creating new content or replacing file contents.",
    },
    ToolMeta {
        name: "edit",
        category: ToolCategory::Filesystem,
        description: "Edit a file by replacing exact text.",
        prompt_hint: "Use for surgical replacements.",
    },
    ToolMeta {
        name: "apply_patch",
        category: ToolCategory::Filesystem,
        description: "Apply an add/update/delete patch across files.",
        prompt_hint: "Use for multi-file or multi-hunk edits in one call.",
    },
    ToolMeta {
        name: "rm",
        category: ToolCategory::Filesystem,
        description: "Delete a file or directory.",
        prompt_hint: "Use for safe deletion only.",
    },
    ToolMeta {
        name: "bash",
        category: ToolCategory::Shell,
        description: "Execute shell commands.",
        prompt_hint: "Use for git, build, and system operations.",
    },
    ToolMeta {
        name: "glob",
        category: ToolCategory::Filesystem,
        description: "Find files by glob pattern.",
        prompt_hint: "Use for path discovery.",
    },
    ToolMeta {
        name: "grep",
        category: ToolCategory::Filesystem,
        description: "Search file contents by regex.",
        prompt_hint: "Use for code and text search.",
    },
    ToolMeta {
        name: "webfetch",
        category: ToolCategory::Network,
        description: "Fetch content from a URL.",
        prompt_hint: "Use for reading remote pages directly.",
    },
    ToolMeta {
        name: "websearch",
        category: ToolCategory::Network,
        description: "Search the web.",
        prompt_hint: "Use for discovery and quick lookup.",
    },
    ToolMeta {
        name: "todowrite",
        category: ToolCategory::Interaction,
        description: "Maintain the session todo list.",
        prompt_hint: "State machine: create list → mark first in_progress → complete → advance.",
    },
    ToolMeta {
        name: "skill",
        category: ToolCategory::Interaction,
        description: "Load a specialized skill by name.",
        prompt_hint: "Use when a task matches an available skill.",
    },
    ToolMeta {
        name: "question",
        category: ToolCategory::Interaction,
        description: "Ask the user structured questions.",
        prompt_hint: "Use to gather preferences or resolve ambiguity.",
    },
    ToolMeta {
        name: "task",
        category: ToolCategory::Interaction,
        description: "Delegate a task to a sub-agent.",
        prompt_hint: "Use for autonomous multi-step research or work.",
    },
    ToolMeta {
        name: "undo_edit",
        category: ToolCategory::Undo,
        description: "Restore a file from an undo blob.",
        prompt_hint: "Use after write/edit when rollback is needed.",
    },
];

pub fn tool_meta(name: &str) -> Option<&'static ToolMeta> {
    TOOL_CATALOG.iter().find(|meta| meta.name == name)
}

pub fn tool_names() -> Vec<&'static str> {
    TOOL_CATALOG.iter().map(|meta| meta.name).collect()
}

pub fn prompt_hints() -> Vec<&'static str> {
    TOOL_CATALOG.iter().map(|meta| meta.prompt_hint).collect()
}

pub fn registry_category_names() -> Vec<(ToolCategory, Vec<&'static str>)> {
    [
        ToolCategory::Filesystem,
        ToolCategory::Shell,
        ToolCategory::Network,
        ToolCategory::Interaction,
        ToolCategory::Undo,
    ]
    .into_iter()
    .map(|category| {
        (
            category,
            TOOL_CATALOG
                .iter()
                .filter(|meta| meta.category == category)
                .map(|meta| meta.name)
                .collect(),
        )
    })
    .collect()
}

pub fn create_tool(name: &str, undo_store: Option<Arc<UndoStore>>) -> Option<Box<dyn super::Tool>> {
    match name {
        "read" => Some(Box::new(ReadTool)),
        "write" => Some(Box::new(WriteTool)),
        "edit" => Some(Box::new(EditTool)),
        "apply_patch" => Some(Box::new(ApplyPatchTool)),
        "rm" => Some(Box::new(RmTool)),
        "bash" => Some(Box::new(BashTool)),
        "glob" => Some(Box::new(GlobTool)),
        "grep" => Some(Box::new(GrepTool)),
        "webfetch" => Some(Box::new(WebFetchTool)),
        "websearch" => Some(Box::new(WebSearchTool)),
        "todowrite" => Some(Box::new(TodoWriteTool)),
        "skill" => Some(Box::new(SkillTool)),
        "question" => Some(Box::new(QuestionTool)),
        "task" => Some(Box::new(TaskTool)),
        "undo_edit" => Some(Box::new(UndoEditTool { undo_store })),
        _ => None,
    }
}

const SUBTASK_EXCLUDE: &[&str] = &["task", "question", "todowrite"];

/// Resolve a frontmatter `tools` spec into concrete tool metadata.
///
/// `spec` accepts:
/// - empty or `"all"` — every tool
/// - `"none"` — no tools
/// - a builtin preset: `read_only`, `no_write`, `no_internet`
/// - a custom preset from config `presets` (overrides builtins on key clash)
/// - an explicit bracketed list: `[read, grep, bash]`
/// - a single tool name
///
/// When `is_subagent` is true, `task` and `question` are always excluded.
pub fn resolve_tool_names(
    spec: &str,
    custom_presets: &std::collections::HashMap<String, Vec<String>>,
    is_subagent: bool,
) -> Vec<&'static ToolMeta> {
    resolve_preset_names(spec, custom_presets)
        .into_iter()
        .filter_map(tool_meta)
        .filter(|meta| !(is_subagent && SUBTASK_EXCLUDE.contains(&meta.name)))
        .collect()
}

fn resolve_preset_names(
    spec: &str,
    custom_presets: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<&'static str> {
    let trimmed = spec.trim();

    // Explicit list: [read, grep, bash]
    if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return inner
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\''))
            .filter_map(|name| tool_meta(name).map(|meta| meta.name))
            .collect();
    }

    let all_names: Vec<&'static str> = TOOL_CATALOG.iter().map(|meta| meta.name).collect();
    let exclude = |drop: &[&str]| -> Vec<&'static str> {
        all_names.iter().filter(|n| !drop.contains(n)).copied().collect()
    };

    // Config preset overlays builtins.
    if let Some(list) = custom_presets.get(trimmed) {
        return list
            .iter()
            .filter_map(|name| tool_meta(name.as_str()).map(|meta| meta.name))
            .collect();
    }

    match trimmed {
        "" | "all" => all_names,
        "none" => Vec::new(),
        "read_only" => exclude(&["write", "edit", "apply_patch", "rm", "bash"]),
        "no_write" => exclude(&["write", "edit", "apply_patch", "rm"]),
        "no_internet" => exclude(&["webfetch", "websearch"]),
        other => tool_meta(other).map(|meta| vec![meta.name]).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn tool_names_cover_all_catalog_entries() {
        let names = tool_names();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"undo_edit"));
    }

    #[test]
    fn prompt_hints_are_available_for_all_tools() {
        let hints = prompt_hints();
        assert_eq!(hints.len(), TOOL_CATALOG.len());
        assert!(hints.iter().all(|hint| !hint.is_empty()));
    }

    #[test]
    fn catalog_groups_tools_by_category() {
        let groups = registry_category_names();
        assert!(
            groups
                .iter()
                .any(|(category, names)| *category == ToolCategory::Filesystem
                    && names.contains(&"read"))
        );
        assert!(
            groups.iter().any(
                |(category, names)| *category == ToolCategory::Shell && names.contains(&"bash")
            )
        );
    }

    #[test]
    fn read_only_strips_write_and_shell_tools() {
        let tools = resolve_tool_names("read_only", &HashMap::new(), false);
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert!(!names.contains(&"write"));
        assert!(!names.contains(&"edit"));
        assert!(!names.contains(&"bash"));
        assert!(names.contains(&"read"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"webfetch"));
    }

    #[test]
    fn all_preset_keeps_write_tools() {
        let tools = resolve_tool_names("all", &HashMap::new(), false);
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert!(names.contains(&"write"));
        assert!(names.contains(&"read"));
    }

    #[test]
    fn empty_spec_defaults_to_all() {
        let tools = resolve_tool_names("", &HashMap::new(), false);
        assert_eq!(tools.len(), TOOL_CATALOG.len());
    }

    #[test]
    fn none_preset_is_empty() {
        let tools = resolve_tool_names("none", &HashMap::new(), false);
        assert!(tools.is_empty());
    }

    #[test]
    fn explicit_list_resolves_named_tools() {
        let tools = resolve_tool_names("[read, grep, bash]", &HashMap::new(), false);
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["read", "grep", "bash"]);
    }

    #[test]
    fn custom_preset_defines_new_set() {
        let mut presets = HashMap::new();
        presets.insert(
            "research".to_string(),
            vec!["read".to_string(), "grep".to_string()],
        );
        let tools = resolve_tool_names("research", &presets, false);
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["read", "grep"]);
    }

    #[test]
    fn custom_preset_overrides_builtin() {
        let mut presets = HashMap::new();
        presets.insert("read_only".to_string(), vec!["read".to_string()]);
        let tools = resolve_tool_names("read_only", &presets, false);
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["read"]);
    }

    #[test]
    fn subagent_excludes_task_and_question() {
        let tools = resolve_tool_names("all", &HashMap::new(), true);
        let names: Vec<&str> = tools.iter().map(|t| t.name).collect();
        assert!(!names.contains(&"task"));
        assert!(!names.contains(&"question"));
        assert!(!names.contains(&"todowrite"));
        assert!(names.contains(&"read"));
    }
}
