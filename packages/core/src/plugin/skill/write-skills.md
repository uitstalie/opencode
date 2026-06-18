<!--
  Built-in skill. Name and description are registered in code at
  packages/core/src/plugin/skill.ts. The body below becomes the
  skill's content.
-->

# Writing Skills

opencode skills are Markdown files with YAML frontmatter. Writing them
correctly avoids silent load failures — the most common reason a skill
doesn't appear is broken frontmatter.

## File structure

```
.opencode/skills/<name>/SKILL.md     (project-level)
~/.config/opencode/skills/<name>/SKILL.md  (global)
```

- The folder name **must** match the `name` field in frontmatter.
- The file **must** be named `SKILL.md` exactly.
- One skill per folder.

## Frontmatter

```yaml
---
name: my-skill
description: "One sentence covering what this skill does AND when to trigger it."
---
```

### Required fields

| Field | Rules |
|-------|-------|
| `name` | Lowercase, hyphen-separated, ≤ 64 chars. Must match folder name. |
| `description` | Required for the skill to appear. Cover **what** it does and **when** to use it. Write in third person ("Use when…"). Front-load trigger keywords. |

### YAML quoting rules (CRITICAL)

YAML is parsed by js-yaml. Unquoted values with these characters will
break parsing **silently** — the skill loads with empty data and never
appears:

- **Colons followed by space** (`: `) — YAML interprets this as a
  nested mapping key. Always quote the entire value:
  ```yaml
  # ❌ description: For X: do Y
  # ✅ description: "For X: do Y"
  ```

- **Leading special chars** — `@`, `#`, `%`, `` ` ``, `{`, `}`, `[`,
  `]`, `,`, `&`, `*`, `?`, `|`, `-`, `<`, `>`, `=`, `!`, `%` can
  cause parse issues in certain positions. When in doubt, quote.

- **Values that look like YAML syntax** — `true`, `false`, `yes`, `no`,
  `null`, numbers, or ISO dates should be quoted:
  ```yaml
  # ❌ version: 1.0      → parsed as number 1
  # ✅ version: "1.0"
  ```

**Rule of thumb**: if your description contains `:`, quote the whole
thing with double quotes. It never hurts and prevents silent failures.

### Optional fields

| Field | Rules |
|-------|-------|
| `license` | SPDX identifier or URL |
| `compatibility` | Version range string |
| `metadata` | Flat string→string map for arbitrary tags |

Unknown frontmatter fields are silently ignored.

## File encoding

- **UTF-8 without BOM**. A BOM at the start of the file will cause
  the frontmatter regex to miss the `---` markers.
- Line endings: LF (`\n`) preferred. CRLF (`\r\n`) is tolerated by
  the sanitizer but can cause edge cases.

## Description best practices

The `description` field determines whether the skill appears in the
agent's system prompt. Skills without a description are filtered out.

A good description:
1. **Says what the skill does and when to use it** — not just one or
   the other.
2. **Front-loads trigger keywords** — filenames, commands, or terms
   the user is likely to say that should trigger this skill.
3. **Uses third person** — "Use when…", not "I help with…".
4. **Gates with "Use ONLY when…"** if the skill should stay quiet on
   adjacent topics.
5. **Keeps it to one sentence** (though longer descriptions are allowed).

Examples:
```yaml
# ✅ Good: specific, front-loaded trigger words, third person
description: "Use when working with Effect v4 TypeScript code. Triggered by 'Effect', 'Layer', 'Schema', 'Effect.gen'."

# ✅ Good: gate with ONLY
description: "Use ONLY when editing opencode.json, .opencode/, or ~/.config/opencode/. Do not use for application code."

# ❌ Bad: no trigger keywords, vague
description: "Helps with some stuff."

# ❌ Bad: first person
description: "I help you write plugins."
```

## Body content

The body after the `---` closing marker is Markdown. It becomes the
skill's content injected when the agent loads this skill.

- Use headings, lists, code blocks normally.
- Keep it focused — the skill is injected as context, so verbosity
  costs tokens.
- Prefer concrete examples and checklists over prose.

## Common failure modes

| Symptom | Likely cause |
|---------|-------------|
| Skill file exists but doesn't appear | Unquoted `:` in description (YAML parse failure) |
| Skill appears but has wrong name | Folder name ≠ frontmatter `name` |
| Skill loads but body is empty | Missing `---` closing marker, or malformed frontmatter |
| BOM at file start | First `---` not detected, entire file treated as body |
| Description is truncated | Unquoted `:` caused YAML to split the value |

## Quick checklist before saving

- [ ] File is `SKILL.md` in a folder matching `name`
- [ ] `name` is lowercase, hyphen-separated
- [ ] `description` is present and quoted if it contains `:`
- [ ] File is UTF-8 without BOM
- [ ] Frontmatter has opening and closing `---` on their own lines
- [ ] Description uses third person with trigger keywords
