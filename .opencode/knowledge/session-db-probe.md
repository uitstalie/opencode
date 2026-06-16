# Opencode Session 数据库探索报告

> 更新时间：2026-06-15

---

## 1. 实际路径 — 找到的所有 SQLite 数据库文件

| 文件路径 | 大小 | 说明 |
|---------|------|------|
| `/home/uitstalie/.local/share/opencode/opencode.db` | **93 MB** | **主 session 数据库**（WAL 模式，另有 `.db-wal` 4MB + `.db-shm` 32KB） |
| `/home/uitstalie/.config/opencode/memory.db` | 28 KB | 全局用户 memory 数据库（非 session） |
| `/mnt/d/dqc/fake_opencode/.opencode/memory/memory.db` | 36 KB | 当前项目 memory 数据库（非 session） |

### opencode.db 内现有数据规模

| 表名 | 行数 |
|------|------|
| `session` | 98 |
| `session_message` | 2 |
| `message` | 8,152 |
| `part` | 35,517 |
| `session_input` | 0 |
| `session_context_epoch` | 0 |

---

## 2. 源码路径 — 数据库路径定义

### 2.1 路径解析链

```
database.path()                         # packages/core/src/database/database.ts:43-55
  → Global.Path.data + "/opencode.db"   # packages/core/src/global.ts:11
    → xdgData! + "/opencode"            # 即 ~/.local/share/opencode
```

### 2.2 关键源文件

| 文件 | 作用 |
|------|------|
| `source-code/opencode/packages/core/src/global.ts` | 定义 `Path.data` = `xdgData + "/opencode"` |
| `source-code/opencode/packages/core/src/database/database.ts` | `path()` 函数返回 `opencode.db` 完整路径 |
| `source-code/opencode/packages/core/src/database/path.ts` | Drizzle 自定义列类型（absolute、directory、path 列） |
| `source-code/opencode/packages/core/src/session/sql.ts` | Session 相关 Drizzle ORM 表定义 |
| `source-code/opencode/packages/core/src/database/schema.gen.ts` | 完整 SQL CREATE TABLE 迁移语句 |
| `source-code/opencode/packages/core/src/database/schema.sql.ts` | `Timestamps`（time_created/time_updated）通用列定义 |

### 2.3 路径确定的详细逻辑 (`database.path()`)

```ts
// database/database.ts:43-55
export function path() {
  // 1. 环境变量 OPENCODE_DB 优先（支持 :memory: 或绝对路径或相对路径）
  if (Flag.OPENCODE_DB) {
    if (Flag.OPENCODE_DB === ":memory:" || isAbsolute(Flag.OPENCODE_DB)) return Flag.OPENCODE_DB
    return join(Global.Path.data, Flag.OPENCODE_DB)
  }
  // 2. 标准 channel (latest/beta/prod) 或禁用 channel DB 时 → opencode.db
  if (
    ["latest", "beta", "prod"].includes(InstallationChannel) ||
    process.env.OPENCODE_DISABLE_CHANNEL_DB === "1" ||
    process.env.OPENCODE_DISABLE_CHANNEL_DB === "true"
  )
    return join(Global.Path.data, "opencode.db")
  // 3. 其他 channel → opencode-<channel>.db
  return join(Global.Path.data, `opencode-${channel}.db`)
}
```

---

## 3. 表结构

### 3.1 完整表列表 (21 张表)

```
__drizzle_migrations   account               account_state
control_account        credential            data_migration
event                  event_sequence        message
migration              part                  permission
project                project_directory     session
session_context_epoch  session_input         session_message
session_share          sqlite_sequence       todo
workspace
```

### 3.2 Drizzle ORM 表定义（源码 `session/sql.ts`）

