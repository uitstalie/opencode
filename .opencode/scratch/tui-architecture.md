# TUI Package Architecture Summary

> **Source repo**: `anomalyco/opencode` (forked at `uitstalie/opencode`)
> **Package**: `@opencode-ai/tui` v1.17.9 (at `packages/tui/`)
> **Analysis date**: 2026-06-24

---

## 1. Package Size

| Metric | Count |
|---|---|
| TS/TSX source files | **147** |
| Total lines of code | **~27,293** |
| Test files | ~50+ |

### Lines by Subdirectory

| Directory | Lines | Description |
|---|---|---|
| `src/component/` | 7,581 | Dialogs, prompt components, reusable widgets |
| `src/routes/` | 4,625 | Home & session views (session/index.tsx alone is 2,648 lines) |
| `src/feature-plugins/` | 3,453 | Diff viewer, sidebar, which-key, notifications |
| `src/context/` | 3,221 | State providers (sync, data, SDK, theme, etc.) |
| `src/ui/` | 2,029 | Dialog primitives, toast, spinner, link, border |
| `src/theme/` | 1,089 | Theme assets & index |
| `src/util/` | 893 | Helpers (renderer, session, format, transcript, etc.) |
| `src/plugin/` | 661 | Plugin runtime, slots, adapters, API, command shim |
| `src/config/` | 594 | TUI config, keybind configuration |
| `src/prompt/` | 386 | Prompt traits, display, stash, history, parts, frecency |

### Key Entry Points

| Export path | Source file | Purpose |
|---|---|---|
| `.` | `src/index.tsx` → `src/app.tsx` | Main `run()` function (Effect-based entry) |
| `./runtime` | `src/runtime.tsx` | TUI startup paths & environment |
| `./config` | `src/config/index.tsx` | Config resolution |
| `./context/sdk` | `src/context/sdk.tsx` | SDK client + SSE connection |
| `./context/sync` | `src/context/sync.tsx` | Global state synchronization (655 lines) |
| `./context/data` | `src/context/data.tsx` | V2 event-based data layer (581 lines) |
| `./plugin/runtime` | `src/plugin/runtime.tsx` | TUI plugin host |
| `./builtins` | `src/feature-plugins/builtins.ts` | Built-in feature plugins |
| `./keymap` | `src/keymap.tsx` | Keybind/command system |
| `./prompt/display` | `src/prompt/display.ts` | Prompt rendering |
| `./editor` | `src/editor.ts` | External editor integration |

---

## 2. Tech Stack

| Category | Technology | Version | Notes |
|---|---|---|---|
| **UI framework** | **SolidJS** | 1.9.10 | Patched (`patches/solid-js@1.9.10.patch`) |
| **TUI renderer** | **@opentui/solid** | 0.3.4 | Internal/proprietary TUI framework built on SolidJS |
| **TUI core** | **@opentui/core** | 0.3.4 | Terminal rendering engine (`CliRenderer`, `BoxRenderable`, `ScrollBoxRenderable`, `DiffRenderable`) |
| **Keymap** | **@opentui/keymap** | 0.3.4 | Keybind dispatching and mode system |
| **Spinner** | **opentui-spinner** | 0.0.7 | Terminal spinner |
| **Async runtime** | **Effect** | 4.0.0-beta.83 | Functional effect system for scoped resources, error handling |
| **State management** | **SolidJS stores** | 1.9.10 | `createStore` / `produce` / `reconcile` |
| **Diffing** | **diff** | 8.0.2 | Text diff generation |
| **Fuzzy matching** | **fuzzysort** | 3.1.0 | Command palette fuzzy search |
| **Utilities** | **remeda** | 2.26.0 | Functional data utilities |
| **CLI clipboard** | **clipboardy** | 4.0.0 | System clipboard access |
| **ANSI stripping** | **strip-ansi** | 7.1.2 | Strip ANSI escape codes from output |
| **External open** | **open** | 10.1.2 | Open URLs in browser |
| **Syntax highlighting** | **tree-sitter** WASM | various | ~30 language parsers loaded as WASM (see `parsers-config.ts`) |

