# Research: Dispatch + provider wiring (apiFormat, settings, frontend, credentials, tests)

- **Query**: Where does the runtime pick the adapter by apiFormat? How is apiFormat defined/validated/surfaced in UI? How do credentials/failover resolve? What's the test pattern?
- **Scope**: internal
- **Date**: 2026-07-03

## A. Adapter dispatch sites (the `match provider.api_format_kind()` blocks)

The dispatch is a hand-written `match` on `ProviderApiFormat`, duplicated across **five** call sites. **Gemini must add a `ProviderApiFormat::Gemini` arm to every one:**

| File:line | Function | Context |
|---|---|---|
| `src-tauri/src/chat/agent/planning.rs:740-757` | `generate_with_chat_provider` | main non-stream path (agent loop) |
| `src-tauri/src/chat/agent/planning.rs:766-783` | `stream_with_chat_provider` | main stream path (agent loop) |
| `src-tauri/src/chat/commands.rs:5303-5320` | `generate_with_chat_provider` | commands-side non-stream |
| `src-tauri/src/kivio_code/vision.rs:114-128` | vision generate | kivio-code vision path |
| `src-tauri/src/chat/image_generation.rs:146,197` | image gen | gated on `OpenAiChat` only (Gemini image gen out of scope unless desired) |

Each arm looks like:
```rust
match provider.api_format_kind() {
    ProviderApiFormat::OpenAiChat => OpenAiChatProvider::new(state, provider, retry).generate(request).await,
    ProviderApiFormat::AnthropicMessages => AnthropicMessagesProvider::new(...).generate(request).await,
    ProviderApiFormat::OpenAiResponses => OpenAiResponsesProvider::new(...).generate(request).await,
    // + ProviderApiFormat::Gemini => GeminiProvider::new(...).generate(request).await,
}
```
Adding a new enum variant makes the compiler flag every non-exhaustive match — a reliable checklist for finding all sites.

## B. `ProviderApiFormat` enum + `ModelProvider` (settings.rs)

Definition (`src-tauri/src/settings.rs:80-111`):
```rust
pub enum ProviderApiFormat { OpenAiChat, AnthropicMessages, OpenAiResponses }

impl ProviderApiFormat {
    pub fn from_raw(raw: &str) -> Self {           // :90-96
        match raw.trim() {
            "anthropic" | "anthropic_messages" => Self::AnthropicMessages,
            "openai_responses" | "responses"   => Self::OpenAiResponses,
            _ => Self::OpenAiChat,                  // default fallback
        }
    }
    pub fn as_str(self) -> &'static str { ... }     // :98-104  "openai_chat"/"anthropic_messages"/"openai_responses"
}
impl ModelProvider { pub fn api_format_kind(&self) -> ProviderApiFormat { ProviderApiFormat::from_raw(&self.api_format) } }  // :107-111
```
**Gemini change:** add variant `Gemini`, accept raw `"gemini" | "google" | "gemini_generate"` (pick canonical string, likely `"gemini"`) in `from_raw`, add its `as_str`.

`ModelProvider` struct (`settings.rs:45-78`): `api_format: String` field with `#[serde(default = "default_api_format")]` (settings.rs:66-67). Relevant sibling fields the adapter reads: `id`, `name`, `api_keys: Vec<String>`, `base_url`, `supports_tools`, `compress_request_body`. `default_api_format()` returns `"openai_chat"` (settings.rs:2224-2226).

### sanitize_settings normalization (settings.rs:1476-1478)
```rust
for provider in &mut settings.providers {
    provider.supports_tools = true;
    provider.api_format = provider.api_format_kind().as_str().to_string();  // canonicalizes raw → canonical
    ...
}
```
So `from_raw`/`as_str` are the single source of canonicalization — adding Gemini's raw aliases + canonical there covers persistence. No other validation of `api_format` exists (unknown values silently become `openai_chat`).

## C. Frontend apiFormat wiring

### `src/api/tauri.ts`
- `ModelProvider.apiFormat: string` field (tauri.ts:632).
- `normalizeProviderApiFormat` (tauri.ts:1010-1014):
  ```ts
  export function normalizeProviderApiFormat(apiFormat?: string): string {
    if (apiFormat === 'anthropic' || apiFormat === 'anthropic_messages') return 'anthropic_messages'
    if (apiFormat === 'openai_responses' || apiFormat === 'responses') return 'openai_responses'
    return 'openai_chat'
  }
  ```
  **Add a Gemini branch** (mirror backend `from_raw`).
- Called in `normalizeProvider` (tauri.ts:1006). Usage/debug DTOs also carry `apiFormat` (tauri.ts:857, 938).

### UI dropdown — `src/settings/SettingsShell.tsx:4197-4206`
```tsx
<Select value={normalizeProviderApiFormat(provider.apiFormat)}
        onChange={(apiFormat) => updateProvider(provider.id, { apiFormat })}
        options={[
          { value: 'openai_chat', label: 'OpenAI Chat' },
          { value: 'openai_responses', label: 'OpenAI Responses' },
          { value: 'anthropic_messages', label: 'Anthropic' },
        ]} />
```
**Add `{ value: 'gemini', label: 'Gemini' }`.** Default new-provider apiFormat is `'openai_chat'` (SettingsShell.tsx:1572,1594; onboarding ProviderSetupPanel.tsx:81,95; ModelSelector.tsx:38).

