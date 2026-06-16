# opencode 插件 event hook 收不到 session 事件 — 上游 PR/Issue 搜索报告

> 搜索日期: 2026-06-15
> 目标仓库: `anomalyco/opencode` (即 `sst/opencode`, 175k stars)
> 目标分支: `dev` (默认分支)

---

## 一、问题回顾

**问题描述**：插件的 `event` hook 因为目录过滤 `if (event.location?.directory !== ctx.directory) return Effect.void`（位于 `packages/opencode/src/plugin/index.ts:252`）而收不到 session 事件。

**根本原因**：`EventV2Bridge.publish()` 依赖 `InstanceRef` 注入 `event.location`，但 session 模块调用 publish 时 `InstanceRef` 可能为 `undefined`，导致事件发布时没有 location，插件端过滤判定 `undefined !== ctx.directory` → 过滤掉事件。

---

## 二、上游源码现状确认

### 2.1 目录过滤 — **仍然存在**

**文件**: `packages/opencode/src/plugin/index.ts` (dev 分支, 第 251-258 行)

```ts
const unsubscribe = yield* events.listen((event) => {
  if (event.location?.directory !== ctx.directory) return Effect.void
  return Effect.sync(() => {
    for (const hook of hooks) {
      void hook["event"]?.({ event: { id: event.id, type: event.type, properties: event.data } as any })
    }
  })
})
```

**结论**: 该过滤逻辑依然在最新 dev 分支中，未被移除或修改。

### 2.2 EventV2Bridge.publish() — InstanceRef 依赖 **仍然存在**

**文件**: `packages/opencode/src/event-v2-bridge.ts` (dev 分支, 第 22-36 行)

```ts
const publish: EventV2.Interface["publish"] = (definition, data, options) =>
  Effect.gen(function* () {
    if (options?.location) return yield* events.publish(definition, data, options)
    const ctx = yield* InstanceRef
    if (!ctx) return yield* events.publish(definition, data, options)  // ← 无 location 注入!
    const workspaceID = yield* WorkspaceRef
    return yield* events.publish(definition, data, {
      ...options,
      location: new Location.Info({
        directory: AbsolutePath.make(ctx.directory),
        ...(workspaceID ? { workspaceID } : {}),
        project: { id: Project.ID.make(ctx.project.id), directory: AbsolutePath.make(ctx.worktree) },
      }),
    })
  })
```

**结论**: 当 `InstanceRef` 为 undefined 时，`publish()` 回退到不带 location 的发布，与上述目录过滤形成死锁。

---

## 三、找到的相关 Issue

### 3.1 直接相关

| # | 标题 | URL | 状态 |
|---|------|-----|------|
| #14808 | Plugin event listener for "session.created" not firing | https://github.com/anomalyco/opencode/issues/14808 | **Closed** (completed) |
| #28065 | Edit Permissions: Instance Ref bug | https://github.com/anomalyco/opencode/issues/28065 | **Open** |
| #28036 | Writing .sh files hangs opencode for ~60s (bash LSP triggers InstanceRef not provided) | https://github.com/anomalyco/opencode/issues/28036 | **Closed** (completed) |
| #27829 | bug: opencode 1.15.1 run exits with InstanceRef not provided | https://github.com/anomalyco/opencode/issues/27829 | **Closed** (completed) |
| #28037 | Plugin permission replies via SDK client are silently dropped on v1.14.51+ (Server.Default and TCP listener use different memoMaps) | https://github.com/anomalyco/opencode/issues/28037 | **Closed** (completed) |

#### 关键摘要

**#14808** — 直接命中 "plugin event 不触发" 问题。用户报告 `session.created` 事件发布但插件未收到。该 issue 被标记为 completed，关联 PR #17517。

**#28065** — "Instance Ref bug" 影响编辑权限。错误信息 `InstanceRef not provided`，自 v1.15 开始出现。仍然 **Open**（已分配 @kitlangton）。

**#28036** — `.sh` 文件写入触发 bash LSP 初始化，触发 `InstanceRef not provided` 错误导致 57s 超时。引用 #27829 为同根因问题。**已关闭**。

**#28037** — 详尽的根因分析：`Server.Default()` 和 TCP listener 使用不同 memoMap，导致插件权限回复被无声丢弃。重点指出了 Effect layer/memoMap 分离导致的服务实例隔离问题。这与 InstanceRef 问题的架构根因相同。**已关闭**（已分配 @kitlangton）。

### 3.2 会话/插件事件相关

| # | 标题 | URL | 状态 |
|---|------|-----|------|
| #5409 | [FEATURE]: SessionStart hook for session lifecycle events | https://github.com/anomalyco/opencode/issues/5409 | **Open** |
| #24958 | Feature Request: experimental.session.pre-compact Hook | https://github.com/anomalyco/opencode/issues/24958 | **Closed** (not planned) |
| #30955 | [Bug] Plugin config hook not re-invoked after /connect — agents disappear | https://github.com/anomalyco/opencode/issues/30955 | **Open** |

