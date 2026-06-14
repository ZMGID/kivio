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

Lens and translator (`main`) floating windows must work while another app, such
as Chrome, is in macOS **native fullscreen** (its own Space).

**Root cause of the long-standing breakage (do not regress):** since macOS
10.14/Big Sur, only an **NSPanel**, or a window owned by an **Accessory
(LSUIElement)** app, may be drawn into *another* app's fullscreen Space. Kivio
runs as `ActivationPolicy::Regular` (Chat needs Dock/Cmd+Tab identity — see
above), so a plain tao **NSWindow** can NOT appear over another app's fullscreen
Space — no matter what `collectionBehavior`, window level, or `orderFrontRegardless`
is set. `visible_on_all_workspaces(true)` / `CanJoinAllSpaces` alone is therefore
insufficient. Any path that *activates* the app (`makeKeyAndOrderFront`,
`activateIgnoringOtherApps`, `set_focus`) additionally yanks the user off the
fullscreen Space.

**The fix:** convert the lens/translator windows into **non-activating NSPanels**
on macOS, via `windows::ensure_overlay_panel`. It is idempotent and:

1. `object_setClass`-es the window into a runtime `NSPanel` subclass
   (`canBecomeKeyWindow=YES` — borderless windows default to NO, so this is
   required for the translator input / lens question box to receive keyboard;
   `canBecomeMainWindow=NO`).
2. Sets `NSWindowStyleMaskNonactivatingPanel` — clicking/focusing the panel does
   not activate Kivio, so it never switches away from the fullscreen Space.
