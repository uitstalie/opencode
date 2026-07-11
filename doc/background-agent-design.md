# 后台 Agent 设计评估

> 分析日期: 2026-07-11（初版），2026-07-11（修订：记忆存储改为纯 .md + flock）
> 分支: rust
> 涉及模块: `core/memory.rs`, `core/session.rs`, `core/config.rs`, `core/agent.rs`, `system_prompt.rs`, `tool/memory_record.rs`, `tool/memory_read.rs`, `tool/task.rs`, `tui/worker.rs`, `tui/session_ops.rs`, `tui/mod.rs`

---

## 1. 背景与目标

openrust 目前所有 agent 执行都是同步的——要么阻塞 worker 线程（主 agent loop、auto-compaction），要么 fire-and-forget 但不返回结果到 live 上下文（title/summary/compact）。

目标是为未来的记忆系统和上下文管理系统做基础设施评估：

1. **记忆 agent**：异步增量提取记忆，维护项目/全局/dreaming 三层存储
2. **上下文管理 agent**：将当前同步的 compaction 异步化，或引入渐进式上下文处理（尚未确定方向）

---

## 2. 现有基础设施

### 2.1 已有的后台 agent 模式

代码库中已存在完整的 fire-and-forget 后台 agent 模式。三个现有功能共用同一套基础设施：

| 功能 | 触发方式 | 文件位置 | 工具集 | max_steps |
|------|---------|---------|--------|-----------|
| `generate_title()` | turn 结束后自动 | `session_ops.rs:328` | `"none"` | 3 |
| `generate_summary()` | turn 结束后自动 | `session_ops.rs:376` | `"none"` | 3 |
| `compact_session()` | 用户 `/compact` | `session_ops.rs:219` | `"none"` | 5 |

三者遵循完全一致的模式：

```rust
// 1. clone 必要上下文
let llm_clone = Arc::clone(&llm);
let store_clone = store.clone();
let cwd = self.cwd.clone();

// 2. 在独立线程上 fire-and-forget
std::thread::spawn(move || {
    let rt = shared_runtime();              // ← 持久 runtime
    let system = agent::builtin_agent_system("...").unwrap();
    rt.block_on(run_agent(
        llm_clone.as_ref(),
        &model,
        &system,
        "none",                             // 无工具
        N,
        None,
        vec![Message::user(prompt)],
        &ToolContext::new(cwd),
    ));
    // 3. 结果写入 store
});
```

### 2.2 shared_runtime

`session_ops.rs:15-23` 定义了进程级持久 tokio runtime：

```rust
fn shared_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to create background tokio runtime")
    })
}
```

- `OnceLock` 保证全局唯一，app 生命周期内不销毁
- `new_current_thread` 在 `std::thread::spawn` 的线程上 `block_on`，各后台任务互不阻塞
- TUI 主线程不受影响（通过 channel 回传结果）

### 2.3 run_agent 自包含性

`task.rs:97` 的 `run_agent()` 是完全自包含的 async 函数，已被四处独立复用：

| 调用方 | 文件 | 用途 |
|--------|------|------|
| TaskTool | `task.rs:67` | 阻塞式 sub-agent |
| Worker auto-compaction | `worker.rs:234` | 同步上下文压缩 |
| CLI headless e2e | `cli/debug/e2e.rs:34` | 命令行 agent 执行 |
| generate_title/summary/compact | `session_ops.rs` | 后台 fire-and-forget |

### 2.4 并发安全审计

| 资源 | 类型 | 线程安全 | 备注 |
|------|------|---------|------|
| `LlmProvider` | `Arc<dyn Send + Sync>` | ✅ | 可多 agent 并发调用 |
| `SessionStore` | `sled::Db` + `Arc<AtomicU64>` | ✅ | sled 内置并发，需独立 session_id |
| `UndoStore` | `Arc<UndoStore>`，内部 `Mutex<u32>` | ✅ | 快照基于内容哈希，无冲突 |
| `ToolContext` | `#[derive(Clone)]`，全 `Arc`/`Option` | ✅ | 可 clone 给后台 agent |
| 文件系统 | `std::fs::*` | ⚠️ 无锁 | 两 agent 同时编辑同一文件会冲突 |
| 交互通道 | `ask_tx` / `permission_tx` | ⚠️ mpsc | 后台 agent 不能阻塞等待用户输入 |

