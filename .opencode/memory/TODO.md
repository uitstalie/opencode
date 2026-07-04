# TODO

## 高优先级
- [x] **Rust TUI Phase 0**：cargo init、clap CLI、Config 加载、Provider trait、LLM 实测
- [x] **Rust TUI Phase 0.9**：api_key AES-256-GCM 加密 + machine-id 绑定
- [x] **Rust TUI Phase 1A**：10 个工具 + permission 系统 + Tool trait 抽象
- [x] **Rust TUI Phase 1A.1**：二进制改名 → openrust
- [x] **Rust TUI Phase 1A.2**：rm 工具 + project scope 管控 + shared paths 模块
- [x] **Rust TUI Phase 1B**：会话管理 + system prompt 渲染
- [x] **Rust TUI Phase 1C**：最小 TUI（ratatui session view, input box, tool loop）
- [x] **Rust TUI Phase 1D**：agent/task flow + 真实 tool loop + provider 消息对齐 + apply_patch + `question`/`todowrite`/`skill`（扩展 `ToolContext` 承载 session/交互上下文）全部完成
- [x] **Rust TUI Phase 1 收尾**：`task` 子 agent（可复用 `run_agent`）+ 真实权限弹窗 `[A]/[D]` + `core/permission.rs`（scope + `$PROJECT`）+ `debug permission check` / `debug e2e` 命令；14 工具齐全，113 测试全绿
- [ ] **Rust TUI Phase 2**：Markdown 渲染 + 语法高亮（tree-sitter）+ Diff 查看器 + 文件树侧边栏
- [ ] nudge 内容动态化：从 constraint 规则自动生成 nudge，替代 `request.ts` 硬编码

## 中优先级
- [ ] 清理 `doc/` 设计文档中标记的 TODO/待接入点

## 测试
- [ ] **Permission Scope 测试用例**（`doc/permission-scope-design.md` Phase 4）：为 scope 匹配逻辑新增测试用例
- [ ] **预存失败修复**：core 3 个失败 + opencode 1 个超时 + 4 typecheck 错误

## 低优先级
- [ ] Context Epoch 替换触发时机验证（agent/model 切换）
- [ ] Skill 热加载 / filesystem watch 失效机制
- [ ] `tool_output` 结构化结果溢出处理
