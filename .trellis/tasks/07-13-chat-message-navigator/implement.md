# Chat Message Navigator — Implementation Plan

## 1. Build the semantic projection

- [x] Extract or extend the existing message-list projection so render items and semantic navigation nodes are produced from one grouping pass.
- [x] Define turn ownership for user messages, single assistant responses, and multi-model `MessageGroup` items.
- [x] Add compaction navigation nodes pointing at the corresponding summary render item.
- [x] Produce bounded preview strings and model aggregation labels without rendering Markdown.
- [x] Add pure unit tests for ordinary turns, consecutive user messages, multi-model groups, compaction nodes, and non-semantic transient items.

## 2. Add the message rail UI

- [x] Create the message navigator component with ordinary, active, hovered, and compaction node variants.
- [x] Add the hover preview card with 2-line question, 3-line answer, and model footer.
- [x] Make the node viewport vertically scrollable without merging nodes.
- [x] Add accessibility labels, keyboard-focus behavior for clickable nodes, and reduced-motion-compatible transitions.

## 3. Integrate with `MessageList`

- [x] Derive the active node from the existing `VirtualizerHandle.findItemIndex()` at the 30% reading baseline.
- [x] Connect node clicks to `scrollToIndex(..., align: 'start')`.
- [x] Connect rail wheel gestures to deterministic previous/next-node navigation and prevent duplicate pixel scrolling.
- [x] Auto-scroll the rail viewport to keep the active node visible when the main message list scrolls.
- [x] Preserve stick-to-bottom, new-user-message jump, and “back to bottom” behavior.

## 4. Layout and responsive behavior

- [x] Position the rail in the left whitespace without shrinking the centered `max-w-3xl` message column.
- [x] Show only when there are at least four turn nodes and the message area is at least 720px wide.
- [x] Keep the existing right-side scrollbar visible and functional.
- [x] Verify light/dark theme styles and narrow-to-wide container-query behavior in code and tests.

## 5. Validation

- [x] Run focused message navigator/model tests.
- [x] Run existing message list and streaming render tests.
- [x] Run `npm run typecheck`.
- [x] Run targeted lint for all changed TypeScript/TSX files. Full-project lint remains blocked by the pre-existing `src/chat/conversationExport.ts` `no-control-regex` error.
- [x] Run focused Vitest coverage for the navigator, grouping, and streaming MessageList integration.
- [ ] Manually verify against a persisted 4+ turn conversation. The local profile currently contains only three 1-turn conversations, so full real-data visual QA remains unavailable without polluting user data.

## Risk and Rollback Points

- After semantic projection: verify render indexes still match every `RenderItem` insertion path before wiring UI.
- After wheel integration: verify one physical gesture causes only semantic navigation and does not also bubble into pixel scrolling.
- After active-node tracking: verify state updates do not occur on every scroll frame when the active id is unchanged.
- If regressions appear, the navigator can be feature-disabled at the `MessageList` render boundary without altering stored conversations or backend behavior.
