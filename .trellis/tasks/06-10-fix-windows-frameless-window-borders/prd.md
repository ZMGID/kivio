# Fix Windows Frameless Window Borders and Shadows

## Goal

Restore clear visual boundaries for Kivio's Windows frameless windows so Chat, translator, and Lens floating UI feel like intentional desktop surfaces rather than borderless web rectangles, while preserving the current custom chrome, transparent-window behavior, low footprint, and macOS behavior.

## Problem Statement

On Windows, Kivio disables native window decorations and shadows for the main translator, Chat, and Lens windows. Some frontend shells have light borders and shadows, but Chat fills the full native window rectangle and Lens cards use weak shadows. As a result, the app edge can disappear against similar backgrounds, especially in light mode.

## What We Know

* The project intentionally uses frameless transparent windows for custom chrome and floating UI.
* Tauri supports undecorated Windows shadows, but documents a 1px white border / top-line side effect and Windows 11 rounded-corner behavior.
* Tao exposes the same Windows-specific undecorated shadow controls and notes that the shadow is hidden by default.
* Windows native/window background alpha is not enough by itself; Kivio must coordinate native window transparency, WebView transparency, and HTML/CSS transparency.
* Lens uses a single window for fullscreen select overlay and floating ready/answer states, so any native shadow setting there must not leak into fullscreen select mode.

## Research References

* [`research/windows-frameless-shadow.md`](research/windows-frameless-shadow.md) - source-backed findings and approach comparison.
* Tauri Window Customization: https://v2.tauri.app/learn/window-customization/
* Tauri `WindowBuilder::shadow`: https://docs.rs/tauri/latest/tauri/window/struct.WindowBuilder.html
* Tao Windows `with_undecorated_shadow`: https://docs.rs/tao/latest/x86_64-pc-windows-msvc/tao/platform/windows/trait.WindowBuilderExtWindows.html
* Tao Windows `set_undecorated_shadow`: https://docs.rs/tao/latest/x86_64-pc-windows-msvc/tao/platform/windows/trait.WindowExtWindows.html

## Requirements

* Improve Windows visual boundaries for the Chat window in normal/restored mode.
* Preserve borderless custom chrome and the existing Windows `WindowControls` UI.
* Preserve Chat maximized behavior: no rounded edge, no inner padding gap, no artificial outer shadow when maximized.
* Improve translator floating window boundary without making the compact window feel bulky.
* Improve Lens ready/answer/translate floating card boundaries.
* Keep Lens fullscreen select overlay visually clean, with no shadow or border around the full-screen overlay.
* Preserve macOS Chat native overlay titlebar behavior and existing macOS shadow behavior.
* Avoid adding new runtime dependencies unless implementation proves the built-in Tauri/Tao APIs are insufficient.
* Keep all styling theme-aware for light, dark, and system themes.

## Non-Functional Requirements

* The fix should not meaningfully increase app package size.
* The fix should not introduce persistent GPU-heavy animations or expensive repaint loops.
* The fix should keep pointer interactions reliable for drag regions, resize behavior, Lens region selection, and normal text input.
* The implementation should stay centralized enough that future frameless windows can reuse the same shell tokens.

## Recommended Technical Approach

Use a hybrid CSS-first implementation with narrowly scoped native-shadow experimentation:

* Add or reuse a Windows platform marker in the frontend so Windows-only shell styles can be applied without changing macOS.
* For Chat, give the non-maximized Windows host a transparent outer gutter and strengthen `.chat-window-shell` border/shadow tokens so shadows have room to render instead of being clipped by the native window rectangle.
* Ensure Chat maximized mode removes the gutter, radius, border, and shadow exactly as it does today.
* Account for Chat min-size/content-size expectations if adding a gutter reduces available content area.
* Strengthen `.window-frosted` or add Windows-specific variants for translator and Lens floating cards.
* Do not enable native shadow for Lens fullscreen select mode.
* Optionally test `shadow(true)` for the Chat window only. Keep it only if manual Windows testing confirms it improves depth without unacceptable 1px line, resize, transparency, or layout regressions.

