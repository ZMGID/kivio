# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Kivio (formerly KeyLingo through v2.4.4; currently v2.7.x) is a desktop **AI assistant** built with **Tauri v2** (Rust backend) and **React 18 + Vite + TailwindCSS v4** (frontend). It runs on macOS and Windows. It began as a screen-level utility — global hotkey-triggered text translation, screenshot OCR/translation, and a Lens capture-then-ask vision overlay — and has grown a full **agentic chat application** (`src/chat/` + `src-tauri/src/chat/`) with a tool-calling agent loop, MCP servers, Skills, sub-agents, a Pyodide code sandbox, and a provider-agnostic model layer (OpenAI-compatible **and** Anthropic Messages). All AI calls go through user-configured providers.

## Common Commands

Use `npm` (lockfile is `package-lock.json`). Rust tooling is managed by Tauri.

- `npm install` — install Node dependencies.
- `npm run dev` — run the full Tauri app (Rust backend + Vite UI). Automatically builds Swift sidecars on macOS. This is the standard dev command.
- `npm run dev:ui` — run the Vite UI dev server only (useful for quick UI iteration without compiling Rust).
- `npm run build` — build the full desktop app bundle via Tauri.
- `npm run build:swift` — build the Swift sidecar binary (`kivio-ocr-helper` for Apple Vision OCR). macOS only; other platforms generate an empty stub to satisfy Tauri's `externalBin` validation.
- `npm run build:ui` — runs `prepare:pyodide` then builds the production UI bundle only (outputs to `dist/`). `npm run prepare:pyodide` (`scripts/prepare-pyodide-assets.mjs`) stages the bundled Pyodide runtime for the code sandbox.
- `npm run preview` — preview the built UI bundle locally.
- `npm run lint` — run ESLint on `.ts` and `.tsx` files (`--max-warnings 0`, so warnings fail).
- `npm run typecheck` — run `tsc --noEmit` for strict TypeScript checks.
- `npm test` — run the **Vitest** frontend test suite once (`npm run test:watch` for watch mode). Run a single file with `npx vitest run src/chat/segments.test.ts`; filter by name with `-t "<pattern>"`.
- `cargo test --manifest-path src-tauri/Cargo.toml` — run Rust unit tests (the agent loop has substantial coverage in `chat/agent/loop_tests.rs`).

There is no e2e runner; manual smoke testing is still required after changes that affect app flows (capture, hotkeys, streaming).

## Architecture

### Frontend-Backend Communication

All Tauri `invoke` calls and event listeners are centralized in **`src/api/tauri.ts`**. This is the single source of truth for the frontend-backend contract. When adding new Rust commands, expose them here first.

Key patterns:
- `api.translateText(text)` — debounced 600ms in `App.tsx`.
- `api.commitTranslation(text)` — copies to clipboard, then **closes** (destroys) the `main` window to avoid the translator WebView lingering in memory, optionally sends paste shortcut to the previous app.
- `api.closeWindow()` — calls `win.close()`, which **destroys** the window to reclaim memory (idle process drops to ~50MB). The `CloseRequested` handler in `lib.rs` only `prevent_close()`s the `lens`/`translate` overlays — and even those immediately run `lens_close` → `destroy()` for full cleanup; `main`/`chat`/`settings` take the default close (destroy). Windows are re-created on demand (`ensure_*_window`), not hidden-and-reused.

### Window Modes and Routing

The app uses **four webview windows**, all serving the same `index.html` / `App.tsx` bundle. `App.tsx` picks which view to render from `window.location.hash` (+ `?mode=` query); the Rust side decides which window to show. Only `main` is declared statically in `tauri.conf.json`; the others are created on demand by helpers in **`src-tauri/src/windows.rs`**:
- **`main`** — translator (default, `392×152`), routed by hash `''`.
- **`settings`** — Settings panel (`#settings` → `Settings.tsx`). `ensure_*`/`get_settings_window`.
- **`chat`** — the agentic chat app (`#chat` → lazy-loaded `chat/Chat.tsx`, wrapped by `ChatWindowHost`). Created via `ensure_chat_window` / `ensure_chat_window_with_hash`; geometry + last route are persisted and restored. `#chat/settings` is the in-chat settings subroute. Routing predicates (`isChatPath`, `isChatSettingsPath`, `hashPath`, route-remembering) live in `src/chat/`.
- **`lens`** — fullscreen transparent overlay for capture + chat (`ensure_lens_window`). Subroute via hash query: `#lens` (chat mode) vs `#lens?mode=translate` (screenshot translate); both share `Lens.tsx`, which reads the query in `readModeFromHash`.

