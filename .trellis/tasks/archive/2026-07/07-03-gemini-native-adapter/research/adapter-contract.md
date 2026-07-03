# Research: Adapter Contract (the binding interface gemini.rs must implement)

- **Query**: What trait + types must `gemini.rs` consume/produce to be a peer adapter?
- **Scope**: internal
- **Date**: 2026-07-03

## Findings

### Files Found

| File Path | Description |
|---|---|
| `src-tauri/src/chat/model/README.md` | The contract doc (runtime owns orchestration; adapters own wire JSON) |
| `src-tauri/src/chat/model/mod.rs` | Module: registers `anthropic` / `openai` / `responses` / `types`; re-exports the provider structs |
| `src-tauri/src/chat/model/types.rs` | All shared types: trait, `GenerateRequest`, `GenerateOutput`, `StreamPart`, `ModelUsage`, `ModelMessage`/`MessagePart`, helpers |

### The contract in my own words (README.md:1-18)

The Chat **runtime** owns orchestration: conversation state, prompt construction, the tool loop, cancellation, persistence, Tauri events. **Provider adapters** own provider JSON and wire protocols. Runtime code passes a `GenerateRequest` to a `LanguageModelProvider` and consumes `GenerateOutput` + `StreamPart` events. OpenAI-compatible, Anthropic Messages (and OpenAI Responses) are **peer** providers; Gemini becomes a fourth peer.

Rules (README.md:10-18):
- Runtime/tool-loop code must NOT inspect OpenAI `choices`, Anthropic `content` blocks, SSE event names, or provider headers. All that lives inside the adapter.
- Adapters may use `serde_json::Value` freely at the wire boundary.
- Tauri event payloads (`chat-stream`, `chat-tool`, `chat-context`) are UI contracts, not provider contracts — stay stable.

### The trait: `LanguageModelProvider` (types.rs:430-438)

```rust
pub type ModelFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ModelError>> + Send + 'a>>;

pub trait LanguageModelProvider {
    fn generate<'a>(&'a self, request: GenerateRequest) -> ModelFuture<'a, GenerateOutput>;
    fn stream<'a>(
        &'a self,
        request: GenerateRequest,
        sink: &'a mut (dyn StreamSink + Send),
    ) -> ModelFuture<'a, GenerateOutput>;
    fn capabilities(&self) -> ProviderCapabilities;
}
```

- `StreamSink` (types.rs:415-426) is a trait with `emit(&mut self, part: StreamPart) -> Result<(), ModelError>`; any `FnMut(StreamPart) -> Result<(), ModelError>` is a `StreamSink`.
- `ProviderCapabilities { tool_calling, vision, streaming, reasoning }` (types.rs:144-150). Anthropic returns `tool_calling: provider.supports_tools`, the other three `true` (anthropic.rs:49-56).
- `ModelError` (types.rs:351-401) has a `kind` (`Other` | `StreamReadInterrupted`). Use the helper `stream_read_error(label, &reqwest::Error)` (types.rs:377-393) when a stream chunk read fails so the loop can classify retryable interruptions.

### `GenerateRequest` — everything the runtime passes (types.rs:208-216)

```rust
pub struct GenerateRequest {
    pub model: String,
    pub system: String,                 // system prompt as a single string (may be empty)
    pub messages: Vec<ModelMessage>,    // canonical conversation (NO system message inside)
    pub tools: Vec<ModelTool>,
    pub options: GenerateOptions,
    pub metadata: RequestMetadata,
}
```

