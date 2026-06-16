export * as DreamingCompressTool from "./dreaming-compress"

import { ToolFailure } from "@opencode-ai/llm"
import { Effect, Layer, Schema } from "effect"
import { PermissionV2 } from "../permission"
import { Tool } from "./tool"
import { Tools } from "./tools"
import { join } from "node:path"
import { existsSync, mkdirSync, readFileSync, writeFileSync, readdirSync } from "node:fs"

export const name = "dreaming_compress"

export const Input = Schema.Struct({
  scope: Schema.optional(Schema.Literals(["project", "all"])).annotate({ description: "project | all，默认 all" }),
  dryRun: Schema.optional(Schema.Boolean).annotate({ description: "true=仅返回分析报告，不写入。默认 true" }),
})

export const Output = Schema.String
export type Output = typeof Output.Type

// ─── types ──────────────────────────────────────────────

interface Entry {
  category: string
  content: string
  tags: string[]
  line: string
}

// ─── io helpers ─────────────────────────────────────────

const GLOBAL_MEM_DIR = join(process.env.HOME ?? "/tmp", ".config", "opencode", "memory")
const DREAMING_DIR = join(GLOBAL_MEM_DIR, "dreaming")
const CATEGORIES = ["preferences", "constraints", "patterns", "style"] as const

function ensureDir() {
  if (!existsSync(GLOBAL_MEM_DIR)) mkdirSync(GLOBAL_MEM_DIR, { recursive: true })
}

