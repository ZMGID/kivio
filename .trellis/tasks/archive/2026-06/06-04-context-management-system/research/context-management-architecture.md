# Context Management Architecture Research

## Goal

Explore how Kivio Chat should show context usage and automatically compress long conversations without breaking the existing Chat stack: OpenAI-compatible providers, Anthropic adapters, tools, Skills, image attachments, streaming, and local JSON conversation storage.

## Current Code Findings

- Chat conversations are persisted as one JSON file per conversation via `src-tauri/src/chat/storage.rs`.
- `Conversation` currently stores only visible `messages` plus assistant `api_messages` for tool-call transcript replay.
- Final provider request context is assembled in `src-tauri/src/chat/commands.rs` by `build_chat_system_prompt` and `build_chat_api_messages`.
- The frontend can only see visible messages. It does not see the full system prompt, Skill catalog/body injection, provider tool schemas, or hidden assistant/tool `api_messages`.
- Settings already include `default_models.compression`, and `Settings::effective_compression_model()` exists. This is the natural model selection point for summary/compression calls.
- The UI already has a rough token estimator in `src/lens/markdown.ts`, used by Lens thinking blocks and Chat assistant metadata.

## External API Notes

- OpenAI Chat Completions responses expose usage metadata in the response object; streaming can expose usage depending on request/provider support. Official reference: https://platform.openai.com/docs/api-reference/chat
- Anthropic Messages responses expose usage fields and Anthropic also documents a token counting endpoint. Official references: https://docs.anthropic.com/en/api/messages and https://docs.anthropic.com/en/api/messages-count-tokens
- Kivio supports arbitrary OpenAI-compatible providers and gateways, so exact usage fields and model context windows cannot be assumed to exist for every provider.

Implication: the product should display an immediate local estimate and optionally refine it with provider usage after a request. Provider usage should improve accuracy but must not be required for core behavior.

## Why Backend-First Measurement

The context meter's denominator and numerator should be computed near request construction, not from visible UI messages only.

Visible UI messages miss:

- System prompt plus local date/time context.
- Skill catalog and active Skill body.
- Enabled tool schemas.
- Hidden `api_messages` stored on assistant messages for tool-call replay.
- Image attachment prompt parts for the latest user message.
- Tool results appended during multi-round tool planning.

Recommended contract:

```text
Conversation JSON -> Rust request builder -> ContextStats -> Tauri event/command -> React indicator
```

## Reference UI Takeaways

The user-provided reference shows a better target than a simple percent badge:

- A persistent small ring plus percent in the chat chrome.
- A Context panel opened from that ring.
- A top-level fullness summary such as `15% Full`.
- A token total such as `~30.5K / 200K Tokens`.
- A horizontal segmented bar where each source owns a colored slice.
- A readable legend with token counts per source.

This is a strong fit for Kivio because our context has multiple invisible contributors. Users should be able to tell whether growth is caused by raw conversation, Skills, MCP/tool definitions, or summarized history.

## Proposed Data Model

Add lightweight metadata to `Conversation`:

```rust
pub struct ContextUsageSegment {
    pub id: String,
    pub label: String,
    pub estimated_tokens: usize,
}

pub struct ConversationContextState {
    pub estimated_input_tokens: usize,
    pub context_window_tokens: Option<usize>,
    pub usage_ratio: Option<f32>,
    pub status: String, // normal | warning | critical | compressed | unknown
    pub segments: Vec<ContextUsageSegment>,
    pub last_measured_at: i64,
    pub last_compressed_at: Option<i64>,
    pub compressed_message_count: usize,
    pub summary: Option<ConversationContextSummary>,
}

pub struct ConversationContextSummary {
    pub id: String,
    pub content: String,
    pub source_message_ids: Vec<String>,
    pub source_until_message_id: String,
    pub token_estimate_before: usize,
    pub token_estimate_after: usize,
    pub created_at: i64,
    pub provider_id: String,
    pub model: String,
}
```

Keep raw messages in the conversation file for user trust and future re-summarization. Compression should affect what goes into the next provider request, not delete user-visible history.

Recommended segment ids for Kivio:

- `system_prompt`: base/custom Chat prompt.
- `runtime_context`: date/time, language, no-think/rules, and Kivio dynamic behavior instructions.
- `tool_definitions`: serialized tool schemas.
- `skills`: Skill catalog and active Skill body.
- `mcp`: MCP-related tool/server metadata when distinguishable.
- `native_tools`: Kivio built-in tool instructions/schemas when distinguishable.
- `summarized_conversation`: injected compressed memory.
- `conversation`: raw visible messages plus assistant/tool `api_messages` replay.
- `attachments`: image/file prompt placeholders.

The backend should measure each contributor before merging it into the provider request. This lets the frontend render a segmented bar without reconstructing prompt internals.

## Request-Building Strategy

When no summary exists:

```text
system + full visible/API transcript + current user message
```

