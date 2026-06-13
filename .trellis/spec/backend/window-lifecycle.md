# Window Lifecycle

> Tauri window lifecycle conventions for Kivio desktop windows.

## Chat Window Restore Contract

Chat is a normal desktop window, unlike Lens and translator floating windows.

On macOS, packaged builds must preserve that normal desktop identity: do not
declare `LSUIElement=true` for the app bundle just to support tray behavior.
Tray and global shortcuts may remain as auxiliary entry points, but Chat should
stay visible to Dock, Cmd+Tab, and window-capture tools during normal launches.
Only the explicit autostart path may temporarily use `ActivationPolicy::Accessory`
to stay quiet before the user opens Chat.

When reusing an existing Chat window from any activation entry point, the path must:

1. Reapply Chat chrome with `apply_chat_window_chrome`.
2. Reapply Chat min-size with `apply_chat_window_min_size`.
3. Reapply normal window behavior with `normalize_chat_window_behavior`.
4. Restore the app activation policy to `Regular` on macOS.
5. Explicitly `unminimize()` when the window is minimized.
6. Then call `show()` and `set_focus()`.

Activation entry points include:

- Dock reopen / macOS `RunEvent::Reopen`
- tray menu "Open AI Client"
- single-instance activation
- settings routes that reuse the `chat` webview window

## Anti-Pattern

Do not restore Chat from macOS Dock reopen with only:

```rust
let _ = window.show();
let _ = window.set_focus();
```

That bypasses Chat normalization and can leave a packaged macOS build restoring a miniaturized `NSWindow` into a malformed surface that still renders React content but is not managed like a normal app window.

## Correct Pattern

Route Chat activations through `open_chat_window` / `open_chat_settings_window`, or a shared helper that includes the full restore contract above.

Lens and translator windows are intentionally different: they may be frameless, transparent, always-on-top, or skipped from the taskbar. Do not copy their restore behavior into Chat.

## macOS Auxiliary Floating Window Contract

Lens and translator floating windows must work while another app, such as
Chrome, is in macOS native fullscreen. Native fullscreen apps live in their own
Space, so `visible_on_all_workspaces(true)` / `CanJoinAllSpaces` is not enough:
it covers ordinary Spaces but does not reliably place an auxiliary Kivio window
into another app's fullscreen Space.

For macOS-only Lens/translator floating windows, use
`apply_macos_workspace_behavior` before showing the window, then use
`apply_macos_auxiliary_window_activation` after `show()`. The behavior helper
must apply native `NSWindow.collectionBehavior` bits synchronously enough for the
subsequent show path, and the activation helper must order the NSWindow from the
main thread so hidden windows join the currently active fullscreen Space.

- `MoveToActiveSpace`
- `FullScreenAuxiliary`
- `Transient`
- `IgnoresCycle`

Do not leave `CanJoinAllSpaces` or `Stationary` on these auxiliary popups when
showing them from a global shortcut. They can keep a hidden window associated
with its previous Space instead of moving it into the fullscreen Space the user
is currently in.

Keep this behavior out of Chat. Chat is a normal desktop window and must continue
through `normalize_chat_window_behavior`, which clears all-workspaces behavior
and preserves normal Dock/Cmd+Tab/window-management identity.

### Wrong

```rust
let _ = window.set_always_on_top(true);
let _ = window.show();
let _ = window.set_focus();
```

This may work on a normal desktop Space but fail silently over a fullscreen
Chrome Space.

### Correct

```rust
let _ = window.set_always_on_top(true);
#[cfg(target_os = "macos")]
apply_macos_workspace_behavior(&window);
let _ = window.show();
#[cfg(target_os = "macos")]
apply_macos_auxiliary_window_activation(&window);
#[cfg(not(target_os = "macos"))]
let _ = window.set_focus();
```

## Lens Window Selection Contract

Lens window selection must treat Chat as a selectable app window. On macOS,
`CGWindowListCopyWindowInfo` reports Chat, Lens, and legacy translator helper
surfaces with the same Kivio / KeyLingo owner name, so do not filter all
Kivio-owned windows as "self".

Correct filtering is:

- Keep the general filters for invalid ids, non-zero layers, near-transparent windows, and tiny windows.
- Filter Kivio-owned auxiliary surfaces such as Lens (`title == "Lens"`), legacy empty-title floating bars, and the small translator window.
- Allow the Kivio / KeyLingo primary Chat window when its title is `Kivio` / `KeyLingo` and its bounds are large enough to be a desktop window.

### Wrong

```rust
if owner == "Kivio" || owner == "KeyLingo" {
    continue;
}
```

That made Lens unable to hover/select Chat after Chat became the primary
desktop window.

### Correct

Route self-filtering through a helper such as `is_kivio_auxiliary_window`, and
cover the boundary with tests:

- Chat-sized `owner=kivio title=Kivio` is selectable.
- Lens-sized/title `owner=kivio title=Lens` is filtered.
- Small translator-sized Kivio windows are filtered.

## Lens Overlay Close Contract

The Lens window is **reused, not destroyed** — `lens_close` / the frontend close path `hide()`s it and re-positions it fullscreen for next time. Because it is reused and borderless (no rounded corners, no traffic lights), if any close path fails to hide it, the window lingers visible on screen and reappears during later Chat use — looking like a malformed "duplicate" window even when Lens was never re-triggered.

Therefore the close path must hide the Lens window **deterministically**, never conditionally on animation state:

- The frontend `closeAfterReset` plays an exit animation (`resetBeforeHide` → `setStage('select')`) before calling `api.lensClose()`. `resetBeforeHide`'s `setStage` fires the stage motion effect, which bumps the animation-level `motionSeqRef`.
- The "should I still hide?" guard MUST distinguish **a genuine new Lens session opening** from the close's own animation side effects. Guard on an open-generation counter (`lensOpenSeqRef`, incremented only by `enterSelect`), NOT on `motionSeqRef`. Guarding on `motionSeqRef` self-trips: the close's own `setStage` bump makes the guard think a new session started, so it skips `lensClose` and leaks the window.

Do not paper over a leaked Lens window with a backend "hide it whenever Chat opens" backstop — fix the close path so the window is always hidden at its source.

### Anti-Pattern

```ts
resetBeforeHide()                                   // bumps motionSeqRef via setStage('select')
const seq = motionSeqRef.current
await waitForFrames(2)
if (seq !== motionSeqRef.current) return            // self-trips on its own animation bump → window leaks
await api.lensClose()
```

### Correct Pattern

```ts
const openSeq = lensOpenSeqRef.current              // only enterSelect() bumps this
resetBeforeHide()
await waitForFrames(2)
if (openSeq !== lensOpenSeqRef.current) return       // only a real new session aborts the hide
await api.lensClose()
```

## Tests Required

- Run `cargo check --manifest-path src-tauri/Cargo.toml`.
- Run `cargo test --manifest-path src-tauri/Cargo.toml` when practical.
- For macOS release candidates, manually smoke-test installed-app Dock restore:
  - open Chat
  - minimize to Dock
  - click the Dock icon
  - verify the restored window has normal titlebar/Dock behavior, can close/minimize, and does not remain stuck above other windows
- For the Lens overlay close path, manually verify the Lens window never lingers: repeatedly open Lens → ask (handoff to Chat) and open Lens → Esc-close, then switch back to Chat; no second borderless window should remain.
