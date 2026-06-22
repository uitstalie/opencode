# TUI Bottom Bar / Footer 区域分析

> 分析对象：`/home/uitstalie/桌面/fake_opencode/packages/tui/`
> 分析日期：2026-06-19

---

## 一、概述

TUI 底部区域**没有单一的主组件**，而是由多层组合而成：App 级底栏、路由页级的 Prompt 区域、子会话的 SubagentFooter、Home 页的 Footer Plugin、以及 Sidebar 的 Footer。

---

## 二、整体布局结构

```
┌───────────────────────────────────────────────┐
│  App (app.tsx)                                │
│  ┌─────────────────────────────────────────┐  │
│  │  Router → Session / Home               │  │
│  │  ┌───────────────────────────────────┐  │  │
│  │  │  Scrollbox (消息列表)              │  │  │
│  │  │  ...                              │  │  │
│  │  │  ┌───────────────────────────┐    │  │  │
│  │  │  │ 【底部区域】(flexShrink=0)│    │  │  │
│  │  │  │                           │    │  │  │
│  │  │  │  ① SubagentFooter        │    │  │  │
│  │  │  │     (子会话时)            │    │  │  │
│  │  │  │                           │    │  │  │
│  │  │  │  ② Prompt 组件            │    │  │  │
│  │  │  │  ┌─ agent/model/var      │    │  │  │
│  │  │  │  ├─ <textarea>          │    │  │  │
│  │  │  │  └─ 状态栏              │    │  │  │
│  │  │  │                           │    │  │  │
│  │  │  │  ③ PermissionPrompt      │    │  │  │
│  │  │  │  ④ QuestionPrompt        │    │  │  │
│  │  │  └───────────────────────────┘    │  │  │
│  │  │  Toast                           │  │  │
│  │  └───────────────────────────────────┘  │  │
│  │                     [Sidebar]          │  │
│  │           ┌─ sidebar_footer slot       │  │
│  └─────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────┐  │
│  │  app_bottom slot                        │  │
│  └─────────────────────────────────────────┘  │
└───────────────────────────────────────────────┘
```

### 关键文件

| 文件 | 角色 |
|------|------|
| `src/routes/session/index.tsx` | 会话页主布局，包含底部区域的组合逻辑 |
| `src/component/prompt/index.tsx` | Prompt 输入组件的全部实现（输入框 + 元信息 + 状态栏） |
| `src/routes/session/subagent-footer.tsx` | 子会话时的底部信息栏 |
| `src/routes/session/footer.tsx` | **（未使用）** 旧版 Footer 组件 |
| `src/routes/home.tsx` | Home 页布局 |
| `src/feature-plugins/home/footer.tsx` | Home 页底栏插件（目录 / MCP / 版本） |
| `src/feature-plugins/sidebar/footer.tsx` | Sidebar 底栏插件（目录 / 版本 / 入门引导） |
| `src/app.tsx` | 顶层 App 布局，含 `app_bottom` slot |

---

## 三、逐层分析

### 3.1 Session 页底部区域 (核心)

位于 `src/routes/session/index.tsx` 第 1285-1323 行：

```tsx
<box flexShrink={0}>
  <Show when={permissions().length > 0}>
    <PermissionPrompt ... />
  </Show>
  <Show when={permissions().length === 0 && questions().length > 0}>
    <QuestionPrompt ... />
  </Show>
  <Show when={session()?.parentID}>
    <SubagentFooter />
  </Show>
  <Show when={visible()}>
    {/* Prompt 组件（含输入框 + 元信息 + 状态栏）*/}
  </Show>
</box>
```

显示逻辑：

```
visible() = !parentID && permissions.length === 0 && questions.length === 0
```

| 条件 | 显示 |
|------|------|
| 有 Permission 请求 | `PermissionPrompt` |
| 有 Question 请求 | `QuestionPrompt` |
| 是子会话 (`parentID` 存在) | `SubagentFooter` |
| 主会话且无待处理请求 | `Prompt` 组件 |

---

### 3.2 Prompt 组件

文件：`src/component/prompt/index.tsx`（全长 1697 行）

#### 3.2.1 视觉元素

**第一行（元信息行）** — 第 1436-1473 行：

```
┌─ [Agent/模式标签] · [模型名] [Provider] · [Variant] ─── [右侧 slot] ─┐
```

