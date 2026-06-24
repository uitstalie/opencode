# Rust 全栈重写设计文档

> 版本: 3.0 | 日期: 2026-06-24 | 分支: `opencode-rust-tui`

---

## 1. 战略变化

| 维度 | v1/v2 | v3 |
|------|-------|-----|
| 范围 | 只重写 TUI | **全栈 Rust**：TUI + CLI + 工具 + LLM |
| 依赖 | TypeScript 后端 + gRPC bridge | **零 TypeScript**，单 Rust 二进制 |
| 通信 | TUI → gRPC → TS backend → LLM | **TUI 直连 LLM API** |
| 交付 | 等后端 gRPC 适配完才能用 | Phase 1 即可独立运行 |

---

## 2. 架构

```
┌──────────────────────────────────────────────┐
│            crates/opencode/  (Rust)           │
│                                              │
│  ┌────────────┐  ┌──────────────────────┐   │
│  │   TUI      │  │        CLI            │   │
│  │  ratatui   │  │  clap (subcommands)   │   │
│  │  crossterm │  │  run / chat / config   │   │
│  └─────┬──────┘  └──────────┬───────────┘   │
│        │                    │                │
│  ┌─────▼────────────────────▼───────────┐   │
│  │           App Core                    │   │
│  │  ┌──────────┐  ┌──────────────────┐  │   │
│  │  │ Session  │  │  Permission       │  │   │
│  │  │ Messages │  │  Config           │  │   │
│  │  │ Events   │  │  System Prompt    │  │   │
│  │  └────┬─────┘  └────────┬─────────┘  │   │
│  └───────┼─────────────────┼────────────┘   │
│          │                 │                 │
│  ┌───────▼─────────────────▼────────────┐   │
│  │           Tool Layer                  │   │
│  │  bash | read | edit | write| glob    │   │
│  │  grep | task | webfetch| websearch   │   │
│  │  question | todowrite | skill        │   │
│  └──────────────────┬───────────────────┘   │
│                     │                        │
│  ┌──────────────────▼───────────────────┐   │
│  │          LLM Providers                │   │
│  │  reqwest → OpenAI / Anthropic / etc   │   │
│  └──────────────────────────────────────┘   │
│                                              │
│  Storage: sled (KV) for session/state       │
│  Config: JSONC via serde                     │
│  Highlight: tree-sitter native x10          │
└──────────────────────────────────────────────┘
```

---

## 3. 技术选型

### 3.1 运行时

| 层 | 选型 | 理由 |
|----|------|------|
| TUI | ratatui 0.28 + crossterm 0.27 | 成熟、活跃、文档好 |
| CLI | clap 4.x (derive) | Rust 事实标准 |
| 异步 | tokio 1.x (multi-thread) | 事实标准 |
| 错误处理 | thiserror + anyhow | 标准模式 |
| 日志 | tracing + tracing-subscriber | 结构化日志 |
| 序列化 | serde + serde_json | 标准 |
| HTTP | reqwest 0.12 | 直连 OpenAI / Anthropic API |
| SSE | reqwest-eventsource 或手动 | LLM streaming |

### 3.2 存储

| 数据 | 方案 |
|------|------|
| Session / Messages | sled (嵌入式 KV，LMDB 底层) |
| Config | JSONC 文件 → serde 解析 |
| User prefs (theme, sidebar) | sled |
| Undo blobs | git2 crate (libgit2 binding) |

### 3.3 工具层

| 工具 | Rust 实现 |
|------|-----------|
| `bash` | `std::process::Command` + tokio spawn |
| `read` | `std::fs::read_to_string` + 分页 |
| `edit` | 字符串替换 + `std::fs::write` |
| `write` | `std::fs::write` + `std::fs::create_dir_all` |
| `glob` | `glob` crate |
| `grep` | `grep` crate (ripgrep lib) |
| `webfetch` | reqwest GET → html2text / markdown |
| `websearch` | reqwest → 搜索引擎 API |
| `question` | TUI dialog |
| `todowrite` | sled 持久化 |
| `skill` | 文件系统加载 |
| `task` | 子 agent (Phase 2+) |

### 3.4 Markdown / 高亮 / Diff

