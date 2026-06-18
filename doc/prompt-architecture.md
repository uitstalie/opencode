# System Prompt 现状结构地图

> 分析日期: 2026-06-16  
> 模型: deepseek/deepseek-v4-pro  
> Agent: build (默认)  
> OpenCode 分支: dev-ai

---

## 1. 组装流程

```
SessionPrompt.runLoop (prompt.ts:1134)
  │
  ├── sys.environment(model)        → 环境信息
  ├── instruction.system()          → 指令文件
  ├── sys.skills(agent)             → 可用 skills
  │
  └── ...env, ...instructions, ...(skills ? [skills] : [])
       → 拼接为 system: string[]
       ↓
  LLMRequestPrep.prepare (llm/request.ts:58-66)
  │
  │  system[0] = [
  │    agent.prompt ?? SystemPrompt.provider(model),  ← ① 提供商标识 (带分支)
  │    ...input.system,                               ← ② 环境+指令+skills
  │    ...(user.system ?? [])                          ← ③ 用户自定义 system
  │  ].join("\n")
  │
  ↓ system = [single_joined_string]
  ↓
  experimental.chat.system.transform hook  ← ④ runtime-orchestrator 修改 system[]
  │
  │  prepend: Constraint Layer
  │  prepend: Role Section (enableRole=true 时)
  │  append:  Tool Reference + Mood(enableMood)+ Dreaming Insights
  │
  ↓ 合并至 ≤2 个 system message
  ↓
  system → ModelMessage[] (role: "system")  → AI SDK streamText
```

---

## 2. 提供商标识分支 (system.ts:25-38)

| 模型匹配条件 | Prompt 文件 | 行数 | 备注 |
|---|---|---|---|
| `model.api.id` 含 `gpt-4/o1/o3` | `beast.txt` | 147 | GPT-4/推理模型 |
| 含 `gpt` + `codex` | `codex.txt` | 79 | GitHub Copilot |
| 含 `gpt` (其他) | `gpt.txt` | 107 | GPT-3.5 等 |
| 含 `gemini-` | `gemini.txt` | 155 | Google Gemini |
| 含 `claude` | `anthropic.txt` | 105 | Anthropic Claude |
| 含 `trinity` | `trinity.txt` | 97 | Trinity |
| 含 `kimi` | `kimi.txt` | 95 | Moonshot Kimi |
| **其它全部 (含 DeepSeek)** | **`default.txt`** | **95** | **回退默认** |

**当前会话**: `deepseek-v4-pro` → 无匹配 → `default.txt`

**⚠️ 实际生效**: build agent 的 `agent.prompt` 被 `opencode.json` 覆盖为:
```
"Build mode: implement directly unless discussion only is requested.
 Read existing code before editing, make the smallest correct change,
 then verify and report clearly."
```
**→ `default.txt` 被完全替换！** (见 §2a)

### 2a. Agent 自定义 prompt 覆盖规则

`llm/request.ts:60`:
```ts
...(input.agent.prompt ? [input.agent.prompt] : SystemPrompt.provider(input.model)),
```

- `agent.prompt` 存在 → **完全替换** provider prompt
- `agent.prompt` 不存在 → 使用 provider prompt

**内置 agent prompt 状态** (agent/agent.ts):

| Agent | 内置 prompt | 纯 XML |
|---|---|---|
| build | 无 | ✅ |
| plan | 无 | ✅ |
| explore | 无 | ✅ |
| compaction | `compaction.txt` | ❌ |
| summary | `summary.txt` | ❌ |
| title | `title.txt` | ❌ |
| memory-extract | `memory-extract.txt` | ❌ |

**用户 opencode.json 覆盖**:

| Agent | prompt | 行数 |
|---|---|---|
| build | `"Build mode: implement directly..."` | 1 |
| plan | `"Plan mode: analyze and produce implementation plans..."` | 1 |
| design | 中等 (~200 chars) | 1 |
| kb-mgr | 中等 (~300 chars) | 1 |

→ **所有用户自定义 agent 的 prompt 都是 1~3 句话，完全替换了原 provider prompt (95-155 行)**

---

## 3. 环境层 (system.ts:55-92)

```ts
SysPrompt.environment(model)
```

