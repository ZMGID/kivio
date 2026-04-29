#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

mod lens;
#[cfg(target_os = "macos")]
mod sck;
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
    atomic::{AtomicBool, AtomicU64, Ordering},
    Mutex, RwLock,
  },
  time::{Duration, Instant},
};

use arboard::Clipboard;
use base64::{engine::general_purpose, Engine as _};
use reqwest::Client;
use reqwest::{header::HeaderMap, StatusCode};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutoStartManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_single_instance::init as init_single_instance;
use uuid::Uuid;

use screenshot::{cleanup_orphan_temp_files, cleanup_temp_file};
use settings::{
  default_question_prompt, default_system_prompt, load_settings, no_think_instruction, persist_settings,
  sanitize_settings, ExplainMessage, Settings,
};
use utils::{language_name, resolve_target_lang};
#[cfg(target_os = "macos")]
use windows::apply_macos_workspace_behavior;
use windows::{ensure_main_window, get_main_window};

#[cfg(target_os = "windows")]
use xcap::Monitor;

/// 应用全局状态
/// 使用 RwLock 保护 settings，允许多读单写；
/// Mutex 用于 explain_images 等需要独占访问的数据；
/// AtomicBool 标记 lens 是否正在进行，防止并发热键触发。
struct AppState {
  settings: RwLock<Settings>,
  explain_images: Mutex<HashMap<String, PathBuf>>,
  current_explain_image_id: Mutex<Option<String>>,
  lens_busy: AtomicBool,
  /// 流式取消代号：每开新的流就 +1，跑流的循环检测到代号变了就立即结束。
  explain_stream_generation: AtomicU64,
  /// API Key 多 key failover 状态：(provider_id, key_idx) → 冷却到期时间。
  /// 某个 key 触发 quota/rate-limit/auth 失败时进入冷却，KEY_COOLDOWN 秒内不再选用。
  key_cooldowns: Mutex<HashMap<(String, usize), Instant>>,
  /// 每个 provider 当前活跃 key idx：上一次成功的 key 优先继续用。
  active_key_idx: Mutex<HashMap<String, usize>>,
  http: Client,
}

/// 单个 key 触发 failover 后的冷却时长。
const KEY_COOLDOWN: Duration = Duration::from_secs(60);

impl AppState {
  /// 安全读取设置（锁中毒时返回内部数据，不 panic）
  fn settings_read(&self) -> std::sync::RwLockReadGuard<'_, Settings> {
    self.settings.read().unwrap_or_else(|e| e.into_inner())
  }
  /// 安全写入设置（锁中毒时返回内部数据，不 panic）
  fn settings_write(&self) -> std::sync::RwLockWriteGuard<'_, Settings> {
    self.settings.write().unwrap_or_else(|e| e.into_inner())
  }
  /// 安全获取解释图片映射锁
  fn images_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PathBuf>> {
    self.explain_images.lock().unwrap_or_else(|e| e.into_inner())
  }
  /// 安全获取当前解释图片 ID 锁
  fn current_id_lock(&self) -> std::sync::MutexGuard<'_, Option<String>> {
    self.current_explain_image_id.lock().unwrap_or_else(|e| e.into_inner())
  }

  /// 选择一个可用的 API Key 索引：
  /// 优先返回 active_key_idx 记录的 idx；若它在冷却中或已被试过，退回到下一个非冷却 idx；
  /// 全部冷却或 tried 已穷举时返回 None（调用方决定是否报错）。
  fn pick_active_key(
    &self,
    provider_id: &str,
    total: usize,
    tried: &HashSet<usize>,
  ) -> Option<usize> {
    if total == 0 {
      return None;
    }
    let now = Instant::now();
    let cooldowns = self
      .key_cooldowns
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    let active = self
      .active_key_idx
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .get(provider_id)
      .copied()
      .unwrap_or(0)
      .min(total.saturating_sub(1));

    let in_cooldown = |idx: usize| {
      cooldowns
        .get(&(provider_id.to_string(), idx))
        .map(|until| *until > now)
        .unwrap_or(false)
    };

    // 1) 优先 active idx（未试过 + 未冷却）
    if !tried.contains(&active) && !in_cooldown(active) {
      return Some(active);
    }
    // 2) 从 active+1 开始环绕扫描
    for offset in 1..total {
      let idx = (active + offset) % total;
      if !tried.contains(&idx) && !in_cooldown(idx) {
        return Some(idx);
      }
    }
    // 3) 全部冷却 → 兜底找一个未试过的（无视冷却，避免完全无 key 可用）
    for offset in 0..total {
      let idx = (active + offset) % total;
      if !tried.contains(&idx) {
        return Some(idx);
      }
    }
    None
  }

  /// 标记某个 key 失败：进入冷却 + 不变更 active_key_idx
  fn mark_key_failed(&self, provider_id: &str, idx: usize) {
    let mut cooldowns = self
      .key_cooldowns
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    cooldowns.insert((provider_id.to_string(), idx), Instant::now() + KEY_COOLDOWN);
  }

  /// 标记某个 key 成功：清除该 idx 的冷却 + 设为 active
  fn mark_key_ok(&self, provider_id: &str, idx: usize) {
    let mut cooldowns = self
      .key_cooldowns
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    cooldowns.remove(&(provider_id.to_string(), idx));
    drop(cooldowns);
    let mut active = self
      .active_key_idx
      .lock()
      .unwrap_or_else(|e| e.into_inner());
    active.insert(provider_id.to_string(), idx);
  }
}

