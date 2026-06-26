# Phase 5: Linux Packaging and Release

**Goal**: Produce and verify Linux distributable.
**Status**: Complete

## Tasks

- [x] **Task 5.1**: Add selected Linux bundle target/config
  - Priority: P0
  - Effort: M
  - Test Expectation: Build selected bundle.
  - Memory Impact: Record release command.
  - Acceptance: Linux artifact builds on Ubuntu 22.04.
  - Notes: Added `src-tauri/tauri.linux.conf.json` with `bundle.targets=["appimage"]`, preserving the existing macOS/Windows targets in `tauri.conf.json`. Verified on Ubuntu 22.04 with `timeout 600s npm run build`: exit 0, produced `src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage` at `140704248` bytes.

- [x] **Task 5.2**: Inspect Linux artifact contents
  - Priority: P0
  - Effort: S
  - Test Expectation: Run package content inspection.
  - Memory Impact: Record inspection command.
  - Acceptance: Required resources present.
  - Notes: Verified final `npm run build` AppImage with AppDir listing and packaged desktop smoke. AppDir contains 7 Skill entrypoints under `usr/lib/Kivio/skills`: `doc-coauthoring`, `docx`, `frontend-design`, `mcp-builder`, `pdf`, `skill-creator`, `xlsx`. The final AppImage smoke used `/tmp/kivio-phase5-final-smoke/output.log` and reported `bundledSkillsDirExists:true`, `skillCount:7`, `builtinSkillCount:7`, `skillWarnings:[]`, and 25/25 Pyodide core/wheel/font assets `present:true` with 0 `present:false`. Final artifact size: `140708344` bytes at `2026-06-24 18:13:01 +0800`.

- [x] **Task 5.3**: Update release checklist and CI strategy
  - Priority: P1
  - Effort: M
  - Test Expectation: Docs + workflow validation.
  - Memory Impact: Update release rules.
  - Acceptance: Linux release steps are documented and reproducible.
  - Notes: Updated `RELEASE_PACKAGING.md` with the Linux AppImage release command, AppDir/desktop-smoke artifact inspection split, and GitHub Actions strategy. Updated `.github/workflows/release.yml` with `platform=linux` and `platform=all`, an `ubuntu-22.04` Linux lane, Linux system dependencies, and Linux AppImage release body entry. Validation passed: `ruby -e "require 'yaml'; YAML.load_file('.github/workflows/release.yml')"` printed `release.yml YAML parse OK`; `timeout 300s cargo test --manifest-path src-tauri/Cargo.toml` reported `1136 passed; 0 failed; 9 ignored`; `timeout 120s npm run typecheck` passed; `git diff --check` produced no output.

## Phase Notes

- Phase 4 completed on Ubuntu 22.04 X11 with AppImage runtime smoke for capture disabled state, RapidOCR route, desktop hotkey/window/tray/autostart behavior, and Agent/Pyodide/skills resources.
- Phase 5 now uses a Linux-specific Tauri config file so `npm run build` produces AppImage on Linux without the ad hoc `--bundles appimage` override.
- AppImage inspection uses two checks: AppDir file listing for bundled Skills, and packaged `KIVIO_DESKTOP_SMOKE=1` output for embedded Pyodide frontend assets. Pyodide is not expected under `usr/lib/Kivio/pyodide`.
- Phase 5 release path is now reproducible: Linux uses `tauri.linux.conf.json` with `npm run build`, artifact inspection covers AppDir Skills and embedded Pyodide assets, and CI has a Linux AppImage lane.

## Phase Completion Checklist

- [x] All tasks above are checked off
- [x] MASTER.md phase count updated
- [x] MASTER.md "Current Status" updated to next phase
