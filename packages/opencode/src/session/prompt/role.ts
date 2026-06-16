/**
 * Prompt role loader — reads active role from user config and returns
 * style-ready content for injection into the system prompt.
 *
 * This replaces the runtime-orchestrator plugin's role injection.
 * Role definition files live at ~/.config/opencode/plugin-config/runtime-orchestrator/roles/*.md
 */

import { readFileSync, existsSync } from "node:fs"
import { join } from "node:path"
import { homedir } from "node:os"

const CONFIG_DIR = join(homedir(), ".config", "opencode", "plugin-config", "runtime-orchestrator")
const CONFIG_PATH = join(CONFIG_DIR, "config.json")

interface RoleConfig {
  activeRole: string
  roles: string[]
  enableRole: boolean
}

/** Escape hatch: set OPENCODE_DISABLE_ROLE_STYLE=true to skip role injection. */
function roleEnabled(): boolean {
  return process.env["OPENCODE_DISABLE_ROLE_STYLE"]?.toLowerCase() !== "true"
}

function loadConfig(): RoleConfig | null {
  try {
    if (!existsSync(CONFIG_PATH)) return null
    return JSON.parse(readFileSync(CONFIG_PATH, "utf-8"))
  } catch {
    return null
  }
}

function loadRoleContent(name: string): string | null {
  try {
    const filepath = join(CONFIG_DIR, "roles", `${name}.md`)
    if (!existsSync(filepath)) return null
    return readFileSync(filepath, "utf-8")
  } catch {
    return null
  }
}

/** Strip plugin-specific sections (moodPrefer, mood_push) — keep style content. */
function stripPluginSections(md: string): string {
  return md
    .replace(/^## moodPrefer\n(?:.+\n)*/gm, "")
    .replace(/^## mood_push 策略\n(?:.+\n)*/gm, "")
    .replace(/\n{3,}/g, "\n\n")
    .trim()
}

/** Load the active role content suitable for prompt injection. */
export function loadRoleForPrompt(): string | null {
  if (!roleEnabled()) return null
  const config = loadConfig()
  if (!config?.activeRole) return null
  const content = loadRoleContent(config.activeRole)
  if (!content) return null
  return stripPluginSections(content)
}

export * as PromptRole from "./role"
