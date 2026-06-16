# opencode 插件 Event Hook 调度机制分析

> **分析日期**: 2026-06-15
> **源码基准**: `packages/opencode/src/` + `packages/core/src/` + `packages/plugin/src/`

---

## 一、插件 Event Hook 的注册与调用链路

### 1.1 Hook 类型定义

来源: `packages/plugin/src/index.ts:222-224`

```ts
export interface Hooks {
  dispose?: () => Promise<void>
  event?: (input: { event: Event }) => Promise<void>
  config?: (input: Config) => Promise<void>
  // ... 其他 hook
}
```

- `event` 是一个**可选**的异步函数，签名为 `(input: { event: Event }) => Promise<void>`
- `Event` 类型来自 `@opencode-ai/sdk` 的 v1 或 v2 生成的联合类型（包含 `session.compacted`、`session.next.*` 等全部事件类型）

### 1.2 Hook 收集

来源: `packages/opencode/src/plugin/index.ts:123-278`

插件初始化发生在 `Plugin.layer` → `InstanceState.make<State>()` 闭包内。该闭包执行流程：

1. **创建 `PluginInput`** (行 149-164)：包含 `client`、`project`、`directory`、`worktree` 等上下文
2. **加载内建插件** (行 166-175)：遍历 `internalPlugins(flags)`，每个内建插件的返回值 `await plugin(input)` 被推入 `hooks: Hooks[]` 数组
3. **加载外部插件** (行 177-238)：通过 `PluginLoader.loadExternal()` 加载用户配置的外部插件，每个成功加载的插件调用 `applyPlugin()` 将其 `hooks` 推入数组
4. **通知 config** (行 241-249)：遍历所有 hook，调用 `(hook as any).config?.(cfg)`
5. **注册事件监听** (行 251-258)：**核心**，在所有 hook 收集完成后注册全局事件监听

### 1.3 事件监听的注册与调用

来源: `packages/opencode/src/plugin/index.ts:251-258`

```ts
const unsubscribe = yield* events.listen((event) => {
  if (event.location?.directory !== ctx.directory) return Effect.void
  return Effect.sync(() => {
    for (const hook of hooks) {
      void hook["event"]?.({ event: { id: event.id, type: event.type, properties: event.data } as any })
    }
  })
})
yield* Effect.addFinalizer(() => unsubscribe)
```

**关键细节**：

| 特性 | 说明 |
|------|------|
| **`events` 来源** | `EventV2Bridge.Service`，它扩展自 `EventV2.Service`，通过 `events.listen()` 监听**全部** EventV2 事件 |
| **目录过滤** | **仅**过滤 `event.location?.directory !== ctx.directory`：只处理当前工作目录下的事件 |
| **钩子调用** | `void hook["event"]?.()` — **fire-and-forget**，不 await 返回的 Promise |
| **错误处理** | 没有 try/catch 包裹；如果 hook 抛同步异常，可能中断后续 hook 遍历；但 EventV2 的 `listen` 通知机制（`observe`）会 catch 非中断类错误并 log |

---

## 二、Session 类型区分：Main vs Child/Task/Sub-agent

### 2.1 结论：**框架层面不做 session 类型过滤**

- `events.listen()` 的回调中**没有**检查 `sessionID` 是否属于 main session 或 child session
- 事件数据中的 `sessionID` 字段会随事件一起传递给插件，但框架不做过滤

### 2.2 Session 层级关系

来源: `packages/core/src/session/schema.ts:27`

```ts
export class Info extends Schema.Class<Info>("SessionV2.Info")({
  id: ID,
  parentID: ID.pipe(optionalOmitUndefined),
  // ...
})
```

- 每个 session 有可选的 `parentID` 字段
- child session（task/sub-agent 产生的会话）有 `parentID`，指向父 session
- main session 的 `parentID` 为 `undefined`

来源: `packages/opencode/src/session/session.ts:563`

```ts
title: input.title ?? (input.parentID ? childTitlePrefix : parentTitlePrefix) + new Date().toISOString(),
```

### 2.3 Session-scoped 事件的 sessionID

大部分 session 级事件（如 `SessionEvent.Compaction.Ended`、`SessionEvent.Step.Started`、`SessionEvent.Tool.Called` 等）在 publish 时都会带上 `sessionID` 字段。

例如 compaction 事件 publish：
来源: `packages/opencode/src/session/compaction.ts:540,549`

```ts
yield* events.publish(SessionEvent.Compaction.Ended, {
  sessionID: input.sessionID,
  // ...
})
yield* events.publish(Event.Compacted, { sessionID: input.sessionID })
```

### 2.4 插件层面如何自行区分

如果插件只关心 main session 的事件，可以在 `event` hook 内部自行判断：

```ts
// 伪代码示例
event: async ({ event }) => {
  const sessionID = event.data?.sessionID || event.properties?.sessionID
  // 需要自行查询 session 的 parentID 来判断是否 main session
}
```

**注意**：`PluginInput.client` 提供了 SDK 客户端，可以查询 session 信息，但插件 event hook 是被动触发的，没有内置的 session 类型过滤机制。

