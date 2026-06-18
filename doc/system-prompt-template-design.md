# System Prompt 模板结构化设计

> 版本: draft-1 | 日期: 2026-06-16 | 模型: deepseek-v4-pro

## 1. 目标

将当前 `join("\n")` 拼接的扁平 system prompt 改造为 **语义标签分段** 结构，让 DeepSeek 能按 section 粒度理解指令来源和优先级。

**核心约束**:
1. **最大 prefix cache 命中** — system prompt 在会话内不变，prefix cache 始终命中
2. **最大规范遵从度** — 每个 section 有明确优先级，LLM 优先服从低编号 section 的规则
3. **动态内容不入 system** — mood 已完全移除

## 2. 当前问题

```
现状: 一条 system message, content = [
  Constraint Layer          ← 来源: plugin prepend
  Agent Prompt              ← 来源: opencode.json / default.txt
  Environment               ← 来源: system.ts
  Instructions              ← 来源: AGENTS.md + rules
  Skills                    ← 来源: system.ts
  Tool Reference            ← 来源: plugin append
  Dreaming Insights         ← 来源: plugin append
].join("\n")
```

**缺陷**:
- 无边界标记，模型无法区分"这是硬约束" vs "这是背景信息"
- 多来源互相覆盖/冲突时无优先级标注
- 插件追加内容挤在末尾，可能被长指令淹没
- 模式切换 (build→plan→design) 时无结构化 anchor

## 3. 设计方案

### 3.1 标签体系

使用 7 个语义 section 标签，按优先级从高到低排列：

| 优先级 | 标签 | 语义 | 生命周期 | 来源 |
|---|---|---|---|---|
| P0 | `<constraint>` | 硬约束，不可协商 | 始终存在 | `plugin/prompt/compose.ts:82-101` |
| P1 | `<identity>` | 身份 + 模式 | 会话级 | `system.ts:25-38` + `agent.prompt` |
| P2 | `<environment>` | 运行时上下文 | 会话级 | `system.ts:55-91` |
| P3 | `<instructions>` | 项目/用户指令 | 会话级 | `instruction.ts:48-103` |
| P4 | `<capabilities>` | 可用 skills + tools | 会话级 | `system.ts:94-106` + `plugin/compose.ts:103-122` |
| P5 | `<style>` | 输出风格 + role | 会话级 | `plugin/compose.ts:124-163` |
| P6 | `<memory>` | 记忆工具指引 | 始终存在 | `plugin/compose.ts:57-80` |
| P7 | `<nudge>` | 每轮提示 | 始终存在 | `plugin/hooks/session.ts:170` |

### 3.2 完整模板

