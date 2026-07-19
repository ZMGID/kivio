# Component Guidelines

> How components are built in this project.

---

## Overview

<!--
Document your project's component conventions here.

Questions to answer:
- What component patterns do you use?
- How are props defined?
- How do you handle composition?
- What accessibility standards apply?
-->

(To be filled by the team)

---

## Component Structure

<!-- Standard structure of a component file -->

(To be filled by the team)

---

## Props Conventions

<!-- How props should be defined and typed -->

(To be filled by the team)

---

## Styling Patterns

### Virtualized row mount motion

**Trigger**: A component is rendered inside `virtua` or another list that unmounts offscreen rows.

**Contract**: Historical rows must not run entrance animation merely because the virtualizer remounted their DOM. Mount/unmount is an implementation detail, not a semantic "new item" event. Gate entrance motion on explicit live/new state owned by the feature.

```tsx
// Wrong: every virtual remount replays opacity + translate motion.
<div className="chat-motion-fade-up">...</div>

// Correct: only the live streaming preview has semantic entrance motion.
const playEntranceAnimation = messageStreaming
<div className={playEntranceAnimation ? 'chat-motion-fade-up' : ''}>...</div>
```

**Cases**:

- Good: a streaming preview enters with motion, then historical remounts remain visually stable.
- Base: a historical row mounts for the first time without entrance motion.
- Bad: scrolling causes old content to fade, translate, shimmer, or otherwise look refreshed.

**Tests required**:

- Assert historical user and assistant rows do not receive the mount-animation class.
- Assert the explicit live/streaming state still receives the intended class.
- For regressions involving virtualization, verify with real wheel deltas and a long conversation; component tests alone do not reproduce DOM churn.

### Streaming data in virtualized lists

**Trigger**: A virtualized list contains a high-frequency streaming tail while historical rows are
stable.

**Contract**:

- Keep historical `data` and navigator/index derivations dependent only on structural message
  identity, grouping, and boundary changes.
- Represent streaming, error, and bottom content as fixed tail slots whose payload is resolved by
  the render callback. Token deltas must not rebuild an O(N) data array or navigator map.
- Coalesce automatic bottom alignment with `requestAnimationFrame`; keep at most one pending frame
  and cancel it on unmount.
- Test `ResizeObserver` mocks must deliver asynchronously. A synchronous callback during React or
  virtua mount can produce a test-only lifecycle `flushSync` warning that browsers do not model.

```tsx
const slots = useMemo(
  () => [...historyItems.map(item => ({ kind: 'history', item })), tailSlot],
  [historyItems],
)

<Virtualizer data={slots}>{renderSlot}</Virtualizer>
```

**Tests required**:

- Spy on the structural navigator builder and assert repeated stream frames do not call it again.
- Verify bottom scrolling is coalesced and cleanup cancels a pending frame.
- Run the long-list streaming test without React lifecycle `flushSync` warnings.

---

## Accessibility

<!-- A11y requirements and patterns -->

(To be filled by the team)

---

## Common Mistakes

### Treating React mount as a product event

**Symptom**: scrolling feels sticky and old content appears to refresh even when frame timing and scroll-handler JavaScript are within budget.

**Cause**: CSS entrance classes are attached unconditionally to a component that a virtualizer repeatedly mounts and unmounts.

**Prevention**: before adding mount animation to list content, determine whether remounts can happen during scrolling. Drive motion from semantic state such as `messageStreaming`, not component lifetime.

### Treating token content as list structure

**Symptom**: CPU usage grows with conversation length during streaming even though only the last
assistant message changes.

**Cause**: token deltas rebuild historical row arrays, navigator maps, or flattened React children.

**Prevention**: separate structural history data from fixed dynamic tail slots and verify the
structural builder call count in tests.