---

## 四、找到的相关 PR

| # | 标题 | URL | 状态 |
|---|------|-----|------|
| #17517 | fix: await plugin event hooks and handle errors in database effects | https://github.com/anomalyco/opencode/pull/17517 | **Open** (未合并) |
| #28051 | fix: preserve bus instance context | https://github.com/anomalyco/opencode/pull/28051 | **Merged** (2026-05-17) |
| #28187 | refactor(sync): publish via EffectBridge.fork for codebase consistency | https://github.com/anomalyco/opencode/pull/28187 | **Merged** (2026-05-18) |
| #31637 | docs: clarify server plugin lifecycle hooks | https://github.com/anomalyco/opencode/pull/31637 | **Open** (目标 2.0) |

#### 关键摘要

**#17517** — 由 @omd0 提交，针对 #14808。修复方案：`await` 每个插件的 event hook 调用 + try/catch 包装 + 数据库 effect 错误处理。**但此 PR 从未被合并**（2026-03-14 提交，至今 Open）。注意此 PR 只修复了"未 await 导致异步错误被吞没"的问题，**并未修改目录过滤逻辑**。

**#28051** — 由 @thdxr 提交。**核心修复**: 线程化 instance context 通过 `Bus.publish`，确保事件发布保留 `InstanceRef`。同时更新 file watcher 和 LSP 事件发布以使用活跃的 instance context。这直接缓解了 `InstanceRef not provided` 问题，但主要针对 `Bus.publish` 而非 `EventV2Bridge.publish`。

**#28187** — 由 @kitlangton 提交。将 `sync/index.ts` 的 publish 回调从 `Effect.runPromise(attachWith(...))` 迁移到 `EffectBridge.fork`，与代码库其余部分保持一致。确保 bridge 在 handler fiber 中捕获正确的 `InstanceRef`/`WorkspaceRef`。这进一步修复了 publish 时 InstanceRef 丢失的问题。

**#31637** — 由 @Desperado 提交的文档 PR。明确指出 `session.start` / `session.end` 钩子不存在，引导用户使用 `event` (如 `session.deleted`) 和 `tool.execute.before` / `after`。

---

## 五、搜索未命中项

以下搜索关键词在 `anomalyco/opencode` 仓库中 **未找到相关 Issue/PR**：

- `"event.location"` + `"ctx.directory"` — 0 结果
- `"event.location"` 单独搜索 — 0 结果
- `"EventV2Bridge"` 在 Issues 中 — 仅 1 个结果 (#29068, 不相关)
- `session.compacted` + plugin — 有结果但不直接相关

`opencode-ai/opencode` 仓库（独立组织）搜索 `event location directory plugin` — 0 结果。

---

## 六、综合结论

### 6.1 该问题是否已知并被修复？

**部分修复，但根因未完全消除。**

1. PR #28051 (merged) 和 #28187 (merged) 修复了 `InstanceRef` 在 event bus publish 链中丢失的问题，这些修复直接缓解了 session 事件 publish 时 InstanceRef 为 undefined 的场景。

2. 但是，**目录过滤逻辑 `if (event.location?.directory !== ctx.directory)` 仍然存在于最新 dev 分支**，没有任何 PR 移除或修改它。

3. PR #17517 直接针对 #14808（插件 event 不触发），但 **从未被合并**，且它也只是修复了 await 丢失问题，未改变过滤逻辑。

### 6.2 当前风险

即使 InstanceRef 在 publish 链中正确传递（通过 #28051 + #28187），只要 `EventV2Bridge.publish()` 中 `InstanceRef` 获取失败（例如 session compaction 等后台进程不在 handler fiber 上下文中调用），事件仍会丢失 location，导致目录过滤拦截。

### 6.3 潜在修复方向

- 在 `EventV2Bridge.publish()` 中，当 `InstanceRef` 不可用时，回退到 `ctx.directory`（需要将该值传入 bridge 层）
- 在 `plugin/index.ts` 的 listener 中，当 `event.location?.directory` 为 undefined 时，不过滤事件（改为允许所有无 location 事件通过，或至少记录 warning）
- 统一 session 模块的 publish 调用点，确保始终在拥有 InstanceRef 的 Effect fiber 中调用

---

## 七、信息来源

| 来源 | URL |
|------|-----|
| 上游仓库 (dev 分支) | https://github.com/anomalyco/opencode |
| plugin/index.ts (最新) | https://github.com/anomalyco/opencode/blob/dev/packages/opencode/src/plugin/index.ts |
| event-v2-bridge.ts (本地副本) | `source-code/opencode/packages/opencode/src/event-v2-bridge.ts` |
| 本地 plugin/index.ts 副本 | `source-code/opencode/packages/opencode/src/plugin/index.ts` |

所有 GitHub Issue/PR 内容通过 `webfetch` 从 GitHub 页面直接获取，为官方原文。搜索日期: 2026-06-15。