### 2.5 Session 数据持久化

消息**逐条实时持久化**，不存在延迟批量写入：

| 事件 | 持久化时机 | 方法 |
|------|-----------|------|
| 用户输入 | `handle_prompt()` 立即 | `persist_message("user", ...)` |
| 工具批次（assistant + tool calls + results） | `PromptEvent::ToolBatch` 到达时 | `persist_message_detail(...)` 逐条 |
| 最终回复 | `PromptEvent::Finish` 到达时 | `persist_message("assistant", ...)` |

当系统回到 "Ready" 等待用户输入时，`store.get_messages(session_id)` 已包含完整对话历史。

**Compaction 不删除原始消息**——`append_compaction()` 仅追加一条 checkpoint（`summary: Some(...)`）。`get_messages()` 返回全部消息；`effective_messages()` 从最后一个 checkpoint 截断。

---

## 3. 记忆系统设计

### 3.0 Rules vs Memory：边界定义

**Rules 和 Memory 是两个独立层，不互相替代，是提炼关系。**

| | Rules | Memory |
|--|-------|--------|
| 本质 | 硬性约束 | 提炼知识 |
| 注入方式 | **每轮自动注入** system prompt | 内容**不注入**；仅注入 `<memory>` 位置索引，内容按需 `memory_read` |
| 来源 | 用户手写 | agent 维护 |
| 体积 | 小而稳定（几十行） | 可持续增长 |
| 权威等级 | 最高，覆盖一切 | 参考性 |
| 格式 | `.md` 文件（rules/ 目录 + AGENTS.md） | `.md` 文件（memory/ 目录，纯文件无 DB） |
| 加载机制 | `load_rules_dir()` 自动全量 | `memory_read` 工具按需查询 |

**核心原则**：

1. Memory 内容**永远不自动注入** system prompt——避免 token 膨胀、避免过时信息干扰
2. System prompt 中注入一个固定 `<memory>` **索引段**——只有存储位置，没有内容也没有计数
3. Agent 看到索引后**自行决定**是否用 `memory_read` 深入读取
4. Rules 优先级高于 Memory——如果冲突，以 rules 为准

**System prompt 中的 `<memory>` 段**（纯静态，render 时零查询）：

```xml
<memory>
项目记忆: .openrust/memory/*.md (scope=project)
全局记忆: ~/.config/openrust/memory/*.md (scope=user)
dreaming: ~/.config/openrust/memory/dreaming/ (scope=dreaming)
用 memory_read 工具按需查阅。
</memory>
```

为什么不需要计数/刷新：
- 存储位置是固定的（固定 .md 文件 + 固定目录），不随内容变化
- "有几条、是什么"由 `memory_read` 实时查询 .md 文件得知，天然新鲜
- `<memory>` 段可硬编码在 system prompt 模板中，render 零开销

**与 Rules 的关系**：

```
SystemPrompt::render():
  <instructions>
    <global-rules>          ← 硬约束，全文注入
    <project-instructions>   ← AGENTS.md，全文注入
    <project-rules>          ← 硬约束，全文注入
  </instructions>
  <memory>                   ← 索引注入，只有位置没有内容
    项目记忆: .openrust/memory/*.md
    全局记忆: ~/.config/openrust/memory/*.md
    dreaming: ~/.config/openrust/memory/dreaming/
    用 memory_read 按需查阅
  </memory>
```

Rules 是"必须遵守的规则"，Memory 是"可供参考的知识索引"。前者全量注入约束行为，后者只给目录、按需取用。

### 3.1 dev-ai 实现调研

dev-ai 分支（commit `88b8638af`）有完整的记忆系统实现，包含 4 个工具 + 专用 agent + compaction 后触发：

| 组件 | TS 文件 | 行数 | 职责 |
|------|---------|------|------|
| `memory_record` | `memory-record.ts` | 157 | 写入记忆（SQLite / .md / dreaming） |
| `memory_read` | `memory-read.ts` | 130 | 读取记忆（按 scope/target/search） |
| `memory_review` | `memory-review.ts` | 105 | 查重统计（GROUP BY 计数） |
| `dreaming_compress` | `dreaming-compress.ts` | 243 | 去重/合并/置信度升级 |
| `memory-extract.txt` | prompt | 146 | 记忆提取 agent 指令 |
| compaction hook | `session/compaction.ts` | 78 | `Effect.forkDetach` 触发 |

