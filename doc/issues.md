# openrust 问题追踪清单

## Bug

### BUG-001: `Config::merge` 不合并 `search_engine` 字段

- **位置**: `crates/openrust/src/core/config.rs`
- **严重程度**: 高
- **描述**: `search_engine` 是 `Config` 的字段，但 `merge()` 函数中从未处理它。项目级配置中的 `search_engine` 不会覆盖全局配置。
- **影响**: 用户无法通过项目级 `.openrust/config.jsonc` 覆盖搜索引擎设置
- **修复建议**: 在 `Config::merge()` 中添加 `search_engine` 的合并逻辑

---

### BUG-002: `read.rs` 图片分支不可达

- **位置**: `crates/openrust/src/tool/read.rs:67-74`
- **严重程度**: 高
- **描述**: `is_binary_path()` 的检查在 `is_image_path()` 之前执行，而所有图片扩展名（png/jpg/gif/webp/bmp/ico）都包含在 `BINARY_EXTENSIONS` 中。因此图片文件在到达 `read_image()` 之前就被拒绝了。
- **影响**: `read_image()` 函数对列出的图片扩展名永远不可达，只有 `svg`（在 IMAGE 但不在 BINARY 中）能到达
- **修复建议**: 交换两个检查的顺序，或从 `BINARY_EXTENSIONS` 中移除图片扩展名

---

### BUG-003: `gemini.rs` tool_idx 在循环内重置

- **位置**: `crates/openrust/src/provider/gemini.rs:217`
- **严重程度**: 高
- **描述**: `let mut tool_idx: usize = 0;` 声明在 `for await` 循环内部，每个 SSE 事件都会重置为 0。如果一个 functionCall 跨两个 chunk 到达，两个 chunk 都会得到 id `tool_0`。
- **影响**: 跨 chunk 的 functionCall 会产生 ID 碰撞（目前 Gemini 实际发送方式下不太可能触发，但属于潜在 bug）
- **修复建议**: 将 `tool_idx` 移到循环外部

---

### BUG-004: `cli/debug/prompt.rs` 使用任意 provider

- **位置**: `crates/openrust/src/cli/debug/prompt.rs`
- **严重程度**: 中
- **描述**: `config.provider.keys().next()` 选择 HashMap 迭代顺序中的第一个 provider，而非与 `config.model` 匹配的 provider。渲染的 prompt 可能使用错误的 provider。
- **影响**: debug prompt 渲染结果与实际运行不一致
- **修复建议**: 使用 `resolve_provider_model()` 获取正确的 provider

---

## 高危问题

### SEC-001: `session delete-all` 无确认提示

- **位置**: `crates/openrust/src/cli/debug/session.rs`
- **严重程度**: 高
- **描述**: `DeleteAll` 命令直接删除所有会话，没有任何确认提示。注释中标注了 "irreversible" 但代码没有实现确认机制。
- **影响**: 用户可能误删所有会话数据
- **修复建议**: 添加交互式确认或 `--force` 标志

---

### SEC-002: `config set` 非原子写入

- **位置**: `crates/openrust/src/cli/debug/config.rs:54,128-129`
- **严重程度**: 高
- **描述**: 读取配置 -> 修改 -> 写入，没有临时文件 + 重命名机制。崩溃中途会损坏全局配置。同时，格式错误的全局配置会被静默覆盖（`unwrap_or(json!({}))`）。
- **影响**: 用户全局配置文件可能损坏或丢失
- **修复建议**: 1) 写入时使用临时文件 + 原子重命名 2) 格式错误时报错而非静默覆盖

---

### SEC-003: API key 暴露在命令行参数

- **位置**: `crates/openrust/src/cli/debug/vault.rs`, `crates/openrust/src/cli/debug/config.rs`
- **严重程度**: 中
- **描述**: `--api-key` 参数将明文密钥暴露在 shell 历史和进程列表中。
- **影响**: API key 可能泄露
- **修复建议**: 支持从 stdin 读取或交互式输入

---

### SEC-004: 权限 tokenizer 无法检测命令替换

- **位置**: `crates/openrust/src/core/permission.rs`
- **严重程度**: 高
- **描述**: 手写的 shell tokenizer 不处理 `$(...)`、反引号替换、子 shell、变量间接引用。例如 `bash -c "rm -rf /"` 会被允许。
- **影响**: 恶意命令可通过命令替换绕过权限检查
- **修复建议**: 文档化此限制，或集成更完整的 shell 解析器

---

## 性能问题

### PERF-001: `verify_provider` 在 UI 线程执行网络请求

- **位置**: `crates/openrust/src/tui/provider_ops.rs`
- **严重程度**: 中
- **描述**: 创建新的 tokio Runtime 并在 UI 线程上 `block_on` 网络请求，导致 TUI 冻结。
- **影响**: 用户在切换 provider 时界面冻结
- **修复建议**: 将验证请求移到后台线程

