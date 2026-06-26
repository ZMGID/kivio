# Risk Assessment

## S.U.P.E.R Architecture Health Summary

| Principle | Status | Key Findings | Transformation Priority |
|:--|:--|:--|:--|
| **S** Single Purpose | 🟡 | Platform features are conceptually separable, but app startup/window/platform code may aggregate responsibilities. | High |
| **U** Unidirectional Flow | 🟡 | UI -> Tauri -> platform flow is intended; Linux branches must not leak backward into UI logic. | High |
| **P** Ports over Implementation | 🟡 | Existing API contract is documented, but Linux capability ports are not yet explicit. | High |
| **E** Environment-Agnostic | 🔴 | Bundle targets, release workflow, and platform dependencies are macOS/Windows-centered. | Critical |
| **R** Replaceable Parts | 🟡 | Provider/agent architecture appears replaceable; platform capture/OCR/packaging replacement cost is still high. | High |

**Overall Health**: 0/5 fully healthy for Linux readiness — Refactoring Needed.

## S.U.P.E.R Violation Hotspots

1. **Packaging/Release**: `tauri.conf.json` lacks Linux targets; release workflow only builds macOS.
2. **Platform Capture/OCR**: Current platform capability evidence is macOS/Windows only.
3. **Governance Drift**: `AGENTS.md` points to `.trellis/`, but `.trellis/` is absent in the current worktree.
4. **Capability Drift**: Existing architecture notes mention a `settings` capability label, while current capability config lists `main`, `chat`, `lens`, `translate`.

## Risk Matrix

| Risk | Impact | Likelihood | Severity | Mitigation |
|:--|:--|:--|:--|:--|
| AppImage cannot package all runtime resources cleanly | Linux release blocked | Medium | High | Feasibility spike before code refactor |
| Wayland blocks screen/window capture or global shortcuts | Core screen-level Agent degraded | High | Critical | Model X11/Wayland/portal separately |
| Linux OCR path missing | Screenshot translation degraded | Medium | High | Separate OCR port; evaluate RapidOCR/local/remote |
| macOS/Windows regression | Existing users impacted | Medium | High | Keep platform cfg isolated and run cross-platform-safe tests |
| Pyodide/Skills resources missing from Linux bundle | Agent document skills fail | Medium | High | Package content inspection gate |
| Governance source mismatch | Future agents follow stale process | High | Medium | Treat `docs/progress/MASTER.md` as active spec entry while `.trellis/` is absent |

## High-Severity Risks

### Linux desktop capability fragmentation

Ubuntu 22.04 may run X11 or Wayland. Screenshot capture, window enumeration, global shortcuts, transparent overlays, always-on-top windows, tray, clipboard, and focus behavior can differ by display server and desktop environment.

Mitigation:

- Add a Linux capability matrix before implementation.
- Define one backend status command that reports session type and supported capabilities.
- Treat unsupported capabilities as explicit degraded states, not hidden failures.

### Packaging target is not yet Linux-ready

Current `tauri.conf.json` bundle targets are:

```json
["dmg", "msi", "nsis"]
```

Mitigation:

- Run AppImage feasibility as a separate task before committing to AppImage.
- If AppImage fails, document evidence and move to `.deb` or tarball plan.
- Add package content inspection as release acceptance criteria.

Phase 2 update:

```text
tauri build --help: possible bundle values include deb, rpm, appimage
npm exec tauri -- build --bundles appimage --ci: reaches Rust compile
final result: fails before AppImage artifact due Linux compile errors
```

Phase 3 update:

```text
cargo check --manifest-path src-tauri/Cargo.toml --target x86_64-unknown-linux-gnu: passed
npm run build:swift && npm exec tauri -- build --bundles appimage --ci: passed
artifact: src-tauri/target/release/bundle/appimage/Kivio_2.7.2_amd64.AppImage
```

The AppImage route is viable on the current Ubuntu 22.04 x86_64 host. Runtime smoke testing remains required before release confidence.

### macOS Swift sidecar does not translate to Linux

Current packaging references `binaries/kivio-ocr-helper`. On non-macOS, the build script may create stubs for validation, but Linux runtime cannot rely on a macOS OCR helper.

Mitigation:

- Separate OCR capability from packaging validation.
- Make Linux OCR implementation explicit or present a documented disabled state.

## Technical Debt

- Linux target is not present in release automation.
- Linux runtime dependencies and system package assumptions are not documented.
- Existing Trellis instructions point to missing `.trellis/` directory.
- Release documentation and workflow appear partially out of sync regarding Windows release handling.

## Testing Risks

- No e2e runner is documented.
- Desktop behavior needs manual smoke tests in Linux sessions.
- AppImage verification must inspect actual package contents, not just successful build exit code.
- Linux-specific Rust modules should have unit tests where logic can be isolated from OS APIs.

## Project Governance Risks

The project has `AGENTS.md` and `CLAUDE.md`, but no `.trellis/` directory in the current worktree. For this run:

- Active spec progress is stored in `docs/progress/MASTER.md`.
- Durable cross-agent rules should remain in `AGENTS.md` or `CLAUDE.md` only if they affect future sessions.
- No repo-local memory file is created unless the user explicitly asks for one.

## Compatibility Concerns

- Tauri Linux bundle dependencies may require system libraries not currently declared.
- Tray/global shortcut/window overlay behavior may differ across GNOME, KDE, X11, and Wayland.
- Linux filesystem/resource paths must not break Pyodide, Skills, MCP, or native tool sandbox behavior.
- AI Provider and Agent stream payloads must stay backward compatible.
