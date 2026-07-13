# Conversation Markdown Export Contract

## Scenario: Export one persisted conversation as Markdown

### 1. Scope / Trigger

- Trigger: adding or changing an individual-conversation export format, save-dialog flow, or export command.
- The export is a read-only projection of persisted chat data. It must not reuse the full internal JSON serialization because that can expose reasoning, tool transcripts, attachment paths, and runtime state.

### 2. Signatures

Backend Tauri command:

```rust
chat_export_conversation_markdown(
    app: AppHandle,
    conversation_id: String,
    path: String,
    language: String,
) -> Result<(), String>
```

Frontend wrapper:

```ts
chatApi.exportConversationMarkdown(
  conversationId: string,
  path: string,
  language: 'zh' | 'en',
): Promise<void>
```

### 3. Contracts

- `conversationId`: id of an existing persisted conversation; the backend loads the authoritative copy through `chat::storage::load_conversation`.
- `path`: destination returned by the native Tauri save dialog. Cancellation is handled before invoking the command.
- `language`: `en` selects English labels; all other values fall back to Chinese labels.
- Output is UTF-8 Markdown containing only:
  - title, created time, updated time, and model;
  - user `content` and assistant final `content` in persisted order, preceded by a compact bold role/timestamp line rather than a heading;
  - attachment names rendered as localized placeholders.
- Output must exclude reasoning, segments, tool calls/results, artifacts, replay messages, provider ids, API configuration, and internal attachment paths.

### 4. Validation & Error Matrix

- Empty destination path -> return `export path is empty` without writing.
- Missing/invalid conversation id -> propagate the storage-layer load error.
- Unwritable destination -> return a contextual `write conversation export: ...` error.
- Native dialog cancellation -> frontend returns silently; backend is not invoked.
- Unknown message role or an internal-only empty message -> omit it from Markdown.

### 5. Good/Base/Bad Cases

- Good: a Chinese conversation with text and attachments exports one `.md` file with compact Chinese role labels, horizontal message separators, and attachment-name placeholders.
- Base: a conversation with no exportable messages still exports its metadata header.
- Bad: copying the persisted conversation JSON or rendering `reasoning`, `tool_calls`, `api_messages`, or attachment `path` values into Markdown.

### 6. Tests Required

- Rust renderer test: assert Chinese and English labels, chronological content, and attachment placeholders.
- Rust privacy regression: seed reasoning, provider id, replay content, and private attachment paths; assert none occur in output.
- Frontend filename test: assert invalid characters, empty titles, Windows reserved names, and Unicode length caps.
- Cross-layer build: command name and camelCase invoke payload must pass `cargo check` and the TypeScript/Vite build.

### 7. Wrong vs Correct

#### Wrong

```ts
const conversation = await chatApi.getConversation(id)
await writeTextFile(path, JSON.stringify(conversation))
```

This exports internal fields in the UI process and duplicates the backend persistence contract.

#### Correct

```ts
const path = await save({ filters: [{ name: 'Markdown', extensions: ['md'] }] })
if (path) await chatApi.exportConversationMarkdown(id, path, lang)
```

The backend loads the persisted source of truth and renders an allowlisted Markdown projection.
