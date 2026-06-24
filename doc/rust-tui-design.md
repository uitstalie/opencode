# Rust TUI 重写设计文档

> 版本: draft-1 | 日期: 2026-06-24 | 分支: `opencode-rust-tui`

## 1. 动机

### 1.1 当前问题

| 维度 | 现状 | 问题 |
|------|------|------|
| 运行时 | TypeScript (Bun) + SolidJS + Effect | 冷启动慢，内存占用高 |
| 渲染引擎 | @opentui/solid v0.3.4（闭源） | 不可控，升级依赖外部发布节奏 |
| 帧率 | 目标 60fps，实际取决于 SSE 事件吞吐 | 大量 SSE 事件时出现掉帧 |
| 语法高亮 | 30 个 tree-sitter WASM parser | 启动加载慢，每次解析需跨 WASM 边界 |
| 类型系统 | TypeScript 类型擦除 | 运行时缺乏类型保障 |

### 1.2 目标

- **启动时间**：从 2-3s 降到 <500ms
- **内存占用**：从 150MB+ 降到 <50MB
- **帧率**：稳定 60fps，不随事件量退降
- **可控性**：全栈自维护，无闭源依赖
- **类型安全**：编译期保证，无运行时类型错误

---

## 2. 当前架构（参考）

```
┌─────────────────────────────────────┐
│  packages/tui/  (~27K lines TSX)    │
│                                     │
│  providers (context/)               │
│    SDKProvider → SyncProvider       │
│      → DataProvider → ThemeProvider │
│                                     │
│  routes/                            │
│    session/index.tsx (2648 行)       │
│    home/                            │
│                                     │
│  feature-plugins/                   │
│    diff-viewer.tsx (1059 行)         │
│    sidebar.tsx        notifications │
│                                     │
│  component/ (7581 行)               │
│    prompt/  dialogs/  spinners      │
│                                     │
│  Key Dependencies:                  │
│    @opentui/solid (闭源渲染层)       │
│    SolidJS 1.9 (响应式)              │
│    Effect 4.0 (异步运行时)            │
│    tree-sitter WASM x30             │
└──────────────┬──────────────────────┘
               │ REST + SSE (JSON)
┌──────────────▼──────────────────────┐
│  packages/opencode (TypeScript)     │
│  HTTP API + SSE event stream        │
│  Session management, LLM, tools     │
└─────────────────────────────────────┘
```

---

## 3. 目标架构

```
┌─────────────────────────────────────┐
│  crates/opencode-tui/  (Rust)       │
│                                     │
│  app/         应用入口、启动流程       │
│  state/       全局状态管理            │
│  proto/       gRPC 协议定义           │
│  client/      gRPC 客户端 + 事件流    │
│                                     │
│  view/                              │
│    session.rs   会话视图 (~2000 行)   │
│    home.rs      首页                 │
│    sidebar.rs   侧边栏               │
│    diff.rs      Diff 查看器           │
│                                     │
│  widget/                            │
│    prompt.rs    输入框               │
│    dialog.rs    弹窗                 │
│    markdown.rs  Markdown 渲染        │
│    spinner.rs   加载指示器            │
│    toast.rs     通知                 │
│                                     │
│  highlight/     tree-sitter 高亮      │
│  keymap/        快捷键系统            │
│  theme/         主题                 │
│  config/        配置解析              │
│                                     │
│  Dependencies:                      │
│    ratatui 0.28 (TUI 渲染)           │
│    crossterm 0.27 (终端后端)          │
│    tonic 0.12 (gRPC)                │
│    prost 0.13 (protobuf)            │
│    tokio 1.x (异步运行时)             │
│    tree-sitter 0.23 (原生语法高亮)    │
│    serde 1.x (序列化)                │
└──────────────┬──────────────────────┘
               │ gRPC (protobuf, bidirectional stream)
┌──────────────▼──────────────────────┐
│  packages/opencode  (扩展)          │
│  + gRPC server (tonic via napi-rs   │
│    或独立 sidecar 进程)              │
│  现有 HTTP API 保留作为 fallback     │
└─────────────────────────────────────┘
```

---

## 4. 技术选型

### 4.1 终端框架

| 依赖 | 版本 | 用途 |
|------|------|------|
| `ratatui` | 0.28 | 声明式 TUI 组件库，提供 Layout/Block/Paragraph/List/Table |
| `crossterm` | 0.27 | 跨平台终端后端，键盘/鼠标事件，kitty 协议 |
| `tui-textarea` | 0.6 | 多行文本输入（prompt 组件基础） |
| `tui-markdown` | — | Markdown 渲染（需自研或找社区方案） |

### 4.2 通信协议（gRPC）

```
为什么要 gRPC 而不是继续用 REST + SSE？

1. 类型安全：protobuf 定义强类型 API，Rust 端 tonic-build 生成客户端
2. 双向流：ServerStreaming RPC 自然替代 SSE，无需手动管理 EventSource
3. 性能：HTTP/2 多路复用 + protobuf 二进制编码，传输体积比 JSON 小 3-5x
4. 生态：tonic + prost 是 Rust gRPC 的事实标准
```

