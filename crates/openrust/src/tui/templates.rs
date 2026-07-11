//! Prompt templates for slash commands.

/// `/init` — project onboarding template.
///
/// Sent as a user message to guide the model through checking the project,
/// investigating the codebase, reporting findings, and scaffolding docs.
pub(super) fn init_template(args: &str) -> String {
    let focus = if args.is_empty() {
        String::new()
    } else {
        format!("\nUser focus: {args}")
    };

    format!(
        r#"Execute the following project initialization immediately. Do not describe or repeat these instructions — start Phase 1 now.{focus}

## Phase 1: Check

Inspect project root. Report for each:
- `AGENTS.md` (required)
- `.openrust/config.json` or `.openrust/config.jsonc` (required)
- `.openrust/memory/progress.md`, `.openrust/memory/TODO.md` (required)
- `.openrust/memory/tech.md`, `.openrust/memory/conclusion.md` (recommended)
- `.openrust/rules/`, `.openrust/agents/`, `.openrust/skills/` (optional)

Detect tech stack from root markers (lightweight): Rust (`Cargo.toml`, workspaces), Node (`package.json`, workspaces), TypeScript (`tsconfig.json`), Python (`pyproject.toml`), etc.

## Phase 2: Investigate

Read highest-value sources first: `README*`, root manifests, lockfile, build/lint/test config, CI, existing `AGENTS.md`, `.cursor/rules/`, `CLAUDE.md`. Trust executable config over prose.

Extract: exact dev commands, required command order, workspace boundaries, framework quirks (codegen, build artifacts), test quirks (fixtures, flaky suites), conventions that differ from defaults.

## Phase 3: Report & Recommend

Report all findings to the user. Include:
- Missing required files (checklist format)
- Detected tech stack
- Recommended rules to add (at minimum project-specific guidelines)

## Phase 4: Scaffold

Ask user to confirm before writing each file. Never overwrite existing files.

- `AGENTS.md`: repo-specific guidance only. Dev commands, architecture, quirks, conventions, test gotchas. Exclude generic advice, tutorials, speculation. If exists, improve in place.
- `.openrust/config.json`: minimal project config if needed.
- `.openrust/memory/progress.md`: current phase and recent milestones
- `.openrust/memory/TODO.md`: immediate tasks (high/medium/low)
- `.openrust/memory/tech.md`: stack summary + key commands (if warranted)
- `.openrust/memory/conclusion.md`: stable design decisions (if any)

## Constraints

- Check first, report, then ask before writing
- Memory: prefer empty over speculative
- Only ask questions the repo cannot answer"#
    )
}
