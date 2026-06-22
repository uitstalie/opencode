# Tech Stack
- 平台：Linux x64，Bun 运行时
- 编译：`bun run build --single`（输出 `packages/opencode/dist/opencode-linux-x64/bin/opencode`）
- 安装路径：`~/.opencode/bin/opencode`
- 版本号格式：`0.0.0-dev-{branch}-{timestamp}`

## Permission scope 架构 #architecture
- `Rule.scope?: string`：glob 路径作用域，`$PROJECT` 动态 token
- `RuleDetail` 联合类型：`Action | { action, scope, others }`
- `evaluate()` 接收 `opScope` + `projectRoot`，展开 `$PROJECT`，走 `scopeMatch()` glob 匹配
- 工具层（bash/read/edit/write/glob/grep）统一通过 `ctx.ask()` 传递 scope/projectRoot
- bash 特殊处理：`opScope` 来自 shell parser 的 `scan.dirs` 文件路径列表，非仅 `cwd`

## TUI 主题色 #ui
- 可用颜色 token：`theme.success`、`theme.warning`、`theme.error`
