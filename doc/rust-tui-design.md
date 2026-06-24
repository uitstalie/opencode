# Rust TUI 重写设计文档

> 版本: draft-2 | 日期: 2026-06-24 | 分支: `opencode-rust-tui`

---

## 1. 动机

### 1.1 当前问题

| 维度 | 现状 | 问题 |
|------|------|------|
| 运行时 | TypeScript (Bun) + SolidJS + Effect | 冷启动慢（2-3s），内存占用高（150MB+） |
| 渲染引擎 | @opentui/solid v0.3.4（闭源） | 不可控，升级依赖外部发布节奏 |
| 帧率 | 目标 60fps，实际取决于 SSE 事件吞吐 | 大量 SSE 事件时出现掉帧 |
| 语法高亮 | 30 个 tree-sitter WASM parser | 启动加载慢，每次解析需跨 WASM 边界 |
| 类型系统 | TypeScript 类型擦除 | 运行时缺乏类型保障 |

### 1.2 目标

| 指标 | 当前 | 目标 |
|------|------|------|
| 启动时间 | 2-3s | **<500ms** |
| 内存占用 | 150MB+ | **<50MB** |
| 帧率 | 不稳定 | **稳定 60fps** |
| 可控性 | 依赖闭源 @opentui | **全栈自维护** |
| 类型安全 | 运行时 | **编译期** |

---

## 2. 当前架构

```
┌─────────────────────────────────┐
│  packages/tui/  (~27K TSX)      │
│  ┌───────────────────────────┐  │
│  │  context/ (3221 行)        │  │
│  │  SDKProvider → SyncProvider│  │
│  │    → DataProvider          │  │
│  │    → Theme/Local/KV/...   │  │
│  ├───────────────────────────┤  │
│  │  routes/ (4625 行)         │  │
│  │  session/index.tsx (2648)  │  │
│  │  home/                     │  │
│  ├───────────────────────────┤  │
│  │  feature-plugins/ (3453 行)│  │
│  │  diff-viewer (1059)        │  │
│  │  sidebar, which-key,       │  │
│  │  notifications             │  │
│  ├───────────────────────────┤  │
│  │  component/ (7581 行)      │  │
│  │  prompt/ dialog/ spinner   │  │
│  ├───────────────────────────┤  │
│  │  plugin/ (661 行)          │  │
│  │  runtime, slots, adapters  │  │
│  └───────────────────────────┘  │
│  Dependencies:                  │
│   @opentui/solid (闭源)          │
│   SolidJS 1.9 + Effect 4.0      │
│   tree-sitter WASM x30          │
└──────────┬──────────────────────┘
           │ REST + SSE (JSON)
┌──────────▼──────────────────────┐
│  packages/opencode (TypeScript) │
│  HTTP API + SSE event stream    │
│  Session/LLM/Tool/Permission    │
└─────────────────────────────────┘
```

---

## 3. 目标架构

```
┌─────────────────────────────────────────┐
│  crates/opencode-tui/  (Rust ~7K 行)    │
│                                         │
│  app/       入口、启动、生命周期           │
│  state/     全局状态 + 事件 reduce        │
│  proto/     protobuf 定义 + 生成代码      │
│  client/    gRPC 客户端封装               │
│  view/      页面级视图                    │
│  widget/    通用 UI 组件                  │
│  builtins/  内置插件（diff/sidebar/...）   │
│  highlight/ tree-sitter 原生高亮          │
│  keymap/    快捷键模式系统                │
│  theme/     主题 + ANSI 调色板            │
│  config/    JSONC 配置解析 + KV          │
│                                         │
│  Cargo.toml:                            │
│   ratatui 0.28, crossterm 0.27          │
│   tonic 0.12, prost 0.13                │
│   tokio 1.x, serde 1.x                  │
│   tree-sitter 0.23, tui-textarea 0.6    │
│   notify 7.x, arboard 3.x               │
│   similar 2.x, pulldown-cmark 0.12      │
│   jsonc-parser (手工或社区 crate)          │
└──────────────┬──────────────────────────┘
               │ gRPC (protobuf over HTTP/2)
               │ 双向 stream: SubscribeEvents
┌──────────────▼──────────────────────────┐
│  packages/opencode/  (扩展)             │
│  ┌───────────────────────────────────┐  │
│  │  src/grpc/     gRPC server (新增)  │  │
│  │  @bufbuild/connect 或 @grpc/grpc-js│  │
│  │  直接嵌入 opencode 进程            │  │
│  │  * session.store 暴露 gRPC        │  │
│  │  * config.store 暴露 gRPC         │  │
│  │  * 事件系统 → gRPC stream 桥接     │  │
│  └───────────────────────────────────┘  │
│  现有 HTTP/SSE API 保留（SDK/旧TUI兼容）  │
└─────────────────────────────────────────┘
```