3. Sets `collectionBehavior = CanJoinAllSpaces | FullScreenAuxiliary | Transient
   | IgnoresCycle` (clears `MoveToActiveSpace` and `Stationary`, mutually exclusive
   with those). **Use `CanJoinAllSpaces`, NOT `MoveToActiveSpace`** (Spotlight /
   Alfred recipe) — the panel then lives on **every** Space, so it is always on the
   current Space and `orderFront` never needs to switch Spaces (no jump). The
   earlier `MoveToActiveSpace` choice does NOT work here: a **non-activating** panel
   only "becomes key" without activating the app, so its "move to the active Space"
   trigger never fires → the window stays homed on its origin Space and `orderFront`
   yanks you over there every time ("每次都跳"). Do NOT use `Stationary` either — it
   pins the reused window to one Space (the reused window then snaps back to the old
   Space). `FullScreenAuxiliary` keeps it working over another app's fullscreen
   Space; `Transient` lets it float with the Space; `IgnoresCycle` keeps it out of
   Cmd+`. **Do NOT revert to `MoveToActiveSpace`.**
4. Sets window level to `NSStatusWindowLevel` (25) — above the menu bar /
   fullscreen content, but below the `screenSaver` (1000) band that causes a
   wrong-Space blink.
5. Sets `hidesOnDeactivate = NO` — **critical**: NSPanel defaults to hiding when
   its owning app deactivates, and the overlay is shown while another app (e.g.
   fullscreen Chrome) is frontmost, so without this the panel vanishes instantly.

Then show with `windows::show_overlay_panel(&window, need_key)`, which calls
`orderFrontRegardless` (and `makeKeyWindow` when `need_key`). Never use
`window.show()` / `set_focus()` on the macOS overlay path.

Apply `ensure_overlay_panel` at overlay-window creation (`ensure_lens_window` and
`ensure_translate_window`, which share the `ensure_overlay_window` factory), in
the `Focused(true)` self-heal (`main.rs`, for labels `"lens"` and `"translate"`),
and once per show in `lens_request_internal` / `toggle_main_window` / tray `"show"`.
Drop `set_always_on_top(true)` on macOS for these windows — the panel owns its level,
and tao's `set_always_on_top` resets the level to `NSFloatingWindowLevel` (and to
`Normal` on toggle-off), which would clobber the status level.

Keep this behavior out of Chat. **Chat must never be converted to an NSPanel** —
it stays a normal desktop NSWindow through `normalize_chat_window_behavior`,
preserving Dock/Cmd+Tab/window-management identity.

### Wrong

```rust
let _ = window.set_always_on_top(true);
let _ = window.show();
let _ = window.set_focus();
```

A plain NSWindow from a Regular-policy app fails silently over another app's
fullscreen Space (no overlay), and `set_focus`/activation switches Spaces.

### Correct

```rust
#[cfg(not(target_os = "macos"))]
let _ = window.set_always_on_top(true);
#[cfg(target_os = "macos")]
windows::ensure_overlay_panel(&window); // idempotent: reclass to non-activating NSPanel
#[cfg(target_os = "macos")]
windows::show_overlay_panel(&window, /* need_key */ true);
#[cfg(not(target_os = "macos"))]
let _ = window.show();
#[cfg(not(target_os = "macos"))]
let _ = window.set_focus();
```

> Validate in a **release** build (`tauri build`), not only `tauri dev` —
> `setLevel`/`setCollectionBehavior` on a plain NSWindow are widely reported to
> work in dev but fail in release; the NSPanel conversion is the path that
> survives release mode.

## Overlay Keyboard Focus & No-Destroy Contract

The lens/translator overlay is a tao `NSWindow` **reclassed at runtime** into a
non-activating `NSPanel` (`object_setClass`, see `ensure_overlay_panel`). Three
macOS facts about that reclassed panel are load-bearing — getting them wrong shows
up as "input won't focus / can't type / Esc doesn't work" or an outright crash:

1. **Prevents-activation tag (whether keyboard works at all).** A non-activating
   panel receives keys via WindowServer "key-focus theft", gated on the
   `kCGSPreventsActivationTagBit` tag that `NSPanel` sets **only at init**
   (`_panelInitCommonCode`). Because we reclass a *live* window (init already ran as
   a plain `NSWindow`), the tag is never set → the panel draws key but gets **no key
   events** (Apple bug FB16484811). Fix in `configure_overlay_panel`: after
   `setStyleMask` adds `NSWindowStyleMaskNonactivatingPanel`, call private
   `-_setPreventsActivation:(YES)` (guarded by `respondsToSelector:`); and override
   `-_isNonactivatingPanel → YES` on the panel subclass (`kivio_overlay_panel_class`).
   Do NOT use `NSApp.activate` to get keyboard — it jumps Spaces over fullscreen.

2. **First responder must land on the WKWebView, re-triggered from the frontend.**
   The lens window is **reused** (hidden on close). On reuse,
   `makeFirstResponder(contentView)` does NOT reliably sink to the inner `WKWebView`
   (contentView is the wry container), so the 2nd+ open needed a manual click to
   focus. Fix: walk the view tree to the real `WKWebView` (`find_wk_webview`) and
   `makeFirstResponder` IT (`focus_overlay_webview`); and because the input only
   exists in the post-capture `ready` stage, the **frontend** re-triggers it —
   `focusLensSurface` calls `api.lensFocusWebview()` at its retry delays
   `[0,40,120,240,420]`. The retries absorb the show→ready timing and keep focus
   stable across repeated opens. (Show-time native FR alone was only "mostly".)

3. **NEVER `destroy()` a reclassed overlay window.** `window.destroy()` on a window
   whose class was swapped via `object_setClass` tears it down against its original
   class and raises an Obj-C exception across FFI →
   `fatal runtime error: Rust cannot catch foreign exceptions, aborting`. So the lens
   window is **reused (hidden), never destroyed** — "recreate fresh each open" is NOT
   available here. `lens_close` must `hide()`, not `destroy()`.

## Overlay Dismissal Focus-Return Contract

> **Status (06-14):** the `lens_close` reactivate-previous-app step was **removed** —
> explicitly reactivating the prior app on lens close raced with the next open's
> key-focus and caused the reused-panel "2nd open not focused" flakiness. With the
> panel properly non-activating (tag above), hiding it returns focus to the prior app
> on its own and never makes Kivio frontmost-windowless, so the Reopen→Chat jump does
> not fire. Only the translator (`main`) still uses the explicit restore below; for
> lens it is intentionally absent.

When a lens / translator overlay (a non-activating NSPanel) is dismissed (Esc,
toggle hotkey, blur, commit), focus must return to **the app that was frontmost
before the overlay was shown** — not to a Kivio window.

**Why this is load-bearing.** The overlay is a non-activating NSPanel: showing it
makes it `key` without activating Kivio. When it is `orderOut`/hidden, AppKit must
pick a new frontmost window/app and **sometimes reactivates the Regular-policy
Kivio process**. At that instant the only thing on screen is the panel — which is
not counted by AppKit's `hasVisibleWindows` and is excluded from
`USER_WINDOW_LABELS` — so the `RunEvent::Reopen { has_visible_windows: false }`
branch fires and **unconditionally `open_chat_window`s**, making Chat jump up /
spawn from nothing (Chat is destroyed on close, so this is a fresh create). The
fix removes the *trigger*, not the symptom: if dismissal returns front to the
previous app, Kivio never becomes "frontmost with no window", so the stray Reopen
never happens.

**Mechanism (`windows.rs`, macOS).** Two `AtomicI32` slots in `AppState`
(`prev_frontmost_pid_lens`, `prev_frontmost_pid_main`; 0 = none) hold the PID
snapshotted at show time. lens (incl. screenshot/text translate, which use the
lens window) and the input-translator (`main` window) are independent overlays
that can be open at once, so each owns its own slot — a single shared slot would
let whichever closes first consume the snapshot and leave the other's close
without a focus hand-back.

- `remember_frontmost_app(slot)` — call BEFORE showing the overlay, and only on a
  path that will actually show it (`lens_request_internal` AFTER the early-return
  guards, right after `ensure_lens_window` succeeds; `toggle_main_window` /
  tray `"show"` on the show path). Stores `NSWorkspace.frontmostApplication.pid`,
  or 0 if that is Kivio itself / unavailable. Do not snapshot on a path that may
  abort before showing (it would leave a stale, never-consumed PID).
- `restore_previous_frontmost_app(app, slot)` — call on dismiss (`lens_close` →
  lens slot; `main.rs` `WindowEvent::CloseRequested` for label `"main"` → main
  slot, without preventing close). Swaps the PID out (idempotent) and
  `NSRunningApplication.activateWithOptions:` on the main thread. 0 ⇒ no-op.
- `forget_frontmost_app(slot)` — clear a slot. Call **both** slots at the START of
  every deliberate Chat-foreground action: `open_chat_window` AND
  `open_chat_settings_window` (opening in-app Settings from the translator is also
  deliberate — without this, the translator's subsequent close yanks focus back to
  the old app and buries Settings). `commit_translation` clears the main slot so
  its existing `[NSApp hide:]` paste-handoff is the sole focus-return (the
  `CloseRequested("main")` restore then no-ops, avoiding redundant activation).

### Rules

- Restore by reactivating the PREVIOUS app (`NSRunningApplication.activate`), never
  by `set_focus` / `makeKeyAndOrderFront` / `activateIgnoringOtherApps` on the
  panel — those would drag Kivio forward (and out of a fullscreen Space).
- Do NOT "fix" this by guarding the `RunEvent::Reopen` branch on `lens_busy` — that
  is a symptom backstop. Remove the trigger (return focus) instead.
- `commit_translation`'s existing `[NSApp hide:]` (paste-handoff path) is left as
  is; the `CloseRequested("main")` restore also runs for it and is benign
  (both yield front to the previous app).
- macOS-only; Windows/Linux focus behavior is unchanged.

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
