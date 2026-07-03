# Research: anthropic.rs as the template for gemini.rs (+ OpenAI-specific fields to avoid)

- **Query**: How is anthropic.rs built end-to-end (the structure gemini.rs mirrors)? Where does openai.rs add OpenAI-only fields?
- **Scope**: internal
- **Date**: 2026-07-03

## Files Found

| File Path | Description |
|---|---|
| `src-tauri/src/chat/model/anthropic.rs` | The existing NON-OpenAI peer adapter — best template for gemini.rs (1279 lines) |
| `src-tauri/src/chat/model/openai.rs` | OpenAI Chat Completions adapter (contains the fields Gemini must NOT carry) |
| `src-tauri/src/api.rs` | Shared HTTP: `send_with_failover`, `with_standard_request_timeout`, `attach_json_body`, `extract_status_code` |
| `src-tauri/src/chat/request_debug.rs` | Request-debug capture buffer (`build_debug_record`, `sanitize_headers`, `record`) |
| `src-tauri/src/usage.rs` | Usage recording (`record_model_call`, `model_usage_from_anthropic_value`, label helpers) |

## anthropic.rs structure (what gemini.rs will mirror)

### 1. Struct + construction (anthropic.rs:20-34)
```rust
pub struct AnthropicMessagesProvider<'a> {
    state: &'a AppState,
    provider: &'a ModelProvider,
    retry_attempts: usize,
}
impl<'a> AnthropicMessagesProvider<'a> {
    pub fn new(state, provider, retry_attempts) -> Self { ... }
}
```
Holds borrows only. Re-exported from `mod.rs:11`. Gemini: `GeminiProvider<'a>` with the same three fields, add `pub use gemini::GeminiProvider;` in mod.rs.

### 2. Trait impl (anthropic.rs:36-57)
`generate` / `stream` just `Box::pin(async move { self.*_inner(...).await })`; `capabilities()` returns `ProviderCapabilities { tool_calling: self.provider.supports_tools, vision: true, streaming: true, reasoning: true }`.

### 3. HTTP call — reuses the shared client + failover (anthropic.rs:60-91, 130-154)
Both `generate_inner` and `stream_inner` call the SAME plumbing:
```rust
let body = self.request_body(&request, stream);
let response = send_with_failover(
    self.state,
    &label,
    self.retry_attempts,
    &self.provider.id,
    &self.provider.api_keys,          // <- multi-key pool; failover handled here
    |key| with_standard_request_timeout(
              crate::api::attach_json_body(
                  self.state.http.post(self.messages_url())
                      .headers(anthropic_headers(key).unwrap_or_default())
                      .header(ACCEPT_ENCODING, "identity"),
                  &body,
                  self.provider.compress_request_body,   // optional gzip
              ),
          ).send(),
).await?;
```
- `self.state.http` is the shared `reqwest::Client`.
- `send_with_failover` (api.rs:268-290) takes a `send: Fn(&str)->Future<Result<Response, reqwest::Error>>` closure; the `&str` is the current API key. Key rotation / cooldown / retry all happen inside. **Gemini plugs in identically** — the ONLY difference is how the key is applied to the request (Anthropic sets `x-api-key: {key}` header; Gemini uses `?key={key}` query param OR `x-goog-api-key: {key}` header — see open questions).
- `attach_json_body` (api.rs:118) serializes + optionally gzips per `compress_request_body`.
- `with_standard_request_timeout` (api.rs:111) applies the standard timeout.
- `.header(ACCEPT_ENCODING, "identity")` disables response compression (Anthropic streaming reliability). Gemini can copy this for streaming.

### 4. Non-stream parse → GenerateOutput (anthropic.rs:93-119, 571-689)
`generate_inner` reads `response.text()`, `serde_json::from_str` → `Value`, then `output_from_anthropic_message(&value, &label)`:
- Checks top-level `error` object first (anthropic.rs:575-581).
- `parse_anthropic_response` walks `content[]` blocks (`text`/`thinking`/`tool_use`) into `content_parts` / `reasoning_parts` / `tool_calls` (anthropic.rs:610-689).
- Builds `provider_messages` via `openai_compatible_message(...)` (anthropic.rs:586-591, 964-998) — **the OpenAI-shaped assistant message required by the contract**.
- `usage` via `anthropic_usage` → `model_usage_from_anthropic_value` (anthropic.rs:960-962, usage.rs:704-720).

Gemini analog: parse `candidates[0].content.parts[]` (`text` / `functionCall` / `thought`), `finishReason`, `usageMetadata`.

### 5. Stream parse → StreamParts (anthropic.rs:121-311)
Manual SSE line buffering: read `response.chunk()`, push to `buffer`, drain on `\n`, feed each line to `parse_anthropic_sse_event` (anthropic.rs:708-792) which returns an internal `AnthropicSseEvent` enum; the loop translates those to `StreamPart`s and accumulates `full` / `reasoning_full` / `tool_calls` / `usage` / `finish_reason`. On `message_stop`/`message_delta` it emits `Finish` and returns `stream_output(...)` (anthropic.rs:1000-1023). On chunk read error it calls `stream_read_error` (types.rs:377) and records failure.

Gemini analog: `:streamGenerateContent?alt=sse` emits `data: {GenerateContentResponse}` JSON per event (each a full candidate delta with `parts`). Simpler than Anthropic's block-delta protocol: for each event, emit `TextDelta` for text parts, `ReasoningDelta` for `thought` parts, and for `functionCall` parts emit `ToolCallStart`+`ToolCallDelta`(full args)+`ToolCallDone`. Track `usageMetadata` (arrives on the final chunk) and `finishReason`.

