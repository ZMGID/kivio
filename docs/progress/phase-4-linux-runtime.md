# Phase 4: Linux Runtime Implementations

**Goal**: Implement Linux paths chosen by feasibility results.
**Status**: Complete

## Tasks

- [x] **Task 4.1**: Implement Linux capture path or documented disabled state
  - Priority: P0
  - Effort: L
  - Test Expectation: Unit tests plus manual smoke.
  - Memory Impact: Record session-specific caveats.
  - Acceptance: Capture behavior verified or degraded explicitly.
  - Notes: Completed by choosing the documented disabled-state path for this phase. Backend `capture_port.rs` keeps non-macOS/non-Windows region capture explicitly unsupported, `platform_capabilities.rs` reports Linux window/region capture as unsupported, and Lens now reads `get_platform_capabilities` to show an unsupported capture hint and block window/region capture gestures when both capabilities are unavailable. Real X11/Wayland capture remains a future implementation, not claimed here.

- [x] **Task 4.2**: Implement Linux OCR route
  - Priority: P1
  - Effort: L
  - Test Expectation: Unit tests plus OCR smoke.
  - Memory Impact: Record model/runtime requirements.
  - Acceptance: OCR works or disabled state is explicit.
  - Notes: Completed for the local RapidOCR route. `rapidocr.rs` now has a test-only explicit model directory constructor so Linux smoke can download ONNX Runtime and PP-OCR models into an isolated temp directory instead of app data. The ignored Linux smoke test downloaded/initialized the runtime and completed `ocr_image` predict on a generated PNG. This proves the local RapidOCR runtime route, not the GUI/AppImage OCR flow.

- [x] **Task 4.3**: Implement Linux hotkey/window/tray behavior
  - Priority: P1
  - Effort: L
  - Test Expectation: Manual smoke checklist.
  - Memory Impact: Record desktop environment caveats.
  - Acceptance: Trigger and window behavior verified.
  - Notes: Completed on the current Ubuntu 22.04 X11 desktop through AppImage runtime smoke, not compile evidence alone. `KIVIO_DESKTOP_SMOKE=1` reported 4/4 default global shortcuts as `registered:true`, `trayPresent:true`, and a visible `chat` window. A launch-at-startup smoke with `settings.launchAtStartup=true` reported `autostartEnabled:true` and wrote `$HOME/.config/autostart/Kivio.desktop`. A real XTest `Ctrl+Shift+G` trigger produced a Kivio window path without the earlier `xcb_xlib_threads_sequence_lost` crash after adding Linux `XInitThreads()` before Tauri startup.

- [x] **Task 4.4**: Verify Agent runtime resources under Linux paths
  - Priority: P0
  - Effort: M
  - Test Expectation: Pyodide/skills packaging smoke.
  - Memory Impact: Record packaging resource invariant.
  - Acceptance: Skills/Pyodide resources load from packaged app.
  - Notes: Completed with packaged AppImage resource smoke and a headless Agent path fix. `KIVIO_DESKTOP_SMOKE=1` now includes a `resources` block. The AppImage smoke reported `resourceDir=/tmp/.mount_*/usr/lib/Kivio`, `bundledSkillsDirExists:true`, `skillCount:7`, `builtinSkillCount:7`, `skillError:null`, empty `skillWarnings`, and Pyodide assets `present:true` through Tauri `asset_resolver`. Phase 6 final smoke expanded that resource check to 25 Pyodide core/wheel/font assets with 0 missing assets. Headless `kivio-code` discovery now handles Linux AppDir layout from `usr/bin/kivio-code` to `usr/lib/Kivio/skills`, covered by `bundled_skills_dir_resolves_linux_appdir_layout`.

## Phase Notes

