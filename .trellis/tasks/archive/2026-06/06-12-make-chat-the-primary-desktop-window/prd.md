# Make Chat the primary desktop window

## Goal

Turn Kivio from a tray/background-helper style app into a standard desktop app where Chat is the primary window. Manual launches should present Chat as a normal Dock/Cmd+Tab/window-capture app, while autostart remains quiet until the user opens Chat from Dock, tray, single-instance activation, or shortcuts.

## Requirements

- Remove the macOS bundle-level agent identity so Kivio is no longer packaged as an `LSUIElement` helper app.
- Keep manual app launch behavior: open Chat by default after startup.
- Keep autostart behavior quiet: `--from-autostart` should not open Chat immediately.
- Preserve Chat as the normal desktop window: not always on top, not visible on all workspaces, not skipped from taskbar/Dock, normal restore path.
- Remove the runtime behavior that switches the app back to `ActivationPolicy::Accessory` when the last user window closes.
- Preserve tray menu, global shortcuts, translator, Lens, and existing Chat chrome styling.
- Keep Dock reopen behavior routed through `open_chat_window` so Chat restore normalization still applies.

## Acceptance Criteria

- [ ] Packaged macOS Info.plist no longer declares `LSUIElement=true`.
- [ ] Normal launch opens Chat and presents Kivio as a standard desktop app.
- [ ] Closing Chat closes the window but does not switch Kivio back into background-helper identity.
- [ ] Clicking the Dock icon after closing all windows reopens Chat.
- [ ] `--from-autostart` startup remains silent until the user activates Chat.
- [ ] Tray actions for Chat, translator, settings, and quit continue to work.
- [ ] `npm run typecheck` passes.
- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` passes or any failure is documented.

## Definition of Done

- Code changes are limited to the app lifecycle/bundle identity path unless verification finds related necessary fixes.
- Backend window lifecycle spec remains satisfied for Chat restore entry points.
- Manual smoke-test notes identify any behavior that cannot be fully verified in this environment.

## Technical Approach

- Remove `LSUIElement` from `src-tauri/Info.plist`.
- Compute `launched_from_autostart` once during `setup` and use it for activation policy and default Chat open behavior.
- On macOS, set `ActivationPolicy::Accessory` only for autostart quiet mode; set/keep `Regular` for normal launches.
- Remove the `CloseRequested` branch that demotes the app to `Accessory` when the last user window closes.
- Keep existing `open_chat_window` and `open_chat_settings_window` behavior that switches macOS back to `Regular` when Chat is opened.
- Keep existing `RunEvent::Reopen` behavior because it already routes Dock reopen through `open_chat_window`.

## Decision (ADR-lite)

Context: Chat should be capturable and managed like a real desktop window. The previous global `LSUIElement=true` identity and runtime `Accessory` demotions made the app behave like a tray helper even when Chat looked like a normal window.

Decision: Make the app bundle a standard macOS app, use Chat as the primary window, and reserve `Accessory` only for quiet autostart before user activation.

Consequences: Kivio will appear in Dock/Cmd+Tab during normal use. Users retain tray and shortcut flows. If some capture tools still fail, a later task can make the Chat titlebar less custom by replacing `TitleBarStyle::Overlay`.

## Out of Scope

- Redesigning Chat chrome or replacing `TitleBarStyle::Overlay`.
- Changing Lens or translator visual/window behavior beyond ensuring existing activation paths still work.
- Removing tray support or global shortcuts.

## Technical Notes

- Relevant spec: `.trellis/spec/backend/window-lifecycle.md`.
- Main files: `src-tauri/Info.plist`, `src-tauri/src/main.rs`, with validation of `src-tauri/src/shortcuts.rs` / `src-tauri/src/windows.rs` behavior.
