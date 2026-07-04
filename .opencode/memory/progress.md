# Project Progress

> 最后更新：2026-06-26

## 已完成

### 基础设施
- 插件系统废弃，全部源码编译；`~/.config/opencode/` 清理 97MB → 340K
- 剔除非 TUI 包（app, desktop, slack, stats 等）
- 项目级 `.opencode/` 配置纳入源码仓库，AGENTS.md 新增部署运维指南
- Build 脚本修复：`packages/app` 缺失时自动跳过 Web UI 构建

### System Prompt 模板化
- 8 个语义 section（`<constraint>` → `<nudge>`），优先级 P0–P7
- 完整渲染管线：`template.ts` → `role.ts` → `prompt.ts` → `request.ts`
- Role 注入：从 `~/.config/opencode/plugin-config/runtime-orchestrator/` 加载

### Permission Scope 系统
- `Rule.scope`（glob）+ `$PROJECT` 动态 token + `others` fallback
- 6 个工具全适配（bash 特殊：从 shell parser `scan.dirs` 提取文件路径）
- `external_directory` 保留独立配置

### TUI
- 状态栏新增 cache hit rate
- 颜色编码：context%（绿<25%/黄25-50%/红>50%）、cache rate（红<90%/黄90-95%/绿>95%）
- TDZ crash 修复（useTheme 移到 memo 之前）

### Memory V2
- 设计定稿：项目 SQLite + 全局 .md 分流存储
- 4 个核心工具：`memory_review`、`memory_record`、`dreaming_compress`、`todowrite`
- Daemon fiber 后台提取骨架完成，`memory_record` 已支持 markdown 同步

### 内置 Skill
- `customize-opencode`：opencode 自身配置参考
- `write-skills`：SKILL.md 格式、YAML 引号规则、常见陷阱

### Init
- `/init` 集成 project-onboarding skill + scaffold 感知
- Template 重写为 action-oriented

### 诊断填充
- Skill 加载静默失败修复（YAML `:` 无引号 → gray-matter 解析失败 + 防御性日志）
- 确认 Context Epoch 跨进程重启复用机制

### Edit Undo Phase 2
- inline `undo` 参数回归（edit.ts / write.ts → `undo?: boolean`，默认 true）
- 连续多步撤回链（undo_edit 返回 redoHash 作为新 undoHash → undo → undo → undo）
- undo-blobs GC（每 10 次保存扫描清理 >24h 的 blob）
- 测试：4 个 undo 用例（参数开关、还原、链式撤回），edit 33 用例全通过

### Rust TUI 重写 — 设计 & Phase 0
- TUI 架构分析：147 文件、~27K 行、@opentui/solid (闭源)、REST+SSE 通信
- 分支 `opencode-rust-tui` 已创建
- 设计文档 `doc/rust-tui-design.md` v3 定稿（13 章节、~800 行、12 个关键决策）
- Rust 工具链 1.96.0 已安装
- `crates/openrust/` cargo init，Cargo.toml 含所有依赖
- clap CLI，Config loader (JSONC)，LlmProvider trait + OpenAI-compat SSE streaming
- Provider 实测：gpt-5.5 / gpt-5.4 均通过（one_route proxy）

### Rust TUI 重写 — Phase 0.9
- API key 加密：AES-256-GCM + machine-id 绑定密钥
- Vault store：`~/.config/openrust/credentials.enc` (chmod 600)
- 自动迁移：config.json 明文 key → vault，原文删除
- 分辨率链：vault → `{NAME}_API_KEY` env → `OPENAI_API_KEY` env

### Rust TUI 重写 — Phase 1A
- 10 个工具：read, write, edit, rm, bash, glob, grep, webfetch, websearch, undo_edit
- Tool trait + ToolParams helper + 宏（try_tool!/require_str!/try_opt!）
- Tool::to_llm_def() → OpenAI function calling 格式
- Tool::execute_checked()：统一 permission 门控
- UndoStore：blob 存储 + 24h GC + 链式撤回
- 测试：48/48 passing，0 compiler warnings

