# Project Progress

> 最后更新：2026-07-11

## Rust 重写里程碑（openrust）
- Phase 0 / 0.9 / 1A–1D / 1 收尾 / 2 全部完成：CLI + Provider + 14 工具 + 会话/权限/system prompt + 最小 TUI + 真实 tool loop + task 子 agent + 权限弹窗 + 富渲染（markdown/syntect/diff/sidebar）
- `cargo test` 274 passed，零 warning；分支 `opencode-rust-tui` 已推送
- 路线图已重估：Phase 3 改为主线能力补齐 + 交互层打磨，Phase 4 改为 TS 退场门槛定义 + 迁移收口，新增 Phase 5 用于对齐 Phase 1 / 2 与最新 `dev-ai-release`

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

### Rust TUI 重写 — Phase 1D + 工具对齐
- TUI：`Tab` 循环切换 agent（`cycle_agent()`）、`/compact` 触发压缩、鼠标滚轮 + PageUp/PageDown 历史滚动
- Compaction：append-only checkpoint 语义（`summary`/`recent` 字段），`replace_messages`/`append_compaction`/`effective_messages` 处理边界
- 真实 tool calling loop 落在 `worker.rs`：模型发 tool call → worker 本地执行 → `role=tool` 结果回灌 → 模型续写（多轮由 worker 内部管理，UI 只收事件）
- Provider `Message` 扩展 `tool_calls`/`name`/`tool_call_id`（OpenAI 兼容），`openai_compat` 序列化进请求体；工具结果带完整元数据持久化到 session 便于 replay
- 工具对齐 opencode：`resolve_path()` 统一相对路径解析、read/write/edit 支持 `path`/`filePath` 别名、webfetch `format` 参数、websearch 描述更新
- 新增 `apply_patch` 工具：opencode 补丁格式（Add/Update/Delete，move 显式拒绝），顺序应用 + 部分失败报告，exact/rstrip/trim 回退匹配；已接入 catalog/registry/scope
- 新增 `todowrite`：全量替换语义持久化 session todo 列表（`SessionStore::replace_tasks`，`Task` 加 `priority`）
- 新增 `skill`：按名扫描 `.opencode/skills`/`skills`/配置目录的 `<name>/SKILL.md`，剥离 frontmatter 返回正文
- 新增 `question`：原生 TUI 选择弹窗，worker↔UI 双向 `AskRequest` 通道，单选/多选/自定义输入，headless 下降级报错不阻塞
- 扩展 `ToolContext`（`session_id`/`store`/`ask_tx`），新增 Interaction 工具类别
- worker 发给模型的 tool 定义改用各工具真实 `parameters()` schema，替换空占位
- `cargo test` 107 passed（从 64 → 100 → 107）
- 已推送到 `origin/opencode-rust-tui`：feat(agent/task flow + tool loop + alignment)、docs、feat(apply_patch)、feat(todowrite/skill/question)、fix(real tool schemas)

### Rust TUI 重写 — Phase 1 收尾（缺口补完）
- `task` 子 agent：抽出可复用 `run_agent()` headless 循环（无 UI 依赖），用指定 agent 的 system prompt 跑完整工具循环返回结果；`ToolContext` 注入 `llm`/`model`/`reasoning_effort`；子 agent 上下文清空 llm/交互通道防递归；`run_agent` 排除 task/question 工具
- 真实权限弹窗：worker 执行 scope 受限工具前发 `PermissionRequest`，TUI 弹 `[A]llow/[D]eny`（A/Y·D/N/Esc·←→·Enter）确认后才执行；`ToolContext` 加 `permission_tx`；无通道时默认拒绝（更安全，移除旧的 auto-allow）
- `core/permission.rs`：正规化 scope 判定 + `$PROJECT`/`${PROJECT}` 展开，`tool/mod.rs` 委派；共享 `crate::tool::run_tool`（worker 与 run_agent 复用）
- 新增 debug 命令：`permission check <tool> <path>`（allow/deny/ask）、`e2e run "<prompt>"`（复用 run_agent 跑完整循环）
- 14 工具齐全（补 task）；`cargo test` 113 passed；零 warning

