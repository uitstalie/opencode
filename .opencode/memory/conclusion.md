# Project Conclusions

## Permission scope 设计决策 #decision #confirmed
- `scope` 检查操作的目标文件路径，非仅 `cwd`（bash 使用 shell parser 解析出的文件路径列表）
- `others` 字段解决"项目内 allow、项目外 deny"的单 JSON key 表达问题（替代旧方案中无法表达的排除语义）
- `$PROJECT` token 运行时解析：由工具层透传 `projectRoot`，不在配置中硬编码
- `external_directory` 保留不变：glob 无法表达"项目外部"的否定语义，需独立配置项

## TUI 颜色编码规则 #decision
- 阈值定义：
  - Context%：`<25%` 绿、`25-50%` 黄、`>50%` 红
  - Cache rate：`<90%` 红、`90-95%` 黄、`>95%` 绿
- 使用 TUI 内置主题色：`theme.success`（绿）、`theme.warning`（黄）、`theme.error`（红）
- 从单 `<text>` 拼接改为多 `<span>` 分别着色
