# Kivio Study: design evidence, implementation, and limits

Date: 2026-10-06. This note records the first Study implementation and the evidence behind its design. It does not introduce a second engineering standard; [the unified engineering rules](../engineering-standards.md) remain authoritative.

## Current architecture: reuse the existing Chat flow (2026-10-07)

The user's later direction is explicit: reuse the existing chat rather than maintain another conversation implementation. This section supersedes the original isolated-IPC architecture below.

`Study material/page/region → Chat.tsx → ChatConversationPane / MessageList / InputBar → existing send controller / execution owner → chat_send_message → shared reply preparation / run_agent_loop → normal conversation repository and protocol`

- `Chat.tsx` remains the single mounted conversation/execution/event owner. Normal Chat and Study use one keep-alive conversation slot, preventing competing composer listeners. Shared components expose a compact reading presentation; they retain the actual editor, draft owner, streaming, cancellation, error and regeneration paths. Ordinary Chat and sub-agent presentation keep their defaults.
- Study owns material blobs, page/region, notes and restart-persistent source drafts only. Its former question/answer renderer, request/stream state, provider transport, two Tauri commands, cancellation map and response loop are removed. A small bridge saves the page PNG through the existing Chat attachment API, then passes immutable typed source metadata into the ordinary send controller. It never calls a model itself.
- Every page has a durable source-bound normal Chat conversation. Each user message stores its own page/crop/mode/attempt metadata and original rendered image attachment. Regeneration uses that saved image and source, not the reader's newer selection. The existing optimistic user also carries the captured source so page labels and full-solution disclosure remain correct while streaming.
- The shared backend derives reading instructions from typed source metadata. Its source-only preparation skips global memory, assistant/set/knowledge/project prompts, tools, hooks, slash-command interpretation, auxiliary vision substitution and built-in search. It then converges into the same Chat runtime and persistence, rather than adding a Study loop. Repository invariants block converting a source-bound conversation to another runtime or tool-bearing profile. Side-model work stays with the selected model; provider-facing cache/session identifiers do not expose material hashes.
- Legacy migration is lazy and idempotent per material/page. Normal repository creation is committed first, then a separate IndexedDB receipt records the link. Retries after interrupted migration cannot duplicate or overwrite the conversation. Original legacy records remain unchanged in a read-only backup, including previously withheld answers and old manually added text. Partial/error turns, times, modes and model identity are retained. Old records did not store image bytes, so their retry is explicitly unavailable until the reader recaptures the source as a new question. A deleted linked conversation is not silently resurrected from backup.
- Shared composer drafts remain authoritative while mounted; a scoped adapter persists their input through the material owner's existing save/CAS path across app restart. Bindings survive page switches so delayed acceptance/restoration updates the original page. Explicit confirmed saved-version recovery rehydrates the shared composer, rather than leaving stale editor text behind. Ordinary Chat drafts are untouched.
- Opening a source-bound conversation from ordinary Chat routes back to its material/page; unavailable material leaves a safe shared transcript and an explicit recovery entry rather than enabling ordinary agent controls. Removing a material retains already-migrated Chat conversations and describes that distinction before confirmation.

The source UI still sends original page/crop images, performs no OCR or PDF text extraction, and preserves the simple Reading Q&A default. The earlier two free vision probes tested the pre-refactor prompt/image shape at `dd6897b7`; they are historical feasibility evidence, **not** a live test of the new native shared-Chat path. Current acceptance must separately exercise ordinary Chat and the integrated reader with clearly simulated provider responses, and compile/test the exact published native head.

## Current flow: direct images, no extraction (2026-10-07)

The latest user direction supersedes the extraction/correction and structured-output experiments recorded below: **send the original page/selected-region image plus the learner’s question and attempt directly to the model; keep the interaction simple.**

- PDF.js renders page pixels only. Study does not call PDF text-layer extraction or an OCR engine, and never reconstructs formula text automatically. Imported images use their displayed pixels. The preview is exactly the bounded raster page/crop attached to the request; rendering/downscaling limits remain explicit.
- A configured image-capable model and a ready image are required. There is no checkbox or silent text-only fallback, no formula-confirmation gate, and no mandatory transcription/correction field. Questions, attempts and notes remain learner-authored.
- Legacy manually added text and historical source text remain locally readable for compatibility, but are not copied into a new request or retry as material. A retry keeps the original question/attempt/page/region and sends that source image.
- Ordinary reading, hint, explanation and check responses stream directly. The strict JSON response envelope/parser and malformed-response disclosure workflow were removed. A small read-only compatibility adapter keeps previously saved structured replies readable and their previously withheld solutions hidden; it imposes no format on new model replies. Optional math-learning instructions remain alongside the default Reading Q&A mode; explicitly chosen full solutions retain their reveal control. Model correctness and hint restraint are still not guaranteed.
- The earlier live probes were text-only historical experiments, not evidence for this image-only flow or live image interpretation. Current browser acceptance checks the exact image payload and direct streaming with a clearly labelled simulated provider; native tests verify image/capability rejection and transport assembly.

