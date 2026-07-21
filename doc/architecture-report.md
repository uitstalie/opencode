# openrust 模块架构梳理报告

## 总览

| 模块 | 文件数 | 总行数 | 测试数 | 核心职责 |
|------|--------|--------|--------|----------|
| `main.rs` | 1 | 112 | 0 | 入口：clap 解析、日志、panic hook |
| `lib.rs` | 1 | 10 | 0 | 模块声明 |
| `system_prompt.rs` | 1 | 466 | 9 | 系统 prompt 组装（7 个 XML 段落） |
| `cli/` | 13 | ~1,600 | 13 | `openrust debug <subcommand>` 调试验证 |
| `core/` | 14 | ~5,700 | 106 | 配置、会话、权限、加密、memory、provider trait |
| `provider/` | 4 | ~1,700 | 10 | 3 个 LLM 协议实现（OpenAI/Anthropic/Gemini） |
| `tool/` | 21 | ~4,600 | 76 | 17 个工具 + 权限门控 + 目录注册 |
| `tui/` | 36 | ~8,800 | 55 | 终端 UI（ratatui + crossterm） |
| **合计** | **~91** | **~23,000** | **269** | |

---

## 1. `cli/` — Debug 调试入口

**结构**: `mod.rs` (dispatch) -> `debug/` (11 个子命令)

| 子命令 | 功能 | 风险等级 |
|--------|------|----------|
| `config` | 查看/修改配置 | **高** — 写入全局配置文件，非原子写入 |
| `provider` | 测试 LLM 连接 | 中 — 网络请求 |
| `session` | 会话管理 | **高** — `delete-all` 无确认 |
| `tool` | 列出/运行工具 | **高** — 可执行任意工具 |
| `vault` | 凭据管理 | **高** — API key 暴露在命令行参数 |
| `e2e` | 端到端测试 | 低 |
| `agent` | 查看 agent 定义 | 低 |
| `permission` | 权限测试 | 低 |
| `prompt` | 渲染 system prompt | 低 |
| `tui` | TUI 回放 | 低 |
| `task` | 任务管理 | 中 |

**关键问题**:
- 4 处重复的 API key 掩码逻辑（UTF-8 边界 panic 风险）
- 测试覆盖不均：`e2e` 和 `permission` 有断言，其余多为空冒烟测试
- `prompt.rs` 使用任意第一个 provider 而非配置的 provider

---

## 2. `core/` — 基础设施层

**14 个文件**，是整个项目的基础。依赖关系为 DAG 结构，无循环依赖。

| 文件 | 行数 | 测试 | 核心功能 |
|------|------|------|----------|
| `config.rs` | 1,163 | 13 | 配置加载/合并/迁移，7 个内置 provider 目录 |
| `session.rs` | 852 | 7 | sled 嵌入式持久化（4 棵树） |
| `permission.rs` | 690 | 37 | 两层权限模型：危险命令检测 + 路径范围检查 |
| `agent.rs` | 683 | 9 | 9 个内置 agent 定义 + frontmatter 解析 |
| `memory.rs` | 464 | 11 | 行导向 `.md` memory 存储（3 个 scope） |
| `models_dev.rs` | 551 | 8 | models.dev 动态目录（5 分钟 TTL 缓存） |
| `provider.rs` | 326 | 0 | LlmProvider trait + 消息类型 + 工厂 |
| `paths.rs` | 222 | 8 | 路径保护策略 |
| `vault.rs` | 198 | 0 | 加密凭据存储 |
| `platform.rs` | 136 | 2 | 跨平台路径检测 |
| `crypto.rs` | 119 | 3 | AES-256-GCM 加密 |
| `compaction.rs` | 226 | 7 | 上下文压缩策略 |
| `token.rs` | 45 | 4 | token 估算（3.5 chars/token） |
| `mod.rs` | 13 | 0 | 模块声明 |

**关键问题**:
- **`config.rs`** 是最大文件（1,163 行），配置数据/IO/合并/解析混在一起
- **`Config::merge` bug**: `search_engine` 字段从不合并
- **`Config::load` 有副作用**：加载时会写入 vault 并打印到 stdout
- **测试非隔离**：`Config::load` 在测试中读取真实全局配置
- **`vault.rs` 和 `provider.rs` 无测试**
- **`permission.rs` 手写的 shell tokenizer** 无法检测 `$(...)` 和反引号替换

---

## 3. `provider/` — LLM 协议实现

