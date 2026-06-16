# OpenCode 项目 CI/CD 与 PR/Issue 流程问题分析

> 分析日期：2026-06-15 | 数据来源：GitHub 公开页面（匿名抓取）

---

## 一、关键数字快照

| 指标 | 数值 | 备注 |
|------|------|------|
| Open PR 总数 | **1,029** | 对 12,499 已关闭 |
| 最老 open PR | **#4604** (2025-11-21) | 年龄 ~7 个月 |
| 最新 open PR | #32400 附近 (2026-06-15) | 持续高频流入 |
| Open bug 估计 | **~120-150** | 无确切计数（匿名浏览限制） |
| Milestones | **0** | 无任何里程碑 |
| Labels | **32** | 中等 |
| Issue 总量 | **5k+** | 页面标注 |
| Stars | 175k | Fork 21.1k |

---

## 二、CI/CD 工作流全景

### 测试与质量（每次 push/PR）

| Workflow | 触发 | 内容 |
|----------|------|------|
| `test` | push(dev) + PR | unit (linux/win) + e2e Playwright (linux/win) + HttpApi gates |
| `typecheck` | push(dev) + PR | TypeScript 类型检查 |
| `pr-standards` | PR opened/edited | PR 标题规范、checklist 检查 |
| `pr-management` | PR opened | PR 标签/assign 管理 |
| `nix-eval` | PR synchronize | Nix 构建评估 |

### 自动化清理（定时/事件驱动）

| Workflow | 频率 | 行为 |
|----------|------|------|
| `close-prs` | 每日 22:00 UTC | 关闭 >1 月且 <2 👍 的 PR，最多 50/天 |
| `close-issues` | 每日 2:00 UTC | 关闭过期 issue（Bun 脚本） |
| `compliance-close` | **每 30 分钟** | 关闭打 `needs:compliance` 标签且 2 小时未修正的 issue/PR |
| `duplicate-issues` | issue opened | 检测重复 issue |
| `triage` | issue opened | 自动分诊/标签 |

### 发布与部署

| Workflow | 触发 | 内容 |
|----------|------|------|
| `publish` | push(dev/ci/beta) + manual | **520 行**：版本 bump → 编译 CLI（多平台）→ Azure 签名 Windows → Electron 打包（6 平台）→ 发布 |
| `deploy` | push(dev/production) | AWS SST 部署 + Cloudflare/PlanetScale/Stripe/Sentry |
| `beta` | 每小时 + manual | OpenCode AI 自动同步 beta 分支 |
| `containers` | — | 容器镜像构建 |
| `publish-github-action` | — | GitHub Action 市场发布 |
| `publish-vscode` | — | VSCode 扩展发布 |

### AI 代理驱动

| Workflow | 说明 |
|----------|------|
| `opencode` | issue comment 或 PR 触发，由 OpenCode AI agent 自动处理 |
| `review` | issue comment 触发，AI 辅助 code review |
| `generate` | 代码自动生成 |
| `Copilot` | PR 触发，GitHub Copilot 审查 PR |

### 文档与通知

| Workflow | 说明 |
|----------|------|
| `docs-update` / `docs-locale-sync` | 文档更新与多语言同步 |
| `notify-discord` | Discord 通知 |
| `storybook` | Storybook 构建 |
| `stats` | 统计数据 |
| `nix-hashes` | Nix 哈希更新 |

---

## 三、问题发现

### 🔴 P0：Issue 关闭与 PR 合并脱钩（证据确凿）

**Case: #14808 → #17517**

| 维度 | 详情 |
|------|------|
| Issue #14808 | "Plugin event listener for session.created not firing"，2026-02-23 开，状态 **Closed** |
| PR #17517 | "fix: await plugin event hooks"，2026-03-14 开，目标 dev，状态 **Open** |
| Issue 关联 | 页面标注 `Closed #17517` |
| PR 现状 | 0 review，无 assignee，无 label，3 commits，last force-push 2026-04-16 |
| 推断 | Issue 被手动关闭 + 链接 PR，但 **PR 从未被合并** |

**风险**：用户（包括维护者）看到 issue 关闭以为 bug 已修复，实际修复代码从未合入主分支。这种 issue/PR **假闭环**是流程设计的系统性缺陷。

### 🟠 P1：PR 积压失控

| 指标 | 数值 |
|------|------|
| Open PR | 1,029 |
| 自动关闭速率 | ≤50/天 |
| 新 PR 流入 | 每日大量（issue 号已到 #32400） |
| 最老未合 | 7 个月（2025-11） |