```xml
<constraint>
## Tool Invocation Rules
- [Memory] 决策前必查 memory_read（含 dreaming）。写入前必 memory_review 查重。稳定结论才 memory_record。
- [Compact] 每轮结束调 compact_check。shouldCompact=true → 立刻 compress，再继续回复。

## Concrete Discipline
- memory 宁缺毋滥：只记稳定结论/长期约束/明确偏好，不写流水账。
- 写入流程：memory_review → 确认无重复 → memory_record；不确定规则走 memory_candidates。
- 上下文接近上限时主动 compact_check，不等溢出。
</constraint>

<identity mode="build">
<!-- ✅ 合并模式: provider prompt (default.txt) + mode 特定 instruction -->
You are opencode, an interactive CLI tool... (default.txt 全量内容)

# Mode: Build
Build mode: implement directly unless discussion only is requested. Read existing code before editing, make the smallest correct change, then verify and report clearly.
</identity>

<environment>
You are powered by the model named deepseek-v4-pro. The exact model ID is deepseek/deepseek-v4-pro

Here is some useful information about the environment you are running in:
- Working directory: {cwd}
- Workspace root folder: {worktree}
- Is directory a git repo: {yes/no}
- Platform: linux
- Today's date: {date}

Project references provide additional directories that can be accessed when relevant.
{references}
</environment>

<instructions>
Instructions from: /mnt/d/dqc/fake_opencode/AGENTS.md
{project_agents_md_content}

Instructions from: ~/.config/opencode/rules/core-operating-rules.md
{core_rules_content}

Instructions from: ~/.config/opencode/rules/internet-rules.md
{internet_rules_content}
</instructions>

<capabilities>
## Available Skills
Skills provide specialized instructions and workflows for specific tasks.
Use the skill tool to load a skill when a task matches its description.
{skills_list}

## Available Tools
- 知识库查询：shared-knowledge skill 或 kb-mgr task
- 跨会话洞察：memory_read(scope=user, layer=dreaming) ...
- 项目记忆：memory_read / memory_record / memory_review / memory_distill
- Token 用量：token_stats / token_history
</capabilities>

<style>
  <role name="maid">
    <boundaries>
      {role_boundaries}
    </boundaries>
    <expression>
      {role_expression_anchors}
    </expression>
    <self_checks>
      {role_extra_rules}
    </self_checks>
  </role>
</style>

<memory>
## Dreaming Insights (cross-session knowledge)
- Dreaming 已从历史 session 提取跨会话习惯/规则/模式，存储在 ~/.config/opencode/memory/dreaming/。
- 做决策前应查阅: memory_read(scope=user, layer=dreaming) 查看全局跨项目模式。
- Dreaming 中标记 #confirmed 的模式应视为与 memory/preferences 同等的约束来源。
</memory>

<nudge>
[CONSTRAINT NUDGE] memory_read(决策前) → compact_check
</nudge>
```

### 3.3 模式 (mode) 如何影响模板

不同 mode 修改 `<identity>` 标签内容，其余 section 保持一致：

| mode | `<identity>` 内容 | 来源 |
|---|---|---|
| build | `Build mode: implement directly...` | opencode.json agent.build.prompt |
| plan | `Plan mode: analyze and produce implementation plans...` | opencode.json agent.plan.prompt |
| design | `Design mode: normalize requirements before implementation...` | opencode.json agent.design.prompt |
| (无 config) | `default.txt` 的 95 行 base prompt | system.ts provider() 回退 |

**建议**: 每个 mode 的 prompt 不用替换整个 `<identity>`，而是用属性标注：

```xml
<identity mode="build" provider="deepseek">
  {mode_specific_instruction}
</identity>
```

### 3.4 风格 (style) 层如何组织

`<style>` 包含 role 定义。

| 子块 | 来源 | 触发条件 |
|---|---|---|
| Output Rules | role 定义的 boundaries + behaviorRules | `enableRole=true` |
| Expression Anchors | role 定义的 expressionAnchors | `enableRole=true` |
| Self Checks | role 定义的 extraRules | `enableRole=true` |

### 3.5 插件如何贡献 section

插件不拥有独立 section，而是**注入到已有 section 中**：

```
插件 hook                    注入目标
──────────────────────────────────────────────────
buildConstraintLayer()   →   <constraint> prepend
buildPluginToolReference()→  <capabilities> append (tools 子块)
buildDreamingInsightSection()→ <memory>
buildStableSystemSections()  → <style> prepend (role 子块)
getConstraintNudgeText()     → <nudge>
buildDynamicTailSections()   → 已移除 (mood 不再需要)
```

**未来扩展**: 如果插件需要增加新 section（如自定义 agent 添加 `<domain_knowledge>`），schema 必须支持可扩展标签。

### 3.6 动态 vs 静态 section (Cache 优先)

**核心原则**: system prompt 前缀不变则 prefix cache 命中。DeepSeek 支持 prompt caching。

所有 7 个 section 在同一会话内不变 → **prefix cache 始终命中**。

| 类型 | section | 变化频率 | Cache 影响 |
|---|---|---|---|
| 静态 | `<constraint>`, `<memory>`, `<nudge>` | 几乎不变 | ✅ 始终命中 |
| 会话级 | `<identity>`, `<environment>`, `<instructions>`, `<capabilities>`, `<style>` | 会话开始确定 | ✅ 同一会话内命中 |

