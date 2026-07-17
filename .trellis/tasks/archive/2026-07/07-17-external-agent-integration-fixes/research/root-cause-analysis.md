# Bug Analysis: External Agent Completion and Model Discovery

## 1. Root Cause Category

- **#17: B/E — Cross-Layer Contract + Implicit Assumption.** Kivio treated Pi's `agent_end` as both logical turn completion and physical stdout completion. Pi's RPC contract actually closes on stdin EOF only after disposing and flushing stdout.
- **#16: B/D — Cross-Layer Contract + Test Coverage Gap.** Model detection ran ACP from a temp cwd and silently used a static fallback. The UI/API/cache contract had no project context, so project JSONC could not influence discovery.

## 2. Why the Initial Behavior Failed

1. The Pi reader stopped at the first logical end event, dropping the pipe before queued writes completed.
2. OpenCode config behavior was approximated through ACP/static data instead of using its native `models` command.
3. A single global cache assumed model availability was independent of cwd.
4. UI requests had no stale-response guard when switching project context.

## 3. Prevention Mechanisms

| Priority | Mechanism | Specific Action | Status |
|---|---|---|---|
| P0 | Architecture | Separate protocol completion from stdout EOF and drain Pi output | DONE |
| P0 | Test coverage | Delayed post-`agent_end` write regression | DONE |
| P0 | Native contract | Discover OpenCode models through `opencode models` | DONE |
| P1 | Context propagation | Pass conversation cwd through frontend, command, probe, and runtime | DONE |
| P1 | Cache isolation | Scope detected agents and model metadata by cwd | DONE |
| P1 | UI race prevention | Ignore responses from stale conversation requests | DONE |
| P1 | Documentation | Add external CLI lifecycle/project-context checklist | DONE |

## 4. Systematic Expansion

- Other external protocols should be reviewed whenever their end event precedes process exit.
- Any future discovery result affected by workspace files must include cwd in both execution and cache keys.
- Native CLI discovery should remain the source of truth when Kivio would otherwise need to duplicate a third-party config schema.

## 5. Knowledge Capture

- [x] Updated `.trellis/spec/guides/cross-layer-thinking-guide.md`.
- [x] Added deterministic Pi pipe-lifecycle tests.
- [x] Added OpenCode parser, cache-isolation tests, and a real temporary-project JSONC verification.
- [x] No template-spec mirror exists in this repository, so no template sync applies.