**关键简化**：
- gRPC server 用 TypeScript 实现，**嵌入 opencode 进程**，无额外进程
- proto 文件 source of truth 在 `packages/opencode/src/proto/`
- Rust 端用 tonic-build 从同一 proto 生成客户端
- 旧 TUI（packages/tui/）**直接废弃**，不共存

---

## 4. 技术选型

### 4.1 终端框架

| 依赖 | 版本 | 用途 |
|------|------|------|
| `ratatui` | 0.28 | 声明式 TUI 组件库，Layout/Block/Paragraph/List/Table |
| `crossterm` | 0.27 | 跨平台终端后端，键盘/鼠标事件，kitty 协议 |
| `tui-textarea` | 0.6 | 多行文本输入（prompt 组件基础） |

### 4.2 通信协议 (gRPC)

```
Server:  TypeScript → packages/opencode/src/grpc/
         使用 @bufbuild/connect (推荐) 或 @grpc/grpc-js
         嵌入 opencode 进程，共享 session store / config / 事件系统

Client:  Rust → crates/opencode-tui/
         tonic + prost, tonic-build 从 proto 生成代码
         tokio 异步 runtime 驱动

Proto:   packages/opencode/src/proto/opencode.proto
         source of truth，两端共享
```

### 4.3 状态管理

ratatui 是 **immediate mode** 渲染，每帧完整绘制。状态管理策略：

```
┌──────────────────────────────────────────┐
│  AppState (Arc<RwLock<AppState>>)         │
│                                          │
│  struct AppState {                       │
│    connection: ConnectionState,          │
│    sessions: Vec<SessionSummary>,        │
│    active_session: Option<SessionView>,  │
│    config: Config,                       │
│    theme: Theme,                         │
│    mode: InputMode,        // normal/insert/command
│    ui: UIState,            // sidebar, dialog stack
│  }                                       │
│                                          │
│  事件循环:                                │
│    tokio::select! {                      │
│      gRPC event → state.write().reduce() │
│      key event  → state.write().handle() │
│      tick       → redraw flag check      │
│    }                                     │
│                                          │
│  渲染循环:                                │
│    if redraw_needed {                    │
│      terminal.draw(|f| ui.render(f, &state.read())) │
│    }                                     │
└──────────────────────────────────────────┘
```

### 4.4 Markdown 渲染

| 方案 | 选型 |
|------|------|
| 解析器 | `pulldown-cmark` 0.12 — Rust 生态标准 Markdown parser |
| TUI 转换 | `ratatui-markdown` — 将 pulldown-cmark AST 转 ratatui Text/Span |
| 代码块 | tree-sitter 语法高亮注入 fenced code blocks |
| 链接 | crossterm 的 `EnableHyperlinks` 序列（现代终端支持） |

### 4.5 语法高亮

| 维度 | 决策 |
|------|------|
| 引擎 | `tree-sitter` 0.23 native，静态链接 |
| 语言 | **10 个核心语言**：TypeScript/TSX, Rust, JSON, Markdown, Python, YAML, TOML, Bash, Go, C |
| 加载 | 全部静态链接（编译后 ~5MB parser 二进制），无运行时加载 |
| 增量解析 | 利用 tree-sitter `edit()` 方法，编辑后仅重解析变更 AST 子树 |

### 4.6 Diff 渲染

