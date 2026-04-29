# KeyLingo Agent Guidelines

KeyLingo is a lightweight desktop translation and AI vision utility for macOS and Windows. Its core focus is a small package size and low runtime footprint, providing instant text translation, screenshot translation, and AI-powered visual Q&A through global shortcuts.

## Tech Stack & Architecture

- **Frontend**: React 18 + TypeScript + Vite + TailwindCSS v4 (ESM)
- **Backend**: Rust + Tauri v2
- **Package Manager**: npm (lockfile: `package-lock.json`)
- **Build Targets**: macOS (DMG), Windows (MSI + NSIS)

The app uses a classic Tauri split architecture: a single-page React frontend invokes Rust backend commands via Tauri's `invoke` bridge. The backend handles global shortcuts, window management, system tray, screenshot capture, HTTP API calls, and settings persistence.

## Project Directory Structure

```
src/                          # Frontend React + TypeScript source
  main.tsx                    # React entry point (mounts to #root)
  App.tsx                     # Root component: switches views by URL hash
  Settings.tsx                # Settings page main component
  Lens.tsx                    # Lens (screenshot translation + AI vision Q&A)
  api/tauri.ts                # Tauri bridge: all invoke calls & event listeners centralized
  settings/                   # Settings UI helper modules
    components.tsx            # Reusable form components (Toggle, Select, Input, etc.)
    i18n.ts                   # Internationalization strings & language utilities
    utils.ts                  # Settings page utilities (hotkey formatting, platform detection)
  index.css                   # Global styles (Tailwind imports, scrollbar, transparent window)
  App.css                     # Component-specific styles

src-tauri/
  src/                        # Rust backend source
    main.rs                   # App main entry: state, commands, hotkeys, tray, API calls
    lens.rs                   # Lens window enumeration and screenshot capture
    screenshot.rs             # Screenshot capture utilities and temp file cleanup
    sck.rs                    # ScreenCaptureKit integration (macOS 14+)
    settings.rs               # Settings data structures, serialization, migration
    windows.rs                # Window creation & retrieval helpers
    utils.rs                  # Language detection, timestamps, etc.
  tauri.conf.json             # Tauri app config (windows, bundling, icons)
  Cargo.toml                  # Rust dependencies
  icons/                      # App icon assets

public/                       # Static assets (icons, SVGs)
.github/workflows/release.yml # GitHub Actions automated release workflow
```

## Core Module Details

### Frontend View Routing (Hash-based)

`App.tsx` switches modes via `window.location.hash`, mapping to different windows/views:

- `''` or `'translator'`: Main translator window (392x152, floating input)
- `'settings'`: Settings page (640x520)
- `'lens'`: Lens window (600x72 select mode / 600x420 answering mode, floating)

### Rust Backend State (`AppState`)

Defined in `main.rs`, the global shared state includes:

- `settings: RwLock<Settings>` — App settings (multiple readers, single writer)
- `explain_images: Mutex<HashMap<String, PathBuf>>` — Temporary image map for Lens
- `current_explain_image_id: Mutex<Option<String>>` — Currently active Lens image
- `lens_busy: AtomicBool` — Concurrency guard for Lens operations
- `explain_stream_generation: AtomicU64` — Stream cancellation token
- `key_cooldowns: Mutex<HashMap<(String, usize), Instant>>` — API key failover cooldown tracking
- `active_key_idx: Mutex<HashMap<String, usize>>` — Currently active API key index per provider
- `http: Client` — Shared HTTP client for API calls

### Settings Persistence & Security

- Settings body is stored in Tauri Store as `settings.json`
- **API Keys are stored directly in `settings.json`** (as of v2.4.0); the `keyring` crate is only used for one-time migration from legacy keyring storage
- On load, `sanitize_settings` cleans data and migrates legacy single-provider configs to the multi-provider system
- `settings.rs` contains full defaults, normalization logic (hotkeys, prompts), and keyring migration helpers

### Multi-Provider Routing

The app supports separate OpenAI-compatible providers for each feature:

- **Text Translation**: `translator_provider_id` + `translator_model`
- **Screenshot Translation / OCR**: `screenshot_translation.provider_id` + `model`
- **Lens (AI Vision)**: `lens.provider_id` + `model`

Each `ModelProvider` has `id`, `name`, `base_url`, `api_keys`, `available_models`, and `enabled_models`.

### API Key Failover

- Each provider stores multiple API keys (`api_keys: string[]`)
- Primary key is `api_keys[0]`, backups follow
- Backend `send_with_failover` automatically rotates to next key on 401/402/403/429 or quota/billing/balance errors
- Failed key enters 60-second cooldown before retry
- **Test Connection intentionally only probes the primary key**

### Platform-Specific Handling

- **macOS**:
  - Screenshots use ScreenCaptureKit (SCK) for interactive region/window capture (macOS 14+)
  - Auto-paste uses AppleScript to send `Command+V`
  - Permission checks: Accessibility (`AXIsProcessTrustedWithOptions`) and Screen Recording (`CGPreflightScreenCaptureAccess`)
  - Cocoa / Objective-C FFI is used for `NSApplication hide:` and workspace behavior
  - Dock icon is hidden (`ActivationPolicy::Accessory`)
  - `sck.rs` handles SCScreenshotManager integration with prewarming for performance
