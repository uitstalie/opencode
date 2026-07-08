# Strategic Conclusions

- 决定完整 Rust 重写 opencode（TUI + CLI + backend），单一二进制，零 TypeScript 依赖，无 gRPC sidecar。Old TypeScript packages/tui/ 在 Phase 4 完全移除，不共存 #decision #architecture #confirmed
- CLI debug-first 策略：所有核心功能必须先通过 `opencode debug` CLI 子命令验证，再构建 TUI 界面。降低调试复杂度，确保后端逻辑独立可测 #decision #confirmed
- Rust TUI 自动化验证以 `debug tui replay --script` 作为脚本回放 harness；非 TTY 环境必须 headless 降级，确保 bash 中可观察、可复现地验证 TUI 场景 #decision #confirmed
- 删除 `mode` 概念：agent 能力完全由 md 文件（frontmatter + body）定义，不再有代码层 mode 分类；工具集控制改由 frontmatter `read_only: true` 布尔值实现（true 时过滤 write/edit/rm/apply_patch/bash）——compaction/title/summary 内置 agent 硬编码调用 `run_agent(...,"subagent",...)`，不依赖 mode 字段，迁移安全 #decision #architecture #confirmed
- 分支合并策略：`opencode-rust-tui` → `rust` 采用完整 merge（非 cherry-pick），保留双方所有功能和历史。cherry-pick 失败的原因是冲突解决时把 TUI 改动逐个回退到 rust 旧接口以编译通过，等于撤销工作，用户叫停。两分支从 `00b960d34` 分叉：rust 21 commits（permission/rules/TODO state machine/skill/provider fix/agent overlay with `mode` field），tui 2 commits（tools preset `tools`/`presets`/`resolve_tool_names` + ESC 中断/followup/光标修复/渐进式工具显示）。合并时需保留 rust 的 agent overlay 与 TUI 的全部功能 #decision #confirmed
