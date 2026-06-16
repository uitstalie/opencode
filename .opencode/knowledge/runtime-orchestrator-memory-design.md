# Runtime Orchestrator — Memory 模块设计

> 来源：`plugins/runtime-orchestrator/memory/` + `index.ts` 工具层
> 日期：2026-06-15

## 整体架构

```
index.ts (8 个工具注册)
 ├── memory_migrate   memory_record    memory_candidates
 ├── memory_distill   memory_promote   memory_review
 ├── memory_read      dreaming_compress
 │
 ├── db.ts       (399行) — SQLite CRUD
 ├── migrate.ts  (211行) — 文件→数据库迁移
 ├── schema.sql   (62行) — 表定义
 └── turn-buffer.ts (11行) — readFileOrEmpty 安全读取
```

## 数据模型（schema.sql）

### 设计原则

**保留 LLM 灵活性，不过度结构化。** category / target / tags / confidence 都是自由文本。

### 三张表

#### global_memory — 全局跨项目记忆

| 字段     | 类型     | 说明                                                |
| -------- | -------- | --------------------------------------------------- |
| category | TEXT     | 自由文本分类：preferences / constraints / patterns / style 或任意 |
| content  | TEXT     | 1-2 句原文                                          |
| tags     | TEXT     | 空格分隔标签：`#confirmed #preference`               |
| confidence | TEXT   | `#confirmed` / `#likely` / `#observed` 或任意       |
| source   | TEXT     | manual / dreaming / import                          |
| metadata | TEXT     | JSON 扩展字段                                       |

**唯一约束**：`UNIQUE(category, content)`

#### project_memory — 项目记忆

| 字段   | 类型 | 说明                                                |
| ------ | ---- | --------------------------------------------------- |
| target | TEXT | progress / TODO / tech / conclusion 或任意           |
| content | TEXT | 1-2 句原文                                         |
| tags   | TEXT | 空格分隔标签                                        |
| metadata | TEXT | JSON 扩展字段                                     |

**唯一约束**：`UNIQUE(target, content)`

#### dreaming_projects — 跨会话模式数据

| 字段             | 类型    | 说明                                      |
| ---------------- | ------- | ----------------------------------------- |
| project_id       | TEXT    | 项目标识（hash 或名称）                   |
| category         | TEXT    | habits / rules / patterns 或任意            |
| content          | TEXT    | 原始观察文本                              |
| confidence       | TEXT    | `#observed` / `#likely` / `#confirmed`    |
| sessions_analyzed | INTEGER | 已分析 session 数                        |
| metadata         | TEXT    | JSON 扩展                                 |

**唯一约束**：`UNIQUE(project_id, category, content)`

### 数据库路径

| 层级   | 路径                               |
| ------ | ---------------------------------- |
| 全局   | `~/.config/opencode/memory.db`     |
| 项目   | `{projDir}/.opencode/memory/memory.db` |

## db.ts — 数据库操作层

### 兼容性

兼容 Bun SQLite (`bun:sqlite`) 和 Node SQLite (`node:sqlite`) 两种运行时。

### CRUD 接口

每张表一套完整的 insert / query / delete + 统计函数：

- `initGlobalDb()` / `initProjectDb()` — 创建表 + 索引 + WAL 模式
- `insertGlobalMemory(db, entry)` — `UNIQUE` 冲突返回 `{ ok: false, error: "duplicate" }`
- `queryGlobalMemory(db, { category?, confidence?, source?, search?, limit? })` — 条件链拼接，LIKE 搜索
- `deleteGlobalMemory(db, { category?, content?, id? })`
- `insertProjectMemory` / `queryProjectMemory` / `deleteProjectMemory` — 对称接口
- `insertDreaming` / `queryDreaming` / `deleteDreaming` — dreaming 专用
- `getGlobalStats(db)` / `getProjectStats(db)` — 按 category/target 分组计数

## migrate.ts — 文件→数据库迁移

### 迁移源

| 函数                   | 源文件                                                        | 目标表              |
| ---------------------- | ------------------------------------------------------------- | ------------------- |
| `migrateGlobalMemory()` | `~/.config/opencode/memory/{preferences,constraints,patterns,style}/MEMORY.md` | global_memory       |
| `migrateProjectMemory()` | `{projDir}/.opencode/memory/{progress,TODO,tech,conclusion}.md` | project_memory      |
| `migrateDreaming()`    | `~/.config/opencode/memory/dreaming/projects/{hash}/{habits,rules,patterns}.md` | dreaming_projects   |

