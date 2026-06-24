# Linux Desktop Capability Matrix

## Current Session

```text
OS: Ubuntu 22.04.5 LTS
Kernel: 6.8.0-40-generic
Desktop: ubuntu:GNOME
Session type: x11
DISPLAY=:1
WAYLAND_DISPLAY=
```

Portal packages:

```text
xdg-desktop-portal 1.14.4-1ubuntu2~22.04.2
xdg-desktop-portal-gnome 42.1-0ubuntu2
xdg-desktop-portal-gtk 1.14.0-1build1
```

Desktop tools:

```text
gdbus /usr/bin/gdbus
dbus-send /usr/bin/dbus-send
loginctl /usr/bin/loginctl
gnome-shell /usr/bin/gnome-shell
xprop /usr/bin/xprop
xwininfo /usr/bin/xwininfo
grim MISSING
slurp MISSING
gnome-screenshot MISSING
import MISSING
scrot MISSING
spectacle MISSING
```

## Capability Matrix

| Capability | Current Evidence | X11 Expectation | Wayland Expectation | Required Adaptation |
|:--|:--|:--|:--|:--|
| Tauri WebView | `webkit2gtk-4.1 2.50.4` present | Supported | Supported | Keep Tauri v2/WebKitGTK path |
| AppImage bundle | `Kivio_2.7.2_amd64.AppImage` built through `npm run build` via `tauri.linux.conf.json` | Supported on current X11 build host | Packaging likely same; runtime smoke still needed | Keep AppImage path; retain smoke/content gates |
| Main/chat/settings/lens windows | AppImage desktop smoke reports `chat` exists and is visible; real `Ctrl+Shift+G` smoke produced a Kivio window path | Supported on current X11 smoke host | Likely supported with compositor limits, still unverified | Keep AppImage smoke as release gate |
| Transparent Lens overlay | Real XTest `Ctrl+Shift+G` produced a Kivio window path on X11 after Linux `XInitThreads()` init; final observed window title was `Kivio`, not `Lens` | Works on current X11 smoke host for the hotkey-triggered window path; real screenshot capture remains disabled | Risky; compositor/portal rules may limit | Keep degraded capability status until Wayland smoke exists |
| Region screenshot | `capture_port.rs` owns platform ports; Lens blocks capture gestures when backend says unsupported | X11 can use X11/screenshot backend | Prefer portal screenshot API | Disabled-state path completed; real Linux capture remains future work |
| Window enumeration | `xprop`/`xwininfo` present | Possible via X11 | Usually restricted | Capability must report unsupported/degraded state |
| Excluding own overlay from capture | macOS-specific SCK path exists | Needs Linux-specific strategy | Needs portal/compositor strategy | Do not assume possible |
| OCR | `ocr_port.rs` owns availability, mode normalization, and local OCR dispatch; Settings exposes Linux RapidOCR without System OCR. Linux ignored smoke passed RapidOCR model download/init/predict in an isolated temp model dir. | Local RapidOCR route verified outside GUI | Local RapidOCR route verified outside GUI; compositor capture remains separate | Keep GUI/AppImage OCR smoke as a separate gate |
| Global shortcuts | AppImage smoke reports all 4 default hotkeys registered; XTest `Ctrl+Shift+G` triggers Lens path | Works on current X11 smoke host | Often restricted under Wayland | Keep degraded capability status until per-session smoke passes |
| Clipboard | Tauri clipboard plugin compiles | Likely supported | Likely supported through desktop APIs | Smoke test copy/paste workflows |
| Tray/AppIndicator | AppImage smoke reports `trayPresent:true`; `ayatana-appindicator3` present | Supported in this GNOME/X11 smoke | Same desktop-dependent caveats | Keep smoke gate for other desktops |
| Autostart | AppImage smoke with `launchAtStartup=true` reports `autostartEnabled:true` and writes `$HOME/.config/autostart/Kivio.desktop` | Verified desktop-file path on current host | Likely same desktop-file path, still desktop-dependent | Create `$HOME/.config/autostart` before auto-launch enable |
| Native shell tool | Unix `libc` dependency visible; cargo check passes | Compile gate passed | Compile gate passed | Runtime command smoke still separate |
| Pyodide resources | Final AppImage smoke resolved 25 Pyodide core/wheel/font assets through Tauri `asset_resolver`, with 0 missing assets | Supported in packaged AppImage on current X11 host | Packaging path should be session-independent; GUI runtime still needs workflow smoke | Keep resource smoke as release gate |
| Skills resources | AppImage smoke found 7 bundled skills under `/tmp/.mount_*/usr/lib/Kivio/skills`; headless AppDir path has a unit test | Supported for GUI and headless AppDir layout | Packaging path should be session-independent | Keep AppDir resource path test and AppImage smoke |

