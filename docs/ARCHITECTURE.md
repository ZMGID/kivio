# Kivio architecture

This document describes the implemented ownership structure and its intended rules. Code-level convergence and platform acceptance are tracked separately in the [architecture convergence spec](./prd/architecture-convergence-spec.md); this document alone is not a platform completion declaration. Product terminology and invariants remain defined by [`CONTEXT.md`](../CONTEXT.md) and the ADRs in [`docs/adr`](./adr).

## Composition and dependency direction

The React and Tauri entry points are composition roots. Feature code depends on another feature only through that feature's `public/*` interface; implementation files are not cross-feature APIs.

```text
React entry / Tauri commands
  ├─ Chat public interfaces       → chat implementation
  ├─ Settings public interfaces   → settings implementation
  ├─ Lens                         → capture/request/history owners
  ├─ Automation                   → definition/run owners
  └─ shared UI + platform adapters

AppState (composition root)
  ├─ ChatRuntimeState + ChatProtocolState + ChatInteractionState
  ├─ ExternalDiscoveryState + LiveSessionRegistry
  ├─ ProviderRuntimeState + background registries
  ├─ LensRuntimeState
  ├─ AutomationRunState
  ├─ MCP runtime state
  └─ Settings persistence gate + platform focus owner
```

`architecture-boundaries.json` declares logical Modules and their roles. `npm run architecture:check` resolves static, type-only, re-exported and literal dynamic TypeScript imports. It checks both the file graph and the graph aggregated by declared Module, reports an exact edge witness for every Module cycle, and rejects adapter-to-feature and other disallowed reverse edges. Cross-feature edges must target a `public/*` module, except at the explicit composition roots (`App.tsx`, `Lens.tsx`, and `main.tsx`). Unmapped source paths fail closed. The temporary violation baseline is empty; a green graph proves dependency direction, not platform behavior.

## Frontend owners

- `src/chat/routeContract.json` is the shared Chat route vocabulary and conformance corpus. `routeCodec.ts` consumes it at runtime; Rust embeds the same declaration for persisted-window restoration. `browserRoute.ts` reads the DOM, `persistence.ts` owns ordered/retryable storage migration, and `conversationTransitionStore.ts` owns the navigation generation that gates asynchronous UI commits without cancelling background execution.
- `src/chat/hooks/useComposerDraft.ts` owns the complete draft used before a conversation exists. The Chat navigation controller controls route-load and popout commit rights. Conversation preparation owns ordered draft patches and returns a recoverable partial conversation on failure. The execution owner holds send claims, in-flight identity, optimistic messages and run generations. Send, run-command, stream-lifecycle, queue, interaction-inbox and popout owners coordinate outcomes and cleanup; run-display modules project stream and tool events. `Chat.tsx` composes these interfaces and renders the page rather than maintaining a second run-terminal authority.
- `src/lens/useLensHistory.ts` owns Lens history ordering, de-duplication, persistence and image eviction.
- `src/lens/useLensSessionCoordinator.ts` owns Lens capture readiness, opening and request generations, cancellation and stale-response isolation. The content controller combines conversation, selection, annotation, translation, history and session owners behind open/hide/restore/handoff transitions; bar motion remains separately owned. History owns image commitment, thumbnails and latest completed-turn snapshots. Closing first conceals the surface, then commits content cleanup only after native hide succeeds; a failed hide restores the same opening. OS window animation timing, canvas and viewport layout remain in the Lens page.
- `src/chat/public/*` and `src/settings/public/*` are deliberately small contracts. They are not general barrels.
- `src/components/i18n.ts` owns application-wide language selection and translations; Settings edits the language through its canonical/draft state and passes the current draft language to embedded Chat-owned views. Chat and Onboarding do not reach through Settings for i18n.
- `src/styles/app.css` is the ordered stylesheet composition root. Global tokens stay in `index.css`; Chat, Settings, Notes and shared window surfaces own separate files. Their import order preserves the previous cascade.

Large shell components may remain large when they are composition code. New stateful behavior belongs in an owner module with a behavioral interface and lifecycle tests, rather than another cluster of shell-local state/effects.

## Settings authority

Rust owns persisted settings defaults, migration, canonicalization and the process-scoped `{ epoch, revision }` sequence. Reads and every successful write return a canonical snapshot; full saves and imports must submit the version on which the edit began, are checked before runtime side effects, and use the same version again for the final CAS. Lightweight mutations remain narrow operations and return the advanced snapshot. All durable settings writes emit the version-only `kivio-settings-changed` event; per-webview caches accept snapshots monotonically and revalidate on window activation. A settings transaction durably persists the canonical value before materializing external CLI configuration. A durable-save failure restores the store's in-memory cache, while a later materialization failure restores both the durable store and external configuration.

