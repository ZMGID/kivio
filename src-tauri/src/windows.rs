use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/**
 * 获取主窗口
 */
pub fn get_main_window(app: &AppHandle) -> Option<WebviewWindow> {
  app.get_webview_window("main")
}

/**
 * 确保主窗口存在（不存在则创建）
 * 从 tauri.conf.json 中读取主窗口配置进行创建
 */
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

/**
 * 确保截图翻译结果窗口存在（不存在则创建）
 * 风格与截图解释窗口对齐：无系统装饰、透明背景、自定义圆角；
 * 首次创建时定位到光标附近的显示器内。
 */
pub fn ensure_screenshot_window(app: &AppHandle) -> Result<WebviewWindow, String> {
  if let Some(window) = app.get_webview_window("screenshot") {
    return Ok(window);
  }

  let window = WebviewWindowBuilder::new(
    app,
    "screenshot",
    WebviewUrl::App("index.html#screenshot".into()),
  )
  .title("Screenshot Translation")
  .inner_size(460.0, 360.0)
  .always_on_top(true)
  .visible_on_all_workspaces(true)
  .resizable(true)
  .decorations(false)
  .shadow(false)
  .transparent(true)
  .visible(false)
  .build()
  .map_err(|e| e.to_string())?;

  #[cfg(target_os = "macos")]
  apply_macos_workspace_behavior(&window);

  // 首次创建：定位到光标附近，clamp 到光标所在显示器。
  if let Ok(cursor) = app.cursor_position() {
    let scale = window.scale_factor().unwrap_or(1.0);
    let win_w = (460.0 * scale) as i32;
    let win_h = (360.0 * scale) as i32;
    let mut tx = cursor.x as i32 - win_w / 2;
    let mut ty = cursor.y as i32 + (24.0 * scale) as i32;

    if let Ok(monitors) = app.available_monitors() {
      for monitor in monitors {
        let mp = monitor.position();
        let ms = monitor.size();
        let mw = ms.width as i32;
        let mh = ms.height as i32;
        if (cursor.x as i32) >= mp.x
          && (cursor.x as i32) < mp.x + mw
          && (cursor.y as i32) >= mp.y
          && (cursor.y as i32) < mp.y + mh
        {
          tx = tx.max(mp.x).min(mp.x + mw - win_w);
          ty = ty.max(mp.y).min(mp.y + mh - win_h);
          break;
        }
      }
    }

    let _ = window.set_position(PhysicalPosition::new(tx, ty));
  }

  Ok(window)
}

/**
 * 确保截图区域选择覆盖层窗口存在（不存在则创建）
 * 全屏透明窗口，用于 Windows 平台的区域选择
 */
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
  .always_on_top(true)
  .decorations(false)
  .transparent(true)
  .skip_taskbar(true)
  .resizable(false)
  .visible(false)
  .build()
  .map_err(|e| e.to_string())
  .map(|window| {
    #[cfg(target_os = "macos")]
    apply_macos_workspace_behavior(&window);
    window
  })
}

/**
 * macOS 平台特有：设置窗口在所有工作区可见
 * 确保截图窗口可以跨越桌面空间显示
 */
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
