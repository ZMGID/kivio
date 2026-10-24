//! Platform selection capture shared by desktop hosts.
use std::time::Duration;
use arboard::Clipboard;
#[cfg(target_os = "macos")]
use crate::platform_permissions::check_accessibility;
/// 模拟一次 Cmd+C(macOS)/Ctrl+C(Windows)。
/// 用于 Lens 启动时把前台 App 的选中文本拷进剪贴板。
/// macOS：直接走 CGEvent（不走 AppleScript），用 Private state source 避免与用户当前
/// 仍按住的热键修饰键(Cmd/Shift/Option)合并出 Cmd+Shift+C 之类的组合。
fn send_copy_shortcut() {
    #[cfg(target_os = "macos")]
    {
        if !check_accessibility(true) {
            eprintln!("[lens-capture] Accessibility permission missing for copy shortcut");
            return;
        }
        use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

        let source = match CGEventSource::new(CGEventSourceStateID::Private) {
            Ok(s) => s,
            Err(_) => {
                eprintln!("[lens-capture] CGEventSource::new(Private) failed");
                return;
            }
        };

        // ANSI 'c' = keycode 8
        const KEY_C: core_graphics::event::CGKeyCode = 8;
        let down = match CGEvent::new_keyboard_event(source.clone(), KEY_C, true) {
            Ok(ev) => ev,
            Err(_) => {
                eprintln!("[lens-capture] CGEvent::new_keyboard_event(down) failed");
                return;
            }
        };
        down.set_flags(CGEventFlags::CGEventFlagCommand);
        down.post(CGEventTapLocation::HID);

        let up = match CGEvent::new_keyboard_event(source, KEY_C, false) {
            Ok(ev) => ev,
            Err(_) => {
                eprintln!("[lens-capture] CGEvent::new_keyboard_event(up) failed");
                return;
            }
        };
        up.set_flags(CGEventFlags::CGEventFlagCommand);
        up.post(CGEventTapLocation::HID);
    }
    #[cfg(target_os = "windows")]
    {
        use enigo::{Enigo, Key, KeyboardControllable};
        let mut enigo = Enigo::new();
        enigo.key_down(Key::Control);
        enigo.key_click(Key::Layout('c'));
        enigo.key_up(Key::Control);
    }
}

/// Accessibility 选区读取的三态结果。
/// - `Text(s)`：AX 取到非空选区。
/// - `Empty`：AX 可用且确认当前没有选区（元素支持选区属性但内容为空），
///   据此可以直接判定无选区，**跳过 Cmd+C 兜底**（消除空选区时原生 App 的系统提示音）。
/// - `Unavailable`：AX 无权限 / 无 focused element / 元素不支持该属性 / 其它错误——
///   无法判定，保留 Cmd+C 兜底（浏览器/Electron/终端等不暴露 AX 的 App，行为不变）。
enum AxSelection {
    Text(String),
    Empty,
    Unavailable,
}

/// macOS: 直接从当前前台控件读取 Accessibility selected text。
/// 这条路径不碰剪贴板，也不受 Lens 热键仍按住的 Cmd/Shift/G 干扰。
#[cfg(target_os = "macos")]
fn read_accessibility_selected_text() -> AxSelection {
    if !check_accessibility(false) {
        eprintln!("[lens-capture] AX unavailable: accessibility permission missing");
        return AxSelection::Unavailable;
    }

    use core_foundation::{
        base::{CFRelease, CFType, CFTypeRef, TCFType},
        string::{CFString, CFStringRef},
    };

    type AXUIElementRef = *const libc::c_void;
    type AXError = i32;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
    }

    const AX_ERROR_SUCCESS: AXError = 0;
    // kAXErrorNoValue：元素支持该属性，但当前没有值（即确认无选区）。
    const AX_ERROR_NO_VALUE: AXError = -25212;

    unsafe {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            eprintln!("[lens-capture] AX unavailable: system-wide element null");
            return AxSelection::Unavailable;
        }

        let focused_attr = CFString::new("AXFocusedUIElement");
        let mut focused_ref: CFTypeRef = std::ptr::null();
        let focused_err = AXUIElementCopyAttributeValue(
            system,
            focused_attr.as_concrete_TypeRef(),
            &mut focused_ref,
        );
        CFRelease(system as CFTypeRef);
        if focused_err != AX_ERROR_SUCCESS || focused_ref.is_null() {
            eprintln!("[lens-capture] AX unavailable: no focused element (err={focused_err})");
            return AxSelection::Unavailable;
        }
        let focused = CFType::wrap_under_create_rule(focused_ref);

        let selected_attr = CFString::new("AXSelectedText");
        let mut selected_ref: CFTypeRef = std::ptr::null();
        let selected_err = AXUIElementCopyAttributeValue(
            focused.as_CFTypeRef() as AXUIElementRef,
            selected_attr.as_concrete_TypeRef(),
            &mut selected_ref,
        );

        // 元素支持该属性但当前无值（无选区）→ 确认无选区。
        if selected_err == AX_ERROR_NO_VALUE {
            eprintln!("[lens-capture] AX confirmed empty selection (kAXErrorNoValue)");
            return AxSelection::Empty;
        }

        if selected_err != AX_ERROR_SUCCESS || selected_ref.is_null() {
            // 元素不支持该属性 / 其它错误 / 空指针 → 无法判定，落 Cmd+C 兜底。
            eprintln!("[lens-capture] AX unavailable: AXSelectedText err={selected_err}");
            return AxSelection::Unavailable;
        }

        let selected = CFType::wrap_under_create_rule(selected_ref);
        match selected.downcast_into::<CFString>() {
            Some(cf) => {
                let text = cf.to_string();
                if text.trim().is_empty() {
                    eprintln!("[lens-capture] AX confirmed empty selection (empty AXSelectedText)");
                    AxSelection::Empty
                } else {
                    AxSelection::Text(text)
                }
            }
            None => {
                eprintln!("[lens-capture] AX unavailable: AXSelectedText not a CFString");
                AxSelection::Unavailable
            }
        }
    }
}

