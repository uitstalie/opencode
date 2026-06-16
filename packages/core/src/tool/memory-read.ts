export * as MemoryReadTool from "./memory-read"

import { ToolFailure } from "@opencode-ai/llm"
import { Effect, Layer, Schema } from "effect"
import { PermissionV2 } from "../permission"
import { Tool } from "./tool"
import { Tools } from "./tools"
import { Location } from "../location"
import { join } from "node:path"
import { existsSync, readFileSync, readdirSync } from "node:fs"
import { createHash } from "node:crypto"
import { Database } from "bun:sqlite"

export const name = "memory_read"

export const Input = Schema.Struct({
  scope: Schema.Literals(["user", "project"]).annotate({ description: "必须二选一: user | project" }),
  name: Schema.optional(Schema.String).annotate({ description: "scope=user 时指定分类: preferences|constraints|patterns|style；layer=dreaming 时可指定项目 hash" }),
  target: Schema.optional(Schema.String).annotate({ description: "仅 scope=project 使用。可选: progress | TODO | tech | conclusion；省略则读取全部项目记忆" }),
  layer: Schema.optional(Schema.String).annotate({ description: "可选: raw | dreaming；默认 raw。dreaming 读取跨会话提炼的记忆" }),
  search: Schema.optional(Schema.String).annotate({ description: "搜索关键词，用于在记忆内容中快速查找" }),
})

export const Output = Schema.String
export type Output = typeof Output.Type

const GLOBAL_MEM_DIR = join(process.env.HOME ?? "/tmp", ".config", "opencode", "memory")
const DREAMING_DIR = join(GLOBAL_MEM_DIR, "dreaming")

function readGlobalMemory(category?: string): string {
  if (!existsSync(GLOBAL_MEM_DIR)) return "(全局记忆目录尚未创建)\n"
  if (category) {
    const path = join(GLOBAL_MEM_DIR, `${category}.md`)
    if (!existsSync(path)) return `# ${category}\n\n(空)\n`
    return readFileSync(path, "utf-8")
  }
  const cats = ["preferences", "constraints", "patterns", "style"]
  const parts: string[] = []
  for (const cat of cats) {
    const path = join(GLOBAL_MEM_DIR, `${cat}.md`)
    if (!existsSync(path)) continue
    parts.push(readFileSync(path, "utf-8"))
  }
  return parts.join("\n") || "(空)"
}

function readDreaming(dir: string, projectHash?: string): string {
  if (!existsSync(DREAMING_DIR)) return "# Dreaming 记忆\n\n(暂无 dreaming 数据)\n"
  let files = readdirSync(DREAMING_DIR).filter(f => f.endsWith(".md"))
  if (projectHash) {
    files = files.filter(f => f.startsWith(projectHash))
    if (files.length === 0) return `# Dreaming (${projectHash})\n\n(暂无此项目的 dreaming 数据)\n`
  }
  const parts: string[] = [`# Dreaming 记忆\n\n`]
  for (const file of files.sort()) {
    const content = readFileSync(join(DREAMING_DIR, file), "utf-8")
    parts.push(content)
  }
  return parts.join("\n")
}

function queryProjectMemory(dir: string, target?: string): string {
  const dbPath = join(dir, ".opencode", "memory", "memory.db")
  if (!existsSync(dbPath)) return "# 项目记忆\n\n(空)\n"
  const db = new Database(dbPath)
  const sql = target
    ? `SELECT target, content, tags, created_at FROM project_memory WHERE target = ? ORDER BY created_at DESC`
    : `SELECT target, content, tags, created_at FROM project_memory ORDER BY target, created_at DESC`
  const rows = target ? db.prepare(sql).all(target) as {target:string,content:string,tags:string,created_at:string}[] : db.prepare(sql).all() as {target:string,content:string,tags:string,created_at:string}[]
  if (!rows || rows.length === 0) return "# 项目记忆\n\n(空)\n"
  let result = `# 项目记忆\n\n`
  for (const row of rows) {
    result += `- [${row.target}] ${row.content}`
    if (row.tags) result += ` ${row.tags}`
    result += `\n`
  }
  return result
}

export const layer = Layer.effectDiscard(
  Effect.gen(function* () {
    const tools = yield* Tools.Service
    const permission = yield* PermissionV2.Service
    const location = yield* Location.Service

    yield* tools
      .register({
        [name]: Tool.make({
          description:
            "读取记忆。制定方案/风格/习惯决策前优先用。全局 memory (scope=user) — 从 .md 文件读取；项目 memory (scope=project) — 从 SQLite 读取。可指定 target/name 过滤，或 search 搜索。",
          input: Input,
          output: Output,
          execute: (input, context) =>
            Effect.gen(function* () {
              yield* permission.assert({
                action: name,
                resources: ["*"],
                sessionID: context.sessionID,
                agent: context.agent,
                source: { type: "tool", messageID: context.assistantMessageID, callID: context.toolCallID },
              })

              if (input.scope === "user") {
                if (input.layer === "dreaming") {
                  const projDir = location.directory
                  const projHash = createHash("sha256").update(projDir).digest("hex").slice(0, 12)
                  return readDreaming(projDir, input.name ?? projHash)
                }
                return readGlobalMemory(input.name)
              }

              const projDir = location.directory
              const raw = queryProjectMemory(projDir, input.target)
              if (input.search) {
                const searchLower = input.search.toLowerCase()
                const filtered = raw.split("\n").filter(l => l.toLowerCase().includes(searchLower)).join("\n")
                return filtered || "(未找到匹配项)"
              }
              return raw
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
