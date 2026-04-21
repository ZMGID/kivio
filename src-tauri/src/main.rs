#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

mod screenshot;
mod settings;
mod utils;
mod windows;

use std::{
  collections::{HashMap, HashSet},
  fs,
  future::Future,
  path::{Path, PathBuf},
  sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, RwLock,
  },
  thread,
  time::Duration,
};

use arboard::Clipboard;
use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use reqwest::{header::HeaderMap, StatusCode};
use serde::Deserialize;
use tauri::{
  AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow,
  WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutoStartManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_single_instance::init as init_single_instance;
use uuid::Uuid;

#[cfg(not(target_os = "windows"))]
use screenshot::capture_screenshot;
use screenshot::cleanup_temp_file;
use settings::{
  default_question_prompt, default_summary_prompt, default_system_prompt, load_settings,
  persist_settings, sanitize_settings, ExplainHistoryRecord, ExplainMessage, Settings,
};
use utils::{current_timestamp, language_name, resolve_target_lang};
#[cfg(target_os = "macos")]
use windows::apply_macos_workspace_behavior;
use windows::{
  ensure_capture_overlay_window, ensure_main_window, ensure_screenshot_window, get_main_window,
};

#[cfg(target_os = "windows")]
use xcap::Monitor;

/// 应用全局状态
/// 使用 RwLock 保护 settings，允许多读单写；
/// Mutex 用于 explain_images 和 pending_capture_mode 等需要独占访问的数据；
/// AtomicBool 用于标记截图翻译/解释是否正在进行，防止并发操作。
struct AppState {
  settings: RwLock<Settings>,
  explain_images: Mutex<HashMap<String, PathBuf>>,
  current_explain_image_id: Mutex<Option<String>>,
  pending_capture_mode: Mutex<Option<String>>,
  screenshot_translation_busy: AtomicBool,
  screenshot_explain_busy: AtomicBool,
  http: Client,
}

/// 自启动参数，用于区分用户手动启动和系统自动启动
const AUTOSTART_ARG: &str = "--from-autostart";

/// 供应商连接输入参数，用于测试连接或获取模型列表时临时传入
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConnectionInput {
  id: Option<String>,
  base_url: String,
  api_key: String,
}

/// 解析供应商的凭据信息
/// 优先使用传入的 ProviderConnectionInput（如测试连接时），否则从 settings 中查找对应的供应商
fn resolve_provider_credentials(
  settings: &Settings,
  provider_id: &str,
  provider: Option<ProviderConnectionInput>,
) -> Result<(String, String), String> {
  if let Some(provider) = provider {
    let id_matches = provider
      .id
      .as_ref()
      .map(|id| id.is_empty() || id == provider_id)
      .unwrap_or(true);

    if id_matches {
      return Ok((provider.base_url, provider.api_key));
    }
  }

  let provider = settings
    .get_provider(provider_id)
    .ok_or_else(|| "Provider not found".to_string())?;
  Ok((provider.base_url.clone(), provider.api_key.clone()))
}

/// 非 Windows 平台：BusyGuard 是一个 RAII 守卫，用于标记截图操作是否正在进行
/// 如果当前已经有截图操作在执行，则返回 None，阻止新的截图操作
#[cfg(not(target_os = "windows"))]
struct BusyGuard<'a> {
  flag: &'a AtomicBool,
}

#[cfg(not(target_os = "windows"))]
impl<'a> BusyGuard<'a> {
  fn new(flag: &'a AtomicBool) -> Option<Self> {
    if flag.swap(true, Ordering::SeqCst) {
      None
    } else {
      Some(Self { flag })
    }
  }
}

#[cfg(not(target_os = "windows"))]
impl Drop for BusyGuard<'_> {
  fn drop(&mut self) {
    self.flag.store(false, Ordering::SeqCst);
  }
}

/// 构建 HTTP 客户端，设置 60 秒超时
fn build_http_client() -> Client {
  Client::builder()
    .timeout(Duration::from_secs(60))
    .build()
    .unwrap_or_else(|err| {
      eprintln!("Failed to build HTTP client: {err}");
      Client::new()
    })
}

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
  state.settings.read().expect("settings lock").clone()
}

/// 获取默认提示词模板
/// 返回翻译模板、截图翻译模板以及解释功能的系统/摘要/提问提示词
#[tauri::command]
fn get_default_prompt_templates() -> serde_json::Value {
  serde_json::json!({
    "translationTemplate": DEFAULT_TRANSLATION_TEMPLATE,
    "screenshotTranslationTemplate": DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE,
    "explainPrompts": {
      "zh": {
        "system": default_system_prompt("zh"),
        "summary": default_summary_prompt("zh"),
        "question": default_question_prompt("zh")
      },
      "en": {
        "system": default_system_prompt("en"),
        "summary": default_summary_prompt("en"),
        "question": default_question_prompt("en")
      }
    }
  })
}

