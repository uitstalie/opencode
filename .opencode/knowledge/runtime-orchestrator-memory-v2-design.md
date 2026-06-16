# Runtime Orchestrator — Memory 模块 V2 设计

> 日期：2026-06-15
> 状态：设计定稿（全部决策已确认）
> 关联讨论：触发时机、存储形态、后台异步提取、边界问题

## 设计原则

1. **按 scope 分流存储**：项目用 SQLite（可追溯），全局用 .md 文件（可直读）
2. **异步后台提取**：插件内直调 LLM API，完全绕开 opencode agent/session
3. **频率分离触发**：project / global / dreaming 三级不同触发频率
4. **生产者-消费者模型**：LLM 生产 → 后台消费，互不阻塞
5. **精简工具层**：4 个核心工具，砍掉多层流转

---

## 一、存储架构

```
┌─────────────────────────────────────────────────────────┐
│                    Memory 存储                           │
│                                                         │
│  全局 memory (scope=user)          项目 memory (scope=project) │
│  ┌──────────────────────┐       ┌──────────────────────┐ │
│  │ .md 文件 (权威)       │       │ SQLite (权威)         │ │
│  │                      │       │                      │ │
│  │ preferences.md       │       │ project_memory       │ │
│  │ constraints.md       │       │   - progress         │ │
│  │ patterns.md          │       │   - TODO             │ │
│  │ style.md             │       │   - tech             │ │
│  │                      │       │   - conclusion       │ │
│  │ 路径:                │       │ dreaming_projects    │ │
│  │ ~/.config/opencode/  │       │                      │ │
│  │   memory/            │       │ 路径:                │ │
│  │                      │       │ .opencode/memory/    │ │
│  │                      │       │   memory.db          │ │
│  └──────────────────────┘       └──────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### 全局 memory — .md 文件格式

每个分类一个文件，bullet 列表。并发写入使用文件锁。

```markdown
# User Preferences

- 使用简体中文回复；术语和路径可保留英文       #preference #confirmed
- 默认直接执行小到中等范围改动                  #preference #confirmed
- 不主动替用户引入额外前提或隐藏目标            #constraint #confirmed
```

- 每行格式：`- {content} {tags}`
- tags：空格分隔的 `#tag`
- 重复检测：写入前归一化内容，与已有条目对比
- **并发安全**：写入前获取文件锁（flock），写入后释放

### 项目 memory — SQLite 结构

```sql
CREATE TABLE project_memory (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  target     TEXT NOT NULL,              -- progress | TODO | tech | conclusion
  content    TEXT NOT NULL,
  tags       TEXT,                       -- 空格分隔标签
  metadata   TEXT,                       -- JSON 扩展
  created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(target, content)
);

CREATE TABLE dreaming_projects (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id        TEXT NOT NULL,
  category          TEXT NOT NULL,        -- habits | rules | patterns
  content           TEXT NOT NULL,
  confidence        TEXT,                -- #observed | #likely | #confirmed
  sessions_analyzed INTEGER DEFAULT 0,
  metadata          TEXT,
  created_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  updated_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(project_id, category, content)
);
```

### 砍掉的内容

- ~~global_memory 表~~ — 全局走 .md 文件
- ~~candidates.md / distilled.md~~ — 多层流转取消
- ~~memory_candidates / memory_distill / memory_promote 工具~~
- ~~memory_migrate 的全局迁移~~
- ~~turn-buffer.ts~~

---

## 二、频率分离触发架构

project、global、dreaming 三者触发频率解耦：

```
触发频率：project > global > dreaming
原因：项目结论随着每轮对话都可能产生；全局偏好变化缓慢；跨会话模式需要大量数据积累
```

### Project memory 触发条件

最频繁。只要对话产生了新信息就应提取。

```
触发条件 (任一满足):
├── 压缩前强制触发            ← compress 工具被调用时
├── token 压力预警            ← 决策 token > 压缩阈值 × 70%
├── 增量累积                  ← 新增 session_message > 20 条
└── 时间窗口                  ← 距上次提取 > 30 分钟 AND 新增 > 5 条
```