### Rust TUI 重写 — Phase 2（富渲染）
- 技术选型（用户确认）：语法高亮用 `syntect`（纯 Rust，fancy-regex，无 C 编译）而非 tree-sitter；Markdown 自研 `pulldown-cmark → ratatui`；Diff 用 `similar` last-turn（不接 git）
- `tui/markdown.rs`：pulldown-cmark 解析 → 标题/粗斜体/行内代码/围栏代码块/有序无序列表/引用/分隔线 → ratatui Line/Span；assistant 消息经 markdown 渲染
- `tui/highlight.rs`：syntect 代码块着色，语言 token/扩展名/名称三级匹配，未知回退纯文本；主题 base16-ocean.dark
- `tui/diff.rs`：`render_diff`（着色）+ `unified_diff`（文本），edit/write 完成后 session 内联显示统一 diff，`/diff` 弹出彩色 diff overlay
- `tui/sidebar.rs`：walkdir 深度受限文件树（跳过 .git/target/node_modules 等）+ notify 实时刷新（`poll_refresh`）；`/files` 切换左侧栏
- 新增 slash 命令 `/files`(`/tree`)、`/diff`；Theme 加 diff/sidebar 样式
- `cargo test` 126 passed（113 → 126）；零 warning

### Rust TUI 重写 — Phase 3 交互层增强（ESC 中断 / 跟进消息 / 工具调用动画）
- **ESC 中断**：`PromptEvent::Aborted` + `abort: Arc<AtomicBool>`；ESC 设置中断标志；pump 处理 Aborted（保存 assistant_preview，清除 pending_prompts，恢复就绪状态）；每回合重置 abort，与关闭标志分离
- **实时跟进消息**：`PromptJob.followup_tx` 通道；运行时用户输入立即显示 + 持久化 + 推送到 self.messages + 发送给 worker；worker 每步注入历史，随工具调用结果发送
- **渐进式工具调用显示**：`ToolCall` 拆分为 `ToolCallStart{id,name}` + `ToolRunning{id}`；`pending_tool_calls: Vec<PendingTool>`（ToolState Created/Running）；session_render 中 spinner 渲染（盲文帧 100ms/帧）
- **30fps 动画修复**：统一 `poll_timeout=33ms`，poll 超时时 `needs_render || ai_running` 强制重绘（此前 spinner 在工具执行期间卡死，因无事件 = 无重绘）
- **Clippy 全面清理**（8 个预存 + auto-fix）：&PathBuf→&Path, vec!→array, match→let, needless_range_loop, unwrap→if let, 添加 too_many_arguments
- `cargo test` 192 passed，零 warning

### 后台 Agent + 记忆系统设计
- Skills/rules/agents 审查修复完成并提交（commit `93e6b44db`，已 rebase 到远程之上）：create-agent 技能 mode→tools 文档修正、read_only 预设补排除 undo_edit、AGENTS.md agents 加载路径去 legacy 目录、skill.rs split_frontmatter 去重、agent.rs body offset CRLF 安全、task.rs 移除 unreachable fallback
- 设计文档 `doc/background-agent-design.md` 创建（~730 行），多轮修订后提交（commit `c2beb7947`）：后台 agent 复用 `shared_runtime()` + `run_agent()` 模式、增量提取（水位线 + delta 快照）、`background_model` config 字段（fallback 主 model）
- dev-ai 分支记忆系统调研完成：4 工具（memory_review/memory_record/dreaming_compress/memory_read）+ 3 层存储（project sled+md / user md / dreaming md）+ memory-extract agent
- **设计文档核心决策全部落定**（P0 不再待确认）：Memory 内容不注入 system prompt 正文，只注入静态 `<memory>` 位置索引段（零查询开销）；存储纯 .md 无 DB（无 sled/SQLite），进程内 Mutex 串行化写，无 git 依赖；工具砍到 2 个（砍掉 memory_review，dreaming_compress 改为独立 /dream 操作）；target/name 统一为 category；去重用 trim+lowercase 完整比较；memory-extract agent 只有 2 工具两层提取（project+user）；后台 agent 并发对策：memory 延迟 5s 错峰

### 记忆系统阶段 1 实现（commit `e5c60d0d0`）
- `core/memory.rs`（~300 行）：纯 .md 存储，Scope/Category 枚举，路径解析（含 sha256 dreaming hash），进程内 Mutex 写锁，完整 content 去重，原子写回，日期计算（无 chrono），11 个单元测试全通过
- `tool/memory_record.rs`（~110 行）：写入工具，scope/category 验证
- `tool/memory_read.rs`（~115 行）：读取工具，无参数 → overview，有参数 → 过滤
- catalog 注册 2 个工具 + Memory 分类
- system_prompt.rs 注入硬编码 `<memory>` 索引段