dev-ai 的触发方式：compaction 完成后 `Event.Compacted` → `Effect.forkDetach` → 创建 child session → memory-extract agent 运行。取最近 20 条消息作为快照输入。

**dev-ai 的设计缺陷**：触发点在 compaction 之后，此时 `session.messages()` 已是压缩后的历史，原始消息已被 summary 替代——记忆 agent 看不到被压缩掉的细节。

### 3.2 三层存储模型

```
项目层 (scope=project)
   └─ {project}/.openrust/memory/{progress,TODO,tech,conclusion}.md
      └─ 每条一行: "- [{date}] {content} #tag1 #tag2"

全局层 (scope=user)
   └─ ~/.config/openrust/memory/{preferences,constraints,patterns,style}.md
      └─ 每条一行: "- [{date}] {content} #tag1 #tag2"

dreaming 层 (scope=dreaming)
   └─ ~/.config/openrust/memory/dreaming/{sha256(cwd)[:12]}.md
      └─ 每条一行: "- [{date}] {content} #tag"
```

**纯 .md，无 DB。** .md 文件是唯一 source of truth。

| 层 | scope | 存储 | 写入条件 |
|----|-------|------|---------|
| 项目层 | `project` | `.openrust/memory/*.md` | 有项目级进展/决策/待办 |
| 全局层 | `user` | `~/.config/openrust/memory/*.md` | 跨项目 ≥2 次出现的偏好/约束 |
| dreaming 层 | `dreaming` | `~/.config/openrust/memory/dreaming/{hash}.md` | `/dream` 手动触发时 |

**并发写保护**：文件锁（`flock`）。主 agent 和后台 memory-extract agent 都可能写入同一 .md 文件，写入前获取独占锁：

```
memory_record 写入流程:
  1. flock(LOCK_EX)          ← 独占锁
  2. 读 .md 全文 → 解析已有条目
  3. 去重检查（完整 content trim+lowercase 比较）
  4. 追加新条目 → 原子写回（写 temp → rename）
  5. flock(LOCK_UN)          ← 释放锁
```

写入频率极低（提取时 + 用户显式要求），锁竞争概率可忽略。

### 3.3 工具设计

#### 工具可用性

| 工具 | 主 agent | memory-extract agent | dreaming agent | 说明 |
|------|---------|---------------------|----------------|------|
| `memory_read` | ✅ | ✅ | ✅ | 查阅记忆 |
| `memory_record` | ✅ | ✅ | ✅ | 写入记忆 |

只有 2 个工具。主 agent 可即时读写记忆，不必等后台 agent。memory-extract 和 dreaming agent 用相同工具集，区别在 system prompt 和触发方式。

#### category 参数

统一用 `category` 替代之前的 `target`/`name`，由 tool 内部按 scope 验证：

| scope | 合法 category | 文件 |
|-------|--------------|------|
| `project` | `progress` \| `TODO` \| `tech` \| `conclusion` | `.openrust/memory/{category}.md` |
| `user` | `preferences` \| `constraints` \| `patterns` \| `style` | `~/.config/openrust/memory/{category}.md` |
| `dreaming` | （忽略，固定写入） | `~/.config/openrust/memory/dreaming/{sha256(cwd)[:12]}.md` |

`scope=dreaming` 时 `category` 可省略。`scope=project|user` 时 `category` 必填，tool 校验不通过则返回错误。

#### 条目格式

每条记忆一行，格式固定：

```
- [2026-07-11] {content} #tag1 #tag2
```

| 字段 | 规则 |
|------|------|
| 日期 | `YYYY-MM-DD`，写入时自动生成 |
| content | 单行；含 `\n` 时替换为空格 |
| tags | 0-N 个，`#` 前缀，空格分隔 |

#### memory_record