/// 自启动参数，用于区分用户手动启动和系统自动启动
const AUTOSTART_ARG: &str = "--from-autostart";

/// 供应商连接输入参数，用于测试连接或获取模型列表时临时传入
/// api_keys 优先；api_key 为兼容旧前端发的单 key 字段（v2.3.x 时的 ProviderConnectionInput）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderConnectionInput {
  id: Option<String>,
  base_url: String,
  #[serde(default)]
  api_keys: Vec<String>,
  #[serde(default)]
  api_key: Option<String>,
}

impl ProviderConnectionInput {
  /// 整理出非空 key 列表：优先 api_keys，回退到 api_key。
  fn merged_keys(&self) -> Vec<String> {
    let mut keys: Vec<String> = self
      .api_keys
      .iter()
      .map(|k| k.trim().to_string())
      .filter(|k| !k.is_empty())
      .collect();
    if keys.is_empty() {
      if let Some(legacy) = self.api_key.as_deref() {
        let trimmed = legacy.trim().to_string();
        if !trimmed.is_empty() {
          keys.push(trimmed);
        }
      }
    }
    keys
  }
}

/// 解析供应商的凭据信息（base_url + 多 key 列表）
/// 优先使用传入的 ProviderConnectionInput（如测试连接时），否则从 settings 中查找对应的供应商
fn resolve_provider_credentials(
  settings: &Settings,
  provider_id: &str,
  provider: Option<ProviderConnectionInput>,
) -> Result<(String, Vec<String>), String> {
  if let Some(input) = provider {
    let id_matches = input
      .id
      .as_ref()
      .map(|id| id.is_empty() || id == provider_id)
      .unwrap_or(true);

    if id_matches {
      return Ok((input.base_url.clone(), input.merged_keys()));
    }
  }

  let provider = settings
    .get_provider(provider_id)
    .ok_or_else(|| "Provider not found".to_string())?;
  Ok((provider.base_url.clone(), provider.api_keys.clone()))
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

/// 使用系统默认浏览器打开外部链接（仅限 https）
#[tauri::command]
#[allow(deprecated)]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
  if !url.starts_with("https://") {
    return Err("Invalid URL".to_string());
  }

  app.shell().open(url, None).map_err(|e| e.to_string())
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
  if let Some(c) = &cursor_opt {
    eprintln!("[lens-pos] cursor (physical): ({}, {})", c.x, c.y);
  } else {
    eprintln!("[lens-pos] cursor_position unavailable, will fall back to primary monitor");
  }

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
  for (i, monitor) in monitors.iter().enumerate() {
    let mp = monitor.position();
    let ms = monitor.size();
    let scale = monitor.scale_factor();
    eprintln!(
      "[lens-pos] monitor[{}] pos=({},{}) size={}x{} scale={}",
      i, mp.x, mp.y, ms.width, ms.height, scale
    );
  }

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
      if p.is_some() {
        eprintln!("[lens-pos] no monitor matched cursor, falling back to primary");
      }
      // primary_monitor 返回 Option<Monitor> 而 monitors iter 给的是 &Monitor，
      // 这里需要从 monitors 里按 name 找回相同的 monitor 引用，避免类型不一致
      p.and_then(|prim| monitors.iter().find(|m| m.name() == prim.name()))
    })
    .or_else(|| {
      eprintln!("[lens-pos] primary unavailable, falling back to monitors[0]");
      monitors.first()
    });

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
  eprintln!(
    "[lens-pos] -> set_position logical=({}, {}) size=({}, {})",
    lx, ly, lw, lh
  );
  let _ = window.set_position(tauri::LogicalPosition::new(lx, ly));
  let _ = window.set_size(tauri::LogicalSize::new(lw, lh));
  if let Ok(op) = window.outer_position() {
    eprintln!("[lens-pos] verify outer_position physical=({}, {})", op.x, op.y);
  }
}