Settings UI state separates the backend canonical snapshot, the acknowledged editing buffer, the current draft and visible merge conflicts. Provider and MCP collections merge by entity identity; plugin-managed MCP rows always follow the backend snapshot. Backend normalization advances the canonical baseline while UI-only placeholder rows remain in the editing buffer. Same-field or delete/edit conflicts retain the draft and block automatic overwrite until the user changes the conflicting value. UI code may validate presentation concerns immediately, but it does not define persisted fallback models, onboarding migration, OCR privacy defaults, provider API-format normalization or prompt-cache migration.

`SettingsEditorController` owns the editable snapshot lifecycle, autosave queue, conflict handling and close-before-flush contract. Import stops if a preceding draft save fails; navigation out of Settings waits for a successful flush. Onboarding restart owns its flush/narrow-write/reflush sequence and never navigates over an unresolved draft. OCR/update downloads, permissions, memory editing, shortcut recording, provider-catalog fetches, backup and provider modal state have separate owners. Local navigation, presentation and read-only loads remain in the page.

## Backend state owners

`AppState` remains the Tauri composition root, with no public mutable lock or atomic fields. The [state ownership inventory](./architecture-state-ownership.md) records each migrated field, its callers, lifecycle and lock order. Its private owners include:

- Chat runtime owns parallel generations, reply reservations, steering/follow-up input and creation coordination. Run completion retires only its own generation; the last run or explicit conversation cancellation clears pending input. Chat protocol owns replay and live subscribers. The agent loop receives a narrow model-execution port rather than the whole `AppState`.
- Chat interactions own pending approvals, session consent, user prompts and answered structured content. Validation and one-shot removal happen atomically.
- External-agent discovery owns bounded caches and single-flight probes; live sessions own reuse, idle eviction and shutdown. The in-process Lens-to-Chat mailbox retains requests until an owner acknowledges them. Release and expiring, renewable leases make unfinished requests claimable after a renderer disappears. This is at-least-once delivery within one backend process, not durable cross-process or exactly-once delivery.
- Provider runtime owns key failover and learned endpoint capabilities. Native background commands and external CLI background tasks have separate registries with explicit completion, cancellation and exit behavior. Request Debug owns its bounded memory buffer and existing disk mirror.
- Lens owns busy acquisition/recovery, open sequence and grace period, selection, reset payload, freeze-frame identity, captured images and request-generation validity. Image registration carries the session sequence captured before the slow OS operation, so closing and immediately reopening cannot admit a late image from the previous session.
- Automation owns active/cancelled run indexes. Starting a run atomically enforces duplicate and concurrency limits; stale cleanup cannot remove a newer run. Cross-domain Chat/agent coordination lives in `automation::application` and reaches Chat cancellation/activity only through narrow ports; neither the runner nor the tool adapter receives `AppState`.
- MCP owns the session pool and persisted tool snapshots. `McpManager` receives a narrow immutable configuration and persistence interface rather than `AppState`; the outer pool lock is never held across transport handshake work.
- Settings persistence owns the full-save permit; platform focus owns the two macOS foreground-return slots. Immutable resources and encapsulated domain handles remain in the composition root.

Callers use narrow operations instead of locking domain maps directly. A state owner must document create, cancel, completion and error cleanup before adding an asynchronous resource. Application-level orchestration still uses `AppState` at command boundaries without reopening mutable domain fields.

## External agents

`RuntimeAgentDef` and `AGENT_DEFS` are the static capability authority. Each definition declares install/update/version behavior, current-config and model-probe strategy, provider-profile strategy, context-window and usage fallback, error recovery, launch/home behavior, sandbox capability, import policy and run policy. Detection supplies runtime availability and dynamic values; top-level orchestration does not infer static behavior from brand-name branches. Binary-only callers without a definition go through an explicit compatibility adapter. Protocol actors continue to communicate with the host through channels and do not acquire `AppHandle`.

MCP form elicitation supports the explicitly validated schema subset documented in `external_agents/ask_user.rs`. Required/optional status and value constraints travel through parser, generated protocol, UI and encoder. Omitted values, explicit empty strings, `0` and `false` remain distinct.

## Storage

Conversation files remain the source of truth. The existing repository contract continues to own keyed locks, revision/CAS, migration barriers and index locks. Conversation, index, project, set, assistant, migration and search implementations are private `chat::storage` modules behind the existing facade. Search tolerates a corrupt conversation as a skipped result rather than blocking valid conversations.

Atomic writes keep the established order: write a temporary file, flush it, then atomically replace the destination. Rename-failure tests prove that an old record stays readable and no temporary file is leaked; restart tests create a fresh storage owner and re-read the committed record from disk. Do not introduce another repository or a second write path for the same logical record.

## Required checks

Before merging architecture changes, run:

```sh
npm run architecture:check
npm run lint
npm run typecheck
npm run test
cargo test --manifest-path src-tauri/Cargo.toml
```

Ignored live external-CLI tests require installed and authenticated third-party tools. Platform window/capture behavior must be smoke-tested on its actual Windows or macOS target; source inspection is not a substitute for that platform result.
