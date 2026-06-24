# Phase 3: Platform Boundary Refactor

**Goal**: Prepare replaceable Linux platform ports without changing feature intent.
**Status**: Completed

## Tasks

- [x] **Task 3.1**: Define platform capability/status contracts
  - Priority: P0
  - Effort: M
  - Test Expectation: Add unit tests for pure contract logic.
  - Memory Impact: Record contract convention.
  - Acceptance: UI can query capability status without platform guessing.
  - Notes: Added `platform_capabilities.rs`, `get_platform_capabilities`, frontend `PlatformCapabilities` types, `api.getPlatformCapabilities()`, and SettingsShell System OCR gating from the backend contract. Linux now reports window/region capture and System OCR as `unsupported`, RapidOCR as `supported` with smoke required, and desktop-dependent capabilities as `degraded`.

- [x] **Task 3.2**: Isolate screenshot/capture platform ports
  - Priority: P0
  - Effort: L
  - Test Expectation: Rust tests for non-OS logic.
  - Memory Impact: Record platform module pattern.
  - Acceptance: macOS/Windows behavior preserved; Linux stub explicit.
  - Notes: 2026-06-24 compile-gate patch aligned Linux capture stub signature with the shared caller and preserved explicit unsupported return. Follow-up boundary patch moved platform-specific region screenshot logic into `capture_port.rs`, kept macOS/Windows callers behind one request type, and left Linux as explicit unsupported until Phase 4.1.

- [x] **Task 3.3**: Isolate OCR platform ports
  - Priority: P1
  - Effort: M
  - Test Expectation: Rust tests for routing/default decisions.
  - Memory Impact: Record OCR fallback rule.
  - Acceptance: Linux OCR route can be implemented independently.
  - Notes: 2026-06-24 compile-gate patch added Linux ONNX Runtime names/URLs/archive path so RapidOCR compiles on Linux. Follow-up routing patch preserves RapidOCR on Linux and keeps System OCR falling back to CloudVision on unsupported platforms. Task 3.3 added `ocr_port.rs` for platform availability, mode normalization, and local OCR dispatch; Settings UI now exposes RapidOCR independently from System OCR. Runtime download/init remains Phase 4.2.

- [x] **Task 3.4**: Isolate hotkey/window/tray platform assumptions
  - Priority: P1
  - Effort: L
  - Test Expectation: Rust/TS tests where possible; smoke checklist otherwise.
  - Memory Impact: Record unsupported behavior handling.
  - Acceptance: Unsupported Linux capability returns explicit state.
  - Notes: 2026-06-24 compile-gate patch moved `libc` to Unix dependencies for Linux shell/kivio-code paths. Capability contract reports Linux global shortcuts, tray, autostart, and transparent overlay as degraded/smoke-required. Settings UI now consumes the backend capability status for global shortcut, transparent overlay, and autostart descriptions without disabling runtime attempts. Actual hotkey/window/tray smoke remains Phase 4.3.

## Phase Notes

- Compile gate passed:
  - `timeout 300s cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-unknown-linux-gnu`
  - `timeout 900s bash -lc 'npm run build:swift && npm exec tauri -- build --bundles appimage --ci'`
- AppImage produced:
  - `src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage`
- OCR routing regression fixed and tested:
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml sanitize_settings`
  - `timeout 300s cargo test --manifest-path src-tauri/Cargo.toml`
  - Full Rust test result: `1126 passed; 0 failed; 8 ignored`
- AppImage rebuild after OCR routing patch passed:
  - `timeout 900s bash -lc 'npm run build:swift && npm exec tauri -- build --bundles appimage --ci'`
  - Result: `Finished 1 bundle at: /home/jn/kivio/src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage`
- Non-GUI AppImage runtime metadata checks passed:
  - `--appimage-help` printed AppImage runtime options
  - `--appimage-offset` printed `944632`
- Capability contract landed:
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml platform_capabilities`
  - `timeout 180s npm run typecheck`
  - `timeout 300s cargo test --manifest-path src-tauri/Cargo.toml`
  - `timeout 300s npm run build:ui`
  - Full Rust test result: `1129 passed; 0 failed; 8 ignored`
- Capture port boundary landed:
  - `timeout 60s rustfmt --check src-tauri/src/capture_port.rs src-tauri/src/platform_capabilities.rs`
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml capture_port`
  - `timeout 300s cargo test --manifest-path src-tauri/Cargo.toml`
  - `timeout 180s npm run typecheck`
  - `git diff --check -- AGENTS.md docs/progress/MASTER.md docs/progress/phase-3-platform-boundary.md docs/analysis/linux-desktop-capability-matrix.md src-tauri/src/capture_port.rs src-tauri/src/lens_commands.rs src-tauri/src/lib.rs`
  - Full Rust test result: `1130 passed; 0 failed; 8 ignored`
- OCR port boundary landed:
  - `rustfmt --edition 2021 --check src-tauri/src/ocr_port.rs`
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml ocr_port`
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml sanitize_settings`
  - `timeout 180s npm run typecheck`
  - `timeout 300s cargo test --manifest-path src-tauri/Cargo.toml`
  - `timeout 300s npm run build:ui`
  - `git diff --check -- AGENTS.md docs/progress/MASTER.md docs/progress/phase-3-platform-boundary.md docs/analysis/linux-desktop-capability-matrix.md src-tauri/src/ocr_port.rs src-tauri/src/settings.rs src-tauri/src/lens_commands.rs src-tauri/src/lib.rs src/settings/SettingsShell.tsx src/settings/ScreenshotTranslationSettings.tsx`
  - Focused OCR test result: `2 passed; 0 failed`
  - Focused settings test result: `27 passed; 0 failed`
  - Full Rust test result: `1132 passed; 0 failed; 8 ignored`
- Desktop capability UI landed:
  - `timeout 120s cargo test --manifest-path src-tauri/Cargo.toml platform_capabilities`
  - `timeout 180s npm run typecheck`
  - `timeout 300s npm run build:ui`
  - Focused platform capability test result: `3 passed; 0 failed`
- This does not prove screen capture, OCR runtime, global shortcut, tray, or overlay behavior. Those remain open acceptance gates.

## Phase Completion Checklist

- [x] All tasks above are checked off
- [x] MASTER.md phase count updated
- [x] MASTER.md "Current Status" updated to next phase