| 功能 | 依赖 |
|------|------|
| Markdown 解析 | pulldown-cmark 0.12 |
| Markdown → TUI | ratatui-markdown (或不成熟则自研) |
| 语法高亮 | tree-sitter 0.23 native x10 语言 |
| Diff 算法 | similar 2.x |
| Diff 渲染 | 自定义 ratatui Span 着色 |
| 模糊匹配 | nucleo（替代 fuzzysort） |
| 剪贴板 | arboard 3.x |

### 3.5 输入

| 功能 | 依赖 |
|------|------|
| 多行输入 | tui-textarea 0.6 |
| 键盘事件 | crossterm event::read |
| 快捷键 | 自研 keymap module |
| 文件监听 | notify 7.x (侧边栏文件树) |

---

## 4. 文件结构

```
crates/
  opencode/
    Cargo.toml
    build.rs                     # tree-sitter parser 编译
    src/
      main.rs                    # 入口：clap CLI 解析 → TUI 或 debug
      app.rs                     # App 生命周期

      cli/                       # ── CLI 层 ──
        mod.rs
        debug.rs                 # DebugCmd 路由
        debug/
          mod.rs
          provider.rs            # debug provider list|test|models
          tool.rs                # debug tool list|run|schema
          config.rs              # debug config validate|show|path
          permission.rs          # debug permission check
          prompt.rs              # debug prompt show
          session.rs             # debug session list|show
          e2e.rs                 # debug e2e

      tui/                       # ── TUI 层 ──
        mod.rs
        app.rs                   # TUI 入口：终端 init、事件循环
        state.rs                 # AppState + Arc<RwLock<>>
        event.rs                 # 事件 reduce
        render.rs                # 顶层 layout (header/body/footer)

        view/                    # 页面级视图
          mod.rs
          session.rs             # 会话消息列表
          home.rs                # 首页 session 列表
          config.rs              # 配置界面

        widget/                  # 通用组件
          mod.rs
          prompt.rs              # 多行输入框
          dialog.rs              # 弹窗
          markdown.rs            # Markdown 渲染
          spinner.rs             # 加载指示器
          toast.rs               # 通知
          list.rs                # 通用列表

        builtins/                # 内置插件
          mod.rs
          diff.rs                # Diff 查看器
          sidebar.rs             # 文件树侧边栏
          which_key.rs           # 快捷键帮助
          notify.rs              # 通知系统
          theme.rs               # 主题选择器

        highlight/               # 语法高亮
          mod.rs
          parsers.rs             # tree-sitter 加载

        keymap/                  # 快捷键
          mod.rs
          bindings.rs

        theme/                   # 主题
          mod.rs
          palette.rs

        config/                  # 配置
          mod.rs
          kv.rs                  # sled KV 存储

      core/                      # ── 核心层 (CLI/TUI 共享) ──
        mod.rs
        session.rs               # Session / Message 模型
        permission.rs            # 权限评估
        config.rs                # JSONC 配置加载
        system_prompt.rs         # System prompt 构建
        event.rs                 # 事件定义
        provider.rs              # LLM provider trait
        tool.rs                  # Tool trait
        agent.rs                 # Agent 定义

      provider/                  # ── LLM Providers ──
        mod.rs
        openai.rs                # OpenAI API
        anthropic.rs             # Anthropic API
        google.rs                # Google API
        openrouter.rs            # OpenRouter
        deepseek.rs              # DeepSeek

      tool/                      # ── 工具实现 ──
        mod.rs
        bash.rs
        read.rs
        edit.rs
        write.rs
        glob.rs
        grep.rs
        webfetch.rs
        websearch.rs
        question.rs
        todowrite.rs
        skill.rs
        task.rs
        undo.rs                   # Undo blob store (git2)
        undo_edit.rs              # undo_edit tool
        apply_patch.rs
```

---

## 5. 核心抽象

### 5.1 LLM Provider Trait

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
        options: RequestOptions,
    ) -> Result<StreamedResponse, ProviderError>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError>;
}

pub struct StreamedResponse {
    pub stream: Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>,
}

pub enum StreamChunk {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallArgs(String),
    ToolCallEnd { id: String },
    Finish { usage: Usage },
}