```
输入: content, scope(project|user|dreaming), category?, tags[]
行为:
  1. 按 scope + category 计算目标 .md 路径
  2. flock(LOCK_EX)
  3. 读文件 → 解析已有条目（文件不存在则视为空）
  4. 去重：已有条目中存在 trim+lowercase 后完全一致的 content → 跳过，返回 "已存在"
  5. 追加: "- [{today}] {content_normalized} #tag1 #tag2"
  6. 原子写回（写 temp → rename）
  7. flock(LOCK_UN)
```

去重策略：**完整 content 比较**（trim + lowercase），不做截断。数据量小，全量比较零开销。语义级去重交给 agent 判断——agent 在写入前 `memory_read` 查看已有条目，自行决定是否值得新增。

#### memory_read

**记忆的唯一访问入口。** Memory 不注入 system prompt，agent 必须通过此工具按需读取。

```
输入: scope(project|user|dreaming)?, category?, search?
行为:
  无参数 → 遍历所有 scope 的所有文件 → 返回各分类条目数概览
  有 scope → 读对应 scope 的 .md 文件 → 解析
  scope=dreaming → 只读当前项目的 dreaming 文件（sha256(cwd)[:12].md）
  有 category → 在 scope 内按 category 过滤
  有 search → 在结果中关键词过滤
  文件不存在 → 返回空
返回: 解析后的结构化条目列表，每条含 (date, content, tags)
注: project/dreaming 路径依赖 ToolContext.cwd 定位
```

### 3.4 后台 Agent 模型选择

后台 agent（title/summary/memory-extract）执行的是总结/提取任务，不需要和主 agent 一样的模型能力。应允许用户指定更便宜的模型。

**Config 新增字段**：

```jsonc
{
  "model": "anthropic/claude-sonnet-4",
  "background_model": "deepseek/deepseek-chat"  // 后台 agent 专用
}
```

**Config 结构变更**（`core/config.rs`）：

```rust
pub struct Config {
    pub model: Option<String>,
    pub background_model: Option<String>,  // ← 新增
    pub provider: HashMap<String, ProviderConfig>,
    pub presets: HashMap<String, Vec<String>>,
}
```

`Config::merge` 中增加一行：`if other.background_model.is_some() { self.background_model = other.background_model; }`

**模型解析链**：

```
resolve_background_model():
  config.background_model  →  使用它（解析 provider + model）
  ↓ (未配置)
  config.model             →  fallback 到主模型
```

**Provider 创建**：

- 同 provider 不同 model（如主 `deepseek/v4-pro`，后台 `deepseek/chat`）：复用 `self.llm`（同一 `LlmProvider` 实例），只传不同 model 字符串
- 不同 provider（如主 `anthropic/sonnet`，后台 `deepseek/chat`）：调 `config.get_provider()` + `provider::create_provider()` 创建新实例

**影响范围**：

现有三个后台方法（`generate_title`, `generate_summary`, `compact_session`）和未来的 `generate_memory` 统一走 `resolve_background_model()`。未配置 `background_model` 时行为不变（fallback 到主模型）。

```rust
// 统一的后台模型解析（session_ops.rs 或 config.rs）
fn resolve_background_provider(
    &self,
) -> Option<(Arc<dyn LlmProvider>, String)> {
    let bg_spec = self.config.background_model.as_deref()
        .or(self.config.model.as_deref())?;
    let (provider_name, model_name) = parse_model_spec(bg_spec)?;
    let (provider_name, wire_model) = /* resolve variant */;
    
    // 同 provider：复用已有 llm
    if provider_name == self.provider_name {
        return Some((Arc::clone(self.llm.as_ref()?), wire_model));
    }
    // 不同 provider：创建新实例
    let resolved = self.config.get_provider(provider_name)?;
    let llm = Arc::from(provider::create_provider(&resolved)?);
    Some((llm, wire_model))
}
```

### 3.5 增量提取模式

**核心改进**（相比 dev-ai 的全量模式）：高频触发时不需要传完整历史，只传增量。

```
触发时（主线程）:
  1. 读取水位线: last_extracted_seq
  2. store.get_messages(session_id)
       .filter(|m| m.seq > last_extracted_seq)   ← 只取增量
  3. 增量为空 → 跳过
  4. 序列化增量 → snapshot（很小）

后台线程:
  run_agent(
    initial = "以下是自上次提取后的新消息:\n{snapshot}
              请 review 已有记忆，仅在有新结论/偏好/决策时更新。
              无新记忆 → 回复「无新记忆」，不强行写入。",
    tools = [memory_read, memory_record],
    max_steps = 15,
  )

完成后:
  推进水位线 → last_extracted_seq = 增量最后一条的 seq
```

