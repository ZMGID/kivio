#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

mod api;
mod apple_intelligence;
mod lens;
#[cfg(target_os = "macos")]
mod macos_ocr;
mod prompts;
#[cfg(target_os = "macos")]
mod sck;
mod screenshot;
mod settings;
mod state;
mod tesseract;
mod utils;
mod windows;
#[cfg(target_os = "windows")]
mod windows_ocr;

use std::{
  collections::{HashMap, HashSet},
  fs,
  io::Write,
  path::PathBuf,
  sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex, RwLock,
  },
  time::Duration,
};

use arboard::Clipboard;
use base64::{engine::general_purpose, Engine as _};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutoStartManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_single_instance::init as init_single_instance;
use uuid::Uuid;

use api::{
  build_http_client, build_ocr_request_body, call_openai_ocr, call_openai_text, call_vision_api,
  effective_retry_attempts, resolve_provider_credentials, send_with_failover, send_with_retry,
  stream_chat_call, stream_translate_combined, ProviderConnectionInput,
};
use prompts::{
  build_combined_translate_prompt, build_ocr_direct_translation_prompt,
  build_screenshot_translation_prompt, build_translation_prompt, COMBINED_TRANSLATE_SEPARATOR,
  DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE, DEFAULT_TRANSLATION_TEMPLATE,
};
use screenshot::{cleanup_orphan_temp_files, cleanup_temp_file};
use settings::{
  default_question_prompt, default_system_prompt, load_settings, persist_settings,
  sanitize_settings, ExplainMessage, OcrMode, Settings,
};
use state::AppState;
use utils::{language_name, resolve_target_lang};
#[cfg(target_os = "macos")]
use windows::apply_macos_workspace_behavior;
use windows::{ensure_main_window, get_main_window};

#[cfg(target_os = "windows")]
use xcap::Monitor;

/// 自启动参数，用于区分用户手动启动和系统自动启动
const AUTOSTART_ARG: &str = "--from-autostart";

/// 应用开机自启动设置
/// 根据传入的 enabled 参数启用或禁用自动启动
fn apply_launch_at_startup(app: &AppHandle, enabled: bool) -> Result<(), String> {
  let auto_launch = app.autolaunch();
  let current = auto_launch.is_enabled().map_err(|e| e.to_string())?;

  if enabled && !current {
    auto_launch.enable().map_err(|e| e.to_string())?;
  } else if !enabled && current {
    auto_launch.disable().map_err(|e| e.to_string())?;
  }

  Ok(())
}

/// 获取当前应用设置
#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
  state.settings_read().clone()
}

/// 获取默认提示词模板
/// 返回翻译模板、截图翻译模板，以及 lens 视觉对话用的系统/提问提示词
#[tauri::command]
fn get_default_prompt_templates() -> serde_json::Value {
  serde_json::json!({
    "translationTemplate": DEFAULT_TRANSLATION_TEMPLATE,
    "screenshotTranslationTemplate": DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE,
    "lensPrompts": {
      "zh": {
        "system": default_system_prompt("zh", true),
        "question": default_question_prompt("zh", true)
      },
      "en": {
        "system": default_system_prompt("en", true),
        "question": default_question_prompt("en", true)
      }
    }
  })
}

/// 保存设置
/// 先对传入的设置进行清理（sanitize），然后应用开机自启动、重新注册热键、持久化设置、更新托盘菜单
/// 如果热键注册失败，则回滚运行时设置到之前的状态
#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<(), String> {
  let previous_settings = state.settings_read().clone();
  let sanitized = sanitize_settings(settings);
  apply_launch_at_startup(&app, sanitized.launch_at_startup)?;
  {
    let mut guard = state.settings_write();
    *guard = sanitized.clone();
  }

  if let Err(err) = register_hotkeys(&app) {
    restore_runtime_settings(&app, &state, &previous_settings);
    return Err(err);
  }

  if let Err(err) = persist_settings(&app, &sanitized) {
    eprintln!("Failed to save settings: {err}");
    restore_runtime_settings(&app, &state, &previous_settings);
    return Err(err);
  }

  if let Err(err) = setup_tray(&app) {
    eprintln!("Failed to update tray: {err}");
  }

  Ok(())
}

/// 翻译文本命令
/// 根据设置中的翻译供应商和模型进行翻译；如果 API Key 为空则返回提示信息
#[tauri::command]
async fn translate_text(state: State<'_, AppState>, text: String) -> Result<String, String> {
  let trimmed = text.trim();
  if trimmed.is_empty() {
    return Ok("".to_string());
  }

  let settings = state.settings_read().clone();
  let provider = settings.get_provider(&settings.translator_provider_id)
    .ok_or_else(|| "Translator provider not found".to_string())?;

  if provider.api_keys.is_empty() {
    return Ok("Missing API Key".to_string());
  }

  let target_lang = resolve_target_lang(&settings.target_lang, trimmed);
  let lang_name = language_name(&target_lang).to_string();
  let prompt = build_translation_prompt(
    trimmed,
    &lang_name,
    settings.translator_prompt.as_deref(),
  );

  let retry_attempts = effective_retry_attempts(&settings);
  // 主翻译路径默认关思考：reasoning 模型对单句翻译几乎无质量收益但显著拖慢；非 reasoning 模型该字段被忽略
  call_openai_text(
    &state,
    provider,
    &settings.translator_model,
    prompt,
    retry_attempts,
    false,
  )
  .await
}

/// 提交翻译结果
/// 将翻译后的文本写入剪贴板，隐藏主窗口，如果启用了自动粘贴则发送粘贴快捷键到之前的应用
#[tauri::command]
async fn commit_translation(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<(), String> {
  if text.trim().is_empty() {
    return Ok(());
  }

  let auto_paste = state.settings_read().auto_paste;
  let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
  clipboard.set_text(text).map_err(|e| e.to_string())?;

  // 先隐藏窗口，让焦点回到之前的应用
  if let Some(window) = get_main_window(&app) {
    let _ = window.hide();
  }

  #[cfg(target_os = "macos")]
  #[allow(deprecated, unexpected_cfgs)]
  unsafe {
    use cocoa::base::{id, nil};
    use objc::{class, msg_send, sel, sel_impl};
    let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
    let _: () = msg_send![ns_app, hide: nil];
  }

  if auto_paste {
    // 增加延迟以确保焦点切换完成
    tokio::time::sleep(Duration::from_millis(600)).await;
    send_paste_shortcut();
  }

  Ok(())
}

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

/// macOS: 直接从当前前台控件读取 Accessibility selected text。
/// 这条路径不碰剪贴板，也不受 Lens 热键仍按住的 Cmd/Shift/G 干扰。
#[cfg(target_os = "macos")]
fn read_accessibility_selected_text() -> Option<String> {
  if !check_accessibility(false) {
    return None;
  }

  use core_foundation::{
    base::{CFRelease, CFType, CFTypeRef, TCFType},
    string::{CFString, CFStringRef},
  };

  type AXUIElementRef = *const libc::c_void;
  type AXError = i32;

  #[link(name = "ApplicationServices", kind = "framework")]
  extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
      element: AXUIElementRef,
      attribute: CFStringRef,
      value: *mut CFTypeRef,
    ) -> AXError;
  }

  const AX_ERROR_SUCCESS: AXError = 0;

  unsafe {
    let system = AXUIElementCreateSystemWide();
    if system.is_null() {
      return None;
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
      return None;
    }
    let focused = CFType::wrap_under_create_rule(focused_ref);

    let selected_attr = CFString::new("AXSelectedText");
    let mut selected_ref: CFTypeRef = std::ptr::null();
    let selected_err = AXUIElementCopyAttributeValue(
      focused.as_CFTypeRef() as AXUIElementRef,
      selected_attr.as_concrete_TypeRef(),
      &mut selected_ref,
    );
    if selected_err != AX_ERROR_SUCCESS || selected_ref.is_null() {
      return None;
    }

    let selected = CFType::wrap_under_create_rule(selected_ref);
    let text = selected.downcast_into::<CFString>()?.to_string();
    if text.trim().is_empty() {
      None
    } else {
      Some(text)
    }
  }
}