| 维度 | 决策 |
|------|------|
| 算法 | `similar` crate — Rust 原生 diff 引擎 |
| 着色 | 自定义 ratatui Span：添加行绿色、删除行红色 |
| 分屏 | ratatui Layout::horizontal 双栏 + 同步滚动 |

### 4.7 文件操作

| 功能 | 依赖 | 用途 |
|------|------|------|
| 文件监听 | `notify` 7.x | 侧边栏文件树实时更新 |
| 剪贴板 | `arboard` 3.x | 跨平台剪贴板访问 |
| 外部编辑器 | `open` crate + `Command::new` | 打开外部编辑器编辑文件 |

### 4.8 其他依赖

| 功能 | 依赖 | 用途 |
|------|------|------|
| 模糊匹配 | `nucleo` | 文件路径/命令自动补全（替代 fuzzysort） |
| 日志 | `tracing` + `tracing-subscriber` | 结构化日志，后续诊断 |
| KV 存储 | `sled` 或 `rusqlite` | 用户偏好持久化（theme、sidebar 状态等） |
| ANSI 处理 | `strip-ansi-escapes` | 清理 LLM 输出中的 ANSI 转义码 |
| HTTP | `reqwest` | gRPC 之外可能需要 REST fallback 调用 |

---

## 5. 组件映射（全量）

| 当前 (TypeScript) | 行数 | 目标 (Rust) | 行数(估) | 说明 |
|---|---|---|---|---|
| `context/sync.tsx` | 655 | `state/sync.rs` | ~400 | gRPC 事件流处理 |
| `context/data.tsx` | 581 | 纳入 `state/` | — | V2 事件层并入 sync |
| `routes/session/index.tsx` | 2648 | `view/session.rs` | ~1500 | 消息列表、权限弹窗、思考切换 |
| `feature-plugins/diff-viewer.tsx` | 1059 | `builtins/diff.rs` | ~800 | 分屏/统一 diff、文件树 |
| `feature-plugins/sidebar.tsx` | ~800 | `builtins/sidebar.rs` | ~500 | 文件树侧边栏 |
| `feature-plugins/which-key.tsx` | ~500 | `builtins/which_key.rs` | ~300 | 快捷键帮助 |
| `feature-plugins/notifications.tsx` | ~600 | `builtins/notify.rs` | ~300 | 通知系统 |
| `feature-plugins/theme-selector.tsx` | ~400 | `builtins/theme.rs` | ~200 | 主题切换 |
| `component/prompt/` | ~1200 | `widget/prompt.rs` | ~600 | 多行输入、自动补全、历史 |
| `component/dialog/` | ~800 | `widget/dialog.rs` | ~350 | 弹窗系统（modal stack） |
| `ui/` | 2029 | `widget/` | ~1200 | 通用 UI 组件（button/spinner/toast/...） |
| `keymap.tsx` | ~600 | `keymap/mod.rs` | ~400 | 模式系统、键绑定 |
| `theme/` | 1089 | `theme/mod.rs` | ~400 | 主题 + ANSI palette |
| `config/` | 594 | `config/mod.rs` | ~350 | JSONC 解析 + schema 验证 |
| `util/` | 893 | 分散到各模块 | — | 内联到使用处 |
| `prompt/display.ts` | 386 | `widget/markdown.rs` | ~300 | Markdown → ratatui Text |
| tree-sitter WASM x30 | — | tree-sitter native x10 | — | 原生绑定 |
| `context/kv.tsx` | ~200 | `config/kv.rs` | ~150 | 持久化用户偏好 |
| **插件系统** (~5000行) | — | **外置插件废弃** | — | 内置功能已映射到 builtins/ |
| **总计 ~27K** | | **~7K Rust** | | |

---

## 6. 协议设计

### 6.1 Proto 文件结构

Proto source of truth: `packages/opencode/src/proto/opencode.proto`

