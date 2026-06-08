# Lens Chat Handoff

## Scenario: Lens Question Transfer To Chat

### 1. Scope / Trigger

- Trigger: changes to `lens_send_to_chat`, `chat_take_external_sends`, Lens screenshot cleanup, Chat external-send events, or Chat frontend send orchestration.
- The handoff bridges Lens temporary screenshot storage, a backend pending-send queue, the Chat window event surface, and the Chat frontend manual send state machine.

### 2. Signatures

- Backend commands:

```rust
lens_send_to_chat(app, state, image_id: String, question: String)
    -> { success: bool, requestId?: string, error?: string }

chat_take_external_sends(state)
    -> { success: bool, requests: PendingChatExternalSend[], error?: string }
```

- Backend queue payload:

```rust
PendingChatExternalSend {
    id: String,
    content: String,
    attachments: Vec<PendingChatExternalAttachment>,
}

PendingChatExternalAttachment {
    id: String,
    type: String, // "image" or "file"
    name: String,
    path: String,
}
```

- Frontend bridge:

```ts
api.lensSendToChat(imageId: string, question: string)
api.chatTakeExternalSends()
api.onChatExternalSendReady(listener)
```

- Event name: `chat-external-send-ready`.

### 3. Contracts

- `image_id` may be empty for text-only handoff. If it is non-empty, resolve it only through `resolve_explain_image_path`.
- `question` is trimmed before queueing. Empty `image_id` plus empty `question` is invalid.
- `lens_send_to_chat` must copy any Lens screenshot to an independent temp PNG before queueing. It must not pass the active Lens temp path directly.
- `lens_send_to_chat` must not create Chat conversations, call `chat_send_message`, call model/runtime code, or bind a specialized assistant.
- The backend must enqueue the request before opening/focusing Chat, then emit `chat-external-send-ready` as a notification. The queue is the source of truth; the event is only a wakeup.
- Chat frontend must call `chat_take_external_sends` on mount and when the ready event fires, then send each request through the same `handleSendMessage` path used by manual input/image send.
- Chat frontend must keep a local queue after taking backend requests. If Chat is currently streaming or a send is in flight, the request stays queued locally and is retried after the run becomes idle.
- External Lens sends should create a normal Chat conversation with no assistant binding. They may use the current/default Chat provider/model and can infer attachment skills the same way manual sends do.
- Lens can close immediately after `lens_send_to_chat` succeeds because Chat will copy the handoff temp attachment into conversation storage during the normal manual send path.

### 4. Validation & Error Matrix

| Condition | Behavior |
|---|---|
| Empty `image_id` and empty `question` | Return `{ success: false, error }`; do not enqueue. |
| Unknown Lens `image_id` | Return resolver error; do not enqueue. |
| Screenshot copy fails | Return copy error; do not enqueue. |
| Chat window open/focus fails after enqueue | Remove the queued request by id and return the open error. |
| Chat window is not mounted when event fires | Request remains in backend queue until Chat calls `chat_take_external_sends` on mount. |
| Chat is already streaming when request is taken | Keep request in Chat frontend local queue; do not drop it. |
| Model generation is slow | User message, attachment preview, streaming state, tool progress, and stop button are owned by `handleSendMessage`, matching manual image send. |
| Chat send fails | Manual send error handling applies; the taken request is not repeatedly re-sent forever. |

### 5. Good/Base/Bad Cases

- Good: Lens has a screenshot and question; bridge copies the image to `lens-chat-<uuid>.png`, queues a `PendingChatExternalSend`, opens Chat, and Chat frontend sends it through `handleSendMessage`.
- Base: Lens has selected text or a typed question but no screenshot; bridge queues a text-only request and Chat sends a normal message.
- Base: Chat is busy with another response; the external request remains queued locally and sends after the current run completes.
- Bad: Calling `chat_send_message` directly from `lens_send_to_chat`; this bypasses optimistic user message rendering, input running state, tool progress, and stream snapshot handling.
- Bad: Depending only on a one-shot event without a queue; Chat can miss the event while the webview mounts and the Lens request disappears.
- Bad: Taking the backend queue and then dropping the request when Chat is busy; this causes intermittent "nothing displayed" behavior.

### 6. Tests Required

- Settings default test: `lens.send_to_chat` defaults to true for `Settings::default()` and empty serialized `LensConfig`.
- Rust compile/check must cover `PendingChatExternalSend` serialization, state initialization, and Tauri command registration.
- Frontend typecheck/lint must cover external-send payload types and hook dependencies.
- Manual smoke test: capture with Lens, ask a question, verify Lens closes, Chat focuses, a new normal conversation appears immediately with the screenshot/question, and generation starts with the normal stop button state.
- Manual slow-response smoke test: use a slow model or tool-using prompt and verify the user message, attachment preview, stream state, and tool calls appear before final completion.
- Manual fallback test: disable the Lens setting and verify existing in-Lens `lens_ask` streaming still answers in the overlay.

### 7. Wrong vs Correct

#### Wrong

```rust
let image_path = resolve_explain_image_path(app, state, image_id)?;
chat_send_message(app, state, conversation_id, question, vec![image_path], None).await?;
lens_close(app)?;
```

This races Lens cleanup and bypasses Chat frontend's manual send state machine.

#### Correct

```rust
let source = resolve_explain_image_path(app, state, image_id)?;
let handoff = temp_dir.join(format!("lens-chat-{}.png", Uuid::new_v4()));
fs::copy(source, &handoff)?;
state.pending_chat_external_sends.lock()?.push(PendingChatExternalSend {
    id,
    content,
    attachments: vec![PendingChatExternalAttachment { path: handoff, ... }],
});
open_chat_window(app)?;
app.emit_to("chat", "chat-external-send-ready", json!({}))?;
```

Chat frontend then calls `chat_take_external_sends()` and sends through `handleSendMessage(..., { forceNewConversation: true })`, reusing the same rendering, stream, tool, and cancellation behavior as manual image sending.
