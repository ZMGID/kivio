# Windows Frameless Window Border and Shadow Research

## Research Goal

Determine the safest way to restore visible borders and shadows for Kivio's Windows frameless Tauri windows without regressing transparency, custom chrome, Lens overlays, resizing, or macOS behavior.

## Commands Used

```bash
smart-search doctor --format json
smart-search exa-search "Tauri v2 window shadow transparent decorations WebviewWindowBuilder" --num-results 6 --include-text --include-highlights --include-domains tauri.app docs.rs --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/01-tauri-window-shadow-search.json
smart-search exa-search "tao Windows undecorated shadow with_undecorated_shadow set_undecorated_shadow" --num-results 6 --include-text --include-highlights --include-domains docs.rs github.com --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/02-tao-undecorated-shadow-search.json
smart-search exa-search "wry transparent window Windows undecorated shadow WebView2 transparent background" --num-results 6 --include-text --include-highlights --include-domains docs.rs github.com --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/03-wry-transparent-window-search.json
smart-search fetch "https://docs.rs/tauri/latest/tauri/window/struct.WindowBuilder.html" --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/04-tauri-window-builder-fetch.json
smart-search fetch "https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html" --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/05-tauri-webview-window-builder-fetch.json
smart-search fetch "https://v2.tauri.app/learn/window-customization/" --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/06-tauri-window-customization-fetch.json
smart-search fetch "https://docs.rs/tao/latest/tao/window/struct.WindowBuilder.html" --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/07-tao-window-builder-fetch.json
smart-search exa-search "docs.rs tao platform windows WindowBuilderExtWindows with_undecorated_shadow set_undecorated_shadow" --num-results 5 --include-text --include-highlights --include-domains docs.rs --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/08-tao-windows-ext-search.json
smart-search fetch "https://docs.rs/tao/latest/x86_64-pc-windows-msvc/tao/platform/windows/trait.WindowBuilderExtWindows.html" --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/09-tao-window-builder-ext-windows-fetch.json
smart-search fetch "https://docs.rs/tao/latest/x86_64-pc-windows-msvc/tao/platform/windows/trait.WindowExtWindows.html" --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/10-tao-window-ext-windows-fetch.json
smart-search fetch "https://github.com/tauri-apps/wry/issues/1026" --format json --output .trellis/tasks/06-10-fix-windows-frameless-window-borders/research/evidence/11-wry-transparency-shadow-issue-fetch.json
```

## Sources

* Tauri Window Customization guide: https://v2.tauri.app/learn/window-customization/
* Tauri `WindowBuilder` docs: https://docs.rs/tauri/latest/tauri/window/struct.WindowBuilder.html
* Tauri `WebviewWindowBuilder` docs: https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html
* Tao `WindowBuilder` docs: https://docs.rs/tao/latest/tao/window/struct.WindowBuilder.html
* Tao Windows builder extension: https://docs.rs/tao/latest/x86_64-pc-windows-msvc/tao/platform/windows/trait.WindowBuilderExtWindows.html
* Tao Windows runtime extension: https://docs.rs/tao/latest/x86_64-pc-windows-msvc/tao/platform/windows/trait.WindowExtWindows.html
* Wry Windows transparency issue: https://github.com/tauri-apps/wry/issues/1026

## Findings

* Tauri's customization guide presents `decorations: false` as the normal path for custom titlebars, so Kivio's self-drawn chrome is aligned with Tauri's intended custom-window model.
* Tauri `WindowBuilder::shadow(true)` explicitly supports shadows for undecorated windows on Windows, but documents side effects: it can add a 1px white border and Windows 11 rounded corners.
* Tao exposes the same lower-level Windows behavior through `with_undecorated_shadow` and `set_undecorated_shadow`; its docs say the drop shadow is hidden by default and enabling it can create a thin 1px top line.
* Tauri/Tao docs both warn that `background_color` alpha is ignored at the native window layer on Windows. Transparent windows must therefore be treated as a layered combination of native window transparency, webview transparency, and HTML/CSS transparency.
* Wry issue history shows that Windows transparency and shadow behavior can interact in surprising ways. This argues for a targeted rollout rather than enabling native shadow on every frameless window type at once.
* Tao upstream has recent Windows work around undecorated-shadow size and resize insets. This suggests the native path is supported, but still needs manual verification on Windows 10 and Windows 11.