/// 保存设置
/// 先对传入的设置进行清理（sanitize），然后应用开机自启动、重新注册热键、持久化设置、更新托盘菜单
/// 如果热键注册失败，则回滚运行时设置到之前的状态
#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<(), String> {
  let previous_settings = state.settings.read().expect("settings lock").clone();
  let sanitized = sanitize_settings(settings);
  apply_launch_at_startup(&app, sanitized.launch_at_startup)?;
  {
    let mut guard = state.settings.write().expect("settings lock");
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

  let settings = state.settings.read().expect("settings lock").clone();
  let provider = settings.get_provider(&settings.translator_provider_id)
    .ok_or_else(|| "Translator provider not found".to_string())?;

  if provider.api_key.trim().is_empty() {
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
  call_openai_text(
    &state.http,
    provider,
    &settings.translator_model,
    prompt,
    retry_attempts,
  )
  .await
}

/// 提交翻译结果
/// 将翻译后的文本写入剪贴板，隐藏主窗口，如果启用了自动粘贴则发送粘贴快捷键到之前的应用
#[tauri::command]
fn commit_translation(app: AppHandle, state: State<AppState>, text: String) -> Result<(), String> {
  if text.trim().is_empty() {
    return Ok(());
  }

  let auto_paste = state.settings.read().expect("settings lock").auto_paste;
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
    thread::sleep(Duration::from_millis(600));
    send_paste_shortcut();
  }

  Ok(())
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

/// 获取截图解释的初始摘要
/// 调用视觉 API 对当前截图进行总结，支持流式输出
#[tauri::command]
async fn explain_get_initial_summary(
  app: AppHandle,
  state: State<'_, AppState>,
  image_id: String,
) -> Result<serde_json::Value, String> {
  let settings = state.settings.read().expect("settings lock").clone();
  let language = settings.screenshot_explain.default_language.clone();
  let retry_attempts = effective_retry_attempts(&settings);
  let stream_enabled = settings.screenshot_explain.stream_enabled;
  let prompt = settings
    .screenshot_explain
    .custom_prompts
    .as_ref()
    .and_then(|p| p.summary_prompt.clone())
    .unwrap_or_else(|| default_summary_prompt(&language));

  let messages = vec![ExplainMessage {
    role: "user".to_string(),
    content: prompt,
  }];

  match call_vision_api(
    &app,
    &state,
    &image_id,
    messages,
    &language,
    retry_attempts,
    stream_enabled,
    "summary",
  )
  .await
  {
    Ok(summary) => Ok(serde_json::json!({ "success": true, "summary": summary })),
    Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
  }
}

/// 对截图解释进行提问
/// 将用户问题附加在 question_prompt 后发送给视觉模型
#[tauri::command]
async fn explain_ask_question(
  app: AppHandle,
  state: State<'_, AppState>,
  image_id: String,
  messages: Vec<ExplainMessage>,
) -> Result<serde_json::Value, String> {
  let settings = state.settings.read().expect("settings lock").clone();
  let language = settings.screenshot_explain.default_language.clone();
  let retry_attempts = effective_retry_attempts(&settings);
  let stream_enabled = settings.screenshot_explain.stream_enabled;

  if messages.is_empty() {
    return Ok(serde_json::json!({
      "success": false,
      "error": "Missing messages"
    }));
  }

  let question_prompt = settings
    .screenshot_explain
    .custom_prompts
    .as_ref()
    .and_then(|p| p.question_prompt.clone())
    .unwrap_or_else(|| default_question_prompt(&language));

  let mut api_messages = messages.clone();
  if let Some(last) = api_messages.pop() {
    let user_question = last.content;
    api_messages.push(ExplainMessage {
      role: "user".to_string(),
      content: format!("{}\n\n用户问题：{}", question_prompt, user_question),
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
  )
  .await
  {
    Ok(response) => Ok(serde_json::json!({ "success": true, "response": response })),
    Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
  }
}

/// 读取解释图片并以 Base64 数据 URL 格式返回
#[tauri::command]
fn explain_read_image(state: State<AppState>, image_id: String) -> Result<serde_json::Value, String> {
  let image_path = resolve_explain_image_path(&state, &image_id)?;
  let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);
  Ok(serde_json::json!({
    "success": true,
    "data": format!("data:image/png;base64,{base64}")
  }))
}

/// 关闭当前解释会话并清理对应的临时图片
#[tauri::command]
fn explain_close_current(app: AppHandle, state: State<AppState>) -> Result<(), String> {
  let current_id = {
    let current = state.current_explain_image_id.lock().expect("current id lock");
    current.clone()
  };

  if let Some(id) = current_id {
    cleanup_explain_image(&app, &id);
  }

  Ok(())
}

/// 保存解释聊天记录到历史记录
/// 最多保留最近 5 条记录
#[tauri::command]
fn explain_save_history(
  app: AppHandle,
  state: State<AppState>,
  messages: Vec<ExplainMessage>,
) -> Result<serde_json::Value, String> {
  let mut settings = state.settings.write().expect("settings lock");

  let timestamp = current_timestamp();
  let record = ExplainHistoryRecord {
    id: timestamp.to_string(),
    timestamp,
    messages,
  };

  let mut history = settings.explain_history.clone();
  history.insert(0, record);
  history.truncate(5);
  settings.explain_history = history;

  let snapshot = settings.clone();
  drop(settings);

  persist_settings(&app, &snapshot)?;
  Ok(serde_json::json!({ "success": true }))
}

/// 获取所有解释历史记录
#[tauri::command]
fn explain_get_history(state: State<AppState>) -> Result<serde_json::Value, String> {
  let settings = state.settings.read().expect("settings lock");
  Ok(serde_json::json!({
    "success": true,
    "history": settings.explain_history
  }))
}

/// 加载指定的解释历史记录
#[tauri::command]
fn explain_load_history(
  state: State<AppState>,
  history_id: String,
) -> Result<serde_json::Value, String> {
  let settings = state.settings.read().expect("settings lock");
  let record = settings.explain_history.iter().find(|h| h.id == history_id);

  match record {
    Some(record) => Ok(serde_json::json!({ "success": true, "record": record })),
    None => Ok(serde_json::json!({ "success": false, "error": "History not found" })),
  }
}

/// 从供应商 API 获取可用模型列表
#[tauri::command]
async fn fetch_models(
  state: State<'_, AppState>,
  provider_id: String,
  provider: Option<ProviderConnectionInput>,
) -> Result<Vec<String>, String> {
    println!("Fetching models for provider: {}", provider_id);
    let settings = state.settings.read().expect("settings lock").clone();
    let (base_url, api_key) = resolve_provider_credentials(&settings, &provider_id, provider)?;
    let retry_attempts = effective_retry_attempts(&settings);

    if api_key.trim().is_empty() {
        return Err("Missing API Key".to_string());
    }

    let url = format!("{}/models", base_url.trim_end_matches('/'));
    println!("Requesting URL: {}", url);

    let response = send_with_retry("Models API", retry_attempts, || {
        state.http
            .get(url.clone())
            .bearer_auth(&api_key)
            .send()
    })
    .await?;

    let value: serde_json::Value = response.json().await.map_err(|e| {
        println!("Json parsing failed: {}", e);
        e.to_string()
    })?;

    let models = value.get("data")
        .and_then(|data| data.as_array())
        .ok_or_else(|| {
            println!("Invalid response structure: {:?}", value);
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

    println!("Fetched {} models", models.len());
    Ok(models)
}

/// 测试供应商连接是否可用
#[tauri::command]
async fn test_provider_connection(
  state: State<'_, AppState>,
  provider_id: String,
  provider: Option<ProviderConnectionInput>,
) -> Result<serde_json::Value, String> {
  let settings = state.settings.read().expect("settings lock").clone();
  let (base_url, api_key) = resolve_provider_credentials(&settings, &provider_id, provider)?;

  if api_key.trim().is_empty() {
    return Ok(serde_json::json!({
      "success": false,
      "error": "Missing API Key"
    }));
  }

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
/// 包括翻译热键、截图翻译热键、截图解释热键；会检测重复热键并给出友好错误提示
fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
  let settings = app.state::<AppState>().settings.read().expect("settings lock").clone();
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
          let handle = app.clone();
          tauri::async_runtime::spawn(async move {
            if let Err(err) = handle_screenshot_translation(&handle).await {
              eprintln!("Screenshot translation error: {err}");
            }
          });
        }
      }) {
        errors.push(format_hotkey_error(
          "screenshot translation",
          &hotkey,
          &err.to_string(),
        ));
      }
    }
  }

  if settings.screenshot_explain.enabled {
    let hotkey = settings.screenshot_explain.hotkey.trim().to_string();
    if hotkey.is_empty() {
      errors.push("Screenshot explain hotkey is empty".to_string());
    } else {
      let hotkey_key = hotkey.to_lowercase();
      if !registered.insert(hotkey_key) {
        errors.push(format!("Duplicate hotkey \"{hotkey}\" for screenshot explain"));
      } else if let Err(err) = shortcut_manager.on_shortcut(hotkey.as_str(), move |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
          let handle = app.clone();
          tauri::async_runtime::spawn(async move {
            if let Err(err) = handle_screenshot_explain(&handle).await {
              eprintln!("Screenshot explain error: {err}");
            }
          });
        }
      }) {
        errors.push(format_hotkey_error(
          "screenshot explain",
          &hotkey,
          &err.to_string(),
        ));
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
) -> Result<PathBuf, String> {
  let sf = if scale_factor.is_finite() && scale_factor > 0.0 {
    scale_factor
  } else {
    1.0
  };

  let absolute_physical_x = ((absolute_x as f64) * sf).round() as i32;
  let absolute_physical_y = ((absolute_y as f64) * sf).round() as i32;

  let monitor = Monitor::from_point(absolute_physical_x, absolute_physical_y)
    .map_err(|e| e.to_string())?;
  let monitor_x = monitor.x().map_err(|e| e.to_string())?;
  let monitor_y = monitor.y().map_err(|e| e.to_string())?;

  let relative_x = absolute_physical_x - monitor_x;
  let relative_y = absolute_physical_y - monitor_y;
  let region_width = ((width as f64) * sf).round() as u32;
  let region_height = ((height as f64) * sf).round() as u32;

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

  eprintln!(
    "capture region debug: abs_logical=({}, {}), abs_physical=({}, {}), monitor=({}, {}), logical=({}, {}, {}x{}), scale={}, physical=({}, {}, {}x{})",
    absolute_x,
    absolute_y,
    absolute_physical_x,
    absolute_physical_y,
    monitor_x,
    monitor_y,
    x,
    y,
    width,
    height,
    sf,
    relative_x,
    relative_y,
    capture_width,
    capture_height
  );

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

/// 非 Windows 平台：区域截图占位符，直接返回不支持的错误
#[cfg(not(target_os = "windows"))]
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

/// 设置截图忙碌状态
/// 根据模式（translate/explain）分别设置对应的 AtomicBool
fn set_capture_busy(state: &AppState, mode: &str) -> Result<(), String> {
  match mode {
    "translate" => {
      if state.screenshot_translation_busy.swap(true, Ordering::SeqCst) {
        Err("Screenshot already in progress".to_string())
      } else {
        Ok(())
      }
    }
    "explain" => {
      if state.screenshot_explain_busy.swap(true, Ordering::SeqCst) {
        Err("Screenshot explain already in progress".to_string())
      } else {
        Ok(())
      }
    }
    _ => Err("Invalid capture mode".to_string()),
  }
}

/// 清除截图忙碌状态
fn clear_capture_busy(state: &AppState, mode: &str) {
  match mode {
    "translate" => state.screenshot_translation_busy.store(false, Ordering::SeqCst),
    "explain" => state.screenshot_explain_busy.store(false, Ordering::SeqCst),
    _ => {}
  }
}

/// 请求开始区域截图
/// 创建或显示全屏透明覆盖层窗口（capture），等待用户在前端选择区域
#[tauri::command]
fn capture_request(app: AppHandle, mode: String) -> Result<(), String> {
  if mode != "translate" && mode != "explain" {
    return Err("Invalid capture mode".to_string());
  }

  let state = app.state::<AppState>();
  {
    let mut pending = state
      .pending_capture_mode
      .lock()
      .map_err(|_| "Capture state lock failed".to_string())?;
    if pending.is_some() {
      return Err("Capture already pending".to_string());
    }
    set_capture_busy(&state, &mode)?;
    *pending = Some(mode.clone());
  }

  let capture_window = match ensure_capture_overlay_window(&app) {
    Ok(window) => window,
    Err(err) => {
      if let Ok(mut pending) = state.pending_capture_mode.lock() {
        *pending = None;
      }
      clear_capture_busy(&state, &mode);
      return Err(err);
    }
  };
  let _ = capture_window.eval("window.dispatchEvent(new Event('capture:reset'));\n");
  if let Err(err) = capture_window.show().map_err(|e| e.to_string()) {
    if let Ok(mut pending) = state.pending_capture_mode.lock() {
      *pending = None;
    }
    clear_capture_busy(&state, &mode);
    return Err(err);
  }
  if let Err(err) = capture_window.set_focus().map_err(|e| e.to_string()) {
    if let Ok(mut pending) = state.pending_capture_mode.lock() {
      *pending = None;
    }
    clear_capture_busy(&state, &mode);
    return Err(err);
  }
  Ok(())
}

/// 提交区域截图
/// 前端完成区域选择后调用此命令，后端根据坐标截取屏幕区域并进行后续处理
#[tauri::command]
async fn capture_commit(
  app: AppHandle,
  absolute_x: i32,
  absolute_y: i32,
  x: i32,
  y: i32,
  width: u32,
  height: u32,
  scale_factor: f64,
) -> Result<(), String> {
  let state = app.state::<AppState>();
  let mode = {
    let mut pending = state
      .pending_capture_mode
      .lock()
      .map_err(|_| "Capture state lock failed".to_string())?;
    pending.take().ok_or_else(|| "No pending capture mode".to_string())?
  };

  if let Some(window) = app.get_webview_window("capture") {
    let _ = window.hide();
  }

  let result = (|| async {
    let temp_path = capture_region_image(
      absolute_x,
      absolute_y,
      x,
      y,
      width,
      height,
      scale_factor,
    )?;
    if mode == "translate" {
      process_screenshot_translation_from_path(&app, temp_path).await
    } else {
      process_screenshot_explain_from_path(&app, temp_path).await
    }
  })()
  .await;

  clear_capture_busy(&state, &mode);
  result
}

/// 取消区域截图
/// 隐藏覆盖层窗口并清除忙碌状态
#[tauri::command]
fn capture_cancel(app: AppHandle) -> Result<(), String> {
  let state = app.state::<AppState>();
  let mode = {
    let mut pending = state
      .pending_capture_mode
      .lock()
      .map_err(|_| "Capture state lock failed".to_string())?;
    pending.take()
  };

  if let Some(mode) = mode {
    clear_capture_busy(&state, &mode);
  }

  if let Some(window) = app.get_webview_window("capture") {
    let _ = window.hide();
  }
  Ok(())
}

/// 设置窗口置顶状态
#[tauri::command]
fn set_always_on_top(window: WebviewWindow, always_on_top: bool) -> Result<(), String> {
  window.set_always_on_top(always_on_top).map_err(|e| e.to_string())
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

  let pos = get_mouse_position(app)
    .map(|cursor| tauri::PhysicalPosition::new((cursor.x + 10.0) as i32, (cursor.y + 10.0) as i32));

  #[cfg(target_os = "macos")]
  {
    let window_for_task = window.clone();
    let _ = window.run_on_main_thread(move || {
      if let Some(pos) = pos {
        if let Err(e) = window_for_task.set_position(pos) {
          eprintln!("Failed to set window position: {}", e);
        } else {
          eprintln!("Window position set to: {}, {}", pos.x, pos.y);
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
      } else {
        eprintln!("Window position set to: {}, {}", pos.x, pos.y);
      }
    } else {
      eprintln!("Failed to get mouse position");
    }
    let _ = window.show();
    let _ = window.set_focus();
  }
}

/// Windows 平台：处理截图翻译热键
/// 隐藏主窗口并请求开始区域截图（由前端覆盖层完成区域选择）
#[cfg(target_os = "windows")]
async fn handle_screenshot_translation(app: &AppHandle) -> Result<(), String> {
  if let Some(window) = get_main_window(app) {
    let _ = window.hide();
  }

  if let Err(err) = capture_request(app.clone(), "translate".to_string()) {
    if let Some(window) = app.get_webview_window("screenshot") {
      let _ = window.emit("screenshot-error", err.clone());
    }
    return Err(err);
  }

  Ok(())
}

/// 恢复运行时设置
/// 当保存设置失败时，将设置、热键、托盘等回滚到之前的状态
fn restore_runtime_settings(app: &AppHandle, state: &State<AppState>, previous: &Settings) {
  if let Err(err) = apply_launch_at_startup(app, previous.launch_at_startup) {
    eprintln!("Failed to rollback launch-at-startup setting: {err}");
  }

  {
    let mut guard = state.settings.write().expect("settings lock");
    *guard = previous.clone();
  }

  if let Err(err) = register_hotkeys(app) {
    eprintln!("Failed to rollback hotkeys: {err}");
  }

  if let Err(err) = setup_tray(app) {
    eprintln!("Failed to rollback tray: {err}");
  }
}

/// 非 Windows 平台：处理截图翻译热键
/// 使用 BusyGuard 防止并发，直接调用系统截图命令获取图片后处理
#[cfg(not(target_os = "windows"))]
async fn handle_screenshot_translation(app: &AppHandle) -> Result<(), String> {
  let state = app.state::<AppState>();
  let _guard = match BusyGuard::new(&state.screenshot_translation_busy) {
    Some(guard) => guard,
    None => {
      if let Some(window) = app.get_webview_window("screenshot") {
        let _ = window.emit("screenshot-error", "Screenshot already in progress");
      }
      return Ok(());
    }
  };

  if let Some(window) = get_main_window(app) {
    let _ = window.hide();
  }

  let temp_path = capture_screenshot()?;
  process_screenshot_translation_from_path(app, temp_path).await
}

/// 处理截图翻译：从图片路径读取并进行 OCR 识别和翻译
/// 流程：显示处理中 -> OCR ->（可选直接翻译）-> 翻译 -> 发送结果事件
async fn process_screenshot_translation_from_path(
  app: &AppHandle,
  temp_path: PathBuf,
) -> Result<(), String> {
  let state = app.state::<AppState>();
  let screenshot_window = match ensure_screenshot_window(app) {
    Ok(window) => window,
    Err(err) => {
      cleanup_temp_file(&temp_path);
      return Err(err);
    }
  };
  if let Err(err) = screenshot_window.emit("screenshot-processing", ()) {
    cleanup_temp_file(&temp_path);
    return Err(err.to_string());
  }
  let _ = screenshot_window.show();
  let _ = screenshot_window.set_focus();

  let settings = state.settings.read().expect("settings lock").clone();
  let provider = match settings.get_provider(&settings.screenshot_translation.provider_id) {
    Some(provider) => provider,
    None => {
      cleanup_temp_file(&temp_path);
      return Err("OCR provider not found".to_string());
    }
  };

  if provider.api_key.trim().is_empty() {
    if let Err(err) = screenshot_window.emit("screenshot-error", "Missing API Key") {
      cleanup_temp_file(&temp_path);
      return Err(err.to_string());
    }
    cleanup_temp_file(&temp_path);
    return Ok(());
  }

  let retry_attempts = effective_retry_attempts(&settings);
  let direct_translate = settings.screenshot_translation.direct_translate;
  let ocr_prompt = if direct_translate {
    let target_lang = resolve_target_lang(&settings.target_lang, "");
    let lang_name = language_name(&target_lang).to_string();
    build_ocr_direct_translation_prompt(
      &lang_name,
      settings.screenshot_translation.prompt.as_deref(),
    )
  } else {
    DEFAULT_OCR_PROMPT.to_string()
  };

  let recognized = call_openai_ocr(
    &state.http,
    provider,
    &settings.screenshot_translation.model,
    &temp_path,
    &ocr_prompt,
    retry_attempts,
  )
  .await;

  let recognized = match recognized {
    Ok(text) => text,
    Err(err) => {
      let _ = screenshot_window.emit("screenshot-error", &err);
      cleanup_temp_file(&temp_path);
      return Err(err);
    }
  };

  if direct_translate {
    if let Err(err) = app.emit(
      "screenshot-result",
      serde_json::json!({ "original": "", "translated": recognized }),
    ) {
      cleanup_temp_file(&temp_path);
      return Err(err.to_string());
    }
    cleanup_temp_file(&temp_path);
    return Ok(());
  }

  if let Err(err) = app.emit(
    "screenshot-result",
    serde_json::json!({ "original": recognized, "translated": "" }),
  ) {
    cleanup_temp_file(&temp_path);
    return Err(err.to_string());
  }

  if recognized.trim().is_empty() {
    cleanup_temp_file(&temp_path);
    return Ok(());
  }

  let target_lang = resolve_target_lang(&settings.target_lang, &recognized);
  let lang_name = language_name(&target_lang).to_string();
  let prompt = build_screenshot_translation_prompt(
    &recognized,
    &lang_name,
    settings.screenshot_translation.prompt.as_deref(),
  );

  let t_provider = settings
    .get_provider(&settings.translator_provider_id)
    .unwrap_or(provider);

  let translated = call_openai_text(
    &state.http,
    t_provider,
    &settings.translator_model,
    prompt,
    retry_attempts,
  )
  .await
  .unwrap_or(recognized.clone());

  if let Err(err) = app.emit(
    "screenshot-result",
    serde_json::json!({ "original": recognized, "translated": translated }),
  ) {
    cleanup_temp_file(&temp_path);
    return Err(err.to_string());
  }

  cleanup_temp_file(&temp_path);
  Ok(())
}

/// Windows 平台：处理截图解释热键
/// 隐藏主窗口并请求开始区域截图
#[cfg(target_os = "windows")]
async fn handle_screenshot_explain(app: &AppHandle) -> Result<(), String> {
  if let Some(window) = get_main_window(app) {
    let _ = window.hide();
  }

  capture_request(app.clone(), "explain".to_string())
}

/// 非 Windows 平台：处理截图解释热键
/// 使用 BusyGuard 防止并发，直接截图后处理
#[cfg(not(target_os = "windows"))]
async fn handle_screenshot_explain(app: &AppHandle) -> Result<(), String> {
  let state = app.state::<AppState>();
  let _guard = match BusyGuard::new(&state.screenshot_explain_busy) {
    Some(guard) => guard,
    None => {
      eprintln!("Screenshot explain already in progress");
      return Ok(());
    }
  };

  if let Some(window) = get_main_window(app) {
    let _ = window.hide();
  }

  let temp_path = capture_screenshot()?;
  process_screenshot_explain_from_path(app, temp_path).await
}

/// 处理截图解释：从图片路径创建解释窗口并记录图片
async fn process_screenshot_explain_from_path(
  app: &AppHandle,
  temp_path: PathBuf,
) -> Result<(), String> {
  let state = app.state::<AppState>();
  let image_id = Uuid::new_v4().to_string();
  let window = match ensure_explain_window(app, &image_id) {
    Ok(window) => window,
    Err(err) => {
      cleanup_temp_file(&temp_path);
      return Err(err);
    }
  };

  {
    let previous_id = {
      let mut current = state.current_explain_image_id.lock().expect("current id lock");
      let prev = current.clone();
      *current = Some(image_id.clone());
      prev
    };

    if let Some(prev_id) = previous_id {
      if prev_id != image_id {
        cleanup_explain_image(app, &prev_id);
      }
    }

    let mut map = state.explain_images.lock().expect("images lock");
    map.insert(image_id.clone(), temp_path);
  }

  let _ = window.show();
  let _ = window.set_focus();

  Ok(())
}

/// 确保解释窗口存在
/// 如果窗口已存在，则通过修改 hash 跳转并触发 hashchange 事件；否则新建窗口
fn ensure_explain_window(app: &AppHandle, image_id: &str) -> Result<WebviewWindow, String> {
  if let Some(window) = app.get_webview_window("explain") {
    let hash = format!("#explain?imageId={}", image_id);
    let script = format!(
      "window.location.hash = {}; window.dispatchEvent(new HashChangeEvent('hashchange'));",
      serde_json::to_string(&hash).map_err(|e| e.to_string())?
    );
    if let Err(err) = window.eval(script) {
      eprintln!("Failed to update explain window hash: {err}");
    }
    return Ok(window);
  }

  let url = format!("index.html#explain?imageId={}", image_id);
  let window = WebviewWindowBuilder::new(app, "explain", WebviewUrl::App(url.into()))
    .title("Screenshot Explain")
    .inner_size(700.0, 800.0)
    .visible_on_all_workspaces(true)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;

  #[cfg(target_os = "macos")]
  apply_macos_workspace_behavior(&window);

  let app_handle = app.clone();
  let image_id = image_id.to_string();
  window.on_window_event(move |event| {
    if let WindowEvent::Destroyed = event {
      cleanup_explain_image(&app_handle, &image_id);
    }
  });

  Ok(window)
}

/// 清理解释图片：从映射中移除并删除临时文件
fn cleanup_explain_image(app: &AppHandle, image_id: &str) {
  let state = app.state::<AppState>();
  let mut map = state.explain_images.lock().expect("images lock");
  if let Some(path) = map.remove(image_id) {
    cleanup_temp_file(&path);
  }
  let mut current = state
    .current_explain_image_id
    .lock()
    .expect("current id lock");
  if current.as_deref() == Some(image_id) {
    *current = None;
  }
}

/// 根据 image_id 解析解释图片的临时路径，并进行安全性校验（必须在 temp_dir 内且文件存在）
fn resolve_explain_image_path(state: &State<AppState>, image_id: &str) -> Result<PathBuf, String> {
  let map = state.explain_images.lock().expect("images lock");
  let path = map
    .get(image_id)
    .ok_or_else(|| "Image not found".to_string())?
    .clone();

  let temp_dir = std::env::temp_dir();
  if !path.starts_with(&temp_dir) {
    return Err("Invalid image path".to_string());
  }
  if !path.exists() {
    return Err("Image missing".to_string());
  }
  Ok(path)
}

/// 调用 OpenAI 兼容的文本聊天接口
/// 发送单轮 user 消息，temperature 设为 0.2，返回模型生成的文本内容
async fn call_openai_text(
  client: &Client,
  config: &settings::ModelProvider,
  model: &str,
  prompt: String,
  retry_attempts: usize,
) -> Result<String, String> {
  let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
  let body = serde_json::json!({
    "model": model,
    "messages": [{ "role": "user", "content": prompt }],
    "temperature": 0.2
  });

  let response = send_with_retry("OpenAI API", retry_attempts, || {
    client
      .post(url.clone())
      .bearer_auth(&config.api_key)
      .json(&body)
      .send()
  })
  .await?;

  let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| "Invalid response".to_string())?;

  Ok(content.trim().to_string())
}

/// 默认翻译提示词模板
const DEFAULT_TRANSLATION_TEMPLATE: &str =
  "Translate the following text to {lang}. Output only the translation.\n\nRules:\n- Preserve existing LaTeX formulas exactly (keep $...$ and $$...$$).\n- If formula-like plain text appears, normalize it to proper LaTeX when needed.\n- Keep the original line breaks and list structure when possible.\n- Do not add explanations.\n\n{text}";

/// 默认截图翻译提示词模板
const DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE: &str =
  "Translate the OCR text below to {lang}. Output only the translation.\n\nRules:\n- Preserve existing LaTeX formulas exactly (keep $...$ and $$...$$).\n- If formula-like plain text appears, normalize it to proper LaTeX when needed.\n- Keep paragraph and line-break structure from OCR text when possible.\n- Correct only obvious OCR character mistakes; do not invent missing content.\n- Do not add explanations.\n\n{text}";

/// 默认 OCR 提示词：要求模型读取图片中的所有文本，并将数学公式转换为 LaTeX 格式
const DEFAULT_OCR_PROMPT: &str =
  "Read all text in this image. For mathematical formulas, use LaTeX format enclosed in $...$ for inline or $$...$$ for block math. Output only the text content, preserving original lines.";

/// 使用模板构建提示词
/// 支持 {text} 和 {lang} 占位符；如果自定义模板为空或不含 {text}，则追加文本内容
fn build_prompt_with_template(
  text: &str,
  lang_name: &str,
  template: Option<&str>,
  default_template: &str,
) -> String {
  let default_prompt = default_template
    .replace("{lang}", lang_name)
    .replace("{text}", text);

  let Some(template) = template else {
    return default_prompt;
  };
  let trimmed = template.trim();
  if trimmed.is_empty() {
    return default_prompt;
  }

  let mut prompt = trimmed.replace("{text}", text).replace("{lang}", lang_name);
  if !trimmed.contains("{text}") {
    prompt = format!("{prompt}\n\n{text}");
  }
  prompt
}

/// 构建普通翻译提示词
fn build_translation_prompt(text: &str, lang_name: &str, template: Option<&str>) -> String {
  build_prompt_with_template(text, lang_name, template, DEFAULT_TRANSLATION_TEMPLATE)
}

/// 构建截图翻译提示词
fn build_screenshot_translation_prompt(text: &str, lang_name: &str, template: Option<&str>) -> String {
  build_prompt_with_template(text, lang_name, template, DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE)
}

/// 构建 OCR 直接翻译提示词
/// 将截图翻译模板嵌入到 OCR 指令中，让模型一次性完成识别和翻译
fn build_ocr_direct_translation_prompt(lang_name: &str, template: Option<&str>) -> String {
  let instruction = build_screenshot_translation_prompt(
    "the text content recognized from this image",
    lang_name,
    template,
  );
  format!(
    "Read all text in this image, then follow this instruction exactly:\n{}\n\nOutput only the final translated text. For mathematical formulas, keep LaTeX notation ($...$ or $$...$$).",
    instruction
  )
}

/// 调用 OpenAI 兼容的 OCR/视觉接口
/// 将图片转为 Base64 后作为 image_url 类型内容发送，temperature 设为 0 以提高识别稳定性
async fn call_openai_ocr(
  client: &Client,
  config: &settings::ModelProvider,
  model: &str,
  image_path: &Path,
  prompt: &str,
  retry_attempts: usize,
) -> Result<String, String> {
  let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);
  let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

  let body = serde_json::json!({
    "model": model,
    "messages": [
      {
        "role": "user",
        "content": [
          {
            "type": "text",
            "text": prompt
          },
          {
            "type": "image_url",
            "image_url": { "url": format!("data:image/png;base64,{base64}") }
          }
        ]
      }
    ],
    "temperature": 0
  });

  let response = send_with_retry("OpenAI OCR", retry_attempts, || {
    client
      .post(url.clone())
      .bearer_auth(&config.api_key)
      .json(&body)
      .send()
  })
  .await?;

  let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| "Invalid OCR response".to_string())?;

  Ok(content.trim().to_string())
}