The capabilities allowlist (`src-tauri/capabilities/default.json`) must list every webview label a plugin permission applies to — currently `["main", "chat", "settings", "lens"]`. When you add a window, add its label here or plugin calls silently fail.

### Frontend Submodules

The settings panel (`src/Settings.tsx`) delegates to helpers in **`src/settings/`**:
- `components.tsx` — reusable UI primitives (Toggle, Select, HotkeyRecorder, etc.).
- `i18n.ts` — bilingual string table (zh/en).
- `utils.ts` — hotkey parsing/formatting and platform detection.
- plus `ProviderModelsPicker`, `ProviderSortableList`, `ModelPairSelect`, `ScreenshotTranslationSettings`, `UsageStatsPanel`, `providerPresets`, `SettingsShell`.

The chat UI lives in **`src/chat/`** (mounted via lazy `Chat.tsx` inside `ChatWindowHost`). It mirrors the Rust agent concepts: message rendering (`MessageList`/`MessageBubble`/`ChatMarkdown`), tool-call and reasoning blocks (`ToolCallBlock`/`ReasoningBlock`/`AskUserBlock`), conversation/project sidebar, model/skill selectors, the Pyodide runner, and error boundaries (`ChatErrorBoundary`/`ToolCallErrorBoundary`/`MarkdownErrorBoundary`). Many of these modules have colocated Vitest `.test.ts(x)` files — keep them green. Lens-specific frontend helpers are in `src/lens/`.

### Multi-Provider System

The app supports multiple AI providers. Each feature can use a different provider/model:
- **Translator** (`translatorProviderId` + `translatorModel`)
- **Screenshot Translation/OCR** (`screenshotTranslation.providerId` + `model`)
- **Lens** (`lens.providerId` + `lens.model`; both blank ⇒ falls back to translator provider/model)
- **Chat** — selected per-conversation in the chat UI (`ModelSelector`); see the Chat/Agent section.

Providers are mostly OpenAI-compatible, but the chat runtime is provider-agnostic and also speaks the **Anthropic Messages** API natively (`chat/model/anthropic.rs`). Providers have `availableModels` (fetched from `/models`) and `enabledModels` (user-selected subset shown in dropdowns). Model selection UI uses colon-delimited values like `providerId:modelName`.

Each provider stores `apiKeys: string[]` (a pool of keys for failover), not a single key. The first entry is the primary; subsequent entries are backups.

### Multi-Key Failover

When a request fails with a quota/rate-limit/auth error, the backend automatically rotates to the next configured key for that provider. Implementation lives across `src-tauri/src/api.rs` and `src-tauri/src/state.rs`:

- `AppState.key_cooldowns` — `(provider_id, key_idx) → Instant` map; failed keys are cooled down for `KEY_COOLDOWN` (60s) before being eligible again.
- `AppState.active_key_idx` — last-known-good idx per provider; subsequent calls start from this idx.
- `send_with_failover(state, label, attempts, provider_id, api_keys, send)` — wraps `send_with_retry`. The `send` closure takes a `&str` (the current key) so the same body builder is reused across keys.
- `is_failover_error(err_msg)` — pattern-matches on HTTP status parsed from the error string. Only 401/402/403/429 trigger key rotation; malformed requests and server/network failures do not burn backup keys.
- Non-failover errors (timeouts, 5xx) still go through `send_with_retry` exponential backoff and don't burn keys.
- `test_provider_connection` deliberately uses only the first key (so users see whether their primary configuration is correct without hidden fallback masking issues).

### Chat / Agent Runtime

The chat app (`src-tauri/src/chat/`) is the largest subsystem. It runs a **provider-agnostic agentic tool loop**, with `src/chat/` (esp. `Chat.tsx`) as the frontend.