/// Windows: 通过 UI Automation TextPattern 直接读取当前前台控件的选区。
/// 这条路径不碰剪贴板；不支持 TextPattern 的控件会自动降级到 Ctrl+C fallback。
#[cfg(target_os = "windows")]
fn read_accessibility_selected_text() -> Option<String> {
  use ::windows::{
    core::Interface,
    Win32::{
      Foundation::RPC_E_CHANGED_MODE,
      System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED},
      UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId},
    },
  };

  unsafe {
    let init_result = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    if init_result.is_err() && init_result != RPC_E_CHANGED_MODE {
      eprintln!("[lens-capture] CoInitializeEx failed: {init_result:?}");
      return None;
    }
    let should_uninitialize = init_result.is_ok();

    let result = (|| {
      let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
      let focused = automation.GetFocusedElement().ok()?;
      let pattern: IUIAutomationTextPattern = focused
        .GetCurrentPattern(UIA_TextPatternId)
        .ok()?
        .cast()
        .ok()?;
      let ranges = pattern.GetSelection().ok()?;
      let count = ranges.Length().ok()?.max(0);
      let mut parts = Vec::new();

      for index in 0..count {
        let range = ranges.GetElement(index).ok()?;
        let text = range.GetText(-1).ok()?;
        let text = String::from_utf16_lossy(&text);
        if !text.trim().is_empty() {
          parts.push(text);
        }
      }

      if parts.is_empty() {
        None
      } else {
        Some(parts.join("\n"))
      }
    })();

    if should_uninitialize {
      CoUninitialize();
    }

    result
  }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read_accessibility_selected_text() -> Option<String> {
  None
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
  extern "C" {
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
    let pressed = keys.iter().any(|key| unsafe {
      (GetAsyncKeyState(key.0 as i32) as u16 & 0x8000) != 0
    });
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
fn capture_active_selection() -> Option<String> {
  if let Some(text) = read_accessibility_selected_text() {
    eprintln!(
      "[lens-capture] selected text captured via Accessibility len={}",
      text.len()
    );
    return Some(text);
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

/// 取走 Rust 端在 lens_request_internal 中暂存的 selection 文本。
/// 取一次清一次：前端 enterSelect 调用，第二次调用立即返回空串。
#[tauri::command]
fn take_lens_selection(state: State<'_, AppState>) -> Result<String, String> {
  match state.pending_selection.lock() {
    Ok(mut guard) => Ok(guard.take().unwrap_or_default()),
    Err(_) => Ok(String::new()),
  }
}

/// 使用系统默认浏览器打开外部链接（仅限 https）
#[tauri::command]
#[allow(deprecated)]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
  if !url.starts_with("https://") {
    return Err("Invalid URL".to_string());
  }

  app.shell().open(url, None).map_err(|e| e.to_string())
}

/// 查询 Apple Intelligence 预设是否应在设置页中显示。
/// 为避免打开设置页时就拉起 sidecar，这里只做轻量本地判断：
/// - 非 macOS → false
/// - sidecar 资源二进制不存在 → false
/// - 其余情况 → true（真正可用性在首次调用时由 sidecar ready 结果决定）
#[tauri::command]
fn apple_intelligence_available(state: State<AppState>) -> bool {
  #[cfg(not(target_os = "macos"))]
  {
    let _ = state;
    return false;
  }
  #[cfg(target_os = "macos")]
  {
    if let Some(app) = state.apple_intelligence.app_handle() {
      let path = app.path().resource_dir().ok().map(|dir| {
        let name = if cfg!(target_os = "windows") {
          "binaries/kivio-ai-helper.exe"
        } else {
          "binaries/kivio-ai-helper"
        };
        dir.join(name)
      });
      return path
        .and_then(|p| p.metadata().ok())
        .map(|m| m.is_file() && m.len() > 0)
        .unwrap_or(false);
    }
    false
  }
}

// ===== Tesseract 离线 OCR 命令 =====
//
// status: 探测 PATH/标准位置上有没有 tesseract,返回版本 + 路径 + 可用包管理器名。
// install: 调平台包管理器(brew/winget/scoop/choco)一键装 tesseract,~1-3 分钟。

/// 查询 Tesseract 是否可用 + 当前平台支持的包管理器。
#[tauri::command]
fn tesseract_status(state: State<AppState>) -> tesseract::TesseractStatus {
  state.tesseract.status()
}

/// 一键安装 tesseract。阻塞到包管理器跑完(成功或失败),前端转圈圈等返回。
#[tauri::command]
async fn tesseract_install(
  state: State<'_, AppState>,
) -> Result<tesseract::TesseractInstallResult, String> {
  let client = state.tesseract.clone();
  Ok(client.install().await)
}

/// 调 GitHub Releases API 检查最新版本
/// 发现新版只返回提示信息，让前端弹"去 GitHub 下载"按钮（不做自动下载安装，避免引入签名密钥那套）
/// 网络失败 / API 限流时返回 available=false 静默处理，不打扰用户
#[tauri::command]
async fn check_github_latest_release(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
  const REPO: &str = "ZMGID/kivio";
  let url = format!("https://api.github.com/repos/{REPO}/releases/latest");

  let response = state
    .http
    .get(&url)
    // GitHub API 要求显式 User-Agent
    .header("User-Agent", format!("Kivio/{}", env!("CARGO_PKG_VERSION")))
    .header("Accept", "application/vnd.github+json")
    .send()
    .await;

  let response = match response {
    Ok(r) => r,
    Err(_) => return Ok(serde_json::json!({ "available": false })),
  };

  if !response.status().is_success() {
    return Ok(serde_json::json!({ "available": false }));
  }

  let value: serde_json::Value = match response.json().await {
    Ok(v) => v,
    Err(_) => return Ok(serde_json::json!({ "available": false })),
  };

  let tag = value.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
  let html_url = value.get("html_url").and_then(|v| v.as_str()).unwrap_or("");
  let body = value.get("body").and_then(|v| v.as_str()).unwrap_or("");
  let published_at = value.get("published_at").and_then(|v| v.as_str()).unwrap_or("");

  // tag_name 通常是 "v2.5.0"，剥掉前缀 v 再比较
  let latest = tag.trim_start_matches('v');
  let current = env!("CARGO_PKG_VERSION");

  Ok(serde_json::json!({
    "available": is_newer_version(latest, current),
    "version": latest,
    "tag": tag,
    "htmlUrl": html_url,
    "body": body,
    "publishedAt": published_at,
  }))
}

/// 朴素 semver 比较：把 "x.y.z" 拆成数字三元组按字典序比较
/// 不处理 prerelease (-beta) / build metadata (+abc)；返回 latest > current
fn is_newer_version(latest: &str, current: &str) -> bool {
  let parse = |s: &str| -> (u32, u32, u32) {
    let mut it = s.split('.').map(|p| {
      // 截断到第一个非数字（兼容 "1.0.0-beta" 这类）
      p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse::<u32>().unwrap_or(0)
    });
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
  };
  parse(latest) > parse(current)
}

/// 从 release JSON 的 assets 数组里挑出当前平台 + 架构的安装包。
/// 匹配规则：
///   - macOS aarch64 → `.dmg` 文件名包含 aarch64 / arm64
///   - macOS x86_64  → `.dmg` 包含 x64 / x86_64
///   - Windows       → `-setup.exe` 结尾（NSIS，覆盖升级体验比 MSI 顺）
fn pick_release_asset(assets: &[serde_json::Value]) -> Option<(String, String)> {
  let arch = std::env::consts::ARCH;
  for asset in assets {
    let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let url = asset.get("browser_download_url").and_then(|v| v.as_str()).unwrap_or("");
    if name.is_empty() || url.is_empty() { continue; }
    let lower = name.to_ascii_lowercase();
    let matched = if cfg!(target_os = "macos") {
      lower.ends_with(".dmg") && match arch {
        "aarch64" => lower.contains("aarch64") || lower.contains("arm64"),
        _ => lower.contains("x64") || lower.contains("x86_64"),
      }
    } else if cfg!(target_os = "windows") {
      lower.ends_with("-setup.exe")
    } else {
      false
    };
    if matched {
      return Some((name.to_string(), url.to_string()));
    }
  }
  None
}

/// 下载新版本安装包到 OS temp dir，边下边 emit "update-download-progress" 事件。
/// 返回本地文件绝对路径。失败 Err 含详细原因（前端显示）。
#[tauri::command]
async fn download_update_asset(
  app: AppHandle,
  state: State<'_, AppState>,
  version: String,
) -> Result<String, String> {
  const REPO: &str = "ZMGID/kivio";
  let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
  let resp = state.http
    .get(&url)
    .header("User-Agent", format!("Kivio/{}", env!("CARGO_PKG_VERSION")))
    .header("Accept", "application/vnd.github+json")
    .send()
    .await
    .map_err(|e| format!("查询 release 失败: {e}"))?;
  if !resp.status().is_success() {
    return Err(format!("GitHub API 返回 {}", resp.status()));
  }
  let value: serde_json::Value = resp.json().await.map_err(|e| format!("解析 release JSON 失败: {e}"))?;
  let assets = value.get("assets").and_then(|v| v.as_array())
    .ok_or_else(|| "release 没有 assets".to_string())?;
  let (name, asset_url) = pick_release_asset(assets)
    .ok_or_else(|| format!("没有匹配当前平台({}/{})的安装包", std::env::consts::OS, std::env::consts::ARCH))?;

  // 决定本地文件名：保留原扩展名（.dmg / .exe）便于 install 流程根据扩展名判断行为
  let ext = std::path::Path::new(&name).extension()
    .and_then(|e| e.to_str()).unwrap_or("bin");
  let safe_version = version.chars().filter(|c| c.is_alphanumeric() || *c == '.' || *c == '-').collect::<String>();
  let dest = std::env::temp_dir().join(format!("kivio-update-{safe_version}.{ext}"));

  let mut resp = state.http
    .get(&asset_url)
    .header("User-Agent", format!("Kivio/{}", env!("CARGO_PKG_VERSION")))
    .send()
    .await
    .map_err(|e| format!("下载失败: {e}"))?;
  if !resp.status().is_success() {
    return Err(format!("下载返回 {}", resp.status()));
  }
  let total = resp.content_length().unwrap_or(0);
  let mut file = fs::File::create(&dest).map_err(|e| format!("创建文件失败: {e}"))?;
  let mut downloaded: u64 = 0;
  let mut last_emitted_pct: i32 = -1;
  while let Some(chunk) = resp.chunk().await.map_err(|e| format!("读取下载流失败: {e}"))? {
    file.write_all(&chunk).map_err(|e| format!("写入失败: {e}"))?;
    downloaded += chunk.len() as u64;
    let pct = if total > 0 { (downloaded * 100 / total) as i32 } else { 0 };
    // 节流：百分比变化才 emit，避免事件洪水（小 chunk 时容易刷爆）
    if pct != last_emitted_pct {
      last_emitted_pct = pct;
      let _ = app.emit("update-download-progress", serde_json::json!({
        "percent": pct,
        "downloadedBytes": downloaded,
        "totalBytes": total,
      }));
    }
  }
  // 收尾再 emit 一次确保 100% 落地
  let _ = app.emit("update-download-progress", serde_json::json!({
    "percent": 100,
    "downloadedBytes": downloaded,
    "totalBytes": total.max(downloaded),
  }));
  Ok(dest.to_string_lossy().to_string())
}

/// 启动安装包并退出当前应用。
/// - macOS（.dmg）：hdiutil 挂载 → cp Kivio.app 到 /Applications → 卸载 → open 新版 → app.exit(0)
/// - Windows（.exe）：spawn NSIS installer，立即 exit 让 installer 能写 exe
#[tauri::command]
fn install_update_and_quit(app: AppHandle, path: String) -> Result<(), String> {
  let p = std::path::Path::new(&path);
  if !p.exists() { return Err(format!("安装包不存在: {path}")); }

  #[cfg(target_os = "macos")]
  {
    use std::process::Command;
    // 显式指定挂载点（用 UUID 避免与同名 volume 已挂载时的名字冲突）。比解析 `hdiutil attach` 的
    // 默认表格输出鲁棒很多 —— 那个输出列用空格 padding,VolumeName 含空格(如重复挂载产生的
    // "Kivio 1")会被 split_whitespace 截断。
    let mount_id = Uuid::new_v4().to_string();
    let mount_point = std::env::temp_dir().join(format!("kivio-mount-{mount_id}"));
    fs::create_dir_all(&mount_point).map_err(|e| format!("创建挂载目录失败: {e}"))?;
    let mount_str = mount_point.to_string_lossy().to_string();
    let attach = Command::new("hdiutil")
      .args([
        "attach",
        "-nobrowse",
        "-readonly",
        "-mountpoint",
        &mount_str,
        &path,
      ])
      .output()
      .map_err(|e| format!("hdiutil attach 失败: {e}"))?;
    if !attach.status.success() {
      let _ = fs::remove_dir(&mount_point);
      return Err(format!("挂载 DMG 失败: {}", String::from_utf8_lossy(&attach.stderr)));
    }
    // 找挂载点下第一个 .app
    let app_in_dmg = fs::read_dir(&mount_point)
      .map_err(|e| format!("读取挂载点失败: {e}"))?
      .filter_map(|e| e.ok())
      .find(|e| e.path().extension().and_then(|s| s.to_str()) == Some("app"))
      .ok_or_else(|| "DMG 内未找到 .app".to_string())?
      .path();
    let app_name = app_in_dmg.file_name()
      .and_then(|s| s.to_str())
      .ok_or_else(|| "解析 .app 名失败".to_string())?
      .to_string();
    let target = PathBuf::from("/Applications").join(&app_name);
    // 删除旧 app 并 cp 新的（rm -rf 失败也忽略，cp 会用 -R 覆盖）
    let _ = Command::new("rm").args(["-rf", &target.to_string_lossy()]).status();
    let cp = Command::new("cp")
      .args(["-R", &app_in_dmg.to_string_lossy(), &target.to_string_lossy()])
      .status()
      .map_err(|e| format!("cp 失败: {e}"))?;
    if !cp.success() {
      let _ = Command::new("hdiutil").args(["detach", "-force", &mount_str]).status();
      let _ = fs::remove_dir(&mount_point);
      return Err("cp 新版本到 /Applications 失败".to_string());
    }
    // 卸载 + 删除空挂载目录
    let _ = Command::new("hdiutil").args(["detach", "-force", &mount_str]).status();
    let _ = fs::remove_dir(&mount_point);
    // 剥掉 quarantine 属性 —— DMG 文件本身带 com.apple.quarantine,挂载后 .app 继承这个属性,
    // cp 到 /Applications 后 Gatekeeper 看到 quarantine + 未公证 → 静默拦截启动。
    // xattr -rd 递归剥掉,与 README 里那条手动命令等效。
    let _ = Command::new("xattr")
      .args(["-rd", "com.apple.quarantine", &target.to_string_lossy()])
      .status();
    // open -n 强制开新实例
    let _ = Command::new("open").args(["-n", &target.to_string_lossy()]).spawn()
      .map_err(|e| format!("open 新版本失败: {e}"))?;
    app.exit(0);
    return Ok(());
  }

  #[cfg(target_os = "windows")]
  {
    use std::process::Command;
    Command::new(&path)
      .spawn()
      .map_err(|e| format!("启动 installer 失败: {e}"))?;
    app.exit(0);
    return Ok(());
  }

  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  {
    let _ = app;
    Err("当前平台不支持自动安装".to_string())
  }
}

/// 读取截图图片并以 Base64 数据 URL 格式返回（lens ready 态显示缩略图用）
#[tauri::command]
fn explain_read_image(app: AppHandle, state: State<AppState>, image_id: String) -> Result<serde_json::Value, String> {
  let image_path = resolve_explain_image_path(&app, &state, &image_id)?;
  let bytes = fs::read(&image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);
  Ok(serde_json::json!({
    "success": true,
    "data": format!("data:image/png;base64,{base64}")
  }))
}

// ====== Lens 模式命令 ======

/// 把 lens 窗口铺满目标显示器（用于 select 态）。
///
/// 显示器选择优先级：
///   1. 光标所在显示器（正常路径）
///   2. primary monitor（cursor_position 失败 / 无 monitor 匹配光标 — 罕见但
///      合盖切外接、睡眠唤醒后 monitor 列表暂时不一致时会发生）
///   3. 第一个 monitor（极端兜底，primary 也拿不到时）
///
/// 任何兜底都比"什么都不做"强 —— 之前的实现这种情况下窗口停留在上次几何，
/// 用户看到的就是 ready 浮条 / 旧位置，体验远差于跳到 primary。
fn lens_position_fullscreen(app: &AppHandle, window: &WebviewWindow) {
  let cursor_opt = app.cursor_position().ok();
  let monitors = match app.available_monitors() {
    Ok(m) if !m.is_empty() => m,
    Ok(_) => {
      eprintln!("[lens-pos] available_monitors returned empty list");
      return;
    }
    Err(e) => {
      eprintln!("[lens-pos] available_monitors err: {}", e);
      return;
    }
  };

  // 1. 找光标所在的 monitor
  let target = cursor_opt.as_ref().and_then(|cursor| {
    monitors.iter().find(|monitor| {
      let mp = monitor.position();
      let ms = monitor.size();
      let mw = ms.width as i32;
      let mh = ms.height as i32;
      (cursor.x as i32) >= mp.x
        && (cursor.x as i32) < mp.x + mw
        && (cursor.y as i32) >= mp.y
        && (cursor.y as i32) < mp.y + mh
    })
  });

  // 2-3. fallback: primary monitor，再不行第一个 monitor
  let target = target
    .or_else(|| {
      let p = app.primary_monitor().ok().flatten();
      // primary_monitor 返回 Option<Monitor> 而 monitors iter 给的是 &Monitor，
      // 这里需要从 monitors 里按 name 找回相同的 monitor 引用，避免类型不一致
      p.and_then(|prim| monitors.iter().find(|m| m.name() == prim.name()))
    })
    .or_else(|| monitors.first());

  let Some(monitor) = target else {
    eprintln!("[lens-pos] no usable monitor found");
    return;
  };

  let mp = monitor.position();
  let ms = monitor.size();
  let scale = monitor.scale_factor();
  let lx = mp.x as f64 / scale;
  let ly = mp.y as f64 / scale;
  let lw = ms.width as f64 / scale;
  let lh = ms.height as f64 / scale;
  let _ = window.set_position(tauri::LogicalPosition::new(lx, ly));
  let _ = window.set_size(tauri::LogicalSize::new(lw, lh));
}

fn lens_position_text_floating(app: &AppHandle, window: &WebviewWindow) {
  const WIDTH: f64 = 640.0;
  const HEIGHT: f64 = 320.0;
  const GAP: f64 = 12.0;

  let _ = window.set_size(tauri::LogicalSize::new(WIDTH, HEIGHT));

  let Some(cursor) = get_mouse_position(app) else {
    let _ = window.center();
    return;
  };

  let mut x = cursor.x + GAP;
  let mut y = cursor.y + GAP;

  if let Ok(monitors) = app.available_monitors() {
    if let Some(monitor) = monitors.iter().find(|monitor| {
      let mp = monitor.position();
      let ms = monitor.size();
      cursor.x >= mp.x as f64
        && cursor.x < (mp.x + ms.width as i32) as f64
        && cursor.y >= mp.y as f64
        && cursor.y < (mp.y + ms.height as i32) as f64
    }) {
      let mp = monitor.position();
      let ms = monitor.size();
      let scale = monitor.scale_factor();
      let width = WIDTH * scale;
      let height = HEIGHT * scale;
      let min_x = mp.x as f64 + GAP;
      let min_y = mp.y as f64 + GAP;
      let max_x = (mp.x + ms.width as i32) as f64 - width - GAP;
      let max_y = (mp.y + ms.height as i32) as f64 - height - GAP;
      x = x.max(min_x).min(max_x.max(min_x));
      y = y.max(min_y).min(max_y.max(min_y));
    }
  }

  let _ = window.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// 入口（公共底层）：打开 lens webview 进入 select 态。
/// mode：
///   - "chat"（默认）：截完进对话栏 ready 态
///   - "translate"：截完直接做 OCR + 翻译，弹原文/译文浮动卡
///   - "translateText"：直接翻译当前选中文本，复用截图翻译结果卡
fn lens_request_internal(app: &AppHandle, mode: &str) -> Result<(), String> {
  // 预热 SCK SCShareableContent 缓存，摊销首次截图的 WindowServer 查询开销。
  // 用户从按热键到选目标 + 单击截图通常 ≥ 300 ms，足以盖住 30-80 ms 的 prewarm。
  #[cfg(target_os = "macos")]
  if mode != "translateText" {
    crate::sck::prewarm();
  }

  let state = app.state::<AppState>();
  // 自愈：busy=true 但 lens 窗口已不可见（外部强关 / dev 重载等异常），重置 busy
  if state.lens_busy.load(Ordering::SeqCst) {
    let visible = app
      .get_webview_window("lens")
      .and_then(|w| w.is_visible().ok())
      .unwrap_or(false);
    if !visible {
      state.lens_busy.store(false, Ordering::SeqCst);
    }
  }
  if state.lens_busy.swap(true, Ordering::SeqCst) {
    return Err("Lens already active".to_string());
  }

  // 必须在 ensure_lens_window/show/set_focus 之前抓取。创建隐藏 webview 在 macOS 上也可能
  // 改变当前 focused UI element，导致 Cmd+C/AXSelectedText 读到 Lens 自己而不是前台 App。
  let pending_selection = if mode == "chat" || mode == "translateText" {
    capture_active_selection()
  } else {
    None
  };
  if mode == "translateText" && pending_selection.is_none() {
    if let Ok(mut guard) = state.pending_selection.lock() {
      *guard = None;
    }
    state.lens_busy.store(false, Ordering::SeqCst);
    return Ok(());
  }

  let window = match windows::ensure_lens_window(app) {
    Ok(w) => w,
    Err(e) => {
      state.lens_busy.store(false, Ordering::SeqCst);
      return Err(e);
    }
  };
  // 结果暂存在 state.pending_selection，等前端 take 走。translate 模式写 None，避免遗留旧值。
  if let Ok(mut guard) = state.pending_selection.lock() {
    *guard = pending_selection;
  }
  // 把 mode 编码进 hash query，前端通过 location.hash 读取（'#lens?mode=translate'）
  let safe_mode = match mode {
    "translate" => "translate",
    "translateText" => "translateText",
    _ => "chat",
  };
  let script = format!(
    "window.location.hash = '#lens?mode={mode}'; window.dispatchEvent(new HashChangeEvent('hashchange')); window.dispatchEvent(new CustomEvent('lens:reset'));",
    mode = safe_mode,
  );
  let _ = window.eval(&script);
  if safe_mode == "translateText" {
    lens_position_text_floating(app, &window);
  } else {
    // 先在 hidden 状态下尝试定位：即便部分系统下 hidden 窗口 set_position 被忽略，也比
    // 不调强（成功则消除"先在旧位置闪一帧再跳到全屏"的可见跳变）。
    lens_position_fullscreen(app, &window);
  }
  let _ = window.show();
  let _ = window.set_focus();
  if safe_mode == "translateText" {
    lens_position_text_floating(app, &window);
  } else {
    // show 后再调，处理 always_on_top + visible_on_all_workspaces 把首次 set_position 吃掉的情况
    lens_position_fullscreen(app, &window);
  }
  Ok(())
}

/// 默认入口：lens 模式（commit 后进 ready 悬浮栏）
#[tauri::command]
fn lens_request(app: AppHandle) -> Result<(), String> {
  lens_request_internal(&app, "chat")
}

/// 截图翻译入口：lens webview 进入 select 态，截完做 OCR + 翻译并弹结果浮卡
#[tauri::command]
fn lens_request_translate(app: AppHandle) -> Result<(), String> {
  lens_request_internal(&app, "translate")
}

#[tauri::command]
fn lens_request_translate_text(app: AppHandle) -> Result<(), String> {
  lens_request_internal(&app, "translateText")
}

/// 返回当前屏幕上可见应用窗口列表（macOS 实际数据；Windows 空数组）。
#[tauri::command]
fn lens_list_windows() -> Vec<lens::WindowInfo> {
  lens::list_windows()
}

/// 整窗截图（macOS）：用 `screencapture -l <id>` 按 window id 截，不会截到 lens webview，
/// 所以无需 hide lens（避免 hide/show 那 ~250ms 的视觉闪烁）。
#[tauri::command]
async fn lens_capture_window(
  app: AppHandle,
  window_id: u32,
) -> Result<serde_json::Value, String> {
  let result = lens::capture_window(window_id);
  let _ = app; // 保留参数避免破坏现有调用签名

  match result {
    Ok(path) => {
      let image_id = Uuid::new_v4().to_string();
      let state = app.state::<AppState>();

      // 自动归档（在 insert 前直接用 path，避免二次加锁）
      archive_captured_image(&app, &path, &image_id);

      {
        let mut map = state.images_lock();
        map.insert(image_id.clone(), path);
      }
      {
        let mut current = state.current_id_lock();
        *current = Some(image_id.clone());
      }
      Ok(serde_json::json!({ "success": true, "imageId": image_id }))
    }
    Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
  }
}

/// 区域截图：复用 capture_region_image 路径，注册 image_id 返回。
#[tauri::command]
async fn lens_capture_region(
  app: AppHandle,
  absolute_x: i32,
  absolute_y: i32,
  x: i32,
  y: i32,
  width: u32,
  height: u32,
  scale_factor: f64,
) -> Result<serde_json::Value, String> {
  // SCK 路径：把自己 PID 传给 capture_region_image，SCK 在 GPU compositor 排除 lens webview，
  // 不再需要 hide webview + sleep 60ms 等 NSWindow.orderOut 生效（旧 `screencapture -R` 会截到全屏透明 lens 自己）。
  // Windows 版 capture_region_image 忽略 exclude_self_pid 参数。
  let _ = app.get_webview_window("lens"); // 仍引用以保证 webview 存活
  let exclude_self_pid: Option<i32> = {
    #[cfg(target_os = "macos")]
    {
      Some(std::process::id() as i32)
    }
    #[cfg(not(target_os = "macos"))]
    {
      None
    }
  };

  let result = capture_region_image(
    absolute_x,
    absolute_y,
    x,
    y,
    width,
    height,
    scale_factor,
    exclude_self_pid,
  );
  match result {
    Ok(path) => {
      let image_id = Uuid::new_v4().to_string();
      let state = app.state::<AppState>();

      // 自动归档（在 insert 前直接用 path，避免二次加锁）
      archive_captured_image(&app, &path, &image_id);

      {
        let mut map = state.images_lock();
        map.insert(image_id.clone(), path);
      }
      {
        let mut current = state.current_id_lock();
        *current = Some(image_id.clone());
      }
      Ok(serde_json::json!({ "success": true, "imageId": image_id }))
    }
    Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
  }
}

/// 多轮提问：调用 vision API 流式发出 lens-stream 事件。
/// 字段全部独立。空字符串使用默认值：
///   - default_language：空 → 跟 settings.target_lang（"auto" 视为 "zh"）
///   - system_prompt / question_prompt：空 → default_system_prompt / default_question_prompt 模板
///   - provider_id / model：空 → fallback 到 translator_provider_id / translator_model
///   - stream_enabled：lens 自身配置
#[tauri::command]
async fn lens_ask(
  app: AppHandle,
  state: State<'_, AppState>,
  image_id: String,
  messages: Vec<ExplainMessage>,
) -> Result<serde_json::Value, String> {
  let settings = state.settings_read().clone();
  let retry_attempts = effective_retry_attempts(&settings);

  let language = if !settings.lens.default_language.is_empty() {
    settings.lens.default_language.clone()
  } else if settings.target_lang.starts_with("zh") || settings.target_lang == "en" {
    settings.target_lang.clone()
  } else {
    "zh".to_string()
  };
  let stream_enabled = settings.lens.stream_enabled;
  let thinking_enabled = settings.lens.thinking_enabled;

  let provider_override = if !settings.lens.provider_id.is_empty() {
    Some(settings.lens.provider_id.clone())
  } else {
    None
  };
  let model_override = if !settings.lens.model.is_empty() {
    Some(settings.lens.model.clone())
  } else {
    None
  };

  let has_image = !image_id.is_empty();

  // question_prompt：lens 自定义 → 默认模板（无图时返回空，不附加前缀）
  let question_prompt = if !settings.lens.question_prompt.is_empty() {
    settings.lens.question_prompt.clone()
  } else {
    default_question_prompt(&language, has_image)
  };

  // system_prompt：lens 显式自定义时传 override，否则交给 call_vision_api 走默认模板
  let system_prompt_override = if !settings.lens.system_prompt.is_empty() {
    Some(settings.lens.system_prompt.clone())
  } else {
    None
  };

  if messages.is_empty() {
    return Ok(serde_json::json!({
      "success": false,
      "error": "Missing messages"
    }));
  }

  // 多轮对话：保留前面所有历史，仅把最后一条用户提问注入 question_prompt
  // question_prompt 为空（纯文本对话）时直接传用户原话，不加前缀
  // 关闭思考时在末尾追加 "/no_think"：Qwen3 hybrid 模型识别后直接关思考；其它模型当无意义文本忽略
  let mut api_messages = messages.clone();
  if let Some(last) = api_messages.pop() {
    let mut content = if question_prompt.is_empty() {
      last.content
    } else {
      format!("{}\n\n用户问题：{}", question_prompt, last.content)
    };
    if !thinking_enabled {
      content.push_str(" /no_think");
    }
    api_messages.push(ExplainMessage {
      role: "user".to_string(),
      content,
    });
  }

  match call_vision_api(
    &app,
    &state,
    &image_id,
    api_messages,
    &language,
    retry_attempts,
    stream_enabled,
    "answer",
    "lens-stream",
    provider_override.as_deref(),
    model_override.as_deref(),
    system_prompt_override.as_deref(),
    thinking_enabled,
  )
  .await
  {
    Ok(response) => Ok(serde_json::json!({ "success": true, "response": response })),
    Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
  }
}

/// 取消正在进行的 lens 流（复用同一代号）。
#[tauri::command]
fn lens_cancel_stream(state: State<AppState>) -> Result<(), String> {
  state
    .explain_stream_generation
    .fetch_add(1, Ordering::SeqCst);
  Ok(())
}

/// 截图翻译（lens translate 模式）：单次调用视觉模型，模型先输出译文 + `<<<ORIGINAL>>>` + 原文。
/// stream_enabled=true 时通过 lens-translate-stream emit 流式 delta（kind=translated → kind=original）。
/// `direct_translate=true` 时降级为纯翻译路径（无原文显示），保留旧行为。
#[tauri::command]
async fn lens_translate(
  app: AppHandle,
  state: State<'_, AppState>,
  image_id: String,
) -> Result<serde_json::Value, String> {
  let temp_path = match resolve_explain_image_path(&app, &state, &image_id) {
    Ok(p) => p,
    Err(e) => return Ok(serde_json::json!({ "success": false, "error": e })),
  };

  let settings = state.settings_read().clone();
  let ocr_provider = match settings.get_provider(&settings.screenshot_translation.provider_id) {
    Some(p) => p.clone(),
    None => return Ok(serde_json::json!({ "success": false, "error": "OCR provider not found" })),
  };
  let provider_is_apple = ocr_provider.base_url == apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
  if !provider_is_apple && ocr_provider.api_keys.is_empty() {
    return Ok(serde_json::json!({ "success": false, "error": "Missing API Key" }));
  }

  let retry_attempts = effective_retry_attempts(&settings);
  let direct_translate = settings.screenshot_translation.direct_translate;
  let st_thinking = settings.screenshot_translation.thinking_enabled;
  let st_stream = settings.screenshot_translation.stream_enabled;

  let target_lang = resolve_target_lang(&settings.target_lang, "");
  let lang_name = language_name(&target_lang).to_string();

  // OCR 引擎路由：System / Tesseract 走 local_ocr_then_translate（先识别再翻译两步）
  // CloudVision 落到下方 call_openai_ocr 单次完成 OCR+翻译的多模态路径。
  // Apple provider 在 macOS 强制走 System：Foundation Models 没视觉,只能识别后再让它翻文字。
  let system_ocr_available = cfg!(any(target_os = "macos", target_os = "windows"));
  if provider_is_apple && !cfg!(target_os = "macos") {
    return Ok(serde_json::json!({
      "success": false,
      "error": "Apple Intelligence is not available on this platform"
    }));
  }
  let mut effective_mode = settings
    .screenshot_translation
    .ocr_mode
    .unwrap_or(OcrMode::CloudVision);
  if provider_is_apple && cfg!(target_os = "macos") && effective_mode == OcrMode::CloudVision {
    effective_mode = OcrMode::System;
  }
  // 平台不支持 System / Tesseract 时(理论上 sanitize 已经处理掉,这里防御性兜底)
  if !system_ocr_available && matches!(effective_mode, OcrMode::System | OcrMode::Tesseract) {
    effective_mode = OcrMode::CloudVision;
  }
  if matches!(effective_mode, OcrMode::System | OcrMode::Tesseract) {
    return local_ocr_then_translate(
      &app,
      &state,
      &temp_path,
      &image_id,
      &lang_name,
      direct_translate,
      st_stream,
      st_thinking,
      &ocr_provider,
      &settings.screenshot_translation.model,
      retry_attempts,
      settings.translator_prompt.as_deref(),
      effective_mode,
      &settings.screenshot_translation.tesseract_language,
    )
    .await;
  }

  let prompt = if direct_translate {
    build_ocr_direct_translation_prompt(
      &lang_name,
      settings.screenshot_translation.prompt.as_deref(),
    )
  } else {
    build_combined_translate_prompt(
      &lang_name,
      settings.screenshot_translation.prompt.as_deref(),
    )
  };

  let emit_done_event = |success: bool, error: Option<&str>| {
    let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({
        "imageId": image_id,
        "done": true,
        "success": success,
        "error": error,
      }),
    );
  };

  // direct_translate：纯翻译，无原文。复用 stream_chat_call kind="translated"。
  if direct_translate {
    if st_stream {
      let translated = match stream_chat_call(
        &app,
        &state,
        &ocr_provider,
        &settings.screenshot_translation.model,
        build_ocr_request_body(&temp_path, &prompt, st_thinking)?,
        retry_attempts,
        &image_id,
        "translated",
        "lens-translate-stream",
      )
      .await
      {
        Ok(t) => t,
        Err(e) => {
          emit_done_event(false, Some(&e));
          return Ok(serde_json::json!({ "success": false, "error": e }));
        }
      };
      emit_done_event(true, None);
      return Ok(serde_json::json!({
        "success": true, "original": "", "translated": translated,
      }));
    }
    let translated = match call_openai_ocr(
      &state,
      &ocr_provider,
      &settings.screenshot_translation.model,
      &temp_path,
      &prompt,
      retry_attempts,
      st_thinking,
    )
    .await
    {
      Ok(text) => {
        let _ = app.emit(
          "lens-translate-stream",
          serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": text }),
        );
        text
      }
      Err(e) => {
        emit_done_event(false, Some(&e));
        return Ok(serde_json::json!({ "success": false, "error": e }));
      }
    };
    emit_done_event(true, None);
    return Ok(serde_json::json!({
      "success": true, "original": "", "translated": translated,
    }));
  }

  // 默认：合并模式 — 单次调用拿译文 + 原文
  if st_stream {
    let (translated, original) = match stream_translate_combined(
      &app,
      &state,
      &ocr_provider,
      &settings.screenshot_translation.model,
      build_ocr_request_body(&temp_path, &prompt, st_thinking)?,
      retry_attempts,
      &image_id,
      "lens-translate-stream",
    )
    .await
    {
      Ok(pair) => pair,
      Err(e) => {
        emit_done_event(false, Some(&e));
        return Ok(serde_json::json!({ "success": false, "error": e }));
      }
    };
    emit_done_event(true, None);
    return Ok(serde_json::json!({
      "success": true, "original": original, "translated": translated,
    }));
  }

  // 非流式：调用一次拿到全文，按分隔符拆 translated / original
  let full = match call_openai_ocr(
    &state,
    &ocr_provider,
    &settings.screenshot_translation.model,
    &temp_path,
    &prompt,
    retry_attempts,
    st_thinking,
  )
  .await
  {
    Ok(text) => text,
    Err(e) => {
      emit_done_event(false, Some(&e));
      return Ok(serde_json::json!({ "success": false, "error": e }));
    }
  };
  let (translated, original) = match full.find(COMBINED_TRANSLATE_SEPARATOR) {
    Some(idx) => {
      let t = full[..idx].trim_end_matches('\n').trim().to_string();
      let o = full[idx + COMBINED_TRANSLATE_SEPARATOR.len()..]
        .trim_start_matches('\n')
        .trim()
        .to_string();
      (t, o)
    }
    None => (full.trim().to_string(), String::new()),
  };
  if !translated.is_empty() {
    let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": translated }),
    );
  }
  if !original.is_empty() {
    let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({ "imageId": image_id, "kind": "original", "delta": original }),
    );
  }
  emit_done_event(true, None);
  Ok(serde_json::json!({
    "success": true, "original": original, "translated": translated,
  }))
}

#[tauri::command]
async fn lens_translate_text(
  app: AppHandle,
  state: State<'_, AppState>,
  text: String,
  request_id: String,
) -> Result<serde_json::Value, String> {
  let original = text.trim().to_string();
  let emit_done = |success: bool, error: Option<&str>| {
    let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({
        "imageId": request_id.clone(),
        "done": true,
        "success": success,
        "error": error,
      }),
    );
  };

  if original.is_empty() {
    let msg = "No selected text".to_string();
    emit_done(false, Some(&msg));
    return Ok(serde_json::json!({ "success": false, "error": msg }));
  }

  let settings = state.settings_read().clone();
  let provider = match settings.get_provider(&settings.screenshot_translation.provider_id) {
    Some(p) => p.clone(),
    None => {
      let msg = "Translation provider not found".to_string();
      emit_done(false, Some(&msg));
      return Ok(serde_json::json!({ "success": false, "error": msg }));
    }
  };
  if provider.base_url != apple_intelligence::APPLE_INTELLIGENCE_BASE_URL && provider.api_keys.is_empty() {
    let msg = "Missing API Key".to_string();
    emit_done(false, Some(&msg));
    return Ok(serde_json::json!({ "success": false, "error": msg }));
  }

  let retry_attempts = effective_retry_attempts(&settings);
  let direct_translate = settings.screenshot_translation.direct_translate;
  let st_thinking = settings.screenshot_translation.thinking_enabled;
  let st_stream = settings.screenshot_translation.stream_enabled;
  let target_lang = resolve_target_lang(&settings.target_lang, &original);
  let lang_name = language_name(&target_lang).to_string();
  let prompt = build_screenshot_translation_prompt(
    &original,
    &lang_name,
    settings.screenshot_translation.prompt.as_deref(),
  );

  let is_apple = provider.base_url == apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
  let translated = if st_stream {
    if is_apple {
      let app_for_emit = app.clone();
      let request_id_for_emit = request_id.clone();
      let mut accumulated = String::new();
      if let Err(err) = state
        .apple_intelligence
        .stream_text(&prompt, |delta| {
          accumulated.push_str(delta);
          let _ = app_for_emit.emit(
            "lens-translate-stream",
            serde_json::json!({
              "imageId": request_id_for_emit.clone(),
              "kind": "translated",
              "delta": delta,
            }),
          );
        })
        .await
      {
        emit_done(false, Some(&err));
        return Ok(serde_json::json!({ "success": false, "error": err }));
      }
      accumulated
    } else {
      let mut body = serde_json::json!({
        "messages": [{ "role": "user", "content": prompt }],
        "stream": true,
        "temperature": 0.2,
      });
      if !st_thinking {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
      }
      match stream_chat_call(
        &app,
        &state,
        &provider,
        &settings.screenshot_translation.model,
        body,
        retry_attempts,
        &request_id,
        "translated",
        "lens-translate-stream",
      )
      .await
      {
        Ok(text) => text,
        Err(err) => {
          emit_done(false, Some(&err));
          return Ok(serde_json::json!({ "success": false, "error": err }));
        }
      }
    }
  } else {
    let result = call_openai_text(
      &state,
      &provider,
      &settings.screenshot_translation.model,
      prompt,
      retry_attempts,
      st_thinking,
    )
    .await;
    match result {
      Ok(text) => {
        let _ = app.emit(
          "lens-translate-stream",
          serde_json::json!({ "imageId": request_id.clone(), "kind": "translated", "delta": text }),
        );
        text
      }
      Err(err) => {
        emit_done(false, Some(&err));
        return Ok(serde_json::json!({ "success": false, "error": err }));
      }
    }
  };

  if !direct_translate {
    let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({ "imageId": request_id.clone(), "kind": "original", "delta": original.clone() }),
    );
  }

  emit_done(true, None);
  Ok(serde_json::json!({
    "success": true,
    "original": if direct_translate { String::new() } else { original.clone() },
    "translated": translated,
  }))
}