## Decision (ADR-lite)

**Context**: Windows frameless transparent windows do not get native borders/shadows when `decorations(false)` and `shadow(false)` are used. Tauri/Tao can enable undecorated shadows, but their docs warn about visible 1px line side effects and platform-specific behavior.

**Decision**: Prefer a CSS-first visual shell fix as the reliable baseline. Treat native Windows undecorated shadow as a scoped enhancement for Chat, not as a global default for every frameless window.

**Consequences**: The baseline fix remains predictable across Kivio's different window modes. If native shadow is adopted for Chat, it must be guarded by Windows-only code and validated manually. Lens overlay remains protected from unwanted full-screen edge artifacts.

## Acceptance Criteria

* [ ] On Windows, Chat restored mode has a clearly visible outer boundary in both light and dark themes.
* [ ] On Windows, Chat maximized mode has no artificial rounded border, gutter, or outer shadow.
* [ ] On Windows, Chat custom window controls remain usable and visually aligned after any shell padding changes.
* [ ] On Windows, Chat resizing still respects minimum content sizes and does not produce clipped controls.
* [ ] On Windows, the translator popover has a visible edge/shadow and no clipped card corners.
* [ ] On Windows, Lens select mode has no visible full-screen border or window shadow.
* [ ] On Windows, Lens ready/answer/translate floating cards have visible boundaries without interfering with drag, selection, or input.
* [ ] macOS Chat, translator, and Lens behavior remain visually unchanged except for shared CSS improvements that are intentionally cross-platform.
* [ ] `npm run lint` passes.
* [ ] `npm run typecheck` passes.
* [ ] `cargo test --manifest-path src-tauri/Cargo.toml` passes when practical.

## Manual Smoke Checklist

* Windows 11: open Chat from tray, resize, maximize, restore, switch light/dark/system theme.
* Windows 11: open translator, type text, close via Esc, reopen near cursor.
* Windows 11: enter Lens select mode and verify the overlay has no border around the screen.
* Windows 11: capture a Lens region, ask a question, show answer card, and verify card boundary.
* Windows 11: run screenshot translation and verify the translate card boundary.
* Windows 10 if available: repeat Chat normal/maximized checks.

## Out of Scope

* Replacing the custom Chat chrome with a native Windows titlebar.
* Redesigning Chat layout or window controls.
* Adding acrylic/vibrancy/blur effects.
* Reworking Lens window architecture into separate overlay and floating windows.
* Linux window styling.
* Release packaging changes.

## User Decision

Phase 2 should use **A. Hybrid CSS-first + scoped native Chat shadow test**.

Implementation should keep CSS shell padding and stronger visual tokens as the reliable baseline, then enable or preserve native Windows shadow only for ordinary Chat windows if it stays scoped away from Lens fullscreen overlay behavior.

## Implementation Plan

1. Add Windows-specific shell styling hooks and strengthen shared frameless window visual tokens.
2. Update Chat host/shell styles for non-maximized Windows restored mode, preserving maximized behavior.
3. Update translator and Lens floating card shadows/rings without changing Lens fullscreen select mode.
4. Optionally test and keep Chat-only native `shadow(true)` if it passes Windows manual checks.
5. Run lint, typecheck, and Rust tests where practical.

## Technical Notes

Likely files to inspect or modify during implementation:

* `src-tauri/tauri.conf.json`
* `src-tauri/src/windows.rs`
* `src/chat/ChatWindowHost.tsx`
* `src/chat/Chat.tsx`
* `src/chat/platform.ts`
* `src/App.tsx`
* `src/Lens.tsx`
* `src/index.css`

Relevant current behavior:

* `main` translator window is created from config with `decorations: false`, `transparent: true`, and `shadow: false`.
* Chat non-macOS builder uses `decorations(false)`, `transparent(true)`, `shadow(false)`, and transparent background color.
* Lens builder uses `decorations(false)`, `shadow(false)`, `transparent(true)`, and transparent background color.
* `.window-container` already reserves 16px for CSS shadow spread, but `.chat-window-host` currently does not.