**Model abstraction (`chat/model/`).** Read `chat/model/README.md` — it's the binding contract. Runtime code never inspects provider JSON: it builds a `GenerateRequest`, hands it to a `LanguageModelProvider`, and consumes `GenerateOutput` + `StreamPart` events. `openai.rs` (OpenAI-compatible) and `anthropic.rs` (Anthropic Messages) are **peer adapters** that own all wire-format details. Do not leak `choices`, Anthropic `content` blocks, or SSE event names into loop/tool code.

**Agent loop (`chat/agent/`).** Orchestration is split into phases threaded by `loop_.rs`: `prepare` → `planning` → `rounds` (tool execution) → `synthesis` → `finalize`, with `compaction.rs` for context window compaction, `stream.rs` for streaming, `stop.rs` for cancellation/system-message patching, and `filter.rs` for per-agent tool allow-listing. The loop is decoupled from Tauri via the **`AgentHost` trait (`host.rs`)** — it emits stream deltas, tool records, and approval requests through that trait, and executes tools through a `ToolExecutor` (`execute.rs`). `loop_tests.rs` exercises the phases with fake hosts; prefer extending it over manual testing for loop changes.

**Tools.** The agent's tool set is assembled per-round from several sources:
- **Native tools (`src-tauri/src/native_tools/`)** — built-in: `web_fetch`, file ops (`read_file`/`write_file`/`edit_file`/`glob_files`/`search_files`/`list_dir`/`stat_path`/`move`/`copy`/`delete`/`create_dir`), `run_command` (shell), and sandbox artifact export. `run_command` with `background:true` (auto-enabled for dev servers like `npm run dev`/`vite`) spawns a tracked background process: stdout+stderr are captured to a per-job temp log (`kivio-bgcmd-<job_id>.log`), the pid/log/status are registered in `AppState.background_commands`, and the model polls with `bash_output` (incremental, by offset) / `list_background` / `kill_background` (cross-platform process-group kill via `kill_process_group` — unix `killpg` SIGTERM→SIGKILL, Windows `taskkill /T /F`). Background commands **survive across turns** (NOT cancelled on run end, unlike sub-agents, which are blocking) and are cleaned up only by `kill_background` or the app-exit sweep in `lib.rs` (`RunEvent::ExitRequested` → `kill_all_background_commands`); startup `cleanup_orphan_temp_files` GCs stale `kivio-bgcmd-*.log`. **Security**: writes/edits are blocked under sensitive home segments (`.ssh`, `.gnupg`, Keychains, …); `MAX_READ_FILE_BYTES` caps reads. Touch these guards carefully.
- **MCP (`src-tauri/src/mcp/`)** — Model Context Protocol client/manager for external tool servers. `native_registry.rs` registers the built-in native tools alongside MCP-provided ones; `ChatToolDefinition` is the unified tool shape consumed by the loop.
- **Skills (`src-tauri/src/skills/`)** — markdown-defined skills (frontmatter + body, like Claude Code skills) discovered from a user dir + built-ins (`discover.rs`), activated mid-run (`runtime.rs`), and optionally backed by runnable scripts. Skill activation re-permits tools, which is why `base_tools` is recomputed each round (see comments in `loop_.rs`).
- **Sub-agents (`src-tauri/src/agents/` + `chat/sub_agent.rs`)** — named personas (built-in → user `<app_data>/agents/*.md` → project `.kivio/agents/*.md`, later layers override by id) with a system-prompt prefix, optional model override, and a tool allow-list **enforced** at spawn via `filter::filter_tools_for_agent`, which also strips the `agent` tool so sub-agents can't recurse. Concurrency is capped by a `SubAgentManager` semaphore (default `DEFAULT_SUB_AGENT_CONCURRENCY` = 12, live-configurable via `settings.chat_tools.sub_agent_concurrency`). The `agent` tool is **blocking + single-result** (the Claude Code Task model): each call awaits `run_sub_agent` to completion and returns the full result inline to the parent. **Parallelism comes from the model emitting MULTIPLE `agent` calls in one message** — `agent` is `parallel_safe`, so a single round runs them concurrently via `execute_parallel_chunk` (join_all, capped by `MAX_PARALLEL_TOOL_CALLS_PER_ROUND` = 12 and the semaphore). The wait stays in the runtime, never in the model token loop — there is no `background`/`await`/`poll` machinery (an earlier dispatch-and-return + `await_agents`/`check_agent_result` design was removed after testing showed it degenerated into polling and hid the running sub-agent from the user). Cancellation cascades from the parent generation (`generation_cascade_active`): user stop / run end ends the sub-agent on its next loop check. The card shows live nested progress via `chat-subagent` events (~350ms); the sub-agent emits NO terminal event (the inline result drives the card to done via `chat-tool`). The task registry is in-memory only (results are lost on restart).

