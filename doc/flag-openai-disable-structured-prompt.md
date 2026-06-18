# Flag: OPENCODE_DISABLE_STRUCTURED_PROMPT

> 版本: draft-1 | 日期: 2026-06-16 | 所属: system prompt 模板化项目

## 1. 目的

结构化 system prompt 的**一键回退开关**。当 `<section>` 标签模板导致 LLM 行为异常时，设置此 flag 即可恢复旧的 `join("\n")` 扁平拼接逻辑。

## 2. 设计

### Flag 定义

**位置**: `packages/core/src/flag/flag.ts` → `Flag` 对象

```ts
// 静态 bool（启动时一次性求值），遵循 OPENCODE_DISABLE_* 约定
OPENCODE_DISABLE_STRUCTURED_PROMPT: truthy("OPENCODE_DISABLE_STRUCTURED_PROMPT"),
```

**语义**:
- `false` (默认) → 启用结构化模板，system prompt 按 8 个 `<section>` 标签组织
- `true` → 禁用，恢复旧版 `join("\n")` 扁平拼接

### 生命周期

| 阶段 | 行为 |
|---|---|
| 定义 | `Flag` 对象新增一行静态字段，无运行时开销 |
| 消费 | `request.ts:56-66` 和 `prompt.ts:1327-1333` 各读取一次 |
| 未来移除 | 结构化模板稳定后（估计 3-6 个月），移除此 flag |

### 读取方式

```ts
import { Flag } from "@opencode-ai/core/flag/flag"

if (Flag.OPENCODE_DISABLE_STRUCTURED_PROMPT) {
  // 旧逻辑: join("\n") 扁平拼接
} else {
  // 新逻辑: template.render() 结构化输出
}
```

## 3. 接入点

### 3.1 request.ts (主拼接点)

```ts
// 当前: line 56-66
const system = [[
  ...(agent.prompt ? [agent.prompt] : SystemPrompt.provider(model)),
  ...input.system,
  ...(user.system ? [user.system] : []),
]].filter(x => x).join("\n")

// 改为:
const system = Flag.OPENCODE_DISABLE_STRUCTURED_PROMPT
  ? [[...provider, ...input.system, ...user].filter(x => x).join("\n")]
  : [buildStructuredSystemPrompt(input)]  // ← template.render()
```

### 3.2 prompt.ts (上游组装)

```ts
// 当前: line 1327-1333
const system = [...env, ...instructions, ...(skills ? [skills] : [])]

// 改为:
const system = Flag.OPENCODE_DISABLE_STRUCTURED_PROMPT
  ? [...env, ...instructions, ...(skills ? [skills] : [])]
  : buildTemplateSections({ env, instructions, skills })  // ← 返回 Section[]
```

### 3.3 plugin 层 (compose.ts)

```ts
// buildConstraintLayer(), buildStableSystemSections() 等
// 当前返回: string | string[]
// 结构化时改为返回: SectionItem[]
// 通过 Flag 控制是否包装标签
```

### 3.4 对 memory 的影响

结构化模板启用后，memory 工具指令从分散在全文中变成收敛到 `<constraint>` 和 `<memory>` 两个 section：

```
旧: "[Memory] 决策前必查..." 混在 constraint layer 大文本中
新: <constraint>
    ## Tool Invocation Rules
    - [Memory] 决策前必查 memory_read...
    ## Concrete Discipline
    - memory 宁缺毋滥...
    </constraint>
    <memory>
    ## Dreaming Insights
    - Dreaming 已提取...
    </memory>
```

如果结构化模板导致 memory 工具调用异常（如 LLM 忽略规则），设置 `OPENCODE_DISABLE_STRUCTURED_PROMPT=true` 即可立刻回退。

## 4. 回退验证清单

设置 `OPENCODE_DISABLE_STRUCTURED_PROMPT=true` 后必须确认：

- [ ] system prompt 格式退化为旧版扁平拼接
- [ ] memory_read 在决策前依然被调用
- [ ] compact_check 每轮依然被触发
- [ ] CONSTRAINT NUDGE 正常工作
- [ ] 所有 plugin hook 行为不变

## 5. 与其他 Flag 的关系

| Flag | 影响 |
|---|---|
| `OPENCODE_EXPERIMENTAL` | **不** cascade 到此 flag。结构化模板是独立功能，不跟随 experimental |
| `OPENCODE_EXPERIMENTAL_NATIVE_LLM` | 正交。native LLM 与 prompt 模板化无关 |
| `OPENCODE_DISABLE_AUTOCOMPACT` | 无影响。compact_check 行为不变 |

## 6. 清理计划

| 时间点 | 操作 |
|---|---|
| Flag 定义当天 | 写入 `flag.ts`，默认 false |
| 结构化模板上线后 1 个月 | 监控回退频率。如 0 次回退 → 进入移除倒计时 |
| 上线后 3 个月 | 移除 Flag 读取，删除旧 `join("\n")` 分支 |
| 上线后 6 个月 | 删除 Flag 定义本身 |
