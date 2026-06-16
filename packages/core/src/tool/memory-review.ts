export * as MemoryReviewTool from "./memory-review"

import { ToolFailure } from "@opencode-ai/llm"
import { Effect, Layer, Schema } from "effect"
import { PermissionV2 } from "../permission"
import { Tool } from "./tool"
import { Tools } from "./tools"
import { Location } from "../location"
import { join } from "node:path"
import { existsSync, readFileSync } from "node:fs"
import { Database } from "bun:sqlite"

export const name = "memory_review"

export const Input = Schema.Struct({
  scope: Schema.optional(Schema.Literals(["user", "project"])).annotate({ description: "project | user，默认两者都显示" }),
  layer: Schema.optional(Schema.String).annotate({ description: "raw | dreaming | all，默认 all" }),
})

export const Output = Schema.String
export type Output = typeof Output.Type

const GLOBAL_MEM_DIR = join(process.env.HOME ?? "/tmp", ".config", "opencode", "memory")

function reviewGlobalMemory(): string {
  const cats = ["preferences", "constraints", "patterns", "style"]
  let report = "# 记忆审阅\n\n"
  let total = 0
  for (const cat of cats) {
    const path = join(GLOBAL_MEM_DIR, `${cat}.md`)
    if (!existsSync(path)) continue
    const lines = readFileSync(path, "utf-8")
      .split("\n")
      .filter(l => l.trim().startsWith("- "))
    total += lines.length
    report += `- ${cat}: ${lines.length} 条\n`
  }
  report += `\n总计: ${total} 条全局记忆\n`
  return report
}

function reviewProjectMemory(dir: string): string {
  const dbPath = join(dir, ".opencode", "memory", "memory.db")
  if (!existsSync(dbPath)) return "# 审阅\n\n项目记忆数据库不存在\n"
  const db = new Database(dbPath)
  const rows = db.prepare("SELECT target, count(*) as cnt FROM project_memory GROUP BY target").all() as {target:string,cnt:number}[]
  let report = "# 记忆审阅\n\n## 项目层 (SQLite)\n"
  if (!rows || rows.length === 0) {
    report += "- 暂无记录\n"
  } else {
    let total = 0
    for (const row of rows) {
      report += `- ${row.target}: ${row.cnt} 条\n`
      total += row.cnt
    }
    report += `\n总计: ${total} 条项目记忆\n`
  }
  return report
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
            "审阅 memory 状态。写入 memory_record 前必须优先调用它查重。全局查 .md 文件统计，项目查 SQLite 统计。",
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

              const projDir = location.directory
              const parts: string[] = []

              if (!input.scope || input.scope === "user") {
                parts.push(reviewGlobalMemory())
              }
              if (!input.scope || input.scope === "project") {
                parts.push(reviewProjectMemory(projDir))
              }

              return parts.join("\n\n") || "(无记忆)"
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