- **Windows**:
  - Screenshots use the `ms-screenclip:` system tool + clipboard polling
  - Region capture uses the `xcap` library's `Monitor::capture_region`, with a fullscreen transparent overlay for frontend region selection
  - Auto-paste uses `enigo` to simulate `Ctrl+V`
  - The `capture` window is pre-created at startup for faster subsequent captures

### HTTP API & Retry Logic

- Backend uses `reqwest` with a uniform 60-second timeout
- All outbound API calls (translate, OCR, vision, fetch models, test connection) go through `send_with_retry`
- Retry policy: exponential backoff for 429 / 5xx / timeout / connection errors; respects `Retry-After` headers
- Retry count is controlled by `retry_enabled` and `retry_attempts` (1-5, default 3)

### Lens Flow

1. Hotkey triggered (`Cmd/Ctrl+Shift+G`)
2. Enter `select` mode: fullscreen overlay showing app windows (hover highlight + label on macOS)
3. User clicks a window or drags a region → capture screenshot
4. Generate `image_id`, store temp image in `explain_images` map
5. Open / reuse the `lens` window; frontend reads the image via `explain_read_image`
6. Call `explain_get_initial_summary` for the initial summary (supports streaming)
7. User can ask follow-up questions via `explain_ask_question` (multi-turn conversation)
8. History keeps the most recent 5 records, persisted via `explain_save_history`
9. Supports pure-text questions without screenshot

### Screenshot Translation Flow

1. Hotkey triggered (`Cmd/Ctrl+Shift+A`) → hide main window
2. **macOS**: call `capture_screenshot()` directly; **Windows**: open `capture_request` overlay and wait for user selection
3. After image submission → OCR (call vision model to recognize text)
4. If `direct_translate` mode is on, OCR returns translated text directly, skipping the second translation step
5. Otherwise: OCR result → text translation → emit `screenshot-result` event to the Lens window

### Screenshot Explain Flow

1. Hotkey triggered (`Cmd/Ctrl+Shift+G`) → capture image (same as above)
2. Generate `image_id`, store temp image in `explain_images` map
3. Open / reuse the `explain` window; frontend reads the image via `explain_read_image`
4. Call `explain_get_initial_summary` for the initial summary (supports streaming)
5. User can ask follow-up questions via `explain_ask_question` (multi-turn conversation)
6. History keeps the most recent 5 records, persisted via `explain_save_history`

## Build & Development Commands

```bash
# Install dependencies
npm install

# Full dev mode (Rust backend + Vite frontend HMR)
npm run dev

# Frontend-only dev (Vite on port 5713)
npm run dev:ui

# Build full app bundle
npm run build

# Build frontend bundle only
npm run build:ui

# Lint (ESLint)
npm run lint

# Preview built frontend bundle
npm run preview
```

Rust dependencies are managed by `cargo`; the Tauri CLI coordinates frontend and backend builds. `tauri.conf.json` defines:

- `beforeDevCommand`: `npm run dev:ui`
- `beforeBuildCommand`: `npm run build:ui`
- `devUrl`: `http://localhost:5713`
- `frontendDist`: `../dist`

## Coding Style & Naming Conventions

- **Languages**: TypeScript + React frontend; standard Rust style for backend
- **Module format**: ESM (`"type": "module"`)
- **Indentation**: 2 spaces
- **Quotes**: single quotes
- **Semicolons**: omitted
- **Naming**:
  - Component files: `PascalCase.tsx`
  - Utility / service files: `camelCase.ts`
- **Styling**: prefer Tailwind utility classes; shared styles in `src/index.css`, component-specific in `src/App.css`

## Testing Strategy

- No automated test runner is configured
- Manual smoke-test checklist after changes:
  1. `npm run dev` — verify the app launches
  2. Global hotkeys (translator, screenshot translation, Lens)
  3. Translation flow (input -> debounce -> result -> Enter to commit/paste)
  4. Screenshot translation / Lens windows open correctly
  5. Settings save/load and persistence across restarts
  6. Provider connection test and model fetching
  7. Unsaved-changes close guard in settings
  8. Theme switching (light/dark/system)

## Deployment & Release

- GitHub Actions workflow at `.github/workflows/release.yml`
- Triggered on `v*` tags or manual `workflow_dispatch`
- Build matrix:
  - `macos-14` -> DMG bundle
  - `windows-latest` -> MSI + NSIS bundles
- Uses `tauri-apps/tauri-action@v0` to build and publish release assets

## Security & Configuration Guidelines

- **Never commit API keys or base URLs**; they are configured through the app settings UI
- API Keys are stored directly in `settings.json` (as of v2.4.0); the `keyring` crate is only used for one-time migration from legacy keyring storage
- External URLs are validated to start with `https://` before opening (`open_external` command)
- Explain image paths are validated to reside inside the system temp directory (`resolve_explain_image_path`)
- If you add new Tauri permissions or capabilities, update `src-tauri/tauri.conf.json` and document defaults

## Commit & Pull Request Guidelines

- Git history follows Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`)
- Use short, imperative subjects
- PRs should include a concise summary, testing notes, and screenshots/GIFs for UI changes
