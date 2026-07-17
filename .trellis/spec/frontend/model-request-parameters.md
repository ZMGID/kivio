# Model Request Parameters

## Scenario: Optional model-scoped sampling parameters

### 1. Scope / Trigger

- Trigger: adding or changing an optional model request parameter such as `temperature` across the model database, settings overrides, UI, and provider request bodies.
- Strict OpenAI-compatible endpoints may reject a normally valid field with HTTP 400. Optional sampling parameters must therefore be model-scoped and omitted unless explicitly configured.

### 2. Signatures

- TypeScript metadata: `ModelInfo.temperature?: number` and `ModelInfo.omitTemperature?: boolean`.
- Rust metadata: `ModelInfo.temperature: Option<f64>` and `ModelInfo.omit_temperature: Option<bool>`.
- Per-request override: `GenerateOptions.temperature: Option<f64>`.
- Backend resolution:

```rust
temperature_for_model(provider: Option<&ModelProvider>, model: &str) -> Option<f64>
temperature_for_request(
    explicit: Option<f64>,
    provider: Option<&ModelProvider>,
    model: &str,
) -> Option<f64>
```

### 3. Contracts

- All built-in models default to no `temperature`; do not introduce a global fallback.
- Model resolution order is: explicit omit tombstone, provider/model numeric override, built-in database value, absent.
- A valid per-request value wins over model metadata.
- Empty UI input persists `omitTemperature: true`; resetting the whole model override restores database behavior.
- OpenAI Chat and raw OpenAI-compatible bodies use a root `temperature` field. Gemini uses `generationConfig.temperature`.
- OpenAI Chat, OpenAI Responses, Anthropic Messages, and Gemini all resolve the same optional model-scoped value and serialize it only when present.
- Store and serialize temperature as `f64` so values such as `0.4` remain `0.4` in request debug output.

### 4. Validation & Error Matrix

| Condition | Required behavior |
|---|---|
| Missing value | Omit the request field |
| Finite value in `0.0..=2.0` | Persist and serialize the exact value |
| `NaN`, infinity, below `0`, or above `2` | Disable UI save or ignore defensively; never serialize it |
| `omitTemperature == true` | Omit even if a database value exists |
| Invalid per-request value with a valid model value | Ignore the invalid explicit value and use the valid model value |
| Legacy settings without either field | Deserialize both as absent |

### 5. Good / Base / Bad Cases

- Good: a provider override sets `temperature: 0.4`; the request debug body contains exactly `0.4`.
- Base: no database or override value exists; the request body has no `temperature` key.
- Bad: every model request receives `temperature: 0.7` or a feature-specific hardcoded `0.2`.

### 6. Tests Required

- Frontend resolution: default absence, numeric override, and explicit omit tombstone.
- Rust metadata: missing, override, omit, range/non-finite rejection, and per-request precedence.
- OpenAI Chat and Gemini body builders: default omission, configured inclusion, and explicit request precedence.
- Raw OpenAI-compatible body helpers used by translator, OCR, vision, and Lens: omission by default and configured insertion.
- Settings serde: a legacy `ModelInfo` object without temperature fields deserializes successfully.
- Repository search: no hardcoded numeric `temperature` remains in production request construction.

### 7. Wrong vs Correct

#### Wrong

```rust
let body = serde_json::json!({
    "model": model,
    "temperature": 0.7,
});
```

#### Correct

```rust
let mut body = serde_json::json!({ "model": model });
if let Some(value) = temperature_for_model(Some(provider), model) {
    body["temperature"] = serde_json::json!(value);
}
```