**水位线**：

| 属性 | 说明 |
|------|------|
| 存储 | SessionStore 新增 sled key: `memory_watermark:{session_id}` |
| 类型 | `u64`（Message.seq） |
| 推进时机 | agent 完成后（非之前），保证 at-least-once |
| 幂等保证 | .md 文件锁 + 完整 content 去重，重复处理不会产生重复记忆 |
| 失败恢复 | agent 崩溃 → 水位线不推进 → 下次重新处理相同增量 → 幂等去重 |

**全量 vs 增量对比**：

| | 全量模式（dev-ai） | 增量模式（本设计） |
|--|---|---|
| 每次输入 | 最近 20 条消息 | 仅上次提取后的新消息 |
| 重复处理 | 高（反复分析同样内容） | 低（只看增量） |
| prompt 大小 | 固定（~20 条） | 可变（通常 1-5 条） |
| 触发频率 | 仅 compaction 后 | 每 turn 或每 N turn |
| LLM token 消耗 | 高 | 低 |
| 漏检风险 | 低（全量扫描） | 极低（增量不丢，水位线保证） |

### 3.6 触发机制

```
mod.rs PromptEvent::Finish 分支:
  self.persist_message("assistant", &assistant);
  self.ai_running = false;
  self.prompt_job = None;
  self.generate_summary();
  self.generate_memory();   // ← 新增
```

触发流程：

```
generate_memory():
  ├─ 检查 LLM 可用性
  ├─ 读取水位线
  ├─ 从 store 读取增量消息
  ├─ 增量为空？→ return（跳过）
  ├─ 序列化增量
  └─ thread::spawn:
       shared_runtime.block_on(run_agent(...))
       推进水位线
```

**节流策略**（可选，后续添加）：
- 最小间隔：距上次触发 < 30s → 跳过
- 最小增量：新消息 < 3 条 → 跳过
- 错峰：Finish 后延迟 5s 再触发，避免与 title/summary 抢同一 LLM 窗口
- Compaction 后强制触发一次（和 dev-ai 一致）

### 3.7 memory-extract agent

**定义**（`core/agent.rs` 新增 builtin）：

```rust
AgentInfo {
    id: "memory-extract",
    title: "Memory Extract",
    description: "Background memory extraction agent.",
    tools: "[memory_read, memory_record]",
    hidden: true,
    max_steps: 15,
    system: BUILTIN_MEMORY_EXTRACT_SYSTEM,
}
```

**工具集**：仅 2 个 memory 工具，无 bash/write/edit/read/grep——agent 不接触项目文件系统，只操作记忆存储。

**System prompt**（从 dev-ai 迁移，适配增量模式）：

核心指令要点（完整 prompt ~120 行）：
1. **增量思维**：分析的是"自上次提取后的新消息"，不是完整对话
2. **宁缺毋滥**：只记稳定结论，不记流水账/临时状态/一次性操作
3. **先读后写**：`memory_read` 查看已有条目 → 自行判断是否重复 → `memory_record` 写入
4. **两层提取**：project（进展/待办/技术决策/结论）→ user（偏好/约束/模式/风格）。dreaming 不在这里做——dreaming 是独立的手动操作。
5. **标签体系**：`#confirmed`/`#likely`/`#decision`/`#architecture`/`#constraint`/`#preference`/`#pattern`/`#style`/`#issue`
6. **无新记忆**：回复"无新记忆"，不强行编造

**标签置信度规则**：
- `#confirmed`：确定无疑的事实
- `#likely`：高概率推断（多次复现后，dreaming agent 会将其升级为 `#confirmed`）

### 3.8 路径适配

dev-ai 路径 → rust 路径映射：

