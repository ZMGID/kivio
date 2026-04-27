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

The app supports multiple OpenAI-compatible providers. Each of the three features can use a different provider/model:
- **Translator** (`translatorProviderId` + `translatorModel`)
- **Screenshot Translation/OCR** (`screenshotTranslation.providerId` + `model`)
- **Screenshot Explain** (`screenshotExplain.providerId` + `model`)

Providers have `availableModels` (fetched from `/models` endpoint) and `enabledModels` (user-selected subset used in dropdowns). Model selection UI uses colon-delimited values like `providerId:modelName`.

### Settings Persistence and Security

- Settings are stored via `tauri-plugin-store` in `settings.json`.
- **API keys are NOT stored in settings JSON**. They are saved to the OS keyring (via the `keyring` crate) and hydrated at runtime. The `api_key` field on `ModelProvider` is populated in-memory at load time.
- **`sanitize_settings`** in `src-tauri/src/settings.rs` handles migration from legacy single-provider configs to the multi-provider system, validates provider existence, and normalizes hotkeys. `normalize_hotkey` canonicalizes modifier aliases to `CommandOrControl`, `Control`, `Alt`, `Shift`, `Super` — use these exact strings when constructing hotkeys.
- Saving settings is transactional: if hotkey registration fails, `restore_runtime_settings` rolls back to the previous state.

### Platform-Specific Screenshot Flows

Screenshot capture is platform-guarded with `cfg(target_os = ...)`:

- **macOS**: Uses the native `screencapture -i` command for interactive region selection. The `BusyGuard` RAII pattern prevents concurrent captures.
- **Windows**: Uses a custom `CaptureOverlay.tsx` (fullscreen transparent webview) for region selection. The lifecycle is: `capture_request` opens the overlay → the user draws a region → `capture_commit` (with pixel coordinates) triggers `xcap` capture → `capture_cancel` dismisses without capturing. Windows also has a clipboard-based fallback in `screenshot.rs` using `ms-screenclip:`.

Busy flags (`screenshot_translation_busy`, `screenshot_explain_busy`) prevent concurrent operations on both platforms.

### Rust Backend Structure

- **`main.rs`** — App state (`AppState`), Tauri commands, OpenAI API calling logic (`call_openai_text`, `call_openai_ocr`, `call_vision_api`), retry logic (`send_with_retry`), hotkey registration, and window lifecycle.
- **`settings.rs`** — Settings schema, serde defaults, migration/sanitization, keyring hydration, persistence.
- **`screenshot.rs`** — Platform-specific screenshot capture (`capture_screenshot`) and temp file cleanup.
- **`windows.rs`** — Window creation helpers (`ensure_main_window`, `ensure_screenshot_window`, `ensure_capture_overlay_window`).
- **`utils.rs`** — Language detection, target language resolution, timestamp utility.

Key crate responsibilities from `Cargo.toml`:
- `enigo` — simulates keyboard paste after translation commit.
- `arboard` — clipboard read/write.
- `keyring` — secure API key storage.
- `reqwest` — HTTP client for OpenAI-compatible APIs.
- `xcap` — Windows region screen capture.

### Streaming

Screenshot explain supports streaming responses. `stream_vision_response` in `main.rs` parses SSE chunks and emits `explain-stream` events to the frontend. The frontend (`ScreenshotExplain.tsx`) merges deltas into the current message using a `streamingRef` to track the active stream.

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