/// 调用视觉 API（截图解释）
/// 支持流式输出：如果 stream 为 true，则通过 stream_vision_response 逐段返回内容
async fn call_vision_api(
  app: &AppHandle,
  state: &State<'_, AppState>,
  image_id: &str,
  messages: Vec<ExplainMessage>,
  language: &str,
  retry_attempts: usize,
  stream: bool,
  stream_kind: &str,
) -> Result<String, String> {
  let settings = state.settings.read().expect("settings lock").clone();
  let provider = settings.get_provider(&settings.screenshot_explain.provider_id)
    .ok_or_else(|| "Explain provider not found".to_string())?;

  let image_path = resolve_explain_image_path(state, image_id)?;
  let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);

  let system_prompt = settings
    .screenshot_explain
    .custom_prompts
    .as_ref()
    .and_then(|p| p.system_prompt.clone())
    .unwrap_or_else(|| default_system_prompt(language));

  let mut api_messages = Vec::new();
  api_messages.push(serde_json::json!({
    "role": "system",
    "content": system_prompt
  }));

  if let Some(first) = messages.first() {
    api_messages.push(serde_json::json!({
      "role": "user",
      "content": [
        { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{base64}") } },
        { "type": "text", "text": first.content }
      ]
    }));

    for message in messages.iter().skip(1) {
      api_messages.push(serde_json::json!({
        "role": message.role,
        "content": message.content
      }));
    }
  }

  let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
  let mut body = serde_json::json!({
    "model": settings.screenshot_explain.model,
    "messages": api_messages,
    "temperature": 0.7,
    "max_tokens": 2000
  });
  if stream {
    body["stream"] = serde_json::json!(true);
  }

  let response = send_with_retry("Vision API", retry_attempts, || {
    state
      .http
      .post(url.clone())
      .bearer_auth(&provider.api_key)
      .json(&body)
      .send()
  })
  .await?;

  if stream {
    return stream_vision_response(app, response, image_id, stream_kind).await;
  }

  let value: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| "Invalid vision response".to_string())?;

  Ok(content.trim().to_string())
}

