# opencode Event 分发深度分析：插件 event hook 为何收不到 session 事件

> **分析日期**: 2026-06-15
> **源码基准**: `packages/opencode/src/` + `packages/core/src/`
> **分析范围**: TUI 模式下，插件的 `event` hook（服务端 plugin）对 EventV2 事件的完整接收链路

---

## 一、架构概览：TUI 模式下的双进程模型

```
┌─ 主进程 (TUI) ──────────────────────────┐
│  @opencode-ai/tui run()                  │
│  createLegacyTuiPluginHost()             │  ← TUI 端插件系统
│  transport.events → RPC                  │
└────────────┬────────────────────────────┘
             │ RPC (Web Worker message)
┌─ Worker 进程 ───────────────────────────┐
│  AppRuntime (EventV2 + Plugin + Session) │
│  Server.listen() HTTP 服务               │
│  GlobalBus → RPC 转发事件到 TUI          │
│  plugin/index.ts 注册 event 监听         │ ← 服务端插件系统 (本次分析重点)
└─────────────────────────────────────────┘
```

**关键结论**: 服务端插件（`plugin/index.ts`）直接在 Worker 进程中监听 EventV2 事件，与 TUI 端插件走的是两套独立系统。

---

## 二、完整调用链路

### 步骤 1: TUI 启动 → Worker 初始化

**文件**: `packages/opencode/src/cli/cmd/tui.ts:182-184`
```ts
setTimeout(() => {
    client.call("checkUpgrade", { directory: cwd }).catch(() => {})
}, 1000).unref?.()
```

**文件**: `packages/opencode/src/cli/tui/worker.ts:52-55`
```ts
async checkUpgrade(input: { directory: string }) {
    await InstanceRuntime.load({ directory: input.directory })
    await upgrade().catch(() => {})
},
```

`InstanceRuntime.load` → `InstanceStore.load` → `InstanceStore.boot`：

**文件**: `packages/opencode/src/project/instance-store.ts:45-63`
```ts
const boot = (input: LoadInput & { directory: string }) =>
    Effect.gen(function* () {
        const ctx: InstanceContext = { directory, worktree, project }
        yield* bootstrap.run.pipe(Effect.provideService(InstanceRef, ctx))  // ← 注入 InstanceRef
        return ctx
    })
```

> **关键点**: `InstanceRef` 在 `boot` 中通过 `Effect.provideService` 注入到 `bootstrap.run` 的 Effect 上下文中。此后所有同步 Effect 操作均可通过 `yield* InstanceRef` 获取该上下文。

---

### 步骤 2: Bootstrap → Plugin.init

**文件**: `packages/opencode/src/project/bootstrap.ts:32-38`
```ts
const run = Effect.gen(function* () {
    const ctx = yield* InstanceState.context  // ← 从 InstanceRef 取
    yield* config.get()
    yield* plugin.init()                       // ← 先于其他服务 init
    yield* Effect.forEach([lsp, shareNext, format, vcs, snapshot, project],
        (s) => s.init(), { concurrency: "unbounded", discard: true })
})
```

> **关键点**: `plugin.init()` 在所有其他服务之前完成，确保插件监听器在其他服务可能发布事件之前就已注册。

---

### 步骤 3: Plugin.init → 注册事件监听器

**文件**: `packages/opencode/src/plugin/index.ts:126, 251-258`
```ts
const events = yield* EventV2Bridge.Service   // ← 获取的是 EventV2Bridge

const unsubscribe = yield* events.listen((event) => {
    // ★ 核心过滤器 ★
    if (event.location?.directory !== ctx.directory) return Effect.void
    return Effect.sync(() => {
        for (const hook of hooks) {
            void hook["event"]?.({ event: { id: event.id, type: event.type, properties: event.data } })
        }
    })
})
```

> **关键问题 1**: `events.listen` 是什么？

---

### 步骤 4: EventV2Bridge 的 listen 实际上是 EventV2 的 listen

**文件**: `packages/opencode/src/event-v2-bridge.ts:15-73`
```ts
export const layer = Layer.effect(Service, Effect.gen(function* () {
    const events = yield* EventV2.Service   // ← 核心 EventV2

    const publish: EventV2.Interface["publish"] = (...) => Effect.gen(function* () {
        // ... 添加 location
    })

    return Service.of({ ...events, publish })  // ← spread events，只覆盖 publish
}))
```

`{ ...events, publish }` 展开 `EventV2.Service` 的全部方法（`listen`, `subscribe`, `sync`, `all`, …），仅 `publish` 被覆盖。

