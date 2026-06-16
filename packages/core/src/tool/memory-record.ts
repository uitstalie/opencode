export * as MemoryRecordTool from "./memory-record"

import { ToolFailure } from "@opencode-ai/llm"
import { Effect, Layer, Schema } from "effect"
import { PermissionV2 } from "../permission"
import { Tool } from "./tool"
import { Tools } from "./tools"
import { Location } from "../location"
import { join } from "node:path"
import { existsSync, mkdirSync, writeFileSync, readFileSync } from "node:fs"
import { createHash } from "node:crypto"
import { Database } from "bun:sqlite"

export const name = "memory_record"

export const Input = Schema.Struct({
  content: Schema.String.annotate({ description: "长期记忆内容，1-2句话；写结论，不写过程" }),
  scope: Schema.Literals(["user", "project", "dreaming"]).annotate({ description: "user=跨项目全局记忆；project=当前项目记忆；dreaming=跨会话提炼的记忆模式" }),
  name: Schema.optional(Schema.String).annotate({ description: "scope=user 必填。全局类别名: preferences | constraints | patterns | style" }),
  target: Schema.optional(Schema.String).annotate({ description: "scope=project 必填。必须是: progress | TODO | tech | conclusion" }),
  tags: Schema.optional(Schema.Array(Schema.String)).annotate({ description: "可选标签: #decision #pattern #issue #constraint #preference #architecture #style" }),
})

export const Output = Schema.String
export type Output = typeof Output.Type

// ─── helpers ───────────────────────────────────────────

const GLOBAL_MEM_DIR = join(process.env.HOME ?? "/tmp", ".config", "opencode", "memory")
const DREAMING_DIR = join(GLOBAL_MEM_DIR, "dreaming")

function ensureGlobalDir() {
  if (!existsSync(GLOBAL_MEM_DIR)) mkdirSync(GLOBAL_MEM_DIR, { recursive: true })
}

function ensureDreamingDir() {
  if (!existsSync(DREAMING_DIR)) mkdirSync(DREAMING_DIR, { recursive: true })
}

function readGlobalEntries(category: string): string[] {
  const path = join(GLOBAL_MEM_DIR, `${category}.md`)
  if (!existsSync(path)) return []
  return readFileSync(path, "utf-8")
    .split("\n")
    .map(l => l.trim())
    .filter(l => l.startsWith("- "))
}

function appendGlobalEntry(category: string, content: string, tags: readonly string[]): void {
  ensureGlobalDir()
  const path = join(GLOBAL_MEM_DIR, `${category}.md`)
  const tagStr = tags.length > 0 ? " " + tags.join(" ") : ""
  const line = `- ${content}${tagStr}\n`
  writeFileSync(path, line, { flag: "a" })
}

function ensureProjectDb(dir: string): Database {
  const memDir = join(dir, ".opencode", "memory")
  if (!existsSync(memDir)) mkdirSync(memDir, { recursive: true })
  const dbPath = join(memDir, "memory.db")
  const db = new Database(dbPath)
  db.run(`
    CREATE TABLE IF NOT EXISTS project_memory (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      target TEXT NOT NULL,
      content TEXT NOT NULL,
      tags TEXT,
      metadata TEXT,
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      UNIQUE(target, content)
    )
  `)
  return db
}

function insertProjectMemory(db: Database, target: string, content: string, tags: readonly string[]): void {
  const tagStr = tags.join(" ")
  db.run(
    "INSERT OR REPLACE INTO project_memory (target, content, tags, updated_at) VALUES (?, ?, ?, CURRENT_TIMESTAMP)",
    [target, content, tagStr]
  )
}

function writeDreamingEntry(dir: string, content: string, tags: readonly string[]): string {
  ensureDreamingDir()
  const projectHash = createHash("sha256").update(dir).digest("hex").slice(0, 12)
  const path = join(DREAMING_DIR, `${projectHash}.md`)
  if (!existsSync(path)) {
    writeFileSync(path, `# Dreaming: 跨会话提炼\n\n`)
  }
  const existing = readFileSync(path, "utf-8")
  if (existing.includes(content)) return `(已存在: ${content.slice(0, 40)}...)`
  const tagStr = tags.length > 0 ? ` ${tags.join(" ")}` : ""
  const timestamp = new Date().toISOString().split("T")[0]
  const line = `- [${timestamp}] ${content}${tagStr}\n`
  writeFileSync(path, line, { flag: "a" })
  return `已写入 dreaming: ${projectHash}/${content.slice(0, 40)}...`
}

// ─── tool ──────────────────────────────────────────────

export const layer = Layer.effectDiscard(
  Effect.gen(function* () {
    const tools = yield* Tools.Service
    const permission = yield* PermissionV2.Service
    const location = yield* Location.Service

    yield* tools
      .register({
        [name]: Tool.make({
          description:
            "记录长期稳定记忆。仅用于已确认偏好/约束/结论/待办；临时排查、讨论草稿、一次性过程不要写。调用前应先 memory_review 查重。\n全局 memory (scope=user) - 跨项目共享，写入 .md 文件；\n项目 memory (scope=project) - 跟项目关联，写入 SQLite；\nDreaming (scope=dreaming) - 跨会话提炼的模式，写入 dreaming/ 目录。",
          input: Input,
          output: Output,
          execute: (input, context) =>
            Effect.gen(function* () {
              yield* permission.assert({
                action: name,
                resources: ["*"],
                save: ["*"],
                sessionID: context.sessionID,
                agent: context.agent,
                source: { type: "tool", messageID: context.assistantMessageID, callID: context.toolCallID },
              })

              const tags = input.tags ?? []
              const tagsLine = tags.length > 0 ? ` [${tags.join(" ")}]` : ""

              if (input.scope === "user") {
                if (!input.name) return "错误: scope=user 时必须指定 name: preferences | constraints | patterns | style"
                appendGlobalEntry(input.name, input.content, tags)
                return `已写入全局记忆: ${input.name}/${input.content}`
              }

              if (input.scope === "dreaming") {
                const projDir = location.directory
                return writeDreamingEntry(projDir, input.content, tags)
              }

              // scope=project
              if (!input.target) return "错误: scope=project 时必须指定 target: progress | TODO | tech | conclusion"
              const projDir = location.directory
              const db = ensureProjectDb(projDir)
              insertProjectMemory(db, input.target, input.content, tags)
              return `已写入项目记忆: ${input.target}/${input.content}`
            }).pipe(
              Effect.mapError((err) => {
                if (err instanceof ToolFailure) return err
                return new ToolFailure({ message: String(err) })
              }),
            ),
        }),
      })
      .pipe(Effect.orDie)
  }),
)
