/**
 * Structured system prompt template.
 *
 * Replaces the old `join("\n")` flat concatenation with semantic XML-like
 * `<section>` tags that help the LLM distinguish constraint types by priority.
 *
 * Rollback: set OPENCODE_DISABLE_STRUCTURED_PROMPT=true to use the old flat path.
 */

import { Flag } from "@opencode-ai/core/flag/flag"

// ─── types ──────────────────────────────────────────────

export interface SectionItem {
  /** Unique identifier within the section. e.g. "[Memory]", "src-AGENTS.md" */
  key: string
  /** Content text (one rule / one instruction block) */
  content: string
  /** Subsection heading. Only meaningful for sections that use headings. */
  subsection?: string
}

export type SectionId =
  | "constraint"
  | "identity"
  | "environment"
  | "instructions"
  | "capabilities"
  | "style"
  | "memory"
  | "nudge"

// ─── renderers (private) ────────────────────────────────

function renderConstraint(items: SectionItem[]): string {
  const subsections = new Map<string, string[]>()
  for (const item of items) {
    const key = item.subsection ?? "_default"
    if (!subsections.has(key)) subsections.set(key, [])
    subsections.get(key)!.push(`- [${item.key}] ${item.content}`)
  }
  const body: string[] = []
  for (const [heading, rules] of subsections) {
    if (heading !== "_default") body.push(`## ${heading}`)
    body.push(...rules)
    body.push("")
  }
  return wrap("constraint", body.join("\n").trimEnd())
}

function renderIdentity(content: string, mode?: string): string {
  const attr = mode ? ` mode="${mode}"` : ""
  return `<identity${attr}>\n${content.trim()}\n</identity>`
}

function renderEnvironment(items: SectionItem[]): string {
  if (isRaw(items)) return wrap("environment", items[0].content.trim())
  const lines = items.map((i) => `${i.key}: ${i.content}`)
  return wrap("environment", lines.join("\n"))
}

function renderInstructions(items: SectionItem[]): string {
  if (isRaw(items)) return wrap("instructions", items[0].content.trim())
  const blocks = items.map((i) => `<!-- source: ${i.key} -->\n${i.content}`)
  return wrap("instructions", blocks.join("\n\n"))
}

function renderCapabilities(items: SectionItem[]): string {
  if (isRaw(items)) return wrap("capabilities", items[0].content.trim())
  const skills = items.filter((i) => i.subsection === "Skills")
  const tools = items.filter((i) => i.subsection === "Tools")
  const body: string[] = []
  if (skills.length > 0) {
    body.push("## Skills")
    body.push(...skills.map((i) => `- ${i.key}: ${i.content}`))
    body.push("")
  }
  if (tools.length > 0) {
    body.push("## Tools")
    body.push(...tools.map((i) => `- ${i.key}: ${i.content}`))
    body.push("")
  }
  return wrap("capabilities", body.join("\n").trimEnd())
}

function renderStyle(items: SectionItem[]): string {
  if (isRaw(items)) return wrap("style", items[0].content.trim())
  const boundaries = items.filter((i) => i.subsection === "boundaries")
  const expression = items.filter((i) => i.subsection === "expression")
  const selfChecks = items.filter((i) => i.subsection === "self_checks")
  const parts: string[] = []
  if (boundaries.length > 0) {
    parts.push(
      wrap("boundaries", boundaries.map((i) => `- ${i.content}`).join("\n")),
    )
  }
  if (expression.length > 0) {
    parts.push(
      wrap("expression", expression.map((i) => `- ${i.content}`).join("\n")),
    )
  }
  if (selfChecks.length > 0) {
    parts.push(
      wrap("self_checks", selfChecks.map((i) => `- ${i.content}`).join("\n")),
    )
  }
  return `<style>\n${parts.join("\n")}\n</style>`
}

function renderMemory(content: string): string {
  return wrap("memory", content.trim())
}

function renderNudge(content: string): string {
  return wrap("nudge", content.trim())
}

// ─── helpers ────────────────────────────────────────────

function wrap(tag: string, body: string): string {
  return `<${tag}>\n${body}\n</${tag}>`
}

/** When content is set directly (not via upsert), items contain a single `_raw` entry. */
function isRaw(items: SectionItem[]): boolean {
  return items.length === 1 && items[0].key === "_raw"
}

const PRIORITY: Record<SectionId, number> = {
  constraint: 0,
  identity: 1,
  environment: 2,
  instructions: 3,
  capabilities: 4,
  style: 5,
  memory: 6,
  nudge: 7,
}

// ─── TemplateSection ────────────────────────────────────

export class TemplateSection {
  constructor(
    readonly id: SectionId,
    readonly priority: number = PRIORITY[id] ?? 99,
    public mode?: string,
    readonly items: SectionItem[] = [],
  ) {}

  upsert(item: SectionItem): this {
    const idx = this.items.findIndex((i) => i.key === item.key)
    if (idx >= 0) {
      this.items[idx] = item
    } else {
      this.items.push(item)
    }
    return this
  }

  remove(key: string): this {
    const idx = this.items.findIndex((i) => i.key === key)
    if (idx >= 0) this.items.splice(idx, 1)
    return this
  }

  clear(): this {
    this.items.length = 0
    return this
  }

  /** Full content replacement (for sections without item structure). */
  get content(): string {
    return this.items.map((i) => i.content).join("\n")
  }

  set content(value: string) {
    this.items.splice(0, this.items.length, { key: "_raw", content: value })
  }

  toString(): string {
    switch (this.id) {
      case "constraint":
        return renderConstraint(this.items)
      case "identity":
        return renderIdentity(this.content, this.mode)
      case "environment":
        return renderEnvironment(this.items)
      case "instructions":
        return renderInstructions(this.items)
      case "capabilities":
        return renderCapabilities(this.items)
      case "style":
        return renderStyle(this.items)
      case "memory":
        return renderMemory(this.content)
      case "nudge":
        return renderNudge(this.content)
    }
  }
}

// ─── Template ───────────────────────────────────────────

export class PromptTemplate {
  private readonly _sections = new Map<SectionId, TemplateSection>()

  get(id: SectionId): TemplateSection | undefined {
    return this._sections.get(id)
  }

  set(section: TemplateSection): this {
    this._sections.set(section.id, section)
    return this
  }

  /** Render all sections in priority order, each on its own line. */
  render(): string {
    return [...this._sections.values()]
      .sort((a, b) => a.priority - b.priority)
      .map((s) => s.toString())
      .filter(Boolean)
      .join("\n")
  }
}

// ─── feature flag ───────────────────────────────────────

/** True when structured template should be used instead of flat join. */
export function enabled(): boolean {
  return !Flag.OPENCODE_DISABLE_STRUCTURED_PROMPT
}