输出结构:
```
You are powered by the model named deepseek-v4-pro.
The exact model ID is deepseek/deepseek-v4-pro

Here is some useful information about the environment you are running in:
<env>
  Working directory: {cwd}
  Workspace root folder: {worktree}
  Is directory a git repo: {yes/no}
  Platform: linux
  Today's date: {date}
</env>

Project references provide additional directories that can be accessed when relevant.
<available_references>
  <reference>
    <name>.opencode</name>
    <path>{path}</path>
    <description>...</description>
  </reference>
  ...
</available_references>
```

**大小**: 动态 (~200-500 chars, 取决于 references 数量)

---

## 4. 指令层 (instruction.ts:34-167)

```ts
Instruction.system()
```

### 4a. 加载顺序

`instruction.ts:60-63`:
```ts
const globalFiles = [
  path.join(global.config, "AGENTS.md"),        // ~/.config/opencode/AGENTS.md
  ...(!flags.disableClaudeCodePrompt            // ~/.claude/CLAUDE.md
    ? [path.join(global.home, ".claude", "CLAUDE.md")]
    : []),
]
```

`instruction.ts:64-68`:
```ts
const instructionFiles = [
  "AGENTS.md",
  ...(!flags.disableClaudeCodePrompt ? ["CLAUDE.md"] : []),
  "CONTEXT.md",  // deprecated
]
```

### 4b. 文件解析逻辑 (instruction.ts:123-133)

1. 全局文件: 遍历 `globalFiles`，取**第一个存在的**
2. 项目文件: `findUp` 从 `cwd` 向上搜索 `instructionFiles`，**找到第一个就 break**

### 4c. 当前加载清单

| 来源 | 路径 | 行数 | 状态 |
|---|---|---|---|
| 全局 AGENTS.md | `~/.config/opencode/AGENTS.md` | — | 不存在 |
| 全局 CLAUDE.md | `~/.claude/CLAUDE.md` | — | 不存在 |
| 项目 AGENTS.md | `fake_opencode/AGENTS.md` | 28 | ✅ |
| 项目 CLAUDE.md | — | — | 不存在 |
| 指令文件 | `core-operating-rules.md` | 25 | ✅ (config.instructions) |
| 指令文件 | `internet-rules.md` | 23 | ✅ (config.instructions) |
| opencode AGENTS.md | `source-code/opencode/AGENTS.md` | ~200 | ✅ (resolve 时动态附加) |

**格式**: `Instructions from: {path}\n{content}`

---

## 5. Skills 层 (system.ts:94-106)

```ts
SysPrompt.skills(agent)
```

- 仅当 agent 未禁用 `skill` 权限时输出
- 输出: "Skills provide specialized instructions..." + 可用 skills 列表 (verbose 模式)

**当前可用 skills**:
- customize-opencode
- dreaming
- shared-knowledge
- write_code_artical_v2

**行数**: ~10-15

---

## 6. 插件注入层 (hooks/session.ts:109-152)

`experimental.chat.system.transform` hook → `createSystemTransformHook()`

### 6a. Constraint Layer (prepend, 固定)

**来源**: `prompt/compose.ts:82-101`

```markdown
## Constraint Layer (CRITICAL)

### Tool Invocation Rules
- [Memory] 决策前必查 memory_read（含 dreaming）。写入前必 memory_review 查重。稳定结论才 memory_record。
- [Compact] 每轮结束调 compact_check。shouldCompact=true → 立刻 compress，再继续回复。
- [Mood] mood 功能已关闭，跳过 mood_push。

### Concrete Discipline
- memory 宁缺毋滥：只记稳定结论/长期约束/明确偏好，不写流水账。
- 写入流程：memory_review → 确认无重复 → memory_record；不确定规则走 memory_candidates。
- 上下文接近上限时主动 compact_check，不等溢出。
```

**行数**: ~15 | **状态**: ✅ 始终注入

### 6b. Role Section (prepend, 条件)

**来源**: `prompt/compose.ts:124-163` → `buildStableSystemSections()`

**条件**: `enableRole: true`

**当前**: `enableRole: false` → **不注入**

### 6c. Tool Reference (append, 固定)

**来源**: `prompt/compose.ts:103-122`

```markdown
## Available Tools
- 知识库查询：`shared-knowledge` skill 或 `kb-mgr` task
- 跨会话洞察：`memory_read(scope=user, layer=dreaming)` ...
- 项目记忆：`memory_read` / `memory_record` / ...
- 情绪状态：mood 功能已关闭，跳过 mood_query/mood_push
- Token 用量：`token_stats` / `token_history`
```