**Other chat modules**: `storage.rs` (conversation persistence), `memory.rs`, `todo.rs` + `plan.rs` (agent task/plan tracking), `ask_user.rs` (mid-run user prompts), `attachments.rs` + `image_generation.rs`, `model_metadata.rs`, `dsml_tools.rs`. `commands.rs` exposes the chat Tauri commands.

**Knowledge base / RAG (`chat/knowledge_base/`)**: multi-library document RAG. Each library binds one `(embedding_provider, model, dim)`; **changing the model rebuilds the index**. Storage is **per-library SQLite** at `{app_data}/knowledge_base/<kb_id>/store.db` via `store.rs` (`rusqlite` + **sqlite-vec `vec0`** for vectors + **FTS5** for keyword) — `libraries.json` stays JSON for the (small, search-free) library list. `vec0` has a fixed dim, so the vector table is per-library, created lazily once the dim is known; chunk text/metadata live in a normal `chunks` table joined to `vec_chunks` by rowid (vtables can't carry extra columns). Legacy V1 `docs.json`/`chunks.json` are migrated into store.db on first open (then renamed `*.migrated`). **Red lines**: use `rusqlite` directly (NOT `tauri-plugin-sql` — sqlx can't load the extension); the extension is registered via `sqlite3_auto_extension` (once, in `store::open_db`) before connections open. **Retrieval is hybrid**: `store::hybrid_search` fuses vector (cosine, `embedding MATCH ? AND k=?`) + FTS5 (BM25) lanes via **Reciprocal Rank Fusion (k=60)**; lane weights come from the `knowledge_base` settings (`hybrid_enabled` toggle + `weight_vector`/`weight_keyword`), pure-vector when keyword weight is 0. An **optional global rerank** (`rerank.rs`, Cohere/Jina-compatible `/rerank`, reuses `send_with_failover`) reorders the over-fetched hits when `rerank_provider_id`/`rerank_model` are set — blank or any failure degrades to the fused order. `chunking.rs` is heading-aware with a CJK-correct token estimate (counts CJK ≈1 token/char — the English 4-chars/token rule undercounts Chinese). `embeddings.rs` is a separate OpenAI-compatible `/embeddings` adapter (**Anthropic has no embeddings endpoint** — do not route it through the chat Anthropic adapter); it reuses `api::send_with_failover`. `ingest.rs` runs the pipeline (parse → chunk → embed → store) in a background `async_runtime::spawn` serialized by a **per-kb async lock** (`kb_lock_for`, avoids lost-update on concurrent uploads), emitting `kb-index` progress events; startup `heal_stale_indexing` flips interrupted `indexing` docs → `error`. `kb_import_url` ingests a web page (fetch → readable text via the shared `web_fetch` extractor → `.md` snapshot so re-index never re-fetches → content-hash dedup). **Document parsing** is `parse.rs` (built-in, offline: txt/md, html via the `web_fetch` scraper, pdf text-layer via `pdf-extract`, docx via zip + WordprocessingML `<w:t>` scan, xlsx via `calamine`) routed by `process.rs::process_document` per the `document_processing` settings: **image files** (png/jpg/webp/…) are OCR'd via Kivio's existing engines (`ocr_engine`: system Apple Vision/Windows OCR, or RapidOCR offline; `off` rejects); PDF `force_ocr` honestly errors (scanned-PDF OCR would need pdfium, deliberately not pulled). **Third-party processors (MinerU/Doc2X/Custom) are suspended** — the adapters live in git history at `518f0e2`. Retrieval entry is the **`knowledge_search` native tool** (registered in `mcp/native_registry.rs`, def in `mcp/types.rs`): it resolves the conversation's mounted `knowledge_base_ids` (empty = no search, never "all"), embeds the query per `(provider,model)` group, runs hybrid + optional rerank, and returns passages tagged `[n]` + structured hits for source cards; when a conversation has libraries attached, `mount_system_prompt` injects guidance to prefer `knowledge_search` and cite `[n]`. Frontend: `src/chat/knowledgeBase.ts` (API + `kb-index` listener), `src/settings/KnowledgeBasePanel.tsx` (Settings "知识库" two-pane page: left nav = 文档处理 / 检索 / library list + 新建; library detail = file import + URL import + reindex), `src/settings/DocumentProcessingPanel.tsx` (Kivio built-in only: OCR engine + PDF strategy + supported-formats list), `src/settings/RetrievalPanel.tsx` (hybrid toggle/weights + rerank picker), `src/chat/KnowledgeBaseChip.tsx` (conversation-level mount selector), `src/chat/citations.ts` + `ChatMarkdown` (answer `[n]` → clickable source popover, map built in `MessageBubble`), and `ToolCallBlock.tsx` source-card rendering. Live retrieval-stack E2E is `chat/knowledge_base/live_e2e_tests.rs` (gated by `KB_E2E=1`, key via env). **V2 complete** (storage / hybrid+rerank / built-in doc processing + image OCR / `[n]` jump / URL import + dedup); history + any deferred ideas in `.trellis/tasks/06-25-knowledge-base-rag/prd.md`.