### What is @opentui?

`@opentui/core`, `@opentui/solid`, and `@opentui/keymap` are **internal/proprietary npm packages** (v0.3.4) — they are NOT in this monorepo. They are published separately and consumed as regular npm dependencies from the catalog. They provide:

- `createCliRenderer()` — creates a terminal renderer with kitty keyboard protocol, mouse support, console, FPS control
- `render(jsx, renderer)` — mounts a SolidJS component tree onto the terminal
- `BoxRenderable`, `ScrollBoxRenderable`, `DiffRenderable` — terminal primitives
- `useTerminalDimensions()`, `useRenderer()` — SolidJS hooks
- Keymap system with modes/bindings/priorities

There is a repo-level script for upgrading opentui: `script/upgrade-opentui.ts`.

---

## 3. Architecture

### 3.1 Communication with Backend: REST + SSE (NOT gRPC/WebSocket/stdio)

The TUI is a **client-only** process. It communicates with the OpenCode server via:

1. **REST API** — `@opencode-ai/sdk/v2` generates a typed HTTP client (`createClient()` from OpenAPI spec). All API calls go through `sdk.client.session.get()`, `sdk.client.config.get()`, etc.

2. **Server-Sent Events (SSE)** — The SDK connects to `sdk.global.event()` which returns a streaming SSE endpoint. All state changes (messages, parts, sessions, permissions, todos, LSP status, etc.) are pushed from the server to the TUI as `GlobalEvent` objects.

3. **Event batching** — Incoming SSE events are queued and flushed in batches with a 16ms throttle window to reduce rendering overhead. The `batch()` SolidJS primitive is used to group store updates.

### 3.2 Component Tree (Provider Hierarchy)

The app wraps everything in a deep provider tree (see `app.tsx` lines 237-336):

```
ExitProvider
  EpilogueProvider
    ErrorBoundary
      TuiPathsProvider
        TuiTerminalEnvironmentProvider
          TuiStartupProvider
            ClipboardProvider
              OpencodeKeymapProvider
                ArgsProvider
                  KVProvider
                    ToastProvider
                      RouteProvider
                        TuiConfigProvider
                          PluginRuntimeProvider
                            SDKProvider
                              ProjectProvider
                                SyncProvider
                                  DataProvider
                                    ThemeProvider
                                      LocalProvider
                                        PromptStashProvider
                                          DialogProvider
                                            FrecencyProvider
                                              PromptHistoryProvider
                                                PromptRefProvider
                                                  EditorContextProvider
                                                    <App />
```

### 3.3 Key Modules

#### State Synchronization (`context/sync.tsx` — 655 lines)
- Two-phase bootstrap: **blocking** (providers, agents, config, capabilities) → **partial** status → **non-blocking** (commands, LSP, MCP, formatters, VCS) → **complete** status
- Event-driven incremental updates via `event.subscribe()`: handles `session.updated`, `message.updated`, `message.part.updated`, `message.part.delta`, `todo.updated`, `permission.asked`, `question.asked`, `lsp.updated`, etc.
- Session hydration: `sync.session.sync(sessionID)` loads messages (last 100) and parts
- Session list: 30-day window, directory-filtered (opt-out via KV flag)
- Binary search for ordered insert/update into arrays

#### V2 Data Layer (`context/data.tsx` — 581 lines)
- Parallel V2 event system handling `V2Event` events: `session.created`, `session.updated`, `session.deleted`, `session.message.created`, `session.message.part.created`, `session.message.part.updated`, etc.
- Location-scoped data stores
- Gated behind `Flag.OPENCODE_EXPERIMENTAL_V2_DATA`

#### Session View (`routes/session/index.tsx` — 2,648 lines)
- The largest single file — renders the entire session UI including:
  - Message list with text/reasoning/tool parts
  - Permission prompts
  - Question prompts
  - Subagent footer
  - Sidebar
  - Scroll handling with acceleration curves
  - Thinking mode toggle
  - Tool output collapse
  - Diff wrap mode
  - Go upsell dialogs
  - Export functionality
