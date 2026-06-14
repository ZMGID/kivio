# Lens Current Flow

## Scope

This note maps the current Lens screenshot / vision Q&A / screenshot translation flow before connecting it to the AI client.

## Frontend Entry Points

* `src/App.tsx` routes `#lens` to `src/Lens.tsx`; `#chat` routes to the AI client `Chat` view. They share the same React app bundle but render different view trees.
* `src/Lens.tsx` owns Lens state locally: selection stage, captured frame, `imageId`, screenshot preview, chat `messages`, translate state, web search state, history UI, annotation arrows, and streaming flags.
* Lens Q&A messages use `ExplainMessage` from `src/api/tauri.ts`: `{ role: 'user' | 'assistant', content, reasoning?, webSearch? }`.
* Lens history is not a Chat conversation. `src/lens/history.ts` stores local history in browser storage, while image persistence is handled separately by `lens_commit_image_to_history`.

## Backend Entry Points

* Tauri commands are registered in `src-tauri/src/main.rs`: `lens_request`, `lens_capture_window`, `lens_capture_region`, `lens_register_annotated_image`, `lens_ask`, `lens_translate`, `lens_cancel_stream`, `lens_commit_image_to_history`, and related commands.
* `src-tauri/src/lens_commands.rs` opens the Lens window, captures screenshots, registers temp image IDs, resolves images, and dispatches Lens ask/translation.
* `AppState` stores Lens images in `explain_images: Mutex<HashMap<String, PathBuf>>` and the active image in `current_explain_image_id`.
* `lens_ask` calls `api::call_vision_api`, passing Lens settings, provider/model overrides, stream setting, prompt overrides, `imageId`, and `ExplainMessage[]`.

## Ask Flow

1. User enters Lens chat mode through a shortcut or command.
2. Lens captures a window/region or takes selected text for text-only mode.
3. Rust registers the screenshot in `explain_images` and returns an `imageId`.
4. Frontend reads the image preview through `explain_read_image`.
5. `Lens.tsx#doSend` appends a local user message and assistant placeholder.
6. If arrows were drawn, the frontend composes an annotated PNG and registers it through `lens_register_annotated_image`.
7. Frontend invokes `lens_ask(imageId, messages, { webSearch })`.
8. Backend optionally runs Lens web search, then calls `call_vision_api`.
9. Streaming uses `lens-stream` events keyed by `imageId`; Lens accumulates deltas into the local assistant placeholder.

## Translation Flow

* Screenshot translation uses `lens_translate` and `lens-translate-stream`, not Chat.
* Text translation through Lens uses `lens_translate_text`.
* Translation has its own original/translated streaming payload shape and should not be merged into Chat runtime unless product requirements change.

## Current Strengths

* Very low ceremony: no conversation creation required.
* Works for temp screenshot Q&A and fast floating overlay UX.
* Supports image annotation before asking.
* Independent cancellation token: `explain_stream_generation`.
* Lens web search status is rendered inline through `lens-web-search`.

## Current Limitations For AI Client Integration

* Lens messages are not persisted as Chat conversations and do not get Chat title, project, assistant, context compression, memory, tools, or MCP behavior.
* Lens uses `call_vision_api`, a direct OpenAI-compatible chat completions path, rather than `chat::agent::run_agent_loop`.
* Lens events are keyed by `imageId`; Chat events are keyed by `conversationId + runId + messageId`.
* Lens screenshot files live under temp/history image maps, not Chat conversation attachment directories.
* Lens has a Lens-specific web search pipeline instead of Chat native tools.

