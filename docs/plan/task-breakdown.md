# Task Breakdown

## Overview

- **Total Phases**: 6
- **Total Tasks**: 18
- **Estimated Total Effort**: XL
- **Execution Mode**: LOCAL_ONLY
- **Code Modification Gate**: No business code changes before user confirms Phase 1 preparation artifacts.

## S.U.P.E.R Design Constraints

- **S (Single Purpose)**: Linux platform services must be split by capability: capture, OCR, hotkey, tray/window, packaging.
- **U (Unidirectional Flow)**: UI calls typed API; backend delegates to platform ports; platform modules never depend on UI.
- **P (Ports over Implementation)**: Define capability/result types before concrete Linux implementations.
- **E (Environment-Agnostic)**: No hardcoded user paths, desktop assumptions, or package locations.
- **R (Replaceable Parts)**: Linux implementations must be swappable between X11, Wayland portal, local OCR, and remote/model fallback.

## Testing and Governance Constraints

- Feature or behavior changes require automated tests where possible.
- Desktop behavior that cannot be automated must get a named smoke checklist.
- Build/test commands should use a timeout; default 60s for focused tests unless a slower build gate is explicitly named.
- Stable gotchas and future-agent rules go to `AGENTS.md`, `CLAUDE.md`, or the resolved progress/memory surface.
- AppImage success is not proven until package contents and runtime smoke are inspected.

## Phase 1: Governance and Linux Readiness Baseline

**Goal**: Establish the non-code-changing baseline and decision rules.
**Prerequisite**: Current working tree can be inspected.
**S.U.P.E.R Focus**: P, E, R

| # | Task | Priority | Effort | Depends On | Lane | S.U.P.E.R | Test Expectation | Memory Impact | Acceptance Criteria |
|:--|:--|:--|:--|:--|:--|:--|:--|:--|:--|
| 1.1 | Create 22-rule Linux adaptation charter | P0 | S | None | A | P,E | Not applicable: docs-only; verify file exists | Update instruction surface if promoted to shared rule | `docs/plan/linux-porting-22-rules.md` exists and is referenced by MASTER |
| 1.2 | Capture bounded architecture and packaging baseline | P0 | M | None | A | P,E,R | Not applicable: docs-only; verify source references | None | `docs/analysis/*.md` records evidence and gaps |
| 1.3 | Initialize LOCAL_ONLY progress tracking | P0 | S | 1.1,1.2 | A | P | Not applicable: docs-only; verify progress files exist | Active progress source established | `docs/progress/MASTER.md` and phase files exist |

### Parallel Lanes

| Lane | Tasks | Combined Effort | Merge Risk | Key Files |
|:--|:--|:--|:--|:--|
| A | 1.1, 1.2, 1.3 | M | Low | `docs/` |

## Phase 2: Linux Feasibility Spikes

**Goal**: Decide whether direct AppImage is viable before deeper refactor.
**Prerequisite**: Phase 1 confirmed.
**S.U.P.E.R Focus**: E, R

| # | Task | Priority | Effort | Depends On | Lane | S.U.P.E.R | Test Expectation | Memory Impact | Acceptance Criteria |
|:--|:--|:--|:--|:--|:--|:--|:--|:--|:--|
| 2.1 | Verify Linux build prerequisites on Ubuntu 22.04 | P0 | M | 1.3 | A | E | Run install/build preflight with timeout | Record required system packages | Dependency list and failing/passing commands recorded |
| 2.2 | Probe Tauri AppImage bundle target | P0 | M | 2.1 | A | E,R | Run bundle feasibility command; inspect output | Record AppImage decision | AppImage proceed/fallback decision documented |
| 2.3 | Build Linux desktop capability matrix | P0 | M | 1.3 | B | P,E,R | Docs/static validation | Add future-agent gotchas if discovered | X11/Wayland/portal matrix completed |

### Parallel Lanes

| Lane | Tasks | Combined Effort | Merge Risk | Key Files |
|:--|:--|:--|:--|:--|
| A | 2.1, 2.2 | M | Medium | package/build configs |
| B | 2.3 | M | Low | docs/capability matrix |

## Phase 3: Platform Boundary Refactor

**Goal**: Prepare replaceable Linux platform ports without changing feature intent.
**Prerequisite**: Phase 2 decision.
**S.U.P.E.R Focus**: S, U, P, R

| # | Task | Priority | Effort | Depends On | Lane | S.U.P.E.R | Test Expectation | Memory Impact | Acceptance Criteria |
|:--|:--|:--|:--|:--|:--|:--|:--|:--|:--|
| 3.1 | Define platform capability/status contracts | P0 | M | 2.3 | A | P,E | Add unit tests for pure contract logic | Record contract convention | UI can query capability status without platform guessing |
| 3.2 | Isolate screenshot/capture platform ports | P0 | L | 3.1 | A | S,U,P,R | Rust tests for non-OS logic | Record platform module pattern | macOS/Windows behavior preserved; Linux stub explicit |
| 3.3 | Isolate OCR platform ports | P1 | M | 3.1 | B | S,P,R | Rust tests for routing/default decisions | Record OCR fallback rule | Linux OCR route can be implemented independently |
| 3.4 | Isolate hotkey/window/tray platform assumptions | P1 | L | 3.1 | C | S,U,E | Rust/TS tests where possible; smoke checklist otherwise | Record unsupported behavior handling | Unsupported Linux capability returns explicit state |