| 文件 | 行数 | 测试 | 协议 |
|------|------|------|------|
| `openai_compat.rs` | 620 | 10 | OpenAI 兼容（DeepSeek, GLM 等） |
| `anthropic.rs` | 429 | 0 | Claude |
| `gemini.rs` | 423 | 0 | Google Gemini |
| `mod.rs` | 259 | 0 | 共享工具：SSE 解析、HTTP 客户端、重试 |

**关键问题**:
- 三个 provider 结构体、构造函数、工厂函数高度重复（~150 行可抽取）
- **`anthropic.rs:234` 有生产代码 `unwrap()`**
- **`gemini.rs:217` 潜在 bug**: `tool_idx` 在 `for await` 循环内声明，跨 chunk 的 functionCall 会碰撞 ID
- **仅 `openai_compat` 支持图片**，其余两个虽声明 `supports_images` 但丢弃多模态内容
- 重试次数硬编码为 1，无用户可配置

---

## 4. `tool/` — 工具系统

**17 个工具** + 权限门控 + 目录注册

| 文件 | 行数 | 测试 | 功能 |
|------|------|------|------|
| `mod.rs` | 381 | 0 | Tool trait, ToolContext, 权限门控 |
| `catalog.rs` | 363 | 11 | 工具注册、预设（read_only/no_write/no_internet） |
| `skill.rs` | 692 | 12 | 技能加载（3 个内置技能） |
| `apply_patch.rs` | 522 | 6 | OpenAI 风格补丁应用 |
| `task.rs` | 423 | 2 | 子 agent 委派（最复杂文件） |
| `bash.rs` | 166 | 3 | Shell 执行 |
| `edit.rs` | 172 | 4 | 文件编辑 |
| `grep.rs` | 214 | 5 | 内容搜索 |
| `read.rs` | 315 | 8 | 文件/目录读取 |
| `webfetch.rs` | 329 | 7 | URL 抓取（SSRF 加固） |
| `websearch.rs` | 299 | 5 | 多引擎搜索（Bing/DDG 回退） |
| `todowrite.rs` | 356 | 7 | 会话 TODO 状态机 |
| `question.rs` | 163 | 2 | 交互式问答 |
| `write.rs` | 169 | 5 | 文件写入 |
| `rm.rs` | 180 | 6 | 文件删除 |
| `undo.rs` | 132 | 3 | 撤销快照存储 |
| `undo_edit.rs` | 124 | 2 | 撤销编辑 |
| `memory_read.rs` | 143 | 0 | 读取 memory |
| `memory_record.rs` | 120 | 0 | 写入 memory |
| `shell.rs` | 131 | 3 | Shell 环境检测 |
| `glob.rs` | 99 | 2 | 文件匹配 |

**关键问题**:
- **`read.rs` bug**: 图片分支不可达（`is_binary_path` 检查在 `is_image_path` 之前）
- **`task.rs:221` 有生产代码 `expect()`**
- **`mod.rs` 和 `shell.rs` 有死代码**
- **`undo_edit.rs` 结构体字段与 `ctx.undo_store` 不一致**
- **目录删除无撤销快照**（文件有，目录没有）

---

## 5. `tui/` — 终端 UI

**36 个文件**，最大模块。

| 文件 | 行数 | 核心功能 |
|------|------|----------|
| `mod.rs` | 877 | 事件循环 + SessionView（~45 字段的 god struct） |
| `dialogs.rs` | 992 | 所有对话框 + provider 配置向导 |
| `session_ops.rs` | 743 | 会话生命周期 + 后台 LLM 任务 |
| `worker.rs` | 613 | Agent 循环（工具执行、重试、流式） |
| `markdown.rs` | 580 | GFM Markdown 渲染 |
| `interaction.rs` | 545 | 鼠标/剪贴板 + 渲染缓存管道 |
| `latex.rs` | 497 | LaTeX -> Unicode 转换 |
| `theme.rs` | 507 | 主题系统（3 个内置主题） |
| `types.rs` | 460 | 共享状态类型 |
| `prompt_flow.rs` | 435 | Prompt 提交路径 |
| `provider_ops.rs` | 371 | Provider/模型切换 |
| `sidebar.rs` | 191 | 文件树 + 文件监视 |
| `html.rs` | 326 | HTML 内嵌解析 |
| `dialog.rs` | 240 | 对话框模型 + slash 命令注册 |
| `pending.rs` | 239 | 权限/问答模态处理 |
| `key_route.rs` | 208 | 键盘路由 |
| `view.rs` | 214 | z-order 图层系统 |
| `layout.rs` | 158 | 布局计算 |
| `util.rs` | 140 | 工具函数 |
| `persist.rs` | 99 | 消息持久化 |
| `input.rs` | 91 | 输入处理 |
| `render.rs` | 110 | 显示渲染入口 |
| `highlight.rs` | 131 | 语法高亮（syntect） |
| `diff.rs` | 65 | Diff 渲染 |
| `session_render.rs` | 64 | 帧渲染 |
| `widgets.rs` | 138 | 状态栏/信息栏 |
| `templates.rs` | 17 | /init 模板 |
| `components/` | 8 files | UI 组件（modal/home/input/session/sidebar/slash_help/status/toast） |

