# Chat scroll stutter diagnosis and fix design

## Problem boundary

The work is limited to the long-conversation render and scroll path. It must distinguish perceived refresh caused by DOM remount/animation from actual message reloads, and distinguish browser rendering pressure from JavaScript/React work before selecting a fix.

Primary code boundaries:

- `src/chat/MessageList.tsx`: virtualizer configuration, scroll callback, navigator derivation, bottom-follow state, and row rendering.
- `src/chat/MessageBubble.tsx`: historical row mount behavior and entrance-animation classes.
- `src/index.css`: `chat-motion-fade-up` animation definition.
- Markdown/KaTeX rendering components reached by `MessageBubble`: content-dependent mount cost.
- Existing `MessageList`/`MessageBubble` tests and browser test tooling: regression and measurement harness.

## Temporary measurement layer

Add a development-only collector that is deliberately easy to delete. It will not be part of the final architecture.

The collector records:

- `requestAnimationFrame` gaps, grouped into over 16.7 ms, 25 ms, and 50 ms buckets;
- browser long tasks through `PerformanceObserver` when supported;
- scroll callback event count, total time, maximum time, and navigator lookup time;
- React `Profiler` commit count and duration around the message list;
- message-row and expensive-content mount/unmount counts keyed by message ID;
- virtual rendered-row count/range changes;
- navigator state-update counts;
- active CSS animation count on newly mounted historical bubbles.

The collector exposes one development-only snapshot/reset entry point so the browser runner can capture structured before/after results. No persistence, telemetry, backend calls, or production logging is added.

## Reproduction and experiment matrix

Use one deterministic long conversation containing representative plain Markdown, code blocks, tables, reasoning, and KaTeX. Drive it with small wheel deltas through a real browser, starting near the bottom and scrolling upward through the range where the user reports the issue.

Run these variants against identical content and wheel input:

1. Baseline: current behavior.
2. Animation isolation: suppress entrance animation for virtualized historical rows.
3. Buffer isolation: increase the `virtua` buffer so expensive rows mount farther outside the viewport.
4. Navigator isolation: temporarily bypass navigator derivation in the scroll callback.
5. Content isolation: compare plain-text rows with KaTeX/Markdown-heavy rows if the first three variants do not identify a dominant cause.

Change one variable per run. Save summarized measurements in the task research notes, not in product code.

## Fix selection rules

- If historical-row remount animation dominates, stop replaying entrance animation for already-existing virtual rows while preserving intentional animation for genuinely new/streaming content.
- If navigator work dominates, coalesce it to at most once per animation frame and avoid state updates when derived values are unchanged.
- If near-viewport expensive mounts dominate, tune the virtualizer buffer to prepare rows earlier without expanding DOM size excessively.
- If KaTeX/style recalculation remains dominant after animation and scheduling changes, apply the smallest content-specific mount optimization supported by the trace.
- Do not remove virtualization or redesign message loading unless the controlled experiments show that smaller fixes cannot meet the acceptance criteria.

Multiple evidence-backed changes may be combined only when each independently improves the measured scenario.

## Compatibility and behavior contracts

- Bottom detection retains the existing leave/re-enter hysteresis.
- Wheel-up continues to release automatic bottom following immediately.
- Streaming content remains pinned only while the user is at the bottom.
- Navigator active/visible nodes remain correct after any scheduling change.
- Conversation changes still reset and align the list to the bottom.
- Reduced-motion behavior remains respected.

## Cleanup and rollback

After the fixed build is measured with the same scenario, delete the collector, profiling wrappers, fixture hooks, diagnostic globals, and logs. Keep only the production fix, regression tests, and task-level measurement notes.

The production fix will be split into small edits so each candidate optimization can be reverted independently if it regresses navigation, bottom following, or content rendering.
