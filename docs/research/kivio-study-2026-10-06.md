# Kivio Study: design evidence, implementation, and limits

Date: 2026-10-06. This note records the first Study implementation and the evidence behind its design. It does not introduce a second engineering standard; [the unified engineering rules](../engineering-standards.md) remain authoritative.

## Current flow: direct images, no extraction (2026-10-07)

The latest user direction supersedes the extraction/correction and structured-output experiments recorded below: **send the original page/selected-region image plus the learner’s question and attempt directly to the model; keep the interaction simple.**

- PDF.js renders page pixels only. Study does not call PDF text-layer extraction or an OCR engine, and never reconstructs formula text automatically. Imported images use their displayed pixels. The preview is exactly the bounded raster page/crop attached to the request; rendering/downscaling limits remain explicit.
- A configured image-capable model and a ready image are required. There is no checkbox or silent text-only fallback, no formula-confirmation gate, and no mandatory transcription/correction field. Questions, attempts and notes remain learner-authored.
- Legacy manually added text and historical source text remain locally readable for compatibility, but are not copied into a new request or retry as material. A retry keeps the original question/attempt/page/region and sends that source image.
- Ordinary hint, explanation and check responses stream directly. The strict JSON response envelope/parser and malformed-response disclosure workflow were removed. A small read-only compatibility adapter keeps previously saved structured replies readable and their previously withheld solutions hidden; it imposes no format on new model replies. The four simple learning-mode instructions remain; explicitly chosen full solutions retain their reveal control. Model correctness and hint restraint are still not guaranteed.
- The earlier live probes were text-only historical experiments, not evidence for this image-only flow or live image interpretation. Current browser acceptance checks the exact image payload and direct streaming with a clearly labelled simulated provider; native tests verify image/capability rejection and transport assembly.

## Product scope

Study adds a material-centered workspace inside the existing Chat application. A learner imports a local PDF, PNG, JPEG, or WebP, reads a page, optionally selects a region, asks for help, and keeps page-bound drafts, attempts, notes, and response history. Reading and local notes do not require a model. Model answers use a provider/model already configured in Kivio. Browser-only preview reports that real answers require the desktop app; it does not substitute canned tutoring responses.

The four modes deliberately have different prompts:

- **One hint**, the initial default: one actionable next step or question, without the final answer or a full derivation.
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

## Short implementation path and ownership

A send follows:

`StudyWorkspace UI → studyWorkspaceStore.sendStudyHelp → studyRequest.requestStudyHelp → api/study IPC → chat/study.rs → existing stream_with_chat_provider`

- `src/chat/study/studyWorkspaceStore.ts` owns page drafts, request identity, in-progress results, original-document updates, and persistence coordination. Navigation does not retarget a running response.
- `studyStorage.ts` owns the IndexedDB schema and storage operations. Materials are identified by a SHA-256 content hash; questions and notes are bound to that document and page. Invalid records and save failures are surfaced rather than silently replaced with a successful-save claim. A persisted revision is checked atomically before every document save so a stale window cannot overwrite newer notes or replies. Conflict recovery keeps local drafts available to copy, then offers an explicitly confirmed reload of the saved version.
- `studyMaterial.ts` owns file validation, local PDF/image loading, bounded rendering, text extraction, and cropped context images. `StudyReader` binds those operations to navigation and selection.
- `studyRequest.ts` owns the teaching instructions, context assembly, and a synchronous snapshot of the input before asynchronous work. It supplies a bounded tail of the current page's discussion.
- `src/api/study.ts` owns the request-local channel and cancellation lifecycle; it contains no tutoring policy or scripted answer.
- `src-tauri/src/chat/study.rs` owns backend input/model checks, in-memory image conversion, request cancellation, and direct use of the existing provider adapters.

There was no prior Study flow to migrate or delete. Reusing the normal Chat conversation path was considered, but that path can discover tools and include memory even in the Chat runtime. Reusing the Lens global stream would couple Study cancellation and delivery to a separate feature. The small Study IPC boundary is justified by its isolated request lifecycle and real external-model seam; it does not duplicate provider-specific HTTP, credentials, OAuth resolution, or streaming protocol parsing.

No imported CLI conversation, native session, or working directory is changed. The existing imported-session ADRs remain intact.

## Provider isolation and request identity

