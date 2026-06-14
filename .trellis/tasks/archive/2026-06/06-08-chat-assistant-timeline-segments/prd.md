# Chat Assistant Timeline Segments

## Goal

Replace the current assistant-message "three bucket" display model with a durable, ordered segment timeline so Chat can preserve and replay process-style agent narration: text, reasoning, and tool activity interleaved in the order they happened.

The main user-facing result is that a generated assistant message should read like a coherent process transcript when appropriate: planning narration, compact tool blocks, intermediate observations, and final synthesis should remain in their original order during streaming and after reload. The implementation should build a long-term foundation, not a short-term preview workaround.

## Problem

Current Chat assistant messages are stored and rendered as three flat fields:

- `content`: one final assistant body
- `reasoning`: one contiguous reasoning block
- `tool_calls`: one unordered-looking list rendered above the answer

This forces the UI into a fixed layout: all tools first, then reasoning, then answer. It cannot represent an agent-style timeline such as:

1. assistant explains what it is checking
2. tool runs
3. assistant narrates what it learned
4. another tool runs
5. final synthesis

Streaming temporarily shows process text through an in-memory snapshot, but persistence only stores the final assistant `content` plus the flat metadata fields. When streaming finishes, the frontend clears the preview and reloads the persisted conversation, which can make intermediate narration appear to be replaced or disappear.

## What I Already Know

- The current backend `ChatMessage` model lives in `src-tauri/src/chat/types.rs` and has flat `content`, `reasoning`, `tool_calls`, `api_messages`, and `model_messages` fields.
- The frontend `ChatMessage` type lives in `src/chat/types.ts` and mirrors the same flat shape.
- `MessageBubble` currently renders all tool calls before reasoning and final answer.
- Streaming state in `Chat.tsx` accumulates `snapshot.content`, `snapshot.reasoning`, and `snapshot.toolCalls`, then clears the snapshot on completion and reloads the conversation.
- The agent runtime already records per-step metadata through `AgentStepResult`, including `step_number`, `phase`, `tool_records`, `streamed`, and `stop_reason`.
- Tool records already carry `round`, lifecycle status, artifacts, and trace metadata, so timeline tool segments can reference tool records instead of duplicating them.
- Chat also creates auxiliary tool records before the main agent run, such as Mixer auxiliary vision analysis for image inputs. These side-task records currently join final `tool_calls`, so timeline rendering must place them instead of leaving them in a detached bucket.
- Backend guidelines require preserving provider replay order and deterministic tool result ordering.

## Product Principles

- The persisted assistant message must be the source of truth for the timeline. Streaming preview and reloaded conversation should render the same structure.
- Final assistant answer content should remain easy to copy, edit, regenerate, and send back into context.
- Tool activity should be visible but compact. Timeline rendering should improve process readability without turning every message into noisy logs.
- Planning/tool-loop narration must not share the same `content` channel as final synthesis.
- Legacy conversations must continue to render correctly.

## Requirements

### 1. Canonical Assistant Segments

Add a durable ordered segment structure to assistant messages.

Recommended shape:

```ts
type ChatMessageSegment =
  | {
      id: string
      kind: 'text'
      phase: 'auxiliary' | 'plain' | 'tool_loop' | 'synthesis'
      order: number
      step_number?: number
      round?: number
      text: string
    }
  | {
      id: string
      kind: 'reasoning'
      phase: 'auxiliary' | 'plain' | 'tool_loop' | 'synthesis'
      order: number
      step_number?: number
      round?: number
      text: string
    }
  | {
      id: string
      kind: 'tool'
      phase: 'auxiliary' | 'tool_loop'
      order: number
      step_number?: number
      round?: number
      tool_call_id: string
    }
```

Backend Rust naming should follow project serde conventions, with frontend aliases only where needed for compatibility.

### 2. Segment Ownership and Compatibility

- Add `segments: Vec<ChatMessageSegment>` to `ChatMessage` with `#[serde(default)]`.
- Keep existing `content`, `reasoning`, and `tool_calls` fields.
- Treat `segments` as the primary display structure when present.
- Treat `content` as the canonical final answer text for copy/edit/context/replay compatibility.
- Treat `reasoning` as the legacy aggregate reasoning field.
- Treat `tool_calls` as the canonical tool record list; tool segments reference records by `tool_call_id`.
- Persisted messages must double-write legacy fields from segments:
  - `content` is all non-empty `kind = 'text'` segments where `phase = 'plain' | 'synthesis'`, sorted by `order` and joined with `\n\n`.
  - `reasoning` is all non-empty `kind = 'reasoning'` segments, sorted by `order` and joined with `\n\n`; store `None`/omit when the result is empty.
  - `tool_calls` is the full `ToolCallRecord` list, in the deterministic order already required by the agent runtime. Timeline rendering order comes from tool segments; record-list order remains the provider/model order contract.