async fn run_system_ocr(
  state: &State<'_, AppState>,
  image_path: &std::path::Path,
) -> Result<String, String> {
  #[cfg(target_os = "macos")]
  {
    return state
      .macos_ocr
      .ocr_image(&image_path.to_string_lossy())
      .await;
  }

  #[cfg(target_os = "windows")]
  {
    let _ = state;
    return windows_ocr::ocr_image(image_path).await;
  }

  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  {
    let _ = (state, image_path);
    Err("System OCR is not available on this platform".to_string())
  }
}

/// Tesseract 5 离线 OCR：dispatch 到 TesseractClient.ocr_image。
/// `lang` 是语言代码(如 "eng")。binary 不在或语言包未下载时返回错误,
/// 路由层会在调用前先 precheck,这里是双层保险。
async fn run_tesseract_ocr(
  state: &State<'_, AppState>,
  image_path: &std::path::Path,
  lang: &str,
) -> Result<String, String> {
  state.tesseract.ocr_image(image_path, lang).await
}

/// 本地 OCR + 任意 provider 翻译的两步链路。
/// `engine` 决定 OCR 来源:`OcrMode::System`(macOS Apple Vision / Windows.Media.Ocr) 或
/// `OcrMode::Tesseract`(本地 Tesseract 5)。`OcrMode::CloudVision` 走另一条单步路径,
/// 不进这里。
/// 翻译可以是 Apple FoundationModels 或任意 OpenAI 兼容 cloud provider。
/// 与 cloud vision 单次 OCR+translate 调用相比,这里手动 emit lens-translate-stream 事件维持前端契约。
///
/// Tesseract 预检:binary 不在 / 语言包未下载时返回结构化错误,前端 lens 据此渲染下载按钮。
#[allow(clippy::too_many_arguments)]
async fn local_ocr_then_translate(
  app: &AppHandle,
  state: &State<'_, AppState>,
  image_path: &std::path::Path,
  image_id: &str,
  lang_name: &str,
  direct_translate: bool,
  st_stream: bool,
  st_thinking: bool,
  translate_provider: &settings::ModelProvider,
  translate_model: &str,
  retry_attempts: usize,
  translator_template: Option<&str>,
  engine: OcrMode,
  tesseract_language: &str,
) -> Result<serde_json::Value, String> {
  let emit_done = |success: bool, error: Option<&str>| {
    let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({
        "imageId": image_id, "done": true, "success": success, "error": error,
      }),
    );
  };

  // 1) OCR via selected local engine
  // Tesseract 找不到 binary 时 ocr_image 自己会返回 "tesseract_binary_missing",
  // 走下面统一 error 分支 emit 给前端,Lens 据此渲染安装提示——不再做单独 precheck。
  let ocr_result = match engine {
    OcrMode::System => run_system_ocr(state, image_path).await,
    OcrMode::Tesseract => run_tesseract_ocr(state, image_path, tesseract_language).await,
    // 路由层只把 System / Tesseract 派发到这里，CloudVision 走另一条单步路径。
    // 仍留 runtime 兜底，防止后续重构时漏掉某个分支。
    OcrMode::CloudVision => Err("internal: CloudVision in local OCR path".to_string()),
  };
  let original = match ocr_result {
    Ok(text) => text,
    Err(err) => {
      emit_done(false, Some(&err));
      return Ok(serde_json::json!({ "success": false, "error": err }));
    }
  };
  if original.trim().is_empty() {
    let msg = "OCR 未识别到文字".to_string();
    emit_done(false, Some(&msg));
    return Ok(serde_json::json!({ "success": false, "error": msg }));
  }
  if !direct_translate {
    let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({ "imageId": image_id, "kind": "original", "delta": original.clone() }),
    );
  }

  // 2) 翻译 prompt：用主翻译那套 (build_translation_prompt) — screenshot 那个模板是给 vision 模型设计的不适用
  let translate_prompt = build_translation_prompt(&original, lang_name, translator_template);

  // 3) Translate via configured provider —— Apple 走 sidecar,其它走 cloud OpenAI 兼容接口
  let is_apple_translate = translate_provider.base_url == apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
  let translated = if st_stream {
    if is_apple_translate {
      let app_for_emit = app.clone();
      let image_id_for_emit = image_id.to_string();
      let mut accumulated = String::new();
      if let Err(err) = state
        .apple_intelligence
        .stream_text(&translate_prompt, |delta| {
          accumulated.push_str(delta);
          let _ = app_for_emit.emit(
            "lens-translate-stream",
            serde_json::json!({
              "imageId": image_id_for_emit, "kind": "translated", "delta": delta,
            }),
          );
        })
        .await
      {
        emit_done(false, Some(&err));
        return Ok(serde_json::json!({ "success": false, "error": err }));
      }
      accumulated
    } else {
      // Cloud streaming: 用 stream_chat_call + 文字消息（不带 image）
      let mut body = serde_json::json!({
        "messages": [{ "role": "user", "content": translate_prompt }],
        "stream": true,
        "temperature": 0.2,
      });
      if !st_thinking { body["thinking"] = serde_json::json!({ "type": "disabled" }); }
      match stream_chat_call(
        app,
        state,
        translate_provider,
        translate_model,
        body,
        retry_attempts,
        image_id,
        "translated",
        "lens-translate-stream",
      )
      .await
      {
        Ok(t) => t,
        Err(err) => {
          emit_done(false, Some(&err));
          return Ok(serde_json::json!({ "success": false, "error": err }));
        }
      }
    }
  } else {
    let result = if is_apple_translate {
      state.apple_intelligence.call_text(&translate_prompt).await
    } else {
      call_openai_text(
        state,
        translate_provider,
        translate_model,
        translate_prompt,
        retry_attempts,
        st_thinking,
      )
      .await
    };
    match result {
      Ok(t) => {
        let _ = app.emit(
          "lens-translate-stream",
          serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": t }),
        );
        t
      }
      Err(err) => {
        emit_done(false, Some(&err));
        return Ok(serde_json::json!({ "success": false, "error": err }));
      }
    }
  };

  emit_done(true, None);
  Ok(serde_json::json!({
    "success": true,
    "original": if direct_translate { String::new() } else { original.clone() },
    "translated": translated,
  }))
}

