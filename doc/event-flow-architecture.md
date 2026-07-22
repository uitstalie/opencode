# TUI 事件流架构分析

> **注意**：§1–§4 是 2026-07 重构**之前**的快照，仅作历史参考。
> 当前架构见 §8（session 统一模型 + 双总线）与 §9（新架构复审）。

## 1. 事件生产者 (Producers)

### 1.1 Prompt Worker (`worker.rs`)

每个 prompt 一个 `std::thread`，运行自己的 tokio runtime，执行完整的 agent 循环（LLM 流 → 工具执行 → 重复，最多 `max_steps` 步）。通过无界 `std::sync::mpsc` channel 产生 `PromptEvent`：

| 事件 | 触发条件 |
|------|----------|
| `AssistantDelta(String)` | `StreamChunk::TextDelta` |
| `ThinkingDelta(String)` | `StreamChunk::ReasoningDelta` |
| `ToolCallStart { id, name }` | `StreamChunk::ToolCallStart` |
| `ToolRunning { id, args }` | `StreamChunk::ToolCallEnd`，在 `run_tool()` 前发送 |
| `ToolBatch { assistant, tool_calls, results }` | 一个步骤的所有工具执行完毕 |
| `Finish { prompt_tokens, cache_hit_tokens }` | 回合结束 |
| `Error(String)` | 致命错误 |
| `Aborted` | `abort` AtomicBool 被设置 |
| `RetryStatus(String)` | 重试倒计时 |

### 1.2 工具 → UI 请求（请求/响应模式，非事件）

工具通过 `ToolContext` 中的 channels 与 UI 通信：