---

### PERF-002: 文件监视器不过滤 target/node_modules

- **位置**: `crates/openrust/src/tui/sidebar.rs`
- **严重程度**: 中
- **描述**: notify watcher 递归监视整个项目根目录，包括 `target/` 和 `node_modules/`。虽然显示时过滤了这些目录，但 watcher 事件仍会触发全量重扫描。
- **影响**: 构建过程中 TUI 频繁重扫描文件树，CPU 占用高
- **修复建议**: 在 watcher 事件处理中过滤被跳过目录的事件

---

### PERF-003: 每帧 3 次 SQLite 查询

- **位置**: `crates/openrust/src/tui/widgets.rs`, `crates/openrust/src/tui/components/sidebar.rs`, `crates/openrust/src/tui/view.rs`
- **严重程度**: 中
- **描述**: `status_line`、`render_todo`、`build_session_base_layers` 每帧都调用 `store.list_tasks` 或 `task_count`，在 AI 运行时 @30fps 产生 90 次/秒 的 SQLite 查询。
- **影响**: 不必要的 CPU 和 IO 开销
- **修复建议**: 缓存任务计数，仅在任务变更时刷新

---

## 代码重复

### DUP-001: 4 处 API key 掩码逻辑

- **位置**: `crates/openrust/src/cli/debug/config.rs:111,150,155`, `provider.rs:51`, `vault.rs:54`
- **严重程度**: 低
- **描述**: `format!("****{}", &k[k.len()-4..])` 在 4 处重复，且存在 UTF-8 边界 panic 风险。
- **修复建议**: 提取 `mask_secret(&str)` 辅助函数到 core

---

### DUP-002: Reducer 两份拷贝

- **位置**: `crates/openrust/src/tui/mod.rs` (pump_prompt_job) + `crates/openrust/src/tui/prompt_flow.rs` (pump_prompt_job_for_stdout)
- **严重程度**: 高
- **描述**: 两个 ~200 行的函数处理相同的 worker 事件，但存在细微差异（如 Aborted 处理不同）。bug 修复需要改两处。
- **修复建议**: 提取共享的 reducer 逻辑

---

### DUP-003: 3 个 provider 结构体重复

- **位置**: `crates/openrust/src/provider/anthropic.rs`, `gemini.rs`, `openai_compat.rs`
- **严重程度**: 低
- **描述**: 三个 provider 的结构体、构造函数、工厂函数、list_models/supports_images/name 实现高度重复（~150 行）。
- **修复建议**: 提取共享的 `ProviderBase` 结构体或宏

---

### DUP-004: slash 命令注册两份

- **位置**: `crates/openrust/src/tui/dialog.rs` (slash_options) + `crates/openrust/src/tui/input.rs` (parse_slash_command)
- **严重程度**: 中
- **描述**: 两个独立的 slash 命令注册处，容易漂移。例如 `/think`, `/model`, `/tree`, `/compaction` 能解析但不出现在帮助中。
- **修复建议**: 统一到一个注册处

---

### DUP-005: 工具参数摘要器两份

- **位置**: `crates/openrust/src/tui/types.rs` (tool_display_input) + `crates/openrust/src/tui/persist.rs` (tool_display_preview)
- **严重程度**: 低
- **描述**: 两个函数以不同方式总结工具参数，键名不一致（`file_path` vs `filePath`）。
- **修复建议**: 统一到一个函数

---

## 死代码

### DEAD-001: `tool/mod.rs` 中未使用的函数

- **位置**: `crates/openrust/src/tool/mod.rs:250`
- **描述**: `is_within_project()` 没有外部调用者（权限流程直接使用 `core::paths::is_within_project`）

---

### DEAD-002: `tool/mod.rs` 中未使用的 `opt_u64`

- **位置**: `crates/openrust/src/tool/mod.rs`
- **描述**: `ToolParams::opt_u64()` 没有调用者（只有 `u64_or` 被使用）

---

### DEAD-003: `tool/shell.rs` 中未使用的 `shell_binary_path`

- **位置**: `crates/openrust/src/tool/shell.rs`
- **描述**: `shell_binary_path()` 没有调用者

---

### DEAD-004: `tui/interaction.rs` 中的死代码

- **位置**: `crates/openrust/src/tui/interaction.rs:160-162`
- **描述**: `if self.is_tool_row(index) { let _ = column; }` 什么都不做

---

## 代码风格违规

### STYLE-001: 生产代码中的 `unwrap()` / `expect()`

