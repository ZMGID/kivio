# Fix AI chat IME enter sending

## Goal

Prevent the AI Chat composer from sending messages when the user presses Enter to confirm an IME candidate. Normal Enter should still send, and Shift+Enter should still insert a newline.

## What I Already Know

* The user reports that the AI chat client sends immediately on Enter, which makes Chinese IME input awkward and easy to mis-send.
* `src/chat/InputBar.tsx` owns the standalone AI Chat composer textarea.
* Translator (`src/App.tsx`) and Lens (`src/Lens.tsx`) already guard Enter handling with `nativeEvent.isComposing || keyCode === 229`.
* `InputBar.tsx` currently sends on `Enter && !Shift` without checking IME composition state.

## Requirements

* In the AI Chat composer, pressing Enter while IME composition is active must not send the message.
* Pressing Enter outside IME composition must continue to send the message when Shift is not held.
* Pressing Shift+Enter must continue to allow newline input.
* Keep the implementation consistent with existing project keyboard handling patterns.

## Acceptance Criteria

* [x] `InputBar` skips send when `e.nativeEvent.isComposing` is true.
* [x] `InputBar` skips send when `e.keyCode === 229`.
* [x] Existing disabled / empty-send guards remain unchanged.
* [x] `npm run typecheck` passes.
* [x] `npm run lint` passes.

## Out of Scope

* Changing Chat send shortcuts or adding user-configurable shortcut settings.
* Changing Lens or Translator input behavior beyond inspecting existing patterns.
* Backend changes.

## Technical Notes

* Relevant frontend spec: `.trellis/spec/frontend/index.md`, `.trellis/spec/frontend/quality-guidelines.md`, `.trellis/spec/frontend/component-guidelines.md`, `.trellis/spec/frontend/type-safety.md`.
* Existing IME-safe pattern:
  * `src/App.tsx` translator input
  * `src/Lens.tsx` Lens/chat input