When summary exists:

```text
system
+ synthetic summary message
+ messages after source_until_message_id
+ current user message
```

The summary should be injected as a system or assistant-style context message with a stable label such as:

```text
Previous conversation summary:
...
```

For strict tool-call providers, do not summarize through the middle of an unresolved assistant `tool_calls` plus matching `tool` result sequence. Compress only complete user/assistant turns, and keep the most recent N complete turns raw.

## Token Estimation Strategy

MVP should avoid adding a heavy tokenizer dependency.

Use a shared Rust estimator equivalent to the existing frontend heuristic:

- ASCII: roughly 4 characters per token.
- CJK/non-ASCII: roughly 1 character per token.
- JSON/tool schemas: estimate serialized JSON text length.
- Images: count a fixed placeholder budget per image attachment until provider-specific image token accounting exists.

Future improvement:

- Add optional provider-specific token counting for Anthropic.
- Use OpenAI-compatible `usage.prompt_tokens` / `usage.total_tokens` when present.
- Add per-model context window presets or a user-editable context window field.

## Context Window Denominator

Current `ModelProvider` does not store context length. Without that denominator, usage ratio can only be an estimate against a default.

Recommended MVP:

- Add optional `contextWindowTokens` to provider/model settings later if needed.
- Ship built-in heuristic presets for common model names only as a best-effort fallback.
- If unknown, use a conservative default such as 128k for modern chat models, but show the UI as estimated rather than exact.

Better product behavior:

- The circular indicator can show exact-looking percent only when a denominator is known.
- When unknown, show `~42k` or `~42k / ?` in tooltip, with color thresholds based on conservative default.

## Auto-Compression Trigger

Recommended defaults:

- Warning: 70% of estimated context window.
- Auto-compress: 85%.
- Critical: 95%, where the next send can force compression before calling the chat model.
- Always keep the latest 6-10 visible turns raw.

Compression should run before the next model call if the estimated request would cross the trigger. It should not run continuously in the background while the user is idle for MVP.

## Compression Prompt Shape

Compression model input should include older complete turns and ask for a dense, factual conversation memory:

- User goals and preferences.
- Decisions made.
- Facts, constraints, file paths, commands, tool results that remain relevant.
- Open tasks and unresolved questions.
- Important artifacts/attachments by name, not raw image data.
- Avoid style narration and avoid inventing facts.

Output should be concise Markdown, ideally bounded by target tokens or characters.

## Editing, Deleting, and Regeneration

Existing behavior:

- Assistant messages can be edited.
- Assistant messages can be deleted.
- Regeneration truncates the conversation at the assistant message being regenerated.

Compression consistency rule:

- If a user edits/deletes/regenerates any message at or before `summary.source_until_message_id`, mark the summary stale and ignore it for future requests until recompressed.
- If edits happen after the summary boundary, keep the summary and recompute usage.
- Regeneration after the boundary should preserve summary only if the boundary remains in the retained prefix.

This avoids a hidden summary contradicting the visible transcript.

## UI Placement

Best first placement:

- Small circular indicator in the Chat header near `ModelSelector`, or in the composer/status footer if the active Chat layout keeps model/status metadata there.
- Tooltip/popover on hover/click:
  - Title: `Context`.
  - Overall fullness and estimated token total.
  - Segmented source bar.
  - Source legend rows with token counts.
  - Raw tokens/messages kept.
  - Summary status.
  - Last compression time.
  - Manual "Compress now" action.

Do not put this inside message cards. It is conversation-level state.

## Tauri Contract Options

Option A: include `context_state` in `Conversation`.

- Pros: one read returns all visible status; persists across restarts.
- Cons: may be stale until recomputed.

Option B: separate command `chat_get_context_stats(conversation_id)`.

- Pros: can compute fresh from current settings/model/tools.
- Cons: extra frontend call and loading state.

Recommended MVP: combine both.

- Persist last known state in conversation JSON.
- Expose `chat_get_context_stats` for fresh recomputation when opening a conversation or changing model/settings.
- Emit `chat-context` events during send/compress so the indicator updates without waiting for full conversation reload.

## Risks

- Underestimating tool schemas or image tokens can still hit provider context limits.
- Summaries can omit details users expected the model to remember.
- Provider context window is often unknown for custom OpenAI-compatible gateways.
- Compression calls add latency and cost.
- Tool-call transcript replay is fragile if compressed in the middle of an API message sequence.

## Recommended MVP

1. Backend computes context estimate from the actual request messages.
2. Header shows a circular context meter with tooltip.
3. Add a conversation-level summary object and request builder support for injecting it.
4. Add manual "Compress now".
5. Add auto-compress before send when usage crosses threshold.
6. Keep raw messages visible and on disk.
7. Mark summaries stale on edits/deletes/regeneration across the summary boundary.
8. Use `default_models.compression` for compression calls, with chat model fallback.
