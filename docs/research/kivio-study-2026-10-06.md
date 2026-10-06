# Kivio Study: design evidence, implementation, and limits

Date: 2026-10-06. This note records the first Study implementation and the evidence behind its design. It does not introduce a second engineering standard; [the unified engineering rules](../engineering-standards.md) remain authoritative.

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

`StudyWorkspace UI → studyWorkspace.sendStudyHelp → studyRequest.requestStudyHelp → api/study IPC → chat/study.rs → existing stream_with_chat_provider`

- `src/chat/study/studyWorkspace.ts` owns page drafts, request identity, in-progress results, original-document updates, and persistence coordination. Navigation does not retarget a running response.
- `studyStorage.ts` owns the IndexedDB schema and storage operations. Materials are identified by a SHA-256 content hash; questions and notes are bound to that document and page. Invalid records and save failures are surfaced rather than silently replaced with a successful-save claim.
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

On Send, the selected provider receives the question, attempt, bounded page history, page number, supplied text, and any disclosed page/region image. Original filenames and document hashes are omitted from the provider prompt. The original file is not uploaded as a whole. The provider's own retention and usage policies still apply, and configured provider charges may apply.

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
