# OpenRust Tool Catalog

> 当前工具实现的抽象索引，用于统一静态元数据、分类和 prompt hint。

## 现状

| 类别 | 工具 | 用途 | 备注 |
|---|---|---|---|
| `fs` | `read` | 读取文件内容 | 带行号、offset、limit |
| `fs` | `write` | 写入文件内容 | 支持 undo snapshot |
| `fs` | `edit` | 精确字符串替换 | 支持 replaceAll |
| `fs` | `rm` | 安全删除文件/目录 | system path / scope 双重保护 |
| `fs` | `glob` | 按 glob 搜索文件 | 路径发现 |
| `fs` | `grep` | 正则搜索内容 | 代码检索 |
| `shell` | `bash` | 运行 shell 命令 | 根据平台选择 `bash / pwsh / powershell / cmd` |
| `net` | `webfetch` | 抓取网页内容 | HTML 清洗为纯文本 |
| `net` | `websearch` | 搜索网页 | DuckDuckGo HTML |
| `undo` | `undo_edit` | 从 undo blob 恢复文件 | 依赖 `UndoStore` |

## 抽象边界

1. `tool::Tool` 仍保留为执行层抽象，负责 `execute()` 和 `to_llm_def()`。
2. `tool::catalog` 负责静态元数据：名称、分类、说明、prompt hint。
3. `cli::debug::tool` 和 `core::system_prompt` 共享同一份 catalog，避免重复维护。
4. 新工具优先先进 catalog，再补执行实现和测试。
