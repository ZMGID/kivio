# Phase 6: Regression, Smoke, and Handoff

**Goal**: Prove Linux target without regressing current platforms.
**Status**: Complete

## Tasks

- [x] **Task 6.1**: Run automated gates
  - Priority: P0
  - Effort: M
  - Test Expectation: `npm run lint`, `npm run typecheck`, `npm test`, `cargo test`.
  - Memory Impact: Record slow/flaky gates.
  - Acceptance: Gates pass or failures are triaged.
  - Notes: Automated gates passed on 2026-06-24. `timeout 120s npm run lint` exited 0. `timeout 120s npm run typecheck` exited 0. `timeout 120s npm test` reported `21 passed (21)` files and `82 passed (82)` tests. `timeout 300s cargo test --manifest-path src-tauri/Cargo.toml` reported `1136 passed; 0 failed; 9 ignored`; Rust still emits existing warnings in `shortcuts.rs` and `capture_geometry.rs`.

- [x] **Task 6.2**: Run Linux desktop smoke checklist
  - Priority: P0
  - Effort: M
  - Test Expectation: Manual smoke with evidence.
  - Memory Impact: Record environment matrix.
  - Acceptance: Core workflows verified on Ubuntu 22.04.
  - Notes: Completed on `Ubuntu 22.04.5 LTS`, kernel `6.8.0-40-generic`, `XDG_SESSION_TYPE=x11`, `DISPLAY=:1`, using final AppImage size `140708344` bytes. Default AppImage smoke at `/tmp/kivio-phase6-default-smoke/output.log` exited 0 and reported 4/4 hotkeys `registered:true`, `trayPresent:true`, visible `chat`, 7 bundled Skills, `skillWarnings:[]`, 25 Pyodide core/wheel/font assets `present:true`, and 0 missing assets. Autostart smoke at `/tmp/kivio-phase6-autostart-smoke/output.log` exited 0 and reported `launchAtStartupSetting:true`, `autostartEnabled:true`, plus generated `$HOME/.config/autostart/Kivio.desktop` at 223 bytes. Real `Ctrl+Shift+G` smoke with `/tmp/kivio-press-ctrl-shift-g` exited 0 and `xwininfo` recorded a `1280x800` Kivio window after the trigger; this proves a real hotkey-triggered window path without an xcb crash, but the X11 title in this run was `Kivio` rather than `Lens`.

- [x] **Task 6.3**: Archive spec-driven artifacts
  - Priority: P1
  - Effort: S
  - Test Expectation: Not applicable: archive/docs.
  - Memory Impact: Preserve durable decisions.
  - Acceptance: Active docs archived per workflow.
  - Notes: Archived analysis, plan, progress, governance snapshots, and handoff notes under `docs/archives/kivio-linux-porting-2026-06-24/`. Added `docs/archives/README.md`, `governance/instruction-surfaces.md`, `governance/memory-surface.md`, and `skill/SKILL.md`. This task is documentation/archive only; validation is file presence plus `git diff --check`.

## Phase Notes

- Phase 5 completed with a reproducible Linux AppImage release path, final artifact inspection, and a Linux GitHub Actions release lane.
- Before starting Phase 6, get explicit checkpoint confirmation. Phase 6 runs broader regression gates and handoff work beyond the packaging changes.
- Current Phase 6 smoke environment: Ubuntu 22.04.5 LTS, kernel 6.8.0-40-generic, X11.

## Phase Completion Checklist

- [x] All tasks above are checked off
- [x] MASTER.md phase count updated
- [x] MASTER.md "Current Status" updated to complete/archive