/// 关闭 lens：清理图片、释放 busy、隐藏窗口。
///
/// hide 前先把窗口几何复位到当前光标所在显示器的全屏，避免下次 show 出来时还停在
/// 上一次截图后的浮动 bar 位置（先在旧位置闪一帧再跳到 select 全屏的可见跳变）。
#[tauri::command]
fn lens_close(app: AppHandle) -> Result<(), String> {
  let state = app.state::<AppState>();
  state
    .explain_stream_generation
    .fetch_add(1, Ordering::SeqCst);
  let current_id = {
    let current = state.current_id_lock();
    current.clone()
  };
  if let Some(id) = current_id {
    cleanup_explain_image(&app, &id);
  }
  state.lens_busy.store(false, Ordering::SeqCst);
  if let Some(window) = app.get_webview_window("lens") {
    // 先复位再隐藏：visible 状态下 set_position 比 hidden 状态更稳。
    // 即便用户下次按热键时光标已经移到别的 monitor，lens_request_internal
    // 还会再调一次 lens_position_fullscreen 修正，这一步只是消除"上次浮动 bar 位置"的残影。
    lens_position_fullscreen(&app, &window);
    let _ = window.hide();
  }
  Ok(())
}

/// 将 lens 窗口缩小为浮动尺寸（截图后非全屏模式用）
/// x/y 为可选，不传则只改尺寸不改位置
#[derive(serde::Deserialize)]
struct FloatingRect {
  x: Option<f64>,
  y: Option<f64>,
  width: f64,
  height: f64,
}

