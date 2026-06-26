<!-- TRELLIS:START -->
# Trellis Instructions

These instructions are for AI assistants working in this project.

This project is managed by Trellis. The working knowledge you need lives under `.trellis/`:

- `.trellis/workflow.md` — development phases, when to create tasks, skill routing
- `.trellis/spec/` — package- and layer-scoped coding guidelines (read before writing code in a given layer)
- `.trellis/workspace/` — per-developer journals and session traces
- `.trellis/tasks/` — active and archived tasks (PRDs, research, jsonl context)

If a Trellis command is available on your platform (e.g. `/trellis:finish-work`, `/trellis:continue`), prefer it over manual steps. Not every platform exposes every command.

If you're using Codex or another agent-capable tool, additional project-scoped helpers may live in:
- `.agents/skills/` — reusable Trellis skills
- `.codex/agents/` — optional custom subagents

Managed by Trellis. Edits outside this block are preserved; edits inside may be overwritten by a future `trellis update`.

<!-- TRELLIS:END -->

## Linux Porting Spec Baseline

This repository currently has no `.trellis/` directory in the worktree. For the
Kivio Linux porting/refactor run, use `docs/progress/MASTER.md` as the active
LOCAL_ONLY progress entry until Trellis state is restored or the user selects a
different tracker.

Do not start Linux adaptation code changes before reviewing:

- `docs/plan/linux-porting-22-rules.md`
- `docs/analysis/project-overview.md`
- `docs/analysis/module-inventory.md`
- `docs/analysis/risk-assessment.md`
- `docs/plan/task-breakdown.md`

Linux adaptation must preserve the current Tauri v2, Rust, React 18,
TypeScript, Vite, and TailwindCSS v4 stack unless a documented feasibility
result proves a specific exception is necessary.

Linux desktop feature availability must flow through the backend
`get_platform_capabilities` command. UI code may keep local platform detection
only as a temporary fallback before the backend contract loads; it must not
promise screenshot, OCR, shortcut, tray, autostart, or overlay behavior from
`navigator.platform` alone.

Screenshot and region capture platform implementations must stay behind
`capture_port.rs`. Command orchestration code may build capture requests and
handle results, but must not reintroduce macOS/Windows/Linux `cfg` capture
branches into the Lens command layer.

OCR platform availability, mode normalization, and local OCR dispatch must stay
behind `ocr_port.rs`. Treat System OCR and RapidOCR as separate capabilities:
Linux may expose RapidOCR while System OCR remains unsupported.

Settings UI copy for desktop-dependent behavior must consume backend
capability status when available. Do not present Linux global shortcut, tray,
autostart, transparent overlay, screenshot, or OCR behavior as fully supported
unless the backend contract reports it and a smoke test is recorded.

RapidOCR Linux runtime support must be proven by either the explicit temp model
directory smoke test in `rapidocr.rs` or a recorded real app-data status check.
Do not claim Linux OCR runtime support from compile success alone.

Linux AppImage desktop behavior must be proven through packaged runtime smoke,
not a bare cargo binary. Use `KIVIO_DESKTOP_SMOKE=1` on
`Kivio_2.7.2_amd64.AppImage` and record hotkey registration, tray presence,
window visibility, and any autostart state before claiming support.

Linux X11 GUI startup must call `XInitThreads()` before Tauri, GTK, WebKit,
global shortcuts, or XTest/Xlib paths can run. Without that early init, real
global shortcut triggers can crash with `xcb_xlib_threads_sequence_lost`.

Linux autostart currently uses the `auto-launch` crate's fixed
`$HOME/.config/autostart` path. Create that directory before enabling
launch-at-startup; do not rely on `XDG_CONFIG_HOME` for this crate version.

Packaged Agent resources must be proven from the AppImage runtime path. The
desktop smoke `resources` block must show bundled skills under
`/tmp/.mount_*/usr/lib/Kivio/skills` and Pyodide assets resolved through Tauri
`asset_resolver`; do not claim Pyodide/skills packaging from `dist/` or build
logs alone.

Linux AppImage content inspection has two different resource surfaces: bundled
Skills are visible in the AppDir under `usr/lib/Kivio/skills`, while Pyodide
frontend assets are embedded and must be verified by the packaged
`KIVIO_DESKTOP_SMOKE=1` asset resolver output. Do not expect a
`usr/lib/Kivio/pyodide` directory in the AppDir.

Headless Linux AppImage Agent code (`kivio-code`) must resolve bundled skills
from the AppDir layout `usr/bin` -> `usr/lib/Kivio/skills`. Keep the
`bundled_skills_dir_resolves_linux_appdir_layout` test passing when touching
skill discovery.
