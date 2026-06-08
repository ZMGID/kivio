# Fix Chat Model Default Sync

## Goal

Fix two settings synchronization bugs found during review: changing a single Chat conversation model must not silently change the global default Chat model, and deleting a provider must not leave stale Chat provider/model fields in the Settings UI state.

## What I Already Know

* `chat_update_conversation` currently persists `default_models.chat` whenever `provider_id` or `model` is present.
* `SettingsShell.deleteProvider` clears `defaultModels.chat` before checking whether it matched the deleted provider, making the stale-field branch ineffective.
* The current agent loop treats tool rounds as unlimited; this avoids the earlier finite-limit behavior where reaching the configured count produced a hard error.

## Requirements

* Conversation model changes should update only the conversation.
* Global default Chat model changes should remain owned by Settings save/default-model controls.
* Provider deletion should clear stale `chatProviderId`/`chatModel` when they point at the deleted provider.
* Keep the current unlimited tool-round behavior unless inspection shows a remaining finite-limit error path.

## Acceptance Criteria

* [ ] `chat_update_conversation` no longer writes or persists global settings for provider/model updates.
* [ ] Deleting a provider clears stale Chat provider/model state when that provider was selected.
* [ ] Lint, typecheck, and Rust tests pass where practical.

## Out Of Scope

* Redesigning model selector UX.
* Reintroducing a finite tool-round setting.
* Changing agent loop behavior unless a direct regression is found.

## Technical Notes

* Backend file: `src-tauri/src/chat/commands.rs`.
* Frontend file: `src/settings/SettingsShell.tsx`.
* Relevant specs: `.trellis/spec/backend/index.md`, `.trellis/spec/frontend/index.md`, `.trellis/spec/guides/index.md`.