### 记忆系统阶段 2 + clippy/test 修复（commits `f74041fa7` / `5d829fddc`）
- memory-extract builtin agent + `generate_memory()` 增量提取：水位线 + 5s 错峰 + fire-and-forget
- SessionStore 加 `memory_watermarks` named tree（key=session_id, value=u64 BE bytes）
- `resolve_background_provider()` / `resolve_background_provider_model()` 实现
- Config 加 `background_model` 字段，fallback 到主 `model`
- `PromptEvent::Finish` 在 `prompt_flow.rs` 和 `mod.rs` 两处触发 `generate_summary()` + `generate_memory()`
- 12 个 pre-existing clippy warnings 全修 + platform_paths 测试平台感知修复

### Provider 重构 — config-driven + 内置注册表（commits `6c83ec689` / `a690570b8` / `89985662f`）
- `openai_compat.rs` 移除所有 `starts_with` 前缀匹配，加 4 个 ModelConfig 字段（`reasoning_options`(自由 JSON) / `reasoning_send_effort`(bool) / `max_tokens_key`(string) / `system_role`(string)），4 个可选字段带默认值
- `builtin_providers()` 完整定义 deepseek/glm/zhipuai-coding-plan/openai/anthropic/gemini，`Config::load()` 用 `entry().or_insert()` 填充；用户同名 provider 完全替换内置（非逐字段 merge）
- 删除 `OpenAICompatProvider` 的 `provider_defaults()` / `defaults` 字段
- Anthropic reasoning fallback：effort→budget（low=8k, mid=16k, high=32k），reasoning_options 可覆盖
- Gemini reasoning fallback：`thinkingConfig:{includeThoughts:true}`，reasoning_options 可覆盖
- openrust.json 精简：deepseek 只需 api_key（内置提供 base_url/models/reasoning）

### `/dream` 命令（commit `5ad0c800c`）
- `SlashCommand::Dream` 变体 + input.rs 解析
- dreaming builtin agent + system prompt（跨 session 模式分析、先读后写、质量优先）
- `dream()` 方法：list_sessions → summary（无 summary 时 fallback 到 title + 最近 3 条消息）→ 后台 dreaming agent（max_steps=20）提取跨 session 模式 → 写入 `scope=dreaming`
- TUI 显示 "dreaming: analyzing N session(s)..."
- `cargo test` 274 passed，零 warning

### openrust identity 清理 + 引导流 + provider 优化（commits `d65e6bbfa` / `cb33af510` / `b8e997d1b` / `a24ce106d` / `7dda287c5` / `da556be8d` / `d21186a47`）
- identity 清理：移除源码全部 opencode 引用（crypto pepper → `openrust-cred-v1`、skill 路径、测试临时文件、注释）
- AGENTS.md 重写：零 opencode 引用，部署路径 `~/.local/share/openrust/bin/openrust`，含 symlink first-time setup、rules/memory/providers 段
- prompt 前缀稳定性修复：`shell_kind` + `skills` 移到 `SystemPrompt` 构造时缓存，`render()` 变纯函数，保证同一实例渲染永远一致
- `/connect` 引导式 provider switch：选 provider → API key 输入（缺时）→ model 选择 → thinking effort（支持的模型自动链入）；新增 `begin_provider_switch`/`save_switch_api_key`/`open_provider_model_dialog`/`switch_provider_model` + `DialogKind::ProviderModel` + `pending_provider` 状态 + `Dialog` title/description 改 `String`
- zhipuai-coding-plan 模型更新：glm-5.2(1M,send_effort) + glm-5.1(auto-migrate) + glm-5-turbo(200K) + glm-4.7(128K)；base_url 修为 `/api/coding/paas/v4`
- thinking effort 链入修复：选完模型后检测 `reasoning_options`/`reasoning_send_effort`，自动弹 thinking effort 对话框
- 清理 `~/.config/openrust/config.json`：删除所有 provider 定义和 model 字段，让内置定义生效

