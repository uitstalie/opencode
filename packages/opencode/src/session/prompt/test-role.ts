// Quick inline test of role.ts functionality
import { readFileSync, existsSync } from "node:fs"
import { join } from "node:path"
import { homedir } from "node:os"

const CONFIG_DIR = join(homedir(), ".config", "opencode", "plugin-config", "runtime-orchestrator")
const CONFIG_PATH = join(CONFIG_DIR, "config.json")

console.log("CONFIG_DIR:", CONFIG_DIR)
console.log("exists:", existsSync(CONFIG_PATH))

const config = JSON.parse(readFileSync(CONFIG_PATH, "utf-8"))
console.log("activeRole:", config.activeRole)
console.log("enableRole:", config.enableRole)

const rolePath = join(CONFIG_DIR, "roles", `${config.activeRole}.md`)
console.log("role exists:", existsSync(rolePath))

const content = readFileSync(rolePath, "utf-8")
const stripped = content
  .replace(/^## moodPrefer\n(?:.+\n)*/gm, "")
  .replace(/^## mood_push 策略\n(?:.+\n)*/gm, "")
  .replace(/\n{3,}/g, "\n\n")
  .trim()

console.log("\n=== STRIPPED ROLE CONTENT (first 500 chars) ===")
console.log(stripped.substring(0, 500))