> **结论**: `EventV2Bridge.Service.listen` **就是** `EventV2.Service.listen`。插件的监听器直接推入核心 EventV2 的 `listeners` 数组。

---

### 步骤 5: EventV2.listen (核心) 的实现

**文件**: `packages/core/src/event.ts:630-637`
```ts
const listen = (listener: Listener): Effect.Effect<Unsubscribe> =>
    Effect.sync(() => {
        listeners.push(listener)
        return Effect.sync(() => {
            const index = listeners.indexOf(listener)
            if (index >= 0) listeners.splice(index, 1)
        })
    })
```

> **结论**: 插件的回调函数被同步推入 `listeners: Listener[]` 数组。所有发布的事件都会遍历此数组。

---

### 步骤 6: 事件发布路径 — 两条路

#### 路径 A: opencode 包内 → EventV2Bridge.publish（主流）

**文件**: `packages/opencode/src/session/session.ts:538,577`
```ts
const events = yield* EventV2Bridge.Service
yield* events.publish(SessionV1.Event.Created, { sessionID, info })
```

**文件**: `packages/opencode/src/session/prompt.ts:124`
```ts
const events = yield* EventV2Bridge.Service
yield* events.publish(Session.Event.Error, { sessionID, error })
```

**文件**: `packages/opencode/src/session/processor.ts:106,147-158`
```ts
const events = yield* EventV2Bridge.Service
yield* events.publish(SessionEvent.Step.Started, { ... })
```

所有的 opencode 包内会话相关代码（`session.ts`, `prompt.ts`, `processor.ts`, `todo.ts`, `compaction.ts`, `revert.ts`, `summary.ts`, `status.ts`, `llm.ts`）均使用 `EventV2Bridge.Service`。

**EventV2Bridge.publish 覆盖版本**:

**文件**: `packages/opencode/src/event-v2-bridge.ts:22-36`
```ts
const publish = (definition, data, options) =>
    Effect.gen(function* () {
        if (options?.location) return yield* events.publish(definition, data, options)
        const ctx = yield* InstanceRef                      // ← 从 Effect 上下文取
        if (!ctx) return yield* events.publish(definition, data, options)  // ← 无 InstanceRef 时无 location!
        const workspaceID = yield* WorkspaceRef
        return yield* events.publish(definition, data, {
            ...options,
            location: new Location.Info({                    // ← 添加 location
                directory: AbsolutePath.make(ctx.directory),
                workspaceID,
                project: { id: ..., directory: ... },
            }),
        })
    })
```

> **关键点**: 当且仅当 `InstanceRef` 在 Effect 上下文中可用时，事件才会被附加 `location`。

#### 路径 B: core 包内 → EventV2.publish（直接）

**文件**: `packages/core/src/event.ts:431-451`
```ts
function publish(definition, data, options?) {
    return Effect.gen(function* () {
        const serviceLocation = Option.getOrUndefined(
            yield* Effect.serviceOption(Location.Service)  // ← 依赖 Location.Service
        )
        const location = options?.location
            ?? (serviceLocation ? { directory: serviceLocation.directory, ... } : undefined)
        // ... location 可能为 undefined
    })
}
```

> **关键点**: core 的 `EventV2.publish` 依赖 `Location.Service` 获取 location。如果 `Location.Service` 不在 Effect 上下文中，事件 **没有 location**。

**在 TUI 模式下，core 的 SessionExecution 是 noop**:

**文件**: `packages/opencode/src/session/session.ts:970`
```ts
Layer.provide(SessionExecution.noopLayer),  // V2 执行器禁用
```

所以路径 B 在 TUI 模式下实际上不会被触发（SessionExecution 是 noop），所有会话事件都走路径 A。

---

### 步骤 7: notify → 遍历 listeners

**文件**: `packages/core/src/event.ts:418-429`
```ts
function notify(event: Payload, isolateListeners: boolean) {
    return Effect.gen(function* () {
        yield* Effect.forEach(
            listeners,                                          // ← 遍历所有监听器
            (listener) => (isolateListeners
                ? observe(event, "listener", listener)          // sync 事件: Effect.suspend 包装
                : listener(event)                               // 非 sync 事件: 直接调用
            ),
            { discard: true },
        )
        // 同时发布到 PubSub (typed + all)
        ...
    })
}
```

`listeners` 数组中包含：
1. EventV2Bridge 自身的监听器（转发到 GlobalBus）
2. Plugin 的监听器