### 解析规则

- 从 `.md` 文件提取 `- ` 开头的 bullets
- 行末 `#tag` 被提取到 tags 字段
- dreaming 行末 `#observed`/`#likely`/`#confirmed` 被提取到 confidence 字段
- 重复条目静默跳过（`error: "duplicate"`）
- 原始文件保留为备份

## 工具层 — 8 个 Tool

### 成熟度提升链

```
┌──────────┐    ┌─────────────┐    ┌──────────────┐    ┌──────────┐
│ raw 观察  │ →  │  candidates  │ →  │  distilled    │ →  │  rules   │
│ (数据库)  │    │  (候选文件)   │    │  (提炼文件)    │    │  (正式规则)│
└──────────┘    └─────────────┘    └──────────────┘    └──────────┘
     ▲               ▲                  ▲                  ▲
     │               │                  │                  │
 memory_record  memory_candidates  memory_distill    memory_promote
 (INSERT INTO)  (追加 candidates.md) (candidates+raw → distilled.md) (distilled → rules/*.md)
```

### 各工具职责

| 工具               | 数据源/目标        | 关键行为                                                      |
| ------------------ | ------------------ | ------------------------------------------------------------- |
| `memory_migrate`   | 文件 → SQLite      | `dryRun=true` 仅预览；执行后原始文件保留为备份                 |
| `memory_record`    | SQLite INSERT      | 仅 1-2 句稳定结论，500 字上限；UNIQUE 冲突返回"重复"          |
| `memory_candidates` | `candidates.md`   | 不确定的观察先存为候选文件；含 `rationale` 可选字段            |
| `memory_distill`   | 文件 → `distilled.md` | 从 raw + candidates 提炼；默认 `dryRun=true` 预览             |
| `memory_promote`   | `distilled.md` → `rules/*.md` | 高影响操作，需确认；按章节匹配来源                            |
| `memory_review`    | SQLite 统计        | 写入前查重；按 scope/layer 展示统计                           |
| `memory_read`      | SQLite SELECT      | 支持 scope/user/project + layer + search 多维度查询            |
| `dreaming_compress` | dreaming 文件     | 去重 + 置信度升级 + 与 rules 交叉验证；默认 dryRun             |

### 关键参数约定

- `scope`: `"project"` | `"user"`
- `layer`: `"raw"` | `"distilled"` | `"candidates"` | `"dreaming"`，默认 `"raw"`
- `target` (项目): `"progress"` | `"TODO"` | `"tech"` | `"conclusion"`
- `name` (全局): `"preferences"` | `"constraints"` | `"patterns"` | `"style"`

## 双模式并存（过渡期）

当前架构同时维护数据库和文件两套存储：

```
数据库 (SQLite)                 文件 (.md)
─────────────────              ────────────────
memory_record ▸ INSERT         memory_candidates ▸ candidates.md
memory_read   ▸ SELECT         memory_distill  ▸ candidates.md → distilled.md
memory_review ▸ 统计            memory_promote  ▸ distilled.md → rules/*.md
                               dreaming_compress ▸ dreaming 文件
```

文件模式在以下工具中仍然活跃：`memory_candidates`、`memory_distill`、`memory_promote`、`dreaming_compress`。

## 集成方式

1. **工具注册**：所有 8 个工具直接注册在 `index.ts` 的 `tool: { ... }` 对象中，LLM 通过 function call 触发
2. **Session hook 注入**（`hooks/session.ts`）：每轮系统提示末尾注入 `[CONSTRAINT NUDGE] memory_read(决策前) → compact_check`
3. **Event hook**（`hooks/event.ts`）：处理 mood 事件和 todo 变更，不直接参与 memory 读写

## 已知设计特点与待改进

- ✅ 数据库 + 文件并存，过渡期不丢数据
- ✅ LLM 友好：字段自由文本，不枚举约束
- ✅ UNIQUE 约束 + `memory_review` 查重双重防重复
- ⚠️ 成熟度链节点间有断点：`memory_distill` 从文件读取，`memory_record` 写入数据库，输入源不完全对齐
- ⚠️ `memory_candidates` 写入文件而非数据库，不参与数据库层的查重统计
- ⚠️ `turn-buffer.ts` 目前仅 11 行 `readFileOrEmpty`，被 `memory_distill` 调用，**按轮缓冲**的设计意图尚未落地