```protobuf
syntax = "proto3";
package opencode.v1;

service OpenCodeService {
  // ── 会话 ──
  rpc ListSessions(SessionFilter) returns (SessionList);
  rpc GetSession(SessionID) returns (Session);
  rpc DeleteSession(SessionID) returns (Empty);

  // ── 消息 ──
  rpc SendMessage(SendMessageRequest) returns (stream MessageEvent);
  rpc CancelMessage(CancelRequest) returns (Empty);

  // ── 事件流 (替代 SSE) ──
  rpc SubscribeEvents(EventFilter) returns (stream Event);

  // ── 配置 ──
  rpc GetConfig(Empty) returns (Config);

  // ── 权限 ──
  rpc ResolvePermission(PermissionRequest) returns (PermissionResponse);

  // ── Todo ──
  rpc ListTodos(Empty) returns (TodoList);
}

// ── 消息类型 ──

message SessionFilter {
  string directory = 1;  // 可选，按工作目录筛选
  int32 limit = 2;       // 默认 50
  int32 offset = 3;
}

message SessionSummary {
  string id = 1;
  string title = 2;
  string mode = 3;
  int64 created_at = 4;
  int64 updated_at = 5;
  string status = 6;     // "idle" | "busy" | "error"
}

message SessionList {
  repeated SessionSummary sessions = 1;
  int32 total = 2;
}

message Message {
  string id = 1;
  string session_id = 2;
  string role = 3;       // "user" | "assistant" | "tool" | "system"
  repeated Part parts = 4;
  int64 created_at = 5;
}

message Part {
  string id = 1;
  string type = 2;       // "text" | "reasoning" | "tool_call" | "tool_result" | "image"
  string content = 3;    // JSON or plain text
  map<string, string> metadata = 4;
}

message Event {
  string type = 1;
  bytes payload = 2;     // JSON, deserialized based on type
}

message Config {
  string model = 1;
  string mode = 2;
  // ... extend as needed
}
```

### 6.2 事件流模型

```
gRPC: SubscribeEvents(EventFilter) returns (stream Event)

EventFilter 可指定:
  - session_ids: 只收特定 session 事件
  - event_types: 只收特定类型 (session.updated, message.part.delta, ...)

Rust 端:
  let stream = client.subscribe_events(filter).await?;
  while let Some(event) = stream.message().await? {
      state.write().reduce(event);
      redraw_tx.send(());  // 触发渲染帧
  }
```

### 6.3 后端实现要点

```typescript
// packages/opencode/src/grpc/server.ts
// 使用 @bufbuild/connect

import { ConnectRouter } from "@bufbuild/connect";
import { OpenCodeService } from "./gen/opencode_connect";

export function grpcRoutes(router: ConnectRouter) {
  router.service(OpenCodeService, {
    async *subscribeEvents(filter, { signal }) {
      // 桥接 opencode 内部事件系统 → gRPC stream
      const bus = yield* EventBus.Service;
      for await (const event of bus.subscribe(filter)) {
        yield { type: event.type, payload: JSON.stringify(event.data) };
      }
    },
    // ... 其他 RPC
  });
}
```

---

## 7. 状态管理详细设计

### 7.1 AppState

```rust
use std::sync::{Arc, RwLock};

pub struct App {
    pub state: Arc<RwLock<AppState>>,
    pub client: GrpcClient,
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
    pub redraw_tx: tokio::sync::watch::Sender<bool>,
}

pub struct AppState {
    // 连接状态
    pub connection: ConnectionState,

    // 会话
    pub sessions: Vec<SessionSummary>,
    pub active_session_id: Option<String>,
    pub active_session: Option<SessionView>,

    // UI 状态
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub dialog_stack: Vec<Dialog>,
    pub sidebar_visible: bool,
    pub status_message: Option<String>,

    // 配置
    pub config: AppConfig,
    pub theme: Theme,

    // 待办
    pub todos: Vec<Todo>,
}

pub struct SessionView {
    pub info: Session,
    pub messages: Vec<Message>,
    pub permissions: Vec<PermissionRequest>,
    pub scroll_offset: u16,
}

pub enum InputMode {
    Normal,
    Insert,
    Command,
}

pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected { reason: String },
}
```

### 7.2 事件 Reduce