**mood 已移除**: 从 system prompt 和 messages.transform 中完全移除。enableMood=false。

## 4. 实现路径

### Phase 1: 定义标签 schema

1. 在 `session/prompt/template.ts` 定义 `TemplateSection` + 子标签类型
2. 定义 `TemplateRenderer` 将嵌套 section 对象渲染为带标签文本
3. 定义 `TemplateComposer` 接收各来源 section 对象，合并为最终 text

```ts
// 核心类型 — 双层嵌套
interface RoleSection {
  name: string
  boundaries: string[]
  expressionAnchors: string[]
  behaviorRules: string[]
  selfChecks: string[]
}
// mood 已移除

interface TemplateSection {
  id: string           // "constraint" | "identity" | ...
  mode?: string        // "build" | "plan" | "design"
  priority: number     // 0-6
  content: string
  children?: {
    roles?: RoleSection[]
  }
  source: string       // "binary" | "config" | "plugin" | "instruction"
}

// 使用 (替换 prompt.ts:1327-1333)
const template = TemplateComposer.compose({
  constraint: buildConstraintSection(),
  identity: buildIdentitySection(agent, model),  // ✅ 合并模式
  environment: buildEnvironmentSection(model),
  instructions: buildInstructionsSection(paths),
  capabilities: buildCapabilitiesSection(skills),
  style: buildStyleSection(role),                // ✅ 仅 role
  memory: buildMemorySection(dreaming),
})
system = [template.render()]
```

### Phase 2: 迁移现有层

1. `system.ts:environment()` → 输出 `<environment>` 块而非裸文本
2. `instruction.ts:system()` → 输出带 `<instructions>` 标签
3. `system.ts:skills()` → 输出带 `<capabilities>` 标签
4. `llm/request.ts:58-66` → ✅ **合并模式**: `agent.prompt` 不再替换 `provider(model)`，而是追加
5. 插件 `system.transform` → 注入子块到 section 内嵌套标签，不再自己拼接顶层级

### Phase 3: 模式优化

1. 每个 mode 的 prompt 作为 `<identity>` 的子块而非替换
2. `default.txt` 作为 provider prompt 始终注入 `<identity>` 上部
3. mode 特定 content 追加为 `<identity>` 下部子块

### Phase 4: 风格 + 约束引擎

1. role → `<style><role>` 嵌套子标签（会话级，不变 → cache 友好）
2. nudge → `<nudge>` 追加到末尾，包含 `[CONSTRAINT NUDGE] memory_read → compact_check`
3. mood → 已移除

## 5. 与其他 provider 的兼容性

| provider | `<section>` 标签支持 | 替代格式 |
|---|---|---|
| DeepSeek (openai-compatible) | ✅ 原生支持 | 标签天然是文本 |
| Claude (Anthropic) | ✅ 已有 XML prompt 规范 | 可复用同结构，换标签名为 `<constraint>` → `<hard_constraint>` |
| GPT-4 | ✅ | 标准 OpenAI 格式 |
| Gemini | ✅ | 相同 |

所有 provider 都把 system prompt 当作纯文本，`<section>` 标签不需要特殊处理。

## 6. 对比：当前 vs 模板化

| 维度 | 当前 | 模板化后 |
|---|---|---|
| 结构 | 单字符串 `join("\n")` | 7 个语义 `<section>` |
| 优先级 | 隐式排列 | `priority` 属性 + 排列顺序 |
| 可扩展 | 插件只能 prepend/append | 插件注入到已有 section 子块 |
| 动态更新 | 重建全部 system prompt | 只重建变化的 section |
| 调试 | 无法定位来源 | 每段标注 `source` 属性 |
| Token 缓存 | 波动大 | 静态 section 前缀缓存稳定 |

## 7. Step 2: 标签内容规范化 + CRUD 接口

### 7.1 标签 → 源码映射

