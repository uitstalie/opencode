# Project Progress

> 最后更新：2026-06-24

## 已完成

### 基础设施
- 插件系统废弃，全部源码编译；`~/.config/opencode/` 清理 97MB → 340K
- 剔除非 TUI 包（app, desktop, slack, stats 等）
- 项目级 `.opencode/` 配置纳入源码仓库，AGENTS.md 新增部署运维指南
- Build 脚本修复：`packages/app` 缺失时自动跳过 Web UI 构建

### System Prompt 模板化
- 8 个语义 section（`<constraint>` → `<nudge>`），优先级 P0–P7
- 完整渲染管线：`template.ts` → `role.ts` → `prompt.ts` → `request.ts`
- Role 注入：从 `~/.config/opencode/plugin-config/runtime-orchestrator/` 加载

### Permission Scope 系统
- `Rule.scope`（glob）+ `$PROJECT` 动态 token + `others` fallback
- 6 个工具全适配（bash 特殊：从 shell parser `scan.dirs` 提取文件路径）
- `external_directory` 保留独立配置

### TUI
- 状态栏新增 cache hit rate
- 颜色编码：context%（绿<25%/黄25-50%/红>50%）、cache rate（红<90%/黄90-95%/绿>95%）
- TDZ crash 修复（useTheme 移到 memo 之前）

### Memory V2
- 设计定稿：项目 SQLite + 全局 .md 分流存储
- 4 个核心工具：`memory_review`、`memory_record`、`dreaming_compress`、`todowrite`
- Daemon fiber 后台提取骨架完成，`memory_record` 已支持 markdown 同步

### 内置 Skill
- `customize-opencode`：opencode 自身配置参考
- `write-skills`：SKILL.md 格式、YAML 引号规则、常见陷阱

### Init
- `/init` 集成 project-onboarding skill + scaffold 感知
- Template 重写为 action-oriented

### 诊断填充
- Skill 加载静默失败修复（YAML `:` 无引号 → gray-matter 解析失败 + 防御性日志）
- 确认 Context Epoch 跨进程重启复用机制

### Edit Undo Phase 2
- inline `undo` 参数回归（edit.ts / write.ts → `undo?: boolean`，默认 true）
- 连续多步撤回链（undo_edit 返回 redoHash 作为新 undoHash → undo → undo → undo）
- undo-blobs GC（每 10 次保存扫描清理 >24h 的 blob）
- 测试：4 个 undo 用例（参数开关、还原、链式撤回），edit 33 用例全通过

## 进行中
- nudge 内容动态化：从 constraint 规则自动生成 nudge

## 待修复（预存问题）
- core: DatabaseMigration 超时、LocationServiceMap 隔离、Npm.add 超时
- opencode: permission.task 实时配置加载超时
- typecheck: server.ts + prompt.test.ts 类型错误（Effect Layer 推断）