```rust
impl AppState {
    pub fn reduce(&mut self, event: &Event) {
        match event.event_type.as_str() {
            "session.updated" => self.handle_session_update(&event.payload),
            "message.created" => self.handle_message_created(&event.payload),
            "message.part.delta" => self.handle_part_delta(&event.payload),
            "permission.asked" => self.handle_permission(&event.payload),
            "todo.updated" => self.handle_todo(&event.payload),
            _ => tracing::debug!("unhandled event: {}", event.event_type),
        }
    }
}
```

### 7.3 事件循环 (main loop)

```rust
// 三个并发任务，通过 channel 协调

// 任务 1: gRPC 事件流 → state reduce
tokio::spawn(async move {
    let mut stream = client.subscribe_events(filter).await?;
    while let Some(event) = stream.message().await? {
        state.write().unwrap().reduce(&event.into());
        let _ = redraw_tx.send(true);
    }
});

// 任务 2: 终端输入 → state handle
tokio::spawn(async move {
    loop {
        let event = crossterm::event::read()?;
        state.write().unwrap().handle_input(event);
        let _ = redraw_tx.send(true);
    }
});

// 任务 3: 渲染循环 (main thread or spawned)
tokio::spawn(async move {
    let mut redraw_rx = redraw_tx.subscribe();
    loop {
        let _ = redraw_rx.changed().await;
        let state = app_state.read().unwrap();
        terminal.draw(|f| ui::render(f, &state))?;
    }
});
```

---

## 8. 配置设计

### 8.1 JSONC 解析策略

`opencode.json` / `.opencode/opencode.jsonc` 保持不变。Rust 侧用 serde 解析：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub model: Option<String>,
    pub mode: Option<String>,
    pub theme: Option<String>,
    pub keybindings: Option<HashMap<String, String>>,
    // ... 映射现有 opencode.json 字段
}

impl AppConfig {
    pub fn load(paths: &[PathBuf]) -> Result<Self, ConfigError> {
        // 1. 读取 JSONC → strip comments → serde_json
        // 2. 多文件 merge (global → project → .opencode/)
        // 3. validate
    }
}
```

JSONC 注释处理：用 `json_comments` crate 或手写简单 strip 逻辑。

### 8.2 KV 持久化

用户偏好（theme、sidebar 状态等）用 `sled` 嵌入式 DB 存储：

```rust
use sled::Db;

pub struct KvStore {
    db: Db,
}

impl KvStore {
    pub fn get(&self, key: &str) -> Option<String> { ... }
    pub fn set(&self, key: &str, value: &str) { ... }
}
```

存储路径：`~/.local/share/opencode/tui-kv/`

---

## 9. 构建集成

### 9.1 目录结构

```
crates/
  opencode-tui/
    Cargo.toml
    build.rs          # tonic-build proto 编译
    src/
      main.rs
      app.rs
      state/
        mod.rs
        event.rs
        sync.rs
      proto/
        mod.rs
        gen/           # tonic-build 生成代码 (.gitignore)
      client/
        mod.rs
        session.rs
      view/
        mod.rs
        session.rs
        home.rs
      widget/
        mod.rs
        prompt.rs
        dialog.rs
        markdown.rs
        spinner.rs
        toast.rs
        list.rs
      builtins/
        mod.rs
        diff.rs
        sidebar.rs
        which_key.rs
        notify.rs
        theme.rs
      highlight/
        mod.rs
        parsers.rs
      keymap/
        mod.rs
        bindings.rs
      theme/
        mod.rs
        palette.rs
      config/
        mod.rs
        kv.rs

packages/
  opencode/
    src/
      proto/
        opencode.proto    ← source of truth
      grpc/
        server.ts
        session.ts
        config.ts
        events.ts
```

### 9.2 Makefile

```makefile
# 项目根目录 Makefile

.PHONY: build build-ts build-rust dev clean

# 全量构建
build: build-ts build-rust

# TypeScript 后端
build-ts:
	cd packages/opencode && bun run build --single --skip-embed-web-ui

# Rust TUI
build-rust:
	cd crates/opencode-tui && cargo build --release

# 开发模式
dev:
	tmux new-session -d -s opencode-dev 'cd packages/opencode && bun dev'
	cd crates/opencode-tui && cargo run

# 清理
clean:
	cd crates/opencode-tui && cargo clean