### Global memory 触发条件

锚定 compact，辅以延迟补充。

```
触发条件:
├── compact 强制触发          ← 最小，每次 compact 都检查 global
└── 延迟补充提取              ← compact 提取完成后，新增 global 相关记忆 > N 条时，
                              延迟 M 分钟再次检查一次
```

### Dreaming 触发条件

锚定 global，积累触发。

```
触发条件:
└── 积累 N 次 global 提取后触发   ← 不每次都跑，积累足够数据才做跨 session 模式发现

    N 次 = 可配置，默认 10 次 global 提取
    从 dreaming_projects 表读取已有模式作为上下文
```

### 防抖

- 同层级同 session 两次提取之间至少间隔 5 分钟
- 提取任务正在运行时，新触发排队（最多排队 1 个）
- 不同层级（project/global/dreaming）的提取可以同时运行

---

## 三、生产者-消费者模型

核心洞察：session 数据库天然就是队列。LLM 对话产生消息（生产者），后台提取任务读取并消化（消费者）。两者不需要同步等待。

```
┌─────────────────────┐     ┌──────────────────────┐
│   生产者 (主 LLM)    │     │   消费者 (后台提取)    │
│                     │     │                      │
│  每轮对话产生消息     │────▶│  从 session DB 读取   │
│  写入 session DB     │     │  用独立 model 提炼    │
│                     │     │  写入 memory          │
│  不感知提取进度       │     │  更新提取游标         │
│                     │     │                      │
│  compress 时:        │     │  互不阻塞             │
│  session DB 已持久化  │     │  compress 不需要等     │
│  提取晚点完成也没关系  │     │  提取完成             │
└─────────────────────┘     └──────────────────────┘
```

### 关键保证

- **compress 不等待 extract**：compress 只压缩 opencode 上下文，不删除 session DB。提取任务从 DB 读历史消息，不受压缩影响。
- **extract 不阻塞 LLM**：提取在后台异步运行，不影响主对话流程。
- **提取游标持久化**：记录 `last_extracted_seq`（session_message 的 seq），重启后从上次位置继续。

### Exit 处理

- 进程退出时 **直接丢弃** 未完成的提取任务
- 重启后从 `last_extracted_seq` 继续，处理新消息
- 丢失的提取窗口由下次触发覆盖（project memory 触发频率足够高）

### 提取粒度：滑窗 + 前期 memory

不是全量传递整个 session，而是：

```
每次提取传给 LLM 的上下文:
├── 窗口内对话：最近 N 条 session_message（N 可配置，默认 80 条）
├── 已有 project memory：上次提取的全部项目结论（作为"已知记忆"上下文）
├── 已有 global memory：上次提取的全部全局偏好（同理）
└── 已有 dreaming 数据：跨 session 模式发现时的历史模式

目标：LLM 基于"已有的记忆 + 新对话"来增量更新，而非每次都重新发现
```

---

## 四、后台异步提取 — 具体实现

### 总体流程

```
触发条件满足
  │
  ▼
┌─────────────────────────────────────────────────────┐
│  runtime-orchestrator 插件内部执行 (不产生 session)    │
│                                                     │
│  ① 读 session SQLite (只读)                          │
│     ~/.local/share/opencode/opencode.db             │
│     SELECT sm.data FROM session_message sm          │
│     JOIN session s ON s.id = sm.session_id          │
│     WHERE s.project_id = 当前项目                     │
│       AND s.agent = 'build'                         │
│       AND s.parent_id IS NULL                       │
│     ORDER BY sm.seq DESC LIMIT {window_size}         │
│                                                     │
│  ② 读已有 memory                                    │
│     project: SELECT FROM project_memory              │
│     global:  readFileSync .md 文件                   │
│                                                     │
│  ③ 读 API key + model 配置                          │
│     auth.json → 明文 key                            │
│     plugin-config → memoryExtractModel/providator    │
│                                                     │
│  ④ 调用 LLM API (fetch, 可配置超时)                  │
│     prompt = 滑窗对话 + 已有记忆 + 提取指令           │
│                                                     │
│  ⑤ 写入 memory                                      │
│     project: INSERT INTO project_memory             │
│     global:  flock → append .md → unlock            │
│                                                     │
│  ⑥ 更新提取游标                                     │
│     last_extracted_seq = 已处理的最大 seq            │
└─────────────────────────────────────────────────────┘
```