| dev-ai | rust | 说明 |
|--------|------|------|
| `.opencode/memory/memory.db` | `.openrust/memory/*.md` | 项目记忆（纯 .md，无 DB） |
| `~/.config/opencode/memory/*.md` | `~/.config/openrust/memory/*.md` | 全局记忆 |
| `~/.config/opencode/memory/dreaming/` | `~/.config/openrust/memory/dreaming/` | dreaming |
| `bun:sqlite` Database | 纯 .md 文件 + `flock` | 存储引擎（去 DB 依赖） |
| `Location.Service` (Effect) | `ToolContext.cwd` | 项目路径 |
| `Effect.forkDetach` | `thread::spawn + shared_runtime` | 异步执行 |

**存储选择：纯 .md，无 DB。** .md 文件是唯一 source of truth。查询靠解析 .md（数据量小，微秒级），并发写靠 `flock`，每条条目自带 `[{date}]` 时间戳标记记录时间。水位线是 session 级元数据，存 SessionStore sled（`memory_watermark:{session_id}` → `u64`），与记忆存储无关。

### 3.9 实现路径 & 改动量

| 组件 | 文件 | 行数 | 说明 |
|------|------|------|------|
| `MemoryStore` | `core/memory.rs`（新） | ~100 | .md 读写封装：解析 + flock 写入 + 去重 |
| `memory_record` tool | `tool/memory_record.rs`（新） | ~110 | 写入工具（主 agent + 提取 agent + dreaming agent） |
| `memory_read` tool | `tool/memory_read.rs`（新） | ~90 | 读取工具 |
| memory-extract agent | `core/agent.rs` | ~50 | builtin agent + system prompt（2 工具） |
| dreaming agent | `core/agent.rs` | ~50 | builtin agent + system prompt（2 工具） |
| dreaming 触发 | `tui/session_ops.rs` + `tui/mod.rs` | ~60 | `/dream` 命令 + 全 session 收集 + fire-and-forget |
| 工具注册 | `tool/mod.rs` + `tool/catalog.rs` | ~15 | 注册 2 个工具 |
| `<memory>` 索引注入 | `system_prompt.rs` | ~5 | 硬编码 `<memory>` 段字符串，零查询开销 |
| 水位线 | `core/session.rs` | ~15 | SessionStore sled key: `memory_watermark:{session_id}` |
| 触发逻辑 | `tui/session_ops.rs` | ~40 | `generate_memory()` + 增量快照 |
| 触发点 | `tui/mod.rs` | 1 | Finish 分支 |
| 后台模型支持 | `core/config.rs` + `tui/session_ops.rs` | ~30 | `background_model` 字段 + `resolve_background_provider()` |
| **总计** | | **~565 行** | |

### 3.10 约束与对策

| 约束 | 对策 |
|------|------|
| LLM rate limit | 低 reasoning_effort + 小 max_steps (15) + 节流 |
| 后台 agent 并发 | Finish 时 title + summary + memory 三个 fire-and-forget 线程可能同时调 LLM；未配 `background_model` 时共用同一 provider，需关注 rate limit。对策：memory 节流延后 ≥5s，错峰发起 |
| 后台 agent 无交互能力 | 设计上不需要交互——全部自主决策 |
| 水位线一致性 | agent 完成后推进；失败不推进；幂等去重兜底 |
| 记忆质量 | 增量模式 + "宁缺毋滥" prompt + agent 写入前 memory_read 自行判断重复 |
| 并发写 .md | flock 独占锁 + 完整 content 去重；主 agent + 后台 agent 可同时写 |
| 多 session 并发 | 每个 session 独立水位线（SessionStore sled key）；.md 写入靠 flock 串行化 |

### 3.11 Dreaming：跨 session 模式提取

**dreaming 不是工具，是用户手动触发的独立操作**，定位类似 `/compact`。

| 属性 | 说明 |
|------|------|
| 触发方式 | 手动（`/dream` 或类似 TUI 命令） |
| 触发频率 | 低（用户觉得积累够了时，或定期整理） |
| 数据来源 | **所有 session**（SessionStore 中该项目的全部 session） |
| 分析目标 | 提取跨 session 的用户语言模式、沟通偏好、重复工作流 |
| 输出位置 | `~/.config/openrust/memory/dreaming/{sha256(cwd)[:12]}.md` |

**与 memory-extract 的区别**：

| | memory-extract | dreaming |
|--|----------------|----------|
| 触发 | 自动（turn 结束） | 手动（`/dream`） |
| 范围 | 当前 session 增量消息 | 所有 session 全量历史 |
| 目的 | 项目级进展/决策/待办 | 跨 session 的模式与偏好 |
| 输出层 | project + user | dreaming |
| 频率 | 每 turn / 每 N turn | 偶尔（手动） |

