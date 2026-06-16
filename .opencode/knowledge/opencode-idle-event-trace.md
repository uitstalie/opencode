# opencode `session.idle` 事件追踪研究

> 基于 `/mnt/d/dqc/fake_opencode/source-code/opencode/` 源码只读分析
> 日期：2026-06-15

---

## 1. `session.idle` 事件：定义与 Publish

### 定义位置

**文件**：`packages/opencode/src/session/status.ts`  
**行号**：35-50

```ts
export const Event = {
  Status: EventV2.define({
    type: "session.status",
    schema: {
      sessionID: SessionID,
      status: Info,
    },
  }),
  // deprecated
  Idle: EventV2.define({
    type: "session.idle",
    schema: {
      sessionID: SessionID,
    },
  }),
}
```

**结论**：
- `Event.Idle` 即 `"session.idle"` 事件，schema 为 `{ sessionID: SessionID }`
- **已被标记为 `deprecated`**（第 43 行注释），新事件 `Event.Status` (`"session.status"`) 包含更丰富的 `status: Info` 字段（可取 `"idle"`、`"busy"`、`"retry"`）

### Publish 发送方

**文件**：`packages/opencode/src/session/status.ts`  
**行号**：78-87

```ts
const set = Effect.fn("SessionStatus.set")(function* (sessionID: SessionID, status: Info) {
  const data = yield* InstanceState.get(state)
  yield* events.publish(Event.Status, { sessionID, status })        // 总是 publish status 事件
  if (status.type === "idle") {
    yield* events.publish(Event.Idle, { sessionID })                // idle 时额外 publish 旧版事件
    data.delete(sessionID)
    return
  }
  data.set(sessionID, status)
})
```

**结论**：
- **每次状态变更都会 publish `Event.Status`**（新事件）
- **仅在状态变为 `"idle"` 时额外 publish `Event.Idle`**（旧事件）
- `set` 方法是唯一 publish `session.idle` 的入口——没有其他地方直接 publish 此事件

### 事件桥接层

**文件**：`packages/opencode/src/event-v2-bridge.ts`（79 行）  
**行号**：38-67

```ts
const unsubscribe = yield* events.listen((event) =>
  Effect.gen(function* () {
    GlobalBus.emit("event", {
      directory: event.location?.directory ?? ctx?.directory,
      project: ctx?.project.id,
      workspace: workspaceID,
      payload: { id: event.id, type: event.type, properties: event.data },
    })
    // ... sync event handling
  }),
)
```

**结论**：
- EventV2Bridge 将所有 EventV2 事件通过 `GlobalBus.emit("event", ...)` 转发给 SDK/前端消费
- `session.idle` 事件的 `properties` 只包含 `{ sessionID: string }`（无 session.title 等上下文）

### 消费者

**文件**：`packages/app/src/context/notification.tsx`  
**行号**：289-296（app 端）、245-254（handleSessionIdle）

```ts
// 监听
const unsub = serverSDK().event.listen((e) => {
  const event = e.details
  if (event.type !== "session.idle" && event.type !== "session.error") return
  if (event.type === "session.idle") {
    handleSessionIdle(directory, event, time)
    return
  }
  // ...
})

// 处理：播放声音 + 桌面通知
const handleSessionIdle = (directory, event, time) => {
  lookup(directory, sessionID).then((session) => {
    if (!session || session.parentID) return   // 子 session 不通知
    playSoundById(settings.sounds.agent())
    append({ type: "turn-complete", session: sessionID })
    platform.notify("响应就绪", session.title)
  })
}
```

**结论**：
- 前端 App 使用 `session.idle` 做通知提醒（声音 + 系统通知）
- `parentID` 不为空的子 session idle 不触发通知

---

## 2. `overflow` / `isOverflow` 调用链

### 纯函数定义

**文件**：`packages/opencode/src/session/overflow.ts`（34 行）  
**行号**：8-34

```ts
const COMPACTION_BUFFER = 20_000

export function isOverflow(input: {
  cfg: ConfigV1.Info
  tokens: SessionV1.Assistant["tokens"]
  model: Provider.Model
  outputTokenMax?: number
}) {
  if (input.cfg.compaction?.auto === false) return false    // 手动关闭自动压缩
  if (input.model.limit.context === 0) return false           // 无上下文限制
  const count = input.tokens.total ||
    input.tokens.input + input.tokens.output + input.tokens.cache.read + input.tokens.cache.write
  return count >= usable(input)                                // token >= 可用上限
}
```

**结论**：纯数学判断——token 总量是否超过模型可用上下文减去缓冲（20K 或 outputTokenMax）。

### Effect 包装

**文件**：`packages/opencode/src/session/compaction.ts`  
**行号**：178-188

```ts
const isOverflow = Effect.fn("SessionCompaction.isOverflow")(function* (input: {
  tokens: SessionV1.Assistant["tokens"]
  model: Provider.Model
}) {
  return overflow({
    cfg: yield* config.get(),
    tokens: input.tokens,
    model: input.model,
    outputTokenMax: flags.outputTokenMax,
  })
})
```

### 调用链：LLM 回复完成 → overflow 检测