#### `session` 表（第 21-65 行）

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | text PK | Session ID |
| `project_id` | text FK → project.id | 所属项目 |
| `workspace_id` | text | 工作区 ID |
| `parent_id` | text | 父 Session ID（fork 链） |
| `slug` | text NOT NULL | 短标识符 |
| `directory` | text NOT NULL | 项目目录路径 |
| `path` | text | 子目录相对路径 |
| `title` | text NOT NULL | 标题 |
| `version` | text NOT NULL | Session 版本（如 "v2"） |
| `share_url` | text | 分享链接 |
| `summary_additions` | integer | 摘要：新增行数 |
| `summary_deletions` | integer | 摘要：删除行数 |
| `summary_files` | integer | 摘要：文件数 |
| `summary_diffs` | json text | Snapshot.FileDiff[] |
| `metadata` | json text | 扩展元数据 |
| `cost` | real (default 0) | 累计费用 |
| `tokens_input` | integer (default 0) | 累计输入 token |
| `tokens_output` | integer (default 0) | 累计输出 token |
| `tokens_reasoning` | integer (default 0) | 累计推理 token |
| `tokens_cache_read` | integer (default 0) | 缓存读 token |
| `tokens_cache_write` | integer (default 0) | 缓存写 token |
| `revert` | json text | 回退信息 {messageID, partID?, snapshot?, diff?} |
| `permission` | json text | 权限规则集 PermissionV1.Ruleset |
| `agent` | text | Agent ID |
| `model` | json text | 模型信息 {id, providerID, variant?} |
| `time_created` | integer | 创建时间戳 |
| `time_updated` | integer | 更新时间戳 |
| `time_compacting` | integer | 压缩时间 |
| `time_archived` | integer | 归档时间 |

索引：`session_project_idx`, `session_workspace_idx`, `session_parent_idx`

#### `session_message` 表（第 118-137 行）

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | text PK | 消息 ID |
| `session_id` | text FK → session.id ON DELETE CASCADE | 所属 Session |
| `type` | text NOT NULL | 消息类型（user/assistant/system/tool 等） |
| `seq` | integer NOT NULL | 顺序号 |
| `time_created` | integer NOT NULL | 创建时间戳 |
| `time_updated` | integer NOT NULL | 更新时间戳 |
| `data` | json text NOT NULL | 消息数据（不含 type 和 id） |

索引：
- `UNIQUE session_message_session_seq_idx (session_id, seq)` — 保证同 session 内 seq 唯一
- `session_message_session_type_seq_idx (session_id, type, seq)` — 按类型查询
- `session_message_session_time_created_id_idx (session_id, time_created, id)` — 按时间排序
- `session_message_time_created_idx (time_created)` — 全局时间索引

#### 其他 session 相关表

| 表名 | 说明 |
|------|------|
| `message` | V1 消息表（id, session_id, time_created, time_updated, data） |
| `part` | V1 消息片段表（id, message_id, session_id, time_created, time_updated, data） |
| `todo` | 待办事项（session_id+position 联合主键） |
| `session_input` | V2 输入队列（prompt, delivery, admitted_seq, promoted_seq） |
| `session_context_epoch` | V2 上下文 epoch 快照 |
| `session_share` | 分享链接 |

---

## 4. 结论

### 4.1 Session 数据库完整路径

- **全局 session 数据库**：`~/.local/share/opencode/opencode.db`
  - 这是 opencode 唯一的 session 数据库
  - 所有项目、所有 session、所有 message/part 全部存储于此
  - 使用 WAL 模式（`PRAGMA journal_mode = WAL`）
  - 可通过环境变量 `OPENCODE_DB` 覆盖路径
  - 可通过 `OPENCODE_DISABLE_CHANNEL_DB` 禁用按 channel 分库

- **项目级别**：**无独立的 session 数据库**
  - 项目 `.opencode/` 下只有 `memory/memory.db`（项目记忆系统），不含 session 数据
  - session 数据全部集中存储于全局 `opencode.db`，通过 `project_id` 字段区分项目归属

- **全局 memory 数据库**：`~/.config/opencode/memory.db`（28KB，非 session 数据）

### 4.2 `session_message` 表关键字段

```
id          TEXT PRIMARY KEY    — 消息唯一 ID
session_id  TEXT NOT NULL       — 外键 → session.id（CASCADE 删除）
type        TEXT NOT NULL       — 消息类型
seq         INTEGER NOT NULL    — 顺序号（唯一索引：session_id + seq）
time_created INTEGER NOT NULL   — 创建时间戳（毫秒）
time_updated INTEGER NOT NULL   — 更新时间戳（毫秒）
data        TEXT NOT NULL       — JSON 格式的消息数据体
```

### 4.3 信息来源

| 来源 | URL / 路径 |
|------|-----------|
| 本地源码（Drizzle 定义） | `source-code/opencode/packages/core/src/session/sql.ts` |
| 本地源码（SQL 迁移） | `source-code/opencode/packages/core/src/database/schema.gen.ts` |
| 本地源码（路径定义） | `source-code/opencode/packages/core/src/database/database.ts` |
| 本地源码（Global 路径） | `source-code/opencode/packages/core/src/global.ts` |
| 实际数据库验证 | `/home/uitstalie/.local/share/opencode/opencode.db` |
| 官方源码仓库 | `github.com/anomalyco/opencode`（本地 source-code/ 副本） |