```

### 9.3 Proto 生成流程

```
packages/opencode/src/proto/opencode.proto
            │
    ┌───────┴───────┐
    │               │
    ▼               ▼
TypeScript        Rust
buf generate    tonic-build
    │               │
    ▼               ▼
src/grpc/gen/   src/proto/gen/
```

---

## 10. 测试策略

### 10.1 单元测试

```rust
// ratatui buffer snapshot 测试
#[test]
fn test_session_view_renders_messages() {
    let state = AppState::test_fixture();
    let mut backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| {
        view::session::render(f, f.size(), &state);
    }).unwrap();

    // snapshot buffer 比对
    let expected = Buffer::with_lines(vec![
        "┌─ Session: test ─────────────────────────────────────────┐",
        "│ user: hello                                             │",
        "│ assistant: hi there                                     │",
        "└─────────────────────────────────────────────────────────┘",
    ]);
    terminal.backend().assert_buffer(&expected);
}
```

### 10.2 集成测试

```rust
// 启动真实 opencode server + gRPC，验证端到端
#[tokio::test]
async fn test_list_sessions_integration() {
    // 1. 启动 opencode server (子进程)
    let mut server = opencode_server_spawn();
    // 2. 创建 gRPC client
    let mut client = GrpcClient::connect(server.grpc_addr()).await.unwrap();
    // 3. 调用 ListSessions
    let sessions = client.list_sessions(SessionFilter::default()).await.unwrap();
    // 4. 验证
    assert!(!sessions.is_empty());
}
```

### 10.3 测试分层

| 层 | 工具 | 覆盖 |
|----|------|------|
| 纯函数 | `#[test]` | state reduce、config parse、diff 算法 |
| UI 渲染 | ratatui TestBackend + buffer snapshot | 所有 view/widget render 函数 |
| 集成 | `#[tokio::test]` + 子进程 opencode | gRPC client、事件流、完整交互 |

---

## 11. 实施阶段（修订）

### Phase 0: 环境搭建（1-2 天）

- [ ] `cargo init crates/opencode-tui`
- [ ] Cargo.toml 依赖声明
- [ ] `build.rs` tonic-build proto 编译
- [ ] `main.rs`：终端初始化 + 事件循环 + 渲染循环骨架
- [ ] ratatui hello world 验证
- [ ] Makefile 集成到 monorepo
- [ ] 后端 `packages/opencode/src/grpc/` 目录骨架 + `@bufbuild/connect` 依赖

### Phase 1: 通信层（4-6 天）

- [ ] 编写完整 `opencode.proto`
- [ ] TS 侧：buf generate → gRPC server 实现
- [ ] TS 侧：事件系统 → gRPC stream 桥接
- [ ] Rust 侧：tonic-build → gRPC client
- [ ] Rust 侧：SubscribeEvents 事件流验证
- [ ] 验证：Rust TUI 能获取 session 列表 + 收到实时事件

### Phase 2: 状态管理 + 首页（4-5 天）

- [ ] `AppState` struct + `Arc<RwLock<AppState>>`
- [ ] 事件循环：gRPC stream → reduce → redraw
- [ ] 渲染循环框架
- [ ] 基础布局（header + body + footer）
- [ ] Session 列表首页
- [ ] 键盘输入处理（keymap 基础）
- [ ] 连接状态指示

### Phase 3: 核心视图（8-12 天）

- [ ] Session 详情视图（消息列表 + 文本渲染）
- [ ] Markdown 渲染（pulldown-cmark → ratatui-markdown）
- [ ] Prompt 输入框（tui-textarea + 历史 + 自动补全）
- [ ] 权限弹窗（dialog stack）
- [ ] Diff 查看器（similar + 分屏布局）
- [ ] 侧边栏（notify + 文件树）
- [ ] 思考模式切换

### Phase 4: 内置插件 + 语法高亮（5-7 天）

- [ ] tree-sitter 原生集成（10 核心语言）
- [ ] Which-key 快捷键帮助
- [ ] Notifications 通知系统
- [ ] Theme selector 主题切换
- [ ] 主题系统（ANSI palette + 6 个内置主题）
- [ ] 配置加载（JSONC 解析 + 验证）