---

## 三、Event Hook 返回值类型

### 3.1 类型定义

来源: `packages/plugin/src/index.ts:224`

```ts
event?: (input: { event: Event }) => Promise<void>
```

- 返回类型为 `Promise<void>`
- 这是一个**异步函数**，插件可以在其中做任意异步操作

### 3.2 实际调用方式

来源: `packages/opencode/src/plugin/index.ts:255`

```ts
void hook["event"]?.({ event: { id: event.id, type: event.type, properties: event.data } as any })
```

- 使用 `void` 操作符：**fire-and-forget**，不等待 Promise 完成
- 如果 event hook 返回的 Promise reject，该错误**不会被捕获**，可能成为 unhandled rejection
- 如果 hook 函数本身抛同步异常，可能中断后续 hook 的遍历（因为 `Effect.sync(() => { ... })` 内的同步异常会传播）

### 3.3 输入参数结构

```ts
{
  event: {
    id: string,        // EventV2.ID，如 "evt_xxx"
    type: string,      // 事件类型字符串，如 "session.compacted", "session.next.tool.called"
    properties: object // 事件的 data 字段（即 publish 时传入的数据），如 { sessionID, ... }
  }
}
```

---

## 四、Event Hook 的注册生命周期

### 4.1 注册时机

Event hook 的注册发生在 `Plugin` 服务的 `layer` 初始化期间，具体流程：

1. **`Plugin.layer`** 定义了一个 `Layer.effect(Service, Effect.gen(...))` (行 123-306)
2. 在 `Effect.gen` 内部，`InstanceState.make<State>()` 的闭包中完成所有初始化工作
3. `InstanceState` 采用 lazy 单例（per-directory），首次访问时才执行闭包
4. 外部通过 `Plugin.Service.init()` 触发初始化

### 4.2 相对于其他服务的顺序

来源: `packages/opencode/src/plugin/index.ts:308-314`

```ts
export const defaultLayer = layer.pipe(
  Layer.provide(EventV2Bridge.defaultLayer),
  Layer.provide(Config.defaultLayer),
  Layer.provide(RuntimeFlags.defaultLayer),
)
```

`Plugin.layer` 依赖：
- `EventV2Bridge` (底层事件系统)
- `Config` (读取插件配置)
- `RuntimeFlags` (环境标志)

### 4.3 与 Compaction 的关系

来源: `packages/opencode/src/session/compaction.ts:597-606`

```ts
export const defaultLayer = Layer.suspend(() =>
  layer.pipe(
    Layer.provide(Plugin.defaultLayer),
    // ...
  )
)
```

`SessionCompaction` 的 layer 依赖 `Plugin.defaultLayer`，这意味着**compaction 事件发生时，plugin event hook 已经注册完成**。

### 4.4 注册流程时序图

```
app bootstrap
  → Plugin.init() (forked)
    → InstanceState.make { ... }
      → Config.get()                    // 读取插件列表
      → 加载内建插件 → hooks.push(...)
      → 加载外部插件 → hooks.push(...)
      → 调用 hook.config(cfg)          // 通知配置
      → events.listen(...)             // ★ 注册全局事件监听
        → 遍历 hooks，调用 hook.event()
      → 注册 finalizer (hook.dispose)
```

---

## 五、Event Hook 不触发的可能原因

### 5.1 目录不匹配

```ts
if (event.location?.directory !== ctx.directory) return Effect.void
```

- `ctx.directory` 是插件初始化时所在的工作目录（项目根目录）
- 如果事件来自**不同目录**的项目（如不同的 workspace），会被过滤
- 如果事件发布时没有设置 `location`（为 `undefined`），**不会被过滤**（`undefined !== "some/path"` 为 `true`，但 `undefined !== undefined` 为 `false` — 即两者都是 `undefined` 时不会过滤，一有一无时会过滤）

### 5.2 插件加载失败

- 内建插件加载失败 (行 167-173)：被 `catch` 后仅 `Effect.option`，不会推入 hooks
- 外部插件加载失败 (行 220-237)：被 `Effect.tryPromise` 包裹，catch 后 log error 但继续
- **症状**：事件监听仍然注册，但 `hooks` 数组中缺少该插件

### 5.3 `event` 属性未定义

```ts
void hook["event"]?.({ event: ... })
```

- 使用了可选链 `?.`，如果插件返回的 `Hooks` 对象没有 `event` 属性，静默跳过
- 检查插件是否正确导出了 `event` 方法

### 5.4 事件类型不是 publish 到 EventV2 的

- 插件 event hook 只订阅通过 `EventV2.Service.publish()` / `EventV2Bridge.Service.publish()` 发布的事件
- `SessionEvent.*` 系列事件（如 `session.next.*`）是通过这个渠道发布的 ✓
- 某些内部事件（如通过 `GlobalBus.emit()` 直接发送的）**不经过**这个渠道 ✗
- 已确认的所有 SessionEvent 发布点：