### Parallel Lanes

| Lane | Tasks | Combined Effort | Merge Risk | Key Files |
|:--|:--|:--|:--|:--|
| A | 3.1, 3.2 | L | Medium | platform contracts/capture |
| B | 3.3 | M | Medium | OCR routing |
| C | 3.4 | L | Medium | window/hotkey/tray |

## Phase 4: Linux Runtime Implementations

**Goal**: Implement Linux paths chosen by feasibility results.
**Prerequisite**: Phase 3 ports.
**S.U.P.E.R Focus**: E, R

| # | Task | Priority | Effort | Depends On | Lane | S.U.P.E.R | Test Expectation | Memory Impact | Acceptance Criteria |
|:--|:--|:--|:--|:--|:--|:--|:--|:--|:--|
| 4.1 | Implement Linux capture path or documented disabled state | P0 | L | 3.2 | A | E,R | Unit tests plus manual smoke | Record session-specific caveats | Capture behavior verified or degraded explicitly |
| 4.2 | Implement Linux OCR route | P1 | L | 3.3 | B | E,R | Unit tests plus OCR smoke | Record model/runtime requirements | OCR works or disabled state is explicit |
| 4.3 | Implement Linux hotkey/window/tray behavior | P1 | L | 3.4 | C | E,R | Manual smoke checklist | Record desktop environment caveats | Trigger and window behavior verified |
| 4.4 | Verify Agent runtime resources under Linux paths | P0 | M | 2.1 | D | E,R | Pyodide/skills packaging smoke | Record packaging resource invariant | Skills/Pyodide resources load from packaged app |

### Parallel Lanes

| Lane | Tasks | Combined Effort | Merge Risk | Key Files |
|:--|:--|:--|:--|:--|
| A | 4.1 | L | Medium | capture |
| B | 4.2 | L | Medium | OCR |
| C | 4.3 | L | Medium | window/hotkey/tray |
| D | 4.4 | M | Low | resources |

## Phase 5: Linux Packaging and Release

**Goal**: Produce and verify Linux distributable.
**Prerequisite**: Phase 4 runtime behavior.
**S.U.P.E.R Focus**: E, R

| # | Task | Priority | Effort | Depends On | Lane | S.U.P.E.R | Test Expectation | Memory Impact | Acceptance Criteria |
|:--|:--|:--|:--|:--|:--|:--|:--|:--|:--|
| 5.1 | Add selected Linux bundle target/config | P0 | M | 2.2,4.4 | A | E,R | Build selected bundle | Record release command | Linux artifact builds on Ubuntu 22.04 |
| 5.2 | Inspect Linux artifact contents | P0 | S | 5.1 | A | E | Run package content inspection | Record inspection command | Required resources present |
| 5.3 | Update release checklist and CI strategy | P1 | M | 5.1 | B | E,R | Docs + workflow validation | Update release rules | Linux release steps are documented and reproducible |

### Parallel Lanes

| Lane | Tasks | Combined Effort | Merge Risk | Key Files |
|:--|:--|:--|:--|:--|
| A | 5.1, 5.2 | M | Medium | Tauri/release config |
| B | 5.3 | M | Low | release docs/workflow |

## Phase 6: Regression, Smoke, and Handoff

**Goal**: Prove Linux target without regressing current platforms.
**Prerequisite**: Phase 5 artifact.
**S.U.P.E.R Focus**: P, E, R

| # | Task | Priority | Effort | Depends On | Lane | S.U.P.E.R | Test Expectation | Memory Impact | Acceptance Criteria |
|:--|:--|:--|:--|:--|:--|:--|:--|:--|:--|
| 6.1 | Run automated gates | P0 | M | 5.2 | A | P,E | `npm run lint`, `npm run typecheck`, `npm test`, `cargo test` | Record slow/flaky gates | Gates pass or failures are triaged |
| 6.2 | Run Linux desktop smoke checklist | P0 | M | 5.2 | B | E,R | Manual smoke with evidence | Record environment matrix | Core workflows verified on Ubuntu 22.04 |
| 6.3 | Archive spec-driven artifacts | P1 | S | 6.1,6.2 | A | P | Not applicable: archive/docs | Preserve durable decisions | Active docs archived per workflow |

### Parallel Lanes

| Lane | Tasks | Combined Effort | Merge Risk | Key Files |
|:--|:--|:--|:--|:--|
| A | 6.1, 6.3 | M | Low | progress/archive |
| B | 6.2 | M | Low | smoke evidence |
