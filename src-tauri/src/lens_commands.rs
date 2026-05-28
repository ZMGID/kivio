use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use base64::{engine::general_purpose, Engine as _};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use uuid::Uuid;

#[cfg(target_os = "windows")]
use xcap::Monitor;

use crate::api::{
    build_ocr_request_body, call_openai_ocr, call_openai_text, call_vision_api,
    effective_retry_attempts, stream_chat_call, stream_translate_combined,
};
use crate::apple_intelligence;
#[cfg(target_os = "windows")]
use crate::capture_geometry::{
    monitor_for_region, windows_monitor_region, CaptureMonitor, CaptureRect,
};
use crate::lens;
use crate::prompts::{
    build_combined_translate_prompt, build_ocr_direct_translation_prompt,
    build_screenshot_translation_prompt, build_translation_prompt, compact_ocr_text,
    COMBINED_TRANSLATE_SEPARATOR,
};
use crate::screenshot::cleanup_temp_file;
use crate::settings::{self, default_question_prompt, ExplainMessage, OcrMode};
use crate::shortcuts::{capture_active_selection, get_mouse_position};
use crate::state::AppState;
use crate::utils::{language_name, resolve_target_lang};
use crate::web_search::{format_web_context, search_web, WebSearchResult};
use crate::windows;
#[cfg(target_os = "windows")]
use crate::windows_ocr;

#[derive(Debug, Clone, Copy)]
struct LensFrame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImageCropRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

pub(crate) fn request_lens_close(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("lens") {
        if window.is_visible().ok().unwrap_or(false) {
            let _ = app.emit_to("lens", "lens-close-request", ());
            return Ok(());
        }
    }
    lens_close(app.clone())
}

#[cfg(target_os = "windows")]
fn insert_temp_explain_image(app: &AppHandle, path: PathBuf) -> String {
    let image_id = Uuid::new_v4().to_string();
    let state = app.state::<AppState>();
    {
        let mut map = state.images_lock();
        map.insert(image_id.clone(), path);
    }
    image_id
}