**关键问题**:
- **Reducer 重复**: `pump_prompt_job` (mod.rs) 和 `pump_prompt_job_for_stdout` (prompt_flow.rs) 是 ~200 行的重复代码
- **`verify_provider` 在 UI 线程执行网络请求** -> 冻结 TUI
- **文件监视器不过滤 target/node_modules** -> 任何 fs 事件都触发全量重扫描
- **每帧 3 次 SQLite 查询**（`list_tasks`/`task_count`）@30fps
- **5 个生产代码 `unwrap()`** 在 dialogs.rs
- **`SessionView` 是 god struct**：~45 字段，impl 分散在 10 个文件
- **slash 命令注册处有两份**（`slash_options` vs `parse_slash_command`）会漂移

---

## 6. 跨模块问题汇总

| 严重级别 | 问题 | 位置 |
|----------|------|------|
| **Bug** | `Config::merge` 不合并 `search_engine` | `core/config.rs` |
| **Bug** | `read.rs` 图片分支不可达 | `tool/read.rs` |
| **Bug** | `gemini.rs` tool_idx 在循环内重置 | `provider/gemini.rs:217` |
| **Bug** | `cli/debug/prompt.rs` 使用任意 provider | `cli/debug/prompt.rs` |
| **高危** | `session delete-all` 无确认 | `cli/debug/session.rs` |
| **高危** | `config set` 非原子写入 | `cli/debug/config.rs` |
| **中危** | `verify_provider` 冻结 UI | `tui/provider_ops.rs` |
| **中危** | 文件监视器性能问题 | `tui/sidebar.rs` |
| **中危** | 权限 tokenizer 无法检测命令替换 | `core/permission.rs` |
| **重复** | 4 处 API key 掩码 | `cli/` 多处 |
| **重复** | reducer 两份拷贝 | `tui/mod.rs` + `prompt_flow.rs` |
| **重复** | 3 个 provider 结构体重复 | `provider/` |
| **死代码** | `is_within_project`, `opt_u64`, `shell_binary_path` | `tool/` |
| **风格** | 6+ 处生产代码 `unwrap()/expect()` | 多处 |

---

## 7. 测试覆盖分布

| 模块 | 测试数 | 覆盖评价 |
|------|--------|----------|
| `core/` | 106 | 良好（permission 37, config 13, memory 11） |
| `tool/` | 76 | 良好（catalog 11, skill 12, read 8） |
| `tui/` | 55 | 不足（纯函数覆盖好，worker/dialogs/provider_ops 无测试） |
| `provider/` | 10 | 不足（仅 openai_compat 有测试） |
| `cli/` | 13 | 不足（多为空冒烟测试） |
| `system_prompt.rs` | 9 | 良好 |

**总计**: 269 测试，303/304 通过

---

## 8. 依赖关系图

```
main.rs
  ├── cli/ (debug 命令)
  │     ├── core::config, core::vault, core::platform
  │     ├── core::session, core::agent, core::permission
  │     ├── core::provider
  │     ├── tool/ (catalog, create_tool, run_agent)
  │     └── system_prompt
  │
  ├── tui/ (主界面)
  │     ├── core::config, core::provider, core::session, core::agent
  │     ├── core::vault, core::models_dev, core::platform, core::compaction
  │     ├── tool/ (catalog, run_tool, AskRequest, PermissionRequest, ToolContext, UndoStore, task::run_agent, todowrite)
  │     └── system_prompt
  │
  └── system_prompt
        ├── core::config, core::platform
        └── tool::shell, tool::skill

core/ (无外部依赖)
  ├── platform -> paths, crypto, config, memory, session, agent, models_dev, vault
  ├── crypto -> vault
  ├── vault -> config (一个循环边)
  ├── token -> compaction
  ├── paths -> permission
  ├── config -> provider (ResolvedProvider), models_dev
  └── provider -> compaction, token, session

provider/ (依赖 core)
  └── core::config, core::provider

tool/ (依赖 core)
  └── core::paths, core::permission, core::session, core::provider, core::agent, core::memory, core::platform, core::config
```
