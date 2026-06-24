import { Schema } from "effect"
import { Effect } from "effect"
import path from "path"
import * as Tool from "./tool"
import { InstanceState } from "@/effect/instance-state"
import { Undo } from "./undo"
import { assertExternalDirectoryEffect } from "./external-directory"
import { EventV2Bridge } from "@/event-v2-bridge"
import { FileSystem } from "@opencode-ai/core/filesystem"
import { Watcher } from "@opencode-ai/core/filesystem/watcher"
import DESCRIPTION from "./undo-edit.txt"

export const Parameters = Schema.Struct({
  filePath: Schema.String.annotate({
    description: "The absolute path to the file to restore",
  }),
  undoHash: Schema.String.annotate({
    description: "The undoHash from a previous edit/write/apply_patch or undo_edit tool result metadata. Use the returned undoHash to chain further undo calls.",
  }),
})

export const UndoEditTool = Tool.define(
  "undo_edit",
  Effect.gen(function* () {
    const undo = yield* Undo.Service
    const events = yield* EventV2Bridge.Service

    const run = Effect.fn("UndoEditTool.execute")(function* (
      params: Schema.Schema.Type<typeof Parameters>,
      ctx: Tool.Context,
    ) {
      const instance = yield* InstanceState.context
      const filePath = path.isAbsolute(params.filePath)
        ? params.filePath
        : path.join(instance.directory, params.filePath)

      yield* assertExternalDirectoryEffect(ctx, filePath)

      yield* ctx.ask({
        permission: "edit",
        patterns: [path.relative(instance.worktree, filePath)],
        always: ["*"],
        scope: filePath,
        projectRoot: instance.worktree,
        metadata: {
          filepath: filePath,
          undoHash: params.undoHash,
        },
      })

      const redoHash = yield* undo.restoreFileBlob(filePath, params.undoHash)

      yield* events.publish(FileSystem.Event.Edited, { file: filePath })
      yield* events.publish(Watcher.Event.Updated, {
        file: filePath,
        event: "change",
      })

      return {
        title: `Undid changes to ${path.relative(instance.worktree, filePath)}`,
        output: `Restored ${filePath} to previous state. Use undo_edit with undoHash ${redoHash} to reverse this undo.`,
        metadata: {
          file: filePath,
          undoHash: redoHash,
        },
      }
    })

    return {
      description: DESCRIPTION,
      parameters: Parameters,
      execute: (params: Schema.Schema.Type<typeof Parameters>, ctx: Tool.Context) =>
        run(params, ctx).pipe(Effect.orDie),
    }
  }),
)

export * as UndoEdit from "./undo-edit"
