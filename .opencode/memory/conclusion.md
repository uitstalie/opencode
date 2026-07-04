# Strategic Conclusions

- 决定完整 Rust 重写 opencode（TUI + CLI + backend），单一二进制，零 TypeScript 依赖，无 gRPC sidecar。Old TypeScript packages/tui/ 在 Phase 4 完全移除，不共存 #decision #architecture #confirmed
- CLI debug-first 策略：所有核心功能必须先通过 `opencode debug` CLI 子命令验证，再构建 TUI 界面。降低调试复杂度，确保后端逻辑独立可测 #decision #confirmed
- Rust TUI 自动化验证以 `debug tui replay --script` 作为脚本回放 harness；非 TTY 环境必须 headless 降级，确保 bash 中可观察、可复现地验证 TUI 场景 #decision #confirmed
