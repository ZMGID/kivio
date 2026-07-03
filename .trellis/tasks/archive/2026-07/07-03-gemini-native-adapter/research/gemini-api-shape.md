# Research: Gemini native API shape (generateContent / streamGenerateContent)

- **Query**: Gemini native request/response shapes, auth, tool round-trip mapping
- **Scope**: external (from model knowledge, cutoff Jan 2026) + internal cross-ref
- **Date**: 2026-07-03

> Source: Google "Gemini API" / Generative Language API (`generativelanguage.googleapis.com`) reference, plus the codebase note at `docs/chat-probe.md:49` confirming the OpenAI-compat shim rejects `promptCacheKey`. Verify exact field names against current Google docs before shipping (Gemini's API evolves; `thinkingConfig` in particular).

## Endpoint + URL construction

- Base (native): `https://generativelanguage.googleapis.com/v1beta` (also `v1`). The `ModelProvider.base_url` should be this root (NOT the `/openai/` OpenAI-compat path some users configure today).
- Non-stream: `POST {base}/models/{model}:generateContent`
- Stream (SSE): `POST {base}/models/{model}:streamGenerateContent?alt=sse`
- **Key structural difference vs OpenAI/Anthropic:** the model id and the method live in the URL path (`models/{model}:generateContent`), NOT in the body. The adapter's `url()` helper takes `request.model` + the stream flag (contrast anthropic.rs:528-530 where the URL is static and model is a body field). Trim a trailing `/` and also tolerate a base URL that already ends in `/v1beta` or `/models`.
- `alt=sse` is REQUIRED for line-delimited `data:` SSE; without it `streamGenerateContent` returns a JSON array stream that's harder to parse incrementally.

## Auth (OPEN QUESTION to decide in design)

Gemini native supports two equivalent auth conventions:
1. **Query param**: `?key={API_KEY}` appended to the URL.
2. **Header**: `x-goog-api-key: {API_KEY}`.

There is NO `Authorization: Bearer` (that's the OpenAI-compat shim only). **Recommendation for this adapter: use the `x-goog-api-key` header** — it keeps the key out of the URL, which matters because:
- `send_with_failover`'s closure receives `&key` and can set a header per attempt (mirrors anthropic.rs:77 `x-api-key`); putting the key in the URL means rebuilding the URL string per key.
- The request-debug panel logs the URL (request_debug.rs); a `?key=` URL would leak the key and need extra sanitizing, whereas headers already go through `sanitize_headers` (anthropic.rs:366). Header auth reuses the existing sanitization path cleanly.

Content-Type: `application/json`. For streaming, mirror anthropic's `Accept-Encoding: identity` (anthropic.rs:78) to avoid compressed SSE issues.

## Request body top-level fields

```jsonc
{
  "contents": [ /* conversation turns, roles "user" | "model" only */ ],
  "systemInstruction": { "parts": [ { "text": "<GenerateRequest.system>" } ] },
  "tools": [ { "functionDeclarations": [ { "name", "description", "parameters": <JSON Schema> } ] } ],
  "toolConfig": { "functionCallingConfig": { "mode": "AUTO" } },   // optional; omit → AUTO
  "generationConfig": {
    "temperature": <options.temperature>,
    "maxOutputTokens": <options.max_tokens>,
    "thinkingConfig": { "includeThoughts": true, "thinkingBudget": <n> }  // thinking models only
  },
  "safetySettings": [ /* optional; may want to set BLOCK_NONE to avoid silent truncation */ ]
}
```

Mapping from `GenerateRequest`:
- `request.system` (non-empty) → `systemInstruction.parts[].text`. **NOT** a `contents` entry — Gemini `contents` roles are only `user`/`model`.
- `request.messages` → `contents[]` (see round-trip below).
- `request.tools` → `tools[0].functionDeclarations[]` using `ModelTool::openai_tool_name()` (types.rs:95-104) as `name`, `description`, and `input_schema` as `parameters`. Likely needs a `normalize_gemini_schema` (Gemini accepts a restricted OpenAPI 3.0 subset — strip `$schema`, `additionalProperties`, unsupported `format` values, and handle `anyOf`/nullable like anthropic.rs:928-958).
- `options.max_tokens`/`temperature` → `generationConfig`.
- `options.thinking_level`/`thinking_enabled` → `generationConfig.thinkingConfig` (map effort→budget, or `includeThoughts`). DO NOT send `reasoning_effort`.
- `options.provider_options` object → merge verbatim into the body last (mirror anthropic.rs:344-348) — the escape hatch for `safetySettings`, `cachedContent`, etc.
- **DO NOT send** `promptCacheKey`/`prompt_cache_key` (the 400 cause), `tool_choice`, `stream_options`, session headers, or top-level `temperature` (goes in generationConfig).

### contents[] shape
```jsonc
// user turn
{ "role": "user", "parts": [ {"text": "..."}, {"inlineData": {"mimeType": "image/png", "data": "<base64>"}} ] }
// assistant turn (note role is "model")
{ "role": "model", "parts": [ {"text": "..."}, {"functionCall": {"name": "web_search", "args": {"query": "x"}}} ] }
// tool result turn (role "user"; some SDKs use "function")
{ "role": "user", "parts": [ {"functionResponse": {"name": "web_search", "response": {"result": "..."}}} ] }
```
Part mapping from `MessagePart` (types.rs:26-54):
- `Text{text}` → `{text}`.
- `Image{mime_type,data}` → `{inlineData:{mimeType,data}}`.
- `ImageUrl{url}` → `{fileData:{fileUri:url, mimeType:...}}` if a GCS/Files URI, else fetch+inline (data URLs must be split to inlineData). Simplest: only support inline; log unsupported remote URLs.
- `ToolCall{name,arguments}` (assistant) → `{functionCall:{name, args: arguments}}` (`args` is a JSON object, not a string).
- `ToolResult{tool_call_id, content}` (tool role) → `{functionResponse:{name: <resolved>, response: {...}}}` — see the id→name problem below.
- `Reasoning{text}` → `{text, thought:true}` on the model turn, OR drop on replay (Responses drops reasoning on replay, types.rs:855). Safer to drop unless round-tripping signed thoughts.

**Role merging required:** consecutive same-role `contents` should be merged (Gemini rejects adjacent same-role turns; Tool→"user" causes this). Reuse the pattern from anthropic.rs:895-926 (`merge_consecutive_anthropic_roles`).

## Response body (generateContent)

```jsonc
{
  "candidates": [ {
    "content": { "role": "model", "parts": [ {"text": "..."}, {"functionCall": {...}}, {"text":"...","thought":true} ] },
    "finishReason": "STOP",          // STOP | MAX_TOKENS | SAFETY | RECITATION | TOOL_CALLS(rare) | OTHER
    "index": 0
  } ],
  "usageMetadata": {
    "promptTokenCount": 12,
    "candidatesTokenCount": 34,
    "totalTokenCount": 46,
    "cachedContentTokenCount": 0,
    "thoughtsTokenCount": 8
  },
  "promptFeedback": { "blockReason": "..." }   // present when input blocked
}
```
Parse → `GenerateOutput`:
- Walk `candidates[0].content.parts[]`: `text` (no `thought`) → append to `text`; `text` with `thought:true` → append to `reasoning`; `functionCall` → a `PendingToolCall` (`id` = synthesize, e.g. `format!("call_{}", uuid)` since Gemini gives no call id; `function_name` = `name`; `arguments` = `args` object; `arguments_raw` = serialized `args`).
- `finishReason` → canonical via a mapper: `STOP→"stop"`, `MAX_TOKENS→"length"`, functionCall-present→`"tool_calls"` (Gemini often returns `STOP` even with a functionCall, so derive `"tool_calls"` when any functionCall part exists — mirror how tool presence matters). `SAFETY`/`RECITATION`/`OTHER→"stop"` (or surface as error).
- `usageMetadata` → `ModelUsage` (see adapter-contract.md ModelUsage section).
- `provider_messages`: build the OpenAI-compatible assistant message (reuse the `openai_compatible_message` pattern, anthropic.rs:964-998).
- Check `promptFeedback.blockReason` / empty `candidates` → return `ModelError`.

## Response body (streamGenerateContent?alt=sse)

Each SSE event is `data: {GenerateContentResponse}` — a full (partial) response object with incremental `candidates[0].content.parts[]`. There are no fine-grained "block delta" events like Anthropic; each chunk carries whole parts. Parsing loop (mirror anthropic.rs:166-311 buffering):
- For each `data:` line → parse JSON → for each part: text→`TextDelta`, thought-text→`ReasoningDelta`, functionCall→emit `ToolCallStart{id,name}` + `ToolCallDelta{id, delta: serialized_args}` + `ToolCallDone{call}` (args arrive whole, no streaming fragments).
- Accumulate `full`, `reasoning`, `tool_calls`, and capture `finishReason` + `usageMetadata` (arrive on the final chunk).
- On stream end emit exactly one `Finish{reason, full}` and return `stream_output(...)` (mirror anthropic.rs:1000-1023).
- On chunk read error use `stream_read_error(&label, &err)` (types.rs:377).

## Tool-call round trip (the critical mapping)

1. Model emits (response): `candidates[].content.parts[].functionCall{name, args}` → adapter makes a `PendingToolCall{ id: synthesized, function_name: name, arguments: args, arguments_raw }`.
2. Runtime executes the tool, produces a `ModelMessage{role: Tool, content: [ToolResult{tool_call_id, content, ...}]}`.
3. Adapter must send it back as `{functionResponse:{ name: <tool name>, response: {...} }}` inside a `role:"user"` content.

**PROBLEM / OPEN QUESTION:** Gemini keys `functionResponse` by function **name**, but the canonical `ToolResult` carries only `tool_call_id`, not the name. And Gemini gives no call id in `functionCall`, so the id was synthesized by the adapter in step 1. To recover the name at replay, the adapter must, while building `contents`, remember the `id→name` map from the preceding assistant `functionCall` parts in the SAME message list, and look it up when serializing the matching `ToolResult`. (Anthropic sidesteps this because it uses `tool_use_id`/`tool_result.tool_use_id` directly, anthropic.rs:855-874 — Gemini can't.) A robust approach: in `gemini_contents_from_generate_request`, first pass to collect `{tool_call_id → function_name}` from all assistant `ToolCall` parts, then map each `ToolResult` via that table; fall back to the raw id string if unknown. Also `functionResponse.response` must be a JSON object — wrap plain-string tool output as `{"result": "<content>"}` (or `{"output": ...}`).

## Caveats / Not Found
- Exact `thinkingConfig` schema (field names `thinkingBudget` vs effort enum, `includeThoughts`) and whether thoughts are returned as `thought:true` parts vs a separate channel have changed across Gemini versions — verify against live docs / probe before finalizing the thinking mapping.
- Whether the tool-result role should be `"user"` or `"function"`: the REST API historically accepts `functionResponse` inside `role:"user"`; some SDK layers use `"function"`. Confirm with a probe round-trip.
- `finishReason` for tool calls: Gemini commonly returns `STOP` with a functionCall part rather than a dedicated `TOOL_CALLS` reason — derive `"tool_calls"` from part presence, don't rely on the reason string.
