use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub fn get_main_window(app: &AppHandle) -> Option<WebviewWindow> {
  app.get_webview_window("main")
}

pub fn ensure_main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
  if let Some(window) = get_main_window(app) {
    return Ok(window);
  }

  let config = app
    .config()
    .app
    .windows
    .iter()
    .find(|w| w.label == "main")
    .ok_or_else(|| "Main window config not found".to_string())?;

  WebviewWindowBuilder::from_config(app, config)
    .map_err(|e| e.to_string())?
    .build()
    .map_err(|e| e.to_string())
}

pub fn ensure_screenshot_window(app: &AppHandle) -> Result<WebviewWindow, String> {
  if let Some(window) = app.get_webview_window("screenshot") {
    return Ok(window);
  }

  WebviewWindowBuilder::new(
    app,
    "screenshot",
    WebviewUrl::App("index.html#screenshot".into()),
  )
  .title("Screenshot Translation")
  .inner_size(500.0, 400.0)
  .always_on_top(true)
  .visible_on_all_workspaces(true)
  .resizable(true)
  .build()
  .map_err(|e| e.to_string())
  .map(|window| {
    #[cfg(target_os = "macos")]
    apply_macos_workspace_behavior(&window);
    window
  })
}

pub fn ensure_capture_overlay_window(app: &AppHandle) -> Result<WebviewWindow, String> {
  if let Some(window) = app.get_webview_window("capture") {
    return Ok(window);
  }

  WebviewWindowBuilder::new(
    app,
    "capture",
    WebviewUrl::App("index.html#capture".into()),
  )
  .title("Capture")
  .fullscreen(true)
  .always_on_top(true)
  .decorations(false)
  .transparent(true)
  .skip_taskbar(true)
  .resizable(false)
  .build()
  .map_err(|e| e.to_string())
  .map(|window| {
    #[cfg(target_os = "macos")]
    apply_macos_workspace_behavior(&window);
    window
  })
}

#[cfg(target_os = "macos")]
pub fn apply_macos_workspace_behavior(window: &WebviewWindow) {
  let window_for_task = window.clone();
  let _ = window.run_on_main_thread(move || {
    let _ = window_for_task.set_visible_on_all_workspaces(true);
  });
}

#[allow(dead_code)]
#[cfg(not(target_os = "macos"))]
pub fn apply_macos_workspace_behavior(_window: &WebviewWindow) {}