/// 入口（公共底层）：打开 lens webview 进入 select 态。
/// mode：
///   - "chat"（默认）：截完进对话栏 ready 态
///   - "translate"：截完直接做 OCR + 翻译，弹原文/译文浮动卡
fn lens_request_internal(app: &AppHandle, mode: &str) -> Result<(), String> {
  // 预热 SCK SCShareableContent 缓存，摊销首次截图的 WindowServer 查询开销。
  // 用户从按热键到选目标 + 单击截图通常 ≥ 300 ms，足以盖住 30-80 ms 的 prewarm。
  #[cfg(target_os = "macos")]
  crate::sck::prewarm();

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
  let window = match windows::ensure_lens_window(app) {
    Ok(w) => w,
    Err(e) => {
      state.lens_busy.store(false, Ordering::SeqCst);
      return Err(e);
    }
  };
  // 把 mode 编码进 hash query，前端通过 location.hash 读取（'#lens?mode=translate'）
  let safe_mode = if mode == "translate" { "translate" } else { "chat" };
  let script = format!(
    "window.location.hash = '#lens?mode={mode}'; window.dispatchEvent(new HashChangeEvent('hashchange')); window.dispatchEvent(new CustomEvent('lens:reset'));",
    mode = safe_mode,
  );
  let _ = window.eval(&script);
  // 先在 hidden 状态下尝试定位：即便部分系统下 hidden 窗口 set_position 被忽略，也比
  // 不调强（成功则消除"先在旧位置闪一帧再跳到全屏"的可见跳变）。
  lens_position_fullscreen(app, &window);
  let _ = window.show();
  let _ = window.set_focus();
  // show 后再调，处理 always_on_top + visible_on_all_workspaces 把首次 set_position 吃掉的情况
  lens_position_fullscreen(app, &window);
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
  } else if settings.target_lang == "zh" || settings.target_lang == "en" {
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
  if ocr_provider.api_keys.is_empty() {
    return Ok(serde_json::json!({ "success": false, "error": "Missing API Key" }));
  }

  let retry_attempts = effective_retry_attempts(&settings);
  let direct_translate = settings.screenshot_translation.direct_translate;
  let st_thinking = settings.screenshot_translation.thinking_enabled;
  let st_stream = settings.screenshot_translation.stream_enabled;

  let target_lang = resolve_target_lang(&settings.target_lang, "");
  let lang_name = language_name(&target_lang).to_string();

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

/// 构造截图翻译 OCR 请求 body（与 call_openai_ocr 保持一致，stream=true）
fn build_ocr_request_body(
  image_path: &Path,
  prompt: &str,
  thinking_enabled: bool,
) -> Result<serde_json::Value, String> {
  let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);
  let mut body = serde_json::json!({
    "messages": [{
      "role": "user",
      "content": [
        { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{base64}") } },
        { "type": "text", "text": prompt }
      ]
    }],
    "temperature": 0.2,
    "max_tokens": 2000,
    "stream": true
  });
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }
  Ok(body)
}

/// 通用流式 chat 调用：发送 body（model 在外层注入）→ 解析 SSE → 通过 stream_vision_response emit。
/// 复用 explain_stream_generation 作取消代号（lens-stream / lens-translate-stream 都共用）。
#[allow(clippy::too_many_arguments)]
async fn stream_chat_call(
  app: &AppHandle,
  state: &State<'_, AppState>,
  provider: &settings::ModelProvider,
  model: &str,
  mut body: serde_json::Value,
  retry_attempts: usize,
  image_id: &str,
  kind: &str,
  event_name: &str,
) -> Result<String, String> {
  body["model"] = serde_json::json!(model);
  let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

  let response = send_with_failover(
    state,
    "Stream chat",
    retry_attempts,
    &provider.id,
    &provider.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet: String = body_text.chars().take(500).collect();
    return Err(format!("Stream HTTP {}: {}", status.as_u16(), snippet));
  }

  let generation = state
    .explain_stream_generation
    .fetch_add(1, Ordering::SeqCst)
    + 1;
  stream_vision_response(
    app,
    response,
    image_id,
    kind,
    event_name,
    &state.explain_stream_generation,
    generation,
  )
  .await
}

