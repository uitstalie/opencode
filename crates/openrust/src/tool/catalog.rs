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
        prompt_hint: "Use to plan and track multi-step work.",
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

const WRITE_TOOLS: &[&str] = &["write", "edit", "rm", "apply_patch", "bash", "undo_edit"];
const SUBTASK_EXCLUDE: &[&str] = &["task", "question"];

/// Returns the tool names that should be available for a given agent mode and
/// subagent context. Plan mode strips write tools; subagents strip task/question.
pub fn tools_for_mode(mode: &str, is_subagent: bool) -> Vec<&'static ToolMeta> {
    let is_plan = mode == "plan" || mode == "explore";
    TOOL_CATALOG
        .iter()
        .filter(|meta| {
            if is_plan && WRITE_TOOLS.contains(&meta.name) {
                return false;
            }
            if is_subagent && SUBTASK_EXCLUDE.contains(&meta.name) {
                return false;
            }
            true
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