**行数**: ~10 | **状态**: ✅ 始终注入

### 6d. Dreaming Insights (append, 条件)

**来源**: `prompt/compose.ts:57-80`

**条件**: Dreaming state 文件存在

**当前**: 存在 → 注入

```markdown
## Dreaming Insights (cross-session knowledge)
- Dreaming 已从历史 session 提取跨会话习惯/规则/模式...
- 做决策前应查阅: `memory_read(scope=user, layer=dreaming)` ...
- Dreaming 中标记 #confirmed 的模式应视为与 memory/preferences 同等的约束来源。
```

**行数**: ~5 | **状态**: ✅ 当前注入

### 6e. Mood Dynamic Section (append, 条件)

**条件**: `enableMood: true`

**当前**: `enableMood: false` → **不注入**

---

## 7. 实际生效的完整 System Prompt (DeepSeek + build agent)

```
┌──────────────────────────────────────────────────────┐
│ Section                    │ Source          │ Lines │
├──────────────────────────────────────────────────────┤
│ Constraint Layer           │ plugin prepend  │   15  │
│ Agent Prompt               │ opencode.json   │    1  │
│ Environment + References   │ system.ts       │   10  │
│ Instructions:              │ instruction.ts  │        │
│   fake_opencode/AGENTS.md  │                 │   28  │
│   core-operating-rules.md  │                 │   25  │
│   internet-rules.md        │                 │   23  │
│   opencode/AGENTS.md       │ resolve 动态附加│ ~200  │
│ Skills                     │ system.ts       │   10  │
│ Tool Reference             │ plugin append   │   10  │
│ Dreaming Insights          │ plugin append   │    5  │
├──────────────────────────────────────────────────────┤
│ **总计**                                       ~330  │
└──────────────────────────────────────────────────────┘
```

**注意**: `default.txt` (95 行) **未生效** — 被 agent prompt 覆盖。

---

## 8. 不进入 system prompt 的内容

| 内容 | 传递方式 |
|---|---|
| Tool 定义 (memory_record, bash, glob, ...) | AI SDK `tools` 参数 (JSON Schema) |
| CONSTRAINT NUDGE | `messages.transform` hook → 注入到 user message 末尾 |
| Agent 专属 prompt (compaction/summary/memory-extract.txt) | 替代 provider prompt，对该 agent 生效 |
| model 参数 (temperature/topP/maxTokens) | AI SDK `params` |
| Permission rules | 工具过滤 + user message inbound |

---

## 9. 关键文件索引

| 文件 | 职责 |
|---|---|
| `session/prompt.ts:1327-1343` | 组装 env + instructions + skills → `system: string[]` |
| `session/llm/request.ts:58-78` | 组装 agent.prompt + system + 插件注入 → 最终 `system: string[]` |
| `session/system.ts:25-38` | `provider(model)` → 模型→prompt 映射 |
| `session/system.ts:55-92` | `environment(model)` → 模型信息 + `<env>` |
| `session/instruction.ts:110-168` | `systemPaths()` / `system()` → AGENTS.md / CLAUDE.md 发现 |
| `session/llm/request.ts:68-78` | `system.transform` hook 触发点 |
| `plugins/runtime-orchestrator/hooks/session.ts:109-152` | `createSystemTransformHook()` → 插件注入 |
| `plugins/runtime-orchestrator/prompt/compose.ts` | 插件注入各段文本 |
| `agent/agent.ts:286-310` | 用户 config agent.prompt 覆盖逻辑 |
| `session/prompt/default.txt` | DeepSeek 回退 provider prompt (当前未生效) |

---

## 10. 问题与改进方向

1. **provider prompt 被覆盖** — 用户自定义 1 句 prompt 替换了 95 行 `default.txt`，导致编码规范、工具策略等全部丢失
2. **无 section 标记** — 所有层 `join("\n")` 拼为一个字符串，LLM 无法区分来源
3. **agent.prompt 语义不清** — 既是"自定义行为"又是"覆盖所有基础指令"，边界模糊
4. **指令文件与 provider prompt 冲突** — AGENTS.md 编码规范与 default.txt 行为准则部分重叠
5. **插件注入完全独立** — Constraint Layer 和 Tool Reference 无序号、无版本、无优先级
6. **硬编码分支** — `system.ts:provider()` 的 if-else 链条无法扩展新模型
7. **无模板化** — 环境信息、指令、skills 都是 Effect.gen 中硬编码拼接字符串