**调用点 1**：`packages/opencode/src/session/processor.ts` 行 750-754

```ts
// 在处理 LLM 流结束事件时 (case "finish"):
if (
  !ctx.assistantMessage.summary &&
  isOverflow({ cfg: yield* config.get(), tokens: usage.tokens, model: ctx.model })
) {
  ctx.needsCompaction = true    // 标记需要压缩
}
```

**调用点 2**：`packages/opencode/src/session/prompt.ts` 行 1214-1221（run loop 中）

```ts
if (
  lastFinished &&
  lastFinished.summary !== true &&
  (yield* compaction.isOverflow({ tokens: lastFinished.tokens, model }))
) {
  yield* compaction.create({ sessionID, agent: lastUser.agent, model: lastUser.model, auto: true })
  continue    // 回到 loop 顶部，下一轮进入 task.type === "compaction" 分支
}
```

### 完整调用链总结

```
处理器 LLM 流结束 (processor.ts:750)
  ↓ 检测 isOverflow → ctx.needsCompaction = true
  ↓ 返回 "compact" (processor.ts:1030)
  ↓
prompt loop (prompt.ts:1202-1211)
  ↓ task.type === "compaction"
  ↓ compaction.process({ auto: true, overflow: ... })
  ↓
压缩处理 (compaction.ts:299-552)
  ↓ 调用 LLM 做摘要压缩
  ↓ result === "continue" ? 继续 : 停止
  ↓
autocontinue 逻辑 (compaction.ts:473-525)
  ↓ 如果 auto=true 且插件允许 → 创建合成 "continue" 用户消息
  ↓
返回 "continue" → prompt loop 继续下一轮
  ↓ 合成消息作为 lastUser → 正常 LLM 调用
  ↓ ... 直到 loop 自然退出 ...
  ↓
run loop 退出 (prompt.ts:1181 break)
  ↓
Runner 完成 (runner.ts:70-81 finishRun)
  ↓ onIdle 回调
  ↓
run-state.ts:62: status.set(sessionID, { type: "idle" })
  ↓
status.ts:82: events.publish(Event.Idle, { sessionID })
  ↓
EventV2Bridge → GlobalBus → 前端 App 收到通知
```

---

## 3. 压缩完成后是否自动发 idle 事件

**关键结论：不直接在压缩完成时发 idle 事件。**

### 时间线说明

| 阶段 | idle 事件？ | 说明 |
|------|------------|------|
| 压缩开始前 | ❌ | session 状态为 `busy`（由 RunLoop 在开始时设置） |
| 压缩处理中 | ❌ | 状态仍为 `busy`，无 idle |
| 压缩完成（`process` 返回 `"continue"`） | ❌ | 状态仍为 `busy`，loop 继续迭代 |
| autocontinue 合成消息后 | ❌ | loop 继续，新 LLM 调用开始 |
| 整个 run loop 退出 | ✅ **在这里发 idle** | `Runner.finishRun` → `onIdle` → `status.set(idle)` |

### 具体代码证据

**run-state.ts 行 59-66**：
```ts
const next = Runner.make<SessionV1.WithParts>(data.scope, {
  onIdle: Effect.gen(function* () {
    data.runners.delete(sessionID)
    yield* status.set(sessionID, { type: "idle" })  // ← 这里才发送 idle
  }),
  onBusy: status.set(sessionID, { type: "busy" }),
  onInterrupt,
})
```

**runner.ts 行 70-81**（`finishRun`）：
```ts
const finishRun = (id, done, exit) =>
  SynchronizedRef.modify(ref, (st) => [
    Effect.gen(function* () {
      if (st._tag === "Running" && st.run.id === id) yield* idle  // ← 转换到 Idle 状态
      yield* complete(done, exit)
    }),
    st._tag === "Running" && st.run.id === id ? ({ _tag: "Idle" } : st,
  ])
```

**结论**：
- 对于 **非 autocontinue** 的压缩（`auto=true` 但 autocontinue 被禁用 → `result === "continue"` 但无合成消息 → 无新 user 消息），loop 会在下一轮找到 `lastAssistant.finish` 为已完成，自然退出 → 发 idle
- 对于 **autocontinue** 的压缩，压缩 → 合成消息 → LLM 回复 → ... → 最终完成 → 发 idle。**整个过程作为一次连续的 RunLoop 运行，中间不发 idle。**
- 这意味着如果你在 `session.idle` 事件中做 hook，你不会在 auto-compaction 的"中间点"收到事件——只在整个会话链结束时收到一次。

---

## 4. `autocontinue` 逻辑

### 触发条件

**文件**：`packages/opencode/src/session/compaction.ts`  
**行号**：444-526

必须在以下**全部**条件满足时触发：

1. 压缩 `process` 返回 `"continue"`（即 LLM 成功完成摘要）
2. `input.auto === true`（自动压缩，而非手动触发）
3. 插件 hook `"experimental.compaction.autocontinue"` 返回 `{ enabled: true }`（默认为 `true`）
4. 非 replay 场景（`if (!replay)`）

### 核心代码