## Reading papers and English articles (2026-10-07)

The later product request broadens Study from exercises to reading and learning. The minimum change is one general **Reading Q&A** default in the existing compact menu, rather than separate translation, paper and figure panels. The empty state and question prompt describe reading; the optional expandable field says “Additional context” in reading mode. Hint, concept explanation, checking an attempt and explicit full solutions remain available for exercises. Existing saved modes, drafts and history remain valid.

Evidence and applied product inferences:

- [Keshav, *How to Read a Paper*](https://web.stanford.edu/class/cs114/reading-keshav.pdf) describes staged, goal-directed reading. [Carey, Steiner & Petri, 2020](https://journals.plos.org/ploscompbiol/article?id=10.1371/journal.pcbi.1008032) recommends choosing a reading goal, separating motivation/methods/data/interpretation, and unpacking figure axes, legends and methods. These are reading guides, not controlled evidence that this UI improves learning. **Inference:** let the reader ask about the passage or figure they are viewing, without a prescribed task sequence.
- [Amano et al., 2023](https://journals.plos.org/plosbiology/article?id=10.1371/journal.pbio.3002184) surveyed 908 environmental-science researchers and documented additional English-related work for non-native speakers. That population does not establish a universal effect size across disciplines. **Inference:** useful reading help includes faithful translation and technical-term explanations while retaining numerals, units and hedging.
- Scope is explicit: the model receives the current original page or selected region, not all pages in the document. The prompt separates visible source claims from explanation/inference, asks for missing or unreadable context, and forbids invented authors, DOIs, page numbers, p-values or causal conclusions from a chart alone. The deterministic source link returns to the actual saved PDF page/region; generated prose is not a verified citation system. These instructions cannot guarantee model compliance.

Real-material acceptance uses Amano et al.'s openly available 27-page CC BY [original PDF](https://journals.plos.org/plosbiology/article/file?id=10.1371/journal.pbio.3002184&type=printable), SHA-256 `0012515e110c81d8fe0a739568c0c20c1521938595851e2b8acfe22be7ce2b8a`. The PDF is fetched during tests, not committed or bundled. Page 1 tests abstract translation/terms and an over-broad whole-paper request. Page 4 tests Figure 1 with axes/legend/full caption, an incomplete crop, source restoration, notes and narrow-layout navigation. The harness independently compares the sent PNG against the rendered page/crop; fixed replies remain visibly simulated and do not prove translation or figure comprehension. Existing real MIT exercise journeys remain regressions. No document-wide index, OCR, citation manager or extra persistent reading panel is added.

### Bounded live image probes

Two anonymous direct-API calls used the current `buildStudyPrompt`, original CC BY abstract/Figure 1 crops, and OpenCode Zen's documented free image-capable `space-bunny-free` model. Both returned HTTP 200, a completed response and reported cost 0; no credentials, retries or settings changes were used. Official [free-model documentation](https://opencode.ai/docs/zen/), [image/cost metadata](https://github.com/anomalyco/models.dev/blob/dev/providers/opencode/models/space-bunny-free.toml), and [terms applying through use](https://opencode.ai/legal/terms-of-service/) were checked. This was direct API prompt/image validation, not the native app's IPC-to-model end-to-end flow.

Observed useful behavior: the abstract translation retained 908 and the environmental-science scope; the figure explanation identified the log10 publication-count axis, reading-time minutes, language/income encodings, 95% confidence bands and limits on causal inference. Observed failures are retained: the abstract response added self-assessment methodology not visible in its crop; the figure response changed “most recently read” into “recently published”; both omitted the requested page/region label. The application source anchor still identifies the actual supplied image. No further model tuning or calls were made. These two examples support limited feasibility only, not general translation accuracy, complete grounding or reliable citation compliance.

## Progressive disclosure after UI audit (2026-10-07)

The later UI review asked for a simpler, faster student workflow. The default composer now shows the question and Send, with compact mode/model menu triggers; attempts open on demand and automatically when checking work. Collapsing an attempt preserves its draft. A single material defaults to a collapsed library with a restore action; multiple materials keep the library available. Reader image details are collapsed rather than filling the page with rendering diagnostics and a second large crop. An always-reachable page/selection Ask action moves to the focused question, and Help retains a small source-context link/thumbnail. Ctrl/Meta+Enter uses the same send validation, and shared Select menus support keyboard focus, navigation and dismissal. These are UI disclosure changes; image-only requests, ordinary streaming, local persistence and retry identity are unchanged.

## Product scope

Study adds a material-centered workspace inside the existing Chat application. A learner imports a local PDF, PNG, JPEG, or WebP, reads a page, optionally selects a region, asks for help, and keeps page-bound drafts, attempts, notes, and response history. Reading and local notes do not require a model. Model answers use a provider/model already configured in Kivio. Browser-only preview reports that real answers require the desktop app; it does not substitute canned tutoring responses.

The current modes deliberately have different prompts:

- **Reading Q&A**, the default: answer questions about visible passages, terms, translation, arguments, methods, figures and tables directly, without requiring an attempt. Optional learner context stays in the existing expandable field.
- **One hint**, the original default and now an optional math mode: one actionable next step or question, without the final answer or a full derivation.
- **Explain**: explain the underlying concept, optionally with a separate small example, without solving the selected exercise.
- **Check my attempt**: require the learner's attempt; identify correct work and the first unsupported/incorrect step, then suggest one next step.
- **Full solution**: an explicit selection and send action requests a worked solution. The answer is presented behind a reveal control.

These are model instructions and interaction choices, not guarantees that every provider will obey the teaching policy or produce a correct answer. The UI warns that AI can make mistakes.

## Primary evidence and applied inferences

### Retrieval practice

Karpicke and Blunt compared learning activities using educational science texts. In their experiments, retrieval practice improved later performance, including inference questions, relative to the studied elaborative concept-mapping condition. The first experiment's delayed test was one week later. This supports taking active learner production seriously; it does not establish an effect size for a document-based AI tutor. [Karpicke & Blunt, 2011, author-hosted paper at Purdue](https://learninglab.psych.purdue.edu/downloads/2011/2011_Karpicke_Blunt_Science.pdf), DOI: 10.1126/science.1199327.

**Applied inference:** encourage an attempt and a learner-authored note before consuming more explanation. Study is not an implementation of the paper's experimental procedure: the source remains visible, and there is no scheduled retrieval or delayed mastery assessment. No retention or grade improvement is claimed.

### Self-explanation

Chi, De Leeuw, Chiu, and Lavancher studied prompted self-explanation while students read a passage about the circulatory system. Their reported results support the possibility that actively explaining material helps build understanding, within that study's population and task. [Chi et al., 1994, publisher abstract](https://onlinelibrary.wiley.com/doi/abs/10.1207/s15516709cog1803_3).

**Applied inference:** make the learner's reasoning visible through the attempt field and page notes; ask the model to respond to the first problematic step rather than overwrite the work. This interface has not itself been evaluated for learning outcomes.

### A current tutoring-product reference

Khan Academy describes Khanmigo as guiding learners toward answers instead of immediately supplying them. That is a relevant product example of a tutoring-oriented interaction. It is not independent evidence that Kivio's implementation improves learning, and Study does not inherit Khanmigo's curriculum integration, moderation, supervision, or age-related safeguards. [Khanmigo official product page](https://khanmigo.ai/), read 2026-10-06.

**Applied inference:** keep a small-hint route distinct from an intentional full-solution route. Full solutions remain available because the learner may be reviewing a worked example or checking finished work. No claim is made that this exact four-mode split is experimentally optimal.

## Original architecture decision (historical)

The first revision used a separate Study workspace request owner, frontend Tauri channel and backend command over shared provider adapters. It reused Markdown and basic controls, but duplicated conversation orchestration, persistence and sidebar UI. Page-bound source isolation was the motivation; it did not require a permanently separate chat implementation. The user requested direct reuse, and that original request path has been removed in favor of the shared architecture above.

## Provider isolation and request identity

The normal Chat reservation/execution/protocol owners now govern concurrency, cancellation, partial responses, retry and late results. A source snapshot is captured before asynchronous attachment storage, and shared send receives an explicit target conversation. Navigating cannot retarget a prepared source or an in-flight answer. Backend validation requires the durable source binding, original image and confirmed vision capability before a request or destructive retry mutation.

Provider adapters, credential/OAuth resolution, accounting and request-debug behavior are the existing implementations. Study does not create credentials or modify settings. Reserved extra-body overrides cannot replace source context/model or enable tools. Source-bound preparation disables tools/search, memory, unrelated prompts and lifecycle hooks. Configured provider charges can still apply; the product has no automatic free-model substitution.

## Current source limits

- Import is local and limited to 25 MB per material. PDFs are limited to 1,000 pages; encrypted PDFs are rejected with an actionable message. Image dimensions are checked before decoding.
- PDF rendering uses a bundled worker and local character-map/font assets. Imported documents do not supply remote asset URLs. Rendering and context images have separate size bounds.
- **There is no OCR or PDF text extraction in Study.** Scanned pages, digital PDFs and imported images all use original rendered pixels. There is no reconstructed formula or required correction workflow. The model can still misread small symbols; the reader can select a clearer region or ask about a larger image when context is missing.
- The backend independently requires confirmed vision capability before sending an image. The reader emits a bounded PNG even for an imported WebP; the backend accepts validated PNG/JPEG data URLs and reuses existing model image preparation.
- The normal Chat context/replay/compaction owner now bounds conversation history. Questions and attempts retain source-specific input limits; there is no extracted page-text payload. Each historical user turn retains its own source label instead of being relabeled with the latest selection.
- The model sees only supplied page/region context and same-page history. It is instructed to ask for missing or unreadable details rather than pretend to see other pages.

## Privacy and untrusted output

Materials, source drafts and notes remain in local IndexedDB; new histories and original image attachments use normal local Chat persistence; this is not encrypted storage or a cloud backup. Clearing application data removes the stored work. Removing a material does not delete the original imported file.

On Send, the selected provider receives the question, attempt, the page conversation within normal Chat context bounds, actual PDF page index, page/region scope, and the disclosed original page/region image. Original filenames and document hashes are omitted from the provider prompt. Original PDF bytes and unselected PDF pages are not uploaded. Sending a whole-page or selected-region image shares that rendered image. The provider's own retention and usage policies still apply, and configured provider charges may apply.

Retry preserves the original question, optional context/attempt, mode, page and region. It always uses that original source image and requires a confirmed vision model. Historical extracted or manually corrected source text is not sent, including on retries.

Existing Kivio provider accounting continues to run. If the user has enabled **Request Debug**, the reused provider adapters can capture request bodies and response details in the existing local bounded buffer and its disk mirror. Study does not switch this setting on or off. The UI discloses this conditional behavior. Removing a Study material does not implicitly clear those separate diagnostics.

Document text, filenames, images, and prior messages are treated as untrusted source content in the prompt. The teaching-mode system instructions are not assembled from document text.

The shared Chat message components use `ChatMarkdown`'s explicit safe `readOnly` content profile in reading presentation (distinct from existing message-action read-only behavior). It keeps mathematical and text formatting, but renders links as text, omits media embeddings, escapes raw HTML before HTML parsing, and avoids HTML/SVG/Mermaid previews, artifact loading, and local-file controls. This closes an otherwise unintended disclosure path in which model-generated image URLs could issue network requests. Normal Chat rendering retains its existing interactive behavior.

## Verification and release boundaries

The implementation has focused mocked-transport tests for mode instructions, required attempts, source bounds, original identity after navigation, provider errors/retry, pre-start cancellation, retry attempt separation, late-delivery isolation, and browser-preview refusal. These tests exercise lifecycle logic but do not prove tutoring quality or contact a paid provider.

The shared read-only renderer has regressions for preserved math/text/tables/code, remote/local/data/artifact images, HTML and code previews, malicious math links, profile changes with unchanged content, and streaming partial syntax. Existing interactive Markdown and Chat protocol regressions are also run to detect unintended changes.

Six backend tests cover tool-free request assembly, configured model/vision checks, reserved override rejection, bounded input, validated in-memory image parts, and cancellation/cleanup scoping. Source review checked the existing provider signatures, Rust visibility, request fields, serde naming, Tauri channel use, and the locked image API. Source review is not a substitute for compilation.

**Local verification limits:** this cloud environment has no `cargo`/`rustc`; Rust compilation and backend tests were not run locally. Local Chromium browser execution was blocked by the environment's socket restrictions. No paid live-model call, signed desktop package run, or real macOS/Windows interaction has been performed here.

Before calling the change ready, inspect the macOS CI result for the **exact published commit**, including `cargo test`, the generated protocol check/typecheck, frontend tests, and applicable browser tests. A pending CI job is not a pass. Desktop validation should cover import/selection, scrolling and keyboard access, theme changes, provider failure, Stop, retry, navigating during a response, reopening persisted work, and save failure. Live model quality, image interpretation, and provider billing behavior remain separate authorized manual checks.

## Historical calculus experiments (2026-10-07)

This section preserves the observations and failed model probes from earlier revisions. The extraction/correction gate and structured-response implementation described here were subsequently removed at the user’s request; the current image-only flow above is authoritative. The real MIT browser journeys remain, adapted to current behavior.

Used the actual public [MIT OCW 18.01SC Integration Techniques problem PDF](https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/50d9ff5b7a30fe96bd69017ca5104d6e_MIT18_01SC_pset5prb.pdf) and its [official solutions](https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/06979381db650b91c0de9d6755f03154_MIT18_01SC_pset5sol.pdf), Arthur Mattuck, Fall 2010. The PDF is fetched from the official URL during acceptance testing, SHA-256 `5015876951651a4f4695ccbbb74e22fdf1a43b1798d4859ed3324cdc9e90d23d`; it is not checked into this repository or bundled with the product. See [MIT source terms](https://ocw.mit.edu/pages/privacy-and-terms-of-use/).

Two independently verified cases:
- 5B-13, problem page 2 / solution page 6: integral of x²/(1+x⁶). A fabricated attempt drops the factor 1/3 while substituting u=x³. Differentiation checks the official antiderivative arctan(x³)/3 + C.
- 5F-2(a), problem page 5 / solution page 18: integral of x eˣ. A fabricated attempt uses a plus instead of a minus in integration by parts. Differentiation checks (x−1)eˣ + C and rejects (x+1)eˣ + C.

The first actual browser run exposed a scrolled-page focus bug: starting a region drag moved the PDF before its coordinates were calculated. `focus({preventScroll:true})`, a failing-before/passing-after unit regression, and the real-PDF browser journey fix this. The original PDF image renders its fractions/exponents; extracted text contains unmapped glyphs and does not reliably preserve mathematical layout. Empty/unmapped PDF extraction now prompts correction or an included page/region image; selected-image preview uses exactly the context image, not a second rendering. This is observable damage detection, not OCR, math recognition or certification of apparently clean text.

A small anonymous direct-API probe used the actual Study prompt builder and OpenCode's documented free `space-bunny-free` model at the existing Zen endpoint. Only the public problems and fabricated attempts were sent, without credentials or settings changes. [Free service documentation](https://opencode.ai/docs/zen/) and [terms applying through use](https://opencode.ai/legal/terms-of-service/). These were direct API calls, not native Tauri IPC or browser-to-model end-to-end tests.

Observed limitations were retained rather than discarded: the original check first endorsed a wrong answer and later contradicted itself. A single verification-before-verdict retest corrected the math but supplied the full solution. The revised structure therefore has an application-side gate, not just a stronger prompt: hint/check streams are not displayed raw; a closed versioned schema is validated only after completion; valid first-step feedback is separated from verification and optional extra solutions. Optional solutions, malformed responses, and interrupted partial responses are mounted only after an explicit reveal action. Old saved plain-text hint/check replies receive the same conservative treatment. Explain/full-solution modes remain distinct.

A bounded three-call structured retest then checked the first hint and both distinct wrong attempts. The hint met the schema. Both checks diagnosed the mathematics correctly without a new final answer but violated the exact field schema (`firstStep` instead of, or in addition to, `firstIssue`); both correctly fell back to explicit reveal. No further tuning or calls were made. This is evidence of safe fallback and remaining provider compatibility limits, **not** full live-tutoring acceptance or general accuracy. A valid schema cannot prove that text within an allowed field is mathematically true or free of spoilers. The UI states that limitation and never awards an automatic correctness/grade badge.

Deterministic browser fixtures exercise display and persistence using visibly simulated responses. They do not establish model teaching quality. Regression coverage retains the observed contradictory/full-solution prose, malformed/duplicate/unknown fields, incomplete streaming, explicit disclosure/re-hiding/reload, exact source-image matching, damaged-text correction, and the two real PDF exercises.
