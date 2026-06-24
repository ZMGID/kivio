# Module Inventory

> 本文件是 Linux 适配前的轻量模块盘点。它刻意不做全量逐文件通读，只记录与 Linux 版本重构适配主线直接相关的模块边界。

## Summary

| Module | Responsibility | Evidence | Linux Relevance | S.U.P.E.R Score |
|:--|:--|:--|:--|:--|
| Frontend Shell | React/Vite UI shell and route selection | `src/`, `package.json` | Low-Medium: mostly preserved | S🟢 U🟡 P🟡 E🟢 R🟡 |
| Tauri API Contract | Frontend-backend invoke/event contract | `src/api/tauri.ts`, `CLAUDE.md` | High: Linux commands must preserve contract | S🟢 U🟢 P🟡 E🟢 R🟡 |
| Rust App Core | Tauri commands, state, windows, settings, runtime | `src-tauri/src/*.rs`, `Cargo.toml` | High: platform branching must be isolated | S🟡 U🟡 P🟡 E🟡 R🟡 |
| Platform Capture/OCR | macOS/Windows screenshot and OCR | `Cargo.toml`, `CLAUDE.md` | Critical: Linux equivalent missing | S🟡 U🟡 P🟡 E🔴 R🟡 |
| Agent Runtime | Chat agent loop, MCP, Skills, native tools | `CLAUDE.md` | Medium: preserve behavior/security | S🟢 U🟢 P🟡 E🟡 R🟡 |
| Packaging/Release | Tauri bundle and release workflows | `tauri.conf.json`, `release.yml`, `RELEASE_PACKAGING.md` | Critical: Linux package target missing | S🟡 U🟡 P🟡 E🔴 R🔴 |
| Governance/Progress | Agent instructions and spec progress | `AGENTS.md`, `CLAUDE.md`, `docs/progress/` | High: required before edits | S🟡 U🟢 P🟢 E🟢 R🟡 |

## Module Details

### Frontend Shell

- **Responsibility**: Render translator, settings, chat, lens views through React/Vite.
- **Public API**: Tauri invoke/event calls via the frontend API layer.
- **Linux notes**: UI should mostly stay unchanged; Linux-specific UX should flow from backend capability flags, not scattered user-agent checks.
- **S.U.P.E.R Assessment**:
  - **S**: Mostly single-purpose at component level.
  - **U**: Must preserve UI -> API -> backend direction.
  - **P**: Needs stable typed contracts for platform capability flags.
  - **E**: Avoid hardcoding macOS/Windows-only assumptions in UI copy or routing.
  - **R**: Platform UX should be replaceable through capability data.

### Tauri API Contract

- **Responsibility**: Centralize frontend-backend commands and events.
- **Linux notes**: New Linux commands should be exposed through existing contract patterns first.
- **S.U.P.E.R Assessment**:
  - **S**: Strong if `src/api/tauri.ts` remains the single contract surface.
  - **U**: Strong if provider/platform internals do not leak upward.
  - **P**: Needs explicit types for Linux capability/status outputs.
  - **E**: Good if backend reports environment rather than frontend guessing it.
  - **R**: Good if command names and payloads stay stable.

### Rust App Core

- **Responsibility**: Tauri setup, windows, commands, settings, state, runtime services.
- **Linux notes**: Platform-dependent behaviors should be moved behind Linux-specific modules or trait-like ports before implementation.
- **S.U.P.E.R Assessment**:
  - **S**: Mixed; app startup often aggregates many responsibilities.
  - **U**: Must avoid UI/platform reverse dependencies.
  - **P**: Platform services need explicit input/output contracts.
  - **E**: Current app is environment-aware but Linux gaps are not modeled.
  - **R**: Improve by isolating screenshot/OCR/hotkey/window behavior.

### Platform Capture/OCR

- **Responsibility**: Capture screen/window/region and OCR text.
- **Linux notes**: Current evidence shows macOS and Windows implementations, but no Linux implementation baseline.
- **S.U.P.E.R Assessment**:
  - **S**: Capture and OCR should remain separate ports.
  - **U**: UI asks backend; backend delegates platform; platform returns data.
  - **P**: Must define serializable capture/OCR result types.
  - **E**: Currently Linux gap is high risk.
  - **R**: Linux backend should be swappable: portal/X11/RapidOCR/remote model.

### Agent Runtime

- **Responsibility**: Provider-agnostic agent loop, tools, MCP, Skills, sub-agents, Pyodide.
- **Linux notes**: Runtime behavior should not change for Linux except filesystem paths, packaged resources, process execution, and permissions.
- **S.U.P.E.R Assessment**:
  - **S**: Existing docs describe separated model/tool/agent concerns.
  - **U**: Preserve provider adapters as peer implementations.
  - **P**: Tool and stream payloads are contract surfaces.
  - **E**: Linux package paths and sandbox resources need validation.
  - **R**: Tool backends must remain replaceable.

### Packaging/Release

- **Responsibility**: Build desktop bundles and package runtime resources.
- **Linux notes**: Must add Linux packaging target only after AppImage feasibility is verified.
- **S.U.P.E.R Assessment**:
  - **S**: Release docs and workflow are separate but may drift.
  - **U**: Build pipeline should flow from config -> build -> package -> verify.
  - **P**: Package resource manifest should be explicit.
  - **E**: Linux target missing, so environment coverage is incomplete.
  - **R**: Current release pipeline is not yet replaceable across OS targets.

### Governance/Progress

- **Responsibility**: Keep rules, task state, risks, and durable decisions visible.
- **Linux notes**: Required before code edits per user request.
- **S.U.P.E.R Assessment**:
  - **S**: `AGENTS.md`, `CLAUDE.md`, and `docs/progress/MASTER.md` have distinct roles.
  - **U**: Future sessions must read `MASTER.md` first.
  - **P**: Progress state is Markdown contract.
  - **E**: Works locally without GitHub.
  - **R**: Can later migrate to GitHub issues if auth/scope becomes available.
