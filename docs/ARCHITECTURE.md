# Kivio architecture

This document describes the implemented ownership structure and its intended rules. It is not a completion declaration: cross-language routing, navigation races, remaining AppState ownership, page controllers, and logical-module dependency checks still need work. The remaining acceptance contract and batch status are in the [architecture convergence spec](./prd/architecture-convergence-spec.md). Product terminology and invariants remain defined by [`CONTEXT.md`](../CONTEXT.md) and the ADRs in [`docs/adr`](./adr).

## Composition and dependency direction

The React and Tauri entry points are composition roots. Feature code depends on another feature only through that feature's `public/*` interface; implementation files are not cross-feature APIs.

```text
React entry / Tauri commands
  ├─ Chat public interfaces       → chat implementation
  ├─ Settings public interfaces   → settings implementation
  ├─ Lens                         → capture/request/history owners
  ├─ Automation                   → definition/run owners
  └─ shared UI + platform adapters

AppState (partially migrated composition root)
  ├─ ChatInteractionState
  ├─ LensRuntimeState
  ├─ AutomationRunState
  ├─ MCP runtime state
  └─ remaining public mutable domain state (migration pending)
```

`npm run architecture:check` parses static, type-only, re-exported and literal dynamic TypeScript imports into a file dependency graph. It rejects selected direction violations and file-level strongly connected components spanning multiple features. It does not yet check cycles in an aggregated logical-module graph; reciprocal Chat/Settings dependencies can therefore pass. Cross-feature edges must target a `public/*` module, except at the explicit composition roots (`App.tsx`, `Lens.tsx`, and `main.tsx`). The current temporary violation baseline is empty, which proves compliance with these rules, not complete ownership convergence.

## Frontend owners

- `src/chat/routeCodec.ts` centralizes frontend Chat route recognition, decoding and rememberability. `browserRoute.ts` reads the DOM; `persistence.ts` owns storage policy. Rust window restoration still has an independent validator with different acceptance rules; unifying the contract is pending.
- `src/chat/hooks/useComposerDraft.ts` owns the complete draft used before a conversation exists. `Chat.tsx` coordinates the owner with persistence and conversation APIs.
- `src/lens/useLensHistory.ts` owns Lens history ordering, de-duplication, persistence and image eviction.
- `src/lens/useLensSessionCoordinator.ts` owns Lens capture readiness, opening and request generations, cancellation and stale-response isolation. `useLensTranslationSession.ts` owns translation stages, results, errors and terminal cleanup.
- `src/chat/public/*` and `src/settings/public/*` are deliberately small contracts. They are not general barrels.
- `src/styles/app.css` is the ordered stylesheet composition root. Global tokens stay in `index.css`; Chat, Settings, Notes and shared window surfaces own separate files. Their import order preserves the previous cascade.

Large shell components may remain large when they are composition code. New stateful behavior belongs in an owner module with a behavioral interface and lifecycle tests, rather than another cluster of shell-local state/effects.

## Settings authority

Rust owns persisted settings defaults, migration, canonicalization and the process-scoped `{ epoch, revision }` sequence. Reads and every successful write return a canonical snapshot; full saves and imports must submit the version on which the edit began, are checked before runtime side effects, and use the same version again for the final CAS. Lightweight mutations remain narrow operations and return the advanced snapshot. All durable settings writes emit the version-only `kivio-settings-changed` event; per-webview caches accept snapshots monotonically and revalidate on window activation. A settings transaction durably persists the canonical value before materializing external CLI configuration. A durable-save failure restores the store's in-memory cache, while a later materialization failure restores both the durable store and external configuration.

Settings UI state separates the backend canonical snapshot, the acknowledged editing buffer, the current draft and visible merge conflicts. Provider and MCP collections merge by entity identity; plugin-managed MCP rows always follow the backend snapshot. Backend normalization advances the canonical baseline while UI-only placeholder rows remain in the editing buffer. Same-field or delete/edit conflicts retain the draft and block automatic overwrite until the user changes the conflicting value. UI code may validate presentation concerns immediately, but it does not define persisted fallback models, onboarding migration, OCR privacy defaults, provider API-format normalization or prompt-cache migration.

## Backend state owners

`AppState` remains the Tauri composition root. The following migrated state has private owners; Chat execution/protocol state, external sessions and input mailboxes still include public mutable fields and wider AppState dependencies:

- Chat interactions own pending approvals, session consent, user prompts and answered structured content. Validation and one-shot removal happen atomically.
- Lens owns busy acquisition/recovery, open sequence and grace period, selection, reset payload, freeze-frame identity, captured images and request-generation validity. Image registration carries the session sequence captured before the slow OS operation, so closing and immediately reopening cannot admit a late image from the previous session.
- Automation owns active/cancelled run indexes. Starting a run atomically enforces duplicate and concurrency limits; stale cleanup cannot remove a newer run. Cross-domain Chat/agent coordination lives in `automation::application` and reaches Chat cancellation/activity only through narrow ports; neither the runner nor the tool adapter receives `AppState`.
- MCP owns the session pool and persisted tool snapshots. `McpManager` receives a narrow immutable configuration and persistence interface rather than `AppState`; the outer pool lock is never held across transport handshake work.

For migrated owners, callers use narrow operations instead of locking domain maps directly. Extending this rule to remaining mutable domains is part of convergence. A state owner must document create, cancel, completion and error cleanup before adding an asynchronous resource.

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
