# Remove Provider Tools Toggle

## Goal

Remove the visible "Supports tools" toggle from model provider settings because cloud providers are now treated as supporting Chat tools by default.

## What I already know

* The user wants the provider-level tools toggle removed.
* `supports_tools` still exists in backend settings and gates Chat tool exposure.
* Apple local providers must still avoid receiving tools because the local Apple adapter rejects tool requests.
* Keeping the stored field is safer than deleting it because existing settings and Rust/TypeScript types expect it.

## Requirements

* Do not show the "Supports tools" row in the model provider configuration UI.
* Normalize non-Apple providers to `supports_tools = true` so old settings with `false` do not silently disable tools.
* Keep Apple local provider tool support disabled internally.
* Avoid broad refactors of the Chat tool pipeline.

## Acceptance Criteria

* [ ] The provider configuration UI no longer contains the Supports tools toggle.
* [ ] Existing non-Apple providers with `supports_tools: false` are sanitized to `true`.
* [ ] Apple Intelligence providers remain sanitized to `false`.
* [ ] Targeted checks pass where practical.

## Out of Scope

* Removing `supports_tools` from persisted schema/types.
* Reworking provider capability detection.
* Changing Chat tool runtime behavior beyond the provider default.

## Technical Notes

* Frontend row: `src/settings/SettingsShell.tsx`
* Backend sanitizer: `src-tauri/src/settings.rs`
* Tool gating references: `src-tauri/src/chat/agent/prepare.rs`, `src-tauri/src/chat/commands.rs`
