# opencode compact/compress 事件机制探测报告

> 探测日期：2026-06-15
> 源码根目录：`/mnt/d/dqc/fake_opencode/source-code/opencode/`
> 全局插件目录：`~/.config/opencode/plugins/`

---

## 1. compact/compress 相关的事件类型（event type）

opencode 的 compaction 涉及两套事件体系：**V2 内部事件系统（SessionEvent）** 和 **GlobalBus 广播事件（Event）**。

### V2 SessionEvent（内部系统，`packages/core/src/session/event.ts`）

| 事件类型 | 说明 | 源码位置 |
|---|---|---|
| `session.next.compaction.started` | compaction 启动（starts） | L424-434，`Compaction.Started` |
| `session.next.compaction.delta` | compaction 流式增量（delta，临时态） | L436-444，`Compaction.Delta` |
| `session.next.compaction.ended` | compaction 结束（ended），含 v1/v2 两版 | L447-468，`Compaction.EndedV1` + `Compaction.Ended` |

### 旧版 EventV2（`packages/opencode/src/session/compaction.ts`）

| 事件类型 | 说明 | 源码位置 |
|---|---|---|
| `session.compacted` | compaction 完成通知 | L29-36，`Event.Compacted` |

此事件在 `compaction.ts` L549 通过 `yield* events.publish(Event.Compacted, { sessionID })` 发送。

### GlobalBus Event Union（`packages/sdk/js/src/v2/gen/types.gen.ts`）

所有通过 `GlobalBus.emit("event", ...)` 广播的事件在 SDK 的 `Event` union type 中定义（L7-94），包含以下与 compact 相关的事件：

- **`session.compacted`** — L85 `EventSessionCompacted`、L1562-1568
- **`tui.command.execute`** — 其中包含 `"session.compact"` 命令（L1430），这是 TUI 发送的手动触发 compact 命令

### 触发时机总结

1. **手动触发**：TUI 发送 `tui.command.execute` 事件，command 值为 `"session.compact"` → `packages/opencode/src/server/tui-event.ts` L15-19 → `packages/server/src/groups/session.ts` L165 的 `HttpApiEndpoint.post("session.compact", ...)`
2. **自动触发**：`SessionCompaction.create()` 被调用时（`packages/opencode/src/session/compaction.ts` L554-585），若 `flags.experimentalEventSystem` 为 true，会 publish `SessionEvent.Compaction.Started`
3. **完成后**：`SessionCompaction.process()` 结束时（L528-551），publish `SessionEvent.Compaction.Ended` 和 `Event.Compacted`

### 关键结论

- **没有** `session.compact` 作为事件类型被 publish（它是 TUI command 名，不是事件 type）
- **没有** `system.compact` 事件
- compaction 真正的事件类型有：`session.next.compaction.started`、`session.next.compaction.ended`、`session.compacted`
- `compaction.delta` 是流式增量事件（临时态），不参与持久化

---

## 2. `compact_check` 工具：内置还是插件暴露？

**`compact_check` 不是 opencode 内置工具，是插件（`runtime-orchestrator`）通过 `tool` hook 注册的。**

### 证据

1. 在 opencode 源码中搜索 `compact_check`：**0 个匹配** — 证实不在内置工具中
2. 实现位置：`~/.config/opencode/plugins/runtime-orchestrator/index.ts` L338-374

```typescript
// L338-374
compact_check: tool({
  description: "Inspect current dcp compression state; when shouldCompact=true and recommendedRange exists, call compress next.",
  args: {},
  async execute() {
    const activeTokens = dcpActiveTokens()
    const estimatedVisibleTokens = dcpEstimatedVisibleTokens()
    const decisionTokens = Math.max(activeTokens.total || 0, estimatedVisibleTokens.total || 0)
    const shouldCompact = decisionTokens >= 140000
    const recommendedRange = dcpRecommendedRange(decisionTokens)
    // ... 返回 JSON 包含 shouldCompact, reason, thresholds, recommendedRange 等
  },
}),
```