#[tauri::command]
fn lens_set_floating(app: AppHandle, rect: FloatingRect) -> Result<(), String> {
  if let Some(window) = app.get_webview_window("lens") {
    if let (Some(x), Some(y)) = (rect.x, rect.y) {
      let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    }
    let _ = window.set_size(tauri::LogicalSize::new(rect.width, rect.height));
  }
  Ok(())
}

// ====== /Lens 模式命令 ======

/// 从供应商 API 获取可用模型列表
#[tauri::command]
async fn fetch_models(
  state: State<'_, AppState>,
  provider_id: String,
  provider: Option<ProviderConnectionInput>,
) -> Result<Vec<String>, String> {
    let settings = state.settings_read().clone();
    let (base_url, api_keys) = resolve_provider_credentials(&settings, &provider_id, provider)?;
    let retry_attempts = effective_retry_attempts(&settings);

    if api_keys.is_empty() {
        return Err("Missing API Key".to_string());
    }

    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let response = send_with_failover(
      &state,
      "Models API",
      retry_attempts,
      &provider_id,
      &api_keys,
      |key| state.http.get(url.clone()).bearer_auth(key).send(),
    )
    .await?;

    let value: serde_json::Value = response.json().await.map_err(|e| {
        format!("Failed to parse models response JSON: {e}")
    })?;

    let models = value.get("data")
        .and_then(|data| data.as_array())
        .ok_or_else(|| {
            "Invalid response format: expected 'data' array".to_string()
        })?
        .iter()
        .filter_map(|m| {
            if let Some(s) = m.as_str() {
                Some(s.to_string())
            } else {
                m.get("id").and_then(|id| id.as_str()).map(|s| s.to_string())
            }
        })
        .collect::<Vec<String>>();

    Ok(models)
}