### LLM 配置（独立可配置）

在 `plugin-config/runtime-orchestrator/config.json` 中增加：

```json
{
  "memoryExtract": {
    "provider": "openai",
    "model": "gpt-4o-mini",
    "timeoutMs": 30000,
    "windowSize": 80,
    "dreamingAccumulateCount": 10
  }
}
```

如果未配置，退回到自动选择（优先用 auth.json 中第一个 api 类型的 provider）。

### 提取 Prompt 模板

```
你是 memory 维护助手。基于以下信息增量更新记忆：

## 已有记忆
### 项目记忆
{existing_project_memory}

### 全局记忆
{existing_global_memory}

## 新增对话
--- BEGIN ---
{conversation_text}
--- END ---

请从新对话中提取，输出 JSON：
{
  "project": [
    { "target": "progress|TODO|tech|conclusion", "content": "...", "tags": "#decision" }
  ],
  "global": [
    { "category": "preferences|constraints|patterns|style", "content": "...", "tags": "#confirmed" }
  ],
  "dreaming": [
    { "category": "habits|rules|patterns", "content": "...", "confidence": "#observed" }
  ]
}

规则：
- 只提取稳定的结论，不提取过程性讨论
- 每条 1-2 句话
- 已有记忆中的结论不需要重复提取
- 不确定的标记 #likely，确定的标记 #confirmed
- 跨 session 重复出现的模式标记 #observed，多次出现升级为 #confirmed
```

### 安全约束

- 不产生 session — 绕行 opencode 的关键优势
- 读 session 数据库是只读操作
- API key 只在进程内存中使用，不落盘
- 提取失败静默重试 1 次，不阻塞主流程

---

## 五、工具层 — 4 个核心工具

### 保留

| 工具 | 功能 | 全局 (scope=user) | 项目 (scope=project) |
|------|------|------------------|---------------------|
| `memory_record` | 写入稳定结论 | flock + append .md | INSERT SQLite |
| `memory_read` | 读取记忆 | readFileSync .md | SELECT SQLite |
| `memory_review` | 审阅/查重 | 统计 .md 行数 | SQLite GROUP BY |
| `dreaming_compress` | 去重合并 dreaming | — | 操作 dreaming_projects |

### 砍掉

| 工具 | 原因 |
|------|------|
| `memory_candidates` | 不确定结论用 `#likely` 标签即可 |
| `memory_distill` | 提炼内置在后台提取中 |
| `memory_promote` | rules 单独管理 |
| `memory_migrate` (全局部分) | 全局已改为 .md |

### `memory_record` 新行为

```
scope=user:
  1. 目标文件: ~/.config/opencode/memory/{name}.md
  2. 归一化 content
  3. 与已有条目对比（归一化后相同 → 跳过）
  4. flock(排他锁) → append → unlock

scope=project:
  1. 打开 memory.db
  2. INSERT INTO project_memory (target, content, tags)
  3. UNIQUE 约束自动防重复
```

### `memory_read` 新行为

```
scope=user, layer=raw:
  → readFileSync .md → 解析 bullets

scope=user, layer=dreaming:
  → 跨项目 dreaming_projects 表聚合

scope=project:
  → SELECT FROM project_memory (支持 target + search 过滤)
```

---

## 六、设计决策汇总