### Rust TUI 重写 — Phase 1A.1
- 二进制改名：opencode → openrust
- 配置域：`~/.config/openrust/`（与 TS opencode 完全隔离）
- User-agent：openrust/0.0

### Rust TUI 重写 — Phase 1A.2
- rm 工具：安全删除（recursive/force），refuses system paths
- `core/paths.rs`：Linux 统一路径保护（SYSTEM_PROTECTED / USER_PROTECTED / PROJECT_PROTECTED）
- Project scope 管控：is_within_project → 所有破坏性工具默认限于 cwd
- Permission::Ask 变体（debug 模式自动放行+标记，TUI 模式弹出确认框）
- read 工具 description 明确替代 cat

### Rust TUI 重写 — Phase 1B / 1B.1 / 1B.2 / 1B.3
- Session / Message / Transcript 持久化完成，固定前缀区与动态历史区分离
- system prompt 分段渲染完成：constraint / identity / environment / instructions / capabilities / style / memory / nudge
- 包级结构重组：`src/lib.rs` 作为库入口，`src/main.rs` 保留薄二进制入口
- `STRUCTURE.md` 记录当前实现索引表
- 平台路径模块 `core::platform` 完成：Windows 10 / Windows 11 / Fedora 44 支持，且细分为 `config / data / cache`
- config、vault、session、undo、prompt 的路径来源统一收口到平台模块
- `cargo test` 已验证通过，当前 58 passed

### Rust TUI 重写 — Phase 1B.4 / 1B.5
- Shell 工具支持按环境选择 `pwsh` / `powershell` / `cmd` / `bash`，prompt 文案和执行器共用同一 shell 判定
- 工具抽象轻量化：新增 `tool::catalog`，统一静态元数据、分类和 prompt hint
- `cli::debug::tool` 改为从 catalog 输出工具现状与分组索引
- `core::system_prompt` 的 capabilities 段改为共享 catalog prompt hints
- 工具 catalog 文档 `TOOL_CATALOG.md` 已补充
- `cargo test` 已验证通过，当前 64 passed

### Rust TUI 重写 — Phase 1C
- `openrust` 默认进入 TUI；支持 `--prompt` 与 `--script` 预动作
- 新增 `cli::debug::tui replay`，用于 bash 观察脚本回放过程
- TUI 最小闭环已能连接模型、显示流式输出、顺序执行脚本输入
- KMP 脚本回放已验证通过：非 TTY 自动降级 headless，DeepSeek `deepseek-v4-pro` 返回符合脚本要求的 KMP 实现输出
- 运行日志已切到 `stderr +` 文件双写，文件落在 `~/.local/share/openrust/log/openrust.log`
- TUI 结构继续拆分为 `input / dialog / render / worker` 四层，`cargo test` 已恢复通过（75 passed, 0 failed），模块边界进入稳定收尾阶段

### Rust TUI 重写 — 配置语义收口
- 项目 `openrust.json` 覆盖全局 `config.json`；同名 provider 直接整块覆盖
- API key 只走 vault + 配置文件，不再依赖环境变量
- `debug config show` / `debug provider test` 已同步新语义
- 模型配置取消隐式兜底；缺少 provider/model 时显式失败，wire model 从 `deepseek/deepseek-v4-pro` 解析为 `deepseek-v4-pro`
- 已提交 `b6932af6a feat(rust): add tui replay flow` 与 `1eb9692de docs(openrust): document tui replay setup`；本地 `crates/openrust/openrust.json` 已忽略，避免提交明文 key

## 进行中
- 继续核对 standalone TUI 的真实运行日志，确认问题来源不再混淆捕获输出与实际 app log

## 下一步
- Phase 1D：工具-TUI 集成（question/todowrite/skill/task + permission 对话框）

## 待修复（预存问题）
- core: DatabaseMigration 超时、LocationServiceMap 隔离、Npm.add 超时
- opencode: permission.task 实时配置加载超时
- typecheck: server.ts + prompt.test.ts 类型错误（Effect Layer 推断）
