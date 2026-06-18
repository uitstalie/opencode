# Project Progress

> 最后更新：2026-06-18

## 已完成

### 基础设施
- 插件系统废弃，`plugins/`、`plugin-config/` 及相关状态文件已全部清理
- 项目定位改为 opencode 源码开发与部署
- AGENTS.md 顶部新增 `Deploy & Update` 部署运维指南
- 修复 enterprise `custom-elements.d.ts` 错误引用和 server `routes` 导出丢失
- 清理所有非 TUI 包（app, desktop, slack, stats, storybook, containers, console, vscode, specs, perf）
- 清理上游 README 翻译和 CONTRIBUTING 文件
- Build 脚本修复：`packages/app` 不存在时自动跳过 Web UI 构建，无需手动 `--skip-embed-web-ui`
- `~/.config/opencode/` 配置从原仓库迁移到本地，项目级 `.opencode/` 纳入源码仓库管理

### System Prompt 模板化项目
- 设计文档：`doc/prompt-architecture.md`、`doc/system-prompt-template-design.md`、`doc/flag-openai-disable-structured-prompt.md`
- 7 个语义 section 标签（`<constraint>` → `<nudge>`，优先级 P0-P7）
- `OPENCODE_DISABLE_STRUCTURED_PROMPT` flag：一键回退到旧扁平拼接
- 角色注入功能：从 config 读取 role 并注入 system prompt
- **渲染管线完整实现**：
  - `template.ts`：`PromptTemplate` / `TemplateSection` 类 + 8 个 section 渲染器
  - `role.ts`：从 `~/.config/opencode/plugin-config/runtime-orchestrator/` 加载 role 内容
  - `prompt.ts`：`buildStructuredSystem()` 统一构建全部 8 个 section：
    - `<constraint>` — 按 `/rules/` 路径从 `instruction.system()` 分流
    - `<identity>` — agent.prompt 或 `SystemPrompt.provider(model)`
    - `<environment>` — `sys.environment(model)`
    - `<instructions>` — 非 rule 的 instruction（AGENTS.md 等）
    - `<capabilities>` — `sys.skills(agent)`
    - `<style>` — `loadRoleForPrompt()`
    - `<memory>` — `.opencode/memory/conclusion.md` + `tech.md`
    - `<nudge>` — 约束感知提示
  - `request.ts`：删除 `buildStructuredSystemPrompt()`，模板启用时直接透传 `input.system`

### Memory V2
- 设计定稿：`runtime-orchestrator-memory-v2-design.md`
- 按 scope 分流存储（项目 SQLite + 全局 .md 文件）
- 异步后台提取：插件内直调 LLM API，绕开 agent/session
- 4 个核心工具，精简工具层
- 背景记忆提取 via daemon fiber

### 诊断与修复
- Skill 加载问题排查：确认 skill 代码加载流水线无 bug，旧二进制重编后修复
- 所有 7 个 skill（含 project-onboarding、plugin-dev）正常加载
- 确认系统提示跨会话缓存机制：Context Epoch 基线跨进程重启复用

## 进行中
- project-onboarding skill 升级（适配 memory V2 + 新项目架构）

## 待办
- 清理 `doc/` 下设计文档中标记的 TODO/待接入点
- Memory V2 后台提取接入实际 LLM API 调用链
- nudge 内容动态化（从约束规则自动生成，替代硬编码）