| 事件类型 | Publish 位置 |
|----------|-------------|
| `SessionEvent.Compaction.Started` | `compaction.ts:578` |
| `SessionEvent.Compaction.Ended` | `compaction.ts:540` |
| `Event.Compacted` (`session.compacted`) | `compaction.ts:549` |
| `SessionEvent.Step.Started/Ended/Failed` | `processor.ts:147,704,941` |
| `SessionEvent.Tool.Called/Success/Failed` | `processor.ts:490,631,556` |
| `SessionEvent.Text.*` | `processor.ts:763,789,822` |
| `SessionEvent.Reasoning.*` | `processor.ts:377,403,252` |
| `SessionEvent.Tool.Input.*` | `processor.ts:318,439,456` |
| `SessionEvent.Shell.*` | `prompt.ts:504,528` |
| `SessionEvent.Prompted` | `prompt.ts:1078` |
| `SessionEvent.AgentSwitched/ModelSwitched` | `prompt.ts:680,692` |
| `SessionEvent.Retried` | `processor.ts:1001` |

### 5.5 事件发布在监听注册之前

- `events.listen()` 注册的是**实时**监听，不会回放历史事件
- 如果某个事件在插件初始化之前就已经 publish，hook 不会收到

### 5.6 Event hook 抛异常导致中断

```ts
return Effect.sync(() => {
  for (const hook of hooks) {
    void hook["event"]?.({ event: ... })
  }
})
```

- `Effect.sync` 包裹的回调中如果某个 hook 抛**同步异常**（非 async reject），会中断 `for` 循环，后续 hook 不会被调用
- async 函数内部的 reject（Promise rejection）因为 `void` 操作符而成为 unhandled rejection，**不会**影响其他 hook

### 5.7 事件被 `experimentalEventSystem` flag 控制

来源: `compaction.ts:538`

```ts
if (flags.experimentalEventSystem) {
  if (summary)
    yield* events.publish(SessionEvent.Compaction.Ended, { ... })
}
```

- `SessionEvent.Compaction.Ended` 受 `RuntimeFlags.experimentalEventSystem` 控制
- 但 `Event.Compacted`（`session.compacted`）不受此 flag 控制，始终 publish

### 5.8 插件使用了非 `Promise<void>` 的返回值

- `Hooks.event` 的签名是 `Promise<void>`
- 如果插件返回非 void 值（如某些 Effect 类型），TypeScript 编译不通过，运行时行为未定义

---

## 六、`session.compacted` 事件的完整订阅链路

### 6.1 事件定义

来源: `packages/opencode/src/session/compaction.ts:29-36`

```ts
export const Event = {
  Compacted: EventV2.define({
    type: "session.compacted",
    schema: {
      sessionID: SessionID,
    },
  }),
}
```

### 6.2 事件发布

来源: `packages/opencode/src/session/compaction.ts:549`

```ts
yield* events.publish(Event.Compacted, { sessionID: input.sessionID })
```

发布时间点：compaction 完成后，`result === "continue"` 时。

### 6.3 事件传播路径

```
compaction.ts: events.publish(Event.Compacted, { sessionID })
  → EventV2Bridge.publish()
    → EventV2.Service.publish()
      → publishEvent()
        → 持久化到 DB (如果 sync)
        → notify()
          → 遍历所有 listeners (包括 plugin 的 listener)
          → PubSub.publish(typed[type], event)
          → PubSub.publish(all, event)
```

### 6.4 谁会订阅它

1. **Plugin event hook**: 通过 `events.listen()` → 遍历 hooks → `hook.event({ event })`
2. **EventV2Bridge**: 通过 `GlobalBus.emit("event", ...)` 转发给 TUI/前端
3. **Typed subscribers**: 通过 `events.subscribe(definition)` 精确订阅某个事件类型
4. **SSE 消费者**: 通过 `EventV2Bridge` → `GlobalBus` → SSE stream

---

## 七、总结

| 问题 | 答案 |
|------|------|
| Hook 注册方式 | 插件加载后，通过 `events.listen()` 注册全局监听，回调中遍历所有 hook 调用 `hook.event()` |
| Main/Child session 区分 | **框架不做区分**，所有 session 事件都会触发。插件需自行检查 `event.properties.sessionID` 对应的 session 的 `parentID` |
| 返回值类型 | `Promise<void>`，调用时 fire-and-forget（`void`），不 await |
| 生命周期 | 在 `Plugin` 服务初始化时（app bootstrap），所有插件加载完毕后注册 |
| 不触发原因 | 目录不匹配、插件未加载/加载失败、`event` 未定义、事件非 EventV2 渠道、发布在注册之前、hook 抛同步异常、experimental flag 控制 |

### 调试建议

如果想验证 event hook 是否被调用，可以在 hook 中添加日志：

```ts
event: async ({ event }) => {
  console.log(`[plugin] received event: ${event.type}`, event.properties)
}
```

检查条目：
1. 日志是否出现 → 确认 hook 被调用
2. `event.type` 是否为 `"session.compacted"` → 确认目标事件是否到达
3. `event.properties.sessionID` 是否正确 → 确认事件来源 session
