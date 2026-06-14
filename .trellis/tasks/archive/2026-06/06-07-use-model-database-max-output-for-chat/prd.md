# Use Model Database Max Output For Chat

## Goal

Chat generation should use model metadata as the source of truth for a model's maximum output tokens, so high-output models are not artificially capped by the current fixed settings dropdown.

## What I Already Know

- Kivio has an internal model database at `src/data/modelDatabase.json` with `contextWindow` and `maxOutput` fields.
- The Rust backend already embeds that database in `src-tauri/src/chat/commands.rs` for context window and capability checks.
- The current Chat request path passes `settings.chat.max_output_tokens` directly to provider requests as `max_tokens`.
- The user's current Chat model is `deepseek-v4-flash`; the database lists `maxOutput` as `131072`, while the saved Chat setting is `32768`.

## Requirements

- Resolve Chat request `max_tokens` from model metadata:
  - provider `model_overrides[model].maxOutput` first,
  - built-in model database `maxOutput` second,
  - existing `settings.chat.max_output_tokens` fallback when metadata is missing.
- Keep existing provider request formats unchanged apart from the resolved token value.
- Preserve the existing context window behavior.
- Add focused Rust coverage for the metadata resolution behavior.

## Acceptance Criteria

- [ ] `deepseek-v4-flash` resolves Chat output limit to `131072` from the built-in model database.
- [ ] Provider model override `maxOutput` takes precedence over built-in database values.
- [ ] Unknown/custom models still use the saved Chat setting.
- [ ] Targeted Rust tests pass.

## Out Of Scope

- Changing the Settings UI dropdown.
- Fetching live model metadata from provider APIs.
- Changing frontend context indicators or pricing UI.

## Technical Notes

- Backend model database helpers currently live in `src-tauri/src/chat/commands.rs`.
- Chat generation calls are in `src-tauri/src/chat/agent/loop_.rs`.
- Relevant spec read: `.trellis/spec/backend/agent-runtime.md`.