/// 流式解析视觉 API 的 SSE 响应
/// 逐 chunk 读取响应体，解析 "data:" 行，提取 delta 中的 content 并通过事件发送到前端
async fn stream_vision_response(
  app: &AppHandle,
  mut response: reqwest::Response,
  image_id: &str,
  kind: &str,
) -> Result<String, String> {
  let mut buffer = String::new();
  let mut full = String::new();

  while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
    let text = String::from_utf8_lossy(&chunk);
    buffer.push_str(&text);

    while let Some(pos) = buffer.find('\n') {
      let line: String = buffer.drain(..=pos).collect();
      let line = line.trim();
      if !line.starts_with("data:") {
        continue;
      }
      let data = line.trim_start_matches("data:").trim();
      if data.is_empty() {
        continue;
      }
      if data == "[DONE]" {
        return Ok(full.trim().to_string());
      }

      let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(val) => val,
        Err(_) => continue,
      };

      let delta = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"))
        .and_then(|delta| delta.get("content"))
        .and_then(|content| content.as_str());

      if let Some(content) = delta {
        full.push_str(content);
        let _ = app.emit(
          "explain-stream",
          serde_json::json!({ "imageId": image_id, "kind": kind, "delta": content }),
        );
      }
    }
  }

  Ok(full.trim().to_string())
}