/// Windows: 通过 UI Automation TextPattern 直接读取当前前台控件的选区。
/// 这条路径不碰剪贴板；不支持 TextPattern 的控件会自动降级到 Ctrl+C fallback。
/// 三态语义（与 macOS 分支对齐）：
///   - TextPattern 明确可用（GetCurrentPattern + cast + GetSelection 均成功）且收集到的非空文本为空
///     → `Empty`（确认无选区，上层跳过 Ctrl+C → 原生输入框不再响）。
///   - 任一环节失败（CoCreateInstance / GetFocusedElement / GetCurrentPattern / cast / GetSelection 等）
///     → `Unavailable`（保持 Ctrl+C 兜底，不碰浏览器/Electron 等不支持 TextPattern 的路径）。
///   - 有非空选区 → `Text`。
/// 保守原则：任何不确定/失败一律 `Unavailable`，只有 TextPattern 明确可用且选区确为空才 `Empty`，
/// 避免 UIA underreport 时漏抓真实选区。
#[cfg(target_os = "windows")]
fn read_accessibility_selected_text() -> AxSelection {
    use ::windows::{
        core::Interface,
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            UI::Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
            },
        },
    };

    unsafe {
        let init_result = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if init_result.is_err() && init_result != RPC_E_CHANGED_MODE {
            eprintln!("[lens-capture] CoInitializeEx failed: {init_result:?}");
            return AxSelection::Unavailable;
        }
        let should_uninitialize = init_result.is_ok();

        // closure 直接产出三态 AxSelection：
        // 拿不到 TextPattern / GetSelection 失败 → Unavailable；
        // TextPattern 明确可用且选区确为空 → Empty；否则 → Text。
        let result = (|| -> AxSelection {
            let automation: IUIAutomation =
                match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("[lens-capture] UIA unavailable: CoCreateInstance failed: {e:?}");
                        return AxSelection::Unavailable;
                    }
                };
            let focused = match automation.GetFocusedElement() {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("[lens-capture] UIA unavailable: GetFocusedElement failed: {e:?}");
                    return AxSelection::Unavailable;
                }
            };
            // GetCurrentPattern + cast 任一失败 → 控件不支持 TextPattern（如浏览器/Electron）→ 兜底。
            let pattern_unknown = match focused.GetCurrentPattern(UIA_TextPatternId) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "[lens-capture] UIA unavailable: GetCurrentPattern(Text) failed: {e:?}"
                    );
                    return AxSelection::Unavailable;
                }
            };
            let pattern: IUIAutomationTextPattern = match pattern_unknown.cast() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("[lens-capture] UIA unavailable: TextPattern cast failed: {e:?}");
                    return AxSelection::Unavailable;
                }
            };
            // 到这里 TextPattern 明确可用；GetSelection 失败仍视为无法判定 → 兜底。
            let ranges = match pattern.GetSelection() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[lens-capture] UIA unavailable: GetSelection failed: {e:?}");
                    return AxSelection::Unavailable;
                }
            };
            let count = ranges.Length().unwrap_or(0).max(0);
            let mut parts = Vec::new();

            for index in 0..count {
                let range = match ranges.GetElement(index) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let text = match range.GetText(-1) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let text = text.to_string();
                if !text.trim().is_empty() {
                    parts.push(text);
                }
            }

            if parts.is_empty() {
                // TextPattern 明确可用 + 选区确为空 → Empty（上层跳过 Ctrl+C → 不响）。
                eprintln!("[lens-capture] UIA confirmed empty selection (TextPattern available)");
                AxSelection::Empty
            } else {
                AxSelection::Text(parts.join("\n"))
            }
        })();

        if should_uninitialize {
            CoUninitialize();
        }

        if let AxSelection::Text(ref text) = result {
            eprintln!(
                "[lens-capture] UIA selected text captured len={}",
                text.len()
            );
        }

        result
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_accessibility_selected_text() -> AxSelection {
    AxSelection::Unavailable
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn clipboard_change_count() -> Option<i64> {
    use cocoa::{
        appkit::NSPasteboard,
        base::{id, nil},
    };
    unsafe {
        let pasteboard = <id as NSPasteboard>::generalPasteboard(nil);
        if pasteboard == nil {
            None
        } else {
            Some(pasteboard.changeCount() as i64)
        }
    }
}

#[cfg(target_os = "windows")]
fn clipboard_change_count() -> Option<i64> {
    use ::windows::Win32::System::DataExchange::GetClipboardSequenceNumber;
    let count = unsafe { GetClipboardSequenceNumber() };
    if count == 0 {
        None
    } else {
        Some(i64::from(count))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn clipboard_change_count() -> Option<i64> {
    None
}

#[cfg(target_os = "macos")]
fn wait_for_copy_shortcut_modifiers_to_clear(timeout: Duration) {
    use core_graphics::{event::CGEventFlags, event_source::CGEventSourceStateID};
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceFlagsState(state_id: CGEventSourceStateID) -> u64;
    }

    let mask = CGEventFlags::CGEventFlagShift.bits()
        | CGEventFlags::CGEventFlagControl.bits()
        | CGEventFlags::CGEventFlagAlternate.bits()
        | CGEventFlags::CGEventFlagCommand.bits();
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let flags = unsafe { CGEventSourceFlagsState(CGEventSourceStateID::CombinedSessionState) };
        if flags & mask == 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(target_os = "windows")]
fn wait_for_copy_shortcut_modifiers_to_clear(timeout: Duration) {
    use ::windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    let keys = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        let pressed = keys
            .iter()
            .any(|key| unsafe { (GetAsyncKeyState(key.0 as i32) as u16 & 0x8000) != 0 });
        if !pressed {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn wait_for_copy_shortcut_modifiers_to_clear(timeout: Duration) {
    std::thread::sleep(timeout.min(Duration::from_millis(120)));
}

/// 在前一个 App 仍持焦点时把选中文本读出来，失败时才模拟 Cmd+C/Ctrl+C 兜底。
/// 失败/Accessibility 权限缺失/剪贴板为非文本格式 → 一律静默降级返回 None。
/// 调用方负责确保此函数在 Lens 窗口 show() 之前执行。
pub fn capture_active_selection() -> Option<String> {
    match read_accessibility_selected_text() {
        AxSelection::Text(text) => {
            eprintln!(
                "[lens-capture] selected text captured via Accessibility len={}",
                text.len()
            );
            return Some(text);
        }
        AxSelection::Empty => {
            // AX 确认当前无选区：直接判定无选区，跳过 Cmd+C 兜底，避免空选区时原生 App 的系统提示音。
            eprintln!("[lens-capture] AX confirmed no selection, skipping Cmd+C fallback");
            return None;
        }
        AxSelection::Unavailable => {
            // AX 无法判定：继续走下面的剪贴板 snapshot + Cmd+C 兜底逻辑。
        }
    }

    // snapshot 原剪贴板文本(仅 text)。若是图片/文件/空，snapshot=None，事后不还原。
    let snapshot: Option<String> = Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok());
    let before_change_count = clipboard_change_count();
    eprintln!(
        "[lens-capture] snapshot present={} len={} change_count={:?}",
        snapshot.is_some(),
        snapshot.as_ref().map(|s| s.len()).unwrap_or(0),
        before_change_count,
    );

    // 等用户松开 Lens 热键修饰键，避免 Cmd+C 与残留 Shift 等组合成 Cmd+Shift+C。
    wait_for_copy_shortcut_modifiers_to_clear(Duration::from_millis(450));
    send_copy_shortcut();
    std::thread::sleep(Duration::from_millis(150));

    let captured: Option<String> = Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok());
    let text_changed = match (&snapshot, &captured) {
        (Some(a), Some(b)) => a != b,
        (None, Some(_)) => true,
        _ => false,
    };
    let after_change_count = clipboard_change_count();
    let pasteboard_changed = match (before_change_count, after_change_count) {
        (Some(before), Some(after)) => before != after,
        _ => false,
    };
    eprintln!(
    "[lens-capture] captured present={} len={} text_changed={} pasteboard_changed={} change_count={:?}",
    captured.is_some(),
    captured.as_ref().map(|s| s.len()).unwrap_or(0),
    text_changed,
    pasteboard_changed,
    after_change_count,
  );

    if let Some(orig) = &snapshot {
        if let Ok(mut cb) = Clipboard::new() {
            let _ = cb.set_text(orig.clone());
        }
    }

    // pasteboard changeCount 覆盖"选中文本与原剪贴板文本完全相同"的情况。
    if !text_changed && !pasteboard_changed {
        return None;
    }
    match captured {
        Some(t) if !t.trim().is_empty() => Some(t),
        _ => None,
    }
}
