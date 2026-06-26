# Phase 2: Linux Feasibility Spikes

**Goal**: Decide whether direct AppImage is viable before deeper refactor.
**Status**: Complete

## Tasks

- [x] **Task 2.1**: Verify Linux build prerequisites on Ubuntu 22.04
  - Priority: P0
  - Effort: M
  - Test Expectation: Run install/build preflight with explicit timeout.
  - Memory Impact: Record required system packages.
  - Acceptance: Dependency list and failing/passing commands recorded.
  - Notes: Completed 2026-06-24. `npm ci` first failed with `ECONNRESET`, then passed with retry env and `--prefer-offline`.

- [x] **Task 2.2**: Probe Tauri AppImage bundle target
  - Priority: P0
  - Effort: M
  - Test Expectation: Run bundle feasibility command and inspect output.
  - Memory Impact: Record AppImage decision.
  - Acceptance: AppImage proceed/fallback decision documented.
  - Notes: Completed 2026-06-24. Tauri supports `appimage`; current project reaches Rust compile but fails on Linux compile blockers.

- [x] **Task 2.3**: Build Linux desktop capability matrix
  - Priority: P0
  - Effort: M
  - Test Expectation: Docs/static validation.
  - Memory Impact: Add future-agent gotchas if discovered.
  - Acceptance: X11/Wayland/portal matrix completed.
  - Notes: Completed 2026-06-24. Current session is Ubuntu GNOME on X11; portal packages are installed.

## Phase Notes

- Evidence documents:
  - `docs/analysis/linux-feasibility.md`
  - `docs/analysis/linux-desktop-capability-matrix.md`
- AppImage decision: plausible target, not directly available until Linux compile blockers are fixed.

## Phase Completion Checklist

- [x] All tasks above are checked off
- [x] MASTER.md phase count updated
- [x] MASTER.md "Current Status" updated to next phase
