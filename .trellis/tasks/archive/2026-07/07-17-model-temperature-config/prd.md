# Model temperature configuration

## Goal

Make model sampling temperature a model-scoped, editable, optional parameter instead of a global unconditional `0.7`, so strict models such as `kimi-k3` work while users can still tune models that support temperature.

## Background

- The current request path defines `GenerateOptions.temperature` as a required `f32` with default `0.7` in `src-tauri/src/chat/model/types.rs:138-159`.
- The OpenAI Chat adapter always serializes `temperature` in `src-tauri/src/chat/model/openai.rs:453-461`. Gemini also always serializes it in `src-tauri/src/chat/model/gemini.rs:294-301`; the other adapters consume the same required option.
- Live provider tests on 2026-07-17 established that `kimi-k3` succeeds with a bare request and fails with the same upstream HTTP 400 when `temperature: 0.7` is present. `prompt_cache_key` succeeds and is not the cause.
- Model metadata already flows through the built-in database and per-provider user overrides: `src/data/modelDatabase.json` → `src/data/modelMatching.ts` → `ModelInfo` → `provider.modelOverrides` → `src/components/ModelDetailDrawer.tsx`.
- The model details drawer already edits optional numeric metadata such as context window, maximum output, and pricing, and persists overrides through `SettingsShell.saveModelOverride`.

## Requirements

- R1. Add an optional `temperature` field to the shared frontend/backend `ModelInfo` contract and the built-in model database schema.
- R2. Display an editable numeric Temperature field in the model details drawer shown from provider model settings.
- R3. Treat an empty Temperature field as “do not send a temperature parameter”.
- R4. Persist user changes through the existing provider `modelOverrides` mechanism and restore the database value when the model override is reset.
- R5. Resolve the effective temperature with the same precedence as other model metadata: exact per-provider override first, then built-in model database, otherwise absent.
- R6. Change model generation options and every provider adapter so temperature is serialized only when an effective value exists.
- R7. When neither the model database nor a provider/model override defines temperature, omit the parameter; there is no historical global `0.7` fallback.
- R8. Preserve existing context window, maximum output, capability, reasoning-effort, pricing, and provider-option behavior.
- R9. Validate user-entered values as finite numbers in the standard sampling range `0.0..=2.0`; invalid values must not be saved or sent.
- R10. Add focused tests for database/override resolution, request serialization with and without temperature, and settings compatibility with older files that lack the field.
- R11. Do not prefill temperature for any existing built-in model in the first release; the model database schema supports the field, but all current entries remain unset.
- R12. Remove every independent hardcoded temperature in translator, OCR, vision, and Lens raw OpenAI-compatible requests; these paths must use the same model metadata/override resolution and omit-by-default behavior.

## Acceptance Criteria

- [x] AC1. Opening a model detail drawer shows a Temperature input alongside other model parameters.
- [x] AC2. Saving a numeric Temperature value persists it in that provider/model override and subsequent requests contain exactly that value.
- [x] AC3. Clearing the Temperature input persists an explicit omit decision when needed and subsequent requests do not contain `temperature`.
- [x] AC4. Resetting a model override restores the built-in database temperature behavior.
- [x] AC5. `kimi-k3` sends no `temperature` by default and the previously reproduced minimal Zen request no longer fails because of `temperature: 0.7`.
- [x] AC6. Models with a built-in temperature value send that value unless a provider override replaces or clears it.
- [x] AC7. Existing settings without temperature deserialize successfully and do not acquire an accidental invalid value.
- [x] AC8. OpenAI Chat, OpenAI Responses, Anthropic Messages, and Gemini request tests verify omission when temperature is absent.
- [x] AC9. Translator, OCR, vision, and Lens requests omit temperature by default and use the configured provider/model override when present.

## Out of Scope

- Conversation-level or per-message temperature controls.
- Automatic remote discovery of a provider model's supported sampling range.
- Adding top-p or top-k controls in this task.
