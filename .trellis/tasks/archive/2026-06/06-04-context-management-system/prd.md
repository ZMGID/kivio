# Context Management System

## Goal

Add a Chat context management system that lets users see how much of the current model context is being used, and automatically compresses older conversation history when the conversation gets too large. The feature should make long chats feel safer and more transparent without deleting visible history.

## What I Already Know

- User wants a small circular indicator that shows current context usage ratio.
- User wants automatic context compression.
- User provided a reference UI: a compact footer/header percentage ring that opens a "Context" panel with a segmented token bar and per-source token breakdown.
- Chat context is assembled in Rust, not purely in React, so backend-side measurement is more reliable.
- Existing Chat supports multi-provider calls, Anthropic adaptation, Skill/MCP/native tools, image attachments, streaming, message edit/delete, and regeneration.
- Settings already include a `compression` default model slot through `default_models.compression`.
- The current provider/model config does not store per-model context window sizes.

## Research References

- [`research/context-management-architecture.md`](research/context-management-architecture.md) - local architecture findings, API notes, data-flow recommendation, and MVP proposal.

## Requirements

### Context Usage Indicator

- Show a small circular context indicator at the conversation level.
- Preferred placement:
  - Desktop Chat header near the model selector, or the composer/status footer if the chat layout keeps model/status metadata there.
  - The indicator should remain visible while typing and while reading long conversations.
- The ring should display a compact percent label next to it, matching the reference pattern like `15%`.
- Clicking the ring opens a lightweight Context panel.
- The indicator should show status by color:
  - Normal below warning threshold.
  - Warning around 70%.
  - Critical around 85-95%.
  - Compressed/stale states when applicable.
- Hover or click should show a Context panel with:
  - Title `Context`.
  - Overall fullness label like `15% Full`.
  - Token total like `~30.5K / 200K Tokens`.
  - A horizontal segmented usage bar, where each segment is colored by source.
  - Per-source rows with color swatch, label, and token count.
  - Estimated input context tokens.
  - Estimated context window or unknown denominator.
  - Usage percentage when denominator is known or estimated.
  - Number of raw visible messages/turns included.
  - Summary/compression status.
  - Last compression time, if any.
- The display must be clearly treated as an estimate when provider/model limits are unknown.

### Context Breakdown Sources

The Context panel should break usage into explainable buckets instead of showing one opaque number.

Recommended MVP buckets:

- System prompt: base assistant prompt and custom Chat prompt.
- Runtime context: date/time, language behavior, no-think instruction, and other dynamic rules injected by Kivio.
- Tool definitions: OpenAI-compatible tool schemas sent to the model.
- Skills: Skill catalog and active Skill body when injected.
- MCP: MCP server/tool metadata that is visible to the model.
- Native tools: Kivio built-in tool instructions and schemas, if separate from MCP/tool definitions.
- Summarized conversation: compressed memory currently injected into the prompt.
- Conversation: raw recent user/assistant messages and hidden assistant/tool API transcript replay.
- Attachments: image/file prompt placeholders, when applicable.

If two buckets are technically merged in the request, the backend can still attribute estimated tokens by measuring the text before merging.

### Context Measurement

- Compute context usage in the Rust Chat layer from the actual request payload shape, including:
  - System prompt and current date/time context.
  - Visible user/assistant messages.
  - Hidden assistant `api_messages` used for tool-call replay.
  - Active Skill/catalog prompt injection.
  - Enabled tool schemas.
  - Latest user image attachment placeholders.
- Reuse or port the existing lightweight token estimate strategy for MVP.
- Prefer provider usage fields after requests when available, but do not depend on them.
- Support a future path for model-specific context windows.

### Compression Behavior

