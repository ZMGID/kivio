# Chat scroll stutter implementation plan

## Ordered checklist

1. Activate the task and load the project frontend specifications with `trellis-before-dev`.
2. Inspect the exact virtualizer, bubble animation, Markdown/KaTeX, and existing test boundaries using CodeGraph and targeted source reads.
3. Add the temporary development-only performance collector and minimal instrumentation described in `design.md`.
4. Create or reuse a deterministic long-conversation browser fixture with realistic small wheel deltas.
5. Capture the baseline: frame gaps, long tasks, scroll-handler timing, React commits, row churn, navigator updates, and active animations.
6. Run controlled variants one at a time:
   - historical-row animation disabled;
   - larger virtualizer buffer;
   - navigator scroll work bypassed/coalesced;
   - plain content versus Markdown/KaTeX-heavy content if still needed.
7. Record the measurements under this task and select the smallest evidence-backed production fix.
8. Implement the fix and add targeted regression coverage for the changed behavior.
9. Re-run the identical browser scenario and compare it with the baseline.
10. Delete all temporary probes, profiling wrappers, fixture hooks, diagnostic globals, and diagnostic logs.
11. Search the repository for probe identifiers to verify cleanup.
12. Run the Trellis quality check and all validation commands.
13. Review whether the root-cause class should be captured in project specs, then commit the final code and task artifacts.

## Validation commands

Use Node `24.15.0`; Node 26 has unrelated `window.localStorage` failures in this repository.

```bash
eval "$(fnm env)"
fnm use 24.15.0 --silent-if-unchanged
npm run typecheck
npm run lint
npm run build:ui
npm test
```

Also run the targeted chat tests and the deterministic browser profiling scenario before and after the fix.

## Risky files and rollback points

- `src/chat/MessageList.tsx`: changes can affect bottom following, scroll-to-index, navigator accuracy, streaming alignment, and virtualizer measurement.
- `src/chat/MessageBubble.tsx` and `src/index.css`: animation changes can affect genuine new-message transitions as well as remounts.
- Markdown/KaTeX components: avoid edits unless measurements show content mounting remains dominant.

Rollback candidate changes independently in reverse order. Preserve commit `d4fc977` bottom-threshold behavior throughout.

## Start gate

Planning is ready when `prd.md`, `design.md`, and `implement.md` are present and converged. The user has explicitly instructed the work to start without additional process questions, so proceed to `task.py start` once these artifacts validate.