- Uses `ScrollBoxRenderable` for the message list
- Sidebar visibility: auto (width > 120) or manual toggle

#### Diff Viewer (`feature-plugins/system/diff-viewer.tsx` — 1,059 lines)
- Git diff and "last turn" snapshot diff modes
- Split (side-by-side) or unified view
- File tree sidebar with expand/collapse
- Single-patch mode
- Hunk navigation (next/previous hunk, next/previous file)
- Uses `DiffRenderable` from @opentui/core

#### Prompt System (`component/prompt/index.tsx`)
- Multi-line terminal input with autocomplete
- File path suggestions via fuzzy matching (fuzzysort)
- Workspace-aware path resolution
- History (up/down arrows), frecency scoring
- Stash support
- Local file attachment

#### Keymap System (`keymap.tsx`)
- Mode-based keybind dispatch (`OPENCODE_BASE_MODE`)
- Plugin-extensible commands via `useBindings()`
- Command palette with fuzzy search
- Global/app/session scoped bindings

#### Plugin System (`plugin/`)
- TUI plugin runtime with route registration
- Slot system for extensibility (`app`, `app_bottom`)
- API surface exposing SDK, dialog, KV, theme, attention, etc.

### 3.4 State Management

- **SolidJS `createStore`** with Immer-style `produce()` for immutable updates
- **`reconcile()`** for efficient bulk updates (e.g., session lists, part arrays)
- **`createSignal()`** for local UI state (toggles, settings)
- **`createMemo()`** for derived state (e.g., `visible()`, `pending()`, `contentWidth()`)
- **KV store** (`context/kv.tsx`) for persistent user preferences (theme selection, sidebar state, terminal title toggle, etc.)
- **Event-driven sync**: server pushes events → `handleEvent()` → batched store updates → SolidJS reactivity propagates to UI

---

## 4. Performance-Sensitive Areas

### 4.1 Rendering Pipeline
- **Target: 60 FPS** (`createCliRenderer({ targetFps: 60 })`)
- The terminal renderer must diff the virtual DOM and output terminal escape sequences on every frame
- Large session message lists with many tool outputs can stress rendering
- **Mitigation**: SSE event batching (16ms flush window), message limit (100 visible), SolidJS `batch()` for grouped updates

### 4.2 SSE Event Processing
- Streaming parts (token-by-token via `message.part.delta` events) trigger frequent store updates
- Text parts are concatenated (`existing + delta`) in the store
- **Mitigation**: 16ms throttle on SSE event flush to batch rendering updates

### 4.3 Diff Viewer
- ~30 tree-sitter WASM parsers loaded for syntax highlighting
- Split view renders two panes of syntax-highlighted code
- Large diffs with many files/hunks require efficient scrolling
- **Mitigation**: `DiffRenderable` is an @opentui core primitive (presumably optimized in the renderer), lazy loading of parsers

### 4.4 Scroll Performance
- Custom scroll acceleration curves (`getScrollAcceleration()`)
- Hit-testing for message navigation uses `scroll.getChildren()` + `c.y` position checks
- `toBottom()` uses `setTimeout(50ms)` to defer scroll after render

### 4.5 Session Sync
- Loading 100 messages + all parts can be heavy for long sessions
- **Mitigation**: Messages 0..(n-100) are trimmed from the store; only visible messages and their parts are kept

### 4.6 Autocomplete
- Fuzzy matching against workspace file tree using fuzzysort
- File listing via `@ff-labs/fff-bun` (frecency-sorted) — patched

### 4.7 Startup Time
- Blocking phase: providers + agents + config + capabilities fetch
- Non-blocking phase: commands, LSP, MCP, formatters, VCS
- Fast boot mode: `OPENCODE_FAST_BOOT` env var skips initial loading UI
- Pre-warm: terminal palette is fetched before ThemeProvider mounts to avoid flash