- Use the configured default compression model via `Settings::effective_compression_model()`.
- If the compression model is unset, inherit the effective Chat model.
- Preserve raw visible conversation messages in local storage.
- Compress only older complete turns into a summary used for future model requests.
- Keep the latest several turns raw so recent interaction remains exact.
- Do not compress through the middle of a tool-call/API transcript sequence.
- Add a manual "Compress now" action in the context popover.
- Auto-compress before send when estimated usage crosses the configured threshold.
- If compression fails, continue safely:
  - If the original request can still fit, send it and show a warning.
  - If likely over limit, block with a clear error and suggest manual compression/model change.

### Summary Consistency

- Store summary metadata in the conversation file.
- Include summary source message ids and a boundary message id.
- If a message at or before the summary boundary is edited, deleted, or regenerated, mark the summary stale and ignore it until recompressed.
- If changes occur after the boundary, keep the summary and recompute stats.
- Never let hidden summary content contradict visible edited history.

### Tauri / Frontend Contract

- Add a typed context state shape shared between Rust and TypeScript.
- Include last known context state in `Conversation` for persistence.
- Add a command such as `chat_get_context_stats(conversation_id)` to recompute fresh stats.
- Optionally emit `chat-context` events during send/compress to update the circular indicator without waiting for full conversation reload.
- The context state should include `segments` so the frontend can render the segmented bar without re-estimating token sources.

Suggested segment shape:

```ts
type ContextUsageSegment = {
  id: 'system_prompt' | 'runtime_context' | 'tool_definitions' | 'skills' | 'mcp' | 'native_tools' | 'summarized_conversation' | 'conversation' | 'attachments'
  label: string
  estimatedTokens: number
  color?: string
}
```

## Acceptance Criteria

- [ ] Chat header shows a compact circular context indicator for the current conversation.
- [ ] Indicator details explain estimated tokens, percent, compression state, and per-source token categories.
- [ ] Context panel includes a segmented usage bar and rows similar to the provided reference image.
- [ ] Context stats are computed from backend request-building inputs, not only visible frontend messages.
- [ ] Long conversations can be manually compressed.
- [ ] Auto-compression runs before send when threshold is crossed.
- [ ] Raw visible messages remain available after compression.
- [ ] Future requests include the summary plus recent raw turns instead of the entire old transcript.
- [ ] Editing/deleting/regenerating messages invalidates stale summaries when needed.
- [ ] Compression model selection uses the existing `default_models.compression` fallback chain.
- [ ] TypeScript typecheck, ESLint, and Rust tests cover the new data contract and summary invalidation behavior.

## Out of Scope

- Deleting or hiding old visible messages as part of compression.
- Exact tokenizer parity for every provider.
- Full per-provider context-window database.
- Background idle-time compression daemon.
- Lens screenshot Q&A context compression.
- SQLite migration for conversations.

## Technical Notes

- Likely Rust files:
  - `src-tauri/src/chat/types.rs`
  - `src-tauri/src/chat/commands.rs`
  - `src-tauri/src/chat/storage.rs`
  - `src-tauri/src/settings.rs`
  - `src-tauri/src/main.rs`
- Likely frontend files:
  - `src/chat/Chat.tsx`
  - `src/chat/types.ts`
  - `src/chat/api.ts`
  - `src/api/tauri.ts`
  - New small component such as `src/chat/ContextIndicator.tsx`
- Relevant existing utilities:
  - `src/lens/markdown.ts` has `estimateTokens` / `formatTokens`.
  - `Settings::effective_compression_model()` already exists.
- Cross-layer flow:
  - Settings/model/tools -> Rust request builder -> context stats -> conversation JSON -> Tauri command/event -> React indicator.

## Recommended MVP

Build this in two slices:

1. **Visibility first**: backend estimate + circular indicator + tooltip/popover + persisted context state.
2. **Compression second**: summary data model + manual compress + auto-compress before send + summary invalidation.

This reduces risk because the indicator can be validated against real conversations before the app starts changing request history.

## Open Question

Should auto-compression be enabled by default once implemented, or should the first version ship with manual compression plus a warning threshold and let users enable auto-compression in settings?
