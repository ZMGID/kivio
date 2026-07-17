# Model temperature configuration — Technical Design

## Problem and Design Principle

Temperature is currently a required generation option with a global default of `0.7`. That makes an optional, model-dependent sampling parameter part of every OpenAI Chat and Gemini request, which breaks strict models such as `kimi-k3`.

The design makes temperature optional and model-scoped. Absence is meaningful: if no built-in model value and no provider/model override exists, the wire request omits the field.

## Data Model

Extend the shared `ModelInfo` contract in Rust and TypeScript with:

- `temperature?: number` / `Option<f64>` — an explicit value to send.
- `omitTemperature?: boolean` / `omit_temperature: Option<bool>` — an override-only tombstone allowing a user to clear a future built-in database value and force omission.

The built-in JSON model database accepts an optional `temperature` property. No current database entry receives a value in this task.

The tombstone is required because the existing override merge uses “override value or database default”. A plain missing numeric field cannot distinguish “inherit the database value” from “the user cleared the input and wants omission”.

## Resolution Contract

Add a backend helper in `chat/model_metadata.rs` that returns `Option<f32>` with this precedence:

1. Exact provider/model override with `omitTemperature == true` → `None`.
2. Exact provider/model override with a valid numeric temperature → that value.
3. Built-in model database with a valid numeric temperature → that value.
4. Otherwise → `None`.

Valid values are finite and within `0.0..=2.0`. Invalid persisted or database values are ignored defensively.

Frontend `resolveModelInfo` mirrors the same precedence for display:

1. Override tombstone shows a blank field.
2. Override numeric value wins.
3. Database numeric value is displayed.
4. Otherwise the field is blank.

## UI and Persistence

Add a Temperature numeric input to `ModelDetailDrawer`, near Context Window and Max Output.

- Placeholder: `-`.
- Step: `0.1`; min: `0`; max: `2`.
- Helper text: blank means the parameter is not sent.
- Entering a valid number sets `temperature` and clears `omitTemperature`.
- Clearing the field removes `temperature` and sets `omitTemperature` so a database value can be explicitly suppressed.
- Invalid/non-finite/out-of-range values keep Save disabled and display a concise validation message.
- Reset continues to remove the whole provider/model override, restoring database behavior.

Persistence reuses `ModelProvider.modelOverrides`; no new settings file or migration command is needed. Older settings deserialize through serde defaults.

## Request Construction

Change `GenerateOptions.temperature` from `f32` to `Option<f32>` and default it to `None`.

OpenAI Chat and Gemini adapters calculate an effective value as:

1. Explicit per-request `GenerateOptions.temperature`, if present.
2. Model metadata/override resolution from the adapter's provider and request model.

They serialize temperature only when the result is `Some`.

Anthropic Messages and OpenAI Responses use the same effective-value resolution as OpenAI Chat and Gemini. They omit the field by default and serialize it only when configured.

Translator text, OCR, vision, Lens selected-text, and local-OCR translation construct raw OpenAI-compatible bodies independently. Route all of them through a shared flat-body helper that conditionally inserts the model-scoped temperature and otherwise omits the field.

Use `f64` end to end for persisted metadata, generation options, resolution, and JSON serialization. This preserves configured decimal values exactly in request debug output instead of exposing `f32` artifacts such as `0.4000000059604645`.

## Compatibility

- Existing settings without the new fields continue to deserialize.
- Existing models change from global `0.7` to provider defaults because the first release intentionally leaves every database temperature unset.
- Existing explicit provider options remain higher-level request-body overrides where already supported; this task does not add conversation-level temperature.
- Request debug records remain accurate because adapters record the same constructed body they send.

## Testing Strategy

- TypeScript model matching tests: absence by default, numeric override, explicit omit tombstone, reset/inheritance semantics.
- Rust model metadata tests: override/database/omit precedence and invalid range rejection.
- OpenAI Chat request-body tests: omit by default, include configured value, explicit request value wins.
- Gemini request-body tests: omit from `generationConfig` by default and include configured value.
- Settings serde test: older provider/model override JSON without temperature remains valid.
- Shared raw OpenAI-compatible body helper test covering default omission and configured insertion.

## Rollback

The change is additive in persisted settings. Rollback removes UI and resolution use; older binaries ignore unknown JSON fields because settings structs deserialize with defaults. No destructive migration is required.