- For old conversations without segments, frontend should synthesize the old display from flat fields.

### 3. Backend Segment Production

The backend agent runtime must produce ordered segments as the run progresses and persist them with the final assistant message.

- Each agent step should contribute segments in actual display order.
- `AgentStepResult` may be extended to carry step-level segments, or the run loop may maintain a per-run segment accumulator that is also copied into step results.
- Tool-loop model text must be stored as `kind = 'text', phase = 'tool_loop'` or as `kind = 'reasoning'`, depending on whether it is safe user-visible narration or provider reasoning.
- Synthesis output must be stored as `kind = 'text', phase = 'synthesis'` and also populate final `content`.
- Plain no-tool responses should be represented as `kind = 'text', phase = 'plain'` and also populate final `content`.
- Reasoning deltas must become reasoning segments and must not be mixed into answer content.
- Tool calls must create `kind = 'tool'` segments at the point in the run where the tool activity belongs.
- Tool segments should reference existing `ToolCallRecord`s, not embed duplicated records.
- Auxiliary side-task tools, including auxiliary vision analysis before the main chat model, must create `kind = 'tool', phase = 'auxiliary'` segments and appear before the main model's plain/tool-loop/synthesis segments.
- Skipped, blocked, approval-rejected, errored, and cancelled tools must still create tool segments when they create visible `ToolCallRecord`s. The timeline should show that the tool step happened and why it did not produce a normal success result.
- Segment ordering must be deterministic across streaming, persistence, reload, and regeneration.

### 4. Planning vs Synthesis Separation

Planning or tool-loop narration must not be accumulated into the same `content` channel as final synthesis.

- During tool-loop phases, visible narration may appear in segments but should not overwrite or pollute final `content`.
- Final `content` should come from synthesis/plain answer phases only.
- If the provider emits text while also emitting tool calls, that text should be preserved as a tool-loop segment, not dropped.
- If tool-loop text is only provider reasoning or internal planning, it should be stored as reasoning or omitted according to existing thinking settings, but it must not be silently reclassified as final answer.

### 5. Streaming Contract

Streaming and persistence must use the same segment structure.

- Frontend stream state should maintain `snapshot.segments`, not only `snapshot.content/reasoning/toolCalls`.
- MVP must extend the existing `chat-stream` event rather than introducing a separate `chat-segment` event. This keeps the current listener lifecycle and done handling in one stream channel while adding segment identity.
- `chat-stream` payloads must include enough segment identity to append deltas to the correct segment:
  - `conversationId`
  - `runId`
  - `messageId`
  - `segmentId`
  - `kind`
  - `phase`
  - `order`
  - `step_number`
  - `round`
  - `delta` or full `text`
  - `tool_call_id` for tool segments
- Tool lifecycle events may continue to use `chat-tool`, but timeline placement should come from tool segments.
- Completion should not cause a blank/flash transition. `finishStreamingRun` must not clear `snapshot.segments` before `reloadConversation` returns with the persisted message, or it must patch the reloaded message with the same `messageId` in place. The user should never see an empty assistant bubble between stream completion and persisted render.

### 6. Frontend Timeline Rendering

`MessageBubble` should render assistant messages by ordered segments when segments exist.

- Text segments render as compact markdown paragraphs.
- Synthesis/plain text segments should visually read as the answer.
- Tool-loop text segments should be visually lighter than final answer text but still readable.
- Reasoning segments should use the existing `ReasoningBlock` affordance, adapted for segment rendering.
- Tool segments render compact `ToolCallBlock` instances using the referenced `ToolCallRecord`.
- Repeated tool lifecycle updates should update the same tool block rather than adding duplicate blocks.
- If a segment references a missing tool record, render a small degraded placeholder rather than crashing.
- Existing artifact rendering must continue to work, including generated image artifacts attached to message/tool records.
- The legacy three-bucket fallback remains for messages without segments.

### 7. Persistence, Migration, and Replay

- Conversation JSON must preserve `segments` through save/load.
- Old conversation JSON without `segments` must deserialize successfully.
- Context construction and provider replay should continue to use existing `content`, `api_messages`, and `model_messages` contracts unless explicitly changed.
- Regeneration should replace the target assistant message with a newly segmented message.
- Editing an assistant final answer should update `content`; segment editing is out of scope for MVP.
- Context compression should not depend on rendering segments unless a future task explicitly designs transcript compression around timeline segments.

### 8. Observability and Debugging