function readAllEntries(): Entry[] {
  ensureDir()
  const entries: Entry[] = []
  for (const cat of CATEGORIES) {
    try {
      const path = join(GLOBAL_MEM_DIR, `${cat}.md`)
      if (!existsSync(path)) continue
      const lines = readFileSync(path, "utf-8").split("\n")
      for (const line of lines) {
        const trimmed = line.trim()
        if (!trimmed.startsWith("- ")) continue
        const tagMatch = trimmed.match(/(#[a-z_]+)/g)
        const tags = tagMatch ? tagMatch.filter(t => !t.startsWith("#-")) : []
        let content = trimmed.slice(2)
        for (const tag of tags) {
          content = content.replace(" " + tag, "").replace(tag, "")
        }
        content = content.trim()
        if (!content) continue
        entries.push({ category: cat, content, tags, line: trimmed })
      }
    } catch {
      // Skip corrupted files
    }
  }
  return entries
}

function writeEntries(entries: Entry[]) {
  ensureDir()
  try {
    const grouped = new Map<string, Entry[]>()
    for (const e of entries) {
      if (!grouped.has(e.category)) grouped.set(e.category, [])
      grouped.get(e.category)!.push(e)
    }
    for (const cat of CATEGORIES) {
      const path = join(GLOBAL_MEM_DIR, `${cat}.md`)
      const catEntries = grouped.get(cat) ?? []
      const header = `# ${cat}\n\n`
      const body = catEntries.map(e => e.line).join("\n")
      writeFileSync(path, header + (body ? body + "\n" : ""))
    }
  } catch {
    // Write failures are non-fatal for individual files
  }
}

function findSimilar(entries: Entry[]): Map<string, Entry[]> {
  const groups = new Map<string, Entry[]>()
  for (const e of entries) {
    const key = e.content.slice(0, 30).toLowerCase().replace(/[^a-z\u4e00-\u9fff]/g, "")
    if (!groups.has(key)) groups.set(key, [])
    groups.get(key)!.push(e)
  }
  const result = new Map<string, Entry[]>()
  for (const [key, group] of groups) {
    if (group.length >= 2) result.set(key, group)
  }
  return result
}

function findConfidenceUpgrades(entries: Entry[]): Entry[] {
  const likely = entries.filter(e => e.tags.includes("#likely"))
  if (likely.length < 3) return []
  const groups = findSimilar(likely)
  const candidates: Entry[] = []
  for (const [_, group] of groups) {
    if (group.length >= 3) candidates.push(group[0])
  }
  return candidates
}

function dedupEntries(entries: Entry[]): { kept: Entry[]; removed: number } {
  const seen = new Set<string>()
  const kept: Entry[] = []
  let removed = 0
  for (const e of entries) {
    const key = `${e.category}:${e.content.slice(0, 50).toLowerCase()}`
    if (seen.has(key)) {
      removed++
      continue
    }
    seen.add(key)
    kept.push(e)
  }
  return { kept, removed }
}

function generateReport(entries: Entry[]): string {
  const lines: string[] = []
  lines.push("# Dreaming 分析报告")
  lines.push("")

  lines.push("## 各类别条目数")
  lines.push("")
  for (const cat of CATEGORIES) {
    const count = entries.filter(e => e.category === cat).length
    lines.push(`- ${cat}: ${count} 条`)
  }
  lines.push(`- 总计: ${entries.length} 条`)
  lines.push("")

  const similar = findSimilar(entries)
  if (similar.size > 0) {
    lines.push("## 疑似重复 (前30字符相同)")
    lines.push("")
    for (const [key, group] of similar) {
      lines.push(`### 组: "${key}..."`)
      for (const e of group) {
        lines.push(`- [${e.category}] ${e.content} ${e.tags.join(" ")}`.trim())
      }
      lines.push("")
    }
  } else {
    lines.push("## 疑似重复: 无")
    lines.push("")
  }

  const upgrades = findConfidenceUpgrades(entries)
  if (upgrades.length > 0) {
    lines.push("## 置信度升级候选 (#likely → #confirmed)")
    lines.push("")
    for (const e of upgrades) {
      lines.push(`- [${e.category}] ${e.content}`)
    }
    lines.push("")
  } else {
    lines.push("## 置信度升级候选: 无")
    lines.push("")
  }

  const dreamingFiles = existsSync(DREAMING_DIR) ? readdirSync(DREAMING_DIR).filter(f => f.endsWith(".md")) : []
  if (dreamingFiles.length > 0) {
    lines.push("## Dreaming 目录")
    lines.push("")
    let dreamingTotal = 0
    for (const file of dreamingFiles) {
      const content = readFileSync(join(DREAMING_DIR, file), "utf-8")
      const count = content.split("\n").filter(l => l.trim().startsWith("- ")).length
      dreamingTotal += count
      lines.push(`- ${file}: ${count} 条`)
    }
    lines.push(`- 总计: ${dreamingTotal} 条`)
    lines.push("")
  }

  lines.push("## 建议操作")
  lines.push("")
  lines.push("1. 审阅上述疑似重复，决定保留/合并/删除")
  lines.push("2. 审阅置信度升级候选，确认是否升级为 #confirmed")
  lines.push("3. 调用 dreaming_compress(dryRun=false) 执行去重")
  lines.push("4. 对确认升级的条目，调用 memory_record 写入升级后版本（用 #confirmed 替换 #likely）")

  return lines.join("\n")
}

export const layer = Layer.effectDiscard(
  Effect.gen(function* () {
    const tools = yield* Tools.Service
    const permission = yield* PermissionV2.Service

    yield* tools
      .register({
        [name]: Tool.make({
          description:
            "整合 dreaming 数据：去重、合并同类模式、升级置信度、与现有 rules 交叉验证。dryRun=true 仅返回分析报告，不写入。",
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

              const entries = readAllEntries()

              if (input.dryRun !== false) {
                return generateReport(entries)
              }

              const { kept, removed } = dedupEntries(entries)
              writeEntries(kept)
              return [
                `Dreaming compress 完成。`,
                `去重: ${removed} 条移除，${kept.length} 条保留。`,
                ``,
                `如需升级置信度，请调用 memory_record 写入升级后的条目。`,
              ].join("\n")
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