## Capability Contract

Task 3.1 added the backend capability contract:

```text
Tauri command: get_platform_capabilities
Rust module: platform_capabilities.rs
Frontend bridge: api.getPlatformCapabilities()
```

Current Linux contract:

- `windowCapture`: `unsupported` — window capture is not implemented on Linux.
- `regionCapture`: `unsupported` — region capture is not implemented on Linux.
- `systemOcr`: `unsupported` — no Linux system OCR route exists.
- `rapidOcr`: `supported`, `smokeRequired=true` — backend keeps OCR as a per-machine smoke gate. In this session, the Linux ignored smoke verified model download, runtime init, and predict in an isolated temp model directory. GUI/AppImage OCR smoke remains separate.
- `globalShortcuts`, `tray`, `autostart`, `transparentOverlay`: `degraded`, `smokeRequired=true`.

Task 3.2 moved region screenshot behavior behind `capture_port.rs`. The caller
now builds a `RegionCaptureRequest` and receives one platform result path. Linux
still returns explicit unsupported; real X11/Wayland capture belongs in that
port, not in Lens command orchestration.

Task 3.3 moved OCR platform decisions behind `ocr_port.rs`. Linux System OCR
normalizes to CloudVision, while RapidOCR remains selectable and status/download
checks are gated by the `rapidOcr` capability instead of `systemOcr`.

Task 3.4 connected desktop-dependent capability status to Settings copy for
global shortcuts, transparent overlay, and autostart. These remain runtime smoke
gates; the change prevents the UI from presenting them as unqualified Linux
support.

Task 4.1 closed the current Linux capture runtime behavior through explicit
disablement rather than a partial X11/Wayland backend. Lens now reads backend
capabilities, shows a capture-unsupported hint when both window and region
capture are unavailable, and blocks the corresponding capture gestures. The
backend `capture_port.rs` unsupported error remains as a defensive guard.

Task 4.2 verified the local Linux RapidOCR route with an ignored smoke test:
`cargo test --manifest-path src-tauri/Cargo.toml rapidocr::tests::linux_smoke_downloads_models_and_initializes_pipeline -- --ignored --nocapture`
returned `1 passed; 0 failed` in `51.95s`. The test uses an explicit temp model
directory, downloads the required runtime/model artifacts, initializes the OCR
pipeline, and runs predict on a generated PNG. This does not verify GUI capture
or AppImage UI invocation.

Task 4.3 verified desktop runtime behavior through the packaged AppImage on the
current X11 session. `KIVIO_DESKTOP_SMOKE=1` reported 4/4 default hotkeys
registered, `trayPresent:true`, and a visible `chat` window. A launch-at-startup
smoke wrote `$HOME/.config/autostart/Kivio.desktop` and reported
`autostartEnabled:true`. A real XTest `Ctrl+Shift+G` trigger produced a Kivio
window path and no longer reproduced the prior Xlib/xcb threading crash after
Linux startup calls `XInitThreads()` before Tauri/GTK/WebKit setup.

Task 4.4 verified Agent resource loading through the packaged AppImage. The
desktop smoke `resources` block reported `resourceDir=/tmp/.mount_*/usr/lib/Kivio`,
`bundledSkillsDirExists:true`, `skillCount:7`, `builtinSkillCount:7`,
`skillWarnings:[]`, and the bundled skill ids `doc-coauthoring`, `docx`,
`frontend-design`, `mcp-builder`, `pdf`, `skill-creator`, and `xlsx`. The same
smoke initially resolved the five Pyodide core files through Tauri
`asset_resolver`; Phase 6 final smoke expanded that check to 25 Pyodide
core/wheel/font assets, all with `present:true` and 0 missing assets. Headless
`kivio-code` now checks the Linux AppDir layout
`usr/bin` -> `usr/lib/Kivio/skills`, covered by
`bundled_skills_dir_resolves_linux_appdir_layout`.

## Immediate Engineering Implications

1. Linux must not be treated as a single backend. At minimum, session type must distinguish X11 and Wayland.
2. UI should query `get_platform_capabilities` before promising screenshot/OCR/hotkey/window behavior.
3. RapidOCR is the verified local Linux OCR path. Linux ONNX Runtime constants compile, OCR settings/routing preserve RapidOCR on Linux, Settings exposes it independently from System OCR, and the ignored smoke test passed model download/init/predict. GUI/AppImage OCR invocation is still a separate proof gate.
4. Global shortcuts, tray, autostart, and Lens show behavior are verified on this X11 host through AppImage smoke. Keep them degraded until each target desktop/session is smoked.
5. AppImage is now a reproducible packaging target on this host via `tauri.linux.conf.json` and `npm run build`; release inspection requires both AppDir Skill listing and packaged asset-resolver smoke for Pyodide.