| 标签 | 当前来源文件 | 当前函数 | 当前格式 |
|---|---|---|---|
| `<constraint>` | `plugin/prompt/compose.ts:82-101` | `buildConstraintLayer()` | markdown `##` + `- [key]` 列表 |
| `<identity>` | `system.ts:25-38` + `prompt.ts:1333` | `provider()` + `agent.prompt` | .txt 文件内容 + 用户 config |
| `<environment>` | `system.ts:55-91` | `environment()` | `<env>` XML + references |
| `<instructions>` | `instruction.ts:48-103` | `system()` + `resolve()` | 文件原文 `join("\n")` |
| `<capabilities>` | `system.ts:94-106` + `plugin/prompt/compose.ts:103-122` | `skills()` + `buildPluginToolReference()` | markdown 列表 |
| `<style>` | `plugin/prompt/compose.ts:124-163` | `buildStableSystemSections(role)` | markdown `#` headers + `-` 列表 |
| `<memory>` | `plugin/prompt/compose.ts:57-80` | `buildDreamingInsightSection()` | markdown 列表 |
| `<nudge>` | `plugin/hooks/session.ts:170` | CONSTRAINT NUDGE (messages.transform) | 单行文本 |

### 7.2 每标签内部结构规范

每个标签内容必须遵循**统一内部格式**，以确保 append/delete 操作可精确定位。

#### `<constraint>` — 规则项列表

```
<constraint>
## Tool Invocation Rules
- [Memory] {rule_text}
- [Compact] {rule_text}

## Concrete Discipline
- {rule_text}
- {rule_text}
</constraint>
```

**规范**:
- 顶层 `##` 标题为 subsection 分组（插件可追加新 subsection）
- 规则项以 `- ` 开头
- **Key 标记**: 工具规则用 `[ToolName]` 格式作 key；行为规则用 `{discipline-N}` 序号作 key
- Append 默认加到所属 subsection 末尾
- Delete 用 `[Key]` 匹配

#### `<identity>` — 纯文本块 + mode 标签

```
<identity mode="build" provider="deepseek">
line 1
line 2
...

## Mode
{mode_specific_instruction}
</identity>
```

**规范**:
- 第一行: provider prompt 原文 (来自 `default.txt` 或 provider 特定 .txt)
- `## Mode` 标题后: agent mode 特定 prompt
- mode 切换时**仅替换 `## Mode` 之后内容**，provider prompt 不变

#### `<environment>` — 键值对 + 引用列表

```
<environment>
model: {id}
provider: {providerID}
cwd: {path}
worktree: {path}
is_git: {yes/no}
platform: {os}
date: {date}
references:
  - name: {name}, path: {path}, description: {desc}
  - name: {name}, path: {path}
</environment>
```

**规范**:
- 每行 `key: value`
- `references:` 后跟缩进列表

#### `<instructions>` — 来源声明 + 内容块

```
<instructions>
<!-- source: /path/to/AGENTS.md -->
{content}

<!-- source: /path/to/CLAUDE.md -->
{content}

<!-- source: ~/.config/opencode/rules/core-operating-rules.md -->
{content}
</instructions>
```

**规范**:
- 每个来源块以 `<!-- source: {path} -->` 开头（HTML 注释，LLM 不可见但可解析）
- 内容原样保留
- 文件路径标识唯一性，用于去重和删除

#### `<capabilities>` — 分类列表

```
<capabilities>
## Skills
- {skill_name}: {description}

## Tools
- {tool_name}: {usage_description}
- {tool_name}: {usage_description}
</capabilities>
```

**规范**:
- `## Skills` / `## Tools` 为固定 subsection
- 每个条目 `- name: description`

#### `<style>` — Role 双层嵌套

```
<style>
<role name="maid">
  <boundaries>
  - {rule}
  - {rule}
  </boundaries>
  <expression>
  - {anchor}
  </expression>
  <self_checks>
  - {check}
  </self_checks>
</role>
</style>
```

