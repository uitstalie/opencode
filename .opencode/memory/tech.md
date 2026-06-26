# Tech Decisions

- 选择 ratatui/crossterm + reqwest/rustls + tokio + sled 全栈 Rust，放弃原 gRPC sidecar 方案，做单一二进制直接 HTTP 通信：简化部署、零外部运行时依赖、减少进程间通信开销 #architecture #decision #confirmed
- Provider trait 抽象所有 LLM API，当前先实现 OpenAI-compatible 端点（覆盖 DeepSeek/GPT-5.4/GPT-5.5 via one_route proxy）：Provider-neutral 设计允许后续扩展其他 API 格式 #architecture #decision #confirmed
- Config 复用现有 `~/.config/opencode/opencode.json`，序列化时用 raw `serde_json::Value` 保留未知字段：避免与 TS opencode 的字段冲突，确保双向兼容 #decision #confirmed
- API key 解析链：config 文件 → `{PROVIDER}_API_KEY` env → `OPENAI_API_KEY` env → `--api-key` CLI arg：兼容多种部署场景，env 优先于默认值 #decision #confirmed
- reqwest 使用 `rustls-tls` feature：避免系统 OpenSSL 依赖，简化跨平台构建 #decision #confirmed