`GenerateOptions` (types.rs:152-178):
```rust
pub struct GenerateOptions {
    pub temperature: f32,               // default 0.7
    pub max_tokens: u32,                // default 8192
    pub stream: bool,
    pub thinking_enabled: bool,         // default true
    pub thinking_level: Option<String>, // "low"|"medium"|"high"; None = don't emit any effort field
    pub provider_options: Value,        // free-form overrides merged into the body (see note)
}
```
- **NOTE — there is NO explicit `tool_choice`, cache-key, session-header, or stream-flag field on the request.** `stream` lives on `GenerateOptions.stream` but the adapters actually take a `stream: bool` param into `request_body(...)`. Cache keys / session headers are **derived by the OpenAI adapter from `metadata.conversation_id`**, not passed as request fields (see anthropic-template.md). Gemini should NOT reproduce those.
- `provider_options` is a JSON object each adapter merges verbatim into its final body as a last step (openai.rs:447-451, anthropic.rs:344-348). This is the generic escape hatch for provider-specific knobs.

`RequestMetadata` (types.rs:180-191):
```rust
pub struct RequestMetadata {
    pub label: String,                     // used for usage/debug labelling + fallback name
    pub usage_source: Option<String>,
    pub usage_operation: Option<String>,
    pub conversation_id: Option<String>,   // OpenAI adapter turns this into cache key + session hdrs
    pub message_id: Option<String>,
}
```

### `GenerateOutput` — what the adapter returns (types.rs:240-310)

```rust
pub struct GenerateOutput {
    pub text: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<PendingToolCall>,
    pub usage: Option<ModelUsage>,
    pub finish_reason: Option<String>,      // canonical: "stop" | "tool_calls" | "length" | "cancelled"
    pub provider_messages: Vec<Value>,      // the assistant turn re-serialized as OpenAI-compatible message(s)
    pub cancelled: bool,
}
```
- `provider_messages` MUST hold an OpenAI-compatible assistant message (role/content/tool_calls/reasoning_content). Both anthropic.rs (`openai_compatible_message`, anthropic.rs:964-998) and the fallback `to_openai_compatible_message` (types.rs:276-309) build exactly this shape. The runtime replays history through these, so **Gemini must emit an OpenAI-shaped assistant message here even though it talks Gemini on the wire.**
- `finish_reason` is a canonical string. Anthropic maps its `stop_reason` → these via `finish_reason_from_anthropic_stop_reason` (anthropic.rs:1025-1033: `end_turn→stop`, `tool_use→tool_calls`, `max_tokens→length`). Gemini's `finishReason` (`STOP`/`MAX_TOKENS`/etc.) needs an equivalent mapper.