### 状态栏重构 + 交互层增强（commits `55d6e915d` / `c7f05d8cf` / `8087df113` / `70b0ee7ba` / `885895543` + retry 待 commit）
- **状态栏拆分**：model/thinking/agent 移到输入框下方独立 info 行；status 行保留 context/cache/tasks/running；`StatusBar::render(info_area, status_area)` + `SessionLayout`/`HomeLayout` 新增 `info: Rect`
- **`/connect` 始终弹 API key**：空回车 = 复用已有 key，非空则保存新 key
- **session delete 子命令**：`debug session delete <id>` + `delete-all`
- **Esc 中断扩展到 LLM 调用层**：`tokio::select!` + 500ms 轮询 abort flag，覆盖 worker + run_agent 的 `llm.chat()` 和 `stream.next()`（此前仅中断 UI 层）
- **context 用协议返回值**：`cache.total`（真实 prompt_tokens）替代 `token::estimate_messages` 估算；首回合无 cache 时回退到估算
- **retry 倒计时**（待 commit）：provider 内部 `retry_with_backoff(0)` 只做单次请求，retry 逻辑移到 worker + run_agent 层；指数退避 2/4/8s；`is_retriable_error`（4xx 除 429/408 不重试）；`PromptEvent::RetryStatus` 推送倒计时到状态栏；三个 provider 均改为 `retry_with_backoff(0)`

## 进行中
- **`/init` command-as-prompt**：正在实现
  1. `SlashCommand::Init(String)` + `SlashResult` 枚举（NotHandled/Handled/Prompt）已加到 types.rs
  2. `handle_slash_command` 改返回 `SlashResult`，添加 Init 分支 — 适配中
  3. `init_template()` 函数（从 dev-ai initialize.txt 改编，4 Phase：Check → Investigate → Report → Scaffold）— 待做
  4. system prompt onboarding hint（AGENTS.md/.openrust/ 缺失时注入）— 待做
  5. `/init` 解析 + `enqueue_or_run_prompt` 调用适配 — 待做
- **mode → read_only 迁移**：用户决定彻底删除 `mode` 概念，agent 能力完全由 md 文件定义；工具集控制改由 frontmatter `read_only: true` 布尔实现。8 步计划已定但尚未实现
- Phase 5 已启动：以最新 `dev-ai-release` 为基线审计 Phase 1 / 2 语义差距，首版矩阵见 `doc/openrust-phase5-alignment.md`

## 下一步
- Phase 3：主线能力补齐（不做 `--mini`，改做欢迎界面 / 最近会话 / 配置状态 / session 入口）+ which-key/通知/主题/keymap/偏好
- Phase 4：先定义 TS 退场门槛和迁移验收矩阵，再决定删除 `packages/tui` / `packages/opencode`
- Phase 5：专项审计 Phase 1 / 2 与最新 `dev-ai-release` 的语义差距，决定哪些回补项前置到 Phase 3/4
- Phase 5A：优先补欢迎界面、agent step limits、read/web/skill failure bounds；Phase 5B 再处理 location/event stream、compaction、prompt transform 架构差距
- 欢迎界面重构：Home 从“最近会话/方向键菜单”改为更接近 opencode 的居中 logo + prompt 首屏；普通输入直接进入 prompt，Enter 创建会话并提交，`/connect` / `/models` 在 Home 直接打开配置流，不再先创建 session；Home 不再吞方向键或普通输入；零配置目录持续显示缺 provider/model/key 的可见提示；发送路径缺配置时留在界面内提示，不再直接退出；清理旧菜单式 Home 残留方法；`cargo test` 140 passed
- `/connect` 配置流增强：保留原有命令行形式 `/connect add <provider> <base-url> <model> [wire-model]`，并新增交互式向导入口（`/connect add` 无参数或 Connect 对话框中的 “Add custom provider”）；可逐步输入 provider、base URL、model、wire model、api key，自定义配置不再依赖单行长命令；`cargo test` 141 passed
- Home 渲染重构：不再复用 Session/Input 主布局并通过拼接区域伪装成首页，而是改为真正独立的 Home 渲染路径；光标定位绑定到真实 Prompt 区，不再手算假位置；窗口缩放按独立布局重新计算；清理旧 Home 伪布局残留；`cargo test` 141 passed
- Home 交互面板化：Home 模式下的 dialog/question/permission/text_input 不再以小型 centered overlay 浮在首页之上，而是占据 Home 主面板区域渲染，避免继续呈现“首页上悬浮一个主界面弹窗”的混合观感；`cargo test` 141 passed

## 待修复（预存问题）
- core: DatabaseMigration 超时、LocationServiceMap 隔离、Npm.add 超时
- opencode: permission.task 实时配置加载超时
- typecheck: server.ts + prompt.test.ts 类型错误（Effect Layer 推断）