- The backend resolves an enabled saved provider and enabled selected model. Credentials stay within the existing backend provider mechanism; Study does not accept frontend API keys or create new credentials.
- Requests contain no agent tools, no built-in web search, no attached knowledge base, and no Chat memory. Study does not create an ordinary Chat conversation or modify provider settings.
- Reserved model `extraBody` keys that could replace context/model, enable tools, or override streaming are rejected for Study. Harmless vendor-specific settings remain available. This does not assert control over a provider's undisclosed server-side behavior.
- A dedicated Tauri `Channel` delivers only the correlated request's text. A fresh transport UUID distinguishes every attempt, including retries of the same logical question.
- Backend active requests are scoped by window label and transport UUID. A started acknowledgement prevents Stop from being lost before registration; a guard removes request state on every exit. Cancellation drops the pending provider future, and the request also has a 180-second timeout.
- The frontend stops applying deltas immediately on abort and rejects with `AbortError`. The workspace keeps partial output and the original question/source for retry.
- Study adds no application-level automatic model retry. It uses the existing configured provider retry/failover mechanism, with the attempt count bounded to 1–3 for this call. A user-initiated retry is a new invocation and can incur another provider charge.

## Source limits and honest extraction behavior

- Import is local and limited to 25 MB per material. PDFs are limited to 1,000 pages; encrypted PDFs are rejected with an actionable message. Image dimensions are checked before decoding.
- PDF rendering uses a bundled worker and local character-map/font assets. Imported documents do not supply remote asset URLs. Rendering and context images have separate size bounds.
- PDF text extraction is best effort. Equations, reading order, text-span boundaries, and region intersections can be inaccurate. The visible reader pipeline limits source text to 16,000 characters and reports truncation.
- **There is no OCR implementation.** A scanned page or imported image may have no extracted text. The learner can paste/correct the problem text or explicitly send a page/region image to a vision-capable model. Model image interpretation is not presented as verified OCR.
- The backend independently requires confirmed vision capability before sending an image. The reader emits a bounded PNG even for an imported WebP; the backend accepts validated PNG/JPEG data URLs and reuses existing model image preparation.
- The prompt assembler has a defensive 40,000-character page-text cap and at most 12 history messages, each at most 4,000 characters. These are additional transport bounds, not a claim that every model's context window can hold the maximum.
- The model sees only supplied page/region context and same-page history. It is instructed to ask for missing or unreadable details rather than pretend to see other pages.

## Privacy and untrusted output

Materials, drafts, notes, and history are kept in local application storage; this is not encrypted storage or a cloud backup. Clearing application data removes the stored work. Removing a material does not delete the original imported file.

On Send, the selected provider receives the question, attempt, bounded page history, page number, supplied text, and any disclosed page/region image. Original filenames and document hashes are omitted from the provider prompt. Original PDF bytes and unselected PDF pages are not uploaded. Sending a whole-page or selected-region image shares that rendered image. The provider's own retention and usage policies still apply, and configured provider charges may apply.

Retry preserves the original question, attempt, source text, and image-inclusion decision. Its disclosure must describe that saved decision rather than a stale checkbox from a different draft. A confirmed vision model is still required when the original question included an image.

Existing Kivio provider accounting continues to run. If the user has enabled **Request Debug**, the reused provider adapters can capture request bodies and response details in the existing local bounded buffer and its disk mirror. Study does not switch this setting on or off. The UI discloses this conditional behavior. Removing a Study material does not implicitly clear those separate diagnostics.

Document text, filenames, images, and prior messages are treated as untrusted source content in the prompt. The teaching-mode system instructions are not assembled from document text.

Study uses the shared `ChatMarkdown` renderer's explicit `readOnly` profile. It keeps mathematical and text formatting, but renders links as text, omits media embeddings, escapes raw HTML before HTML parsing, and avoids HTML/SVG/Mermaid previews, artifact loading, and local-file controls. This closes an otherwise unintended disclosure path in which model-generated image URLs could issue network requests. Normal Chat rendering retains its existing interactive behavior.

## Verification and release boundaries

The implementation has focused mocked-transport tests for mode instructions, required attempts, source bounds, original identity after navigation, provider errors/retry, pre-start cancellation, retry attempt separation, late-delivery isolation, and browser-preview refusal. These tests exercise lifecycle logic but do not prove tutoring quality or contact a paid provider.

The shared read-only renderer has regressions for preserved math/text/tables/code, remote/local/data/artifact images, HTML and code previews, malicious math links, profile changes with unchanged content, and streaming partial syntax. Existing interactive Markdown and Chat protocol regressions are also run to detect unintended changes.

Six backend tests cover tool-free request assembly, configured model/vision checks, reserved override rejection, bounded input, validated in-memory image parts, and cancellation/cleanup scoping. Source review checked the existing provider signatures, Rust visibility, request fields, serde naming, Tauri channel use, and the locked image API. Source review is not a substitute for compilation.