`PendingToolCall` (types.rs:231-238):
```rust
pub struct PendingToolCall {
    pub id: String,
    pub function_name: String,
    pub arguments: Value,           // parsed JSON
    pub arguments_raw: String,      // raw JSON string (source of truth for replay)
    pub arguments_parse_error: Option<String>,
}
```
- Use `parse_tool_arguments(raw) -> (Value, Option<String>)` (types.rs:440-455) to fill `arguments`/`arguments_parse_error`. For Gemini the `functionCall.args` arrives as an already-parsed JSON **object** (not a string) — serialize it to `arguments_raw` and set `arguments` directly (mirror Anthropic's `tool_use.input` handling, anthropic.rs:651-667).

### `StreamPart` — all stream event variants (types.rs:312-343)

```rust
pub enum StreamPart {
    TextDelta { delta: String },
    ReasoningDelta { delta: String },
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, delta: String },   // partial JSON of arguments
    ToolCallDone { call: PendingToolCall },
    ToolResult { tool_call_id: String, content: String },  // (not emitted by model adapters)
    Finish { reason: String, full: String },        // full = full accumulated text
    Error { message: String },
}
```
Anthropic's streaming loop shows the expected emission order (anthropic.rs:191-311): `TextDelta`/`ReasoningDelta` interleaved, then per tool: `ToolCallStart` → `ToolCallDelta`* → `ToolCallDone`, and finally exactly one `Finish { reason, full }`. Gemini `streamGenerateContent` sends whole `parts` per chunk (no incremental arg deltas), so Gemini can emit `ToolCallStart` + a single `ToolCallDelta` (full args) + `ToolCallDone` per functionCall, or just `ToolCallStart`+`ToolCallDone` — the loop only requires `ToolCallDone` to carry the assembled `PendingToolCall`.

### `ModelUsage` shape (types.rs:218-229)

```rust
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}
```
Gemini's `usageMetadata` fields → this map: `promptTokenCount→input_tokens`, `candidatesTokenCount→output_tokens`, `totalTokenCount→total_tokens`, `cachedContentTokenCount→cached_input_tokens`, `thoughtsTokenCount→reasoning_tokens`. (Compare the Anthropic mapper `model_usage_from_anthropic_value`, `src-tauri/src/usage.rs:704-720`, which computes `total = input+output` when absent — Gemini gives `totalTokenCount` directly.)

### Message representation — `ModelMessage` / `MessagePart` (types.rs:26-80)

```rust
pub enum ModelRole { User, Assistant, Tool }        // types.rs:8-24 (as_str: user/assistant/tool)

pub enum MessagePart {                              // types.rs:26-54, tag="type" snake_case
    Text { text },
    Image { mime_type, data },                      // data = base64
    ImageUrl { url },
    ToolCall { id, name, arguments: Value, arguments_raw: String },
    ToolResult { tool_call_id, content: String, is_error, artifacts: Vec<ChatToolArtifact> },
    Reasoning { text },
}

pub struct ModelMessage { pub role: ModelRole, pub content: Vec<MessagePart> }
```

Gemini mapping the adapter must implement (see gemini-api-shape.md for wire detail):
- `ModelRole::User` → Gemini `contents[]` with `role:"user"`; `Text→{text}`, `Image→{inlineData:{mimeType,data}}`, `ImageUrl→{fileData}` or inline.
- `ModelRole::Assistant` → `role:"model"`; `Text→{text}`, `ToolCall→{functionCall:{name,args}}`, `Reasoning→{text, thought:true}` (or drop on replay like Responses does).
- `ModelRole::Tool` (a `ToolResult`) → a `role:"user"` (or `"function"`) content with a `{functionResponse:{name, response}}` part. **Gemini keys the response by function NAME, not by call id** — the adapter must recover the name for a given `tool_call_id` (see open question).
- System prompt: `GenerateRequest.system` → top-level `systemInstruction`, NOT a message (Gemini `contents[]` roles are only `user`/`model`).

`ModelTool` (types.rs:82-127): fields `id, name, description, source, input_schema: Value, sensitive`. `openai_tool_name()` (types.rs:95-104) is the sanitized wire name to use for tool declarations (Anthropic reuses it, anthropic.rs:558). Gemini `functionDeclarations[].name` should use the same `openai_tool_name()` so tool-name round-tripping matches the loop's expectations. `input_schema` is a JSON Schema object — Gemini accepts a (restricted) OpenAPI-subset schema; Anthropic passes it through with a small `normalize_anthropic_schema` fixup (anthropic.rs:928-958) that Gemini may also need (e.g. stripping unsupported fields / handling nullable `anyOf`).

### Helpers reusable by gemini.rs (types.rs)
- `parse_tool_arguments` (types.rs:440-455), `tool_arguments_to_raw` (types.rs:463-469).
- The `openai_*_from_*` conversion helpers are OpenAI-shaped; Gemini writes its own `gemini_contents_from_generate_request` (mirror `anthropic_messages_from_generate_request`, anthropic.rs:532-552) but must still produce an **OpenAI-compatible** `provider_messages` entry for `GenerateOutput` (reuse the `openai_compatible_message` pattern from anthropic.rs:964-998).

## Caveats / Not Found
- No `tool_choice` in the contract — the OpenAI adapter hardcodes `"auto"` locally (openai.rs:417). Gemini's equivalent is `toolConfig.functionCallingConfig.mode` (default AUTO); the adapter can omit it.
- `GenerateOptions.stream` exists but adapters are actually driven by which method (`generate` vs `stream`) the dispatcher calls; `request_body(request, stream: bool)` takes the flag explicitly.