3. **暴露给主 agent 的机制**：通过 `config()` hook 注入 `experimental.primary_tools`（L275-288）：
   ```typescript
   config: async (opencodeConfig) => {
     const primaryTools = opencodeConfig.experimental.primary_tools ?? []
     for (const name of ["compress", "compact_check"]) {
       if (!primaryTools.includes(name)) primaryTools.push(name)
     }
     opencodeConfig.experimental.primary_tools = primaryTools
     // 同时设置 permission: allow
     if ((permission as any).compact_check === undefined) (permission as any).compact_check = "allow"
   },
   ```

### 结论
- `compact_check` 是 **插件定义的工具**，不在 opencode 内置工具列表中
- 它通过 `tool` 对象注册，再通过 `config()` hook 的 `experimental.primary_tools` 注入主 agent 可见工具列表
- 实际调用的压缩逻辑委托给 `dcp-forget` 的 `scripts/dcp-compress-tool.mjs` 和 `scripts/dcp-state.mjs`

---

## 3. compress/compact/prune 流程入口

### 3.1 compact 流程入口

**文件**: `packages/opencode/src/session/compaction.ts`

| 入口 | 行号 | 说明 |
|---|---|---|
| `Interface.create()` | L140-159（接口定义）、L554-585（实现） | 创建 compaction user message + compaction part，可选 publish `Compaction.Started` 事件 |
| `Interface.process()` | L146-152（接口定义）、L299-552（实现） | 实际执行 compaction：选择 tail、触发插件 hook、调用 LLM 生成摘要、写入 compaction message、处理 autocontinue |
| `Interface.prune()` | L145（接口定义）、L252-297（实现） | 向后遍历 tool outputs，超过 `PRUNE_PROTECT`（40K tokens）的旧 tool 输出标记为 compacted，释放上下文空间 |
| `Interface.isOverflow()` | L141-144（接口定义）、L178-188（实现） | 检查是否溢出，委托给 `overflow()` 函数（`packages/opencode/src/session/overflow.ts`），判断逻辑含 `compaction.reserved` 和 `compaction.auto` 配置 |

### 3.2 调用链

1. **TUI 手动触发**：`packages/server/src/groups/session.ts` L165 → HTTP POST `/api/session/:sessionID/compact` → handler `session.compact` → 调用 `compact()` → 进入 `SessionCompaction.create()` + `process()`
2. **session/prompt.ts 自动触发**：`packages/opencode/src/session/prompt.ts` L1216-1219：检查 `compaction.isOverflow()` → 若 true 且 `auto=true` → 调用 `compaction.create()`
3. **processor 级触发**：`packages/opencode/src/session/processor.ts` L927-1030：检查 `compaction.auto !== false` → 若 `needsCompaction` → 返回 `"compact"` 结果
4. **prune 自动触发**：`packages/opencode/src/session/prompt.ts` L1399：每轮结束后 `compaction.prune({ sessionID })` 被 fork 执行

### 3.3 overflow 检查

**文件**: `packages/opencode/src/session/overflow.ts`
- L15-28：`isOverflow()` 函数，基于 `compaction.reserved`（保留 token 数）和 `compaction.auto`（是否开启自动压缩）判断
- 被 `SessionCompaction.Service` 的 `isOverflow` 包装（compaction.ts L178-188）

### 3.4 插件层面的 compress（dcp-forget）

全局 `dcp-forget` 插件合并进 `runtime-orchestrator` 后，提供了插件级别的 `compress` 工具：

- **入口文件**：`~/.config/opencode/plugins/runtime-orchestrator/index.ts` L376-399
- **核心逻辑**：调用 `compressRanges()`（来自 `~/.config/opencode/plugins/dcp-forget/scripts/dcp-compress-tool.mjs` L31-116）
- **DDD 脚本**：
  - `scripts/dcp-compress-tool.mjs` — 主压缩逻辑：消息引用分配 → 搜索上下文构建 → 边界解析 → 摘要验证 → 状态应用
  - `scripts/dcp-search.mjs` — 搜索和选择逻辑：buildSearchContext、resolveSelection、resolveBoundaryIds
  - `scripts/dcp-state.mjs` — 状态管理：allocateBlockId、applyCompressionState
  - `scripts/dcp-block-placeholders.mjs` — block 占位符生成
  - `scripts/index.mjs` — barrel export，聚合以上模块