### 6. Body builder (anthropic.rs:317-350) — the shape gemini mirrors
```rust
fn request_body(&self, request: &GenerateRequest, stream: bool) -> Value {
    // model, messages (anthropic_messages_from_generate_request), max_tokens
    // system (if non-empty) -> top-level "system"
    // stream flag
    // tools (anthropic_tools_from_model_tools) if non-empty
    // thinking_level -> {"thinking":{"type":"adaptive"}, "output_config":{"effort":level}}
    // finally: merge request.options.provider_options object verbatim
}
```
Note: Anthropic does NOT emit `temperature` in its body (only openai.rs does). Gemini puts everything under `generationConfig` (`maxOutputTokens`, `temperature`, `thinkingConfig`).

### 7. Message + tool builders (anthropic.rs:532-569, 815-893)
- `anthropic_messages_from_generate_request` maps each `ModelMessage` to a wire message, then `merge_consecutive_anthropic_roles` (anthropic.rs:895-926) folds adjacent same-role turns (needed because Tool→"user"). **Gemini needs the SAME merge** (Gemini also rejects consecutive same-role `contents`, and Tool results map to `user`/`function` role).
- `anthropic_content_blocks` (anthropic.rs:815-893) is the per-part→wire-block switch (text/image/tool_use/tool_result/thinking), role-gated. Gemini writes `gemini_parts_from_message`.
- `anthropic_tools_from_model_tools` (anthropic.rs:554-569) dedups by `openai_tool_name()` and emits `{name, description, input_schema}`. Gemini emits `functionDeclarations[]` `{name, description, parameters}` and wraps in `tools:[{functionDeclarations:[...]}]`.
- `normalize_anthropic_schema` (anthropic.rs:928-958) unwraps nullable `anyOf` + injects empty `properties` for object schemas. Gemini's schema validator is stricter (OpenAPI subset) — a `normalize_gemini_schema` is likely needed (strip `$schema`, `additionalProperties`, `format` variants Gemini rejects, etc.).

### 8. Usage + request-debug recording (anthropic.rs:352-511)
Each call path invokes `record_usage_success/failure` (usage.rs `record_model_call`) and `record_debug_success/failure` (guarded by `self.state.request_debug_enabled()`, zero-cost when off — anthropic.rs:379,413). `debug_request_headers` (anthropic.rs:355-367) rebuilds the sanitized header set for the debug panel; it must MIRROR the real send headers exactly (uses `sanitize_headers`). **Gemini must reproduce this pattern** so the request-debug panel and usage stats work — and its `debug_request_headers` must reflect the Gemini auth choice (query key won't show as a header; if `?key=` is used, the debug URL will contain the key and needs sanitizing — an open question).

### 9. Helpers
- `messages_url()` = `format!("{}/messages", base_url.trim_end_matches('/'))` (anthropic.rs:313-315, 528-530). Gemini URL is model+method-specific: `{base}/models/{model}:generateContent` vs `:streamGenerateContent?alt=sse` — the URL depends on `request.model` and stream flag (unlike Anthropic/OpenAI where model is in the body). This is the biggest structural difference.
- `request_label` (anthropic.rs:1035-1043), `finish_reason_from_anthropic_stop_reason` (anthropic.rs:1025-1033), `non_empty` (anthropic.rs:1045-1052).

### 10. Tests (anthropic.rs:1054-1279) — mirror these for gemini.rs
See dispatch-and-provider-wiring.md §Tests.

## Where openai.rs adds OpenAI-SPECIFIC fields — DO NOT carry into gemini.rs

All in `openai.rs`:

| Field / behavior | Location | Note |
|---|---|---|
| `prompt_cache_key` + `promptCacheKey` (dual-write from `metadata.conversation_id`) | openai.rs:419-430 | **This is exactly what 400s on Gemini's OpenAI-compat shim** (`Unknown name "promptCacheKey"`, see chat-probe.md:49). Gemini native must NOT send these. Gemini has its own explicit context-caching API (`cachedContent`), unrelated. |
| `stream_options: {include_usage: true}` | openai.rs:405-407 | OpenAI-only. Gemini returns `usageMetadata` natively in stream chunks. |
| `tool_choice: "auto"` | openai.rs:417 | OpenAI-only. Gemini equivalent = `toolConfig.functionCallingConfig.mode` (omit → AUTO). |
| `reasoning_effort` (from `thinking_level`) | openai.rs:439-446 | OpenAI naming. Gemini uses `generationConfig.thinkingConfig` (`thinkingBudget` / `includeThoughts`). |
| `thinking: {type:"disabled"}` (base_url-gated) | openai.rs:431-435 | OpenAI-compat proxy hack. |
| Session-affinity headers `x-session-id` / `x-session-affinity` (from conversation_id) | openai.rs:294-307, 577-588 | opencode-style routing headers. NOT needed / potentially rejected by Gemini native. |
| `temperature` at top level | openai.rs:400 | Gemini puts temperature under `generationConfig`. |
| Auth: `Authorization: Bearer {key}` | openai.rs:318 | Gemini uses `?key=` or `x-goog-api-key`, NOT Bearer. |

Anthropic's `thinking_level` mapping for contrast (anthropic.rs:335-343): `{"thinking":{"type":"adaptive"}, "output_config":{"effort": level}}`. Each adapter maps `thinking_level` to its own family — Gemini maps it to `thinkingConfig` (budget or effort-equivalent).

## Caveats / Not Found
- No trait-object registry; dispatch is a hand-written `match` in three places (see dispatch doc). Gemini requires editing each match arm.
- The `provider_options` escape hatch (merged verbatim) lets users add arbitrary Gemini body knobs without code changes — worth preserving as the last step in `request_body`.