```ts
// compaction.ts:473-525
if (!replay) {
  const info = yield* provider.getProvider(userMessage.model.providerID)
  if (
    (yield* plugin.trigger(
      "experimental.compaction.autocontinue",
      {
        sessionID: input.sessionID,
        agent: userMessage.agent,
        model: yield* provider.getModel(...).pipe(Effect.orDie),
        provider: { source: info.source, info, options: info.options },
        message: userMessage,
        overflow: input.overflow === true,
      },
      { enabled: true },    // ← 默认启用！
    )).enabled
  ) {
    const continueMsg = yield* session.updateMessage({
      id: MessageID.ascending(),
      role: "user",
      sessionID: input.sessionID,
      agent: userMessage.agent,
      model: userMessage.model,
    })
    const text =
      (input.overflow
        ? "The previous request exceeded the provider's size limit due to large media attachments. ..."
        : "") +
      "Continue if you have next steps, or stop and ask for clarification if you are unsure how to proceed."
    yield* session.updatePart({
      // ...
      type: "text",
      metadata: { compaction_continue: true },    // ← 内部标记
      synthetic: true,
      text,
    })
  }
}
```

### 合成消息特征

- `role: "user"`（模拟用户消息）
- `synthetic: true`（标识为系统生成）
- `metadata: { compaction_continue: true }`（内部标记，非稳定 API）
- 文本内容：`"Continue if you have next steps, or stop and ask for clarification..."`（overflow 时前缀额外说明）
- 消息 ID 新分配（`MessageID.ascending()`）

### 插件 hook 定义

**文件**：`packages/plugin/src/index.ts` 行 310-326

```ts
/**
 * Called after compaction succeeds and before a synthetic user
 * auto-continue message is added.
 *
 * - `enabled`: Defaults to `true`. Set to `false` to skip the synthetic
 *   user "continue" turn.
 */
"experimental.compaction.autocontinue"?: (
  input: {
    sessionID: string
    agent: string
    model: Model
    provider: ProviderContext
    message: UserMessage
    overflow: boolean
  },
  output: { enabled: boolean },
) => Promise<void>
```

### 关键行为总结

| 场景 | 行为 |
|------|------|
| 自动压缩 + 默认 autocontinue | 压缩后自动注入 continue 消息，模型继续执行 |
| 自动压缩 + 插件禁用 autocontinue | 压缩后不发合成消息，loop 退出 → idle 事件 |
| 手动压缩 (`auto=false`) | 不触发 autocontinue，loop 退出 → idle |
| `overflow === true` 场景 | 合成消息前缀额外说明 media 附件被移除 |

---

## 5. 总体架构全景

```
用户发送 prompt
  ↓
SessionPrompt.prompt() → RunLoop (prompt.ts)
  ↓                        ↓
  status.set(busy)    Runner.ensureRunning(work)
  ↓                        ↓
  processor.process()      onBusy → status.set(busy)
  ↓                        ↓
  每轮 LLM 完成后：          work 完成 →
  - isOverflow 检查         finishRun → idle 回调
  - needsCompaction?        ↓
  ↓                    status.set(idle)
  返回 "compact" /       ↓
  "continue" / "stop"   publish session.status(busy→idle)
  ↓                     + publish session.idle (deprecated)
  loop 中：              ↓
  - compaction 任务     GlobalBus → 前端通知
  - autocontinue 合成消息
  - 正常 LLM 调用
  ↓
  loop 退出 → work 完成
```

### 新/旧事件区别

| 事件 | 类型 | 状态 |
|------|------|------|
| `session.status` | `Event.Status` | ✅ 当前推荐，含完整状态信息 |
| `session.idle` | `Event.Idle` | ⚠️ deprecated，仅 idle 时附加发送 |
| `session.error` | `Session.Event.Error` | ✅ 错误事件 |
| `session.compacted` | `SessionCompaction.Event.Compacted` | ✅ 压缩完成事件 |

### 依赖关系

```
SessionStatus (status.ts)
  └─ 依赖 EventV2Bridge → EventV2 (core/event.ts) → PubSub + SQLite
  └─ 被依赖：
       ├─ SessionRunState (run-state.ts) → onIdle/onBusy 回调
       ├─ SessionProcessor (processor.ts:931,957) → 错误时的 idle
       └─ SessionPrompt (prompt.ts:1142) → busy 状态设置
```

---

## 6. 实践建议

1. **新代码使用 `session.status` 事件**而非 `session.idle`，后者已标记 deprecated
2. **监听 idle 的正确方式**：subscribe `session.status` 过滤 `status.type === "idle"`
3. **不能在压缩"中间点"做 hook**：auto-compaction + autocontinue 场景下，压缩→续写作为连续 process 运行，idle 只在末端发一次
4. **禁用 autocontinue 的方法**：通过插件实现 `"experimental.compaction.autocontinue"` hook 返回 `{ enabled: false }`
5. **overflow 区别于普通压缩**：overflow 场景会 replay 导致溢出的那条用户消息（去除 media），并在合成消息前附加说明

---

> 所有代码引用来自本地只读副本：`/mnt/d/dqc/fake_opencode/source-code/opencode/`
> 源码时效性：基于 opencode 源码树，具体 commit 未记录