**Code sandbox**: chat can run Python via **Pyodide** in the webview (frontend `src/chat/pyodideRunner.ts`); the runtime assets are bundled at build time (see `prepare:pyodide`) and document Skills (pdf/docx/xlsx) depend on it — see Release.

**Streaming & events**: the chat UI contracts are the Tauri events `chat-stream`, `chat-tool`, and `chat-context` (and there is a separate Lens stream — see Streaming). These payload shapes are UI contracts, not provider contracts — keep them stable.

`usage.rs` records per-call token usage (logged under a `usage/` dir) and feeds the Settings usage panel (`src/settings/UsageStatsPanel.tsx`).

### Settings Persistence and Security

- Settings are stored via `tauri-plugin-store` in `settings.json`, **including API keys** (in the `providers[].apiKeys` array).
- Older versions (≤ v2.3.x) stored keys in the OS keyring. On first launch under v2.4+, `migrate_legacy_keyring_keys` reads any leftover keyring entries into `settings.api_keys[0]` and deletes the keyring entry. From then on, the keyring is never written.
- The `keyring` crate dependency is retained only for that one-shot migration path and can be removed once all users have upgraded.
- **`sanitize_settings`** in `src-tauri/src/settings.rs` handles migration from legacy single-provider configs to the multi-provider system, validates provider existence, and normalizes hotkeys. It also migrates the legacy single `apiKey` field on each `ModelProvider` (read via the `api_key_legacy` field with `#[serde(rename = "apiKey")]`) into `api_keys[0]`. `normalize_hotkey` canonicalizes modifier aliases to `CommandOrControl`, `Control`, `Alt`, `Shift`, `Super` — use these exact strings when constructing hotkeys.
- Saving settings is transactional: if hotkey registration fails, `restore_runtime_settings` rolls back to the previous state.

### Screenshot Capture and OCR

**Capture** is platform-guarded with `cfg(target_os = ...)`:

- **macOS** — `src-tauri/src/sck.rs` uses ScreenCaptureKit (`screencapturekit` crate, `macos_14_0` feature). No `screencapture` shell-out.
- **Windows** — `xcap` crate captures full-screen / window content (the dependency is `cfg`-gated to Windows in `Cargo.toml`).

Both platforms route through the **Lens overlay** (`Lens.tsx`): the overlay presents hover-highlighted app windows or a draggable region; user click / drag commits via `lens_capture_window` / `lens_capture_region` Tauri commands. The capture commands receive logical-pixel coordinates from the overlay and call the platform-specific module to produce a PNG in `temp_dir`.

A single busy flag (`AppState.lens_busy`, `AtomicBool`) prevents concurrent overlays. `lens_request_internal` swaps it true on entry; `lens_close` resets it. A reactive self-heal in `lens_request_internal` clears a stale flag if the previous run leaked it (e.g. on panic).