### Phase 5: 打磨 + 废弃旧 TUI（4-6 天）

- [ ] 性能调优（事件批处理、渲染节流）
- [ ] 全套快捷键映射（对比旧 TUI keymap）
- [ ] 错误处理 / 重连机制
- [ ] `packages/tui/` 代码移除
- [ ] 构建脚本更新（移除旧 TUI 构建步骤）
- [ ] `bun dev` 适配 Rust TUI 启动

**总预估**：26-38 天（取决于 Phase 3 复杂度）

---

## 12. 后端变更清单

### 12.1 packages/opencode/ 新增

```
src/
  proto/
    opencode.proto          # Proto 定义 (source of truth)
    buf.gen.yaml            # buf generate 配置
  grpc/
    gen/                    # buf generate 输出 (.gitignore)
    server.ts               # gRPC server 初始化
    session.ts              # Session RPC handler
    config.ts               # Config RPC handler
    events.ts               # SubscribeEvents handler
    permission.ts           # ResolvePermission handler
```

### 12.2 packages/opencode/ 改动

- `package.json`：加 `@bufbuild/connect`、`@bufbuild/protobuf`
- `src/index.ts`：gRPC server 启动（复用 HTTP 端口或独立端口）
- `session/store.ts`：暴露 gRPC 需要的查询接口
- `config/`：暴露 Config RPC
- `permission/`：暴露 Permission RPC

### 12.3 不移除的内容

- HTTP REST API 不变（SDK、ACP 客户端仍依赖）
- SSE event stream 不变（旧客户端兼容）
- `packages/tui/` 在 Phase 5 移除

---

## 13. 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| ratatui 组件灵活度不足 | 高 | 中 | 自定义 widget trait，参考 SolidJS 响应式模式设计 |
| 全量重写工期超预期 | 高 | 高 | 分阶段交付，Phase 2 即可用（session 列表），Phase 3 已可日常使用 |
| ratatui-markdown 不成熟 | 中 | 中 | 自研 fallback：pulldown-cmark AST → 手工 Span 组装 |
| gRPC 协议频繁变更 | 中 | 中 | proto 版本化 (v1)，REST fallback |
| tree-sitter parser 构建复杂 | 低 | 中 | tree-sitter 提供预编译 binding，Cargo.toml 声明依赖即可 |
| 插件外置用户流失 | 低 | 低 | 旧 TUI 废弃后外置插件自然失去渲染目标，文档提前公告 |
| gRPC server 嵌入影响主进程稳定性 | 低 | 高 | 独立 tokio runtime，错误隔离；gRPC 崩溃不影响 HTTP |

---

## 14. 已决问题汇总

| # | 问题 | 决策 | 理由 |
|---|------|------|------|
| 1 | TUI 框架 | **ratatui + crossterm** | 社区最活跃，immediate mode 匹配需求 |
| 2 | 重写策略 | **全量重写，旧 TUI 直接废弃** | 架构最干净，无维护双重代码负担 |
| 3 | 通信协议 | **gRPC** | 类型安全 + 双向流 + Rust 一等支持 |
| 4 | gRPC server 语言 | **TypeScript，嵌入 opencode 进程** | 共享内部状态，零额外部署 |
| 5 | 插件系统 | **内置全部保留，外置废弃** | diff/sidebar/which-key/notify/theme 适配到 builtins/ |
| 6 | Markdown 渲染 | **ratatui-markdown + pulldown-cmark** | pulldown-cmark 成熟，ratatui-markdown 社区活跃 |
| 7 | 语法高亮 | **tree-sitter native，10 核心语言静态链接** | 编译后 ~5MB，不需 WASM 跨边界 |
| 8 | 配置格式 | **JSON/JSONC 不变** | 零用户迁移成本 |
| 9 | 构建集成 | **独立 cargo + Makefile** | 简单解耦，不侵入 bun/turbo |
| 10 | 测试 | **cargo test + 集成测试** | snapshot buffer 测 UI，子进程测 gRPC |
| 11 | 旧 TUI | **Phase 5 移除 packages/tui/** | 无共存期，直接替换 |
