# Thinking preview behavior and verification

The thinking header is a single 24px line: status, duration, and the newest nonempty reasoning line. New characters scroll horizontally into view; a new line replaces the previous preview. Finishing reasoning or the whole turn preserves the last preview without a height change. No frame, background, left border, or gradient overlays are used.

Clicking opens the complete plain-text thought in a scrollable area capped at 16rem. It remains open on completion. Scrolling upward pauses vertical following until the reader returns within 24px of the bottom. Collapsing restores the single-line preview and unmounts the full text after the disclosure animation. Historical conversations still mount collapsed.

## Reference implementations

- [Hermes Desktop, pinned source](https://github.com/NousResearch/hermes-agent/blob/67f7e1d6b3115b8812caa58156c0f9b5531af072/apps/desktop/src/components/assistant-ui/thread/message-parts.tsx#L166): remembers a previously visible live preview, retains it after completion, respects explicit disclosure choices, and observes content height before scrolling. Kivio retains these interaction principles.
- [ZCode, pinned source](https://github.com/zai-org/ZCode/blob/328c1a0c0ffaa5a4f65e8fa199af5e4c20706e5f/packages/ui/src/components/ai-elements/reasoning.tsx#L242): uses the latest nonempty line as its collapsed streaming summary and follows its horizontal end. Kivio uses this single-line presentation but keeps the final preview visible, as requested.

## Kivio integration

`ReasoningBlock` owns manual disclosure, live-preview retention, and local scrolling. The preview observes its text and viewport width, updating `scrollLeft` only when needed. It extracts the final nonempty line without splitting the complete history into an array. The full text is mounted only when expanded; a content-height observer then follows new lines without unconditional per-token `scrollTop` writes. Completion preserves the current view and reader intent. The existing stream store's batching remains in use; no extra token buffer or continuous animation loop was introduced.

`TimelineSegments` retains the surrounding Working group when live reasoning was shown; otherwise its old automatic folding would hide the child anyway. Groups first loaded from history remain collapsed, as do tool-only groups after completion. The existing keyed MessageList handoff keeps component state when the live row becomes history. Reopening the conversation resets this transient presentation state.

`ReasoningPreviewContext` carries only the parallel-column visibility policy from `MessageGroup` to these two consumers. It lets unfocused columns suspend previews and unmount their text without confusing loss of focus with reasoning completion. It does not store conversation data or streaming text.

## Verification

- Tests cover latest-line extraction (including CRLF and trailing empty lines), horizontal following after text/viewport changes, lazy full-text rendering, explicit expand/collapse, duration retention, and vertical reading intent. Timeline, multi-column focus, and actual MessageList live-to-history tests preserve the completed preview. Existing history/disclosure tests remain in place.
- An Edge fixture using the actual React component and application stylesheet tested 700px and 280px widths with 5,000 historical reasoning lines and 80 incremental updates. Both kept the header at 24px, followed the latest characters, omitted full history from the DOM, and produced 0px answer movement on completion. A new short line reset horizontal scrolling. Manual expansion and subsequent unmount on collapse passed as well.
- Browser verification is component-level, not a whole-application performance guarantee. Temporary fixtures were removed. Native desktop WebView and a real model response were not exercised for this change.
