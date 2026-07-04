# Tech Decisions

- 选择 ratatui/crossterm + reqwest/rustls + tokio + sled 全栈 Rust，放弃原 gRPC sidecar 方案，做单一二进制直接 HTTP 通信：简化部署、零外部运行时依赖、减少进程间通信开销 #architecture #decision #confirmed
- Provider trait 抽象所有 LLM API，当前先实现 OpenAI-compatible 端点（覆盖 DeepSeek/GPT-5.4/GPT-5.5 via one_route proxy）：Provider-neutral 设计允许后续扩展其他 API 格式 #architecture #decision #confirmed
- Config 使用 openrust 独立配置域，并采用项目 `openrust.json` > 全局 `config.json` 的覆盖语义；同名 provider 由项目配置整块替换，避免全局 provider 字段泄漏到项目运行时 #decision #confirmed
- API key 解析只允许 vault + 配置文件，不再读取 `{PROVIDER}_API_KEY` / `OPENAI_API_KEY` 等环境变量：避免隐藏运行时行为和不可审计凭据来源 #decision #confirmed
- reqwest 使用 `rustls-tls` feature：避免系统 OpenSSL 依赖，简化跨平台构建 #decision #confirmed
- 模型配置必须显式提供，缺少 provider/model 时失败；wire model 会从 `provider/model` 形式解析为供应商实际模型名，避免向 DeepSeek 发送 `deepseek/deepseek-v4-pro` 这类错误名称 #decision #confirmed