**执行流程**（类似 `compact_session()`）：

```
/dream 命令:
  ├─ 收集所有 session 的消息（或 summary）
  ├─ 序列化为大上下文快照
  └─ thread::spawn:
       shared_runtime.block_on(run_agent(
         agent = "dreaming",
         initial = "以下是本项目所有 session 的历史摘要:\n{snapshot}
                   请分析用户在跨 session 中表现出的语言模式、沟通偏好和重复工作流。
                   提取到全局模式时用 memory_record(scope=dreaming) 写入。",
         tools = "[memory_read, memory_record]",
         max_steps = 20,
       ))
```

**dreaming agent**（`core/agent.rs` 新增 builtin）：

```rust
AgentInfo {
    id: "dreaming",
    title: "Dreaming",
    description: "Cross-session pattern extraction.",
    tools: "[memory_read, memory_record]",
    hidden: true,
    max_steps: 20,
    system: BUILTIN_DREAMING_SYSTEM,
}
```

**数据量控制**：所有 session 全量消息可能很大。策略：
- 优先用 session summary（`generate_summary` 已生成），不用原始消息
- summary 不足时 fallback 到 session 标题 + 最近 N 条消息
- 极端情况（session 过多）分批处理

**System prompt 要点**：
1. 分析的是**跨 session 的模式**，不是单个 session 的内容
2. 重点是用户行为：语言风格、沟通习惯、重复操作模式、偏好工具/流程
3. 先 `memory_read(scope=user)` 看已有偏好，避免重复
4. 用 `memory_record(scope=dreaming)` 写入，每条标注 `#pattern` / `#style` / `#preference`

---

## 4. 上下文管理异步化评估

### 4.1 当前上下文管理机制

存在三条路径，互不协调：

```
worker.rs:202         worker.rs:276         session_ops.rs:219
     ↓                      ↓                      ↓
 自动压缩               历史 trim               手动 /compact
 (同步阻塞 worker)      (同步即时)              (异步 fire-forget)
     ↓                      ↓                      ↓
 LLM summary           "[trimmed]"            LLM summary
 改写 history          截断 history            写 store，不改 history
```

| 属性 | 自动压缩 | 历史 trim | 手动 /compact |
|------|---------|----------|-------------|
| 执行方式 | 同步（阻塞 loop） | 同步（即时） | 异步（fire-forget） |
| LLM 调用 | 是 | 否 | 是 |
| 结果 | 改写 live history | 截断 live history | 仅写 store |
| 质量 | 结构化 summary | "[trimmed]" 占位 | 结构化 summary |
| 延迟 | ~3-10s 阻塞 | 0 | 0 |

### 4.2 核心矛盾

上下文管理与记忆 agent 的本质区别：结果**必须同步到 worker 的 `history` 向量**才能在下次 LLM 调用前生效。

worker 的 `history` 是 `block_on(async { ... })` 内的局部变量。后台 agent 完成时，worker 可能已经用未压缩的 history 发了下一轮请求。**这是 auto-compaction 必须同步的原因**——它 `await run_agent()` 拿到结果后直接 `history.drain()`。

### 4.3 四条异步化路径

#### 路径 1：预计算 + 延后应用（shadow compaction）

```
后台 agent 持续监控 token 量
  └─ 60% 阈值时启动 shadow compaction（不阻塞 worker）
  └─ 完成后把 summary 放入 channel

worker loop 每步检查：
  ├─ 有 shadow summary 可用？→ 应用（即时，无阻塞）
  └─ 到 80% 硬阈值还没拿到？→ 回退同步压缩（当前行为）
```

- **改动**：worker 增加 `summary_rx: Option<Receiver<String>>`，每步 `try_recv`
- **优点**：正常零阻塞；有降级保底
- **缺点**：shadow 可能在 worker 已 trim 后才完成（白跑）
- **风险**：低

#### 路径 2：分层上下文（hot/warm/cold）