---

## 4. 插件 `event` hook 能收到的事件类型

### 4.1 hook 定义

**文件**: `packages/plugin/src/index.ts`

```typescript
// L224
export interface Hooks {
  event?: (input: { event: Event }) => Promise<void>
  // ...
}
```

`Event` 类型来自 `@opencode-ai/sdk`，实际定义在 SDK gen 文件中。

### 4.2 事件转发机制

**文件**: `packages/opencode/src/event-v2-bridge.ts`

- L38-67：`events.listen()` 监听所有内部 `EventV2` 事件
- L42-47：通过 `GlobalBus.emit("event", { ...payload })` 将事件广播给所有订阅者
- L48-66：对于有 `sync` 配置的持久化事件，还会额外发送一条 `type: "sync"` 的广播事件
- 事件 payload 格式：`{ directory, project, workspace, payload: { id, type, properties } }`

### 4.3 完整 Event 类型列表

参见 `packages/sdk/js/src/v2/gen/types.gen.ts` L7-94，共 **53 个**事件类型：

| 分类 | 事件类型 |
|---|---|
| **模型/插件/集成** | `models-dev.refreshed`, `plugin.added`, `integration.updated`, `catalog.updated` |
| **Session 生命周期** | `session.created`, `session.updated`, `session.deleted` |
| **Session 状态** | `session.status`, `session.idle`, `session.error`, `session.diff`, `session.compacted` |
| **Message 生命周期** | `message.updated`, `message.removed`, `message.part.updated`, `message.part.removed`, `message.part.delta` |
| **V2 内部事件（session.next.*）** | `session.next.agent.switched`, `session.next.model.switched`, `session.next.moved`, `session.next.prompted`, `session.next.prompt.admitted`, `session.next.prompt.promoted`, `session.next.interrupt.requested`, `session.next.context.updated`, `session.next.synthetic`, `session.next.shell.started`, `session.next.shell.ended`, `session.next.step.started`, `session.next.step.ended`, `session.next.step.failed`, `session.next.text.started`, `session.next.text.delta`, `session.next.text.ended`, `session.next.reasoning.started`, `session.next.reasoning.delta`, `session.next.reasoning.ended`, `session.next.tool.input.started`, `session.next.tool.input.delta`, `session.next.tool.input.ended`, `session.next.tool.called`, `session.next.tool.progress`, `session.next.tool.success`, `session.next.tool.failed`, `session.next.retried`, `session.next.compaction.started`, `session.next.compaction.delta`, `session.next.compaction.ended` |
| **安装/升级** | `installation.updated`, `installation.update-available` |
| **文件系统** | `file.edited`, `project-directories.updated`, `file-watcher.updated` |
| **权限** | `permission.v2.asked`, `permission.v2.replied`, `permission.asked`, `permission.replied` |
| **PTY** | `pty.created`, `pty.updated`, `pty.exited`, `pty.deleted` |
| **Question** | `question.v2.asked`, `question.v2.replied`, `question.v2.rejected`, `question.asked`, `question.replied`, `question.rejected` |
| **TODO** | `todo.updated` |
| **LSP** | `lsp.updated` |
| **TUI 事件** | `tui.prompt.append`, `tui.command.execute`, `tui.toast.show`, `tui.session.select` |
| **MCP** | `mcp.tools.changed`, `mcp.browser.open.failed` |
| **命令** | `command.executed` |
| **项目** | `project.updated` |
| **VCS** | `vcs.branch.updated` |
| **工作区/Worktree** | `workspace.ready`, `workspace.failed`, `workspace.status`, `worktree.ready`, `worktree.failed` |
| **全局** | `server.connected`, `global.disposed`, `server.instance.disposed` |
| **参考** | `reference.updated` |

### 4.4 实际使用案例

全局 `runtime-orchestrator` 插件只处理其中 3 种事件类型（`~/.config/opencode/plugins/runtime-orchestrator/hooks/event.ts` L27-67）：

