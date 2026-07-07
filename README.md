# openrust — opencode 的完整 Rust 重写

> 基于 [uitstalie/opencode](https://github.com/uitstalie/opencode) fork，对原 TypeScript 实现进行完整 Rust 重写。

## 与上游差异

- **单一 Rust 二进制**：`openrust`，零 TS/Node/Bun 依赖，无 gRPC sidecar
- **全栈 Rust**：reqwest + rustls + tokio + sled + ratatui/crossterm
- **移除全部 TS 包**：原 `packages/` 下 23 个 TypeScript 包、bun/turbo 工具链、husky hooks、SST infra、nix TS 工具链均已移除
- **debug-first**：核心功能通过 `openrust debug <subcommand>` CLI 验证，再接入 TUI

## 分支

| 分支 | 用途 |
|------|------|
| `dev` | fork 默认分支 |
| `opencode-rust-tui` | Rust TUI 工作分支（镜像到 `origin/rust`） |

## 构建

```bash
cd crates/openrust && cargo build --release

# 输出
ls target/release/openrust
```

## 部署（本机热替换）

```bash
cd crates/openrust && cargo build --release && \
  cp ~/.opencode/bin/openrust ~/.opencode/bin/openrust.bak && \
  cp target/release/openrust ~/.opencode/bin/openrust
```

替换后需重启 opencode 生效。

## 同步上游

```bash
git checkout opencode-rust-tui
git fetch upstream dev
git rebase upstream/dev   # 注意：上游是 TS，rebase 可能冲突，通常仅用于参考
git push origin opencode-rust-tui
```

## 许可

上游项目基于 MIT License。本 fork 的修改部分同样遵循 MIT。