/// 测试供应商连接是否可用
/// 多 key：测试时只用第一个 key（避免一次连接测试遍历多 key 让用户困惑）
#[tauri::command]
async fn test_provider_connection(
  state: State<'_, AppState>,
  provider_id: String,
  provider: Option<ProviderConnectionInput>,
) -> Result<serde_json::Value, String> {
  let settings = state.settings_read().clone();
  let (base_url, api_keys) = resolve_provider_credentials(&settings, &provider_id, provider)?;

  let api_key = match api_keys.first() {
    Some(k) if !k.trim().is_empty() => k.clone(),
    _ => {
      return Ok(serde_json::json!({
        "success": false,
        "error": "Missing API Key"
      }));
    }
  };

  let retry_attempts = effective_retry_attempts(&settings);
  let url = format!("{}/models", base_url.trim_end_matches('/'));
  let result = send_with_retry("Provider API", retry_attempts, || {
    state
      .http
      .get(url.clone())
      .bearer_auth(&api_key)
      .send()
  })
  .await;

  match result {
    Ok(_) => Ok(serde_json::json!({ "success": true })),
    Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
  }
}

/// 获取平台权限状态（仅限 macOS：辅助功能和屏幕录制权限）
#[tauri::command]
fn get_permission_status() -> serde_json::Value {
  #[cfg(target_os = "macos")]
  {
    let accessibility = check_accessibility(false);
    let screen_recording = check_screen_recording_permission();
    return serde_json::json!({
      "platform": "macos",
      "accessibility": accessibility,
      "screenRecording": screen_recording,
    });
  }

  #[cfg(not(target_os = "macos"))]
  {
    serde_json::json!({
      "platform": "other",
      "accessibility": true,
      "screenRecording": true,
    })
  }
}

/// 打开系统权限设置面板（仅限 macOS）
#[tauri::command]
fn open_permission_settings(kind: String) -> Result<(), String> {
  #[cfg(target_os = "macos")]
  {
    use std::process::Command;

    let target = match kind.as_str() {
      "accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
      "screen-recording" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
      _ => return Err("Unsupported permission kind".to_string()),
    };

    Command::new("open")
      .arg(target)
      .output()
      .map_err(|e| e.to_string())?;
    return Ok(());
  }

  #[cfg(not(target_os = "macos"))]
  {
    let _ = kind;
    Err("Permission settings are only available on macOS".to_string())
  }
}

/// 注册全局热键
/// 包括翻译热键、截图翻译热键、lens 热键；会检测重复热键并给出友好错误提示
fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
  let settings = app.state::<AppState>().settings_read().clone();
  let shortcut_manager = app.global_shortcut();
  shortcut_manager.unregister_all().map_err(|e| e.to_string())?;
  let mut errors = Vec::new();
  let mut registered = HashSet::new();

  let format_hotkey_error = |scope: &str, hotkey: &str, error_message: &str| {
    let normalized = error_message.to_lowercase();
    if normalized.contains("already registered")
      || normalized.contains("already in use")
      || normalized.contains("hotkey") && normalized.contains("registered")
    {
      format!(
        "Hotkey conflict for {scope}: \"{hotkey}\" is already in use. Please change this shortcut or close the app that is occupying it."
      )
    } else {
      format!("Failed to register {scope} hotkey \"{hotkey}\": {error_message}")
    }
  };

  if !settings.hotkey.trim().is_empty() {
    let hotkey = settings.hotkey.trim().to_string();
    let hotkey_key = hotkey.to_lowercase();
    if !registered.insert(hotkey_key) {
      errors.push(format!("Duplicate hotkey \"{hotkey}\" for translator"));
    } else if let Err(err) = shortcut_manager.on_shortcut(hotkey.as_str(), move |app, _shortcut, event| {
      if event.state == ShortcutState::Pressed {
        toggle_main_window(app);
      }
    }) {
      errors.push(format_hotkey_error("translator", &hotkey, &err.to_string()));
    }
  }

  if settings.screenshot_translation.enabled {
    let hotkey = settings.screenshot_translation.hotkey.trim().to_string();
    if hotkey.is_empty() {
      errors.push("Screenshot translation hotkey is empty".to_string());
    } else {
      let hotkey_key = hotkey.to_lowercase();
      if !registered.insert(hotkey_key) {
        errors.push(format!("Duplicate hotkey \"{hotkey}\" for screenshot translation"));
      } else if let Err(err) = shortcut_manager.on_shortcut(hotkey.as_str(), move |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
          // 切换行为：Lens 可见时关闭，不可见时打开截图翻译
          if lens_is_active(app) {
            let _ = lens_close(app.clone());
          } else {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
              if let Err(err) = lens_request_translate(handle) {
                eprintln!("Screenshot translation trigger error: {err}");
              }
            });
          }
        }
      }) {
        errors.push(format_hotkey_error(
          "screenshot translation",
          &hotkey,
          &err.to_string(),
        ));
      }
    }

    let text_hotkey = settings.screenshot_translation.text_hotkey.trim().to_string();
    if text_hotkey.is_empty() {
      errors.push("Selected text screenshot translation hotkey is empty".to_string());
    } else {
      let hotkey_key = text_hotkey.to_lowercase();
      if !registered.insert(hotkey_key) {
        errors.push(format!("Duplicate hotkey \"{text_hotkey}\" for selected text screenshot translation"));
      } else if let Err(err) = shortcut_manager.on_shortcut(text_hotkey.as_str(), move |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
          if lens_is_active(app) {
            let _ = lens_close(app.clone());
          } else {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
              if let Err(err) = lens_request_translate_text(handle) {
                eprintln!("Selected text screenshot translation trigger error: {err}");
              }
            });
          }
        }
      }) {
        errors.push(format_hotkey_error(
          "selected text screenshot translation",
          &text_hotkey,
          &err.to_string(),
        ));
      }
    }
  }

  if settings.lens.enabled {
    let hotkey = settings.lens.hotkey.trim().to_string();
    if hotkey.is_empty() {
      errors.push("Lens hotkey is empty".to_string());
    } else {
      let hotkey_key = hotkey.to_lowercase();
      if !registered.insert(hotkey_key) {
        errors.push(format!("Duplicate hotkey \"{hotkey}\" for lens"));
      } else if let Err(err) = shortcut_manager.on_shortcut(hotkey.as_str(), move |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
          // 切换行为：Lens 可见时关闭，不可见时打开
          if lens_is_active(app) {
            let _ = lens_close(app.clone());
          } else {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move {
              if let Err(err) = lens_request(handle) {
                eprintln!("Lens trigger error: {err}");
              }
            });
          }
        }
      }) {
        errors.push(format_hotkey_error("lens", &hotkey, &err.to_string()));
      }
    }
  }

  if errors.is_empty() {
    Ok(())
  } else {
    Err(errors.join("\n"))
  }
}

/// 获取当前鼠标位置
fn get_mouse_position(app: &AppHandle) -> Option<tauri::PhysicalPosition<f64>> {
  app.cursor_position().ok()
}

/// Windows 平台：截取指定区域的屏幕图像
/// 需要将逻辑坐标根据缩放因子转换为物理坐标，再转换为相对于显示器的相对坐标
#[cfg(target_os = "windows")]
fn capture_region_image(
  absolute_x: i32,
  absolute_y: i32,
  x: i32,
  y: i32,
  width: u32,
  height: u32,
  scale_factor: f64,
  _exclude_self_pid: Option<i32>,
) -> Result<PathBuf, String> {
  // 先用前端传入的 scale factor 估算物理坐标，用于定位目标显示器
  let sf = if scale_factor.is_finite() && scale_factor > 0.0 {
    scale_factor
  } else {
    1.0
  };
  let estimated_px = ((absolute_x as f64) * sf).round() as i32;
  let estimated_py = ((absolute_y as f64) * sf).round() as i32;

  // 定位目标显示器：优先用 from_point，失败时遍历所有显示器作为 fallback
  let monitor = Monitor::from_point(estimated_px, estimated_py).or_else(|_| {
    Monitor::all()
      .map_err(|e| e.to_string())?
      .into_iter()
      .find(|m| {
        let Ok(mx) = m.x() else { return false };
        let Ok(my) = m.y() else { return false };
        let Ok(mw) = m.width() else { return false };
        let Ok(mh) = m.height() else { return false };
        let right = mx + mw as i32;
        let bottom = my + mh as i32;
        estimated_px >= mx && estimated_px < right && estimated_py >= my && estimated_py < bottom
      })
      .ok_or_else(|| "No monitor found at the given position".to_string())
  })?;

  let monitor_x = monitor.x().map_err(|e| e.to_string())?;
  let monitor_y = monitor.y().map_err(|e| e.to_string())?;
  let monitor_scale = monitor.scale_factor().map_err(|e| e.to_string())? as f64;

  // 使用显示器实际 scale factor 重新计算物理坐标
  // 这可以修正前端 devicePixelRatio 在多屏幕不同 DPI 下可能不准确的情况
  let absolute_physical_x = ((absolute_x as f64) * monitor_scale).round() as i32;
  let absolute_physical_y = ((absolute_y as f64) * monitor_scale).round() as i32;

  let relative_x = absolute_physical_x - monitor_x;
  let relative_y = absolute_physical_y - monitor_y;
  let region_width = ((width as f64) * monitor_scale).round() as u32;
  let region_height = ((height as f64) * monitor_scale).round() as u32;

  let monitor_width = monitor.width().map_err(|e| e.to_string())?;
  let monitor_height = monitor.height().map_err(|e| e.to_string())?;
  if relative_x < 0
    || relative_y < 0
    || region_width == 0
    || region_height == 0
    || (relative_x as u32) >= monitor_width
    || (relative_y as u32) >= monitor_height
  {
    return Err("Invalid capture region".to_string());
  }

  let max_width = monitor_width.saturating_sub(relative_x as u32);
  let max_height = monitor_height.saturating_sub(relative_y as u32);
  let capture_width = region_width.min(max_width).max(1);
  let capture_height = region_height.min(max_height).max(1);

  let image = monitor
    .capture_region(
      relative_x as u32,
      relative_y as u32,
      capture_width,
      capture_height,
    )
    .map_err(|e| e.to_string())?;

  let temp_path = std::env::temp_dir().join(format!("screenshot-{}.png", Uuid::new_v4()));
  image.save(&temp_path).map_err(|e| e.to_string())?;
  Ok(temp_path)
}

/// macOS 平台：区域截图，走 ScreenCaptureKit。
/// `exclude_self_pid` 传 `Some(pid)` 让 SCK 在 GPU compositor 阶段排除该 PID 的所有窗口
/// （lens webview 自己），无需 hide+sleep 60ms。
#[cfg(target_os = "macos")]
fn capture_region_image(
  absolute_x: i32,
  absolute_y: i32,
  _x: i32,
  _y: i32,
  width: u32,
  height: u32,
  _scale_factor: f64,
  exclude_self_pid: Option<i32>,
) -> Result<PathBuf, String> {
  crate::sck::capture_region(
    absolute_x as f64,
    absolute_y as f64,
    width as f64,
    height as f64,
    exclude_self_pid,
  )
}

/// 其他平台：占位
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn capture_region_image(
  _absolute_x: i32,
  _absolute_y: i32,
  _x: i32,
  _y: i32,
  _width: u32,
  _height: u32,
  _scale_factor: f64,
) -> Result<PathBuf, String> {
  Err("Region capture is not supported on this platform".to_string())
}

