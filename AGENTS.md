# openrust — Rust AI Coding Agent

单一 Rust 二进制 `openrust`，零 TS/Node/Bun 依赖，无 gRPC sidecar。

- **默认分支**：`dev`（当前 TUI 工作分支：`rust`，镜像于 `origin/rust`）
- **源码**：`crates/openrust/`
- 修改源码后需 编译 → 替换二进制 → 重启 openrust

---

## Deploy & Update (AI Operations Guide)

### Platform

- Cross-platform: Linux (x64) and Windows (x64)
- 路径由 `core::platform::PlatformPaths` 管理：

| Scope | Linux | Windows |
|-------|-------|---------|
| config | `~/.config/openrust/` | `%APPDATA%\openrust\` |
| data | `~/.local/share/openrust/` | `%LOCALAPPDATA%\openrust\` |
| cache | `~/.cache/openrust/` | `%LOCALAPPDATA%\openrust\cache\` |
| binary | `~/.local/share/openrust/bin/openrust` | `%LOCALAPPDATA%\openrust\bin\openrust.exe` |

- Build tool: `cargo`（在 `crates/openrust/`）
- Binary name: `openrust`

### Quick Build (current platform only)

```bash
cd crates/openrust && cargo build --release
```

Output: `crates/openrust/target/release/openrust`

### First-time Setup (one-time)

```bash
mkdir -p ~/.local/share/openrust/bin ~/.local/bin && \
  cp crates/openrust/target/release/openrust ~/.local/share/openrust/bin/openrust && \
  ln -sf ~/.local/share/openrust/bin/openrust ~/.local/bin/openrust
```

`~/.local/bin` must be in `PATH`. Verify: `which openrust` should print `~/.local/bin/openrust`.

### Quick Build + Replace (compile & hot-swap)

```bash
cd crates/openrust && cargo build --release && \
  cp ~/.local/share/openrust/bin/openrust ~/.local/share/openrust/bin/openrust.bak && \
  cp target/release/openrust ~/.local/share/openrust/bin/openrust
```

On Windows, use `copy` instead of `cp`.

After replacement, the user must **restart openrust** for the new binary to take effect.

⚠️ **Always backup before replacing**. If the new binary fails, restore from `.bak`.

### Full Update Workflow

When user says "update openrust" or "pull latest and rebuild":

1. `git pull origin <branch>`
2. `cd crates/openrust && cargo build --release`
3. `cp ~/.local/share/openrust/bin/openrust ~/.local/share/openrust/bin/openrust.bak`
4. `cp crates/openrust/target/release/openrust ~/.local/share/openrust/bin/openrust`
5. Tell user to restart openrust

Symlink (`~/.local/bin/openrust`) is created once during first-time setup and does not need updating.

### Build Flags

| Flag | Effect |
|------|--------|
| `--release` | Optimized production build (LTO + codegen-units=1) |
| `--target <triple>` | Cross-compile for a specific target |
| `--no-default-features` | Build without default features (if added later) |

### Troubleshooting

- **build fails**: `cd crates/openrust && cargo check`
- **clippy warnings**: `cd crates/openrust && cargo clippy` and fix before committing.
- **binary doesn't start**: `~/.local/share/openrust/bin/openrust --version`, verify arch (`uname -m` should be x86_64 on Linux).

---

## Branch Names

Use a short branch name of at most three words, separated by hyphens. Do not use slashes or type prefixes such as `feat/` or `fix/`.

Examples: `session-recovery`, `fix-scroll-state`, `provider-auth`.

## Commits and PR Titles

Use conventional commit-style messages and PR titles: `type(scope): summary`.

Valid types are `feat`, `fix`, `docs`, `chore`, `refactor`, and `test`. Scopes are optional; use the affected Rust module or area when helpful, e.g. `core`, `tui`, `cli`, `tool`, `provider`, or `config`.

Examples: `fix(tui): simplify thinking toggle rendering`, `docs: update build guide`, `chore(openrust): bump deps`.

## Style Guide

### General Principles

- Keep things in one function unless composable or reusable.
- Do not extract single-use helpers preemptively. Inline the logic at the call site unless the helper is reused, hides a genuinely complex boundary, or has a clear independent name that improves the caller.
- Prefer `Result` propagation (`?`) over `unwrap()`/`expect()`/`panic!` in non-test code.
- Prefer iterators and combinators (`map`, `filter`, `flat_map`, `collect`) over explicit `for` loops when it reads cleanly; use type-aware patterns (`Option::map`, `Result::map_err`).
- Prefer `&str` / `&[T]` over owned `String` / `Vec<T>` in function parameters when ownership isn't needed.
- Avoid unnecessary `.clone()`; prefer borrows, `Cow`, or restructure to move. Clone only when a copy is genuinely needed.
- Rely on type inference where obvious; annotate function signatures (params and return types) for clarity and to enforce contracts.
- Prefer `const` bindings; use `let mut` only when mutation is required. Use `if let` / `match` / early returns instead of mut reassignment.
- Add comments for non-obvious constraints and surprising behavior, not for obvious assignments or control flow.
- Keep helpers close to the code they support, below the main export when that improves readability. Extract only when it names a real concept like `resolve_config` or `read_metadata`.

Reduce total variable count by inlining when a value is only used once.

```rust
// Good
let journal: Journal = read_json(&path.join("journal.json")).await?;