**Local verification limits:** this cloud environment has no `cargo`/`rustc`; Rust compilation and backend tests were not run locally. Local Chromium browser execution was blocked by the environment's socket restrictions. No paid live-model call, signed desktop package run, or real macOS/Windows interaction has been performed here.

Before calling the change ready, inspect the macOS CI result for the **exact published commit**, including `cargo test`, the generated protocol check/typecheck, frontend tests, and applicable browser tests. A pending CI job is not a pass. Desktop validation should cover import/selection, scrolling and keyboard access, theme changes, provider failure, Stop, retry, navigating during a response, reopening persisted work, and save failure. Live model quality, image interpretation, and provider billing behavior remain separate authorized manual checks.

## 2026-10-07: real calculus acceptance and safeguards

Used the actual public [MIT OCW 18.01SC Integration Techniques problem PDF](https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/50d9ff5b7a30fe96bd69017ca5104d6e_MIT18_01SC_pset5prb.pdf) and its [official solutions](https://ocw.mit.edu/courses/18-01sc-single-variable-calculus-fall-2010/06979381db650b91c0de9d6755f03154_MIT18_01SC_pset5sol.pdf), Arthur Mattuck, Fall 2010. The PDF is fetched from the official URL during acceptance testing, SHA-256 `5015876951651a4f4695ccbbb74e22fdf1a43b1798d4859ed3324cdc9e90d23d`; it is not checked into this repository or bundled with the product. See [MIT source terms](https://ocw.mit.edu/pages/privacy-and-terms-of-use/).

Two independently verified cases:
- 5B-13, problem page 2 / solution page 6: integral of x²/(1+x⁶). A fabricated attempt drops the factor 1/3 while substituting u=x³. Differentiation checks the official antiderivative arctan(x³)/3 + C.
- 5F-2(a), problem page 5 / solution page 18: integral of x eˣ. A fabricated attempt uses a plus instead of a minus in integration by parts. Differentiation checks (x−1)eˣ + C and rejects (x+1)eˣ + C.

The first actual browser run exposed a scrolled-page focus bug: starting a region drag moved the PDF before its coordinates were calculated. `focus({preventScroll:true})`, a failing-before/passing-after unit regression, and the real-PDF browser journey fix this. The original PDF image renders its fractions/exponents; extracted text contains unmapped glyphs and does not reliably preserve mathematical layout. Empty/unmapped PDF extraction now prompts correction or an included page/region image; selected-image preview uses exactly the context image, not a second rendering. This is observable damage detection, not OCR, math recognition or certification of apparently clean text.

A small anonymous direct-API probe used the actual Study prompt builder and OpenCode's documented free `space-bunny-free` model at the existing Zen endpoint. Only the public problems and fabricated attempts were sent, without credentials or settings changes. [Free service documentation](https://opencode.ai/docs/zen/) and [terms applying through use](https://opencode.ai/legal/terms-of-service/). These were direct API calls, not native Tauri IPC or browser-to-model end-to-end tests.

Observed limitations were retained rather than discarded: the original check first endorsed a wrong answer and later contradicted itself. A single verification-before-verdict retest corrected the math but supplied the full solution. The revised structure therefore has an application-side gate, not just a stronger prompt: hint/check streams are not displayed raw; a closed versioned schema is validated only after completion; valid first-step feedback is separated from verification and optional extra solutions. Optional solutions, malformed responses, and interrupted partial responses are mounted only after an explicit reveal action. Old saved plain-text hint/check replies receive the same conservative treatment. Explain/full-solution modes remain distinct.

A bounded three-call structured retest then checked the first hint and both distinct wrong attempts. The hint met the schema. Both checks diagnosed the mathematics correctly without a new final answer but violated the exact field schema (`firstStep` instead of, or in addition to, `firstIssue`); both correctly fell back to explicit reveal. No further tuning or calls were made. This is evidence of safe fallback and remaining provider compatibility limits, **not** full live-tutoring acceptance or general accuracy. A valid schema cannot prove that text within an allowed field is mathematically true or free of spoilers. The UI states that limitation and never awards an automatic correctness/grade badge.

Deterministic browser fixtures exercise display and persistence using visibly simulated responses. They do not establish model teaching quality. Regression coverage retains the observed contradictory/full-solution prose, malformed/duplicate/unknown fields, incomplete streaming, explicit disclosure/re-hiding/reload, exact source-image matching, damaged-text correction, and the two real PDF exercises.