pub struct RequestOptions {
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub system: String,
}
```

### 5.2 Tool Trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters(&self) -> serde_json::Value; // JSON Schema
    async fn execute(&self, params: serde_json::Value, ctx: ToolContext) -> Result<ToolOutput, ToolError>;
}

pub struct ToolContext {
    pub session_id: String,
    pub message_id: String,
    pub work_dir: PathBuf,
    pub ask: Box<dyn Fn(PermissionRequest) -> Result<PermissionResponse, AskError>>,
}

pub struct ToolOutput {
    pub title: String,
    pub output: String,
    pub metadata: serde_json::Value,
}
```

---

## 6. 状态管理

```
┌──────────────────────────────────────────────────┐
│  AppState (全局唯一，Arc<RwLock<AppState>>)        │
│                                                  │
│  view:        当前视图 (Home | Session | Config)   │
│  sessions:    Vec<SessionSummary>                 │
│  active:      Option<ActiveSession>               │
│  config:      AppConfig                           │
│  theme:       Theme                               │
│  mode:        InputMode (Normal | Insert | Cmd)    │
│  ui:          UIState (dialog stack, sidebar, ..) │
│  connection:  ConnectionState                     │
│                                                  │
│  事件循环:                                        │
│    tokio::select! {                              │
│      crossterm event → app.handle_input()        │
│      LLM stream msg → app.handle_chunk()         │
│      bg job finish   → app.handle_complete()     │
│    }                                             │
│                                                  │
│  渲染 (immediate mode):                           │
│    terminal.draw(|f| app.render(f))              │
│    每帧根据 state.read() 完整绘制，               │
│    ratatui 处理 diff → 最小终端输出                │
└──────────────────────────────────────────────────┘
```

### 6.1 状态更新模型

```rust
impl App {
    fn dispatch(&mut self, action: Action) {
        match action {
            Action::StartSession { mode, prompt } => {
                // 1. 保存 user message
                // 2. 构建 system prompt
                // 3. 调用 LLM provider
                // 4. fork background task for streaming
            }
            Action::StreamChunk { session_id, chunk } => {
                // 追加到 active session messages
                self.needs_redraw = true;
            }
            Action::ToolCall { session_id, tool_name, args } => {
                // fork tool execution
            }
            Action::ToolResult { session_id, output } => {
                // 追加 tool result，继续 LLM loop
            }
            Action::KeyPress { key } => {
                // 根据 InputMode 分发到 prompt / command / normal handler
            }
        }
    }
}
```

---

## 7. CLI 设计

### 7.1 入口

```bash
opencode                  # 无参数 → 启动 TUI
opencode run "prompt"     # headless 模式
opencode debug <cmd>      # 调试/测试命令
opencode --version
opencode --help
```

### 7.2 Debug 子命令

在 TUI 可用之前，`opencode debug` 是开发和测试的**唯一入口**。每个 core 模块必须有对应的 debug 命令。

```bash
# ── Provider 测试 ──
opencode debug provider list
  # 列出所有已配置的 LLM provider

opencode debug provider test <name> [--prompt "hello"]
  # 测试指定 provider：发送 prompt，streaming 输出，显示 usage
  # 示例: opencode debug provider test openai --prompt "say hi"

opencode debug provider models <name>
  # 列出 provider 的可用模型

# ── 工具测试 ──
opencode debug tool list
  # 列出所有已注册的工具

opencode debug tool run <name> [--params '{"key":"val"}']
  # 直接在真实文件系统上运行工具（⚠️ 无权限检查！）
  # 示例: opencode debug tool run bash --params '{"command":"ls -la"}'
  # 示例: opencode debug tool run read --params '{"filePath":"/tmp/test.txt"}'
  # 示例: opencode debug tool run edit --params '{"filePath":"...","oldString":"a","newString":"b"}'

opencode debug tool schema <name>
  # 打印工具的 JSON Schema 参数定义

# ── 配置测试 ──
opencode debug config validate
  # 验证 opencode.json 配置合法性

opencode debug config show
  # 打印解析后的配置（隐藏 secrets）

opencode debug config path
  # 显示配置文件路径（global + project）

# ── 权限测试 ──
opencode debug permission check <tool> <path>
  # 测试某工具对某路径的权限结果
  # 示例: opencode debug permission check edit ./src/main.rs
  # 输出: allow | deny | ask

# ── System Prompt 测试 ──
opencode debug prompt show [--mode build|plan]
  # 打印当前配置下的完整 system prompt

# ── Session 测试 ──
opencode debug session list
  # 列出已保存的 sessions

opencode debug session show <id>
  # 显示 session 消息历史

# ── E2E 测试 ──
opencode debug e2e "create a file hello.txt with content 'world'"
  # 完整端到端：发送 prompt → LLM → tools → 验证结果
  # 输出最终文件内容 + token usage
```