- **Agent/模式标签**：如 `Build`、`Plan`，或在 Shell 模式下显示 `Shell`
  - 颜色 = Agent 的标识颜色，带渐变 fade-in 动画
  - 来源：`local.agent.current().name` → `Locale.titlecase()`
- **模型名**：如 `claude-sonnet-4`
  - 来源：`local.model.parsed().model`
- **Provider 标签**：如 `Anthropic`
  - 来源：`local.model.parsed().provider`
  - 颜色：muted，leader 激活时更暗
- **Variant 标签**：如 `reasoning`（仅当当前模型支持 variant 时显示）
  - 来源：`local.model.variant.current()`
  - 颜色：`theme.warning` + bold
- **右侧插槽**：`session_prompt_right` Plugin Slot
  - 无内置内容，留给插件扩展

**第二行（输入区域）** — 第 1363-1435 行：

```
╹ ┌──────────────────────────────────────────────────────┐
  │ <textarea>                                             │
  │ placeholder: "Ask anything... \"Fix a TODO...\""       │
  │ 可多行，高度受 max_height 限制（默认 max(6, height/3)）│
  │ 高亮：leader 激活时变 muted                            │
  └──────────────────────────────────────────────────────┘
```

- 实际的 `TextareaRenderable` 组件
- Placeholder 随机循环显示提示词列表
- Shell 模式（输入 `!`）切换 placeholder 为命令示例
- 支持 paste（文本 / 图片 / 文件）、extmarks（虚拟文本标记）、自动补全

**第三行（状态栏 / 底部信息栏）** — 第 1502-1671 行：

```
┌─────────────────────────────── 左侧 ──────────────────── ┌───────────────── 右侧 ──────────────────┐
```

**左侧内容（Switch 分支）：**

1. **当 LLM 活跃时** (`status().type !== "idle"`):
   - Spinner 动画（blocks 风格，颜色 = Agent 色）
   - 错误重试信息（truncated, 可点开完整错误）
   - `esc interrupt` 提示
   - 按两次 Esc = abort 会话

2. **当正在创建工作区时**: workspace notice 信息

3. **当正在移动会话时**: move progress spinner

4. **默认**: 空（prop hint）

**右侧内容：**

1. **编辑器上下文文件标签** — 显示当前打开的文件（如 `config.ts#42`）
   - 来源：`editor.selection()` → `editorContext()` → 提取文件名 + 选区行号
   - 颜色：pending 时 `theme.secondary`，否则 `theme.textMuted`

2. **Token 用量 / 费用**（normal 模式，仅当有 assistant 消息时）:
   - 格式：`12,345 (43%) · $0.12`
   - 来源：最后一条有 output tokens 的 assistant 消息
   - 计算：`input + output + reasoning + cache.read + cache.write`
   - Context 百分比 = `tokens / model.limit.context`
   - 费用 = `session.cost`，用 USD 格式化

3. **快捷键提示**（normal 模式，无 token 用量时）:
   - `Ctrl+A agents`
   - `Ctrl+K commands`

4. **Shell 模式提示**:
   - `esc exit shell mode`

---

### 3.3 SubagentFooter 组件

文件：`src/routes/session/subagent-footer.tsx`（132 行）

显示时：会话 `parentID` 存在（即子会话）

左侧：
- **Subagent 标签**：从 title 中提取 `@agentName subagent`，如 `Builder`
- **索引**：`(2 of 4)` — 在所有兄弟子会话中的位置
- **Token 用量 / 费用**：格式同 Prompt 组件