- Entered after Phase 3 completed platform capability, capture port, OCR port, and desktop capability UI boundaries.
- Linux capture disabled state verified:
  - `timeout 180s npm run typecheck`
  - `timeout 300s npm run build:ui`
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml capture_port`
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml platform_capabilities`
  - `git diff --check -- src/Lens.tsx src/settings/i18n.ts src-tauri/src/capture_port.rs src-tauri/src/platform_capabilities.rs`
  - Focused capture port test result: `1 passed; 0 failed`
  - Focused platform capability test result: `3 passed; 0 failed`
- Linux RapidOCR route verified:
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml rapidocr`
  - `timeout 900s cargo test --manifest-path src-tauri/Cargo.toml rapidocr::tests::linux_smoke_downloads_models_and_initializes_pipeline -- --ignored --nocapture`
  - `timeout 300s cargo test --manifest-path src-tauri/Cargo.toml`
  - `timeout 180s npm run typecheck`
  - `git diff --check`
  - Focused RapidOCR test result: `5 passed; 0 failed; 1 ignored`
  - Ignored Linux smoke result: `1 passed; 0 failed`; finished in `51.95s`
  - Full Rust test result: `1133 passed; 0 failed; 9 ignored`
  - TypeScript result: `tsc --noEmit` passed
  - Diff whitespace result: passed
  - Boundary: GUI/AppImage OCR invocation is still unverified.
- Linux hotkey/window/tray behavior verified:
  - `timeout 600s npm run tauri -- build --bundles appimage --verbose`
  - `KIVIO_DESKTOP_SMOKE=1 KIVIO_DESKTOP_SMOKE_EXIT_AFTER_MS=6000 ./src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage`
  - Default AppImage smoke result: 4/4 default hotkeys `registered:true`, `trayPresent:true`, `chat` window `visible:true`
  - Autostart smoke result: `autostartEnabled:true`, `launchAtStartupSetting:true`, generated `$HOME/.config/autostart/Kivio.desktop` with `Exec=...Kivio_2.7.2_amd64.AppImage --from-autostart`
  - Real trigger smoke: `/tmp/kivio-press-ctrl-shift-g` sent `Ctrl+Shift+G`; final phase-6 `xwininfo` evidence recorded a `1280x800` Kivio window; AppImage session ended by timeout with no xcb assertion crash
  - Full Rust test result: `1135 passed; 0 failed; 9 ignored`
  - TypeScript result: `tsc --noEmit` passed
  - Known warnings remain in Rust output: unused `app` in `shortcuts.rs`, dead `MappedSourceRect`/`source_rect_for_region`, and unused `AxSelection` variants
- Linux Agent runtime resources verified:
  - `timeout 600s npm run tauri -- build --bundles appimage --verbose`
  - `KIVIO_DESKTOP_SMOKE=1 KIVIO_DESKTOP_SMOKE_EXIT_AFTER_MS=6000 ./src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage`
  - AppImage resource smoke result: `resourceDir=/tmp/.mount_*/usr/lib/Kivio`
  - AppImage resource smoke result: `bundledSkillsDirExists:true`, `skillCount:7`, `builtinSkillCount:7`, `skillWarnings:[]`
  - Skill ids verified: `doc-coauthoring`, `docx`, `frontend-design`, `mcp-builder`, `pdf`, `skill-creator`, `xlsx`
  - Pyodide asset resolver verified: `pyodide.asm.js` `1229099` bytes, `pyodide.asm.wasm` `10088051` bytes, `pyodide-lock.json` `106335` bytes, `python_stdlib.zip` `2341872` bytes, `pyodide-package-manifest.json` `1556` bytes; all `present:true`
  - Headless Agent resource path test: `cargo test --manifest-path src-tauri/Cargo.toml bundled_skills_dir_resolves` -> `2 passed; 0 failed`
  - Full Rust test result: `1136 passed; 0 failed; 9 ignored`
  - TypeScript result: `tsc --noEmit` passed
  - Diff whitespace result: passed

## Phase Completion Checklist

- [x] All tasks above are checked off
- [x] MASTER.md phase count updated
- [x] MASTER.md "Current Status" updated to next phase
