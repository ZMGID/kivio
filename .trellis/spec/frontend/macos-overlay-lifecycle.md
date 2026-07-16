# macOS Overlay Lifecycle

## 1. Scope / Trigger

This contract applies whenever the Lens, screenshot translation, replace translation, selected-text translation, or input translator creates, focuses, closes, or destroys a macOS overlay. These windows start as Tauri/tao `NSWindow` instances and are reclassified as nonactivating `NSPanel` subclasses.

## 2. Signatures

- `ensure_overlay_window(app, label, title, mode) -> Result<WebviewWindow, String>` creates Lens-family overlays.
- `show_overlay_panel(window, need_key)` orders a panel without activating Kivio and optionally gives its WKWebView keyboard focus.
- `focus_overlay_webview(window)` refreshes first-responder focus without calling Tauri `set_focus`.
- `restore_previous_frontmost_app(app, slot)` restores another app only when it is no longer frontmost.
- `destroy_overlay_window(window)` restores the original tao class and destroys the window in one guarded main-thread operation.

## 3. Contracts

- Cold-created hidden overlays must use `.focused(false).visible(false)` before `build()`.
- Lens-family launch paths must not call `reassert_previous_frontmost_app`, `window.set_focus`, `makeKeyAndOrderFront`, or `activateIgnoringOtherApps`.
- Keyboard focus must come from `show_overlay_panel(..., true)` or `focus_overlay_webview()` after the window has become a nonactivating panel.
- `restore_previous_frontmost_app` must be a no-op when the recorded PID is already frontmost; activating with `NSApplicationActivateAllWindows` in that state visibly reorders all windows.
- A reclassified overlay must never be destroyed directly. Restore its original tao class and call `destroy()` inside the same guarded main-thread closure.
- Input translator teardown and Lens-family launch behavior are separate concerns: fixing IME teardown must not add foreground activation to screenshot launch paths.
- A tray left-click is an intentional transition to Chat. If a Lens-family overlay is active, call `lens_close(app.clone())` synchronously before `open_chat_window(app)` so an always-on-top overlay cannot cover Chat.

## 4. Validation & Error Matrix

| Condition | Required behavior |
|---|---|
| Hidden overlay cold creation | Kivio does not become frontmost and Chat ordering does not change |
| Overlay needs keyboard input | Make the panel key and set WKWebView first responder without activating Kivio |
| Recorded app is already frontmost | Clear the slot and skip activation |
| Recorded app is no longer frontmost | Restore it on the AppKit main thread |
| Original overlay class is available | Restore class, then destroy in the same guarded main-thread closure |
| Original class or main-thread scheduling is unavailable | Hide as a safe fallback; do not destroy from an unsafe thread |
| Tray left-click while Lens/translate is visible | Destroy the active overlay through `lens_close`, then reveal Chat |

## 5. Good / Base / Bad Cases

- Good: Chat is visible, another app is frontmost, and Lens opens without bringing Chat forward or flashing desktop windows.
- Base: Chat is hidden and all screenshot modes repeatedly open, close, and cold-create without changing the frontmost app.
- Bad: Launch code activates the previous app before or after showing the panel; macOS reorders all of that app's windows and the desktop flashes.
- Bad: A custom `KivioOverlayPanel` is passed directly to tao/WebKit teardown; an Objective-C KVO exception crosses Rust and aborts the process.
- Bad: Tray click emits an asynchronous frontend close request and immediately opens Chat; the old overlay can remain above the Chat window.

## 6. Tests Required

- Manually cold-launch Lens, screenshot translation, and replace translation with Chat both visible and hidden; assert no desktop flash and no Chat reorder.
- Close each overlay with Escape, reopen it at least five times, and assert the Kivio PID remains alive.
- Run input translation with an active Chinese IME composition, then test Escape, shortcut-toggle cancellation, and Enter submission; assert the WebView is destroyed and the process remains alive.
- Open Lens and quick translation, then left-click the tray icon; assert the overlay is destroyed before Chat becomes visible and cannot cover it.
- Run `cargo check --manifest-path src-tauri/Cargo.toml`, `npm run lint`, and `npm run typecheck`.

## 7. Wrong vs Correct

### Wrong

```rust
let window = WebviewWindowBuilder::new(app, label, url)
    .visible(false)
    .build()?;
reassert_previous_frontmost_app(app, slot);
show_overlay_panel(&window, true);
reassert_previous_frontmost_app(app, slot);
```

### Correct

```rust
let window = WebviewWindowBuilder::new(app, label, url)
    .focused(false)
    .visible(false)
    .build()?;
ensure_overlay_panel(&window);
show_overlay_panel(&window, true);
```

Tray transition:

```rust
if lens_is_active(app) {
    lens_close(app.clone())?;
}
open_chat_window(app)?;
```