右侧（导航按钮）：
- `Parent [Esc]` — 返回父会话
- `Prev [` — 上一个子会话
- `Next `]` — 下一个子会话

每个按钮有 hover 高亮效果。

---

### 3.4 Home 页底栏

文件：`src/feature-plugins/home/footer.tsx`

通过 Plugin Slot `home_footer`（`routes/home.tsx` 第 91 行）渲染：

```
/项目/目录:分支                3 MCP  /status            v0.0.0-dev-...
```

- **目录**：如果有选择目标目录显示路径 + 分支
- **MCP**：连接数 + 状态指示（绿色 ⊙ 已连接 / 红色 ⊙ 失败）
- **版本**：`props.api.app.version`

---

### 3.5 Sidebar 底栏

文件：`src/feature-plugins/sidebar/footer.tsx`

通过 Plugin Slot `sidebar_footer`（`routes/session/sidebar.tsx` 第 90 行）渲染：

- **Getting Started 面板**（未连接 provider 且未被 dismiss 时）
- **会话目录路径**：`parent/directory/name`
- **OpenCode 品牌 + 版本**：`• OpenCode v0.0.0-dev-...`

---

### 3.6 未使用的旧 Footer 组件

文件：`src/routes/session/footer.tsx`（91 行）

**该组件未被任何文件导入或使用。** 它曾经实现的功能：
- 左侧：当前工作目录
- 右侧：欢迎提示（未连接时）、权限提示、LSP 数量、MCP 数量、`/status` 入口

这些功能已迁移到：
- Home 页底栏（`home/footer.tsx`）
- Sidebar 底栏（`sidebar/footer.tsx`）
- Prompt 的状态栏

---

## 四、数据流

### 4.1 主要数据源

```
useSync()           → sync.data.* (session, message, provider, mcp, lsp, permission, ...)
useLocal()          → local.agent, local.model, local.model.variant (用户选择)
useRoute()/useRouteData() → route.sessionID, route.type
useEditorContext()  → editor.selection (编辑器上下文)
useDirectory()      → project.instance.path().directory (当前目录+分支)
useConnected()      → 是否有已连接且非零费用 provider
```

### 4.2 Prompt 组件内部状态

```ts
createStore<{
  prompt: PromptInfo  // { input: string, parts: Part[] }
  mode: "normal" | "shell"
  extmarkToPartIndex: Map<number, number>
  interrupt: number
  placeholder: number
}>()
```

### 4.3 信息流向图

```
SyncProvider (SSE/API 数据)    LocalProvider (用户选择)
        │                              │
        ├─ sync.data.provider ────────→ 模型名 / Provider 标签
        ├─ sync.data.provider ────────→ Token 用量计算 (usage())
        ├─ sync.session ──────────────→ session.cost → 费用显示
        ├─ sync.data.session_status ──→ status().type → Spinner / 重试
        ├─ sync.data.mcp ────────────→ MCP 数量 / 状态
        ├─ sync.data.vcs.branch ──────→ 目录:分支 (useDirectory)
        ├─ sync.data.permission ──────→ visible() 判断 → PermissionPrompt
        ├─ sync.data.question ────────→ visible() 判断 → QuestionPrompt
        │
        ├─ local.agent.current() ────→ Agent 名称 / 颜色
        ├─ local.model.parsed() ─────→ 模型名 / Provider
        ├─ local.model.variant ──────→ Variant 标签
        │
editor.selection() ───────────────────→ 编辑器文件标签
project.workspace ────────────────────→ workspace label / notice
```

---

## 五、插件 Slot 汇总

| Slot 名称 | 位置 | 渲染模式 | 内置插件 |
|-----------|------|----------|----------|
| `app_bottom` | app.tsx 最底 | 无限制 | 无 |
| `home_footer` | home.tsx 底部 | `single_winner` | `home/footer.tsx` |
| `home_prompt` | home.tsx 中部 | `replace` | 无（用 `<Prompt>` 兜底） |
| `home_prompt_right` | home.tsx Prompt 内 | 无限制 | 无 |
| `session_prompt` | session/index.tsx | `replace` | 无（用 `<Prompt>` 兜底） |
| `session_prompt_right` | session Prompt 内 | 无限制 | 无 |
| `sidebar_footer` | session/sidebar.tsx | `single_winner` | `sidebar/footer.tsx` |

---

## 六、关键结论

1. **没有单一底部组件** — 底部区域由 3 个主要渲染分支（Prompt / SubagentFooter / PermissionPrompt）和 3 个插件 slot（home_footer / sidebar_footer / app_bottom）组合而成。

2. **Prompt 组件是最核心的底部元素** — 它包含了输入框、元信息行（agent/model/variant）和状态栏（token 用量、快捷键提示、spinner 等）三层结构。

3. **`routes/session/footer.tsx` 是死代码** — 该文件和导出的 `Footer` 函数未被任何文件引用，可以安全删除。

4. **信息流是自顶向下的** — 所有数据来自 context（Sync / Local / Editor 等），没有子组件向父组件的事件传递（除了 Prompt 的 onSubmit 回调和 ref.set）。

5. **状态栏是 Prompt 的一部分** — Token 用量和费用信息在 Prompt 组件内部通过 `usage()` memo 计算，而不是独立的状态栏组件。
