#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
use tauri::{
  AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_shell::ShellExt;
use uuid::Uuid;

use screenshot::{capture_screenshot, cleanup_temp_file};
use settings::{
  default_question_prompt, default_summary_prompt, default_system_prompt, load_settings,
  persist_settings, sanitize_settings, ExplainHistoryRecord, ExplainMessage, OpenAIConfig, Settings,
};
use utils::{current_timestamp, language_name, resolve_target_lang};
use windows::{apply_macos_workspace_behavior, ensure_main_window, ensure_screenshot_window, get_main_window};

struct AppState {
  settings: RwLock<Settings>,
  explain_images: Mutex<HashMap<String, PathBuf>>,
  current_explain_image_id: Mutex<Option<String>>,
  screenshot_translation_busy: AtomicBool,
  screenshot_explain_busy: AtomicBool,
  http: Client,
}

struct BusyGuard<'a> {
  flag: &'a AtomicBool,
}

impl<'a> BusyGuard<'a> {
  fn new(flag: &'a AtomicBool) -> Option<Self> {
    if flag.swap(true, Ordering::SeqCst) {
      None
    } else {
      Some(Self { flag })
    }
  }
}

impl Drop for BusyGuard<'_> {
  fn drop(&mut self) {
    self.flag.store(false, Ordering::SeqCst);
  }
}

fn build_http_client() -> Client {
  Client::builder()
    .timeout(Duration::from_secs(60))
    .build()
    .unwrap_or_else(|err| {
      eprintln!("Failed to build HTTP client: {err}");
      Client::new()
    })
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
  state.settings.read().expect("settings lock").clone()
}