- Segment records should carry enough metadata to debug ordering bugs: `id`, `order`, `phase`, `step_number`, and `round`.
- Tool segments should correlate to `ToolCallRecord.trace_id` / `span_id` where available through `tool_call_id`.
- Unit tests should cover segment order through multi-round tool execution.

## Acceptance Criteria

- [ ] Assistant messages can persist ordered `segments` containing interleaved text, reasoning, and tool references.
- [ ] During streaming, the frontend renders the same segment structure that will be persisted after completion.
- [ ] After stream completion and conversation reload, visible process narration and tool placement do not disappear or collapse back into three buckets.
- [ ] Tool-loop/planning narration does not populate final `content`.
- [ ] Final `content` remains the final answer text for copy/edit/context compatibility.
- [ ] Persisted legacy fields are double-written from segments: `content` joins plain/synthesis text segments, `reasoning` joins reasoning segments, and `tool_calls` keeps the full deterministic record list.
- [ ] Legacy conversations without `segments` still render with the existing flat-field fallback.
- [ ] Tool records remain deterministic and provider replay compatibility is preserved.
- [ ] A multi-round tool run renders in chronological order: narration/reasoning, tool, narration/reasoning, tool, synthesis.
- [ ] Auxiliary side-task tools render in the timeline before the main model response instead of only appearing in a top-level tool bucket.
- [ ] Skipped/blocked/approval-rejected/cancelled visible tool records have matching timeline tool segments.
- [ ] Finishing a streaming run does not clear `snapshot.segments` before the persisted message is applied, and reload patches the same assistant message without an empty intermediate frame.
- [ ] Regeneration creates a fresh segmented assistant message.
- [ ] TypeScript typecheck and Rust tests pass where practical.

## Out of Scope

- Do not implement a short-term preview-only workaround.
- Do not remove `content`, `reasoning`, or `tool_calls` from storage.
- Do not redesign provider replay or migrate to another API protocol.
- Do not build a full trace viewer.
- Do not make individual timeline segments user-editable in this task.
- Do not redesign context compression around segments in this task.
- Do not change Lens chat rendering unless it directly consumes shared Chat message types and breaks type safety.

## Technical Notes

Likely impacted files:

- `src-tauri/src/chat/types.rs`
- `src-tauri/src/chat/agent/types.rs`
- `src-tauri/src/chat/agent/stream.rs`
- `src-tauri/src/chat/agent/loop_.rs`
- `src-tauri/src/chat/commands.rs`
- `src/chat/types.ts`
- `src/chat/Chat.tsx`
- `src/chat/MessageList.tsx`
- `src/chat/MessageBubble.tsx`
- `src/chat/ReasoningBlock.tsx`
- `src/chat/ToolCallBlock.tsx`

Relevant specs:

- `.trellis/spec/backend/agent-runtime.md`
- `.trellis/spec/guides/cross-layer-thinking-guide.md`
- `.trellis/spec/frontend/index.md`

## Testing Strategy

- Backend unit tests:
  - serde compatibility for old messages without segments
  - segment order for plain response
  - segment order for one tool round
  - segment order for multi-round tool execution
  - auxiliary tool records create auxiliary tool segments before main model segments
  - skipped/blocked/approval-rejected/cancelled tool records still create timeline tool segments
  - legacy `content`, `reasoning`, and `tool_calls` are derived consistently from segments
  - cancellation/regeneration preserves persisted segment consistency
- Frontend type/build:
  - `npm run typecheck`
  - `npm run lint`
- Backend test commands:
  - targeted `cargo test --manifest-path src-tauri/Cargo.toml chat::agent:: -- --nocapture`
  - full `cargo test --manifest-path src-tauri/Cargo.toml` when practical
- Manual smoke tests:
  - one plain chat response
  - one tool call response
  - multi-tool/multi-round response
  - auxiliary vision analysis with image input
  - Plan-mode blocked tool and approval-rejected tool
  - regenerate response
  - reload conversation after stream completion
  - open an old conversation without segments

## Open Question

Should tool-loop visible text render as normal timeline narration by default, or should it be visually grouped under a collapsible "过程" style distinct from the final answer?

Recommended answer: render it as lightweight timeline narration by default, not hidden behind the reasoning collapse. The product goal is process-style storytelling, and hiding narration would recreate the current "process disappeared" feeling. True provider reasoning should still use `ReasoningBlock`.

## Definition of Done

- PRD reviewed and accepted.
- Implementation preserves old conversations.
- New segment model is documented in code/types.
- Backend and frontend agree on segment serialization.
- Required tests pass or skipped commands are explicitly recorded with reasons.