/// 截图翻译合并模式流：单次调用模型，按 `<<<ORIGINAL>>>` 分隔符把 SSE delta 拆成两段。
/// 分隔符前的 chunk emit kind="translated"；分隔符后的 chunk emit kind="original"。
/// 返回 (translated, original) 完整文本。
///
/// 关键点：
/// - 分隔符可能跨 SSE chunk 边界 → 用 tail 缓冲住末尾 (SEPARATOR.len()-1) 字节防止把分隔符前缀当成译文 emit 出去
/// - tail 切片必须落在 UTF-8 char boundary，否则 String::drain 会 panic（用户截图常含 CJK，每字 3 字节）
async fn stream_translate_combined(
  app: &AppHandle,
  state: &State<'_, AppState>,
  provider: &settings::ModelProvider,
  model: &str,
  mut body: serde_json::Value,
  retry_attempts: usize,
  image_id: &str,
  event_name: &str,
) -> Result<(String, String), String> {
  body["model"] = serde_json::json!(model);
  let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

  let mut response = send_with_failover(
    state,
    "Stream translate combined",
    retry_attempts,
    &provider.id,
    &provider.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet: String = body_text.chars().take(500).collect();
    return Err(format!("Stream HTTP {}: {}", status.as_u16(), snippet));
  }

  let my_gen = state
    .explain_stream_generation
    .fetch_add(1, Ordering::SeqCst)
    + 1;

  let sep = COMBINED_TRANSLATE_SEPARATOR;
  let sep_len = sep.len();

  let mut sse_buf = String::new();
  let mut tail = String::new();
  let mut translated = String::new();
  let mut original = String::new();
  let mut sep_seen = false;

  let emit_done = |reason: &str| {
    let _ = app.emit(
      event_name,
      serde_json::json!({
        "imageId": image_id, "delta": "", "done": true, "reason": reason,
      }),
    );
  };

  loop {
    if state.explain_stream_generation.load(Ordering::SeqCst) != my_gen {
      // 取消：把 tail 当作 translated flush（避免末尾几个字符丢失），再 emit done
      if !tail.is_empty() && !sep_seen {
        translated.push_str(&tail);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
        );
      }
      emit_done("cancelled");
      return Ok((translated, original));
    }

    let chunk = match response.chunk().await {
      Ok(Some(c)) => c,
      Ok(None) => break,
      Err(e) => {
        emit_done("error");
        return Err(e.to_string());
      }
    };

    let text = String::from_utf8_lossy(&chunk);
    sse_buf.push_str(&text);

    while let Some(pos) = sse_buf.find('\n') {
      let line: String = sse_buf.drain(..=pos).collect();
      let line = line.trim();
      if !line.starts_with("data:") {
        continue;
      }
      let data = line.trim_start_matches("data:").trim();
      if data.is_empty() {
        continue;
      }
      if data == "[DONE]" {
        // flush tail
        if !sep_seen && !tail.is_empty() {
          translated.push_str(&tail);
          let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
          );
        }
        emit_done("done");
        return Ok((translated, original));
      }

      let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(val) => val,
        Err(_) => continue,
      };

      let delta_obj = value
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("delta"));

      // 推理链 emit（恒定 kind="translated"，前端在主面板渲染）
      if let Some(r) = delta_obj
        .and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
      {
        let _ = app.emit(
          event_name,
          serde_json::json!({
            "imageId": image_id, "kind": "translated", "delta": "", "reasoningDelta": r,
          }),
        );
      }

      let content = delta_obj
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

      let Some(c) = content else { continue };

      if sep_seen {
        original.push_str(c);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": "original", "delta": c }),
        );
        continue;
      }

      tail.push_str(c);
      if let Some(idx) = tail.find(sep) {
        // 分隔符命中：拆 before / after，trim 掉分隔符相邻的换行，分别发出
        let before: String = tail.drain(..idx).collect();
        // 移除分隔符本身
        tail.drain(..sep_len);
        let after: String = std::mem::take(&mut tail);

        let before_emit = before.trim_end_matches('\n').to_string();
        if !before_emit.is_empty() {
          translated.push_str(&before_emit);
          let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": before_emit }),
          );
        }
        sep_seen = true;
        let after_emit = after.trim_start_matches('\n').to_string();
        if !after_emit.is_empty() {
          original.push_str(&after_emit);
          let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "original", "delta": after_emit }),
          );
        }
      } else {
        // 没命中：emit 安全前缀（保留末尾 sep_len-1 字节防止跨 chunk 分隔符被切碎）
        let max_emit = tail.len().saturating_sub(sep_len.saturating_sub(1));
        if max_emit == 0 {
          continue;
        }
        // 找一个合法 char boundary（CJK 字符多字节，不能切到字符中间）
        let mut safe = max_emit;
        while safe > 0 && !tail.is_char_boundary(safe) {
          safe -= 1;
        }
        if safe == 0 {
          continue;
        }
        let to_emit: String = tail.drain(..safe).collect();
        translated.push_str(&to_emit);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": to_emit }),
        );
      }
    }
  }

  // SSE 流结束（连接关闭）但没收到 [DONE]：flush tail
  if !sep_seen && !tail.is_empty() {
    translated.push_str(&tail);
    let _ = app.emit(
      event_name,
      serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
    );
  }
  emit_done("done");
  Ok((translated, original))
}

