# opencode (uitstalie + DeepSeek custom fork)

> 基于 [anomalyco/opencode](https://github.com/anomalyco/opencode) 的个人扩展版本，针对 TUI 场景深度定制。

## 与上游差异

- **精简包结构**：移除 desktop、web、vscode、console、slack、stats 等非 TUI 组件
- **插件事件修复**：修复开放插件 event hook 无法接收内部 session 事件的问题
- **自维护升级**：`opencode upgrade` 改为 git + 本地编译流程，不从上游 GitHub Releases 拉取
- **git hooks 管控**：分支提交规则（release 只接受 cherry-pick 等）

## 分支

| 分支 | 用途 |
|------|------|
| `dev-ai` | 开发分支 |
| `dev-ai-release` | 稳定发布分支（默认） |

## 构建

```bash
# 安装依赖
bun install

# 编译当前平台
cd packages/opencode
bun run build --single --skip-embed-web-ui

# 输出
ls dist/bun-linux-x64/bin/opencode
```

## 同步上游

```bash
git checkout dev-ai
git fetch upstream dev
git rebase upstream/dev
git push origin dev-ai
```

## 许可

上游项目基于 MIT License。本 fork 的修改部分同样遵循 MIT。

---

*维护者：uitstalie · AI 辅助：DeepSeek*