### Provider presets — `src/settings/providerPresets.ts`
`ProviderPreset = { name, baseUrl }` (only name + OpenAI-compat base URL; **no apiFormat field** — presets don't set format). Current presets: DeepSeek, OpenRouter, SiliconFlow, GLM, Ollama. **No Gemini preset exists.** If a Gemini preset is wanted, either extend `ProviderPreset` with an optional `apiFormat` or add a preset whose base URL is Gemini's native root (`https://generativelanguage.googleapis.com/v1beta`) — note presets currently only prefill baseUrl, so the user would still pick "Gemini" in the format dropdown.

### ThinkingLevelSelector (src/chat/ThinkingLevelSelector.tsx:53-56)
Resolves a provider's `apiFormat` and calls `api.reasoningEffortsForModel(model, apiFormat)`. Backend `reasoning_efforts_for_model(model, api_format)` (model_metadata.rs:143-161) branches on `api_format == "anthropic_messages"`. If Gemini exposes thinking levels, add a Gemini branch here + in `chat_reasoning_efforts_for_model` (commands.rs:1175-1177).

## D. Credentials / base_url / failover resolution (chat path)

- The adapter is constructed with `&ModelProvider` directly; it reads `provider.base_url`, `provider.api_keys`, `provider.id` itself. There is **no separate credential-resolution step** for the chat path — the provider struct carries everything.
- Failover: `send_with_failover(state, label, attempts, &provider.id, &provider.api_keys, |key| ...)` (api.rs:268-290). The closure receives each candidate key; rotation/cooldown/retry live in `send_with_failover_cancelable` (api.rs:292+) keyed by `provider.id`. Gemini plugs in identically; only the per-key application differs (query param vs header — see gemini-api-shape.md).
- `retry_attempts` is threaded from the caller into `Provider::new(state, provider, retry_attempts)`.
- Shared HTTP client: `state.http`. Body attach + optional gzip: `crate::api::attach_json_body(builder, &body, provider.compress_request_body)` (api.rs:118). Timeout: `with_standard_request_timeout` (api.rs:111). Status extraction from error strings for usage/debug: `crate::api::extract_status_code(err)` (api.rs:227).

## E. Test pattern to mirror

### Rust adapter unit tests
`anthropic.rs` tests (anthropic.rs:1054-1279) and `openai.rs` tests (around openai.rs:880-991) cover:
1. **Body building via the production path** — construct a headless state + `ModelProvider`, build a `GenerateRequest`, call `adapter.request_body(&request, stream)`, assert wire fields. Helper `build_anthropic_body` (anthropic.rs:1088-1123) and openai's `request_body_carries_session_cache_key_and_stream_usage` (openai.rs:932-991). Uses `crate::state::AppState::new_headless(Settings::default(), std::env::temp_dir())` (anthropic.rs:1089).
2. **Message → wire mapping** — `canonical_text_image_and_tool_result_become_content_blocks` (anthropic.rs:1140-1181), `canonical_assistant_tool_call_becomes_tool_use` (anthropic.rs:1183-1206).
3. **Response → GenerateOutput parse** — `parses_anthropic_output_to_generate_output` (anthropic.rs:1208-1230) asserts text/reasoning/tool_calls/finish_reason/usage.
4. **SSE parse** — `parses_anthropic_stream_text_reasoning_and_tool_use` (anthropic.rs:1232-1278) feeds raw `data:` lines to the SSE parser and asserts the decoded events + assembled tool call.

Gemini tests should mirror all four: `generationConfig`/`systemInstruction`/`functionDeclarations` body assertions (incl. asserting NO `promptCacheKey`/`tool_choice`), `contents[]`+`functionResponse` mapping, `candidates` parse, and `alt=sse` chunk parse.

A `ModelProvider` literal for tests must set every field (see anthropic.rs:1093-1106): `id, name, api_keys, api_key_legacy: None, base_url, available_models, enabled_models, supports_tools, enabled, api_format, model_overrides: Default::default(), compress_request_body`.

### End-to-end validation — chat-probe
`docs/chat-probe.md` (debug-only file channel). Write `<app_data>/chat_probe/request.json` with `{id, prompt, provider, model, cwd}`; watcher runs the FULL agent loop through the real GUI generation path and writes `result.json` with `answer` + `toolCalls` + `error`. **chat-probe.md:49 explicitly documents THIS task**: Gemini's OpenAI-compat endpoint returns `400 Unknown name "promptCacheKey"`, and states "正确方向是以后为 Gemini 做原生接口协议适配 (peer adapter)". Use probe to confirm the native adapter (a) no longer 400s and (b) the model calls tools correctly. Windows path: `%APPDATA%\com.zmair.kivio\chat_probe\request.json`.

## Caveats / Not Found
- `image_generation.rs` only handles `OpenAiChat` (image_generation.rs:146,197) — Gemini image generation is a separate concern; the non-exhaustive match there will need at minimum a fallthrough arm.
- No central adapter registry/factory; every dispatch match is manual. Rely on the compiler's non-exhaustive-match errors to find all sites after adding the enum variant.