**OCR** for screenshot translation has three implementations:

- **macOS system OCR** (`macos_ocr.rs`) — spawns `kivio-ocr-helper` Swift sidecar that calls Apple Vision. The helper is a persistent subprocess; requests/responses are JSON over stdin/stdout. Built via `npm run build:swift`.
- **Windows system OCR** (`windows_ocr.rs`) — calls `Windows.Media.Ocr` APIs directly via Windows Runtime bindings.
- **RapidOCR offline** (`rapidocr.rs`) — cross-platform PaddleOCR ONNX pipeline for users who want fully offline OCR without system dependencies. Downloads ONNX Runtime + models on first use. User-initiated install only; no automatic fallback.

### Rust Backend Structure

- **`main.rs`** — Tauri commands, update flow, hotkey registration, tray setup, window lifecycle, capture orchestration, and app startup.
- **`api.rs`** — HTTP client setup, provider credential resolution, retry/failover, OpenAI-compatible text/OCR/vision calls, and SSE stream parsing.
- **`state.rs`** — `AppState`, lock helpers, Lens runtime state, and multi-key cooldown / active-key selection.
- **`settings.rs`** — Settings schema, serde defaults, `sanitize_settings` migration/validation, one-shot `migrate_legacy_keyring_keys` (gated by `legacy_keyring_migrated` flag), `persist_settings` (mirrors `apiKeys[0]` to legacy `apiKey` field for downgrade compat).
- **`screenshot.rs`** — Temp PNG cleanup helpers (`cleanup_temp_file` for one-shot, `cleanup_orphan_temp_files` for app-startup GC of stale `lens-*.png` / `screenshot-*.png` older than 24 h).
- **`sck.rs`** — macOS-only ScreenCaptureKit wrapper invoked by `lens_capture_window` / `lens_capture_region`.
- **`lens.rs`** — Lens overlay state machine support: `lens_list_windows` (macOS only; Windows returns `[]`), capture coord helpers.
- **`windows.rs`** — Window helpers for all four windows: `ensure_main_window`, `ensure_chat_window`(`_with_hash`), `ensure_lens_window`, `get_main_window`/`get_settings_window`/`get_chat_window`, chat-window chrome/min-size/geometry helpers, plus `apply_macos_workspace_behavior` for `visibleOnAllWorkspaces`.
- **`utils.rs`** — Language detection, target language resolution, timestamp helper.
- **`commands.rs`** — General Tauri command implementations (settings, window management, clipboard, testing).
- **`lens_commands.rs`** — Lens-specific Tauri commands (capture, explain, streaming, history).
- **`shortcuts.rs`** — Global hotkey registration and management.
- **`updates.rs`** — Auto-update check and GitHub release polling.
- **`prompts.rs`** — Default prompt templates for translator, screenshot translation, and Lens features.
- **`web_search.rs`** — Lens web search integration (Tavily / Exa providers). Called when Lens decides to search for current facts, unfamiliar visible text, or external context.
- **`usage.rs`** — Per-call token-usage logging (`usage/` dir) and aggregation for the Settings usage panel.
- **`chat/`** — the agentic chat subsystem (see Chat / Agent Runtime): `agent/` (loop phases), `model/` (provider adapters), plus `storage`, `memory`, `todo`, `plan`, `ask_user`, `attachments`, `image_generation`, `sub_agent`, `commands`, etc.
- **`mcp/`** — MCP client/manager and the unified `ChatToolDefinition` tool registry (`native_registry` + external servers).
- **`native_tools/`** — built-in agent tools (web fetch, file ops, shell, sandbox export) with path/size security guards.
- **`skills/`** — Skill discovery/parse/activation/run (markdown-defined skills).
- **`agents/`** — sub-agent persona definitions (built-in + user + project layers).
- **`macos_ocr.rs`** — macOS Apple Vision OCR via Swift sidecar (`kivio-ocr-helper`). Persistent subprocess with JSON stdin/stdout protocol.
- **`windows_ocr.rs`** — Windows system OCR via `Windows.Media.Ocr` APIs.
- **`rapidocr.rs`** — Cross-platform offline OCR using PaddleOCR ONNX models. Downloads ONNX Runtime + models on user-initiated install.
- **`capture_geometry.rs`** — Coordinate transformation helpers for multi-monitor screenshot capture.

