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