- `session.error` — 记录错误，影响 mood（grumpy 方向）
- `session.idle` — 推进轮次，影响 mood（recenter 向默认值）
- `todo.updated` — 记录 todo 变更，影响 mood（完成任务重置 roundsWithoutCompletion）

---

## 5. `dcp-forget` 插件代码结构

### 5.1 目录结构

`~/.config/opencode/plugins/dcp-forget/` 是一个**纯脚本目录**（无独立插件入口 index.ts），其功能已合并到 `runtime-orchestrator` 插件中。

```
dcp-forget/
└── scripts/
    ├── index.mjs           # barrel export，聚合所有模块 (5 行)
    ├── dcp-state.mjs       # 状态管理：loadDcpState, saveDcpState, allocateBlockId, applyCompressionState, dcpPruneMessages
    ├── dcp-message-ids.mjs # 消息 ID 分配：assignMessageRefs, dcpExtractPlain
    ├── dcp-search.mjs      # 搜索/选择：buildSearchContext, resolveSelection, resolveBoundaryIds, buildBoundaryLookup, validateNonOverlapping
    ├── dcp-compress-tool.mjs # 压缩工具主逻辑：compressRanges, validateArgs
    └── dcp-block-placeholders.mjs # block 占位符：injectBlockPlaceholders, parseBlockPlaceholders
```

### 5.2 入口文件

- **不存在独立的 `index.ts`** — `dcp-forget` 本身不通过 opencode 插件工厂直接加载
- 运行时的调用链为：`runtime-orchestrator/index.ts` L15-21 → `import { ... } from "../dcp-forget/scripts/index.mjs"` → 加载所有脚本模块

### 5.3 关键脚本功能

| 脚本文件 | 核心导出 | 用途 |
|---|---|---|
| `scripts/index.mjs` | barrel export (L1-5) | 统一导出所有模块 |
| `scripts/dcp-state.mjs` | `loadDcpState()`, `saveDcpState()`, `allocateBlockId()`, `applyCompressionState()`, `dcpPruneMessages()` | JSON 文件级别的状态持久化（`dcp-forget-state.json`） |
| `scripts/dcp-message-ids.mjs` | `assignMessageRefs()`, `dcpExtractPlain()` | 给消息分配引用 ID（mNNNN），提取纯文本表示 |
| `scripts/dcp-search.mjs` | `buildSearchContext()`, `resolveSelection()`, `resolveBoundaryIds()`, `buildBoundaryLookup()`, `validateNonOverlapping()` | 构建搜索上下文、解析消息边界、验证范围不重叠 |
| `scripts/dcp-compress-tool.mjs` | `compressRanges()`, `validateArgs()` | 压缩工具主逻辑：接收 range 数组，生成压缩 block，写入状态 |
| `scripts/dcp-block-placeholders.mjs` | `injectBlockPlaceholders()`, `parseBlockPlaceholders()` | 将压缩后的 block 注入消息流，使用 `[DCP_BLOCK:bN]` 标记 |

### 5.4 在 runtime-orchestrator 中的集成

`~/.config/opencode/plugins/runtime-orchestrator/index.ts`：
- L54-58：初始化时加载 `dcpState`（来自 `dcp-forget-state.json`）
- L298-303：在 `chat.messages.transform` hook 中，跟踪可见消息引用，执行 `dcpPruneMessages()`
- L338-374：提供 `compact_check` 工具，读取 dcp 状态返回压缩建议
- L376-399：提供 `compress` 工具，调用 `compressRanges()` 执行实际压缩
- L311-330：在 `event` hook 中监听 `session.idle`，更新 turn 计数并触发 dcp 持久化

### 5.5 墓碑机制

之前版本中 `dcp-forget` 包含墓碑（tombstone）机制，用于记录被遗忘的消息。但经过多轮迭代后：
- 墓碑已关闭：`ENABLE_FORGET_TOMBSTONES=false`
- 当前仅保留 `compress`、`compact_check`、`forget_query`(metadata-only)、`forget_lookup` 功能
- 相关决策见 compression block `b6` 的摘要：*"设计拍板：基于'不可恢复的墓碑没有意义'，决定最小修改 dcp-forget，直接关闭墓碑机制，但保留 compact/compress"*