/// 重试延迟基础值（毫秒）
const RETRY_BASE_DELAY_MS: u64 = 500;
/// 重试延迟最大值（毫秒）
const RETRY_MAX_DELAY_MS: u64 = 10_000;

/// 获取实际的重试次数
/// 如果重试功能被禁用，则返回 1（即只尝试一次）
fn effective_retry_attempts(settings: &Settings) -> usize {
  if settings.retry_enabled {
    settings.retry_attempts as usize
  } else {
    1
  }
}

/// 从响应头中解析 Retry-After 值（秒），转换为毫秒延迟
fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
  headers
    .get("retry-after")
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse::<u64>().ok())
}

/// 判断 HTTP 状态码是否可重试
/// 包括 429（限流）和所有服务器错误（5xx）
fn is_retryable_status(status: StatusCode) -> bool {
  status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// 判断请求错误是否可重试
/// 包括超时和连接错误
fn is_retryable_error(error: &reqwest::Error) -> bool {
  error.is_timeout() || error.is_connect()
}

/// 计算重试延迟
/// 优先使用服务器返回的 Retry-After 头；否则使用指数退避策略
fn retry_delay_ms(attempt: usize, retry_after: Option<u64>) -> u64 {
  if let Some(seconds) = retry_after {
    return seconds.saturating_mul(1000);
  }

  let delay = RETRY_BASE_DELAY_MS.saturating_mul(2u64.saturating_pow((attempt - 1) as u32));
  delay.min(RETRY_MAX_DELAY_MS)
}

/// 带重试机制的 HTTP 发送函数
/// 对可重试的错误（限流、服务器错误、超时、连接失败）进行指数退避重试
async fn send_with_retry<F, Fut>(label: &str, attempts: usize, mut send: F) -> Result<reqwest::Response, String>
where
  F: FnMut() -> Fut,
  Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
  let attempts = attempts.max(1);
  let mut last_error: Option<String> = None;

  for attempt in 1..=attempts {
    match send().await {
      Ok(response) => {
        let status = response.status();
        if status.is_success() {
          return Ok(response);
        }

        let retry_after = parse_retry_after(response.headers());
        let text = response.text().await.unwrap_or_default();
        let err_msg = format!("{} Error: {} - {}", label, status, text);

        if is_retryable_status(status) && attempt < attempts {
          last_error = Some(err_msg);
          let delay = retry_delay_ms(attempt, retry_after);
          eprintln!("{} retrying in {}ms (attempt {}/{})", label, delay, attempt, attempts);
          tokio::time::sleep(Duration::from_millis(delay)).await;
          continue;
        }

        return Err(format!("{} (attempt {}/{})", err_msg, attempt, attempts));
      }
      Err(err) => {
        let err_msg = format!("{} Error: {}", label, err);
        if is_retryable_error(&err) && attempt < attempts {
          last_error = Some(err_msg);
          let delay = retry_delay_ms(attempt, None);
          eprintln!("{} retrying in {}ms (attempt {}/{})", label, delay, attempt, attempts);
          tokio::time::sleep(Duration::from_millis(delay)).await;
          continue;
        }
        return Err(format!("{} (attempt {}/{})", err_msg, attempt, attempts));
      }
    }
  }

  Err(last_error.map(|msg| {
    format!("{} (attempt {}/{})", msg, attempts, attempts)
  }).unwrap_or_else(|| {
    format!("{} Error: exceeded retry attempts ({})", label, attempts)
  }))
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
/// 调整窗口大小为 420x520，取消置顶，显示并聚焦，同时通过 hash 路由切换到设置页面
fn open_settings_window(app: &AppHandle) -> Result<(), String> {
  let window = ensure_main_window(app)?;
  let _ = window.set_size(tauri::LogicalSize::new(420.0, 520.0));
  let _ = window.set_always_on_top(false);
  let _ = window.show();
  let _ = window.set_focus();
  let _ = window.eval(
    "window.location.hash = '#settings'; window.dispatchEvent(new HashChangeEvent('hashchange'));",
  );
  let _ = window.emit("open-settings", ());
  Ok(())
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
  use tauri::tray::TrayIconBuilder;

  let lang = app
    .state::<AppState>()
    .settings
    .read()
    .expect("settings lock")
    .settings_language
    .clone()
    .unwrap_or_else(|| "zh".to_string());

  let menu = build_tray_menu(app, &lang)?;

  if let Some(tray) = app.tray_by_id("main") {
    tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
    return Ok(());
  }

  let icon_bytes = include_bytes!("../icons/icon.png");
  let icon_image = image::load_from_memory(icon_bytes)
    .map_err(|e| e.to_string())?
    .to_rgba8();
  let (width, height) = icon_image.dimensions();
  let tray = TrayIconBuilder::<tauri::Wry>::with_id("main")
    .icon(tauri::image::Image::new_owned(icon_image.into_raw(), width, height))
    .menu(&menu)
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
    .build(app)
    .map_err(|e| e.to_string())?;

  tray.set_tooltip(Some("KeyLingo".to_string())).map_err(|e| e.to_string())?;
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
      if let Err(err) = open_settings_window(app) {
        eprintln!("Single-instance activation failed: {err}");
      }
    }))
    .plugin(tauri_plugin_global_shortcut::Builder::new().build())
    .plugin(tauri_plugin_clipboard_manager::init())
    .plugin(tauri_plugin_store::Builder::default().build())
    .plugin(tauri_plugin_shell::init())
    .plugin(autostart_plugin)
    .on_window_event(|window, event| {
      match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
          api.prevent_close();
          if window.label() == "capture" {
            let _ = capture_cancel(window.app_handle().clone());
          } else {
            let _ = window.hide();
          }
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

      let settings = load_settings(&app.handle());
      if let Err(err) = apply_launch_at_startup(&app.handle(), settings.launch_at_startup) {
        eprintln!("Failed to apply launch-at-startup setting: {err}");
      }

      app.manage(AppState {
        settings: RwLock::new(settings),
        explain_images: Mutex::new(HashMap::new()),
        current_explain_image_id: Mutex::new(None),
        pending_capture_mode: Mutex::new(None),
        screenshot_translation_busy: AtomicBool::new(false),
        screenshot_explain_busy: AtomicBool::new(false),
        http: build_http_client(),
      });

      if let Err(err) = register_hotkeys(&app.handle()) {
        eprintln!("Failed to register hotkeys: {err}");
      }
      if let Err(err) = setup_tray(&app.handle()) {
        eprintln!("Failed to setup tray: {err}");
      }

      #[cfg(target_os = "windows")]
      {
        // Windows 平台：如果不是通过自启动启动的，则默认打开设置窗口
        let launched_from_autostart = std::env::args().any(|arg| arg == AUTOSTART_ARG);
        if !launched_from_autostart {
          if let Err(err) = open_settings_window(&app.handle()) {
            eprintln!("Failed to open settings on launch: {err}");
          }
        }

        // 预创建截图覆盖层窗口并隐藏，以便后续快速显示
        if let Ok(capture_window) = ensure_capture_overlay_window(&app.handle()) {
          let _ = capture_window.hide();
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
      explain_get_initial_summary,
      explain_ask_question,
      explain_read_image,
      explain_close_current,
      explain_save_history,
      explain_get_history,
      explain_load_history,
      fetch_models,
      test_provider_connection,
      get_permission_status,
      open_permission_settings,
      capture_request,
      capture_commit,
      capture_cancel,
      set_always_on_top
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