#[tauri::command]
fn get_default_prompt_templates() -> serde_json::Value {
  serde_json::json!({
    "translationTemplate": DEFAULT_TRANSLATION_TEMPLATE,
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

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<(), String> {
  let sanitized = sanitize_settings(settings);
  {
    let mut guard = state.settings.write().expect("settings lock");
    *guard = sanitized.clone();
  }

  if let Err(err) = persist_settings(&app, &sanitized) {
    eprintln!("Failed to save settings: {err}");
    return Err(err);
  }

  register_hotkeys(&app)?;
  if let Err(err) = setup_tray(&app) {
    eprintln!("Failed to update tray: {err}");
  }

  Ok(())
}

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

#[tauri::command]
fn commit_translation(app: AppHandle, state: State<AppState>, text: String) -> Result<(), String> {
  if text.trim().is_empty() {
    return Ok(());
  }

  let auto_paste = state.settings.read().expect("settings lock").auto_paste;
  let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
  clipboard.set_text(text).map_err(|e| e.to_string())?;

  // Always hide window first to allow focus to switch back to previous app
  if let Some(window) = get_main_window(&app) {
    let _ = window.hide();
  }

  #[cfg(target_os = "macos")]
  unsafe {
    use cocoa::base::{id, nil};
    use objc::{msg_send, sel, sel_impl, class};
    let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
    let _: () = msg_send![ns_app, hide: nil];
  }

  if auto_paste {
    // Increased delay to ensure focus switch completes
    thread::sleep(Duration::from_millis(600));
    send_paste_shortcut();
  }

  Ok(())
}

#[tauri::command]
fn open_external(app: AppHandle, url: String) -> Result<(), String> {
  if !url.starts_with("https://") {
    return Err("Invalid URL".to_string());
  }

  app.shell().open(url, None).map_err(|e| e.to_string())
}

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

#[tauri::command]
fn explain_get_history(state: State<AppState>) -> Result<serde_json::Value, String> {
  let settings = state.settings.read().expect("settings lock");
  Ok(serde_json::json!({
    "success": true,
    "history": settings.explain_history
  }))
}

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

#[tauri::command]
async fn fetch_models(state: State<'_, AppState>, provider_id: String) -> Result<Vec<String>, String> {
    println!("Fetching models for provider: {}", provider_id);
    let settings = state.settings.read().expect("settings lock").clone();
    let provider = settings.get_provider(&provider_id)
        .ok_or_else(|| "Provider not found".to_string())?;
    let retry_attempts = effective_retry_attempts(&settings);

    if provider.api_key.trim().is_empty() {
        return Err("Missing API Key".to_string());
    }

    let url = format!("{}/models", provider.base_url.trim_end_matches('/'));
    println!("Requesting URL: {}", url);

    let response = send_with_retry("Models API", retry_attempts, || {
        state.http
            .get(url.clone())
            .bearer_auth(&provider.api_key)
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


fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
  let settings = app.state::<AppState>().settings.read().expect("settings lock").clone();
  let shortcut_manager = app.global_shortcut();
  shortcut_manager.unregister_all().map_err(|e| e.to_string())?;
  let mut errors = Vec::new();
  let mut registered = HashSet::new();

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
      errors.push(format!("Failed to register translator hotkey \"{hotkey}\": {err}"));
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
        errors.push(format!("Failed to register screenshot translation hotkey \"{hotkey}\": {err}"));
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
        errors.push(format!("Failed to register screenshot explain hotkey \"{hotkey}\": {err}"));
      }
    }
  }

  if errors.is_empty() {
    Ok(())
  } else {
    Err(errors.join("\n"))
  }
}

fn get_mouse_position(app: &AppHandle) -> Option<tauri::PhysicalPosition<f64>> {
  app.cursor_position().ok()
}

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
  let screenshot_window = ensure_screenshot_window(app)?;
  screenshot_window.emit("screenshot-processing", ())
    .map_err(|e| e.to_string())?;
  let _ = screenshot_window.show();
  let _ = screenshot_window.set_focus();

  let settings = state.settings.read().expect("settings lock").clone();
  let provider = settings.get_provider(&settings.screenshot_translation.provider_id)
    .ok_or_else(|| "OCR provider not found".to_string())?;

  if provider.api_key.trim().is_empty() {
    screenshot_window
      .emit("screenshot-error", "Missing API Key")
      .map_err(|e| e.to_string())?;
    cleanup_temp_file(&temp_path);
    return Ok(());
  }

  let retry_attempts = effective_retry_attempts(&settings);
  let recognized = call_openai_ocr(
    &state.http,
    provider,
    &settings.screenshot_translation.model,
    &temp_path,
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

  let translated = if recognized.trim().is_empty() {
    "".to_string()
  } else {
    let target_lang = resolve_target_lang(&settings.target_lang, &recognized);
    let lang_name = language_name(&target_lang).to_string();
    let prompt = build_translation_prompt(
      &recognized,
      &lang_name,
      settings.screenshot_translation.prompt.as_deref(),
    );
    
    // Also use the translator provider for the secondary translation
    let t_provider = settings.get_provider(&settings.translator_provider_id)
        .unwrap_or(provider);

    call_openai_text(
      &state.http,
      t_provider,
      &settings.translator_model,
      prompt,
      retry_attempts,
    )
    .await
    .unwrap_or(recognized.clone())
  };

  app
    .emit(
      "screenshot-result",
      serde_json::json!({ "original": recognized, "translated": translated }),
    )
    .map_err(|e| e.to_string())?;

  cleanup_temp_file(&temp_path);
  Ok(())
}

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
  let image_id = Uuid::new_v4().to_string();

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

  let window = ensure_explain_window(app, &image_id)?;
  let _ = window.show();
  let _ = window.set_focus();

  Ok(())
}



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

const DEFAULT_TRANSLATION_TEMPLATE: &str =
  "Translate the following text to {lang}. Only output the translation.\n\n{text}";

fn build_translation_prompt(text: &str, lang_name: &str, template: Option<&str>) -> String {
  let default_prompt = DEFAULT_TRANSLATION_TEMPLATE
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

async fn call_openai_ocr(
  client: &Client,
  config: &settings::ModelProvider,
  model: &str,
  image_path: &Path,
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
            "text": "Read all text in this image. For mathematical formulas, use LaTeX format enclosed in $...$ for inline or $$...$$ for block math. Output only the text content, preserving original lines."
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

const RETRY_BASE_DELAY_MS: u64 = 500;
const RETRY_MAX_DELAY_MS: u64 = 10_000;

fn effective_retry_attempts(settings: &Settings) -> usize {
  if settings.retry_enabled {
    settings.retry_attempts as usize
  } else {
    1
  }
}

fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
  headers
    .get("retry-after")
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse::<u64>().ok())
}

fn is_retryable_status(status: StatusCode) -> bool {
  status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn is_retryable_error(error: &reqwest::Error) -> bool {
  error.is_timeout() || error.is_connect()
}

fn retry_delay_ms(attempt: usize, retry_after: Option<u64>) -> u64 {
  if let Some(seconds) = retry_after {
    return seconds.saturating_mul(1000);
  }

  let delay = RETRY_BASE_DELAY_MS.saturating_mul(2u64.saturating_pow((attempt - 1) as u32));
  delay.min(RETRY_MAX_DELAY_MS)
}

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


#[cfg(target_os = "macos")]
fn check_accessibility(open_if_needed: bool) -> bool {
    use std::process::Command;
    unsafe {
        #[link(name = "ApplicationServices", kind = "framework")]
        extern "C" {
            fn AXIsProcessTrustedWithOptions(options: *mut libc::c_void) -> bool;
        }
        
        // simple check without options first
        if AXIsProcessTrustedWithOptions(std::ptr::null_mut()) {
            return true;
        }

        if open_if_needed {
            // We open System Settings directly instead of trying to trigger the AX prompt via FFI.
            eprintln!("Accessibility not trusted, opening preferences...");
            let _ = Command::new("open")
              .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
              .output();
        }
        false
    }
}


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


fn tray_labels(lang: &str) -> (&'static str, &'static str, &'static str) {
  match lang {
    "en" => ("Show Translator", "Settings", "Quit"),
    _ => ("显示翻译器", "设置", "退出"),
  }
}

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
            let _ = window.show();
            let _ = window.set_focus();
          }
          Err(err) => eprintln!("Failed to ensure main window: {}", err),
        }
      }
      "settings" => {
        match ensure_main_window(app) {
          Ok(window) => {
            // 使用 LogicalSize 确保在 Retina 屏幕上尺寸正确
            let _ = window.set_size(tauri::LogicalSize::new(420.0, 520.0));
            let _ = window.show();
            let _ = window.set_focus();
            // 直接通过 JavaScript 设置 hash，触发前端 hashchange 事件
            let _ = window.eval("window.location.hash = '#settings'; window.dispatchEvent(new HashChangeEvent('hashchange'));");
          }
          Err(err) => eprintln!("Failed to ensure main window: {}", err),
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

fn main() {
  tauri::Builder::default()
    .plugin(tauri_plugin_global_shortcut::Builder::new().build())
    .plugin(tauri_plugin_clipboard_manager::init())
    .plugin(tauri_plugin_store::Builder::default().build())
    .plugin(tauri_plugin_shell::init())
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
        // Hide Dock icon by switching to accessory activation policy.
        let _ = app
          .handle()
          .set_activation_policy(tauri::ActivationPolicy::Accessory);
      }

      let settings = load_settings(&app.handle());
      app.manage(AppState {
        settings: RwLock::new(settings),
        explain_images: Mutex::new(HashMap::new()),
        current_explain_image_id: Mutex::new(None),
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
      fetch_models
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
