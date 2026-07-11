# TODO

## 高优先级
- [ ] **完成 `/init` 实现**：`handle_slash_command` 返回 `SlashResult`（2 个调用点 mod.rs:418/838 待适配）→ `init_template()` 函数 → system prompt onboarding hint → `/init` 解析 + enqueue 适配
- [ ] **push 未 push commits**：当前分支 `rust` 领先 `origin/rust` 6+ commits（55d6e915d ~ 885895543 + retry 待 commit）
- [x] **后台 Agent + 记忆系统 P0 设计决策**：①记忆注入机制——Memory 内容不注入 system prompt 正文，只注入静态 `<memory>` 位置索引；②Memory vs Rules 优先级——Rules 是硬约束全量注入，Memory 按需 tool 读取。全部确认
- [x] 更新 `doc/background-agent-design.md` 补充 P0 决策后提交（commit `c2beb7947`）
- [x] 阶段 1 实现：`core/memory.rs` MemoryStore + 2 个 memory 工具（memory_read/memory_record），commit `e5c60d0d0`。注：原计划 4 工具，最终砍到 2 个（memory_review 删除，dreaming_compress 改为 /dream 命令）
- [x] 阶段 2 实现：memory-extract agent + 增量触发（commit `f74041fa7`）。clippy/test 修复 commit `5d829fddc`。274 passed
- [x] 阶段 2.5：dreaming agent + `/dream` 命令（commit `5ad0c800c`）
- [ ] **分支合并 `opencode-rust-tui` → `rust`**：完整 merge 保留双方功能——rust 的 agent overlay（`mode`/`current_agent_info`）+ permission system，TUI 的 tools preset + ESC 中断/followup/渐进式工具显示/光标修复。解决冲突时保留双方改动而非回退，完成后 cargo check + test + clippy 全绿再 push `origin rust`
- [x] **Rust TUI Phase 0**：cargo init、clap CLI、Config 加载、Provider trait、LLM 实测
- [x] **Rust TUI Phase 0.9**：api_key AES-256-GCM 加密 + machine-id 绑定
- [x] **Rust TUI Phase 1A**：10 个工具 + permission 系统 + Tool trait 抽象
- [x] **Rust TUI Phase 1A.1**：二进制改名 → openrust
- [x] **Rust TUI Phase 1A.2**：rm 工具 + project scope 管控 + shared paths 模块
- [x] **Rust TUI Phase 1B**：会话管理 + system prompt 渲染
- [x] **Rust TUI Phase 1C**：最小 TUI（ratatui session view, input box, tool loop）
- [x] **Rust TUI Phase 1D**：agent/task flow + 真实 tool loop + provider 消息对齐 + apply_patch + `question`/`todowrite`/`skill`（扩展 `ToolContext` 承载 session/交互上下文）全部完成
- [x] **Rust TUI Phase 1 收尾**：`task` 子 agent（可复用 `run_agent`）+ 真实权限弹窗 `[A]/[D]` + `core/permission.rs`（scope + `$PROJECT`）+ `debug permission check` / `debug e2e` 命令；14 工具齐全，113 测试全绿
- [x] **Rust TUI Phase 2**：Markdown 渲染（pulldown-cmark 自研）+ 语法高亮（syntect 纯 Rust）+ Diff 查看器（similar last-turn）+ 文件树侧边栏（notify）
- [ ] nudge 内容动态化：从 constraint 规则自动生成 nudge，替代 `request.ts` 硬编码
- [ ] **mode → read_only 迁移（8 步）**：①还原 agent.rs 临时 mode 变更 → ②删除 AgentInfo.mode 字段 → ③添加 read_only: bool + frontmatter 解析（默认 false）→ ④重写 tools_for_mode→tools_for(read_only, is_subagent)：read_only=true 过滤 WRITE_TOOLS → ⑤更新 worker.rs/task.rs 调用方 → ⑥general/explore 设 hidden=true（修复 Tab 误切）→ ⑦更新 frontmatter 解析测试 + tools_for/visible_agents 测试 → ⑧cargo test + cargo clippy
- [ ] **状态栏重构**：model/thinking/mode 从单行状态栏移到输入框下方 info line；status 行保留 context/cache/tasks/running

## 中优先级
- [ ] 清理 `doc/` 设计文档中标记的 TODO/待接入点

## 测试
- [ ] **Permission Scope 测试用例**（`doc/permission-scope-design.md` Phase 4）：为 scope 匹配逻辑新增测试用例
- [ ] **预存失败修复**：core 3 个失败 + opencode 1 个超时 + 4 typecheck 错误

## 低优先级
- [ ] Context Epoch 替换触发时机验证（agent/model 切换）
- [ ] Skill 热加载 / filesystem watch 失效机制
- [ ] `tool_output` 结构化结果溢出处理
