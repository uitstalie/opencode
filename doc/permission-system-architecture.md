# Permission System Architecture

> 基于 [uitstalie/opencode](https://github.com/uitstalie/opencode.git) fork（`dev` 分支）的源码分析。
> 分析日期：2026-06-18

---

## 1. 文件索引 (File Index)

### 核心层 (packages/core) — V2 权限系统

| 文件 | 职责 |
|------|------|
| `packages/core/src/permission.ts` | **V2 权限主模块**：Service 层、`assert`/`ask`/`reply` 逻辑、评估引擎、RejectedError/DeniedError |
| `packages/core/src/permission/schema.ts` | V2 权限 Schema：`Effect`、`Rule`、`Ruleset` |
| `packages/core/src/permission/saved.ts` | 持久化权限（SQLite 存储）：用户选 "Always allow" 时写入 DB，跨会话保留 |
| `packages/core/src/permission/sql.ts` | 权限 SQL 表定义（Drizzle ORM） |
| `packages/core/src/v1/permission.ts` | V1 权限类型定义：`Rule`、`Ruleset`、`Request`、`Reply` 等 |
| `packages/core/src/v1/config/permission.ts` | V1 配置权限类型：`ConfigPermissionV1.Info` — opencode.json 中 `permission` 字段的类型 |
| `packages/core/src/util/wildcard.ts` | Wildcard 模式匹配引擎（核心匹配逻辑） |
| `packages/core/src/location-mutation.ts` | Location 解析 + `externalDirectoryPermission` 辅助函数 |

### 应用层 (packages/opencode) — V1 权限系统（主用户界面）

| 文件 | 职责 |
|------|------|
| `packages/opencode/src/permission/index.ts` | **V1 权限主模块**：Service 层、`fromConfig()`、`merge()`、`evaluate()`、`disabled()`、`expand()` |
| `packages/opencode/src/permission/evaluate.ts` | 仅 re-export `evaluate` |
| `packages/opencode/src/permission/arity.ts` | Bash 命令 arity 字典（用于权限 UI 显示命令前缀） |
| `packages/opencode/src/agent/agent.ts` | Agent 定义（含默认权限规则）、构建 agent 权限集 |
| `packages/opencode/src/agent/subagent-permissions.ts` | 子 agent 权限派生逻辑（task tool 使用） |
| `packages/opencode/src/config/config.ts` | 配置加载：`opencode.json` → `permission` 字段 + `OPENCODE_PERMISSION` 环境变量 |
| `packages/opencode/src/tool/external-directory.ts` | 外部目录检查：`assertExternalDirectory` / `assertExternalDirectoryEffect` |
| `packages/opencode/src/tool/tool.ts` | Tool 定义与 `Context.ask()` 接口 |
| `packages/opencode/src/session/tools.ts` | Session 工具解析：创建 tool context，将 `ctx.ask()` 连接到 `Permission.ask()` |
| `packages/opencode/src/session/prompt.ts` | Session prompt：将从 task tool 的 agent 权限合并到 session |
| `packages/opencode/src/session/llm.ts` | LLM 调用：pre-approve 已允许的工具 |
| `packages/opencode/src/session/llm/request.ts` | 请求构建：`resolveTools()` 按权限规则过滤工具列表 |
| `packages/opencode/src/session/system.ts` | 系统上下文生成：按权限过滤 skills |
| `packages/opencode/src/project/instance-context.ts` | `containsPath()` — 判断路径是否在项目边界内 |

### CLI / UI 层

| 文件 | 职责 |
|------|------|
| `packages/opencode/src/cli/cmd/run/footer.permission.tsx` | TUI 权限对话框（JSX 组件）：Allow once / Always / Reject |
| `packages/opencode/src/cli/cmd/run/permission.shared.ts` | 权限 UI 状态机（纯逻辑，可测试）：`permissionRun()`、`permissionInfo()` |
| `packages/opencode/src/server/routes/instance/httpapi/handlers/permission.ts` | HTTP API 处理函数 |
| `packages/opencode/src/server/routes/instance/httpapi/groups/permission.ts` | HTTP API 路由分组 |
| `packages/server/src/handlers/permission.ts` | 服务端 permission handler |
| `packages/server/src/groups/permission.ts` | 服务端 permission route group |

### SDK / 类型导出

| 文件 | 职责 |
|------|------|
| `packages/sdk/js/src/v2/gen/types.gen.ts` | 生成的 V2 SDK 类型（含所有 permission 相关类型） |
| `packages/sdk/js/src/gen/types.gen.ts` | 生成的 V1 SDK 类型 |
| `packages/sdk/openapi.json` | OpenAPI 规范（含 permission schema） |

### 测试

| 文件 | 职责 |
|------|------|
| `packages/core/test/permission.test.ts` | V2 权限核心测试 |
| `packages/opencode/test/permission-task.test.ts` | Task 权限测试 |
| `packages/opencode/test/cli/run/permission.shared.test.ts` | 权限 UI 状态机测试 |
| `packages/opencode/test/acp/permission.test.ts` | ACP 权限测试 |

### 文档

| 文件 | 职责 |
|------|------|
| `packages/web/src/content/docs/permissions.mdx` | 英文权限文档 |

---

## 2. 双系统架构：V1 vs V2

当前代码库存在**两套并行的权限系统**：

### V1 权限（opencode 应用层 — 主用户界面）

由 `packages/opencode/src/permission/index.ts` 定义，服务于 opencode 应用层（session 管理、工具协调、prompt 构建）。

**类型定义：**
```typescript
// packages/core/src/v1/permission.ts
Rule = {
  permission: string    // 操作类型，如 "bash", "read", "edit", "external_directory"
  pattern: string       // 资源模式，如 "src/*.ts", "pwd", "*"
  action: "allow" | "deny" | "ask"
}
Ruleset = Rule[]
```

- `permission` = 操作类型（action type）
- `pattern` = 资源模式（resource pattern）
- `action` = 权限效果

### V2 权限（core 工具层）

由 `packages/core/src/permission.ts` 定义，服务 core 层工具（bash、edit、read 等）。

**类型定义：**
```typescript
// packages/core/src/permission/schema.ts
Rule = {
  action: string        // 操作类型，如 "read", "bash", "edit"
  resource: string      // 资源标识，如 "src/index.ts", "pwd"
  effect: "allow" | "deny" | "ask"
}
Ruleset = Rule[]
```

- `action` = 操作类型（对应 V1 的 `permission`）
- `resource` = 资源标识（对应 V1 的 `pattern`）
- `effect` = 权限效果（对应 V1 的 `action`）

> **关键差异：** V1 的 `permission` + `action` = V2 的 `action` + `effect`。字段名语义正好相反，这是一个需要注意的分歧点。

---

## 3. 配置 Schema（opencode.json）

### 类型定义

```typescript
// packages/core/src/v1/config/permission.ts

// 简单形式：所有操作统一权限
type SimpleConfig = "ask" | "allow" | "deny"

// 对象形式：为每个操作类型分别配置
type ObjectConfig = {
  read?:    RuleConfig          // RuleConfig = Action | Record<string, Action>
  edit?:    RuleConfig
  glob?:    RuleConfig
  grep?:    RuleConfig
  list?:    RuleConfig
  bash?:    RuleConfig
  task?:    RuleConfig
  external_directory?: RuleConfig
  todowrite?:   Action         // 仅支持简单 action
  question?:    Action
  webfetch?:    Action
  websearch?:   Action
  lsp?:         RuleConfig
  doom_loop?:   Action
  skill?:       RuleConfig
  [key: string]: RuleConfig | undefined   // 支持自定义权限键
}
```

### 配置示例

```jsonc
{
  "permission": {
    "*": "ask",                     // 默认：所有操作需询问
    "read": {
      "*": "allow",                 // 允许读取所有文件
      "*.env": "ask",               // 但 .env 文件需询问
      "*.env.*": "ask",
      "*.env.example": "allow"      // .env.example 明确允许
    },
    "edit": {
      "*": "ask",                   // 编辑操作需询问
      "src/*.ts": "allow"           // 编辑 src 下 .ts 文件允许
    },
    "bash": "allow",                // bash 完全允许
    "external_directory": {
      "*": "ask",                   // 访问外部目录需询问
      "/tmp/opencode/*": "allow"   // 允许访问 /tmp/opencode/
    }
  }
}
```

### 配置来源优先级

1. **默认规则** — 在 `agent.ts` 中硬编码（每个 agent 有各自的默认权限）
2. **用户 opencode.json** — `cfg.permission`（项目级或用户级）
3. **CLI flag** — `OPENCODE_PERMISSION` 环境变量（JSON 格式，最高优先级覆盖）
4. **Session 级** — 通过 `session.prompt()` 注入的 tools 权限

---

## 4. 规则评估逻辑

### V1 评估 (`packages/opencode/src/permission/index.ts`)

```typescript
export function evaluate(permission: string, pattern: string, ...rulesets: Ruleset[]): Rule {
  return rulesets
    .flat()                                                           // 合并所有规则集
    .findLast((rule) =>                                               // 最后匹配的规则胜出
      Wildcard.match(permission, rule.permission) &&                  // 权限名匹配
      Wildcard.match(pattern, rule.pattern)                           // 模式匹配
    ) ?? { action: "ask", permission, pattern: "*" }                 // 默认：ask
}
```

**关键特性：**
- **`findLast`** — 后定义的规则优先级更高（最后匹配的覆盖前面的）
- **默认行为** — 无规则匹配时默认 `"ask"`（询问用户）
- **合并策略** — `Permission.merge()` 就是 `Array.flat()` 拼接，无去重

### V2 评估 (`packages/core/src/permission.ts`)

```typescript
export function evaluate(action: string, resource: string, ...rulesets: Ruleset[]): Rule {
  return rulesets
    .flat()
    .findLast((rule) => Wildcard.match(action, rule.action) && Wildcard.match(resource, rule.resource))
    ?? { action, resource: "*", effect: "ask" }
}

// 运行时评估流程（Check → Deny first, then Ask, then Allow）
const evaluateInput = function* (input: AssertInput) {
  const rules = yield* configured(input.sessionID, input.agent)
  if (denied(input, rules)) return { effect: "deny", rules }
  const all = [...rules, ...(yield* savedRules())]
  const effects = input.resources.map((resource) => evaluate(input.action, resource, all).effect)
  const effect = effects.includes("deny") ? "deny" : effects.includes("ask") ? "ask" : "allow"
  return { effect, rules: all }
}
```

**V2 评估优先级：**
1. **Configured rules** — 先检查 agent 配置的规则（包含 deny）
2. **Deny first** — 配置规则中有 deny 直接返回 deny，不检查 saved rules
3. **Saved rules** — 若配置未 deny，合并已保存的 "always allow" 规则
4. **Multi-resource** — 对每个 resource 分别评估；任一 deny → deny；否则任一 ask → ask；全部 allow → allow

### Wildcard 匹配 (`packages/core/src/util/wildcard.ts`)

```typescript
export function match(input: string, pattern: string) {
  const normalized = input.replaceAll("\\", "/")
  let escaped = pattern
    .replaceAll("\\", "/")
    .replace(/[.+^${}()|[\]\\]/g, "\\$&")   // 转义正则特殊字符
    .replace(/\*/g, ".*")                    // * → 任意字符
    .replace(/\?/g, ".")                     // ? → 单个字符

  // 特殊处理：尾随空格后跟 .* → 可选匹配
  if (escaped.endsWith(" .*")) escaped = escaped.slice(0, -3) + "( .*)?"

  return new RegExp("^" + escaped + "$", process.platform === "win32" ? "si" : "s").test(normalized)
}
```

- 所有 `\` 归一化为 `/`
- `*` 匹配任意字符序列，`?` 匹配单个字符
- 大小写敏感（Linux），Windows 下不敏感
- 路径匹配示例：`src/*.ts` 匹配 `src/index.ts`、`src/utils/helper.ts`（`src/**/*.ts` 也一样，因为 `*` 也匹配 `/`）

---

## 5. 权限类型清单 (Known Permission Keys)

| 权限键 | 类型 | 说明 | 带路径/模式 |
|--------|------|------|------------|
| `read` | `RuleConfig` | 读取文件 | ✅ 支持路径模式 |
| `edit` | `RuleConfig` | 编辑文件（含 write、apply_patch） | ✅ 支持路径模式 |
| `glob` | `RuleConfig` | 文件搜索（按文件名模式） | ✅ 支持路径模式 |
| `grep` | `RuleConfig` | 内容搜索 | ✅ 支持路径模式 |
| `list` | `RuleConfig` | 列出目录 | ✅ 支持路径模式 |
| `bash` | `RuleConfig` | 执行 shell 命令 | ✅ 命令字符串作为模式 |
| `task` | `RuleConfig` | 调用子 agent | ✅ agent 名作为模式 |
| `external_directory` | `RuleConfig` | 访问项目外目录 | ✅ 支持路径模式 |
| `todowrite` | `Action` | 写 TODO 列表 | ❌ 仅全局 |
| `question` | `Action` | 向用户提问 | ❌ 仅全局 |
| `webfetch` | `Action` | 抓取网页 | ❌ 仅全局 |
| `websearch` | `Action` | 网页搜索 | ❌ 仅全局 |
| `lsp` | `RuleConfig` | 使用 LSP | ✅ 支持路径模式 |
| `doom_loop` | `Action` | 持续运行（避免死循环） | ❌ 仅全局 |
| `skill` | `RuleConfig` | 加载 skill | ✅ skill 名作为模式 |
| 自定义键 | `RuleConfig` | 任意自定义工具/插件 | 取决于实现 |

> **注意：** `todowrite`、`question`、`webfetch`、`websearch`、`doom_loop` 的 Config 类型是 `Action`（仅全局 allow/deny/ask），实际 config 解析（`fromConfig`）会将简单 action 展开为 `{ permission: key, pattern: "*", action: value }`，运行时仍然支持模式匹配。类型标注的 `Action` 只是文档约定。

---

## 6. 完整流程追踪

### 6.1 配置加载 → 规则存储

```
opencode.json / ~/.config/opencode/opencode.json
    │
    ▼
Config.load (packages/opencode/src/config/config.ts)
    │  读取 cfg.permission
    │  合并 OPENCODE_PERMISSION 环境变量
    │  合并 cfg.tools 中的工具开关
    ▼
Agent.state (packages/opencode/src/agent/agent.ts)
    │  Permission.fromConfig(defaults)  → 默认规则
    │  Permission.fromConfig(cfg.permission) → 用户规则
    │  Permission.merge(defaults, user) → 合并
    │  各 agent (build/plan/general/explore/...) 有各自默认规则
    ▼
Agent.Info.permission: PermissionV1.Ruleset
```

### 6.2 运行时权限检查（Tools 调用）

```
LLM 调用 Tool
    │
    ▼
SessionTools.resolve() (packages/opencode/src/session/tools.ts)
    │  创建 ToolContext { ask: (req) => permission.ask({...}), ... }
    ▼
Tool.execute(args, ctx)
    │
    ├─ ctx.ask({ permission: "edit", patterns: ["src/index.ts"], ... })
    │      或 ctx.ask({ permission: "bash", patterns: ["git status"], ... })
    │
    ▼
Permission.ask() (packages/opencode/src/permission/index.ts)
    │
    ├─ 1. evaluate(permission, pattern, ruleset, approved)
    │       │  check: 配置规则 > session 规则 > 已批准的规则
    │       │  findLast 匹配
    │       │
    │       ├─ deny  → 抛出 DeniedError → LLM 收到错误消息
    │       ├─ allow → 跳过（不提示用户）
    │       └─ ask   → needsAsk = true
    │
    ├─ 2. 若 needsAsk：
    │       │  创建 PendingEntry { info, deferred }
    │       │  发布 Event.Asked 事件
    │       │  阻塞等待 Deferred
    │       │
    │       ├─ 用户选 "once"  → Deferred.succeed
    │       ├─ 用户选 "always" → Deferred.succeed + approved.push(rule)
    │       └─ 用户选 "reject" → Deferred.fail(RejectedError)
    │
    └─ 3. 返回（或抛出错误）
```

### 6.3 外部目录检查（独立流程）

```
Tool.execute() 在执行操作前
    │
    ├─ external_directory 检查（独立于主权限检查）
    │   │
    │   ▼
    │   LocationMutation.resolve(path) → 判断是否在项目目录内
    │       │  FSUtil.contains(location.directory, absolute)
    │       │  if (lexicallyInternal) → 无 externalDirectory
    │       │  if (!lexicallyInternal) → externalDirectory = { resource, save }
    │   │
    │   ▼
    │   Permission.assert({ action: "external_directory", resources: [resource], ... })
    │       │  这是一个**独立的权限检查**，有自己的 action="external_directory"
    │       │  用户可以在 opencode.json 中配置 external_directory 规则
    │       │
    │       └─ deny / ask / allow（与主权限相同的三态逻辑）
    │
    └─ 主权限检查（bash / edit / read...）
```

**external_directory 的特殊默认值：**
```typescript
// agent.ts 中的默认配置
external_directory: {
  "*": "ask",                                      // 默认询问
  ...whitelistedDirs                               // 白名单目录 allow
}

// 白名单包括：
// - Truncate.GLOB (临时输出目录)
// - path.join(Global.Path.tmp, "*")
// - skill 目录 (skillDirs)
// - reference 目录 (referenceDirs)
```

### 6.4 V2 Core Tools 的权限检查

V2 core tools（`packages/core/src/tool/bash.ts`、`edit.ts`、`read.ts` 等）使用 `PermissionV2.assert()`：

```
Tool.execute()
    │
    ├─ 1. externalDirectory 检查（同上）
    │
    ├─ 2. PermissionV2.assert({
    │       action: "edit",
    │       resources: [target.resource],
    │       sessionID, agent, source
    │   })
    │       │
    │       ▼
    │   evaluateInput() → configured → deny? → saved → ask? → allow?
    │       │
    │       ├─ deny  → DeniedError (含相关规则)
    │       ├─ allow → 直接继续
    │       └─ ask   → 发布 Asked event + 阻塞等待 reply
    │
    └─ 3. 执行实际操作
```

### 6.5 持久化权限 (Always Allow)

```
用户选 "Always allow"
    │
    ▼
Permission.reply({ reply: "always" })
    │
    ├─ 保存到 approved[] (内存，session 内有效)
    │
    └─ 保存到 PermissionSaved (SQLite，跨 session 有效)
        │  仅保存 request.save 中的资源模式
        │  下次 V2 assert 时通过 savedRules() 加载
        ▼
        PermissionSaved.add({ projectID, action, resources })
```

### 6.6 Tool 过滤（按权限禁用工具）

```
LLM 请求构建时
    │
    ▼
resolveTools() (packages/opencode/src/session/llm/request.ts)
    │  Permission.disabled(toolNames, ruleset)
    │       │  检查 pattern === "*" && action === "deny"
    │       ▼
    │  返回完全禁用的工具名 Set
    │
    ▼
过滤后的工具列表发送给 LLM（被 deny 的工具对 LLM 不可见）
```

---

## 7. 路径作用域 (Path Scoping) — 当前限制

### 7.1 现有的路径相关能力

| 能力 | 实现方式 | 粒度 |
|------|----------|------|
| 项目内外边界 | `external_directory` 权限 + `containsPath()` | ❌ 仅项目边界（worktree/directory） |
| 文件级读写控制 | `read`/`edit` 权限的 `pattern` 字段 | ✅ 支持 glob 模式 |
| 外部目录白名单 | `external_directory` 配置中的路径模式 | ✅ 支持 glob 模式 |
| 环境变量保护 | 默认 `read: { "*.env": "ask" }` | ✅ 通过 pattern 匹配 |

### 7.2 关键缺失：细粒度路径作用域

当前**没有**以下能力：

1. **没有 `path_scope` 或 `workspace_scope` 字段** — 权限规则（V1 `permission` 或 V2 `action`）只有 `pattern`/`resource` 一个维度，无法同时表达"限制某操作在特定目录内"的额外约束。

2. **路径作用域是隐式的**：
   - `external_directory` 只做"项目内 vs 项目外"的二元判断
   - 项目内的路径控制完全依赖 `pattern` 字段，但 `pattern` 同时也是资源标识，语义不够明确
   - 例如：你可以配置 `edit: { "src/*.ts": "allow" }`，但这不是"限制 edit 到 src/"作用域，而是"允许匹配 src/*.ts 的文件"

3. **bash 的路径作用域缺失**：
   - bash 权限的 `pattern` 是命令字符串（如 `"git status"`），不是路径
   - 没有机制限制 bash 只能操作特定目录
   - bash 的外部目录访问只通过 `external_directory` 检查 working directory

4. **subagent 权限继承不完整**：
   - `deriveSubagentSessionPermission()` 只继承 `external_directory` 和 `deny` 规则
   - 不继承父 session 的路径作用域限制

### 7.3 `external_directory` 的本质

`external_directory` 是一个**独立权限类型**，不是通用路径作用域机制：

```
Tool 内部流程：
1. 先检查 external_directory（独立 assert）
2. 再检查工具自身权限（edit/bash/read...的 assert）
```

这意味着：
- `external_directory` 和工具权限是**两个独立的检查点**
- 无法在一条规则中同时表达"允许 read 但限制在 some/path/"
- 当前架构不支持类似 `{ action: "read", resource: "*", scope: "src/" }` 的语义

### 7.4 相关代码示例

**external_directory 的调用模式：**
```typescript
// 每个需要文件访问的工具都独立调用：
// - packages/opencode/src/tool/edit.ts
// - packages/opencode/src/tool/read.ts
// - packages/opencode/src/tool/write.ts
// - packages/opencode/src/tool/glob.ts
// - packages/opencode/src/tool/grep.ts
// - packages/opencode/src/tool/lsp.ts
// - packages/opencode/src/tool/apply_patch.ts
// - packages/opencode/src/tool/shell.ts
// - packages/core/src/tool/bash.ts
// - packages/core/src/tool/edit.ts
// - packages/core/src/tool/write.ts
// - packages/core/src/tool/apply-patch.ts

yield* assertExternalDirectoryEffect(ctx, filepath, { kind: "file" })
```

**`containsPath` 的边界判定：**
```typescript
// packages/opencode/src/project/instance-context.ts
export function containsPath(filepath: string, ctx: InstanceContext): boolean {
  if (FSUtil.contains(ctx.directory, filepath)) return true    // 在工作目录内
  if (ctx.worktree === "/") return false                        // 非 git 项目跳过
  return FSUtil.contains(ctx.worktree, filepath)                // 在 worktree 内
}
```

---

## 8. 当前限制总结 (Current Limitations)

### 架构层面

| 限制 | 影响 |
|------|------|
| **双系统并行（V1 + V2）** | 字段名语义不同（`permission`/`action` vs `action`/`effect`），增加理解和维护成本 |
| **无通用路径作用域** | 无法在一条规则中限制工具的操作目录范围；只能通过 `external_directory` 做项目边界检查 |
| **external_directory 与工具权限分离** | 每次新增文件操作工具需要**手动添加**两个检查调用，容易遗漏 |
| **pattern 语义过载** | V1 的 `pattern` 既是资源标识（文件路径）也是命令标识（bash command），对不同权限类型意义不同 |
| **无递归继承** | 子 agent 不继承父 session 的路径作用域限制 |

### 功能层面

| 限制 | 说明 |
|------|------|
| 无法定义目录级作用域 | 不能表达 "允许 bash 但只能操作 /tmp/mydir/" |
| 无法限制网络访问域名 | `webfetch`/`websearch` 只有全局 allow/deny/ask |
| 无法限制 shell 命令 | bash 权限按命令字符串匹配，没有类似 `dangerous_commands` 的分类 |
| 无时间窗口 | 没有 "允许接下来 5 分钟的 bash" 这样的临时授权 |
| 无操作计数限制 | 没有 "允许 read 最多 10 次" |

---

## 9. 关键代码片段速查

### 评估入口
```typescript
// packages/opencode/src/permission/index.ts:39
export function evaluate(permission: string, pattern: string, ...rulesets: PermissionV1.Ruleset[]): PermissionV1.Rule
```

### 配置转换
```typescript
// packages/opencode/src/permission/index.ts:197
export function fromConfig(permission: ConfigPermissionV1.Info): PermissionV1.Rule[]
```

### 工具 ask 入口
```typescript
// packages/opencode/src/session/tools.ts:63
ask: (req) => permission.ask({
  ...req,
  sessionID: input.session.id,
  tool: { messageID, callID },
  ruleset: Permission.merge(input.agent.permission, input.session.permission ?? []),
})
```

### 外部目录检查
```typescript
// packages/opencode/src/tool/external-directory.ts:15
export const assertExternalDirectoryEffect = Effect.fn("Tool.assertExternalDirectory")(...)
```

### V2 权限 assert
```typescript
// packages/core/src/permission.ts:223
const assert = EffectRuntime.fn("PermissionV2.assert")((input: AssertInput) => ...)
```

### 持久化存储
```typescript
// packages/core/src/permission/saved.ts
export const Info = Schema.Struct({
  id: ID,
  projectID: ProjectV2.ID,
  action: Schema.String,
  resource: Schema.String,
})
```

---

## 10. 修订记录

| 日期 | 修订内容 |
|------|----------|
| 2026-06-18 | 初版：基于 `dev` 分支源码分析 |