### 7.3 clap 定义

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "opencode")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Headless: run a prompt and exit
    #[arg(short, long)]
    run: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    Debug(DebugCmd),
}

#[derive(Subcommand)]
enum DebugCmd {
    Provider(ProviderCmd),
    Tool(ToolCmd),
    Config(ConfigCmd),
    Permission(PermissionCmd),
    Prompt(PromptCmd),
    Session(SessionCmd),
    E2e(E2eCmd),
}

#[derive(Subcommand)]
enum ProviderCmd {
    List,
    Test { name: String, prompt: Option<String> },
    Models { name: String },
}

#[derive(Subcommand)]
enum ToolCmd {
    List,
    Run { name: String, params: String },
    Schema { name: String },
}

// ... etc
```

### 7.4 实现策略

```
Phase 0:  CLI 骨架 (clap + debug provider list)
Phase 1:  core 实现 (providers, tools, config)
         → 全部通过 `opencode debug` 测试
Phase 2:  session + permission
         → 全部通过 `opencode debug` 测试
Phase 3+: TUI 在 core 稳定后开始搭建
```

**核心原则：TUI 上线之前，所有功能必须能通过 CLI 测试。**

---

## 8. Phase 1 范围（硬边界）

### 8.1 工具：全部 13 个

| 工具 | Phase 1 CLI | Phase 1 TUI |
|------|-------------|-------------|
| `bash` | `debug tool run bash` ✅ | 摘要显示 `🔧 bash: <command>` |
| `read` | `debug tool run read` ✅ | 同上 |
| `edit` | `debug tool run edit` ✅ | 同上 |
| `write` | `debug tool run write` ✅ | 同上 |
| `glob` | `debug tool run glob` ✅ | 同上 |
| `grep` | `debug tool run grep` ✅ | 同上 |
| `webfetch` | `debug tool run webfetch` ✅ | 同上 |
| `websearch` | `debug tool run websearch` ✅ | 同上 |
| `question` | `debug tool run question` ✅ | 弹窗：选项选择 |
| `todowrite` | `debug tool run todowrite` ✅ | 摘要 |
| `skill` | `debug tool run skill` ✅ | 摘要 |
| `task` | `debug tool run task` ✅ | 摘要（子 agent 异步） |
| `undo_edit` | `debug tool run undo_edit` ✅ | 摘要 |
| `apply_patch` | `debug tool run apply_patch` ✅ | 摘要 |

### 8.2 TUI 显示：纯文本 + 工具摘要

```
┌─ opencode ──────────────────────────────────────────┐
│                                                      │
│  user: 创建一个 hello.txt 写入 Hello World             │
│                                                      │
│  assistant: 我来创建这个文件。                          │
│                                                      │
│  🔧 write: hello.txt                                 │
│     Wrote file successfully.                         │
│                                                      │
│  assistant: 文件已创建。                               │
│                                                      │
├──────────────────────────────────────────────────────┤
│  > _                                        [Send]   │
└──────────────────────────────────────────────────────┘
```

**不包含（Phase 1 不做）**：
- ❌ Markdown 渲染（反引号、标题、列表等）
- ❌ 语法高亮
- ❌ Diff 查看器
- ❌ 侧边栏
- ❌ 首页 session 列表
- ❌ 快捷键帮助 (which-key)
- ❌ 通知系统
- ❌ 主题切换
- ❌ 思考模式切换
- ❌ 滚动加速曲线

### 8.3 权限弹窗：最简实现

```
┌─ Permission ────────────────────────────────────────┐
│                                                      │
│  bash: rm -rf /tmp/test                              │
│                                                      │
│  [A] Allow  [D] Deny                                 │
│                                                      │
└──────────────────────────────────────────────────────┘
```

只有两个选项：Allow / Deny。无 scope 显示、无 always 选项、无 reason 输入。

### 8.4 页面：仅 Session 视图

启动后直接进入 session 视图（新建 session）。无首页路由、无多 session 切换。

### 8.5 CLI Debug：全覆盖

```bash
# Provider
opencode debug provider list
opencode debug provider test openai --prompt "hello"
opencode debug provider models openai