```
hot  = 最近 N 条消息（始终 verbatim）
warm = 结构化 summary（后台 agent 增量维护）
cold = store 完整历史（按需检索）

worker 每轮发送：system(warm) + last_N(hot)
后台 agent 每 K 轮：读增量消息 → 更新 warm summary
```

- **改动**：大——发送逻辑、warm 存储位置、增量触发
- **优点**：无信息丢失、无阻塞、平滑
- **缺点**：warm 质量依赖后台频率；最复杂
- **风险**：中高

#### 路径 3：checkpoint + trim（异步总结，同步截断）

```
后台 agent（异步）：定期写结构化 checkpoint
worker（同步）：只做 trim_history（已有），插入 "[checkpoint @id]"
LLM 需要旧上下文：通过 tool 从 checkpoint 检索
```

- **改动**：中——checkpoint 格式、检索 tool、trim 微调
- **优点**：完全解耦，worker 永不阻塞
- **缺点**：引入 RAG 式不确定性
- **风险**：中

#### 路径 4：渐进式摘要

```
后台 agent 每 N 轮：
  读增量消息 → 生成增量摘要 → 合并进 anchored summary → 存入 store

worker：
  发送时用 store 里的 latest summary 替代旧消息
  截断逻辑不变（trim_history）
```

- **改动**：中——增量合并逻辑、触发频率
- **优点**：summary 质量更稳定（小步快跑 vs 一次性压缩）
- **缺点**：需协调"增量完成"与"worker 使用旧 summary"的时间窗
- **风险**：中

### 4.4 基础设施就绪度

| 能力 | 已有 | 位置 |
|------|------|------|
| 持久 runtime | ✅ | `shared_runtime()` |
| fire-and-forget 线程 | ✅ | `thread::spawn` |
| `run_agent` 自包含 | ✅ | `task.rs:97` |
| compaction prompt 模板 | ✅ | `compaction.rs:16-51` |
| 增量 summary 支持 | ✅ | `build_compaction_prompt(prev, ...)` |
| token 估算 | ✅ | `token::estimate` |
| trim 逻辑 | ✅ | `trim_history` |
| worker ← 后台通信 channel | ❌ | 需新增 |
| 分层/增量 summary 存储 | ❌ | store 目前是 append-only |

### 4.5 可行性结论

全部四条路径均可基于现有基础设施实现。核心难点不在实现，在于"后台 agent 结果如何同步到 worker 的 live `history`"这一协调问题。路径 1（shadow compaction）是最小改动、最低风险的起点。

---

## 5. 分阶段路线

各阶段互相不冲突，可独立推进：

### 阶段 1：记忆工具 + 存储层 + 索引注入

- 实现 `core/memory.rs`（MemoryStore：.md 解析 + flock 写入 + 去重）
- 实现 2 个工具：`memory_record`, `memory_read`
- 注册到 catalog
- 在 `system_prompt.rs` 中注入硬编码 `<memory>` 索引段（纯静态字符串）
- 可独立测试（通过 CLI debug 子命令）

### 阶段 2：记忆 agent + 增量触发

- 新增 `memory-extract` builtin agent + system prompt
- 实现 `generate_memory()` 增量提取（水位线 + 快照 + fire-and-forget）
- 在 `PromptEvent::Finish` 处接入触发
- 可选节流策略
- **不需要改 system prompt 组装**——memory-extract 是 hidden agent，自带 prompt

### 阶段 2.5：dreaming（手动跨 session 提取）

- 新增 `dreaming` builtin agent + system prompt
- 实现 `/dream` 命令（TUI 输入解析 + session_ops 触发）
- 收集所有 session summary → 序列化 → fire-and-forget
- 数据量控制策略实测调优
- 独立于阶段 2，可并行开发

### 阶段 3：shadow compaction（路径 1，独立于记忆系统）

- 解决 worker auto-compaction 同步阻塞痛点
- 改动：worker 增加 channel + `try_recv` 检查
- 降级路径：回退当前同步行为

### 阶段 4：渐进式或分层上下文（路径 2/4，待定）

- 根据 shadow compaction 实测效果决定是否需要
- 需要实测 summary 质量与延迟数据

### 阶段 5：按需检索（路径 3，待定）

- 如果上述方案仍有信息丢失，考虑 RAG 式 checkpoint 检索
- 引入新的 tool，LLM 主动拉取旧上下文