## Current Repo Constraints

* `src-tauri/tauri.conf.json` creates the `main` translator window with `decorations: false`, `transparent: true`, and `shadow: false`.
* `src-tauri/src/windows.rs` sets Chat and Lens windows to frameless transparent windows on non-macOS and explicitly disables shadows.
* `src/index.css` already has a reusable `.window-container` + `.window-frosted` pattern for small floating windows; this reserves 16px transparent padding so CSS shadows are not clipped.
* Chat uses `.chat-window-host` and `.chat-window-shell`, but the shell currently fills the entire native window, so its CSS shadow cannot spread outside the real window rectangle.
* Lens select mode is a full-screen transparent overlay; enabling native window shadow globally for the Lens window risks drawing unwanted edges around the screen overlay.

## Feasible Approaches

### Approach A: Native Windows Undecorated Shadow

Enable Tauri/Tao shadow for undecorated Windows windows, keeping custom chrome.

Pros:
* Uses platform-supported shadow behavior.
* Restores native depth and potentially Windows 11 rounded-corner integration.
* Minimal frontend layout churn for Chat.

Cons:
* Tauri/Tao document a 1px white/top line side effect.
* Native shadow can affect window bounds, resize hit testing, and visual appearance across Windows versions.
* Unsafe to enable blindly for Lens select mode because it is a full-screen transparent overlay.

### Approach B: CSS Shell Padding and Stronger Visual Tokens

Keep native shadows disabled, but create a real transparent margin inside the webview for non-maximized Windows windows and strengthen the self-drawn border/shadow tokens.

Pros:
* Fully controlled by existing frontend CSS and platform detection.
* Avoids native 1px line and WebView2 transparency/shadow edge cases.
* Can be applied consistently across Chat, translator, and Lens floating cards.

Cons:
* Requires layout/min-size adjustments so Chat content does not shrink unexpectedly.
* CSS shadows still exist only inside the native transparent window; they do not behave exactly like OS shadows.
* More frontend work than flipping `shadow(true)`.

### Approach C: Use Native Decorations for Chat

Restore `decorations(true)` for Chat on Windows and rely on normal Windows borders/shadows.

Pros:
* Most native behavior for resizing, taskbar, shadow, and focus.
* Least likely to have transparent-window clipping problems.

Cons:
* Conflicts with Kivio's custom Chat chrome and current WindowControls.
* Would create a visible native titlebar unless the UI is redesigned.
* Not appropriate for translator or Lens floating windows.

## Recommendation

Use a hybrid implementation:

* Make CSS shell padding and stronger visual tokens the baseline fix for Windows frameless windows.
* Consider native undecorated shadow only for ordinary non-overlay windows, especially Chat, after targeted Windows manual testing.
* Do not enable native shadow for Lens select/fullscreen overlay. Lens floating answer and translate cards should use CSS-only borders and shadows.
* Keep macOS behavior unchanged because Chat already uses native overlay titlebar and system shadow there.

## Manual Verification Needed

* Windows 11: Chat normal, resized, maximized, restored, dark/light themes.
* Windows 10 if available: same Chat checks, because rounded-corner/shadow behavior differs.
* Translator popover: verify no clipped corners, no invisible hit area surprises, and no excessive size increase.
* Lens select mode: verify no full-screen border/shadow around the overlay.
* Lens ready/answer/translate states: verify floating cards have visible boundaries without blocking click/drag behavior.