`close-prs` 的阈值（<2 👍，>1 月）是合理的反垃圾机制，但清除速率远低于流入速率。部分 open PR 可能需要人工 triage（如 `needs:issue` 标签的 PR）。

### 🟡 P2：无 Milestone 管理

- **Milestones: 0**，仓库完全不使用里程碑做版本规划
- 没有 release 跟踪、没有路线图可见性
- 对 175k star 的项目来说，缺乏对外的发布预期管理

### 🟡 P2：CI 覆盖缺口 — 无插件集成测试

- `test.yml` 覆盖 unit + e2e (Playwright)，但 **没有插件系统的专门集成测试 job**
- 考虑到 opencode 的核心扩展机制是 plugin/MCP，`session.created` 不触发的 bug（#14808）如果在 CI 有 plugin smoke test 可能更早发现
- 当前的 e2e 测试是 app 级别（electron playwright），不覆盖 plugin 事件总线

### 🟢 P3：close-prs 清理策略可能过于激进

- 关闭条件：>1 月 + <2 👍
- 问题：高质量 fix PR（如 #17517）如果没有足够的社区点赞，也会被关闭
- `close-prs` 的 `--sleep-ms 20000` 限速 20s/个，每日 50 个需 ~17 分钟

---

## 四、流程优点

| 优点 | 详述 |
|------|------|
| **发布流水线成熟** | publish.yml（520 行）覆盖全平台编译、签名、打包，非 trivial |
| **合规自动化严格** | compliance-close 每 30min 检查，2h 宽限期，杜绝垃圾 issue |
| **AI 深度集成** | opencode/beta/review/Copilot 等多个 AI-agent workflow，自用自建 |
| **CONTRIBUTING.md 完善** | issue-first 策略、conventional commits、vouch 机制、2h 合规窗口 |
| **跨平台测试** | unit + e2e 同时跑 Linux 和 Windows |

---

## 五、改进建议

| 优先级 | 建议 | 理由 |
|--------|------|------|
| P0 | **禁止手动关闭 issue 而不合并 PR**：添加 workflow 检测 issue 被 close 时 linked PR 是否已 merged，未 merged 则自动 reopen issue | 防止 #14808/#17517 式假闭环 |
| P1 | **PR 分级 triage**：对 open PR 按 `needs:issue`、CI failed、有 review 等分类，优先处理可合入的 | 1,029 积压需要结构化处理 |
| P1 | **建立 milestones**：至少对下一个 release 建立里程碑，关联 PR/issue | 对外沟通和内部协调 |
| P2 | **增加 plugin smoke test**：在 CI 加入 plugin 事件总线的最小集成测试（启动→加载 plugin→验证 event 触发） | #14808 类问题可更早发现 |
| P2 | **PR 自动关闭策略优化**：对有完整 CI pass + 有 linked issue 的 PR 提高关闭阈值（或豁免），防止高质量 PR 被误关 | 保护有实质贡献的小众 PR |

---

## 六、信息来源

| 页面 | URL |
|------|-----|
| Actions 列表 | https://github.com/anomalyco/opencode/actions |
| Open PR（oldest） | https://github.com/anomalyco/opencode/pulls?q=is%3Apr+is%3Aopen+sort%3Acreated-asc |
| Open bugs | https://github.com/anomalyco/opencode/issues?q=is%3Aissue+is%3Aopen+label%3Abug+sort%3Acreated-asc |
| Issue #14808 | https://github.com/anomalyco/opencode/issues/14808 |
| PR #17517 | https://github.com/anomalyco/opencode/pull/17517 |
| test.yml | https://github.com/anomalyco/opencode/blob/dev/.github/workflows/test.yml |
| close-prs.yml | https://github.com/anomalyco/opencode/blob/dev/.github/workflows/close-prs.yml |
| close-issues.yml | https://github.com/anomalyco/opencode/blob/dev/.github/workflows/close-issues.yml |
| compliance-close.yml | https://github.com/anomalyco/opencode/blob/dev/.github/workflows/compliance-close.yml |
| beta.yml | https://github.com/anomalyco/opencode/blob/dev/.github/workflows/beta.yml |
| publish.yml | https://github.com/anomalyco/opencode/blob/dev/.github/workflows/publish.yml |
| deploy.yml | https://github.com/anomalyco/opencode/blob/dev/.github/workflows/deploy.yml |
| Workflows 目录 | https://github.com/anomalyco/opencode/tree/dev/.github/workflows |
| CONTRIBUTING.md | https://github.com/anomalyco/opencode/blob/dev/CONTRIBUTING.md |

> ⚠️ 匿名抓取存在数据不完整风险（pagination 被截断、部分数据需登录），数字均为保守下界估计。
