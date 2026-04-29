# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

KeyLingo is a lightweight desktop translation and AI vision utility built with **Tauri v2** (Rust backend) and **React 18 + Vite + TailwindCSS v4** (frontend). It runs on macOS and Windows and provides global hotkey-triggered text translation, screenshot OCR/translation, and screenshot explanation via OpenAI-compatible APIs.

## Common Commands

Use `npm` (lockfile is `package-lock.json`). Rust tooling is managed by Tauri.

- `npm install` — install Node dependencies.
- `npm run dev` — run the full Tauri app (Rust backend + Vite UI). This is the standard dev command.
- `npm run dev:ui` — run the Vite UI dev server only (useful for quick UI iteration without compiling Rust).
- `npm run build` — build the full desktop app bundle via Tauri.
- `npm run build:ui` — build the production UI bundle only (outputs to `dist/`).
- `npm run preview` — preview the built UI bundle locally.
- `npm run lint` — run ESLint on `.ts` and `.tsx` files.

There is no test runner configured. Manual smoke testing is required after changes.

## Architecture

### Frontend-Backend Communication

All Tauri `invoke` calls and event listeners are centralized in **`src/api/tauri.ts`**. This is the single source of truth for the frontend-backend contract. When adding new Rust commands, expose them here first.

Key patterns:
- `api.translateText(text)` — debounced 600ms in `App.tsx`.
- `api.commitTranslation(text)` — copies to clipboard, hides window, optionally sends paste shortcut to the previous app.
- Window "close" methods (`closeWindow`, `closeScreenshotWindow`, `closeExplainWindow`) actually call `win.hide()` — windows are reused, never destroyed.

### Window Modes and Routing

The app uses **one main webview window** that switches views via `window.location.hash`:
- `''` or `'translator'` — main translation input (360x120)
- `'settings'` — settings panel (420x520)
- `'screenshot'` — screenshot translation result
- `'explain'` — screenshot explanation chat
- `'capture'` — region selection overlay (fullscreen transparent)

`App.tsx` reads the hash to determine the mode and resizes the window accordingly. Additional dedicated Tauri windows are created dynamically for `screenshot`, `explain`, and `capture` modes via `src-tauri/src/windows.rs`. Window behavior and bundle targets are configured in **`src-tauri/tauri.conf.json`**.

The explain window has two size states: compact (480×280, thumbnail + summary) and expanded (520×680, full chat). It receives `imageId` as a `?imageId=` URL query param; the frontend uses this to fetch the base64 image and conversation history from the Rust backend.

### Settings UI Submodules

The settings panel (`src/Settings.tsx`) delegates to helpers in **`src/settings/`**:
- `components.tsx` — reusable UI primitives (Toggle, Select, HotkeyRecorder, etc.).
- `i18n.ts` — bilingual string table (zh/en).
- `utils.ts` — hotkey parsing/formatting and platform detection.

### Multi-Provider System

The app supports multiple OpenAI-compatible providers. Each feature can use a different provider/model:
- **Translator** (`translatorProviderId` + `translatorModel`)
- **Screenshot Translation/OCR** (`screenshotTranslation.providerId` + `model`)
- **Lens** (`lens.providerId` + `lens.model`; both blank ⇒ falls back to translator provider/model)

Providers have `availableModels` (fetched from `/models` endpoint) and `enabledModels` (user-selected subset used in dropdowns). Model selection UI uses colon-delimited values like `providerId:modelName`.

Each provider stores `apiKeys: string[]` (a pool of keys for failover), not a single key. The first entry is the primary; subsequent entries are backups.

### Multi-Key Failover

When a request fails with a quota/rate-limit/auth error, the backend automatically rotates to the next configured key for that provider. Implementation lives in `main.rs`:

- `AppState.key_cooldowns` — `(provider_id, key_idx) → Instant` map; failed keys are cooled down for `KEY_COOLDOWN` (60s) before being eligible again.
- `AppState.active_key_idx` — last-known-good idx per provider; subsequent calls start from this idx.
- `send_with_failover(state, label, attempts, provider_id, api_keys, send)` — wraps `send_with_retry`. The `send` closure takes a `&str` (the current key) so the same body builder is reused across keys.
- `is_failover_error(err_msg)` — pattern-matches on the error string (`Error: 401/402/403/429`) and body keywords (`insufficient_quota`, `quota_exceeded`, `rate_limit_exceeded`, `billing`, `out of credit`, `insufficient balance`).
- Non-failover errors (timeouts, 5xx) still go through `send_with_retry` exponential backoff and don't burn keys.
- `test_provider_connection` deliberately uses only the first key (so users see whether their primary configuration is correct without hidden fallback masking issues).

### Settings Persistence and Security