#[tauri::command]
pub(crate) fn explain_read_image(
    app: AppHandle,
    state: State<AppState>,
    image_id: String,
) -> Result<serde_json::Value, String> {
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
fn lens_position_fullscreen(app: &AppHandle, window: &WebviewWindow) -> Option<LensFrame> {
    #[cfg(target_os = "macos")]
    {
        match lens_position_fullscreen_macos(window) {
            Ok(frame) => {
                lens_clear_interactive_region(window);
                return Some(frame);
            }
            Err(err) => {
                eprintln!("[lens-pos] AppKit fullscreen positioning failed: {err}");
            }
        }
    }

    let cursor_opt = app.cursor_position().ok();
    let monitors = match app.available_monitors() {
        Ok(m) if !m.is_empty() => m,
        Ok(_) => {
            eprintln!("[lens-pos] available_monitors returned empty list");
            return None;
        }
        Err(e) => {
            eprintln!("[lens-pos] available_monitors err: {}", e);
            return None;
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
        return None;
    };

    let mp = monitor.position();
    let ms = monitor.size();
    let scale = monitor.scale_factor();
    let lx = mp.x as f64 / scale;
    let ly = mp.y as f64 / scale;
    let lw = ms.width as f64 / scale;
    let lh = ms.height as f64 / scale;
    let _ = window.set_position(tauri::PhysicalPosition::new(mp.x, mp.y));
    let _ = window.set_size(tauri::PhysicalSize::new(ms.width, ms.height));
    lens_clear_interactive_region(window);
    Some(LensFrame {
        x: lx,
        y: ly,
        width: lw,
        height: lh,
    })
}

#[cfg(target_os = "macos")]
fn lens_position_fullscreen_macos(window: &WebviewWindow) -> Result<LensFrame, String> {
    if macos_is_main_thread() {
        return unsafe { run_lens_position_fullscreen_macos(window) };
    }

    let window_for_task = window.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    window
        .run_on_main_thread(move || {
            let result = unsafe { run_lens_position_fullscreen_macos(&window_for_task) };
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    rx.recv_timeout(std::time::Duration::from_millis(250))
        .map_err(|e| e.to_string())?
}

#[cfg(target_os = "macos")]
fn macos_is_main_thread() -> bool {
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let is_main: bool = msg_send![class!(NSThread), isMainThread];
        is_main
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
unsafe fn run_lens_position_fullscreen_macos(window: &WebviewWindow) -> Result<LensFrame, String> {
    use cocoa::base::{id, nil, NO};
    use cocoa::foundation::{NSPoint, NSRect};
    use objc::{class, msg_send, sel, sel_impl};

    let ns_window_ptr = match window.ns_window() {
        Ok(ptr) if !ptr.is_null() => ptr as id,
        _ => return Err("Lens NSWindow is unavailable".to_string()),
    };

    let screens: id = msg_send![class!(NSScreen), screens];
    if screens == nil {
        return Err("NSScreen.screens returned nil".to_string());
    }
    let count: usize = msg_send![screens, count];
    if count == 0 {
        return Err("No NSScreen available".to_string());
    }

    let mouse: NSPoint = msg_send![class!(NSEvent), mouseLocation];
    let mut target: id = nil;
    for idx in 0..count {
        let screen: id = msg_send![screens, objectAtIndex: idx];
        if screen == nil {
            continue;
        }
        let frame: NSRect = msg_send![screen, frame];
        if mouse.x >= frame.origin.x
            && mouse.x < frame.origin.x + frame.size.width
            && mouse.y >= frame.origin.y
            && mouse.y < frame.origin.y + frame.size.height
        {
            target = screen;
            break;
        }
    }
    if target == nil {
        target = msg_send![class!(NSScreen), mainScreen];
    }
    if target == nil {
        target = msg_send![screens, objectAtIndex: 0usize];
    }
    if target == nil {
        return Err("No target NSScreen available".to_string());
    }

    let target_frame: NSRect = msg_send![target, frame];
    let primary: id = msg_send![screens, objectAtIndex: 0usize];
    if primary == nil {
        return Err("No primary NSScreen available".to_string());
    }
    let primary_frame: NSRect = msg_send![primary, frame];
    let top_left_y = primary_frame.origin.y + primary_frame.size.height
        - (target_frame.origin.y + target_frame.size.height);

    let _: () = msg_send![ns_window_ptr, setFrame: target_frame display: NO];

    Ok(LensFrame {
        x: target_frame.origin.x,
        y: top_left_y,
        width: target_frame.size.width,
        height: target_frame.size.height,
    })
}

#[cfg(target_os = "windows")]
fn lens_clear_interactive_region(window: &WebviewWindow) {
    use ::windows::Win32::Graphics::Gdi::SetWindowRgn;

    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let _ = SetWindowRgn(hwnd, None, false);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn lens_clear_interactive_region(_window: &WebviewWindow) {}

#[cfg(target_os = "windows")]
fn lens_set_interactive_region(
    window: &WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    use ::windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ};

    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let left = (x * scale).round() as i32;
    let top = (y * scale).round() as i32;
    let right = ((x + width) * scale).round() as i32;
    let bottom = ((y + height) * scale).round() as i32;

    unsafe {
        let region = CreateRectRgn(left, top, right.max(left + 1), bottom.max(top + 1));
        if region.is_invalid() {
            return Err("CreateRectRgn failed".to_string());
        }
        if SetWindowRgn(hwnd, Some(region), false) == 0 {
            let _ = DeleteObject(HGDIOBJ(region.0));
            return Err("SetWindowRgn failed".to_string());
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn lens_set_interactive_region(
    window: &WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    if let (Ok(pos), Ok(scale)) = (window.outer_position(), window.scale_factor()) {
        let _ = window.set_position(tauri::LogicalPosition::new(
            (pos.x as f64 / scale) + x,
            (pos.y as f64 / scale) + y,
        ));
    }
    let _ = window.set_size(tauri::LogicalSize::new(width, height));
    Ok(())
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

    let _ = window.set_position(tauri::PhysicalPosition::new(
        x.round() as i32,
        y.round() as i32,
    ));
}

/// 入口（公共底层）：打开 lens webview 进入 select 态。
/// mode：
///   - "chat"（默认）：截完进对话栏 ready 态
///   - "translate"：截完直接做 OCR + 翻译，弹原文/译文浮动卡
///   - "translateText"：直接翻译当前选中文本，复用截图翻译结果卡
pub(crate) fn lens_request_internal(app: &AppHandle, mode: &str) -> Result<(), String> {
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
    cleanup_lens_freeze_frame(app);
    state
        .explain_stream_generation
        .fetch_add(1, Ordering::SeqCst);

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
    let mut freeze_frame_image_id: Option<String> = None;
    if safe_mode == "translateText" {
        lens_position_text_floating(app, &window);
    } else {
        // 先在 hidden 状态下尝试定位：即便部分系统下 hidden 窗口 set_position 被忽略，也比
        // 不调强（成功则消除"先在旧位置闪一帧再跳到全屏"的可见跳变）。
        let frame = lens_position_fullscreen(app, &window);
        freeze_frame_image_id = prepare_windows_freeze_frame(app, frame);
    }
    let _ = window.show();
    let _ = window.set_focus();
    let frame = if safe_mode == "translateText" {
        lens_position_text_floating(app, &window);
        None
    } else {
        // show 后再调，处理 always_on_top + visible_on_all_workspaces 把首次 set_position 吃掉的情况
        lens_position_fullscreen(app, &window)
    };
    let reset_detail = match frame {
        Some(frame) => serde_json::json!({
                "frame": {
                    "x": frame.x,
                    "y": frame.y,
                    "width": frame.width,
                    "height": frame.height,
                },
                "freezeFrameImageId": freeze_frame_image_id,
        }),
        None => freeze_frame_image_id
            .map(|image_id| serde_json::json!({ "freezeFrameImageId": image_id }))
            .unwrap_or_else(|| serde_json::json!({})),
    };
    let reset_detail = serde_json::to_string(&reset_detail).unwrap_or_else(|_| "{}".to_string());
    let script = format!(
        "window.location.hash = '#lens?mode={mode}'; window.dispatchEvent(new HashChangeEvent('hashchange')); window.dispatchEvent(new CustomEvent('lens:reset', {{ detail: {detail} }}));",
        mode = safe_mode,
        detail = reset_detail,
    );
    let _ = window.eval(&script);
    Ok(())
}

/// 默认入口：lens 模式（commit 后进 ready 悬浮栏）
#[tauri::command]
pub(crate) fn lens_request(app: AppHandle) -> Result<(), String> {
    lens_request_internal(&app, "chat")
}

/// 截图翻译入口：lens webview 进入 select 态，截完做 OCR + 翻译并弹结果浮卡
#[tauri::command]
pub(crate) fn lens_request_translate(app: AppHandle) -> Result<(), String> {
    lens_request_internal(&app, "translate")
}

#[tauri::command]
pub(crate) fn lens_request_translate_text(app: AppHandle) -> Result<(), String> {
    lens_request_internal(&app, "translateText")
}

/// 返回当前屏幕上可见应用窗口列表（macOS 实际数据；Windows 空数组）。
#[tauri::command]
pub(crate) fn lens_list_windows() -> Vec<lens::WindowInfo> {
    lens::list_windows()
}

/// 整窗截图（macOS）：用 `screencapture -l <id>` 按 window id 截，不会截到 lens webview，
/// 所以无需 hide lens（避免 hide/show 那 ~250ms 的视觉闪烁）。
#[tauri::command]
pub(crate) async fn lens_capture_window(
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
pub(crate) async fn lens_capture_region(
    app: AppHandle,
    absolute_x: i32,
    absolute_y: i32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
    freeze_frame_image_id: Option<String>,
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

    let result = capture_region_from_freeze_frame(
        &app,
        freeze_frame_image_id.as_deref(),
        x,
        y,
        width,
        height,
        scale_factor,
    )
    .unwrap_or_else(|| {
        capture_region_image(
            absolute_x,
            absolute_y,
            x,
            y,
            width,
            height,
            scale_factor,
            exclude_self_pid,
        )
    });
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
            if let Some(freeze_id) = freeze_frame_image_id.as_deref() {
                cleanup_lens_freeze_frame_if_current(&app, freeze_id);
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
pub(crate) async fn lens_ask(
    app: AppHandle,
    state: State<'_, AppState>,
    image_id: String,
    messages: Vec<ExplainMessage>,
    web_search: Option<bool>,
    provider_id: Option<String>,
    model: Option<String>,
    quick_action: Option<bool>,
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

    let provider_override = provider_id
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !settings.lens.provider_id.is_empty() {
                Some(settings.lens.provider_id.clone())
            } else {
                None
            }
        });
    let model_override = model
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if !settings.lens.model.is_empty() {
                Some(settings.lens.model.clone())
            } else {
                None
            }
        });

    let has_image = !image_id.is_empty();

    let is_quick_action = quick_action.unwrap_or(false);
    let question_prompt = if is_quick_action {
        String::new()
    } else if !settings.lens.question_prompt.is_empty() {
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

    let web_search_requested = web_search.unwrap_or(false);
    let mut web_search_results: Vec<WebSearchResult> = Vec::new();
    let web_context = if web_search_requested && settings.lens.web_search.enabled {
        let user_question = messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.trim())
            .unwrap_or_default();
        let explicit_search = explicitly_requests_web_search(user_question);
        let mut plan = if explicit_search {
            WebSearchToolPlan {
                should_search: true,
                query: cleanup_explicit_search_query(user_question),
                reason: "User explicitly requested web search".to_string(),
            }
        } else {
            emit_lens_web_search(
                &app,
                &image_id,
                "searching",
                "",
                "Planning web search",
                &[],
                None,
            );
            plan_lens_web_search_tool_call(
                &app,
                &state,
                &image_id,
                user_question,
                &language,
                retry_attempts,
                provider_override.as_deref(),
                model_override.as_deref(),
            )
            .await
            .unwrap_or_else(|err| {
                eprintln!("[lens-web-search] tool planning failed: {}", err);
                WebSearchToolPlan {
                    should_search: false,
                    query: String::new(),
                    reason: format!("tool planning failed: {err}"),
                }
            })
        };
        if plan.should_search && plan.query.trim().is_empty() {
            plan.query = user_question.trim().chars().take(180).collect();
        }
        if !plan.should_search {
            eprintln!(
                "[lens-web-search] ai_tool=none reason={:?}",
                plan.reason
            );
            emit_lens_web_search(
                &app,
                &image_id,
                "skipped",
                "",
                &plan.reason,
                &[],
                None,
            );
            String::new()
        } else if plan.query.trim().is_empty() {
            eprintln!("[lens-web-search] ai_tool=web_search but query is empty");
            emit_lens_web_search(
                &app,
                &image_id,
                "skipped",
                "",
                "Search query is empty",
                &[],
                None,
            );
            String::new()
        } else {
            let now = chrono::Local::now();
            let runtime_context = format!(
                "Runtime context:\nCurrent local date/time: {}",
                now.format("%Y-%m-%d %H:%M:%S %:z")
            );
            eprintln!(
                "[lens-web-search] ai_tool=web_search provider={:?} query={:?} reason={:?}",
                settings.lens.web_search.provider,
                plan.query,
                plan.reason
            );
            emit_lens_web_search(
                &app,
                &image_id,
                "searching",
                &plan.query,
                &plan.reason,
                &[],
                None,
            );
            match search_web(&state, &settings.lens.web_search, &plan.query, retry_attempts).await {
                Ok(results) => {
                    let tool_result = if results.is_empty() {
                        "Web search was requested, but the search provider returned no results. Do not claim current web facts from search."
                            .to_string()
                    } else {
                        format_web_context(&results)
                    };
                    emit_lens_web_search(
                        &app,
                        &image_id,
                        "done",
                        &plan.query,
                        &plan.reason,
                        &results,
                        None,
                    );
                    let result_count = results.len();
                    web_search_results = results;
                    let context = format!(
                        "{}\n\nTool call:\nweb_search(query: {:?})\n\nTool result:\n{}\n\nUse this tool result when it is relevant. Cite sources with [1], [2], etc. Do not invent sources.",
                        runtime_context,
                        plan.query,
                        tool_result
                    );
                    eprintln!(
                        "[lens-web-search] results={} context_chars={}",
                        result_count,
                        context.chars().count()
                    );
                    context
                }
                Err(err) => {
                    eprintln!("[lens-web-search] error={}", err);
                    emit_lens_web_search(
                        &app,
                        &image_id,
                        "error",
                        &plan.query,
                        &plan.reason,
                        &[],
                        Some(&err),
                    );
                    return Ok(serde_json::json!({
                      "success": false,
                      "error": err
                    }));
                }
            }
        }
    } else {
        if web_search_requested {
            eprintln!(
                "[lens-web-search] requested=true but disabled in settings"
            );
        }
        String::new()
    };

    // 多轮对话：保留前面所有历史，仅把最后一条用户提问注入 question_prompt
    // question_prompt 为空（纯文本对话）时直接传用户原话，不加前缀
    // 关闭思考时在末尾追加 "/no_think"：Qwen3 hybrid 模型识别后直接关思考；其它模型当无意义文本忽略
    let mut api_messages = messages.clone();
    if let Some(last) = api_messages.pop() {
        let original_question = last.content.clone();
        let mut content = if question_prompt.is_empty() {
            last.content
        } else {
            format!("{}\n\n用户问题：{}", question_prompt, original_question)
        };
        if !thinking_enabled {
            content.push_str(" /no_think");
        }
        api_messages.push(ExplainMessage {
            role: "user".to_string(),
            content,
        });
        if !web_context.is_empty() {
            api_messages.push(ExplainMessage {
                role: "assistant".to_string(),
                content: "I will call the web_search tool before answering.".to_string(),
            });
            api_messages.push(ExplainMessage {
                role: "user".to_string(),
                content: format!(
                    "Original user question:\n{}\n\nTool result from web_search:\n{}\n\nNow answer the original user question using the tool result when relevant. If the tool result is insufficient or irrelevant, say so clearly. Cite sources with [1], [2], etc. when using search results.",
                    original_question,
                    web_context
                ),
            });
        }
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
        Ok(response) => Ok(serde_json::json!({
            "success": true,
            "response": response,
            "webSearchResults": web_search_results,
        })),
        Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
    }
}

fn emit_lens_web_search(
    app: &AppHandle,
    image_id: &str,
    status: &str,
    query: &str,
    reason: &str,
    results: &[WebSearchResult],
    error: Option<&str>,
) {
    let _ = app.emit(
        "lens-web-search",
        serde_json::json!({
            "imageId": image_id,
            "status": status,
            "query": query,
            "reason": reason,
            "results": results,
            "error": error,
        }),
    );
}

fn explicitly_requests_web_search(text: &str) -> bool {
    let lowered = text.to_lowercase();
    lowered.contains("搜索")
        || lowered.contains("搜一下")
        || lowered.contains("查一下")
        || lowered.contains("联网")
        || lowered.contains("上网查")
        || lowered.contains("web search")
        || lowered.contains("search web")
        || lowered.contains("search the web")
        || lowered.contains("look up")
        || lowered.contains("google")
}

fn cleanup_explicit_search_query(text: &str) -> String {
    let mut query = text.trim().to_string();
    for marker in [
        "帮我", "请", "搜索一下", "搜索", "搜一下", "查一下", "联网查一下", "联网查", "上网查一下",
        "上网查", "web search", "search the web", "search web", "look up", "google",
    ] {
        query = query.replace(marker, " ");
    }
    let query = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .trim_matches(['"', '\'', '`', '，', '。', '？', '?', '！', '!'])
        .chars()
        .take(180)
        .collect::<String>();
    if query.trim().is_empty() {
        text.trim().chars().take(180).collect()
    } else {
        query
    }
}

struct WebSearchToolPlan {
    should_search: bool,
    query: String,
    reason: String,
}

async fn plan_lens_web_search_tool_call(
    app: &AppHandle,
    state: &State<'_, AppState>,
    image_id: &str,
    user_question: &str,
    language: &str,
    retry_attempts: usize,
    provider_override: Option<&str>,
    model_override: Option<&str>,
) -> Result<WebSearchToolPlan, String> {
    let user_question = user_question.trim();
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S %:z");
    let prompt = format!(
        "You may call exactly one tool before answering the user.\n\n\
         Current local date/time: {}\n\
         This date/time is for tool planning only. The final answering step will not receive it unless you call web_search, so do not use it to compute an answer inside the planner.\n\n\
         Available tool:\n\
         - web_search(query): search the web for current, external, or identifying information.\n\n\
         Decide whether to call web_search after inspecting the screenshot and the user question.\n\
         If the user explicitly asks to search, look up, google, use the web, 联网, 搜索, 搜一下, or 查一下, you must call web_search.\n\
         Call web_search when the answer depends on current facts, public web knowledge, identifying a visible product/person/place/page/error, prices, docs, news, release info, or anything not fully knowable from the screenshot alone.\n\
         Treat the screenshot itself as a source of possible search triggers: first inspect visible text, logos, names, titles, errors, code, page/UI labels, citations, and objects. If any important visible item is unfamiliar, ambiguous, branded, named, technical, current, or needs outside context to explain accurately, call web_search.\n\
         When the user asks about screenshot content, broad questions like 这是什么, 这个什么意思, 怎么回事, 怎么解决, 帮我看看, explain this, what is this, or why is this happening should call web_search if the screenshot contains unfamiliar or ambiguous visible names, terms, abbreviations, logos, titles, citations, claims, error messages/codes, product names, people, places, websites, companies, model names, package/library names, or UI/page text. Do not answer by guessing from the screenshot if a search result could clarify what it is, where it comes from, whether it is current, or why it matters. If you are unsure whether a visible screenshot item needs external context, prefer calling web_search.\n\
         For screenshot-driven searches, build the query from the most distinctive visible text plus the user's intent, not from generic words like screenshot or image.\n\
         Current-date and relative-time questions count as current facts: 今天/明天/后天/昨天是几号, 今天星期几, 现在几点, today/tomorrow/yesterday, current date/time, day of week, etc. For these, call web_search with a concise query that includes the original question and relevant locale/date context.\n\
         Do not call it for simple OCR, translation, summarization, UI explanation, or questions answerable directly from the screenshot.\n\n\
         Output strict JSON only, no markdown:\n\
         {{\"tool\":\"web_search\",\"query\":\"concise search query including visible names/text\",\"reason\":\"short reason\"}}\n\
         or\n\
         {{\"tool\":\"none\",\"query\":\"\",\"reason\":\"short reason\"}}\n\n\
         User question: {}",
        now,
        user_question
    );
    let system_prompt = if language.starts_with("zh") {
        "你是 Lens 的工具调用规划器。先看截图和用户问题，只输出严格 JSON，决定是否调用 web_search。"
    } else {
        "You are Lens's tool-call planner. Inspect the screenshot and user question. Output strict JSON only."
    };

    let raw = call_vision_api(
        app,
        state,
        image_id,
        vec![ExplainMessage {
            role: "user".to_string(),
            content: prompt,
        }],
        language,
        retry_attempts,
        false,
        "answer",
        "lens-stream",
        provider_override,
        model_override,
        Some(system_prompt),
        false,
    )
    .await?;

    parse_web_search_tool_plan(&raw)
}

fn parse_web_search_tool_plan(raw: &str) -> Result<WebSearchToolPlan, String> {
    let json_text = extract_first_json_object(raw)
        .ok_or_else(|| format!("tool planner returned non-JSON: {}", raw.chars().take(300).collect::<String>()))?;
    let value: serde_json::Value = serde_json::from_str(&json_text)
        .map_err(|err| format!("tool planner JSON parse failed: {err}; body: {}", raw.chars().take(300).collect::<String>()))?;
    let tool = value.get("tool").and_then(|v| v.as_str()).unwrap_or("none");
    let query = value
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .trim_matches(['"', '\'', '`'])
        .chars()
        .take(180)
        .collect::<String>();
    let reason = value
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .chars()
        .take(240)
        .collect::<String>();

    Ok(WebSearchToolPlan {
        should_search: tool == "web_search",
        query,
        reason,
    })
}

fn extract_first_json_object(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in raw[start..].char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                let end = start + offset + ch.len_utf8();
                return Some(raw[start..end].to_string());
            }
        }
    }
    None
}

/// 取消正在进行的 lens 流（复用同一代号）。
#[tauri::command]
pub(crate) fn lens_cancel_stream(state: State<AppState>) -> Result<(), String> {
    state
        .explain_stream_generation
        .fetch_add(1, Ordering::SeqCst);
    Ok(())
}

/// 截图翻译（lens translate 模式）：单次调用视觉模型，模型先输出译文 + `<<<ORIGINAL>>>` + 原文。
/// stream_enabled=true 时通过 lens-translate-stream emit 流式 delta（kind=translated → kind=original）。
/// `direct_translate=true` 时降级为纯翻译路径（无原文显示），保留旧行为。
#[tauri::command]
pub(crate) async fn lens_translate(
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
        None => {
            return Ok(serde_json::json!({ "success": false, "error": "OCR provider not found" }))
        }
    };
    let provider_is_apple =
        ocr_provider.base_url == apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
    if !provider_is_apple && ocr_provider.api_keys.is_empty() {
        return Ok(serde_json::json!({ "success": false, "error": "Missing API Key" }));
    }
    if !provider_is_apple && settings.screenshot_translation.model.trim().is_empty() {
        return Ok(serde_json::json!({
          "success": false,
          "error": "Please select a model first"
        }));
    }

    let retry_attempts = effective_retry_attempts(&settings);
    let direct_translate = settings.screenshot_translation.direct_translate;
    let st_thinking = settings.screenshot_translation.thinking_enabled;
    let st_stream = settings.screenshot_translation.stream_enabled;

    let target_lang = resolve_target_lang(&settings.target_lang, "");
    let lang_name = language_name(&target_lang).to_string();

    // OCR 引擎路由：System / RapidOcr 走 local_ocr_then_translate（先识别再翻译两步）
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
    // 平台不支持 System / RapidOcr 时(理论上 sanitize 已经处理掉,这里防御性兜底)
    if !system_ocr_available && matches!(effective_mode, OcrMode::System | OcrMode::RapidOcr) {
        effective_mode = OcrMode::CloudVision;
    }
    if matches!(effective_mode, OcrMode::System | OcrMode::RapidOcr) {
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
pub(crate) async fn lens_translate_text(
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
    if provider.base_url != apple_intelligence::APPLE_INTELLIGENCE_BASE_URL
        && provider.api_keys.is_empty()
    {
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

/// RapidOCR 离线 OCR：dispatch 到 RapidOcrClient.ocr_image。
/// 模型 / dylib 文件未下载时返回 "rapidocr_models_missing",
/// 路由层会在调用前先 precheck,这里是双层保险。
async fn run_rapidocr_ocr(
    state: &State<'_, AppState>,
    image_path: &std::path::Path,
) -> Result<String, String> {
    state.rapidocr.ocr_image(image_path).await
}

/// 本地 OCR + 任意 provider 翻译的两步链路。
/// `engine` 决定 OCR 来源:`OcrMode::System`(macOS Apple Vision / Windows.Media.Ocr) 或
/// `OcrMode::RapidOcr`(本地 RapidOCR PaddleOCR ONNX)。`OcrMode::CloudVision` 走另一条单步路径,
/// 不进这里。
/// 翻译可以是 Apple FoundationModels 或任意 OpenAI 兼容 cloud provider。
/// 与 cloud vision 单次 OCR+translate 调用相比,这里手动 emit lens-translate-stream 事件维持前端契约。
///
/// RapidOCR 预检:dylib / 模型文件未下载时返回结构化错误,前端 lens 据此渲染下载按钮。
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
    // RapidOCR 找不到模型文件时 ocr_image 自己会返回 "rapidocr_models_missing",
    // 走下面统一 error 分支 emit 给前端,Lens 据此渲染下载提示——不再做单独 precheck。
    let ocr_result = match engine {
        OcrMode::System => run_system_ocr(state, image_path).await,
        OcrMode::RapidOcr => run_rapidocr_ocr(state, image_path).await,
        // 路由层只把 System / RapidOcr 派发到这里,CloudVision 走另一条单步路径。
        // Legacy 兜底变体在 sanitize_settings 中会被正常化为 CloudVision,理论不会到这里。
        // 仍留 runtime 兜底,防止后续重构时漏掉某个分支。
        OcrMode::CloudVision | OcrMode::Legacy => {
            Err("internal: non-local OCR mode reached local_ocr_then_translate".to_string())
        }
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
    // 折叠 OCR 引擎产生的多余空行,避免被翻译模型一字不漏 echo 进译文占空间。
    let original = compact_ocr_text(&original);
    if !direct_translate {
        let _ = app.emit(
      "lens-translate-stream",
      serde_json::json!({ "imageId": image_id, "kind": "original", "delta": original.clone() }),
    );
    }

    // 2) 翻译 prompt：用主翻译模板。新版默认模板已经加了"输入像 OCR 输出时用上下文修错 + 压缩空行"的规则,
    // 跟纯文本翻译共用一份模板;用户在 Settings 里改 translator_prompt 同样会作用到这条路径。
    let translate_prompt = build_translation_prompt(&original, lang_name, translator_template);

    // 3) Translate via configured provider —— Apple 走 sidecar,其它走 cloud OpenAI 兼容接口
    let is_apple_translate =
        translate_provider.base_url == apple_intelligence::APPLE_INTELLIGENCE_BASE_URL;
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
            if !st_thinking {
                body["thinking"] = serde_json::json!({ "type": "disabled" });
            }
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
pub(crate) fn lens_close(app: AppHandle) -> Result<(), String> {
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
    cleanup_lens_freeze_frame(&app);
    state.lens_busy.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window("lens") {
        // 先隐藏再复位：避免 visible 状态下从浮动尺寸 resize 到全屏时用户看到闪屏
        // （尤其是 Windows 上 translateText 浮动弹窗点击外部关闭时）。
        // hidden 状态下 set_position 在部分系统下可能被忽略，但 lens_request_internal
        // 打开时会在 show 前后各调一次 lens_position_fullscreen 修正，足够兜底。
        let _ = window.hide();
        lens_position_fullscreen(&app, &window);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn prepare_windows_freeze_frame(app: &AppHandle, frame: Option<LensFrame>) -> Option<String> {
    let settings = app.state::<AppState>().settings_read().clone();
    if !settings.lens.windows_freeze_frame_selection {
        return None;
    }
    let frame = frame?;
    let width = frame.width.round().max(1.0) as u32;
    let height = frame.height.round().max(1.0) as u32;
    let path = capture_region_image(
        frame.x.round() as i32,
        frame.y.round() as i32,
        0,
        0,
        width,
        height,
        1.0,
        None,
    )
    .map_err(|err| {
        eprintln!("[lens-freeze] capture failed: {err}");
        err
    })
    .ok()?;
    let image_id = insert_temp_explain_image(app, path);
    let state = app.state::<AppState>();
    {
        let mut freeze = state
            .lens_freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *freeze = Some(image_id.clone());
    }
    Some(image_id)
}

#[cfg(not(target_os = "windows"))]
fn prepare_windows_freeze_frame(_app: &AppHandle, _frame: Option<LensFrame>) -> Option<String> {
    None
}

fn capture_region_from_freeze_frame(
    app: &AppHandle,
    freeze_frame_image_id: Option<&str>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
) -> Option<Result<PathBuf, String>> {
    let image_id = freeze_frame_image_id?;
    let state = app.state::<AppState>();
    let is_current_freeze = {
        let freeze = state
            .lens_freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        freeze.as_deref() == Some(image_id)
    };
    if !is_current_freeze {
        return None;
    }

    let path = match resolve_explain_image_path(app, &state, image_id) {
        Ok(path) => path,
        Err(err) => return Some(Err(err)),
    };
    Some(crop_freeze_frame_image(
        &path,
        x,
        y,
        width,
        height,
        scale_factor,
    ))
}

fn crop_freeze_frame_image(
    path: &Path,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
) -> Result<PathBuf, String> {
    let image = image::open(path).map_err(|e| e.to_string())?;
    let rect = freeze_frame_crop_rect(
        x,
        y,
        width,
        height,
        scale_factor,
        image.width(),
        image.height(),
    )
    .ok_or_else(|| "Invalid freeze-frame capture region".to_string())?;
    let cropped = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
    let temp_path = std::env::temp_dir().join(format!("screenshot-{}.png", Uuid::new_v4()));
    cropped.save(&temp_path).map_err(|e| e.to_string())?;
    Ok(temp_path)
}

fn freeze_frame_crop_rect(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
    image_width: u32,
    image_height: u32,
) -> Option<ImageCropRect> {
    if width == 0 || height == 0 || image_width == 0 || image_height == 0 {
        return None;
    }
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let x = (x as f64 * scale).round() as i32;
    let y = (y as f64 * scale).round() as i32;
    let width = (width as f64 * scale).round().max(1.0) as u32;
    let height = (height as f64 * scale).round().max(1.0) as u32;

    let left = x.clamp(0, image_width as i32);
    let top = y.clamp(0, image_height as i32);
    let right = (x as i64 + width as i64).clamp(left as i64, image_width as i64) as i32;
    let bottom = (y as i64 + height as i64).clamp(top as i64, image_height as i64) as i32;

    if right <= left || bottom <= top {
        return None;
    }

    Some(ImageCropRect {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

fn cleanup_lens_freeze_frame(app: &AppHandle) {
    let state = app.state::<AppState>();
    let image_id = {
        let mut freeze = state
            .lens_freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        freeze.take()
    };
    if let Some(image_id) = image_id {
        cleanup_explain_image(app, &image_id);
    }
}

fn cleanup_lens_freeze_frame_if_current(app: &AppHandle, image_id: &str) {
    let state = app.state::<AppState>();
    let should_cleanup = {
        let mut freeze = state
            .lens_freeze_frame_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if freeze.as_deref() == Some(image_id) {
            *freeze = None;
            true
        } else {
            false
        }
    };
    if should_cleanup {
        cleanup_explain_image(app, image_id);
    }
}

/// 将 lens 窗口缩小为浮动尺寸（截图后非全屏模式用）
/// x/y 为可选，不传则只改尺寸不改位置
#[derive(serde::Deserialize)]
pub(crate) struct FloatingRect {
    x: Option<f64>,
    y: Option<f64>,
    width: f64,
    height: f64,
}

#[tauri::command]
pub(crate) fn lens_set_floating(app: AppHandle, rect: FloatingRect) -> Result<(), String> {
    let Some(window) = app.get_webview_window("lens") else {
        return Ok(());
    };

    #[cfg(target_os = "windows")]
    {
        if let (Some(x), Some(y)) = (rect.x, rect.y) {
            lens_set_interactive_region(&window, x, y, rect.width, rect.height)?;
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let (Some(x), Some(y)) = (rect.x, rect.y) {
            let _ = window.set_position(tauri::LogicalPosition::new(x, y));
        }
        let _ = window.set_size(tauri::LogicalSize::new(rect.width, rect.height));
    }

    Ok(())
}

/// macOS:用 AppKit 原生 `[window.animator setFrame:display:]` 一次 IPC 触发动画。
/// 之前的 JS rAF 循环每帧打 IPC + 两次独立 AppKit 调用,coalescing 后实际帧率掉到 ~50fps。
/// 这里改成单次调度,Core Animation 在合成器线程按显示器原生刷新率插值。
///
/// duration_ms 与前端 TRANSITION_MS 对齐;timing function 用 cubic-bezier(0.22, 1, 0.36, 1)
/// 与原 CSS transition 完全一致。
#[cfg(target_os = "macos")]
#[tauri::command]
pub(crate) fn lens_animate_floating(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    duration_ms: f64,
) -> Result<(), String> {
    let Some(window) = app.get_webview_window("lens") else {
        return Ok(());
    };
    // AppKit 调用必须落在主线程;run_on_main_thread 立即返回,动画后续由 Core Animation 驱动。
    app.run_on_main_thread(move || unsafe {
        run_lens_animate_macos(&window, x, y, width, height, duration_ms);
    })
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
unsafe fn run_lens_animate_macos(
    window: &WebviewWindow,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    duration_ms: f64,
) {
    use cocoa::base::{id, nil, NO};
    use cocoa::foundation::{NSPoint, NSRect, NSSize};
    use objc::runtime::{Class, Sel};
    use objc::{class, msg_send, sel, sel_impl};

    let ns_window_ptr = match window.ns_window() {
        Ok(ptr) if !ptr.is_null() => ptr as id,
        _ => return,
    };

    // top-left logical → NSScreen 全局底原点。
    // ns_y = primary_h - top_left_y - height 跨多屏通用:NSScreen 全局原点在主屏底左,
    // 其它屏只是该坐标系里的偏移,不影响这里的换算。
    let screens: id = msg_send![class!(NSScreen), screens];
    if screens == nil {
        return;
    }
    let count: usize = msg_send![screens, count];
    if count == 0 {
        return;
    }
    let primary: id = msg_send![screens, objectAtIndex: 0usize];
    let primary_frame: NSRect = msg_send![primary, frame];
    let primary_h = primary_frame.size.height;

    let ns_y = primary_h - y - height;
    let target_rect = NSRect::new(NSPoint::new(x, ns_y), NSSize::new(width, height));

    // CAMediaTimingFunction 的 functionWithControlPoints:::: 是「关键字+3个匿名冒号」的
    // 多 colon 选择器,objc 0.2 的 sel!() 不支持这种形式,这里用 Sel::register +
    // 直接 objc_msgSend FFI 调用。返回的 timing 是 autoreleased,setTimingFunction: 会 retain。
    extern "C" {
        fn objc_msgSend();
    }
    type FnSig = unsafe extern "C" fn(*const Class, Sel, f32, f32, f32, f32) -> id;
    let send: FnSig = std::mem::transmute(objc_msgSend as *const ());
    let timing_cls = class!(CAMediaTimingFunction);
    let timing_sel = Sel::register("functionWithControlPoints::::");
    let timing: id = send(timing_cls, timing_sel, 0.22, 1.0, 0.36, 1.0);
    if timing == nil {
        return;
    }

    let nsac = class!(NSAnimationContext);
    let _: () = msg_send![nsac, beginGrouping];
    let ctx: id = msg_send![nsac, currentContext];
    if ctx != nil {
        let _: () = msg_send![ctx, setDuration: duration_ms / 1000.0];
        let _: () = msg_send![ctx, setTimingFunction: timing];
    }
    let animator: id = msg_send![ns_window_ptr, animator];
    // display:NO → AppKit 不每帧强同步 displayIfNeeded,重绘交给合成器,
    // 减少 WKWebView 在 resize 过程中的 reflow + paint 压力。
    let _: () = msg_send![animator, setFrame: target_rect display: NO];
    let _: () = msg_send![nsac, endGrouping];
}

/// 非 macOS:fallback 到立即 snap 到目标矩形;前端用 setTimeout 模拟动画完成时序。
#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub(crate) fn lens_animate_floating(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    duration_ms: f64,
) -> Result<(), String> {
    let _ = duration_ms;
    let Some(window) = app.get_webview_window("lens") else {
        return Ok(());
    };
    let _ = window.set_position(tauri::LogicalPosition::new(x, y));
    let _ = window.set_size(tauri::LogicalSize::new(width, height));
    Ok(())
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
    let _ = (x, y, scale_factor);
    let monitors = Monitor::all().map_err(|e| e.to_string())?;
    let monitor_geometry = monitors
        .iter()
        .map(|m| {
            Ok(CaptureMonitor {
                x: m.x().map_err(|e| e.to_string())?,
                y: m.y().map_err(|e| e.to_string())?,
                width: m.width().map_err(|e| e.to_string())?,
                height: m.height().map_err(|e| e.to_string())?,
                scale_factor: m.scale_factor().map_err(|e| e.to_string())? as f64,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let region = CaptureRect {
        x: absolute_x as f64,
        y: absolute_y as f64,
        width: width as f64,
        height: height as f64,
    };
    let monitor_index = monitor_for_region(region, &monitor_geometry)
        .ok_or_else(|| "No monitor found for capture region".to_string())?;
    let capture_region = windows_monitor_region(region, monitor_geometry[monitor_index])
        .ok_or_else(|| "Invalid capture region".to_string())?;
    let monitor = &monitors[monitor_index];

    let image = monitor
        .capture_region(
            capture_region.x,
            capture_region.y,
            capture_region.width,
            capture_region.height,
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

#[tauri::command]
pub(crate) fn lens_register_annotated_image(
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
            eprintln!(
                "[image-archive] failed to create dir {}: {}",
                archive_dir.display(),
                e
            );
            return;
        }
    }
    if !archive_dir.is_dir() {
        eprintln!(
            "[image-archive] archive path is not a directory: {}",
            archive_dir.display()
        );
        return;
    }

    let now = chrono::Local::now();
    let short_uuid = &image_id[..image_id.len().min(8)];
    let filename = format!("kivio-{}-{}.png", now.format("%Y-%m-%d-%H%M%S"), short_uuid);
    let dest = archive_dir.join(&filename);

    if let Err(e) = std::fs::copy(temp_path, &dest) {
        eprintln!(
            "[image-archive] failed to copy {} -> {}: {}",
            temp_path.display(),
            dest.display(),
            e
        );
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
pub(crate) fn lens_commit_image_to_history(
    app: AppHandle,
    state: State<AppState>,
    image_id: String,
) -> Result<(), String> {
    let dst = lens_history_dir(&app)?.join(format!("{image_id}.png"));
    if dst.exists() {
        return Ok(()); // 幂等
    }
    let map = state.images_lock();
    let Some(src) = map.get(&image_id) else {
        return Err("Image is no longer available for history".to_string());
    };
    if !src.exists() {
        return Err("Image file is no longer available for history".to_string());
    }
    fs::copy(&src, &dst).map_err(|e| format!("commit image to history: {e}"))?;
    Ok(())
}

/// 从历史持久目录删除指定 image_id 对应的 PNG。
/// 前端 history 淘汰一条记录时调用，避免目录无限增长。
#[tauri::command]
pub(crate) fn lens_delete_history_image(app: AppHandle, image_id: String) -> Result<(), String> {
    let dir = lens_history_dir(&app)?;
    let path = dir.join(format!("{image_id}.png"));
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("remove history image: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freeze_frame_crop_rect_clamps_to_image_bounds() {
        assert_eq!(
            freeze_frame_crop_rect(-10, 8, 30, 20, 1.0, 100, 80),
            Some(ImageCropRect {
                x: 0,
                y: 8,
                width: 20,
                height: 20,
            })
        );

        assert_eq!(
            freeze_frame_crop_rect(90, 70, 30, 20, 1.0, 100, 80),
            Some(ImageCropRect {
                x: 90,
                y: 70,
                width: 10,
                height: 10,
            })
        );
    }

    #[test]
    fn freeze_frame_crop_rect_rejects_empty_or_outside_region() {
        assert_eq!(freeze_frame_crop_rect(10, 10, 0, 20, 1.0, 100, 80), None);
        assert_eq!(freeze_frame_crop_rect(120, 10, 20, 20, 1.0, 100, 80), None);
        assert_eq!(freeze_frame_crop_rect(10, 90, 20, 20, 1.0, 100, 80), None);
    }

    #[test]
    fn freeze_frame_crop_rect_scales_logical_region_to_physical_pixels() {
        assert_eq!(
            freeze_frame_crop_rect(10, 12, 40, 20, 1.5, 300, 200),
            Some(ImageCropRect {
                x: 15,
                y: 18,
                width: 60,
                height: 30,
            })
        );
    }
}
