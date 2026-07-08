# Tech Decisions

- 选择 ratatui/crossterm + reqwest/rustls + tokio + sled 全栈 Rust，放弃原 gRPC sidecar 方案，做单一二进制直接 HTTP 通信：简化部署、零外部运行时依赖、减少进程间通信开销 #architecture #decision #confirmed
- Provider trait 抽象所有 LLM API，当前先实现 OpenAI-compatible 端点（覆盖 DeepSeek/GPT-5.4/GPT-5.5 via one_route proxy）：Provider-neutral 设计允许后续扩展其他 API 格式 #architecture #decision #confirmed
- Config 使用 openrust 独立配置域，并采用项目 `openrust.json` > 全局 `config.json` 的覆盖语义；同名 provider 由项目配置整块替换，避免全局 provider 字段泄漏到项目运行时 #decision #confirmed
- API key 解析只允许 vault + 配置文件，不再读取 `{PROVIDER}_API_KEY` / `OPENAI_API_KEY` 等环境变量：避免隐藏运行时行为和不可审计凭据来源 #decision #confirmed
- reqwest 使用 `rustls-tls` feature：避免系统 OpenSSL 依赖，简化跨平台构建 #decision #confirmed
- 模型配置必须显式提供，缺少 provider/model 时失败；wire model 会从 `provider/model` 形式解析为供应商实际模型名，避免向 DeepSeek 发送 `deepseek/deepseek-v4-pro` 这类错误名称 #decision #confirmed
- Compaction 采用 append-only checkpoint（`summary`/`recent` 字段）而非覆写历史，`effective_messages()` 从最新 checkpoint 加载：保留完整历史、可回溯、避免破坏性写入 #architecture #decision #confirmed
- Tool calling loop 完全内聚在 `worker.rs`，多轮工具执行由 worker 内部管理，UI 只接收事件：UI 与工具编排解耦，模型/工具多轮循环不泄漏到界面层 #architecture #decision #confirmed
- `todowrite`/`question`/`skill` 工具暂缓移植：它们依赖 session/交互上下文，超出当前 `ToolContext` 能力，需先扩展 `ToolContext`（加 sessionID 等）再逐个补 #decision #confirmed
- `task` 子 agent 用可复用 `run_agent()` headless 循环实现，`ToolContext` 注入 `llm`/`model`；子 agent 上下文清空 llm 与交互通道防止递归调用 #architecture #decision #confirmed
- 权限交互：scope 受限工具在交互模式下经 `permission_tx` 往返弹 `[A]/[D]` 确认；无确认通道时默认拒绝（安全优先），废弃旧的 auto-allow-in-debug #architecture #decision #confirmed
- ESC 中断保留流式传输的部分助手文本（而非丢弃），中断时丢弃 pending 提示队列（干净中断）；跟进消息用独立 channel（followup_tx），不复用 pending_prompts——主循环持久化消息，worker 仅在当前回合注入 #architecture #decision #confirmed
- 30fps 统一轮询（33ms poll_timeout）：AI 运行时 poll 超时强制重绘驱动 spinner，非运行时省 CPU——替代之前 1ms 空轮询 #decision #confirmed
- `mode` 字段语义混乱根因：它被当作 primary/subagent 分类用，但 `tools_for_mode` 期望工具集语义（plan/explore/all），导致 write 过滤从未生效（builtin plan agent mode="primary" ≠ "plan"）——需彻底删除 #decision #confirmed