/// 关闭 lens：清理图片、释放 busy、隐藏窗口。
///
/// hide 前先把窗口几何复位到当前光标所在显示器的全屏，避免下次 show 出来时还停在
/// 上一次截图后的浮动 bar 位置（先在旧位置闪一帧再跳到 select 全屏的可见跳变）。
#[tauri::command]
fn lens_close(app: AppHandle) -> Result<(), String> {
  let state = app.state::<AppState>();
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

// ====== /Lens 模式命令 ======

/// 从供应商 API 获取可用模型列表
#[tauri::command]
async fn fetch_models(
  state: State<'_, AppState>,
  provider_id: String,
  provider: Option<ProviderConnectionInput>,
) -> Result<Vec<String>, String> {
    println!("Fetching models for provider: {}", provider_id);
    let settings = state.settings_read().clone();
    let (base_url, api_keys) = resolve_provider_credentials(&settings, &provider_id, provider)?;
    let retry_attempts = effective_retry_attempts(&settings);

    if api_keys.is_empty() {
        return Err("Missing API Key".to_string());
    }

    let url = format!("{}/models", base_url.trim_end_matches('/'));
    println!("Requesting URL: {}", url);

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
          let handle = app.clone();
          tauri::async_runtime::spawn(async move {
            if let Err(err) = lens_request_translate(handle) {
              eprintln!("Screenshot translation trigger error: {err}");
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
          let handle = app.clone();
          tauri::async_runtime::spawn(async move {
            if let Err(err) = lens_request(handle) {
              eprintln!("Lens trigger error: {err}");
            }
          });
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

  eprintln!(
    "capture region: abs_logical=({}, {}), monitor=({}, {}), monitor_scale={}, physical=({}, {}), region={}x{}",
    absolute_x,
    absolute_y,
    monitor_x,
    monitor_y,
    monitor_scale,
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

/// 清理截图临时文件：从映射中移除并删除磁盘文件
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
fn resolve_explain_image_path(
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

/// 调用 OpenAI 兼容的文本聊天接口
/// 发送单轮 user 消息，temperature 设为 0.2，返回模型生成的文本内容
async fn call_openai_text(
  state: &State<'_, AppState>,
  config: &settings::ModelProvider,
  model: &str,
  prompt: String,
  retry_attempts: usize,
  thinking_enabled: bool,
) -> Result<String, String> {
  let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
  let mut body = serde_json::json!({
    "model": model,
    "messages": [{ "role": "user", "content": prompt }],
    "temperature": 0.2
  });
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }

  let response = send_with_failover(
    state,
    "OpenAI API",
    retry_attempts,
    &config.id,
    &config.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
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

/// 截图翻译合并模式分隔符。模型先输出译文，再单独一行 `<<<ORIGINAL>>>`，再输出原文。
/// 流式解析时按此切分两段，分别 emit kind="translated" / "original"。
pub const COMBINED_TRANSLATE_SEPARATOR: &str = "<<<ORIGINAL>>>";

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

/// 构建合并模式提示词：模型在一次调用中先输出译文、再 `<<<ORIGINAL>>>` 分隔符、再输出原文
/// 这样译文先出现在流里（用户立即看到结果），整体只走一次 round-trip
///
/// 用户自定义 template（settings.screenshot_translation.prompt）若非空，会被作为
/// "Translation rules" 块注入；空则使用默认规则。{lang} 占位符替换为目标语言；{text}
/// 在合并模式不存在外部参数 → 替换为占位说明 "the recognized text"。
fn build_combined_translate_prompt(lang_name: &str, template: Option<&str>) -> String {
  const DEFAULT_RULES: &str = "- Preserve LaTeX formulas ($...$ inline, $$...$$ block).\n\
     - Keep paragraph and line-break structure.\n\
     - Correct only obvious OCR mistakes; do not invent missing content.\n\
     - No commentary, no section headers, no labels.";

  let rules = template
    .map(str::trim)
    .filter(|t| !t.is_empty())
    .map(|t| {
      t.replace("{lang}", lang_name)
        .replace("{text}", "the recognized text")
    })
    .unwrap_or_else(|| DEFAULT_RULES.to_string());

  format!(
    "Read this screenshot. Output two sections in this exact order, separated by a line containing only `{sep}`:\n\n\
     1. Translation in {lang}: a faithful translation of all text shown in the screenshot.\n\
     2. Original recognized text exactly as it appears in the screenshot.\n\n\
     Translation rules:\n{rules}\n\n\
     Format guard:\n\
     - The line `{sep}` must appear exactly once on its own line, between the two sections.\n\
     - No commentary, no labels like 'Translation:' or 'Original:'.\n\n\
     Output format (replace placeholders):\n\
     <translation>\n\
     {sep}\n\
     <original>",
    lang = lang_name,
    sep = COMBINED_TRANSLATE_SEPARATOR,
    rules = rules,
  )
}

/// 调用 OpenAI 兼容的 OCR/视觉接口
/// 将图片转为 Base64 后作为 image_url 类型内容发送，temperature 设为 0 以提高识别稳定性
async fn call_openai_ocr(
  state: &State<'_, AppState>,
  config: &settings::ModelProvider,
  model: &str,
  image_path: &Path,
  prompt: &str,
  retry_attempts: usize,
  thinking_enabled: bool,
) -> Result<String, String> {
  let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
  let base64 = general_purpose::STANDARD.encode(bytes);
  let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

  // 与 lens 的 vision body 对齐：image 在 text 前、显式 max_tokens。
  // thinking 按调用方传入：截图翻译默认 false（节省时间），lens 默认 true。
  let mut body = serde_json::json!({
    "model": model,
    "messages": [
      {
        "role": "user",
        "content": [
          {
            "type": "image_url",
            "image_url": { "url": format!("data:image/png;base64,{base64}") }
          },
          {
            "type": "text",
            "text": prompt
          }
        ]
      }
    ],
    "temperature": 0.2,
    "max_tokens": 2000
  });
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }

  let response = send_with_failover(
    state,
    "OpenAI OCR",
    retry_attempts,
    &config.id,
    &config.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  // 显式检查 HTTP 状态：非 2xx 把原始 body 文本带回，避免后续 .json() 抛出含糊的 "error decoding response body"
  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet: String = body_text.chars().take(500).collect();
    return Err(format!("OCR HTTP {}: {}", status.as_u16(), snippet));
  }

  let raw = response.text().await.map_err(|e| format!("OCR read body: {}", e))?;
  let value: serde_json::Value = serde_json::from_str(&raw)
    .map_err(|e| format!("OCR parse JSON: {} (body: {})", e, raw.chars().take(500).collect::<String>()))?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| format!("Invalid OCR response: {}", raw.chars().take(500).collect::<String>()))?;

  Ok(content.trim().to_string())
}

/// 调用视觉 API（截图解释 / Lens 共用）
/// 支持流式输出：如果 stream 为 true，通过 stream_vision_response 逐段 emit `event_name` 事件。
/// `provider_id_override` 非空时使用指定 provider/model（用于 lens 选择独立模型）；空则走 explain 配置。
async fn call_vision_api(
  app: &AppHandle,
  state: &State<'_, AppState>,
  image_id: &str,
  messages: Vec<ExplainMessage>,
  language: &str,
  retry_attempts: usize,
  stream: bool,
  stream_kind: &str,
  event_name: &str,
  provider_id_override: Option<&str>,
  model_override: Option<&str>,
  system_prompt_override: Option<&str>,
  thinking_enabled: bool,
) -> Result<String, String> {
  let settings = state.settings_read().clone();
  let provider_id = provider_id_override
    .filter(|s| !s.is_empty())
    .unwrap_or(&settings.translator_provider_id);
  let provider = settings.get_provider(provider_id)
    .ok_or_else(|| "Vision provider not found".to_string())?;

  // image_id 为空 → 走纯文本对话路径（不附图）
  let has_image = !image_id.is_empty();

  let mut api_messages = Vec::new();
  // 优先用调用方传入的 system_prompt_override；否则用默认模板（区分有/无图片）
  // 关闭思考时在 system 末尾追加显式禁止指令，作为参数层不生效时的兜底
  let system_prompt_to_use: Option<String> = {
    let base = match system_prompt_override.filter(|s| !s.is_empty()) {
      Some(s) => s.to_string(),
      None => default_system_prompt(language, has_image),
    };
    if !thinking_enabled {
      Some(format!("{}{}", base, no_think_instruction(language)))
    } else {
      Some(base)
    }
  };
  if let Some(sp) = system_prompt_to_use {
    api_messages.push(serde_json::json!({
      "role": "system",
      "content": sp
    }));
  }

  if has_image {
    let image_path = resolve_explain_image_path(app, state, image_id)?;
    let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
    let base64 = general_purpose::STANDARD.encode(bytes);
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
  } else {
    // 纯文本：每条 message 直接 push（无图）
    for message in messages.iter() {
      api_messages.push(serde_json::json!({
        "role": message.role,
        "content": message.content,
      }));
    }
  }

  let model = model_override
    .filter(|s| !s.is_empty())
    .unwrap_or(&settings.translator_model);
  let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
  let mut body = serde_json::json!({
    "model": model,
    "messages": api_messages,
    "temperature": 0.7,
    "max_tokens": 2000
  });
  if stream {
    body["stream"] = serde_json::json!(true);
  }

  // 关闭思考模式：仅塞 DeepSeek/Kimi 官方文档约定的 thinking={type:"disabled"} 字段。
  // 不再注入 chat_template_kwargs / enable_thinking / reasoning_effort —— 这些是 vLLM/Qwen/OpenAI
  // 私有字段，第三方代理（如 OpenRouter / 反代）做严格校验时会以 400 拒绝整个请求（实测 DeepSeek
  // 路径上 chat_template_kwargs 直接报错）。提示词层的 no-think 指令是更稳的兜底。
  if !thinking_enabled {
    body["thinking"] = serde_json::json!({ "type": "disabled" });
  }

  let response = send_with_failover(
    state,
    "Vision API",
    retry_attempts,
    &provider.id,
    &provider.api_keys,
    |key| {
      state
        .http
        .post(url.clone())
        .bearer_auth(key)
        .json(&body)
        .send()
    },
  )
  .await?;

  // 先检查 HTTP 状态：非 2xx 直接读出 body 文本作为错误，避免后续 .json() / chunk() 拿到非预期格式时抛出含糊的 "error decoding response body"。
  let status = response.status();
  if !status.is_success() {
    let body_text = response.text().await.unwrap_or_default();
    let snippet = body_text.chars().take(500).collect::<String>();
    return Err(format!("Vision API HTTP {}: {}", status.as_u16(), snippet));
  }

  if stream {
    // 启动新流：递增代号，存到本流持有的快照里；后续 chunk 循环只要发现全局代号 != 自己的快照就退出。
    let generation = state
      .explain_stream_generation
      .fetch_add(1, Ordering::SeqCst)
      + 1;
    return stream_vision_response(
      app,
      response,
      image_id,
      stream_kind,
      event_name,
      &state.explain_stream_generation,
      generation,
    )
    .await;
  }

  // 非流式：先读 raw text，再 parse JSON，把原始 body 作为错误信息便于诊断。
  let raw = response.text().await.map_err(|e| format!("Vision API read body: {}", e))?;
  let value: serde_json::Value = serde_json::from_str(&raw)
    .map_err(|e| format!("Vision API parse JSON: {} (body: {})", e, raw.chars().take(500).collect::<String>()))?;
  let content = value
    .get("choices")
    .and_then(|choices| choices.get(0))
    .and_then(|choice| choice.get("message"))
    .and_then(|message| message.get("content"))
    .and_then(|content| content.as_str())
    .ok_or_else(|| format!("Invalid vision response: {}", raw.chars().take(500).collect::<String>()))?;

  Ok(content.trim().to_string())
}

/// 流式解析视觉 API 的 SSE 响应
/// 逐 chunk 读取响应体，解析 "data:" 行，提取 delta 中的 content 并通过 `event_name` emit。
/// 支持取消：调用方持有 `my_generation`，全局代号 `generation_atom` 一旦变化即视为被新流或外部取消作废。
async fn stream_vision_response(
  app: &AppHandle,
  mut response: reqwest::Response,
  image_id: &str,
  kind: &str,
  event_name: &str,
  generation_atom: &AtomicU64,
  my_generation: u64,
) -> Result<String, String> {
  let mut buffer = String::new();
  let mut full = String::new();

  let emit_done = |reason: &str, full_text: &str| {
    let _ = app.emit(
      event_name,
      serde_json::json!({
        "imageId": image_id,
        "kind": kind,
        "delta": "",
        "done": true,
        "reason": reason,
        "full": full_text,
      }),
    );
  };

  loop {
    if generation_atom.load(Ordering::SeqCst) != my_generation {
      emit_done("cancelled", full.trim());
      return Ok(full.trim().to_string());
    }

    let chunk = match response.chunk().await {
      Ok(Some(c)) => c,
      Ok(None) => break,
      Err(e) => {
        emit_done("error", full.trim());
        return Err(e.to_string());
      }
    };

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
        emit_done("done", full.trim());
        return Ok(full.trim().to_string());
      }

      let value: serde_json::Value = match serde_json::from_str(data) {
        Ok(val) => val,
        Err(_) => continue,
      };

      let delta_obj = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("delta"));

      // 推理模型（DeepSeek-R1 / Kimi 等）把链路放在 delta.reasoning_content
      // 部分实现用 delta.reasoning。两种字段都尝试取，只要有就 emit。
      let reasoning = delta_obj
        .and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

      if let Some(r) = reasoning {
        let _ = app.emit(
          event_name,
          serde_json::json!({
            "imageId": image_id,
            "kind": kind,
            "delta": "",
            "reasoningDelta": r,
          }),
        );
      }

      let content = delta_obj
        .and_then(|d| d.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

      if let Some(content) = content {
        full.push_str(content);
        let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": kind, "delta": content }),
        );
      }
    }
  }

  emit_done("done", full.trim());
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

/// 判断错误信息是否触发 key failover
/// failover 条件：429（限流）/ 401（鉴权失败）/ 402（余额不足）/ 403（权限/封禁）+
/// body 含 quota / rate_limit / billing / credit / balance 关键字
fn is_failover_error(err_msg: &str) -> bool {
  // 状态码：send_with_retry 失败信息含 " Error: <STATUS>"，如 " Error: 429"
  let status_hits = err_msg.contains(" Error: 429")
    || err_msg.contains(" Error: 401")
    || err_msg.contains(" Error: 402")
    || err_msg.contains(" Error: 403");
  if status_hits {
    return true;
  }
  let lower = err_msg.to_lowercase();
  lower.contains("insufficient_quota")
    || lower.contains("quota_exceeded")
    || lower.contains("rate_limit_exceeded")
    || lower.contains("rate-limit")
    || lower.contains("out_of_credit")
    || lower.contains("out of credit")
    || lower.contains("insufficient balance")
    || lower.contains("insufficient_balance")
    || lower.contains("exceeded your current quota")
    || lower.contains("billing")
}

/// 多 key failover 包装：在 api_keys 列表上依次尝试，遇到 failover-eligible 错误自动切下一 key
/// 内层每次尝试仍走 send_with_retry（处理网络抖动 / 服务端 5xx 等通用重试）
async fn send_with_failover<F, Fut>(
  state: &AppState,
  label: &str,
  attempts: usize,
  provider_id: &str,
  api_keys: &[String],
  send: F,
) -> Result<reqwest::Response, String>
where
  F: Fn(&str) -> Fut,
  Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
  let total = api_keys.len();
  if total == 0 {
    return Err(format!("{} Error: No API key configured", label));
  }

  let mut tried: HashSet<usize> = HashSet::new();
  let mut last_err: Option<String> = None;

  while tried.len() < total {
    let idx = match state.pick_active_key(provider_id, total, &tried) {
      Some(i) => i,
      None => break,
    };
    tried.insert(idx);
    let key = api_keys[idx].as_str();

    match send_with_retry(label, attempts, || send(key)).await {
      Ok(resp) => {
        state.mark_key_ok(provider_id, idx);
        return Ok(resp);
      }
      Err(err_msg) => {
        if is_failover_error(&err_msg) && tried.len() < total {
          state.mark_key_failed(provider_id, idx);
          eprintln!(
            "[failover] {} key #{}/{} failed, switching to next: {}",
            label,
            idx + 1,
            total,
            err_msg
          );
          last_err = Some(err_msg);
          continue;
        }
        // 非 failover 错误（或已穷举所有 key）→ 直接返回
        if is_failover_error(&err_msg) {
          state.mark_key_failed(provider_id, idx);
        }
        return Err(err_msg);
      }
    }
  }

  Err(
    last_err.unwrap_or_else(|| format!("{} Error: all {} keys exhausted", label, total)),
  )
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
    .settings_read()
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
        key_cooldowns: Mutex::new(HashMap::new()),
        active_key_idx: Mutex::new(HashMap::new()),
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
      lens_list_windows,
      lens_capture_window,
      lens_capture_region,
      lens_ask,
      lens_translate,
      lens_cancel_stream,
      lens_close,
      lens_commit_image_to_history,
      lens_delete_history_image,
      set_always_on_top
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