两个都会被调用。

---

## 三、核心问题分析

### 问题 1: 事件是否有 location？

| 事件 | 发布位置 | 发布接口 | location 来源 |
|------|---------|---------|--------------|
| `session.created` | `session.ts:577` | EventV2Bridge | `InstanceRef` ✅ |
| `session.updated` | `session.ts:788` | EventV2Bridge | `InstanceRef` ✅ |
| `session.deleted` | `session.ts:664` | EventV2Bridge | `InstanceRef` ✅ |
| `session.error` | `prompt.ts:302`, `processor.ts:930` | EventV2Bridge | `InstanceRef` ✅ |
| `session.diff` | `revert.ts:77`, `summary.ts:114` | EventV2Bridge | `InstanceRef` ✅ |
| `session.next.step.started` | `processor.ts:147` | EventV2Bridge | `InstanceRef` ✅ (需 `experimentalEventSystem`) |
| `session.next.text.*` | `processor.ts:271` | EventV2Bridge | `InstanceRef` ✅ (需 `experimentalEventSystem`) |
| `session.status` | `status.ts:80` | EventV2Bridge | `InstanceRef` ✅ |
| `todo.updated` | `todo.ts:64` | EventV2Bridge | `InstanceRef` ✅ |
| `session.next.model.switched` | `core/session.ts:388` | **EventV2 (core)** | `Location.Service` ⚠️ (open in opencode? 可能无 location) |

### 问题 2: 插件的过滤器

**文件**: `packages/opencode/src/plugin/index.ts:251-252`
```ts
if (event.location?.directory !== ctx.directory) return Effect.void
```

- `event.location?.directory` = `AbsolutePath.make(ctx.directory)` → **branded string**
- `ctx.directory` = `InstanceContext.directory` → **plain string**
- JavaScript 层面两者是同一字符串值（`AbsolutePath` 是 branded type，运行时就是 string），`!==` 比较正常

> ⚠️ **但是！** 如果事件 **没有 location**（即 `event.location === undefined`），则 `event.location?.directory` = `undefined`，而 `ctx.directory` = 某个路径字符串。`undefined !== "/path/to/project"` → **`true`** → 过滤器返回 `Effect.void` → **事件被丢弃！**

### 问题 3: 哪些场景下 event.location 为 undefined？

#### 场景 A: `InstanceRef` 不在 Effect 上下文中

如果发布事件的代码不在 `InstanceStore.boot` 提供的 `Effect.provideService(InstanceRef, ctx)` 范围内，则 `EventV2Bridge.publish` 的第 26 行 `const ctx = yield* InstanceRef` 取到 `undefined`（默认值），然后走第 27 行：
```ts
if (!ctx) return yield* events.publish(definition, data, options)  // 无 location!
```

可能的来源：
- 在 `EffectBridge` 的 callback 之外裸调用的代码
- 未通过 `InstanceStore.provide()` 包装的 Effect

#### 场景 B: core 包直接调用 `EventV2.publish`

`core/src/session.ts:388`（`switchModel`）使用 `EventV2.Service.publish` 发布事件，不经过 EventV2Bridge：

```ts
yield* events.publish(SessionEvent.ModelSwitched, { ... })  // 无 location 参数！
```

这里 `EventV2.publish` 会尝试从 `Location.Service` 获取 location。如果 `Location.Service` 在当前 Effect 上下文中不可用，事件就没有 location。

#### 场景 C: core 包的其他位置

**文件**: `packages/core/src/session/runner/llm.ts:93`
```ts
const events = yield* EventV2.Service  // ← core 的，非 bridge
```

但此代码属于 V2 `SessionRunner`，而 opencode TUI 模式使用 `SessionExecution.noopLayer`，故此路径不会被触发。

---

## 四、关键发现总结

### 主路径（TUI 模式下的正常流程）

```
HTTP 请求 → instance-context middleware (提供 InstanceRef)
  → SessionPrompt / SessionProcessor
    → EventV2Bridge.Service.publish
      → 检查 InstanceRef → 添加 location
        → EventV2.Service.publish
          → publishEvent → notify → 遍历 listeners
            → Plugin listener 回调
              → 检查 event.location?.directory !== ctx.directory
                → 匹配 → 调用 hook["event"]
```

在此路径下，事件有 location，过滤器通过，插件可以收到事件 ✅

### 可能导致插件收不到事件的原因

