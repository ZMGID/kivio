# Fix AI Chat Streaming Repetition and Disappearing Messages

## Goal

Fix the AI Chat client so streamed assistant output displays once, then persists in the conversation after generation completes.

## What I Already Know

- User reports two visible failures: streamed text repeats while generating, and the conversation disappears after output finishes.
- The affected surface is the Chat client in `src/chat/**` plus chat streaming commands in `src-tauri/src/chat/**` / `src-tauri/src/api.rs`.
- `src/chat/Chat.tsx` already has uncommitted work adding optimistic pending user messages and a send-in-flight guard.
- `src-tauri/src/chat/commands.rs` already has uncommitted work saving the user message before calling the model.
- React StrictMode is enabled in `src/main.tsx`; async Tauri listeners need a cancelled cleanup guard or duplicate listeners can survive dev remounts.
- The Chat cross-layer contract says `chat-stream` emits `{ imageId, kind: 'answer', delta, reasoningDelta?, done?, reason?, full? }`.

## Requirements

- Chat stream listeners must not be duplicated by async listener setup or React StrictMode remounts.
- Each streamed assistant chunk should be appended exactly once in the UI.
- Backend streaming should emit incremental `delta` text even if an OpenAI-compatible provider sends cumulative text snapshots.
- Sending a message should keep the user message visible during generation.
- When generation completes, the persisted conversation should include both the user message and assistant response.
- Failed generation should not remove the saved user message.

## Acceptance Criteria

- [x] `chat-stream` event listener cleanup follows the established async cancelled-flag pattern.
- [x] SSE parsing emits only the new suffix when provider payloads contain cumulative content.
- [x] Existing Lens and screenshot translation streaming behavior remains compatible.
- [x] `npm run lint` passes.
- [x] `npm run typecheck` passes.
- [x] `cargo test --manifest-path src-tauri/Cargo.toml` passes.

## Out of Scope

- Adding attachments or image support to Chat.
- Redesigning the Chat UI.
- Adding frontend unit or e2e test infrastructure.

## Technical Notes

- Relevant files inspected: `src/chat/Chat.tsx`, `src/chat/MessageList.tsx`, `src/chat/api.ts`, `src/api/tauri.ts`, `src-tauri/src/chat/commands.rs`, `src-tauri/src/api.rs`, `src-tauri/src/chat/storage.rs`, `src-tauri/src/chat/types.rs`.
- Relevant specs: `.trellis/spec/frontend/type-safety.md`, `.trellis/spec/guides/cross-layer-thinking-guide.md`, `.trellis/spec/guides/code-reuse-thinking-guide.md`.
