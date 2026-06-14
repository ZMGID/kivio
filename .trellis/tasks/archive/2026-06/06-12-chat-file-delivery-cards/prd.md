# Chat File Delivery Cards

## Goal

When an assistant produces a report or other substantial generated content, Kivio should present the final answer as a short conversational summary plus a clickable file card. The card should feel like a delivered document: it has a file-type icon, title, preview, open action, and overflow affordance, matching the reference image the user provided.

## What I Already Know

* The user wants to mimic the file delivery pattern shown in the reference image, not the intermediate reasoning flow.
* The target scenario is a prompt such as "调查一下最近的AI资讯，然后给我个总结报告。"
* The desired result is a concise assistant message followed by an attached report card, likely for Markdown first.
* The repository already has a Kivio chat interface, native file tools, attachment support, markdown rendering, tool call blocks, and chat persistence.

## Assumptions

* MVP should support generated Markdown report cards first.
* The card should open the generated local file using the system default app or an existing safe preview/open command.
* The feature should reuse existing chat persistence and native file/tool plumbing where possible.
* The model/tool layer may already produce file artifacts; the UI should surface those artifacts instead of relying on brittle text parsing where possible.

## Open Questions

* None blocking. The MVP will use the recommended artifact-only path and leave long-answer auto-file detection for a later task.

## Requirements

* Render generated non-image file artifacts in assistant messages as distinct file cards below the assistant text.
* Include file type, title/name, short preview, size when available, and an open action.
* Support Markdown report files as the first-class MVP format, with graceful cards for text, CSV, JSON, HTML, and XLSX artifacts.
* Preserve cards across chat reloads.
* Keep the answer text concise; the full report content belongs in the delivered file.
* Continue rendering generated image artifacts exactly as image previews rather than file cards.

## Acceptance Criteria

* [ ] A generated Markdown artifact can appear under an assistant response as a file card.
* [ ] The card shows a recognizable Markdown/document icon, filename/title, and a preview excerpt.
* [ ] Clicking the card opens the exported file when a persisted path is available, or an equivalent safe preview when only inline artifact data is available.
* [ ] The file card persists after app reload / conversation reload.
* [ ] Existing image attachments, tool call blocks, and normal text-only answers continue to render normally.
* [ ] File cards do not duplicate artifacts already rendered inline as Markdown images.

## Definition of Done

* Lint passes.
* TypeScript typecheck passes.
* Relevant Rust tests pass when backend changes are made.
* Focused frontend tests are added or updated for card rendering/persistence where practical.
* Manual smoke test covers a message with a file card and a message without one.

## Out of Scope

* Full document editor behavior inside chat.
* Cloud upload/share links.
* Automatic conversion to PDF/DOCX.
* Rebuilding the reasoning/steps UI shown in the second image.

## Technical Notes

* Initial likely frontend files: `src/chat/MessageBubble.tsx`, `src/chat/types.ts`, `src/chat/persistence.ts`, `src/chat/ChatAttachments.tsx`, `src/chat/ToolCallBlock.tsx`.
* Initial likely backend files: `src-tauri/src/chat/types.rs`, `src-tauri/src/chat/commands.rs`, `src-tauri/src/native_tools/files.rs`, `src-tauri/src/native_tools/sandbox_exports.rs`.
* Existing `ChatToolArtifact` carries `name`, `mime_type` / `mimeType`, `data_url` / `dataUrl`, and `size_bytes` / `sizeBytes`.
* `MessageBubble` currently flattens `message.artifacts` plus `toolCall.artifacts` and renders unreferenced image artifacts as image previews. Non-image artifacts are currently not surfaced as document cards.
* `run_python` captures files such as `md`, `txt`, `html`, `csv`, `json`, `xlsx`, and image outputs as artifacts; backend exports them under `~/Kivio/runs/{conversation}/{message}/` for roughly seven days but does not expose the exported path in the artifact shape yet.
* Existing `chat_open_attachment` opens stored attachment files by path, but it is scoped to conversation attachments. A generated artifact opener should validate paths under the sandbox export tree or use safe temporary previews for inline-only artifacts.

## Technical Approach

1. Extend the artifact shape with optional persisted path metadata for backend-exported sandbox artifacts. Keep old conversations compatible by making the field optional.
2. Update sandbox artifact export to return enough metadata to attach local file paths back to `ChatToolArtifact` records before they are persisted/emitted.
3. Add a focused frontend `GeneratedFileArtifacts` renderer for non-image artifacts:
   * document-style card with file icon, title, metadata, preview excerpt, and overflow/open affordance
   * Markdown/text-like previews decoded from safe `data:` URLs
   * binary/table formats show type and size without trying to parse contents in the UI
4. Add a Tauri command/API helper to open generated artifact files safely when a local exported path exists.
5. Keep image artifacts on the existing image preview path.

## Decision (ADR-lite)

**Context**: There are two possible ways to imitate the reference: auto-convert long assistant text into files, or render artifacts that tools already generated. Auto-conversion risks surprising users and changing model behavior; artifact rendering builds on existing runtime contracts.

**Decision**: MVP renders explicit generated artifacts as file delivery cards, with Markdown/text previews and safe open behavior. It does not auto-save arbitrary long assistant answers.

**Consequences**: The experience works best when the model uses `run_python` or file tools to generate a report artifact. A future task can add explicit "save this answer as..." actions or model prompt nudges for report-style requests.
