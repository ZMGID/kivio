# Adapt AI Client Chrome For Windows

## Goal

Adapt the AI client window chrome for Windows before release. Windows should keep the frameless custom window, but use Windows-native control placement: the sidebar titlebar action pill should move left because there are no macOS traffic lights, and minimize / maximize / close controls should live at the top right.

## What I already know

* Scope is only the AI client UI.
* Translation window, Lens, screenshot/OCR, installer, and update behavior are out of scope.
* macOS already uses native traffic lights in the top-left area and should keep the current layout.
* Windows has no macOS traffic lights, so the top-left spacing should collapse.
* Windows should show custom-drawn minimize, maximize/restore, and close controls in the upper-right corner.
* Windows controls should be wrapped in a pill container, visually matching the existing top-left action pill style.
* The Chat window is already configured as frameless on non-macOS in `src-tauri/src/windows.rs`.

## Requirements

* Keep the AI client window frameless on Windows.
* On Windows, place custom window controls at the top-right corner of the Chat window.
* On Windows, wrap those controls in a pill, not native titlebar chrome.
* On Windows, move the sidebar titlebar action pill left into the space otherwise reserved for macOS traffic lights.
* Preserve macOS Chat layout and native traffic light behavior.
* Preserve existing sidebar collapse, new chat, and other titlebar actions.
* Keep drag regions usable while ensuring buttons remain clickable.

## Acceptance Criteria

* [ ] Windows Chat titlebar shows minimize, maximize/restore, and close controls at the top right.
* [ ] Windows sidebar top action pill is aligned left without macOS traffic-light padding.
* [ ] macOS Chat layout remains visually unchanged.
* [ ] `npm run lint` passes.
* [ ] `npm run typecheck` passes.

## Out of Scope

* Lens UI or capture behavior.
* Translation floating window UI.
* OCR, screenshots, hotkeys, installer, update behavior.
* Reworking the whole Chat visual system.

## Technical Notes

* Likely files: `src/chat/Sidebar.tsx`, `src/chat/Chat.tsx`, `src/chat/WindowControls.tsx`, `src/chat/platform.ts`, `src/App.css`, `src/index.css`.
* Existing platform helpers expose `isMac`, `usesNativeTitlebar`, `chatTitlebarRowClass`, and `chatTitlebarMacInsetClass`.
* Existing `WindowControls` already has a non-macOS branch with Windows-style controls.
