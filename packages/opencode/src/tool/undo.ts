/**
 * Undo blob store backed by isomorphic-git.
 *
 * Stores file snapshots as git blobs in a bare repository so files changed
 * by AI tools (edit, write, apply_patch) can be restored without requiring
 * a system git installation.
 *
 * Storage:  {Global.Path.data}/undo-blobs/.git/objects/
 * Blob API: git write-blob → oid  /  git read-blob → content
 *
 * GC: blobs older than TTL (default 24h) are cleaned up periodically.
 */

import git from "isomorphic-git"
import fs from "node:fs"
import fsp from "node:fs/promises"
import path from "path"
import { Effect, Context, Layer } from "effect"
import { LayerNode } from "@opencode-ai/core/effect/layer-node"
import { Global } from "@opencode-ai/core/global"

const undoDir = path.join(Global.Path.data, "undo-blobs")
const gitdir = path.join(undoDir, ".git")

/** Blob TTL in milliseconds (24 hours) */
const BLOB_TTL_MS = 24 * 60 * 60 * 1000

/** Run GC every N saves */
const GC_INTERVAL = 10

// singleton init — isomorphic-git needs a bare git directory for blob storage
let initPromise: Promise<void> | undefined

function ensureInit() {
  if (!initPromise) {
    initPromise = fsp.mkdir(undoDir, { recursive: true }).then(() =>
      git.init({ fs, dir: undoDir, gitdir, bare: true }).catch(() => {}),
    )
  }
  return initPromise
}

/** Clean up blobs older than TTL by scanning loose object files */
async function runGC() {
  const objectsDir = path.join(gitdir, "objects")
  const now = Date.now()
  const cutoff = now - BLOB_TTL_MS

  try {
    const entries = await fsp.readdir(objectsDir, { withFileTypes: true })
    for (const entry of entries) {
      if (!entry.isDirectory() || entry.name.length !== 2) continue
      const subdir = path.join(objectsDir, entry.name)
      const files = await fsp.readdir(subdir, { withFileTypes: true })
      for (const file of files) {
        if (file.isDirectory()) continue
        const filePath = path.join(subdir, file.name)
        try {
          const stat = await fsp.stat(filePath)
          if (stat.mtimeMs < cutoff) {
            await fsp.unlink(filePath)
          }
        } catch {
          // file may have been deleted concurrently
        }
      }
      // remove empty subdirectories
      try {
        const remaining = await fsp.readdir(subdir)
        if (remaining.length === 0) await fsp.rmdir(subdir)
      } catch {
        // ignore
      }
    }
  } catch {
    // objects dir may not exist yet
  }
}

// ─── service interface ───────────────────────────────────

export interface Interface {
  /** Save the content of `filePath` as a git blob. Returns the blob oid (SHA-1 hex). */
  readonly saveFileBlob: (filePath: string) => Effect.Effect<string, Error>
  /** Restore `filePath` from blob `oid`. Returns the oid of the previous state (for redo). */
  readonly restoreFileBlob: (filePath: string, oid: string) => Effect.Effect<string, Error>
}

export class Service extends Context.Service<Service, Interface>()("@opencode/Undo") {}

// ─── layer ───────────────────────────────────────────────

export const layer = Layer.effect(
  Service,
  Effect.gen(function* () {
    yield* Effect.tryPromise(() => ensureInit())

    let saveCount = 0

    const saveFileBlob = Effect.fn("Undo.saveFileBlob")(function* (filePath: string) {
      const exists = yield* Effect.tryPromise(async () => {
        try { await fsp.access(filePath); return true }
        catch { return false }
      })
      if (!exists) return ""
      const content = yield* Effect.tryPromise(() => fsp.readFile(filePath))
      const oid = yield* Effect.tryPromise(() =>
        git.writeBlob({ fs, dir: undoDir, gitdir, blob: new Uint8Array(content) }),
      )
      saveCount++
      if (saveCount % GC_INTERVAL === 0) {
        yield* Effect.tryPromise(() => runGC())
      }
      return oid
    })

    const restoreFileBlob = Effect.fn("Undo.restoreFileBlob")(function* (filePath: string, oid: string) {
      // Save current state so it can be redone
      const redoOid = yield* saveFileBlob(filePath)

      const result = yield* Effect.tryPromise(() =>
        git.readBlob({ fs, dir: undoDir, gitdir, oid }),
      )
      yield* Effect.tryPromise(async () => {
        await fsp.mkdir(path.dirname(filePath), { recursive: true })
        await fsp.writeFile(filePath, result.blob)
      })

      return redoOid
    })

    return { saveFileBlob, restoreFileBlob } satisfies Interface
  }),
)

export const defaultLayer = layer

export const node = LayerNode.make(layer, [])

export * as Undo from "./undo"
