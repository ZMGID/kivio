# Instruction Surfaces Snapshot

## Shared Surface

- Active file: `AGENTS.md`
- Archive snapshot: `governance/AGENTS.md`
- Role: shared project rules for Codex, Cursor, and other Markdown-aware agents.

## Claude Code Surface

- Active file: `CLAUDE.md`
- Archive snapshot: `governance/CLAUDE.md`
- Role: Claude Code-specific repository guidance.

## Other Platform Surfaces

- `.trellis/` is referenced by `AGENTS.md`, but was absent in the worktree during this run.
- No `.cursor/`, `.windsurf/`, `.clinerules*`, or repo-local `.codex/` rule surface was found or created for this run.

## Stable Linux Porting Rules Added

- Use `docs/progress/MASTER.md` as the active LOCAL_ONLY continuity entry until Trellis state is restored.
- Linux desktop feature availability must flow through backend capabilities, not `navigator.platform` alone.
- Screenshot capture must stay behind `capture_port.rs`.
- OCR platform availability and local OCR dispatch must stay behind `ocr_port.rs`.
- AppImage runtime claims require packaged smoke evidence from `KIVIO_DESKTOP_SMOKE=1`.
- Packaged Skills are verified from `usr/lib/Kivio/skills`; Pyodide frontend assets are verified through the packaged Tauri asset resolver.
- Linux AppImage `kivio-code` must resolve skills from `usr/bin` to `usr/lib/Kivio/skills`.