**gRPC Service 定义（初版）**：

```protobuf
service OpenCode {
  // 会话管理
  rpc ListSessions(SessionFilter) returns (SessionList);
  rpc GetSession(SessionID) returns (Session);
  rpc CreateSession(CreateSessionRequest) returns (Session);
  rpc DeleteSession(SessionID) returns (Empty);

  // 消息
  rpc SendMessage(SendMessageRequest) returns (stream MessageEvent);
  rpc CancelMessage(CancelRequest) returns (Empty);

  // 事件流（替代 SSE）
  rpc SubscribeEvents(EventFilter) returns (stream Event);

  // 配置
  rpc GetConfig(Empty) returns (Config);

  // 权限
  rpc ResolvePermission(PermissionRequest) returns (PermissionResponse);
}
```

### 4.3 状态管理

ratatui 没有 SolidJS 式的细粒度响应式。替代方案：

```
┌──────────────────────────────────────────┐
│  AppState (单一全局状态树)                  │
│                                          │
│  struct AppState {                       │
│    sessions: Vec<Session>,               │
│    active_session: Option<SessionView>,  │
│    config: Config,                       │
│    status: ConnectionStatus,             │
│    theme: Theme,                         │
│    keymap_mode: KeymapMode,              │
│    todos: Vec<Todo>,                     │
│    dialogs: Vec<Dialog>,                 │
│  }                                       │
│                                          │
│  更新模型:                                │
│    gRPC event → handle_event() →         │
│    state.lock().update() →              │
│    redraw flag set → next frame render    │
│                                          │
│  渲染模型（ratatui immediate mode）:       │
│   每帧根据当前 state 完整绘制，             │
│   无需 diff/patch，ratatui 自己处理增量    │
└──────────────────────────────────────────┘
```

### 4.4 异步运行时

- **tokio**：Rust 生态的 de facto 标准
- gRPC 客户端 (tonic) 天然 tokio 兼容
- crossterm 事件循环集成 tokio

---

## 5. 组件映射

| 当前 (TypeScript) | 目标 (Rust) | 行数估计 | 说明 |
|---|---|---|---|
| `context/sync.tsx` (655行) | `state/sync.rs` | ~400 | gRPC 事件流处理 |
| `context/data.tsx` (581行) | `state/data.rs` | ~300 | V2 事件层（可选） |
| `routes/session/index.tsx` (2648行) | `view/session.rs` | ~1500 | 消息列表、权限弹窗、思考切换 |
| `feature-plugins/diff-viewer.tsx` (1059行) | `view/diff.rs` | ~800 | 分屏/统一 diff、文件树 |
| `feature-plugins/sidebar.tsx` | `view/sidebar.rs` | ~400 | 文件树侧边栏 |
| `component/prompt/` | `widget/prompt.rs` | ~600 | 多行输入、自动补全、历史 |
| `component/dialog/` | `widget/dialog.rs` | ~300 | 弹窗系统 |
| `feature-plugins/which-key.tsx` | `keymap/which_key.rs` | ~200 | 快捷键帮助 |
| `keymap.tsx` | `keymap/mod.rs` | ~400 | 模式系统、键绑定 |
| `context/kv.tsx` | `config/kv.rs` | ~150 | 持久化用户偏好 |
| `theme/` (1089行) | `theme/mod.rs` | ~400 | 主题系统 |
| `ui/` (2029行) | `widget/` | ~1200 | 通用 UI 组件 |
| `prompt/display.ts` | `widget/markdown.rs` | ~300 | Markdown 渲染 |
| tree-sitter WASM x30 | tree-sitter native | — | 原生绑定 |
| 其他配置/插件/~5000行 | 暂不迁移 | — | 插件系统、编辑器集成等 |
| **总计 ~27K** | **~7K Rust** | | 核心功能 |

---

## 6. 实施阶段

### Phase 0: 环境搭建（1-2 天）

- [ ] `cargo init` 在 `crates/opencode-tui/`
- [ ] Cargo.toml 加依赖：ratatui, crossterm, tokio, tonic, prost
- [ ] 基础 main.rs：初始化终端 → 事件循环 → 渲染循环
- [ ] ratatui hello world 验证
- [ ] 集成到 monorepo 的构建流程（Makefile / justfile）

### Phase 1: 通信层（3-5 天）

- [ ] 编写 `.proto` 文件，定义核心 API
- [ ] tonic-build 生成 Rust 客户端
- [ ] 后端添加 gRPC server（napi-rs 嵌入或独立 sidecar）
- [ ] Rust 端实现 gRPC client + tokio 事件流
- [ ] 验证：Rust TUI 能获取 session 列表

### Phase 2: 状态管理 + 基础 UI（5-7 天）

- [ ] `AppState` struct + `Arc<RwLock<AppState>>`
- [ ] 事件循环：gRPC stream → parse events → update state
- [ ] 渲染循环：state → ratatui widgets
- [ ] 基础布局：header + body + footer
- [ ] Session 列表视图
- [ ] 键盘输入处理（keymap 基础）

### Phase 3: 核心视图（7-10 天）