/// 切换主窗口显示/隐藏
/// 隐藏时直接隐藏；显示时窗口跟随鼠标位置偏移 (10,10) 弹出，翻译器保持置顶
fn toggle_main_window(app: &AppHandle) {
  let window = match ensure_main_window(app) {
    Ok(window) => window,
    Err(err) => {
      eprintln!("Failed to ensure main window: {}", err);
      return;
    }
  };

  let visible = window.is_visible().unwrap_or(false);
  if visible {
    let _ = window.hide();
    return;
  }

  let _ = window.set_always_on_top(true);

  // 重置 hash 为翻译模式，防止之前打开过设置导致显示设置界面
  let _ = window.eval(
    "window.location.hash = ''; window.dispatchEvent(new HashChangeEvent('hashchange'));",
  );

  let pos = get_mouse_position(app)
    .map(|cursor| tauri::PhysicalPosition::new((cursor.x + 10.0) as i32, (cursor.y + 10.0) as i32));

  #[cfg(target_os = "macos")]
  {
    let window_for_task = window.clone();
    let _ = window.run_on_main_thread(move || {
      if let Some(pos) = pos {
        if let Err(e) = window_for_task.set_position(pos) {
          eprintln!("Failed to set window position: {}", e);
        }
      } else {
        eprintln!("Failed to get mouse position");
      }
      let _ = window_for_task.show();
      let _ = window_for_task.set_focus();
    });
    return;
  }

  #[cfg(not(target_os = "macos"))]
  {
    if let Some(pos) = pos {
      if let Err(e) = window.set_position(pos) {
        eprintln!("Failed to set window position: {}", e);
      }
    } else {
      eprintln!("Failed to get mouse position");
    }
    let _ = window.show();
    let _ = window.set_focus();
  }
}

/// 恢复运行时设置
/// 当保存设置失败时，将设置、热键、托盘等回滚到之前的状态
fn restore_runtime_settings(app: &AppHandle, state: &State<AppState>, previous: &Settings) {
  if let Err(err) = apply_launch_at_startup(app, previous.launch_at_startup) {
    eprintln!("Failed to rollback launch-at-startup setting: {err}");
  }

  {
    let mut guard = state.settings_write();
    *guard = previous.clone();
  }

  if let Err(err) = register_hotkeys(app) {
    eprintln!("Failed to rollback hotkeys: {err}");
  }

  if let Err(err) = setup_tray(app) {
    eprintln!("Failed to rollback tray: {err}");
  }
}

/// 接收前端合成的带箭头标注 PNG（base64 编码），落盘到 temp_dir、注册新 image_id。
/// 不再次归档:归档目录里只保留 capture 时的原图,合成版只活在 temp_dir。
/// 原 image_id 对应的临时文件在切到新 image_id 后立即清理，避免同一会话里堆积 orphan。
#[tauri::command]
fn lens_register_annotated_image(
  state: State<AppState>,
  base64_png: String,
) -> Result<serde_json::Value, String> {
  let bytes = match general_purpose::STANDARD.decode(base64_png.as_bytes()) {
    Ok(b) => b,
    Err(e) => {
      return Ok(serde_json::json!({
        "success": false,
        "error": format!("base64 decode failed: {e}")
      }));
    }
  };

  let temp_path = std::env::temp_dir().join(format!("lens-{}.png", Uuid::new_v4()));
  if let Err(e) = std::fs::write(&temp_path, &bytes) {
    return Ok(serde_json::json!({
      "success": false,
      "error": format!("write png failed: {e}")
    }));
  }

  // 不归档:归档目录只保留 capture 时的原图,合成版只活在 temp_dir + history。
  let image_id = Uuid::new_v4().to_string();
  let previous_image_id = {
    let current = state.current_id_lock();
    current.clone()
  };

  {
    let mut map = state.images_lock();
    map.insert(image_id.clone(), temp_path);
  }
  {
    let mut current = state.current_id_lock();
    *current = Some(image_id.clone());
  }
  if let Some(previous_image_id) = previous_image_id {
    if previous_image_id != image_id {
      let mut map = state.images_lock();
      if let Some(previous_path) = map.remove(&previous_image_id) {
        cleanup_temp_file(&previous_path);
      }
    }
  }

  Ok(serde_json::json!({ "success": true, "imageId": image_id }))
}

/// 清理截图临时文件：从映射中移除并删除磁盘文件
/// 把截图自动归档到用户指定目录（best-effort，失败不阻塞主流程）
fn archive_captured_image(app: &AppHandle, temp_path: &std::path::Path, image_id: &str) {
  let settings = app.state::<AppState>().settings_read().clone();
  if !settings.image_archive_enabled || settings.image_archive_path.is_empty() {
    return;
  }

  let archive_dir = std::path::Path::new(&settings.image_archive_path);
  if !archive_dir.exists() {
    if let Err(e) = std::fs::create_dir_all(archive_dir) {
      eprintln!("[image-archive] failed to create dir {}: {}", archive_dir.display(), e);
      return;
    }
  }
  if !archive_dir.is_dir() {
    eprintln!("[image-archive] archive path is not a directory: {}", archive_dir.display());
    return;
  }

  let now = chrono::Local::now();
  let short_uuid = &image_id[..image_id.len().min(8)];
  let filename = format!("kivio-{}-{}.png", now.format("%Y-%m-%d-%H%M%S"), short_uuid);
  let dest = archive_dir.join(&filename);

  if let Err(e) = std::fs::copy(temp_path, &dest) {
    eprintln!("[image-archive] failed to copy {} -> {}: {}", temp_path.display(), dest.display(), e);
  } else {
    eprintln!("[image-archive] archived to {}", dest.display());
  }
}

fn cleanup_explain_image(app: &AppHandle, image_id: &str) {
  let state = app.state::<AppState>();
  let mut map = state.images_lock();
  if let Some(path) = map.remove(image_id) {
    cleanup_temp_file(&path);
  }
  let mut current = state.current_id_lock();
  if current.as_deref() == Some(image_id) {
    *current = None;
  }
}

/// `{app_data_dir}/lens-history/` —— 历史记录引用的截图持久化目录。
/// 区别于 temp_dir：temp_dir 系统会清，且 lens_close 会立即删；这里只在用户从历史里淘汰条目时才删。
fn lens_history_dir(app: &AppHandle) -> Result<PathBuf, String> {
  let base = app
    .path()
    .app_data_dir()
    .map_err(|e| format!("app_data_dir unavailable: {e}"))?;
  let dir = base.join("lens-history");
  if !dir.exists() {
    fs::create_dir_all(&dir).map_err(|e| format!("create lens-history dir: {e}"))?;
  }
  Ok(dir)
}

/// 根据 image_id 解析图片实际路径。
///
/// 解析顺序：
///   1. 内存 HashMap（当前活跃截图）→ 必须落在 temp_dir，文件存在
///   2. `lens-history/{image_id}.png`（历史记录从 temp 拷贝过来的持久副本）
///
/// 1 失败时退到 2，使得用户重启后从历史里恢复对话仍能继续提问。
pub(crate) fn resolve_explain_image_path(
  app: &AppHandle,
  state: &State<AppState>,
  image_id: &str,
) -> Result<PathBuf, String> {
  // 1. 活跃截图
  {
    let map = state.images_lock();
    if let Some(path) = map.get(image_id).cloned() {
      let temp_dir = std::env::temp_dir();
      if !path.starts_with(&temp_dir) {
        return Err("Invalid image path".to_string());
      }
      if path.exists() {
        return Ok(path);
      }
    }
  }
  // 2. 历史持久副本
  let history_path = lens_history_dir(app)?.join(format!("{image_id}.png"));
  if history_path.exists() {
    return Ok(history_path);
  }
  Err("Image not found".to_string())
}

/// 把当前活跃图片复制到 `lens-history/{image_id}.png`，让它在 temp 文件被
/// lens_close 清理后仍能被历史记录引用。前端在 history-add 完成后调一次。
#[tauri::command]
fn lens_commit_image_to_history(
  app: AppHandle,
  state: State<AppState>,
  image_id: String,
) -> Result<(), String> {
  let src = {
    let map = state.images_lock();
    map.get(&image_id).cloned()
  };
  let Some(src) = src else {
    // 已经被 lens_close 清掉 → 大概率前端在我们之前已经把图存过了，直接当成幂等成功返回
    return Ok(());
  };
  if !src.exists() {
    return Ok(());
  }
  let dst = lens_history_dir(&app)?.join(format!("{image_id}.png"));
  if dst.exists() {
    return Ok(()); // 幂等
  }
  fs::copy(&src, &dst).map_err(|e| format!("commit image to history: {e}"))?;
  Ok(())
}

/// 从历史持久目录删除指定 image_id 对应的 PNG。
/// 前端 history 淘汰一条记录时调用，避免目录无限增长。
#[tauri::command]
fn lens_delete_history_image(app: AppHandle, image_id: String) -> Result<(), String> {
  let dir = lens_history_dir(&app)?;
  let path = dir.join(format!("{image_id}.png"));
  if path.exists() {
    fs::remove_file(&path).map_err(|e| format!("remove history image: {e}"))?;
  }
  Ok(())
}


/// macOS 平台：检查辅助功能权限
/// 如果 open_if_needed 为 true 且未授权，则自动打开系统设置面板
#[cfg(target_os = "macos")]
fn check_accessibility(open_if_needed: bool) -> bool {
    use std::process::Command;
    unsafe {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrustedWithOptions(options: *mut libc::c_void) -> bool;
        }

        // 先进行简单检查（不传入选项）
        if AXIsProcessTrustedWithOptions(std::ptr::null_mut()) {
            return true;
        }

        if open_if_needed {
            // 直接打开系统设置，而不是尝试通过 FFI 触发授权弹窗
            eprintln!("Accessibility not trusted, opening preferences...");
            let _ = Command::new("open")
              .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
              .output();
        }
        false
    }
}

/// macOS 平台：检查屏幕录制权限
#[cfg(target_os = "macos")]
fn check_screen_recording_permission() -> bool {
  unsafe {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
      fn CGPreflightScreenCaptureAccess() -> bool;
    }
    CGPreflightScreenCaptureAccess()
  }
}

/// 发送粘贴快捷键到当前活动应用
/// macOS 通过 AppleScript 发送 Command+V；Windows 通过 enigo 模拟 Ctrl+V
fn send_paste_shortcut() {
  #[cfg(target_os = "macos")]
  {
    if !check_accessibility(true) {
        eprintln!("Accessibility permission missing!");
        return;
    }

    use std::process::Command;
    eprintln!("Sending Paste Shortcut via AppleScript...");
    match Command::new("osascript")
      .arg("-e")
      .arg("tell application \"System Events\" to keystroke \"v\" using command down")
      .output()
    {
      Ok(output) => {
         if !output.status.success() {
             eprintln!("AppleScript failed: {}", String::from_utf8_lossy(&output.stderr));
         } else {
             eprintln!("AppleScript success");
         }
      }
      Err(e) => eprintln!("Failed to execute AppleScript: {}", e),
    }
  }
  #[cfg(target_os = "windows")]
  {
    use enigo::{Enigo, Key, KeyboardControllable};
    let mut enigo = Enigo::new();
    enigo.key_down(Key::Control);
    enigo.key_click(Key::Layout('v'));
    enigo.key_up(Key::Control);
  }
}

/// 打开设置窗口
/// 调整窗口大小为 640x520，取消置顶，显示并聚焦，同时通过 hash 路由切换到设置页面
fn open_settings_window(app: &AppHandle) -> Result<(), String> {
  let window = ensure_main_window(app)?;
  let _ = window.set_always_on_top(false);
  let _ = window.set_size(tauri::LogicalSize::new(640.0, 520.0));

  let window_for_task = window.clone();
  let _ = window.run_on_main_thread(move || {
    let _ = window_for_task.center();
    let _ = window_for_task.show();
    let _ = window_for_task.set_focus();
  });

  let _ = window.eval(
    "window.location.hash = '#settings'; window.dispatchEvent(new HashChangeEvent('hashchange'));",
  );
  // 仅向 main webview 发送 open-settings 事件，避免广播到 screenshot/explain 等其他 webview
  // 导致它们也被切到设置视图（出现多个设置界面的 bug）。
  let _ = app.emit_to("main", "open-settings", ());
  Ok(())
}