- **`AskRequest`** (`question` 工具）：发送问题请求，**同步阻塞**等待 UI 回复 `Vec<String>`
- **`PermissionRequest`** (`execute_checked`)：权限请求，**同步阻塞**等待 UI 回复 `bool`

非交互模式下这两个 channel 为 `None`，工具会直接报错而非挂起。

### 1.3 后台线程

- **Sub-agent 进度** (`tool/task.rs`)：嵌套 agent 循环发送状态字符串到 `progress_tx`
- **Fire-and-forget 线程** (`session_ops.rs`)：标题生成、摘要、压缩、memory 提取、`/dream`。这些**不发送 UI 事件**，结果只写入 `SessionStore`
- **Sidebar 文件监视器** (`sidebar.rs`)：`notify` watcher 线程发送 `PathBuf`
- **models.dev 刷新**：缓存刷新线程，无 UI 事件

### 1.4 UI → Worker

- **`followup_tx: Sender<String>`**：回合中的用户消息，worker 在步骤边界时取出注入

---

## 2. 事件消费者 (Consumers)

### 2.1 主交互循环 (`mod.rs`)

每次迭代按顺序执行：

1. `pump_prompt_job()` — 非阻塞取出 worker 事件
2. `poll_ask_request()` — 非阻塞检查 question 请求
3. `poll_permission_request()` — 非阻塞检查权限请求
4. `sidebar.poll_refresh()` — 文件树刷新
5. `maybe_start_next_prompt()` — 取出队列中的下一个 prompt
6. Toast 过期检查
7. `event::poll(33ms)` — crossterm 输入事件

### 2.2 Headless 模式 (`prompt_flow.rs`)

`pump_prompt_job_for_stdout` 是 `pump_prompt_job` 的**近乎完全重复的副本**，只是渲染目标不同（stdout 纯文本 vs ratatui）。

### 2.3 对话框 (`dialogs.rs`, `pending.rs`)

对话框是**纯本地 UI 状态**，不接收 worker 事件。键盘路由优先级：

```
pending_permission > pending_question > pending_text_input > dialog > main input
```

---

## 3. Channel 清单

| Channel | 类型 | 方向 | 用途 |
|---------|------|------|------|
| `tx`/`rx` | `mpsc::<PromptEvent>` (unbounded) | worker → UI | 主要事件流 |
| `ask_tx`/`ask_rx` | `mpsc::<AskRequest>` | worker → UI | Question 请求 |
| `responder` (AskRequest) | `mpsc::Sender<Vec<String>>` | UI → worker | Question 回复 |
| `permission_tx`/`permission_rx` | `mpsc::<PermissionRequest>` | worker → UI | 权限请求 |
| `responder` (PermissionRequest) | `mpsc::Sender<bool>` | UI → worker | 权限回复 |
| `followup_tx`/`followup_rx` | `mpsc::<String>` | UI → worker | 回合中插入消息 |
| `progress_tx`/`progress_rx` | `mpsc::<String>` | worker → UI | Sub-agent 进度 |
| sidebar `tx`/`rx` | `mpsc::<PathBuf>` | bg → UI | 文件监视 |

共享标志：`shutdown` (Arc<AtomicBool>), `abort` (Arc<AtomicBool>)

---

## 4. 事件流图

```
┌────────────────────────────── SessionView (main thread) ──────────────────────────────┐
│                                                                                        │
  crossterm keys ─────► route_key ─► [permission?] [question?] [text_input?] [dialog?]   │
  paste / mouse ──────►                                                                │
│                                                                                        │
│   Enter on input ─► handle_slash_command / enqueue_or_run_prompt ─┐                   │
│        Esc ─► abort.store(true)                                  │                   │
│                                                                  ▼                   │
│  ┌─────────────┐   spawn_prompt_worker (std::thread + tokio RT)  │                   │
│  │ prompt_job  │◄─────────────────────────────────────────────────┘                   │
│  │             │                                                                       │
│  │  run_inner loop (per iteration):                                                    │
│  │   1. pump_prompt_job()  ◄── try_recv ── PromptEvent ──┐                            │
│  │   2. poll_ask_request() ◄── try_recv ── AskRequest ──┤                            │
│  │   3. poll_permission()  ◄── try_recv ── PermRequest ─┤                            │
│  │   4. sidebar.poll_refresh()◄─ try_recv ─ PathBuf ────┤                            │
│  │   5. maybe_start_next_prompt() (pending_prompts)     │                            │
│  │   6. toast expiry                                   │                            │
│  │   7. event::poll(33ms) → render if dirty/ai_running │                            │
│  └─────────────┘                                       │                            │
└────────────────────┬───────────────────────────────────┼────────────────────────────┘
                     │ followup_tx (String)             │
                     ▼                                  │
┌───────────────── Worker thread (per turn) ───────────┴──────────────────────────────┐
│  loop (steps ≤ max_steps):                                                          │
│    check shutdown/abort ─► auto-compact if near context window                      │
│    llm.chat(history.clone(), ...) with retry/backoff ─► RetryStatus events          │
│    stream loop:                                                                     │
│      TextDelta ─► AssistantDelta          ReasoningDelta ─► ThinkingDelta           │
│      ToolCallStart ─► ToolCallStart       ToolCallEnd ─► ToolRunning ─► run_tool()  │
│        ├─ permission gate: Decision::Ask ─► PermissionRequest ─► [BLOCK] ◄── UI key │
│        ├─ question tool: ─► AskRequest ─► [BLOCK] ◄────────────────────── UI dialog │
│        └─ task tool: sub-agent run_agent ─► progress_tx strings                     │
│    after step: ToolBatch event; drain followup_rx; push results into history        │
│    finish: Finish / Error / Aborted                                                 │
└─────────────────────────────────────────────────────────────────────────────────────┘

Fire-and-forget (no UI events, write to SessionStore only):
title / summary / manual compact / generate_memory (5s delay) / dream / models.dev refresh
```

---

## 5. 已识别问题

### Bug / 正确性

1. ~~**`SessionRuntimeGuard` 对 panic 无效**~~ ✅ 已修复（guard 在 `run_inner` 之前创建）

2. **在 async executor 上阻塞 `recv()`** (`tool/mod.rs:335`, `tool/question.rs:80`)
   - 权限/问题等待是同步 `mpsc::Receiver::recv()` 在 async 代码中
   - 违反项目异步准则（"sync work belongs in `spawn_blocking`"）
   - **ESC-abort 无法中断待处理的权限/问题**

3. **`Aborted`/`Error` 后模态对话框未清理** (`mod.rs:687-715`)
   - abort/error 时未清除 `ui.pending_question` / `ui.pending_permission`
   - 对话框保持打开，回复发送到已关闭的 responder（静默忽略）

4. **Auto-compaction 后历史分歧**
   - Worker 压缩自己的 `history`，但 TUI 的 `self.messages` 未压缩
   - 下次 prompt 克隆未压缩的 `self.messages` 到新 worker
   - **auto-compaction 只在单个回合内有效**

### 效率问题

5. **重复的事件消费逻辑** (`mod.rs:597-731` vs `prompt_flow.rs:131-255`)
   - ~130 行近乎相同的 `match` on `PromptEvent`
   - Headless 版本缺少 `capture_diff`、task-count 更新、abort 时的 assistant 文本保存

6. ~~**Headless 模式忙轮询**~~ ✅ 已修复（10ms sleep）

7. **每次 LLM 调用都克隆完整历史** (`worker.rs:296`)
   - `llm.chat(history.clone(), tool_defs.clone(), ...)`
   - 长会话反复 O(history) 复制

8. **每个 delta 一个事件，无合并**
   - 每个 `TextDelta` 都是单独的 `PromptEvent::AssistantDelta(String)`
   - 无背压机制（所有 channel 无界）

9. **Sidebar 在任何 dirty 事件时全量重扫描** (`sidebar.rs:78-92`)
   - 扫描在 UI 线程执行

10. **Fire-and-forget 线程对 UI 不可见**
    - 标题/摘要/memory/dream 结果只在 tracing 或下次读取 store 时可见
    - 失败对用户静默

---

## 6. 改进建议

### 高优先级

1. **统一两个 pump 函数**
   - 提取 `handle_prompt_event(&mut self, event) -> PumpOutcome`
   - 只有渲染目标不同（ratatui vs stdout）

2. **修复 terminal guard**
   - 在 `run_inner` 之前创建 `SessionRuntimeGuard`

3. **将阻塞等待移出 executor**
   - `execute_checked` 和 question 工具使用 `tokio::task::spawn_blocking`
   - 或使用 `oneshot` channel + abort 检查

4. **历史单一数据源**
   - Worker 独占 `history`
   - `ToolBatch`/`Finish` 携带已提交的消息或发送 `Compacted` 事件

5. **清理终止事件的模态状态**
   - `Aborted` 和 `Error` 时清除 `ui.pending_question` / `ui.pending_permission`

### 中优先级

6. **Headless 循环添加阻塞**
   - 使用 `recv_timeout(Duration::from_millis(50))` 替代忙轮询

7. **合并 delta 事件**
   - Worker 中批量累积文本，定期发送
   - 或使用有界 channel 添加背压

8. **避免每步历史克隆**
   - `llm.chat` 接受 `&[Message]` 或 `Arc<[Message]>`

9. **暴露后台线程结果**
   - 添加完成 channel 用于 title/summary/memory/dream
   - 失败时发送 toast 通知

### 长期考虑

10. **统一 `UiEvent` 枚举**
    ```rust
    enum UiEvent {
        Prompt(PromptEvent),
        Ask(AskRequest),
        Permission(PermissionRequest),
        Progress(String),
        Sidebar,
        BgTask(BgTaskResult),
    }
    ```
    - 单一 channel 替代多个 `try_recv` 轮询
    - 事件顺序确定
    - 简化未来生产者添加

---

## 7. 剩余待处理问题

| 问题 | 类型 | 说明 |
|------|------|------|
| PERF-001 | 异步架构 | verify_provider 在 UI 线程执行网络请求 |
| DESIGN-002 | 功能设计 | 子 agent 无 undo store |
| DESIGN-003 | 功能设计 | rm.rs 目录删除无撤销快照 |

**已跳过（待重构时处理）**:
- TEST-003/004/006: TUI worker/dialogs/cli 测试
- DESIGN-001: SessionView god struct（需要大型重构）

---

## 8. 目标架构：Session 统一模型 + 双总线

### 8.1 核心思想

**一切皆 session，事件以 session 锚定，UI 按 session 路由，持久化只是 session 的副作用。**

Main agent、sub-agent、后台任务（title/summary/memory/dream/compact）本质上是同一种东西：一个 agent 实例在跑自己的上下文。区别仅在于：

| 维度 | Main | SubAgent | Background |
|------|------|----------|------------|
| 是否阻塞父级 | — | 是（task 工具同步等待） | 否（fire-and-forget） |
| 交互能力（Ask/Permission） | 有 | 有（可配置） | 无（非交互） |
| 事件路由 | 对话视图 | 对应 tool call 进度行 | toast / 静默写 store |
| 上下文 | 持久会话 | 临时会话，完成即过期 | 临时会话，完成即过期 |

### 8.2 身份模型

每个 agent 实例拥有唯一 session，`SessionId` 即事件路由的 tag：

```rust
enum AgentKind { Main, SubAgent, Background }

// Session 记录扩展（core/session.rs）
struct Session {
    id: String,                    // main: "session-*", 临时: "sub-*" / "bg-*"
    kind: AgentKind,               // serde default = Main（向后兼容）
    parent_id: Option<String>,     // agent 树，支持嵌套 task
    ...
}

// 运行时注册表（内存）
struct SessionEntry {
    id: SessionId,
    kind: AgentKind,
    parent: Option<SessionId>,
    abort: Arc<AtomicBool>,        // 按 session 粒度的取消
    created_at: Instant,           // TTL / 过期销毁依据
}
```

### 8.3 双总线

**总线 1：Session 事件总线（core → 前端）**

替代现在散落的 6 条 channel（prompt/ask/permission/progress/followup/sidebar），统一为一条：

```rust
struct SessionEvent {
    session: SessionId,
    kind: AgentKind,               // 冗余 tag，免查注册表
    payload: EventPayload,
}

enum EventPayload {
    Prompt(PromptEvent),           // delta / tool lifecycle / finish
    Ask(AskRequest),               // 仍带 oneshot responder（请求/响应，非纯事件）
    Permission(PermissionRequest), // 同上
    Progress(String),              // sub-agent 进度
    Done { result: Result<String, String> },  // 后台任务完成/失败
}
```

- 初版用单条 `mpsc<SessionEvent>`（单订阅者：TUI 或 headless）
- 未来多订阅者（telemetry/日志/测试）再升级 `tokio::sync::broadcast`
- 引入有界 channel + worker 侧 delta 合并，顺带解决背压问题
- `ToolContext` 不再持有裸 `ask_tx`/`permission_tx`/`progress_tx`，改为持有统一 `SessionEventSender`（解决 core 依赖 UI channel 的分层问题）

**总线 2：UI 总线（TUI 内部）**（✅ 已实现，`tui/ui_bus.rs`）

```rust
enum UiEvent {
    Input(crossterm::event::Event),  // 输入线程转发
    Session(Box<SessionEvent>),      // session 总线转发线程
    Tick,                            // 动画/家政 tick（运行中 33ms，空闲 500ms）
}
```

- 主循环从"`event::poll(33ms)` + 多个 `try_recv` 轮询"改为**阻塞 `recv()` + 批量 drain + 每批最多渲染一次**
- Tick 驱动：tool spinner 动画、toast 过期、sidebar 文件监视刷新
- 空闲时 tick 降频到 500ms（仅家政），CPU 占用接近零
- Headless 模式不受影响（直接 drain session 总线）

### 8.4 路由策略（tag + 集中 demux）

```rust
match (event.kind, event.payload) {
    (Main, Prompt(e))       => 对话视图渲染,
    (SubAgent, Progress(s)) => 更新对应 tool call 进度行,
    (_, Ask/Permission(r))  => 弹对话框（Background 不产生此类事件）,
    (Background, Done(..))  => toast 通知 / 静默,
}
```

选择集中 match 而非动态注册 handler：订阅者只有 TUI 和 headless 两个且行为固定，`Box<dyn Fn>` 注册表引入不必要的生命周期复杂度。等真有插件需求再升级。

### 8.5 顺序与取消语义

- **顺序**：不同 session 的事件允许任意交错（tag 区分）；同一 session 内天然有序（单线程生产者）。不需要全局顺序。
- **取消**：Esc abort main session 并沿 agent 树级联到其 sub-agent；Background session 独立存活（标题生成不该被 Esc 杀）。
- **交互权限**：仅 Main 和 SubAgent 可发 Ask/Permission；Background 一律非交互（等价于现在 channel 为 None 的行为）。

### 8.6 临时 session 持久化

**前期保留持久化**（便于还原/调试 sub-agent 和后台任务的问题），后期可通过配置关闭。

- 临时 session 与 main 共用 `SessionStore`，`kind` 字段区分；`list_sessions()` 默认只返回 Main，临时会话通过 debug 子命令查看
- sub-agent：每步提交 assistant/tool 消息到自己的 session
- 后台任务：prompt + 最终结果写入自己的 session
- 启动时 TTL 清理：删除超过 N 天（默认 7）的临时 session
- 配置开关：`persist_agent_sessions: bool`（默认 true），后期稳定后可关

### 8.7 顺带修复的旧问题

1. Auto-compaction 历史分歧（§5.4）：历史归属 session，单一数据源
2. 后台任务静默失败（§5.10）：`Done` 事件天然可见
3. 两个 pump 重复（§5.5）：共享 `handle_session_event()`
4. core → UI 分层泄漏：`ToolContext` 只依赖 `SessionEventSender`

### 8.8 迁移路径（已完成 ✅）

1. ✅ `core/session.rs`：Session 增加 `kind`/`parent_id`；`list_sessions` 过滤临时会话（`list_agent_sessions` 用于调试）
2. ✅ `core/event.rs`（新）：`SessionEvent`/`EventPayload`/`SessionEventSender`；`AskRequest`/`PermissionRequest`/`PromptEvent` 上移到 core
3. ✅ `run_agent`（tool/task.rs）：临时 session 创建（`create_agent_session`）+ 历史持久化（`persist` 参数）+ progress 走总线
4. ✅ `worker.rs`：PromptEvent 经 `SessionEventSender` 发送；`ToolContext` 三条 channel 合并为 `events`
5. ✅ TUI/headless：统一 `drain_session_events` + demux（Ask/Permission/Progress/Done 共享 handler）；headless 忙轮询修复（10ms sleep）
6. ✅ 后台任务（session_ops.rs）：`bg_session_anchor()` 创建 bg session、持久化、发 `Done` 事件 → TUI toast
7. ✅ 启动清理（`cleanup_agent_sessions`，7 天 TTL）+ `persist_agent_sessions` 配置开关（默认 true）

**仍未统一**：TUI 与 headless 对 `Prompt` 事件的渲染分支仍各自实现（pending_tool_calls / capture_diff / collapsed 展示差异属正常分歧），后续可提取共享的 `handle_prompt_event(state, event)` 进一步收敛。

---

## 9. 新架构复审（2026-07）

事件总线 + UI 总线 + vsync 帧合成落地后，对照 §5 的旧问题逐条复查，并排查新引入的问题。

### 9.1 旧问题现状

| # | 问题 | 状态 | 说明 |
|---|------|------|------|
| 1 | terminal guard 对 panic 无效 | ✅ 已修复 | guard 在 `run_inner` 之前创建 |
| 2 | async executor 上阻塞 `recv()`（权限/问题） | ✅ 已修复 | `wait_response()`：recv_timeout(100ms) 轮询 + abort/shutdown 信号量检查；background agent 权限一律 deny、question 工具从 tool_defs 剔除；sub-agent 请求经 bus 路由到主界面并标注来源 |
| 3 | Aborted/Error 后模态对话框未清理 | ✅ 已修复 | `dismiss_pending_modals()` 在 Aborted/Error 分支清理并回绝 responder；Esc 在 AI 运行中专职"取消并停止回合"（含权限/问题对话框），"返回"导航改用 ← 键 |
| 4 | auto-compaction 历史分歧 | ✅ 已修复 | worker 压缩后发 `Compacted` 事件；TUI 从 store 重建历史（`compaction_window`：最近 checkpoint 前 3 条全量消息 → 对话结束），三方数据源对齐 |
| 5 | 两个 pump 重复 | 🟡 部分修复 | 总线 demux（Ask/Permission/Progress/Done）已共享；`Prompt` 渲染分支仍 TUI/headless 各一份 |
| 6 | headless 忙轮询 | ✅ 已修复 | 10ms sleep |
| 7 | 每次 LLM 调用克隆完整历史 | ✅ 已修复 | `LlmProvider::chat` 改为借用 `&[Message]`/`&[ToolDef]`，调用点零拷贝 |
| 8 | delta 无合并、无背压 | 🟡 缓解 | 渲染侧已被帧合并封顶 60fps；但 worker 仍每 delta 一个事件，channel 仍无界 |
| 9 | sidebar dirty 时 UI 线程全量重扫描 | ❌ 仍存在 | 现在挂在 Tick 上执行，仍是 UI 线程同步扫描 |
| 10 | 后台线程对 UI 不可见 | ✅ 已修复 | `Done` 事件 + toast |

### 9.2 新引入的问题

| # | 问题 | 严重度 | 说明 |
|---|------|--------|------|
| N1 | `run_agent` 在 async executor 上同步写 sled | 低 | 持久化 `persist_msg` 是阻塞 IO，嵌在流式循环里；sled 够快，但严格说应 `spawn_blocking` 或批量提交 |
| N2 | headless 回合结束后 bus 无人 drain | 低 | 最后一个 prompt 结束后 bg 线程（summary/memory）仍可能发事件，channel 无界堆积直到进程退出（短命进程，影响小） |
| N3 | 权限/问题请求与 abort 的交互未改善 | ✅ 已修复 | UI 侧 Esc 拒绝+停止（#3）；worker 侧 `wait_response` 每 100ms 检查 abort/shutdown 信号量，双向可中断 |
| N4 | `Prompt` 事件隐式假设来自 Main session | 低 | demux 未按 `event.kind` 区分渲染目标；目前 run_agent 不产生 Prompt 事件所以安全，但架构上是个未声明的约定 |
| N5 | tick 线程/session 转发线程不随 shutdown 退出 | 低 | 依赖进程退出回收；交互模式下无碍，库化复用时需注意 |

### 9.3 建议处理顺序

1. **#3（模态清理）**：Aborted/Error 分支清除 pending 对话框 —— 小改动，正确性问题
2. **#2/#3-N3（abort 可中断的权限等待）**：`recv_timeout` 轮询 + abort 检查，或 `spawn_blocking`
3. **#4（历史单一数据源）**：worker 回合结束时把提交的 history 通过事件回传（`Finish` 携带或新增 `HistoryCommitted` 事件），TUI 替换 `self.messages`
4. **#8（worker 侧 delta 攒批）**：按 ~8ms 窗口合并 TextDelta，进一步降低事件量
5. **N1（持久化批量提交）**：`persist_msg` 缓冲到步骤边界批量写