- Settings are stored via `tauri-plugin-store` in `settings.json`, **including API keys** (in the `providers[].apiKeys` array).
- Older versions (≤ v2.3.x) stored keys in the OS keyring. On first launch under v2.4+, `migrate_legacy_keyring_keys` reads any leftover keyring entries into `settings.api_keys[0]` and deletes the keyring entry. From then on, the keyring is never written.
- The `keyring` crate dependency is retained only for that one-shot migration path and can be removed once all users have upgraded.
- **`sanitize_settings`** in `src-tauri/src/settings.rs` handles migration from legacy single-provider configs to the multi-provider system, validates provider existence, and normalizes hotkeys. It also migrates the legacy single `apiKey` field on each `ModelProvider` (read via the `api_key_legacy` field with `#[serde(rename = "apiKey")]`) into `api_keys[0]`. `normalize_hotkey` canonicalizes modifier aliases to `CommandOrControl`, `Control`, `Alt`, `Shift`, `Super` — use these exact strings when constructing hotkeys.
- Saving settings is transactional: if hotkey registration fails, `restore_runtime_settings` rolls back to the previous state.

### Platform-Specific Screenshot Flows

Screenshot capture is platform-guarded with `cfg(target_os = ...)`:

- **macOS**: Uses the native `screencapture -i` command for interactive region selection. The `BusyGuard` RAII pattern prevents concurrent captures.
- **Windows**: Uses a custom `CaptureOverlay.tsx` (fullscreen transparent webview) for region selection. The lifecycle is: `capture_request` opens the overlay → the user draws a region → `capture_commit` (with pixel coordinates) triggers `xcap` capture → `capture_cancel` dismisses without capturing. Windows also has a clipboard-based fallback in `screenshot.rs` using `ms-screenclip:`.

Busy flags (`screenshot_translation_busy`, `lens_busy`) prevent concurrent operations on both platforms.

### Rust Backend Structure

- **`main.rs`** — App state (`AppState`), Tauri commands, OpenAI API calling logic (`call_openai_text`, `call_openai_ocr`, `call_vision_api`), retry logic (`send_with_retry`), multi-key failover (`send_with_failover`, `is_failover_error`), hotkey registration, and window lifecycle.
- **`settings.rs`** — Settings schema, serde defaults, migration/sanitization, legacy keyring migration, persistence.
- **`screenshot.rs`** — Platform-specific screenshot capture (`capture_screenshot`) and temp file cleanup.
- **`windows.rs`** — Window creation helpers (`ensure_main_window`, `ensure_screenshot_window`, `ensure_capture_overlay_window`).
- **`utils.rs`** — Language detection, target language resolution, timestamp utility.

Key crate responsibilities from `Cargo.toml`:
- `enigo` — simulates keyboard paste after translation commit.
- `arboard` — clipboard read/write.
- `keyring` — legacy API key storage (read-only; v2.4+ stores keys in `settings.json`, `keyring` is retained only for one-shot migration of pre-v2.4 installs).
- `reqwest` — HTTP client for OpenAI-compatible APIs.
- `xcap` — Windows region screen capture.

### Streaming

Lens supports streaming responses. `stream_vision_response` in `main.rs` parses SSE chunks and emits `lens-stream` events to the frontend. The frontend (`Lens.tsx`) appends deltas to the last assistant message in the chat list.

## Release

Releases are built via GitHub Actions (`.github/workflows/release.yml`). Pushing a `v*` tag triggers builds for:
- **macOS** — DMG bundle (`--bundles dmg`)
- **Windows** — MSI + NSIS bundles (`--bundles msi,nsis`)

Manual releases are also supported via `workflow_dispatch`.

## Code Style

- TypeScript + React, ESM (`"type": "module"`).
- 2-space indentation, single quotes, no semicolons.
- Components use `PascalCase.tsx`; utilities/services use `camelCase.ts`.
- Tailwind utility classes for UI; shared styles in `src/index.css`, component-specific in `src/App.css`.
- Dark mode uses a `.dark` class on `document.documentElement` (configured via `@custom-variant dark` in Tailwind v4).
- Git commits follow Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`).

## Important Implementation Details

- **macOS**: The app hides its Dock icon (`ActivationPolicy::Accessory`) and uses `visibleOnAllWorkspaces` for all windows.
- **Windows**: Manual launch opens settings by default. Autostart uses a dedicated `--from-autostart` arg to avoid popping up settings. Single-instance guard ensures clicking the app icon focuses the existing instance.
- **LaTeX math**: Both screenshot result and explain use `react-markdown` + `remark-math` + `rehype-katex` for rendering LaTeX formulas.
- **Prompt templates**: Default prompts are defined in Rust (`main.rs`) and exposed via `get_default_prompt_templates`. Custom prompts support `{lang}` and `{text}` placeholders.