- [ ] Session 详情视图（消息列表 + 文本渲染）
- [ ] Prompt 输入框（多行、历史、自动补全）
- [ ] 权限弹窗
- [ ] Diff 查看器
- [ ] 侧边栏
- [ ] Markdown 渲染

### Phase 4: 语法高亮 + 主题（3-5 天）

- [ ] tree-sitter native 集成
- [ ] 增量解析
- [ ] 主题系统（color palette + ANSI mapping）

### Phase 5: 打磨 + 切换（3-5 天）

- [ ] 性能调优（渲染批处理、事件合并）
- [ ] 快捷键系统完善
- [ ] 与现有 HTTP API 的兼容性回退
- [ ] 切换脚本（`opencode --tui=rust`）

---

## 7. 后端变更

### 7.1 gRPC Server 方案

**推荐方案：独立 sidecar 进程**

```
┌──────────┐   HTTP   ┌──────────────┐
│  TUI     │─────────▶│  opencode    │
│  (Rust)  │          │  (TypeScript)│
│          │  gRPC    │              │
│          │◀────────▶│  + gRPC      │
└──────────┘          │    sidecar   │
                      └──────────────┘
```

gRPC sidecar 作为 opencode 启动时 fork 的子进程，共享 session store 和配置。通过 Unix domain socket 或 localhost 通信。

**备选方案：napi-rs 嵌入**
将 tonic gRPC server 编译为 Node.js native addon，直接嵌入 opencode 进程。但构建复杂度高，不推荐首版。

### 7.2 后端改动清单

- [ ] `packages/opencode/` 添加 proto 编译脚本
- [ ] `packages/opencode/src/grpc/` gRPC server 实现
- [ ] `session.store` 暴露 gRPC 接口
- [ ] `config.store` 暴露 gRPC 接口
- [ ] 事件系统桥接：internal event → gRPC stream
- [ ] HTTP API 保留不变（兼容旧 TUI）

---

## 8. 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| gRPC sidecar 与主进程通信延迟 | 中 | 中 | Unix socket + 共享内存 fallback |
| ratatui 组件不如 SolidJS 灵活 | 高 | 中 | 自定义 widget trait，参考 SolidJS 响应式思路 |
| 全量重写工期超预期 | 高 | 高 | 分阶段交付，每阶段有可用产物 |
| tree-sitter 原生语法高亮性能不达预期 | 低 | 低 | 保留 WASM fallback |
| 闭源 @opentui 的某些特性无法复现 | 中 | 中 | 聚焦核心功能，异步特性差异化处理 |
| gRPC 协议变更导致前后端不兼容 | 低 | 高 | proto 版本化，保留 REST fallback |

---

## 9. 文件结构（目标）

```
crates/
  opencode-tui/
    Cargo.toml
    src/
      main.rs            # 入口：终端初始化、事件/渲染循环
      app.rs             # App 结构体，生命周期
      state.rs           # AppState + update/reduce
      event.rs           # 事件类型定义
      proto/
        mod.rs           # protobuf generated code
        opencode.proto   # proto 定义
      client/
        mod.rs           # gRPC client 封装
        session.rs       # session API
        event_stream.rs  # 事件流订阅
      view/
        mod.rs
        session.rs       # 会话视图
        home.rs          # 首页
        sidebar.rs       # 侧边栏
        diff.rs          # Diff 查看器
      widget/
        mod.rs
        prompt.rs        # 输入框
        dialog.rs        # 弹窗
        markdown.rs      # Markdown 渲染
        spinner.rs       # 加载指示器
        toast.rs         # 通知
        list.rs          # 列表
      highlight/
        mod.rs           # tree-sitter 集成
        parsers.rs       # 语言 parser 加载
      keymap/
        mod.rs           # 快捷键系统
        bindings.rs      # 键绑定定义
        which_key.rs     # 快捷键帮助
      theme/
        mod.rs           # 主题系统
        palette.rs       # 调色板
      config/
        mod.rs           # 配置解析
        kv.rs            # 持久化 KV

packages/
  opencode/
    src/
      grpc/              # gRPC server (新增)
        server.ts
        session.ts
        config.ts
        events.ts
      proto/
        opencode.proto   # proto 定义 (source of truth)
```

---

## 10. 关键待决问题

| # | 问题 | 选项 | 推荐 |
|---|------|------|------|
| 1 | gRPC sidecar 用 Rust 还是 Go 实现？ | Rust (同语言栈) / Go (更成熟的 gRPC 生态) | **Rust**：统一语言栈，减少认知负担 |
| 2 | gRPC server 嵌入 opencode 还是独立进程？ | napi-rs 嵌入 / 独立 sidecar | **独立 sidecar**：解耦、易调试 |
| 3 | 是否需要保留 REST fallback？ | 保留 / 完全替换 | **保留**：保证旧 TUI 和 SDK 兼容 |
| 4 | tree-sitter parser 是静态链接还是动态加载？ | 静态链接 / 动态加载 | **按需编译**：5-10 个高频语言静态链接，其余按需加载 |
| 5 | 插件系统是否迁移？ | 一期不迁移 / 全部重写 | **一期不迁移**：插件系统 ~5000 行，非核心路径 |