# Tool (全部 13 个)
opencode debug tool list
opencode debug tool run bash --params '{"command":"ls -la"}'
opencode debug tool schema edit

# Config
opencode debug config validate
opencode debug config show
opencode debug config path

# Permission
opencode debug permission check edit ./src/main.rs
opencode debug permission check bash .

# System Prompt
opencode debug prompt show --mode build

# E2E
opencode debug e2e "创建 hello.txt 写入 Hello World"
```

### 8.6 Phase 1 完成标准

- [ ] `cargo build --release` 编译通过，零 warning
- [ ] `opencode debug provider test openai` 返回 LLM 响应
- [ ] `opencode debug tool run <name>` 全部 13 个工具通过
- [ ] `opencode debug config validate` 对合法/非法配置分别返回正确结果
- [ ] `opencode debug permission check` 返回 allow/deny/ask
- [ ] `opencode debug e2e "..."` 完整 tool loop 执行成功
- [ ] `opencode` (无参数) 启动 TUI，输入 prompt 后看到消息和工具执行

---

## 9. 实施阶段（修订 3）

### Phase 0: CLI 骨架 + Provider（2-3 天）

**目标**: `opencode debug provider test openai` 能调用 LLM 并流式输出

- [ ] `cargo init crates/opencode`
- [ ] Cargo.toml：clap (derive), tokio, reqwest, serde, tracing
- [ ] `main.rs`：clap CLI 入口，tracing 初始化
- [ ] `core/provider.rs`：LlmProvider trait
- [ ] `provider/openai.rs`：`chat()` + streaming
- [ ] `cli/debug/provider.rs`：list / test / models 命令
- [ ] `opencode debug provider test openai --prompt "hello"`

### Phase 1: 全部工具 + Config + Session + 最小 TUI（7-10 天）

**目标**: 全部 13 个工具可 CLI 测试；`opencode` 启动 TUI 能聊天 + 工具调用

**A. 工具 + 配置（3-4 天，纯 CLI）**

- [ ] `core/config.rs`：JSONC 加载 + validate + show + path
- [ ] `core/tool.rs`：Tool trait + ToolRegistry
- [ ] `tool/bash.rs` → `opencode debug tool run bash`
- [ ] `tool/read.rs` / `edit.rs` / `write.rs` / `glob.rs` / `grep.rs`
- [ ] `tool/webfetch.rs` / `websearch.rs` / `question.rs` / `todowrite.rs`
- [ ] `tool/skill.rs` / `task.rs` / `undo_edit.rs` / `apply_patch.rs`
- [ ] `undo.rs`：git2 blob store
- [ ] `cli/debug/tool.rs`：list / run / schema（全部 13 个）
- [ ] `cli/debug/config.rs`：validate / show / path
- [ ] `cli/debug/permission.rs`：check <tool> <path>
- [ ] `cli/debug/prompt.rs`：show
- [ ] `cli/debug/session.rs`：list / show
- [ ] `cli/debug/e2e.rs`：完整 tool loop

**B. Session + Permission + System Prompt（2-3 天）**

- [ ] `core/session.rs`：Session/Messages 模型，sled 持久化
- [ ] `core/permission.rs`：Rule.scope 评估 + $PROJECT 展开
- [ ] `core/system_prompt.rs`：8 section 模板渲染
- [ ] `core/event.rs`：事件类型定义

**C. 最小 TUI（2-3 天）**

- [ ] `tui/app.rs`：ratatui + crossterm 终端初始化，事件循环
- [ ] `tui/state.rs`：AppState (Arc<RwLock>)，dispatch/reduce
- [ ] `tui/view/session.rs`：纯文本消息列表 + 滚动
- [ ] `tui/widget/prompt.rs`：tui-textarea 多行输入，Enter 发送
- [ ] 工具调用显示：`🔧 tool_name: summary`
- [ ] 权限弹窗：`[A] Allow  [D] Deny`
- [ ] LLM streaming → TUI 实时文本追加
- [ ] tool loop：LLM → tool call → execute → result → LLM

**Phase 1 完成标准**: 见 §8.6

### Phase 2: Markdown + 语法高亮 + Diff（5-7 天）

- [ ] tree-sitter 编译集成（10 核心语言）
- [ ] `tui/highlight/`：代码块自动高亮
- [ ] `tui/widget/markdown.rs`：pulldown-cmark → ratatui-markdown
- [ ] `tui/builtins/diff.rs`：Git diff + last turn diff，分屏/统一视图
- [ ] `tui/builtins/sidebar.rs`：文件树（notify 实时更新）

### Phase 3: 内置插件 + 主题 + 打磨（4-6 天）

- [ ] `tui/builtins/which_key.rs`：快捷键帮助
- [ ] `tui/builtins/notify.rs`：通知系统
- [ ] `tui/builtins/theme.rs`：主题切换
- [ ] `tui/theme/`：6 个内置主题 + ANSI palette
- [ ] `tui/keymap/`：Normal/Insert/Command 模式
- [ ] `tui/config/kv.rs`：sled 持久化用户偏好
- [ ] 错误处理完善（重连、超时、provider 切换）
- [ ] 首页 session 列表

### Phase 4: 清理 TS 代码（2-3 天）

- [ ] 移除 `packages/tui/`
- [ ] 移除 `packages/opencode/`
- [ ] 更新 Makefile：单一 `cargo build --release`
- [ ] 更新 AGENTS.md

**总预估**: 20-29 天

---

## 10. v2 vs v3 关键差异

| 维度 | v2 | v3 |
|------|-----|-----|
| TS 后端 | 保留 `packages/opencode/` | **移除** |
| gRPC | tonic + proto + sidecar | **不需要** |
| 工具执行 | gRPC → TS backend → OS | **直接 std::process::Command** |
| LLM 调用 | gRPC → TS backend → reqwest | **直连 API** |
| 配置 | JSONC 从 TS 传过来 | **直接读取文件** |
| 构建 | cargo + bun + Makefile | **cargo only** |
| 复杂度 | 中（需要维护两套代码） | **低（单代码库）** |

---

## 11. 风险

| 风险 | 概率 | 缓解 |
|------|------|------|
| Provider API 差异大 | 中 | Provider trait 抽象，每个 provider 独立实现 |
| ratatui 组件不满足需求 | 中 | 自定义 widget，ratatui 支持完全自定义 render |
| tree-sitter parser 编译复杂 | 低 | tree-sitter 0.23 有预编译 binding，Cargo.toml 声明即可 |
| LLM streaming 实现缺陷多 | 中 | reqwest + tokio Stream 成熟，参考现有 TS 实现 |
| 权限系统迁移遗漏 | 中 | 对照 TS 源码逐个迁移测试用例 |
| 工期超预期 | 高 | Phase 1 即可独立运行，Phase 2 已可日常使用 |

---

## 12. 已决问题（v3）

| # | 问题 | 决策 |
|---|------|------|
| 1 | 语言 | **100% Rust**，零 TypeScript |
| 2 | 架构 | **单二进制**，TUI + CLI + 后端一体 |
| 3 | 通信 | **无 gRPC**，直连 LLM API |
| 4 | 存储 | **sled** (KV)，不引入 SQLite |
| 5 | 构建 | **cargo build --release**，无 bun/turbo |
| 6 | 交付节奏 | **Phase 1 即可用**，渐进增强 |
| 7 | 旧代码 | **Phase 5 清理** packages/tui/ + packages/opencode/ |
| 8 | Provider | **reqwest 直连**，每个 provider 独立 module |
| 9 | 配置 | **JSONC → serde**，兼容现有 opencode.json |
| 10 | 语法高亮 | **tree-sitter native x10**，静态链接 |

---

## 13. Proto Definitions for V3

No proto or gRPC. Provider-specific JSON payloads are the only wire format.

```
V2:   TUI (Rust) → gRPC → Backend (TS) → HTTP → LLM
V3:   TUI (Rust) ──────────── HTTP ──────────→ LLM
```

---

*This design is now complete. If a topic needs more detail (e.g., permission migration plan, tree-sitter parser list, provider-specific API differences), it should be written as a supplementary document, not part of this main design.*