fn lens_is_active(app: &AppHandle) -> bool {
  if let Some(state) = app.try_state::<AppState>() {
    if state.lens_busy.load(Ordering::SeqCst) {
      let visible = app
        .get_webview_window("lens")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
      if visible {
        return true;
      }
      state.lens_busy.store(false, Ordering::SeqCst);
    }
  }

  app
    .get_webview_window("lens")
    .and_then(|window| window.is_visible().ok())
    .unwrap_or(false)
}

fn focus_lens_window(app: &AppHandle) -> bool {
  let Some(window) = app.get_webview_window("lens") else {
    return false;
  };
  if !window.is_visible().ok().unwrap_or(false) {
    return false;
  }
  let _ = window.show();
  let _ = window.set_focus();
  true
}

/// 自动激活 app（单实例二次启动 / Windows 普通启动默认设置页）时使用。
/// 如果用户正在拉起 Lens，就不要再抢 main 窗口到设置页。
fn open_settings_window_for_activation(app: &AppHandle) -> Result<(), String> {
  if lens_is_active(app) {
    let _ = focus_lens_window(app);
    return Ok(());
  }
  open_settings_window(app)
}

/// 根据语言返回托盘菜单的标签文本
fn tray_labels(lang: &str) -> (&'static str, &'static str, &'static str) {
  match lang {
    "en" => ("Show Translator", "Settings", "Quit"),
    _ => ("显示翻译器", "设置", "退出"),
  }
}

/// 构建托盘菜单
fn build_tray_menu(
  app: &AppHandle,
  lang: &str,
) -> Result<tauri::menu::Menu<tauri::Wry>, String> {
  use tauri::menu::{Menu, MenuItem};
  let (show_label, settings_label, quit_label) = tray_labels(lang);
  let show = MenuItem::with_id(app, "show", show_label, true, None::<&str>)
    .map_err(|e| e.to_string())?;
  let settings = MenuItem::with_id(app, "settings", settings_label, true, None::<&str>)
    .map_err(|e| e.to_string())?;
  let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)
    .map_err(|e| e.to_string())?;
  Menu::with_items(app, &[&show, &settings, &quit]).map_err(|e| e.to_string())
}

/// 设置系统托盘图标和菜单
/// 如果托盘已存在则只更新菜单；否则创建新的托盘图标并绑定菜单事件
fn setup_tray(app: &AppHandle) -> Result<(), String> {
  use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

  let lang = app
    .state::<AppState>()
    .settings_read()
    .settings_language
    .clone()
    .unwrap_or_else(|| "zh".to_string());

  let menu = build_tray_menu(app, &lang)?;

  if let Some(tray) = app.tray_by_id("main") {
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    return Ok(());
  }

  let icon_bytes = include_bytes!("../icons/tray-icon.png");
  let icon_image = image::load_from_memory(icon_bytes)
    .map_err(|e| e.to_string())?
    .to_rgba8();
  let (width, height) = icon_image.dimensions();
  let tray = TrayIconBuilder::<tauri::Wry>::with_id("main")
    .icon(tauri::image::Image::new_owned(icon_image.into_raw(), width, height))
    // macOS template image：纯黑透明 PNG,系统按 light/dark 主题自动反色为白
    // (Windows/Linux 上 ignore 此 flag,直接显示原图)
    .icon_as_template(true)
    .menu(&menu)
    .show_menu_on_left_click(false)
    .on_menu_event(|app, event| match event.id().as_ref() {
      "show" => {
        match ensure_main_window(app) {
          Ok(window) => {
            let _ = window.set_always_on_top(true);
            let _ = window.show();
            let _ = window.set_focus();
          }
          Err(err) => eprintln!("Failed to ensure main window: {}", err),
        }
      }
      "settings" => {
        if let Err(err) = open_settings_window(app) {
          eprintln!("Failed to open settings window: {}", err);
        }
      }
      "quit" => {
        app.exit(0);
      }
      _ => {}
    })
    .on_tray_icon_event(|tray, event| {
      if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
      } = event
      {
        let app = tray.app_handle().clone();
        // 切换行为：Lens 可见时关闭，不可见时打开
        if lens_is_active(&app) {
          let _ = lens_close(app);
        } else {
          tauri::async_runtime::spawn(async move {
            if let Err(err) = lens_request_internal(&app, "chat") {
              eprintln!("Tray click lens trigger error: {}", err);
            }
          });
        }
      }
    })
    .build(app)
    .map_err(|e| e.to_string())?;

  tray.set_tooltip(Some("Kivio".to_string())).map_err(|e| e.to_string())?;
  Ok(())
}

/// 应用入口函数
/// 初始化 Tauri Builder，加载插件，配置窗口事件处理，设置全局状态、热键和托盘
fn main() {
  let autostart_plugin = {
    #[cfg(target_os = "macos")]
    {
      tauri_plugin_autostart::Builder::new()
        .arg(AUTOSTART_ARG)
        .macos_launcher(MacosLauncher::LaunchAgent)
        .build()
    }
    #[cfg(not(target_os = "macos"))]
    {
      tauri_plugin_autostart::Builder::new()
        .arg(AUTOSTART_ARG)
        .build()
    }
  };

  tauri::Builder::default()
    .plugin(init_single_instance(|app, _args, _cwd| {
      if let Err(err) = open_settings_window_for_activation(app) {
        eprintln!("Single-instance activation failed: {err}");
      }
    }))
    .plugin(tauri_plugin_global_shortcut::Builder::new().build())
    .plugin(tauri_plugin_clipboard_manager::init())
    .plugin(tauri_plugin_store::Builder::default().build())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(autostart_plugin)
    .on_window_event(|window, event| {
      match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
          api.prevent_close();
          let _ = window.hide();
        }
        tauri::WindowEvent::Focused(true) => {
          #[cfg(target_os = "macos")]
          if let Some(webview_window) = window.app_handle().get_webview_window(window.label()) {
            apply_macos_workspace_behavior(&webview_window);
          }
        }
        _ => {}
      }
    })
    .setup(|app| {
      #[cfg(target_os = "macos")]
      {
        // 隐藏 Dock 图标，将应用设置为 accessory 激活策略
        let _ = app
          .handle()
          .set_activation_policy(tauri::ActivationPolicy::Accessory);
      }

      // 清理上次崩溃 / 强杀 / 旧版本遗留的截图 PNG（24h 之前的，避免误删并发实例的活文件）
      cleanup_orphan_temp_files();

      let settings = load_settings(&app.handle());
      if let Err(err) = apply_launch_at_startup(&app.handle(), settings.launch_at_startup) {
        eprintln!("Failed to apply launch-at-startup setting: {err}");
      }

      app.manage(AppState {
        settings: RwLock::new(settings),
        explain_images: Mutex::new(HashMap::new()),
        current_explain_image_id: Mutex::new(None),
        lens_busy: AtomicBool::new(false),
        explain_stream_generation: AtomicU64::new(0),
        pending_selection: Mutex::new(None),
        key_cooldowns: Mutex::new(HashMap::new()),
        active_key_idx: Mutex::new(HashMap::new()),
        http: build_http_client(),
        apple_intelligence: apple_intelligence::AppleIntelligenceClient::new(&app.handle()),
        #[cfg(target_os = "macos")]
        macos_ocr: macos_ocr::MacOcrClient::new(&app.handle()),
        tesseract: tesseract::TesseractClient::new(),
      });

      if let Err(err) = register_hotkeys(&app.handle()) {
        eprintln!("Failed to register hotkeys: {err}");
      }
      if let Err(err) = setup_tray(&app.handle()) {
        eprintln!("Failed to setup tray: {err}");
      }

      // 预创建 lens webview（隐藏），让 WebView2 提前完成首次绘制 + 加载 React
      // 资源。第一次按热键时只走 show()，避免"窗口创建 → WebView2 首帧默认背景
      // 渲染白色 → CSS 把 html/body/#root 设成 transparent"这个时序里的全屏白闪。
      // 仅 Windows 启用：macOS 创建隐藏 webview 可能影响前台应用 focus，进而干扰
      // chat 模式的 Cmd+C/AXSelectedText 选区捕获（lens_request_internal 已有
      // 应对，但预创建会把这层风险提前到 setup 之外的代码路径），且 macOS 上
      // WKWebView 默认不会有 WebView2 这种白闪。
      #[cfg(target_os = "windows")]
      if let Err(err) = windows::ensure_lens_window(&app.handle()) {
        eprintln!("Failed to pre-create lens window: {err}");
      }

      // 启动后 5s 静默检查更新（settings.auto_check_update 控制）
      // 发现新版 → emit "update-available" 事件，前端 Settings 打开时会展示提示
      // 失败 / 限流 / 网络问题全部静默，不打扰用户
      let app_handle = app.handle().clone();
      tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let state: State<AppState> = app_handle.state();
        if !state.settings_read().auto_check_update {
          return;
        }
        if let Ok(value) = check_github_latest_release(state).await {
          if value.get("available").and_then(|v| v.as_bool()).unwrap_or(false) {
            let _ = app_handle.emit("update-available", value);
          }
        }
      });

      #[cfg(target_os = "windows")]
      {
        // Windows 平台：如果不是通过自启动启动的，则默认打开设置窗口
        let launched_from_autostart = std::env::args().any(|arg| arg == AUTOSTART_ARG);
        if !launched_from_autostart {
          let app_handle = app.handle().clone();
          tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Err(err) = open_settings_window_for_activation(&app_handle) {
              eprintln!("Failed to open settings on launch: {err}");
            }
          });
        }
      }
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      get_settings,
      get_default_prompt_templates,
      save_settings,
      translate_text,
      commit_translation,
      open_external,
      explain_read_image,
      fetch_models,
      test_provider_connection,
      get_permission_status,
      open_permission_settings,
      lens_request,
      lens_request_translate,
      lens_request_translate_text,
      lens_list_windows,
      lens_capture_window,
      lens_capture_region,
      lens_register_annotated_image,
      lens_ask,
      lens_translate,
      lens_translate_text,
      lens_cancel_stream,
      lens_close,
      lens_set_floating,
      take_lens_selection,
      lens_commit_image_to_history,
      lens_delete_history_image,
      check_github_latest_release,
      download_update_asset,
      install_update_and_quit,
      apple_intelligence_available,
      tesseract_status,
      tesseract_install
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn is_newer_version_handles_basic_semver() {
    assert!(is_newer_version("2.5.0", "2.4.0"));
    assert!(is_newer_version("2.4.1", "2.4.0"));
    assert!(is_newer_version("3.0.0", "2.99.99"));
    assert!(!is_newer_version("2.4.0", "2.4.0"));
    assert!(!is_newer_version("2.3.9", "2.4.0"));
    assert!(!is_newer_version("1.99.99", "2.0.0"));
  }

  #[test]
  fn is_newer_version_strips_prerelease_suffix() {
    // "1.0.0-beta" 截到第一个非数字 → 1.0.0；与 1.0.0 平等
    assert!(!is_newer_version("1.0.0-beta", "1.0.0"));
    assert!(is_newer_version("1.0.1-beta", "1.0.0"));
  }

  #[test]
  fn is_newer_version_handles_missing_patch() {
    // "2.5" 视为 2.5.0
    assert!(is_newer_version("2.5", "2.4.0"));
    assert!(!is_newer_version("2.5", "2.5.0"));
  }

  #[test]
  fn is_newer_version_handles_garbage_input() {
    // 解析失败的部分都视为 0，不 panic
    assert!(!is_newer_version("", "1.0.0"));
    assert!(is_newer_version("1.0.0", ""));
    assert!(!is_newer_version("garbage", "1.0.0"));
  }
}
