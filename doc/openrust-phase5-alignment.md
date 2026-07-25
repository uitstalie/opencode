# OpenRust Phase 5 主线对齐审计

> 启动日期：2026-07-04  
> 对齐基线：`dev-ai-release` (`0d32d1f29 cli: add --mini`)  
> Rust 分支：`opencode-rust-tui`

## 目标

Phase 5 不推翻 Phase 1 / 2 已完成结论，而是审计这些能力与最新 `dev-ai-release` 的语义差距。

产出用于决定：

- 哪些差距可以并入 Phase 3 主线能力补齐
- 哪些差距必须成为 Phase 4 删除 TS 代码前置门槛
- 哪些能力属于 Rust 分支本地增强，需要单独决定是否长期保留

## 主线变化信号

最新 `dev-ai-release` 中与 Phase 1 / 2 相关的高风险变化集中在：

- `cli: add --mini`（主线信号；OpenRust 明确不采用 mini，改为欢迎界面）
- `feat(server): stream events across locations`
- `fix(tui): use bridged event stream for data`
- `fix(tui): render skill load errors inline`
- `fix(core): handle read file failures`
- `fix(core): bound web tool failures`
- `fix(core): handle missing read paths`
- `fix(core): honor configured agent step limits`

## 差距矩阵

| 域 | 最新主线语义 | OpenRust 当前状态 | 分类 | 建议归属 |
|---|---|---|---|---|
| 启动入口 / 欢迎界面 | 主线新增 `--mini` 轻量入口 | OpenRust 明确不采用 mini；改为默认进入欢迎界面，提供最近会话、配置状态、快速新建/恢复入口 | 有意差异 + 补行为 | Phase 3 |
| TUI 数据通路 | 主线改为 bridged event stream，并支持跨 location 事件流 | OpenRust TUI worker 目前是本地线程 + mpsc 事件流，没有 location/event bridge 抽象 | 需要重新设计 | Phase 5 先设计，Phase 3/4 前置 |
| Session ownership/location | 主线 server events across locations，数据通路带 location 语义 | OpenRust 当前 session/store/location 仍是单进程本地路径模型 | 需要重新设计 | Phase 5 |
| Agent step limits | 主线支持 configured agent step limits，并有 max steps prompt 禁止继续工具调用 | OpenRust `task::run_agent` 有固定 `MAX_TURNS = 24`，但未读取 agent/config step limit，也没有 max-step text-only prompt | 补行为即可 | Phase 3 |
| Tool replay compatibility | 主线 request 在存在历史 tool call 且当前 tools 为空时注入 `_noop` 兼容 Copilot | OpenRust 每轮传完整 tool catalog；尚未覆盖“历史 tool call + tools disabled”重放兼容 | 补测试即可 / 补行为待判定 | Phase 5 |
| Tool registry metadata | 主线工具执行返回 metadata，注入 session/message/tool call 等追踪字段 | OpenRust `ToolResult::Structured` 有 metadata，但多数工具和 runner 未系统写入追踪元数据 | 补行为即可 | Phase 4 门槛 |
| Read tool missing/failure | 主线 read 对缺失路径、二进制、媒体限制、路径逃逸做统一 ToolFailure | OpenRust read 已覆盖 missing path、offset、目录分页、相对路径，但错误模型较简单，缺少二进制/媒体/路径逃逸等完整语义 | 补行为即可 | Phase 3 |
| Web tool failure bounds | 主线 webfetch/websearch 有 URL 校验、timeout 上限、Cloudflare retry、content-type 限制和统一 ToolFailure | ~~OpenRust webfetch 有 timeout 上限和 HTTP 错误处理，但 URL/content-type/Cloudflare/错误归一化较弱~~ **已补齐（2026-07-25）**：webfetch 有 URL/私网校验、timeout 上限、按 format 的 Accept 头、403+`cf-mitigated: challenge` 时用诚实 UA 重试一次、content-type 限制、5MB 上限；websearch 多引擎 fallback，全部引擎失败时返回 ToolFailure 而非空结果 | ~~补行为即可~~ 已对齐 | Phase 3 |
| Skill load errors | 主线 skill 加载 frontmatter 错误会发布 Session.Event.Error，并在 TUI inline 显示 | OpenRust skill 只剥离 frontmatter，不解析/验证 frontmatter，也没有 inline load error 事件 | 补行为即可 | Phase 3/4 门槛 |
| Skill discovery | 主线支持 built-in skill、项目/全局/配置路径/URL、多目录扫描、权限过滤、重复名覆盖规则 | OpenRust skill 支持 `.opencode/skills`、`skills`、全局 config skills；不支持 URL、内置注册顺序、权限过滤完整语义 | 补行为即可 | Phase 3 |
| Memory tools | 当前 Rust 分支有 memory/todo 方向；`dev-ai-release` 基线没有 `memory-review/record` 文件 | 这是本分支增强，不是落后主线；需决定是否保留为 OpenRust 独立能力 | 本地增强待决策 | Phase 5 决策项 |
| Compaction | 主线 compaction 有 tail selection、token estimate、plugin hooks、prune、auto-continue、media stripping、error state | OpenRust 使用 append-only checkpoint (`summary`/`recent`) 和 `effective_messages()`，设计更简单 | 需要重新设计 | Phase 5/Phase 4 门槛 |
| Prompt construction | 主线 prompt/request 有 message transforms、role/template、tool availability、cache/steering 修正 | OpenRust system prompt 已模板化，但未覆盖主线 plugin transform、tool disable、cache-sensitive steering 细节 | 需要重新设计 | Phase 5 |
| TUI tool rendering | 主线修复 subagent tool rows、inline tool spacing、skill load errors inline | OpenRust 已有 tool complete/diff/sidebar 渲染，但未按主线逐项验证这些 UI 边界 | 补测试即可 | Phase 3 |

## 第一批结论

1. **Phase 3 不能只做 UI 打磨**：必须纳入欢迎界面、agent step limits、read/web/skill failure bounds。
2. **Phase 4 不能直接删 TS**：必须先完成 tool metadata、compaction、prompt transform、location/event stream 的替代验收。
3. **Phase 5 需要先产出设计决策**：TUI bridged event stream、location/session ownership、compaction 与 prompt transform 都不是小补丁。
4. **memory 工具是本地增强**：不能用“是否对齐 dev-ai-release”衡量，需要单独决定是否作为 OpenRust 差异化能力保留。

## 建议执行顺序

1. **Phase 5A：行为补齐清单**
   - 欢迎界面：首页状态、最近会话、配置/provider 检查、快速新建/恢复入口
   - agent step limits + max-step prompt
   - read/web/skill failure bounds

2. **Phase 5B：架构差距设计**
   - location-aware event stream
   - compaction parity vs append-only checkpoint
   - prompt/message transform 和 tool replay 兼容

3. **Phase 5C：删除 TS 门槛**
   - 为 `packages/opencode` / `packages/tui` 建立替代矩阵
   - 标记每项为 `done / partial / missing / intentionally different`

## 待确认

- `memory_*` 是否作为 OpenRust 一等能力保留，还是先从 TS parity 矩阵中剥离
- location/event stream 是否要在 Rust 单机版本中完整实现，还是先定义本地等价接口
- compaction 是追主线复杂语义，还是保留 OpenRust append-only checkpoint 并补兼容层
- 欢迎界面的最小范围：仅最近会话 + 新建入口，还是同时显示 provider/config/agent 状态
