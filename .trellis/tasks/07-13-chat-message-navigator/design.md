# Chat Message Navigator — Technical Design

## Overview

Add a semantic, scrollable message rail to the left side of the chat message viewport. The rail is derived entirely from the existing in-memory conversation and compaction state. It does not change persistence or backend contracts.

The rail and the existing pixel scrollbar coexist. The message list remains the source of truth for reading position; the rail projects that position into conversation turns.

## Architecture

### 1. Stable navigation model

Extract a pure projection that builds semantic navigation nodes while the existing message list items are assembled. Avoid maintaining a second, subtly different grouping algorithm.

```ts
type MessageNavigatorNode =
  | {
      kind: 'turn'
      id: string
      targetRenderIndex: number
      userMessageId: string
      title: string
      answerPreview: string
      modelLabel: string
    }
  | {
      kind: 'compaction'
      id: string
      targetRenderIndex: number
      title: string
      answerPreview: string
    }
```

One `turn` starts at a user message and owns the following assistant message or folded multi-model group until the next user message. A multi-model group contributes one turn node. Completed compaction summary items contribute a separate `compaction` node.

The builder must return or expose both render items and navigation nodes from the same pass so `targetRenderIndex` cannot drift when spacer, compaction, streaming, error, or group items change.

### 2. Message navigator component

Create a focused presentation component under `src/chat/`, owned by `MessageList`.

Responsibilities:

- Render one fixed-height row per node without merging nodes.
- Center the node stack vertically while it fits; when it exceeds the viewport, let the track grow from the start and scroll normally.
- Maintain its own overflow viewport when nodes exceed available height.
- Highlight the active node.
- Keep ordinary node geometry uniform while idle. Project the virtualizer's visible render-index interval onto semantic turn intervals and color every intersecting node dark. While the pointer is inside the rail, compute each node's width from its vertical distance to the pointer (bounded radius with eased falloff), producing a local fisheye effect; clear all inline widths on pointer leave. Keyboard focus retains a fixed visible affordance.
- Keep the active node visible as the message viewport moves.
- Render a hover preview card anchored to the hovered row.
- Send click and wheel navigation intents upward.

The component must not subscribe to `streamingStore`. Its inputs should contain stable, truncated preview strings. Token-level streaming updates must not rebuild the full rail.

### 3. Active-node projection

Continue using `MessageList` as the scroll owner. On its existing `handleScroll`, use `VirtualizerHandle.findItemIndex()` at a reading baseline:

```text
baseline offset = scrollOffset + viewportSize * 0.30
```

Map the returned render index to the most recent semantic node at or before that index. This keeps a long response active until the next turn crosses the reading baseline. Store only the active node id in React state, and avoid a state update when the id is unchanged.

Dynamic measurements remain owned by `virtua`; no DOM query or manually maintained message-height table is needed.

### 4. Navigation behavior

- Click: call `scrollToIndex(targetRenderIndex, { align: 'start', smooth })`.
- Reduced motion: set `smooth` to false when `prefersReducedMotion()` is true.
- Rail wheel: prevent the rail's independent browser wheel scroll, resolve direction from `deltaY`, select the previous/next semantic node, and navigate the message list to it.
- Main list scroll: update the active node, then ensure that node is visible inside the rail's overflow viewport.
- A navigation action should update `stickToBottomRef`: jumping away from the last node disables bottom following; jumping to the last node may restore it only when the resulting message position is at the bottom.

Wheel input should be throttled or accumulated so a trackpad gesture does not skip many turns uncontrollably. The exact helper should be deterministic and unit-testable.

### 5. Layout and responsiveness

Place the rail inside the existing relative `MessageList` shell, visually to the left of `.chat-message-list-inner`. It should overlay reserved outer whitespace rather than reduce the `max-w-3xl` content width.

At medium container widths (720–871px), the centered message column leaves insufficient outer whitespace. Add a navigator-presence class to the list shell and increase the message column's left padding only in this range. At wider widths, return to the natural outer gutter. This prevents rail ticks from reading as Markdown bullets or covering content.

Visibility conditions:

- At least four `turn` nodes.
- Chat message area container width at least 720px.

Use a container query or measured container width consistent with the existing `chat-main` container. CSS must hide the rail below the threshold without affecting the existing scrollbar.

### 6. Preview content

For a turn node:

- User content: maximum two visual lines.
- Assistant content: maximum three visual lines.
- Footer: model name for one model or an aggregate multi-model label.

For a compaction node:

- Fixed title describing context compaction.
- Summary excerpt from the existing compaction record.

Preview extraction should be a pure, bounded operation. MVP may normalize whitespace and truncate raw message content; it should not mount `ChatMarkdown` inside the popover.

## State and Data Flow

```text
messages + contextState + live structural state
                  |
                  v
       shared render-model projection
          |                    |
          v                    v
   RenderItem[]       MessageNavigatorNode[]
          |                    |
          v                    v
       virtua             MessageNavigator
          |
          v
 handleScroll -> findItemIndex -> active node id -> rail auto-reveal

 rail click/wheel -> targetRenderIndex -> virtua.scrollToIndex
```

## Compatibility and Risks

- Preserve the existing `scrollRef`, `VirtualizerHandle`, stick-to-bottom behavior, and bottom button.
- Do not intercept wheel events outside the rail.
- Prevent rail wheel events from bubbling into the main scroll container and causing a second pixel scroll.
- Streaming placeholders and errors are not semantic nodes.
- A conversation beginning with an assistant/system artifact before the first user message does not create a turn node.
- Compaction nodes are structural nodes but do not count toward the four-turn visibility threshold.
- Preview cards must not capture unintended wheel events or cause layout width changes.

## Rollback Shape

The feature is frontend-only and additive. Rollback consists of removing the rail component, projection fields, and related styles/tests; no stored data migration or backend rollback is required.