// Bad
let journal_path = path.join("journal.json");
let journal: Journal = read_json(&journal_path).await?;
```

### Error Handling

- Use `thiserror` for library error enums with context-rich variants; use `anyhow` for application-level glue where the specific error type isn't important.
- Propagate errors with `?`; convert at boundaries with `.map_err(|e| MyError::Inner(e.to_string()))` or `From` impls.
- Never `unwrap()`/`expect()` on fallible operations in production code; tests may use them freely.
- Avoid `unwrap_or_default()` when a missing value is a real error; prefer explicit handling.

### Control Flow

Avoid `else` statements. Prefer early returns.

```rust
// Good
fn foo(cond: bool) -> u32 {
    if cond { return 1; }
    2
}

// Bad
fn foo(cond: bool) -> u32 {
    if cond { 1 } else { 2 }
}
```

### Async

- Use `tokio` as the runtime. Heavy/sync work (file IO that isn't tokio-aware, CPU loops) belongs in `tokio::task::spawn_blocking`, not on the async executor.
- Keep async functions `Send` when they may run on a multi-threaded runtime; avoid holding `!Send` guards across `.await`.
- One provider stream per turn; do not spawn nested tool loops that escape the worker.

### Naming

- `snake_case` for functions, methods, variables, modules, crates.
- `PascalCase` for types, traits, enum variants.
- `SCREAMING_SNAKE_CASE` for constants.
- Module file names are `snake_case.rs`.

### Module Layout

The crate follows a flat module tree under `crates/openrust/src/`:

- `cli/` — CLI entrypoint and `debug` subcommands (debug-first strategy: core logic is verified via `openrust debug <subcommand>` before TUI integration).
- `core/` — session, config, provider (trait + types), vault, crypto, compaction, permission, paths, platform, agent, session_input, token, memory.
- `tool/` — tool registry and individual tools.
- `tui/` — ratatui/crossterm terminal UI, worker, rendering.
- `provider/` — concrete LLM provider implementations (the `LlmProvider` trait itself lives in `core/provider.rs`).
- `system_prompt.rs` — system prompt assembly (kept out of `core/` to avoid layering violations into `tool/`).

When adding a module, follow the existing `mod.rs` + sibling-file pattern. Keep `tool/` and `tui/` Location-scoped; do not let model resolution or tool registry leak into the UI layer.

## Project Configuration Layout

openrust uses a unified `.openrust/` directory for project-level configuration:

```
project-root/
├── AGENTS.md              # Project-level AI instructions (injected into system prompt)
├── .openrust/
│   ├── config.jsonc       # Project config override (JSONC)
│   ├── config.json        # Project config override (plain JSON, alternative)
│   ├── agents/            # Custom agent definitions (markdown with frontmatter)
│   │   └── *.md
│   ├── skills/            # Custom skills
│   │   └── */SKILL.md
│   ├── rules/             # Project-level rules (markdown, concatenated)
│   │   └── *.md
│   └── memory/            # Project-level memory (category-based .md files)
│       └── {category}.md
```

**Loading priorities** (first match wins):

| Resource | Search order |
|----------|-------------|
| AGENTS.md | `<cwd>/AGENTS.md` (only) |
| Config | `.openrust/config.jsonc` → `.openrust/config.json` → `openrust.json` (legacy) → global `~/.config/openrust/config.json` |
| Agents | `.openrust/agents/` → global `~/.config/openrust/agents/` |
| Skills | `.openrust/skills/` → `skills/` → global `~/.config/openrust/skills/` |
| Rules | `.openrust/rules/*.md` (concatenated) + global `~/.config/openrust/rules/*.md` (concatenated) |

AGENTS.md is injected into the system prompt's `<instructions>` section wrapped in `<project-instructions>` tags.

Project rules are injected as `<project-rules>`, global rules as `<global-rules>`. All `*.md` files in each rules directory are sorted by filename and concatenated.

### Built-in Providers

`Config::load()` 自动填充内置 provider 定义（deepseek, glm, zhipuai-coding-plan, openai, anthropic, gemini）。用户只需提供 `api_key`，其余字段（base_url, models, reasoning 等）开箱即用。用户定义同名 provider 时**完全替换**内置定义。

## Testing

- Avoid mocks as much as possible; test actual implementations.
- Do not duplicate logic into tests.
- Run tests from the crate directory: `cd crates/openrust && cargo test`.
- Tests may use `unwrap()`/`expect()` freely.

## Type Checking & Linting

- Always run `cargo check` (and ideally `cargo clippy`) from `crates/openrust/` before pushing.
- Never leave `clippy` warnings in committed code.
