# Permission Scope 增强设计

> 版本: draft-1 | 日期: 2026-06-18 | 基于: [uitstalie/opencode](https://github.com/uitstalie/opencode.git) fork

## 1. 目标

给权限规则增加 `scope`（路径作用域）字段，使权限可以限定在特定目录范围内生效，解决当前"`bash: allow` 全局生效，无法对其在特定目录"的问题。

**核心约束**：
1. `scope` 替换 `external_directory` 的角色，两者不并存
2. 向后兼容 — 现有无 scope 的配置照旧工作
3. 简单 glob 语法，无排除/否定

## 2. 类型变更

### 2.1 Rule 类型

```typescript
// packages/core/src/v1/permission.ts

// 旧
interface Rule {
  permission: string     // "bash" | "read" | "edit" | ...
  pattern: string        // "git *" | "src/*.ts" | "*"
  action: "allow" | "deny" | "ask"
}

// 新
interface Rule {
  permission: string
  pattern: string
  action: "allow" | "deny" | "ask"
  scope?: string         // NEW: 路径作用域，glob 模式
}
```

### 2.2 Config 类型

```typescript
// packages/core/src/v1/config/permission.ts

// 旧
type Action = "allow" | "deny" | "ask"
type RuleConfig = Action | Record<string, Action>

// 新
type RuleDetail = Action | { action: Action; scope?: string }
type RuleConfig = RuleDetail | Record<string, RuleDetail>
```

**兼容性**：
- `"read": "allow"` → 仍有效（RuleDetail = Action）
- `"read": { "*": "allow" }` → 仍有效（Record<string, Action>，Action 是 RuleDetail 的子集）
- `"bash": { "*": { "action": "allow", "scope": "~/proj/**" } }` → 新格式

### 2.3 evaluate() 签名变更

```typescript
// packages/opencode/src/permission/index.ts

export function evaluate(
  permission: string,
  pattern: string,
  opScope: string | undefined,  // NEW: 操作的路径上下文
  ...rulesets: Ruleset[]
): Rule
```

**匹配逻辑**：
```
规则生效条件 = 
  Wildcard.match(permission, rule.permission)  // 权限类型匹配
  AND Wildcard.match(pattern, rule.pattern)     // 资源模式匹配
  AND (rule.scope 为空                            // 规则无 scope 限制
       OR opScope 为空                            // 操作无 scope 信息
       OR Wildcard.match(opScope, rule.scope))    // 路径作用域匹配
```

- `rule.scope` 为空 → 规则适用于所有路径（向后兼容）
- `opScope` 为空 → 无法判断路径，规则也参与匹配（保守策略）
- 两者都存在 → 必须 Wildcard 匹配成功

## 3. 各工具的 scope 计算

| 权限类型 | opScope 来源 | 示例 |
|----------|-------------|------|
| `bash` | 工作目录 (cwd) | `"/home/user/project-a"` |
| `read` | 目标文件路径 | `"/home/user/project-a/src/index.ts"` |
| `edit` | 目标文件路径 | `"/home/user/project-a/src/foo.ts"` |
| `write` | 目标文件路径 | `"/home/user/project-a/output/data.json"` |
| `glob` | 搜索目录 | `"/home/user/project-a/src"` |
| `grep` | 搜索目录 | `"/home/user/project-a/src"` |
| `task` | 子 agent 的上下文目录 | `"/home/user/project-a"` |
| `lsp` | 目标文件路径 | `"/home/user/project-a/src/index.ts"` |
| `external_directory` | 外部路径（后续废弃） | `"/tmp/something"` |

## 4. 配置示例

### 4.1 最简场景：项目内自由，项目外需确认

```jsonc
{
  "permission": {
    "*": "ask",                              // 全局默认：ask

    // 项目内全面放开
    "bash": {
      "*": { "action": "allow", "scope": "~/projects/my-app/**" }
    },
    "read": {
      "*": { "action": "allow", "scope": "~/projects/my-app/**" }
    },
    "edit": {
      "*": { "action": "allow", "scope": "~/projects/my-app/src/**" }
    },
    // 没 scope 的规则 fall through 到全局 "*": "ask"

    "webfetch": "allow",
    "task": "allow"
  }
}
```

### 4.2 多项目场景

```jsonc
{
  "permission": {
    "*": "ask",
    "bash": {
      "*": { "action": "allow", "scope": "~/projects/{project-a,project-b}/**" },
      "*": "ask"  // 其他目录的 bash 需要确认
    },
    "read": "allow"  // read 全局允许（无 scope = 传统行为）
  }
}
```

### 4.3 混合使用（有无 scope 共存）

```jsonc
{
  "permission": {
    "read": {
      "*": "allow",
      "*.env": { "action": "ask", "scope": "**" },  // .env 文件在任意目录都 ask
      ".secrets/*": "deny"                           // .secrets/ 在任何目录都 deny
    },
    "edit": {
      "*.ts": { "action": "allow", "scope": "src/**" },  // src/ 下 .ts 直接允许
      "*.ts": "ask"                                       // 其他目录的 .ts 需确认
    }
  }
}
```

### 4.4 纯 deny 场景：禁止在特定目录执行危险操作

```jsonc
{
  "permission": {
    "bash": {
      "rm *": {
        "action": "deny",
        "scope": "~/important-data/**"  // 保护重要数据目录
      },
      "*": "allow"
    }
  }
}
```

## 5. external_directory 迁出策略

`external_directory` 目前是独立权限类型，角色由 `scope` 接管：

### 迁移映射

| external_directory 配置 | 等效 scope 配置 |
|--------------------------|-----------------|
| `"external_directory": { "*": "deny" }` | 各文件操作工具添加带 `scope` 的 deny 规则 |
| `"external_directory": { "*": "ask", "/tmp/**": "allow" }` | 同上，scope 规则中 `/tmp/**` 可特殊处理 |

### 迁移步骤

1. **Phase 1（本阶段）**：`fromConfig()` 中，遇到 `external_directory` 配置时，自动展开为各工具（read/edit/write/glob/grep/bash）的 scoped deny/ask 规则
2. **Phase 2（后续）**：文档标注 `external_directory` 为 deprecated
3. **Phase 3（远期）**：移除 `external_directory` 独立类型

### fromConfig() 自动迁移逻辑

```typescript
// pseudo-code
if (config.external_directory) {
  const fileTools = ["read", "edit", "write", "glob", "grep", "bash", "lsp"]
  for (const rule of expandExternalDirRules(config.external_directory)) {
    for (const tool of fileTools) {
      // external_directory deny * → tool deny * with scope=!worktree
      // Actually, we can't express negation in glob. Instead:
      // external_directory deny means: operations outside worktree are denied
      // This is equivalent to: all operations require scope check
      // But we'll handle this as a separate evaluation dimension initially
      rules.push({
        permission: tool,
        pattern: "*",
        action: rule.action,
        scope: rule.scope  // from external_directory rules
      })
    }
  }
}
```

Wait — `external_directory: { "*": "deny" }` 语义是"项目外 deny"，但 glob 只支持正向匹配，不支持 `!project/**`。这需要一个转换策略。

**转换策略**：
- `external_directory` 的规则本质上是"对项目外路径施加的限制"
- 直接转换不可行（因为 glob 无否定），所以暂时保留 `external_directory` 作为 `scope` 的补充语义
- 当 `scope` 存在时：限制规则在 scope 内生效
- 当 `external_directory` 存在时：继续对超出 scope 的操作施加额外限制

**实际上，scope 和 external_directory 是正交的**：
- scope 控制"规则在哪里适用"
- external_directory 控制"超出项目边界时的行为"

所以设计调整：**scope 不替换 external_directory，而是专注解决"规则范围限定"问题**。

但用户之前选择了"替换"，让我重新考虑...

实际上，用户的场景可以这样描述：
- 原 external_directory 用法：`"external_directory": { "*": "ask", "/tmp/**": "allow" }`
- 这个意思是：项目内自动允许（通过 scope=project），项目外一般 ask，但 /tmp/ 例外 allow

如果 scope 单纯地限定规则范围，那 `external_directory` 的语义也可以被实现：不做全局的 external_directory 检查，而是把"项目外"处理为"没有 scope 匹配的规则 → fall through 到全局默认"。

例如：
```jsonc
{
  "bash": {
    "*": { "action": "allow", "scope": "~/projects/**" },  // 项目内 bash allow
    "*": "ask"                                               // 非项目内 bash ask
  }
}
```

这样就达到了 external_directory 的效果，且更灵活。

但这里有个问题：`pattern` 也是 `*`，两条规则除了 scope 不同，其他一样。findLast 会匹配到哪条？按照 `findLast` 语义，第二条（`"*": "ask"`）会先被检查。如果 scope 不匹配第一条，第一条被跳过，第二条（无 scope）匹配 → ask。

这样逻辑是对的！所以 external_directory 确实可以被 scope 替换。

不过，如果用户想要更简单的配置（只需要"项目内外区分"而不想为每个工具都写两行），我可以提供一个快捷语法。但这不是第一阶段要做的事。

OK，回到设计。scope 替换 external_directory 是可行的。转换策略就是：
- 用户把 external_directory 规则转换为各工具带 scope 的规则
- fromConfig() 自动做这个转换

不，等一下。external_directory 的默认规则 `"*": "ask"` 和 scope 的 `fall through` 语义冲突。在 scope 设计中：
- 操作路径不在任何 scope 规则内 → 自动 fall through 到无 scope 的规则或全局默认
- 全局默认通常是 `"ask"`

所以 external_directory 的默认行为（项目外 ask）已经被 scope 的 fall through 语义覆盖了。不需要额外处理。用户只需要为"项目内"写 allow 规则即可。

总结：external_directory 可以安全废弃。现有使用 external_directory 的用户需要迁移到 scope，我们可以提供迁移文档并在 fromConfig 中打印 deprecation 警告。

## 6. 实施计划

### Phase 1: 类型定义

| 文件 | 变更 |
|------|------|
| `packages/core/src/v1/permission.ts` | Rule 增加 `scope?: string` |
| `packages/core/src/v1/config/permission.ts` | 增加 `RuleDetail` 类型，`RuleConfig` 扩展为 `RuleDetail \| Record<string, RuleDetail>` |
| `packages/opencode/src/permission/index.ts` | `evaluate()` 增加 `opScope` 参数；`fromConfig()` 支持解析新格式 |

### Phase 2: 工具层接入

| 文件 | 变更 |
|------|------|
| `packages/opencode/src/session/tools.ts` | `ctx.ask()` 支持传递 scope |
| `packages/opencode/src/tool/bash.ts` \| shell 相关 | 传递 cwd 作为 opScope |
| `packages/opencode/src/tool/read.ts` | 传递 filepath 作为 opScope |
| `packages/opencode/src/tool/edit.ts` | 同上 |
| `packages/opencode/src/tool/write.ts` | 同上 |
| `packages/opencode/src/tool/glob.ts` | 传递 searchDir 作为 opScope |
| `packages/opencode/src/tool/grep.ts` | 同上 |
| `packages/opencode/src/tool/lsp.ts` | 传递 filepath |
| `packages/opencode/src/tool/task.ts` | 传递 agent context path |
| `packages/core/src/tool/bash.ts` | V2 版本同理 |
| `packages/core/src/tool/edit.ts` | 同上 |
| `packages/core/src/tool/write.ts` | 同上 |

### Phase 3: 兼容性

| 文件 | 变更 |
|------|------|
| `packages/opencode/src/permission/index.ts` | `fromConfig()` 检测 `external_directory`，转换为 scoped 规则 + 打印 deprecation 警告 |
| `packages/opencode/src/tool/external-directory.ts` | 标记 deprecated，保留 stub 但行为委托给 scope 检查 |

### Phase 4: 验证

- 编译通过
- 现有单元测试不退化
- 新增 scope 匹配测试用例

## 7. 已决问题

| # | 问题 | 决策 |
|---|------|------|
| 1 | scope 语义 | 限制规则生效范围（不匹配 scope 则规则被跳过） |
| 2 | scope vs external_directory | scope 替换 external_directory |
| 3 | scope 语法 | 简单 glob（无排除/否定） |
| 4 | 向后兼容 | 无 scope 的规则 = 全局适用，照旧 |

## 8. 待决问题

| # | 问题 | 暂定方案 |
|---|------|----------|
| 1 | glob 是否支持 `~` 展开 | 是，evaluate() 中展开 `~` 为 `$HOME` |
| 2 | glob 是否支持 `{a,b}` brace 展开 | 待定（Wildcard.match 目前不支持，需扩展） |
| 3 | subagent 是否继承 scope | 待定（当前 subagent 不继承，可能保持以便子 agent 在隔离环境中运行） |
