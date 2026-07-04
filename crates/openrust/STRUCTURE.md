# OpenRust 代码结构索引

> 记录当前已经实现的 Rust 重写结构，便于后续按模块继续推进。

## 包级结构

| 层级 | 路径 | 作用 | 状态 |
|---|---|---|---|
| package | `crates/openrust/` | Rust 重写主包 | 已启用 |
| library | `src/lib.rs` | 包级模块总入口 | 已启用 |
| binary | `src/main.rs` | CLI 入口与命令分发 | 已启用 |

## 已实现模块索引

| 模块 | 路径 | 主要职责 | 现状 |
|---|---|---|---|
| `cli` | `src/cli/mod.rs` | 顶层 debug 命令分发 | 已实现 |
| `cli::debug::provider` | `src/cli/debug/provider.rs` | provider list / test / models | 已实现 |
| `cli::debug::tool` | `src/cli/debug/tool.rs` | tool list / run / schema | 已实现 |
| `cli::debug::config` | `src/cli/debug/config.rs` | config show / path / set | 已实现 |
| `cli::debug::vault` | `src/cli/debug/vault.rs` | vault test / status / set | 已实现 |
| `cli::debug::session` | `src/cli/debug/session.rs` | session list / show | 已实现 |
| `cli::debug::prompt` | `src/cli/debug/prompt.rs` | system prompt show | 已实现 |
| `core::config` | `src/core/config.rs` | JSONC 配置加载、provider 解析 | 已实现 |
| `core::crypto` | `src/core/crypto.rs` | API key 加密 / 解密 | 已实现 |
| `core::platform` | `src/core/platform.rs` | 平台判定与 config/data/cache 路径布局 | 已实现 |
| `core::paths` | `src/core/paths.rs` | 系统路径保护、project scope | 已实现 |
| `core::provider` | `src/core/provider.rs` | LLM provider trait、消息与流式响应 | 已实现 |
| `core::session` | `src/core/session.rs` | Session / Message / Transcript 持久化 | 已实现 |
| `core::system_prompt` | `src/core/system_prompt.rs` | 固定前缀 + 动态区 system prompt 渲染 | 已实现 |
| `core::vault` | `src/core/vault.rs` | encrypted credentials 存取 | 已实现 |
| `provider` | `src/provider/mod.rs` | provider 实现入口 | 部分实现 |
| `provider::openai_compat` | `src/provider/openai_compat.rs` | OpenAI-compatible provider | 已实现 |
| `tool` | `src/tool/mod.rs` | tool trait、权限检查、工具注册 | 已实现 |
| `tool::bash` | `src/tool/bash.rs` | shell 执行 | 已实现 |
| `tool::read` | `src/tool/read.rs` | 文件读取 | 已实现 |
| `tool::edit` | `src/tool/edit.rs` | 文件修改 | 已实现 |
| `tool::write` | `src/tool/write.rs` | 文件写入 | 已实现 |
| `tool::glob` | `src/tool/glob.rs` | glob 搜索 | 已实现 |
| `tool::grep` | `src/tool/grep.rs` | 正则搜索 | 已实现 |
| `tool::rm` | `src/tool/rm.rs` | 安全删除 | 已实现 |
| `tool::webfetch` | `src/tool/webfetch.rs` | 网页抓取 | 已实现 |
| `tool::websearch` | `src/tool/websearch.rs` | 网页搜索 | 已实现 |
| `tool::undo` | `src/tool/undo.rs` | undo blob 存储 | 已实现 |
| `tool::undo_edit` | `src/tool/undo_edit.rs` | 撤回编辑 | 已实现 |
| `tui` | `src/tui/mod.rs` | TUI 入口占位 | 未完成 |

## 当前结构说明

1. `src/lib.rs` 负责包级导出，供 `main.rs` 和后续测试/扩展复用。
2. `src/main.rs` 保留为极薄二进制入口，只做参数解析和命令分发。
3. `core::platform` 负责平台判定和跨平台目录布局，Windows 与 Fedora 的路径差异只在这里收口，且细分为 `config / data / cache` 三层。
4. `core::session` 负责会话数据持久化，`SessionTranscript` 显式分出固定前缀和动态历史。
5. `core::system_prompt` 负责 system prompt 的分段渲染，固定 section 会保持稳定，动态内容从会话和配置中注入。
6. `cli::debug::*` 是当前开发验证入口，后续新增能力优先补这里的命令。