---

## 5. Build System

### Building
The TUI package does **NOT have its own build step**. There is no `vite.config.ts`, no `bun run build` script, and no output `dist/` directory.

Instead, the TUI is **imported directly as TypeScript source** by other packages (primarily `packages/opencode` and `packages/cli`). The Bun runtime can execute TypeScript natively.

### Type Checking
```bash
cd packages/tui && bun typecheck
# which runs: tsgo --noEmit
```

Uses `@typescript/native-preview` (TypeScript 7.0.0-dev) via `tsconfig.json` which extends `@tsconfig/bun/tsconfig.json`.

### JSX Configuration
```json
{
  "jsx": "preserve",
  "jsxImportSource": "@opentui/solid"
}
```

JSX is NOT compiled to `h()` calls — it's preserved and the `@opentui/solid/preload` bunfig preload handles JSX transformation at runtime.

### Build Orchestration
- **Turbo** (v2.8.13) manages task orchestration
- **Bun** (v1.3.14) as package manager and runtime
- `bunfig.toml` preloads `@opentui/solid/preload` for both dev and test

### Tests
- `bun test --timeout 30000 --only-failures`
- Test fixtures in `test/fixture/` provide mock TUI SDK/runtime/plugin/environment

---

## 6. Dependencies

### Direct Dependencies (non-workspace)

| Package | Version | Purpose |
|---|---|---|
| `@opentui/core` | 0.3.4 | Terminal rendering engine |
| `@opentui/solid` | 0.3.4 | SolidJS → terminal bridge |
| `@opentui/keymap` | 0.3.4 | Keybind system |
| `solid-js` | 1.9.10 | Reactive UI framework |
| `effect` | 4.0.0-beta.83 | Functional effect system |
| `diff` | 8.0.2 | Text diffing for diff viewer |
| `fuzzysort` | 3.1.0 | Fuzzy search for autocomplete/command palette |
| `remeda` | 2.26.0 | Data manipulation utilities |
| `clipboardy` | 4.0.0 | System clipboard |
| `strip-ansi` | 7.1.2 | ANSI escape stripping |
| `open` | 10.1.2 | URL opener |
| `opentui-spinner` | 0.0.7 | Terminal spinner |

### Workspace Dependencies

| Package | Import path |
|---|---|
| `@opencode-ai/core` | `workspace:*` |
| `@opencode-ai/plugin` | `workspace:*` |
| `@opencode-ai/sdk` | `workspace:*` |
| `@opencode-ai/ui` | `workspace:*` |

### Dev Dependencies

| Package | Version |
|---|---|
| `@tsconfig/bun` | catalog (1.0.9) |
| `@types/bun` | catalog (1.3.13) |
| `@typescript/native-preview` | catalog (7.0.0-dev) |

---

## 7. Rust Code / Rust Rewrite

**No Rust code exists anywhere in this repository.**

Search results across all `*.md`, `*.toml`, `*.json`, `*.ts`, and `*.tsx` files for "Rust", "Tauri", "rewrite", "native UI" found:
- Zero Rust source files (`.rs`)
- Zero references to a Rust rewrite or Tauri
- The only "rust" references are tree-sitter WASM parsers for the Rust language (syntax highlighting in the diff viewer)

The TUI is entirely TypeScript-based. The `@opentui` packages (the rendering engine) are external npm packages of unknown implementation language, but likely also TypeScript given the npm packaging and SolidJS integration patterns.

---

## Summary

- **147 source files, ~27k lines** of TypeScript/TSX
- Built on **SolidJS** with a proprietary TUI rendering stack (**@opentui** v0.3.4), NOT Ink/React
- **REST + SSE** communication with backend (no gRPC/WebSocket/stdio)
- **Event-driven state sync** with 16ms batching for performance
- **Single large Session component** (2,648 lines) handling message rendering, permissions, questions, diff viewing
- **No build step** — imported as source TypeScript, executed directly by Bun
- **No Rust** — fully TypeScript; no Rust rewrite discussion found