1. **`experimentalEventSystem` 未启用**（最可能的原因）
   - `processor.ts:129`: `mirrorAssistant = flags.experimentalEventSystem && ...`
   - 所有 V2 会话事件（`session.next.step.started`, `session.next.text.*`, `session.next.reasoning.*`, `session.next.tool.*`）都受此 flag 控制
   - 需要设置环境变量：`OPENCODE_EXPERIMENTAL=1` 或 `OPENCODE_EXPERIMENTAL_EVENT_SYSTEM=1`

2. **事件无 location**（次可能的原因）
   - 如果发布事件的代码不在 `InstanceRef` 上下文中执行
   - 事件通过 core `EventV2.Service.publish` 发布且 `Location.Service` 不可用

3. **目录不匹配**（较少见）
   - 如果 `ctx.directory`（来自 `InstanceContext.directory`）与 `event.location.directory`（来自 `AbsolutePath.make(ctx.directory)`）不相等
   - 通常不会发生（同一来源），但路径规范化（`FSUtil.resolve`）可能有影响

4. **时序问题**（可能性低）
   - `plugin.init()` 在 bootstrap 中先于其他服务完成，监听器在其他服务发布事件前已注册
   - HTTP 中间件在处理器执行前已完成 `InstanceStore.load`（包含 `plugin.init`）

---

## 五、建议排查步骤

1. **确认 flag 状态**：设置 `OPENCODE_EXPERIMENTAL_EVENT_SYSTEM=1` 或 `OPENCODE_EXPERIMENTAL=1` 环境变量

2. **验证 V1 事件**：即使不启 `experimentalEventSystem`，`session.created`、`session.updated`、`session.deleted`、`session.error` 等 V1 事件也应能被接收。如果这些也收不到，则是 location 或目录匹配问题。

3. **在 plugin/index.ts:251 添加日志**：
   ```ts
   yield* Effect.logInfo("plugin event received", {
       id: event.id, type: event.type,
       locationDir: event.location?.directory,
       ctxDir: ctx.directory,
   })
   ```

4. **检查 `Location.Service`**：搜索是否有代码在核心 EventV2 上发布事件但不提供 Location.Service 或 InstanceRef

---

## 六、源码位置索引

| 组件 | 文件 | 行号 |
|------|------|------|
| CLI 入口 | `packages/opencode/src/index.ts` | — |
| TUI 命令 | `packages/opencode/src/cli/cmd/tui.ts` | 71-223 |
| TUI Worker | `packages/opencode/src/cli/tui/worker.ts` | 52-55 |
| InstanceStore.boot | `packages/opencode/src/project/instance-store.ts` | 45-63 |
| Bootstrap.run | `packages/opencode/src/project/bootstrap.ts` | 32-46 |
| Plugin 层 (事件注册) | `packages/opencode/src/plugin/index.ts` | 123-278 |
| Plugin 事件过滤器 | `packages/opencode/src/plugin/index.ts` | 251-258 |
| EventV2Bridge 层 | `packages/opencode/src/event-v2-bridge.ts` | 17-73 |
| EventV2Bridge.publish | `packages/opencode/src/event-v2-bridge.ts` | 22-36 |
| EventV2 (核心) publish | `packages/core/src/event.ts` | 431-451 |
| EventV2 (核心) listen | `packages/core/src/event.ts` | 630-637 |
| EventV2 (核心) notify | `packages/core/src/event.ts` | 418-429 |
| EventV2 (核心) publishEvent | `packages/core/src/event.ts` | 385-407 |
| Session 服务 (V1 事件) | `packages/opencode/src/session/session.ts` | 538,577 |
| SessionPrompt (V1 错误) | `packages/opencode/src/session/prompt.ts` | 124,302 |
| SessionProcessor (V2 事件) | `packages/opencode/src/session/processor.ts` | 106,129,147 |
| SessionV1.Event 定义 | `packages/core/src/v1/session.ts` | 573-631 |
| SessionEvent (V2) 定义 | `packages/core/src/session/event.ts` | 1-511 |
| EffectBridge (上下文保存) | `packages/opencode/src/effect/bridge.ts` | 7-82 |
| InstanceRef 定义 | `packages/opencode/src/effect/instance-ref.ts` | 1-10 |
| RuntimeFlags | `packages/opencode/src/effect/runtime-flags.ts` | 48 |
| InstanceState | `packages/opencode/src/effect/instance-state.ts` | 1-68 |
| Location.Info / Location.Ref | `packages/core/src/location.ts` | 8-20 |
| AbsolutePath Schema | `packages/core/src/schema.ts` | 31 |