**规范**:
- 子标签 `<boundaries>`, `<expression>`, `<self_checks>` 为固定子块
- 每条规则以 `- ` 开头

#### `<memory>` — 说明文本 + 规则列表

```
<memory>
## Dreaming Guide
- {rule}
- {rule}

## Reminder
{dreaming_reminder_text}
</memory>
```

#### `<nudge>` — 纯单行

```
<nudge>
[CONSTRAINT NUDGE] memory_read(决策前) → compact_check
</nudge>
```

**规范**: 每次只包含一个 nudge 指令，可多行但每行独立。

### 7.3 规范化操作接口 (CRUD)

`TemplateSection` 提供 item 级操作 API，隐藏内部格式差异：

```ts
interface SectionItem {
  key: string           // 唯一标识, e.g. "[Memory]", "src-AGENTS.md"
  content: string       // 内容文本
  subsection?: string   // 所属 subsection, e.g. "Tool Invocation Rules"
}

interface TemplateSection {
  id: string            // "constraint" | "identity" | ...
  mode?: string
  priority: number
  items: SectionItem[]  // 内部结构化条目
  source: string

  // ── CRUD ──
  /** 新增或替换 item (key 存在则替换) */
  upsert(item: SectionItem): void

  /** 删除指定 key 的 item */
  remove(key: string): void

  /** 清空整个 section */
  clear(): void

  /** 追加文本到 section 末尾 (非结构化) */
  appendRaw(text: string): void

  /** 渲染为最终标签文本 */
  render(): string
}
```

**关键设计**:
- `upsert` 等价于 **规范化追加** — key 存在就替换，不存在就追加到 subsection 末尾
- `remove` 等价于 **规范化删除** — 按 key 精确定位，不影响其他条目
- `render()` 按 section 类型输出正确的标签+内部格式
- `appendRaw` 作为逃生舱，允许追加不被 item 管理的原始文本

### 7.4 使用示例

```ts
// 1. 约束层追加一个新规则
constraint.upsert({
  key: "[Sanity]",
  content: "每次提问前必须 sanity_check 验证上下文完整性",
  subsection: "Tool Invocation Rules",
})

// 2. 删除 mood 约束 (因为 enableMood=false)
constraint.remove("[Mood]")

// 3. 追加新 instruction 文件
instructions.upsert({
  key: "src-rules/custom.md",
  content: readFile("custom.md"),
})

// 4. style 追加角色边界
style.items.push({
  key: "boundary-3",
  content: "禁止在未确认时执行 git push",
})

// 5. 渲染最终 system prompt
const system = sections.map(s => s.render()).join("\n")
// → "<constraint>...</constraint>\n<identity>...</identity>\n..."
```

### 7.5 渲染规则

每个 `TemplateSection` 的 `render()` 行为由其 `id` 决定：

| id | 渲染方式 |
|---|---|
| constraint | `## {subsection}\n- [{key}] {content}\n...` |
| identity | `<identity mode="{mode}">{content}</identity>` |
| environment | `<environment>\n{key: value}\n...</environment>` |
| instructions | `<instructions>\n<!-- source: {key} -->\n{content}\n</instructions>` |
| capabilities | `<capabilities>\n## Skills\n...\n## Tools\n...</capabilities>` |
| style | `<style><role><boundaries>...  </boundaries>...</role></style>` |
| memory | `<memory>...</memory>` |
| nudge | `<nudge>...</nudge>` |

渲染输出统一使用 `\n` 换行，无尾随空行。

## 8. 已决问题 (✅ 确认)

1. **✅ 标签嵌套深度**: 采用**双层嵌套** — `<style><role><boundaries>...</boundaries></role></style>`
2. **✅ agent.prompt 策略**: **合并模式** — provider prompt 保留在 `<identity>` 上部，agent.prompt 作为 mode 特定 instruction 追加在下部
3. **style section 动态更新机制**: 待定
4. **instruction 膨胀**: 待定
5. **兼容回退**: 待定