Key crate responsibilities from `Cargo.toml`:
- `enigo` — simulates keyboard paste after translation commit.
- `arboard` — clipboard read/write.
- `keyring` — legacy API key storage (read-only; v2.4+ stores keys in `settings.json`, `keyring` is retained only for one-shot migration of pre-v2.4 installs).
- `reqwest` — HTTP client for OpenAI-compatible APIs.
- `screencapturekit` — macOS ScreenCaptureKit binding (used by `sck.rs`).
- `xcap` — Windows screen / window capture.
- `oar-ocr` + `ort` — RapidOCR ONNX Runtime bindings for offline OCR.
- `windows` crate — Windows Runtime bindings for system OCR APIs.

### Streaming

Lens supports streaming responses via two SSE-relay event channels emitted by stream helpers in `api.rs`:
- `lens-stream` — chat answers; deltas accumulate into the last assistant message in `Lens.tsx`. Supports `delta.reasoning_content` for reasoning-mode models.
- `lens-translate-stream` — screenshot translate; emits `kind="translated"` deltas, then a `<<<ORIGINAL>>>` separator, then `kind="original"` deltas. Frontend splits the stream into translation (top) + original (small grey reference, bottom).

Cancellation is via `AppState.explain_stream_generation` (`AtomicU64`) — each new stream snapshots its generation; the inner chunk loop bails when the global moves past it.

## Release

Releases are built via GitHub Actions (`.github/workflows/release.yml`). Pushing a `v*` tag triggers builds for:
- **macOS** — DMG bundle (`--bundles dmg`)
- **Windows** — MSI + NSIS bundles (`--bundles msi,nsis`)

Manual releases are also supported via `workflow_dispatch`.

Bundled document Skills require their execution runtime in the installer. If `pdf`, `docx`, and `xlsx` Skills are packaged, the release must also package the Python/Pyodide sandbox runtime, `python_stdlib.zip`, and local wheels for common packages such as `numpy`, `pandas`, `matplotlib`, `scipy`, `sympy`, `scikit-learn`, `statsmodels`, `pillow`, `seaborn`, and `micropip`. `run_python` should prefer bundled local Pyodide resources; CDN package loading is only a fallback. Before publishing, inspect the final DMG / MSI / NSIS artifacts and verify that both `skills/pdf|docx|xlsx` and the Python/Pyodide runtime package files are inside the installed app resources. Follow `docs/RELEASE_PACKAGING.md` for the exact flow; do not publish releases from memory.

## Code Style

- TypeScript + React, ESM (`"type": "module"`).
- 2-space indentation, single quotes, no semicolons.
- Components use `PascalCase.tsx`; utilities/services use `camelCase.ts`.
- Tailwind utility classes for UI; shared styles in `src/index.css`, component-specific in `src/App.css`.
- Dark mode uses a `.dark` class on `document.documentElement` (configured via `@custom-variant dark` in Tailwind v4).
- Git commits follow Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`).

## Important Implementation Details

- **Swift sidecars**: macOS uses a Swift helper binary built via `scripts/build-swift-sidecar.js`:
  - `kivio-ocr-helper` — Apple Vision OCR (required for macOS system OCR).
  - Non-macOS platforms generate empty stubs to satisfy Tauri's `externalBin` validation.
- **macOS**: The app hides its Dock icon (`ActivationPolicy::Accessory`) and uses `visibleOnAllWorkspaces` for all windows.
- **Windows**: Manual launch opens settings by default. Autostart uses a dedicated `--from-autostart` arg to avoid popping up settings. Single-instance guard ensures clicking the app icon focuses the existing instance.
- **LaTeX math**: Both screenshot result and explain use `react-markdown` + `remark-math` + `rehype-katex` for rendering LaTeX formulas.
- **Prompt templates**: Default prompts and prompt composition live in Rust (`prompts.rs` plus defaults exposed through `get_default_prompt_templates`). Custom prompts support `{lang}` and `{text}` placeholders.
