# Diagnose chat scroll stutter

## Goal

Identify and remove the remaining perceptible stutter while scrolling long chat conversations. The diagnosis must use repeatable runtime measurements rather than visual guesswork, and the previously fixed bottom-button behavior must remain stable.

## Background

- Commit `d4fc977` stopped the "scroll to bottom" button from repeatedly mounting and unmounting inside the bottom threshold.
- The user confirms the button now behaves correctly, but scrolling still feels uneven or "sticky".
- `src/chat/MessageList.tsx:3` uses `virtua`; its scroll callback at `src/chat/MessageList.tsx:411` performs bottom geometry work, three `findItemIndex` lookups, navigator calculations, and possible React state updates.
- Virtualized rows mount and unmount as the rendered range changes. Historical assistant bubbles use `chat-motion-fade-up`, so a remounted row can replay an entrance animation and visually resemble a refresh.
- Commit `e94c42d` previously removed virtualization because mounting KaTeX-rich DOM during scrolling caused style recalculation and stutter; commit `c4ed1c4` later reintroduced `virtua`. This is evidence to test, not a conclusion that virtualization must be removed.
- Heavy Markdown, KaTeX, tables, images, reasoning blocks, and a small virtual-list buffer may amplify mount and measurement costs.

## Requirements

- Add temporary development-only performance probes with no production runtime overhead.
- Record enough evidence to distinguish among:
  - high-frequency scroll-handler JavaScript work;
  - React commits caused by scroll-derived state;
  - `virtua` measurement/range corrections;
  - message-row mount/unmount churn;
  - browser long tasks, dropped/late animation frames, layout, and paint pressure;
  - expensive content types such as Markdown, KaTeX, tables, images, and reasoning sections.
- Reproduce the issue with a deterministic long-conversation fixture and real wheel/trackpad-style deltas.
- Compare the same scenario with one variable changed at a time: historical-row animation, virtual-list buffer, navigator work, and content complexity.
- Preserve current behavior for automatic bottom following, the bottom button, message navigation, streaming content, and conversation switching.
- Use probe results to identify the dominant cause before changing performance-sensitive behavior.
- Add or update regression coverage for any behavior changed by the eventual fix.
- Remove every probe, temporary fixture hook, global diagnostic object, and diagnostic log before the final commit.

## Acceptance Criteria

- [x] A repeatable test captures scroll-event timing, frame gaps, relevant React commits, virtual-row churn, and long-task evidence.
- [x] Probe output identifies the dominant source(s) of the residual stutter with concrete measurements.
- [x] The chosen fix removes the measured hot path or churn without breaking bottom-follow or navigator behavior.
- [x] Re-running the identical test shows a material improvement in the metric associated with the root cause (target: at least 30% fewer late frames or elimination of the identified redundant animation/commit churn).
- [x] Historical messages do not visually replay an entrance/refresh effect merely because virtualization remounts them.
- [x] The bottom button remains stable around the bottom threshold and navigator state remains correct while scrolling.
- [x] All temporary performance probes and fixture hooks are removed after diagnosis and verification; a repository search confirms they are absent from final product code.
- [x] Type-check, lint, targeted tests, production build, and the full compatible-Node test suite pass.

## Out of Scope

- Replacing the entire chat rendering architecture without evidence that `virtua` itself is the root cause.
- Backend message loading or persistence changes unless probes show an unexpected backend reload during scrolling.
- Keeping a permanent performance dashboard, logging system, or hidden diagnostics mode.
