# Scroll stutter measurements

## Scenario

- Chromium through Playwright, 1280×720 development UI.
- Deterministic mock conversation: 70 user/assistant turns (140 messages).
- Assistant rows include reasoning, Markdown, GFM tables, code blocks, and KaTeX.
- Each run starts at the bottom and sends 110 upward wheel deltas of 36px with 12ms spacing.
- Temporary probes measured RAF gaps, long tasks, scroll/navigator time, React commits, row mounts, virtual range changes, and state updates. All probes and fixture hooks were removed after verification.

## Baseline

Three baseline runs were consistent:

- 110 scroll events per run.
- Scroll callback total: 2.8–4.6ms; maximum individual callback: 0.2–0.3ms.
- Navigator calculation total: 2.1–3.4ms.
- React commits: 117; total actual duration: 67.7–85.7ms; maximum commit: 7.4–12.8ms.
- Message mounts/unmounts: 24/23; 12 mounted rows contained heavy content.
- Historical entrance animations replayed: 24 per run.
- Virtual rendered range changed 24 times and held only 1–3 rows because the rows were tall.
- No browser long tasks and no RAF gaps above 25ms were observed in this controlled run.

Direct animation sampling showed old user and assistant rows starting new `chat-motion-fade-up` animations as they re-entered the virtual range. The animation remained active for roughly 200ms and applied both opacity and upward translation, matching the reported "refresh" and sticky visual movement.

## Controlled variants

### Historical animation disabled

- Historical entrance animations: 24 → 0.
- Directly sampled active historical `chat-motion-fade-up` animations: maximum 2 → 0.
- Row churn, navigator updates, and bottom-state updates remained unchanged.

### Navigator disabled

- Scroll callback total fell from roughly 3–5ms to 0.6ms across 110 events.
- React commits remained 117 and row mounts/unmounts remained 24/23.
- Historical entrance animations remained 24.

Conclusion: navigator work is measurable but too small to explain the perceived stutter.

### Virtualizer buffer increased from 200px to 800px

- Rendered rows increased from 1–3 to 4–8.
- Virtual range changes fell from 24 to 15.
- Mounts increased from 24 to 30 and entrance animations increased from 24 to 30 because more heavy rows were prepared at once.
- React commit count/duration did not materially improve.

Conclusion: a larger buffer trades fewer range changes for more simultaneous heavy mounts and does not address the refresh effect.

## Root cause and fix

The dominant user-visible cause was unconditional `chat-motion-fade-up` on every `MessageBubble`. `virtua` correctly unmounts offscreen rows; when an old row returns, React creates its DOM again and CSS treats it as a brand-new message, replaying opacity `0 → 1` and `translateY(8px) → 0`.

The production fix limits the bubble entrance animation to `messageStreaming === true`. Historical user and assistant messages no longer replay mount motion, while the live streaming assistant preview keeps its intended entrance animation.

## Post-cleanup verification

After removing every probe and fixture hook, the same browser flow reported:

- maximum active historical entrance animations while scrolling: 0;
- bottom button visible after scrolling upward: yes;
- bottom distance after clicking the button: 1px (within the existing threshold).

Targeted tests also cover historical user/assistant rows without the class and the streaming preview with the class.