| 位置 | 代码 | 风险 |
|------|------|------|
| `provider/anthropic.rs:234` | `blocks.get(&index).unwrap()` | 可能 panic |
| `tool/task.rs:221` | `stream.expect("retry loop sets stream or returns")` | 逻辑上安全但违反规范 |
| `tui/worker.rs` | `stream.expect(...)` | 逻辑上安全但违反规范 |
| `tui/dialogs.rs` (5处) | `draft.editing_index.unwrap()` | 状态不同步时 panic |
| `tui/session_ops.rs` | `.expect("failed to create background tokio runtime")` | 启动失败时 panic |
| `core/memory.rs` | `.expect("memory WRITE_LOCK poisoned")` | 锁中毒后永久失败 |

---

### STYLE-002: 文档与实际不符

| 位置 | 问题 |
|------|------|
| `core/provider.rs` | `detect_protocol` 文档说 "then base_url heuristic" 但没有实现 |
| `core/session.rs` | 注释说 "Phase 1B" 已过时 |
| `system_prompt.rs` | 注释说 "Phase 1B" 已过时 |
| `main.rs` | `#![allow(dead_code)]` 标注 "Phase 0" 应移除 |
| AGENTS.md | 提到 `session_input` 模块不存在 |

---

## 测试不足

### TEST-001: `core/vault.rs` 无测试

- **位置**: `crates/openrust/src/core/vault.rs`
- **描述**: 加密凭据存储没有单元测试，依赖 crypto 测试和 cli debug 路径

---

### TEST-002: `core/provider.rs` 无测试

- **位置**: `crates/openrust/src/core/provider.rs`
- **描述**: LlmProvider trait 和消息类型没有单元测试

---

### TEST-003: `tui/worker.rs` 无测试

- **位置**: `crates/openrust/src/tui/worker.rs`
- **描述**: Agent 循环（最复杂的控制流）没有单元测试

---

### TEST-004: `tui/dialogs.rs` 无测试

- **位置**: `crates/openrust/src/tui/dialogs.rs`
- **描述**: ~1000 行的对话框和向导逻辑没有单元测试

---

### TEST-005: `provider/anthropic.rs` 和 `gemini.rs` 无测试

- **位置**: `crates/openrust/src/provider/anthropic.rs`, `gemini.rs`
- **描述**: 两个 provider 实现没有单元测试，特别是 `lower_messages` 转换逻辑

---

### TEST-006: `cli/` 模块测试覆盖不均

- **位置**: `crates/openrust/src/cli/debug/`
- **描述**: 只有 `e2e.rs` 和 `permission.rs` 有真正的断言测试；`agent.rs`, `session.rs`, `task.rs` 是空冒烟测试；`prompt.rs`, `provider.rs`, `tool.rs`, `vault.rs` 完全无测试

---

## 设计问题

### DESIGN-001: `SessionView` 是 god struct

- **位置**: `crates/openrust/src/tui/mod.rs`
- **严重程度**: 中
- **描述**: ~45 个字段，impl 分散在 10 个文件中。高认知负荷，任何修改都涉及这个类型。
- **建议**: 考虑拆分为多个子结构体（RenderState, DialogState, SessionState 等）

---

### DESIGN-002: `tool/task.rs` 子 agent 无 undo store

- **位置**: `crates/openrust/src/tool/task.rs`
- **严重程度**: 低
- **描述**: 子 agent 通过 `catalog::create_tool(meta.name, None)` 创建工具，undo_edit 在子 agent 中永远失败。
- **建议**: 传递 undo_store 到子 agent 或文档化此限制

---

### DESIGN-003: `rm.rs` 目录删除无撤销快照

- **位置**: `crates/openrust/src/tool/rm.rs`
- **严重程度**: 低
- **描述**: 文件删除前有 undo 快照，但递归目录删除没有。
- **建议**: 为目录删除添加快照或文档化此限制

---

### DESIGN-004: `Config::load` 有副作用

- **位置**: `crates/openrust/src/core/config.rs`
- **严重程度**: 中
- **描述**: 加载配置时会写入 vault 并打印到 stdout（带 emoji）。库层代码不应有 IO 副作用。
- **建议**: 将迁移逻辑分离为显式的 `migrate()` 函数

---

### DESIGN-005: 测试非隔离

- **位置**: `crates/openrust/src/core/config.rs`
- **严重程度**: 中
- **描述**: `Config::load` 在测试中读取真实全局配置路径（`~/.config/openrust/config.json`），测试结果依赖于开发机器的实际配置状态。
- **建议**: 使全局配置路径可注入

---

## 追踪状态

| 状态 | 数量 |
|------|------|
| Bug | 4 |
| 高危问题 | 4 |
| 性能问题 | 3 |
| 代码重复 | 5 |
| 死代码 | 4 |
| 代码风格违规 | 2 |
| 测试不足 | 6 |
| 设计问题 | 5 |
| **总计** | **33** |
