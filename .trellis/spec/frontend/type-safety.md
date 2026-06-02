# Type Safety

> Type safety patterns in this project.

---

## Overview

<!--
Document your project's type safety conventions here.

Questions to answer:
- What type system do you use?
- How are types organized?
- What validation library do you use?
- How do you handle type inference?
-->

(To be filled by the team)

---

## Type Organization

<!-- Where types are defined, shared types vs local types -->

(To be filled by the team)

---

## Validation

<!-- Runtime validation patterns (Zod, Yup, io-ts, etc.) -->

(To be filled by the team)

---

## Common Patterns

<!-- Type utilities, generics, type guards -->

(To be filled by the team)

---

## Forbidden Patterns

<!-- any, type assertions, etc. -->

(To be filled by the team)

---

## Scenario: Chat MVP Cross-Layer Contract

### 1. Scope / Trigger
- Trigger: Chat features span React state, the centralized Tauri bridge, Rust Tauri commands, settings persistence, and JSON file storage.
- Apply this contract whenever changing `src/chat/**`, `src/api/tauri.ts`, or `src-tauri/src/chat/**`.

### 2. Signatures
- `chat_get_conversations(offset: usize, limit: usize, folder?: string) -> { success: true, conversations: ConversationListItem[] }`
- `chat_get_conversation(conversationId: string) -> { success: true, conversation: Conversation }`
- `chat_create_conversation(providerId?: string, model?: string, folder?: string) -> { success: true, conversation: Conversation }`
- `chat_send_message(conversationId: string, content: string, attachments: string[]) -> { success: boolean, conversation?: Conversation, error?: string }`
- `chat_update_conversation(conversationId: string, title?: string, pinned?: boolean, folder?: string, providerId?: string, model?: string) -> { success: true, conversation: Conversation }`
- `chat-stream` event payload is `ChatStreamPayload` in `src/api/tauri.ts`.

### 3. Contracts
- Conversation IDs are backend-owned and must match `conv_*`; frontend must pass IDs returned by chat commands or parsed from `#chat/{conversation_id}`.
- Stored Rust fields use snake_case (`provider_id`, `created_at`); Tauri command argument names from TypeScript use camelCase (`conversationId`, `providerId`).
- Chat model defaults live in settings as `chatProviderId` and `chatModel`; when absent, backend sanitization falls back to Lens provider/model, then translator provider/model.
- Model providers expose `enabledModels` and `availableModels`; do not read a `models` array in Chat UI.
- Streaming payload fields: `{ imageId, kind: 'answer', delta, reasoningDelta?, done?, reason?, full? }`.

### 4. Validation & Error Matrix
- Invalid conversation ID -> backend returns `Err("Invalid conversation id: ...")`; do not construct file paths directly in frontend.
- Missing conversation file -> backend returns a failed invoke; frontend shows a visible load/send error.
- Pagination offset beyond loaded index length -> backend returns an empty list, never panics.
- Stream `reason: 'error'` -> frontend clears streaming state, shows an error, and re-enables input.
- Stream `reason: 'cancelled'` -> frontend clears streaming state without forcing a conversation reload.

### 5. Good/Base/Bad Cases
- Good: `ModelSelector` uses `enabledModels.length > 0 ? enabledModels : availableModels`, then persists through `chat_update_conversation`.
- Base: `#chat` shows the empty Chat shell; `#chat/{id}` loads the referenced conversation.
- Bad: listening for `{ kind: 'chunk', text }` on `chat-stream`; backend emits `delta` and `done`, not `text`.

### 6. Tests Required
- `npm run typecheck` must catch mismatches between bridge types and Chat UI props.
- `npm run lint` must pass without `any` stream payload listeners.
- `cargo test --manifest-path src-tauri/Cargo.toml` must include settings fallback assertions when changing chat defaults.
- `npm run build:ui` must pass after route or lazy-loaded Chat component changes.

### 7. Wrong vs Correct

#### Wrong
```tsx
listen<any>('chat-stream', (event) => {
  if (event.payload.kind === 'chunk') {
    setStreamingContent((prev) => prev + event.payload.text)
  }
})
```

#### Correct
```tsx
api.onChatStream((payload) => {
  if (payload.delta) {
    setStreamingContent((prev) => prev + payload.delta)
  }
  if (payload.done) {
    setStreaming(false)
  }
})
```
