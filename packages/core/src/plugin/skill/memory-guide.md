<!--
  Built-in skill. Name and description are registered in code at
  packages/core/src/plugin/skill.ts. The body below becomes the
  skill's content.
-->

# Memory Guide

You have access to a three-layer memory system for persisting stable
conclusions, preferences, and patterns across sessions.

## Architecture

| Layer | Scope | Storage | Targets |
|-------|-------|---------|---------|
| **Project** | `scope=project` | SQLite (`memory.db`) + `.opencode/memory/` | `progress`, `TODO`, `tech`, `conclusion` |
| **Global** | `scope=user` | `~/.config/opencode/memory/` .md files | `preferences`, `constraints`, `patterns`, `style` |
| **Dreaming** | scope=dreaming | `~/.config/opencode/memory/dreaming/` .md files | per-project hash |

## Project Layer (`scope=project`)

Four targets for project-specific memory. All written to both SQLite and .opencode/memory/{target}.md.

### `progress` — Completed Milestones

**When**: A distinct body of work reaches a clear milestone. NOT every individual step or tool call.

"完成 Qdrant 替换 Faiss 的索引迁移" ✓
"执行了 git status" ✗

### `TODO` — Pending Items

**When**: A non-transient task is identified that needs follow-up. NOT temporary debugging notes or in-progress steps.

"验证新索引在 1M+ 向量规模下的召回率" ✓
"试试加个 try-catch 看看" ✗

### `tech` — Technical Decisions with Rationale

**When**: A technology/architecture choice is made AND the reason is clear. NOT bare facts without reasoning.

"选择 Redis pub/sub 取代 RabbitMQ：部署简单，团队已有运维经验" ✓
"用了 Redis" ✗

### `conclusion` — Strategic Decisions

**When**: A project-level decision that affects future direction. NOT routine implementation choices.

"决定废弃副 agent 机制，合并回主 agent 流程" ✓
"把 `sleep(100)` 改成 `sleep(200)`" ✗

---

## Global Layer (`scope=user`)

Four categories for cross-project, recurring patterns. Written to markdown files.

### `preferences` — User Preferences

**When**: Same preference appears ≥2 times across projects or sessions. Single-instance preferences are not recorded.

"使用简体中文回复，代码标识符保留英文" ✓
"这次项目用 TypeScript" ✗ (单次选择)

### `constraints` — Hard Constraints

**When**: User explicitly states a prohibition or non-negotiable rule. Record immediately on first occurrence.

"不提交 secrets 到 git" ✓
"先检查远程分支状态再合并" ✓ (如果用户明确说"必须")

### `patterns` — Reusable Workflows

**When**: A multi-step process is repeated and can be codified. NOT one-off sequences.

"改配置后先编译再重启验证" ✓

### `style` — Code & Communication Style

**When**: A stylistic convention is observed ≥2 times. Covers code style, reply style, formatting preferences.

"配置优先最小变更而非重构" ✓

---

## Dreaming Layer

Dreaming refines and compresses global memory. Do NOT call it proactively — only when:

- Global preferences entries reach ≥10
- User explicitly says "dream" / "dreaming" / "整理记忆" / "提取模式"
- Obvious duplicate/contradictory global entries exist

Use `dreaming_compress(dryRun=true)` first to preview, then `dreaming_compress(dryRun=false)` to apply.

---

## Tools

| Tool | Purpose | Key Rule |
|------|---------|----------|
| `memory_read(scope, target/name)` | Read existing memories | Call BEFORE key decisions |
| `memory_review(scope, target/name)` | Check for duplicates | Call BEFORE every write |
| `memory_record(scope, content, target/name, tags)` | Write new memory | Only stable conclusions |
| `dreaming_compress(dryRun)` | Merge/refine global memory | Preview first |

## Write Protocol

```
1. memory_read(scope=project) or memory_read(scope=user) — understand what exists
2. For each candidate entry:
   a. memory_review(...) — dedup check
   b. memory_record(...)  — write if unique and stable
3. If nothing to record → say nothing, don't fabricate
```

## Tags

| Tag | Meaning | Use On |
|-----|---------|--------|
| `#confirmed` | Verified conclusion | All layers |
| `#likely` | High-confidence inference | All layers |
| `#decision` | Decision point | Project + Global |
| `#architecture` | Architecture-related | Project |
| `#constraint` | Hard constraint | Global |
| `#preference` | User preference | Global |
| `#pattern` | Reusable pattern | Global |
| `#style` | Style convention | Global |
| `#issue` | Known bug/issue | Project |

## Forbidden

1. Do NOT fabricate memories from things not in conversation
2. Do NOT write without `memory_review` first
3. Do NOT record transient chat ("hello", "thanks", "what time is it")
4. Do NOT record single atomic operations ("git status output", "compiled successfully")
5. Do NOT confuse layers: project decisions → scope=project, cross-project patterns → scope=user
6. Do NOT record implementation details (line numbers, variable names, exact function signatures)
7. Do NOT reply with verbose analysis — just say what was recorded or "no new memories"
