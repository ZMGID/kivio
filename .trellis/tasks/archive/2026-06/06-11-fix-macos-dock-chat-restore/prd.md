# Fix macOS Dock Chat Window Restore

## Goal

Fix an installed macOS build issue where the Chat window can restore from Dock minimization into a malformed always-on-top-looking state that macOS does not treat like a normal window.

## What I Already Know

- The user reproduced this mainly in the installed macOS package, especially after minimizing the Chat window to the Dock and clicking the app/Dock icon again.
- The page content remains interactive, which points to an NSWindow / Tauri window lifecycle problem rather than a React rendering crash.
- Restarting the app recreates the native window and clears the bad state.
- `RunEvent::Reopen` currently has a branch that can call only `show()` and `set_focus()` for an existing visible user window.
- The frontend `revealChatWindow` path handles `isMinimized()`, `unminimize()`, geometry restore, show, and focus, but the Dock reopen path may bypass it.

## Assumptions

- The malformed screenshot is caused by restoring a miniaturized macOS `chat` window without fully reapplying Chat's normal-window behavior and unminimizing it.
- The fix should be backend-first so packaged builds and Dock reopen behavior are consistent even if the frontend route effect does not run.

## Requirements

- Dock reopen for Chat must use the same normalization as other Chat open paths.
- Restoring an existing Chat window must explicitly unminimize before show/focus.
- Chat must remain a normal desktop window on macOS: not always-on-top, not visible on all workspaces, not skipped from the Dock/taskbar.
- Lens and translator floating-window behavior must be unchanged.
- The fix should be narrowly scoped and avoid changing chat content rendering.

## Acceptance Criteria

- [ ] `open_chat_window` unminimizes existing Chat windows before show/focus.
- [ ] macOS `RunEvent::Reopen` delegates Chat restoration to the Chat open path instead of directly doing `show()` / `set_focus()` on Chat.
- [ ] Existing visible non-Chat user windows can still be focused on Dock reopen.
- [ ] Rust code builds and targeted checks pass where practical.

## Definition of Done

- Lint/type/test checks relevant to the touched code are run where practical.
- Any inability to run packaged macOS smoke testing is reported.

## Out of Scope

- Redesigning Chat window UI/chrome.
- Changing Lens overlay, translator window, or screenshot selection behavior.
- Adding new frontend tests for visual layout.

## Technical Notes

- Relevant files inspected:
  - `src-tauri/src/main.rs`
  - `src-tauri/src/shortcuts.rs`
  - `src-tauri/src/windows.rs`
  - `src/App.tsx`
  - `src/chat/persistence.ts`
- Backend spec index has no specific window lifecycle guide; follow existing module boundaries and small backend change conventions.