| 决策点 | 结论 | 理由 |
|--------|------|------|
| 全局存储 | .md 文件 | 人类可读、git diff、LLM 直读 |
| 项目存储 | SQLite | 可追溯、时间戳、结构化查询 |
| 提取方式 | 插件内直调 LLM | 绕开 session，零循环风险 |
| 提取粒度 | 滑窗 + 前期 memory | 成本可控，上下文不爆 |
| 触发频率 | project > global > dreaming | 按信息密度自然分层 |
| project 触发 | compact + token预警 + 增量 | 复合条件保覆盖 |
| global 触发 | compact 强制 + 延迟补充 | 锚定 compact，不遗漏 |
| dreaming 触发 | 积累 N(默认10)次 global | 需要足够数据 |
| 竞争模型 | 生产者-消费者 | compress 不阻塞 extract |
| exit 处理 | 直接丢弃，重启重来 | 简单可靠，丢失窗口短 |
| LLM 选择 | 独立可配置 model | 不抢主任务 token 预算 |
| 并发安全 | 全局 .md 文件锁 | 防止多项目同时写入 |
| 工具数量 | 8 → 4 | 砍掉多层流转 |

---

## 七、与现有代码的关系

| 模块 | 处理 | 说明 |
|------|------|------|
| `memory/db.ts` | 精简 | 删除 global_memory 操作，保留 project + dreaming |
| `memory/schema.sql` | 修改 | 删除 global_memory 表 |
| `memory/migrate.ts` | 精简 | 删除 migrateGlobalMemory |
| `memory/turn-buffer.ts` | 删除 | 无实际用途 |
| `index.ts` 工具定义 | 重写 | 4 工具替代 8，分流逻辑内聚 |
| `plugin-config/config.json` | 扩展 | 增加 memoryExtract 配置段 |

---

## 八、已确认的实现细节

### 8.1 Session 数据库路径与结构 ✅

| 项目 | 结论 |
|------|------|
| **全局 session DB** | `~/.local/share/opencode/opencode.db`（所有项目共享，WAL 模式） |
| **环境变量覆盖** | `OPENCODE_DB` 可覆盖路径 |
| **项目级 session** | 不存在独立文件，通过 `session.project_id` 区分归属 |

**提取时用到的关键表**：

```
session                    — 有 project_id / agent / parent_id / time_created
session_message            — 有 session_id / type / seq / data(JSON) / time_created
                          — 唯一索引: (session_id, seq)
```

**提取查询模板**：

```sql
SELECT sm.seq, sm.type, sm.data, sm.time_created
FROM session_message sm
JOIN session s ON s.id = sm.session_id
WHERE s.project_id = '{current_project_id}'
  AND s.agent = 'build'
  AND s.parent_id IS NULL          -- 排除 task fork
  AND sm.seq > {last_seq}          -- 增量（如果有游标）
ORDER BY sm.seq DESC
LIMIT {window_size}
```

### 8.2 提取游标持久化 ✅

**决定：不存精确游标，用时间+防重替代。**

理由：
- 滑窗提取每次都传"已有记忆 + 最新 N 条对话"
- LLM 基于已有记忆判断哪些是新结论
- UNIQUE 约束自动跳过重复
- 不需要精确追踪"上次提取到第几条"

如果未来需要游标（性能优化），在项目 `memory.db` 中增加元数据表：

```sql
CREATE TABLE extract_meta (
  session_id TEXT PRIMARY KEY,
  last_extracted_seq INTEGER NOT NULL,
  last_extracted_at INTEGER NOT NULL
);
```

### 8.3 文件锁跨平台 ✅

| 平台 | 方案 |
|------|------|
| Linux | `fs.openSync(path, 'a')` + `flock(fd, 'ex')` — POSIX 标准 |
| macOS | 同上，`flock` 行为一致 |
| Windows | 降级方案：写临时文件 + 原子 rename |

实现时用 `try-catch` 包裹，锁获取失败时静默跳过（下次触发再写）。全局 .md 文件并发写入频率极低（只 compact 时触发），实际几乎不会竞争。
