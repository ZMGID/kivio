use tauri::{
  window::Color, AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

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
 * 确保 Lens 窗口存在（不存在则创建）
 * 单 webview 三态：select 全屏 / ready 悬浮 600x72 / answering 悬浮 600x420。
 * 创建时尺寸为悬浮态默认值；后端按需要 set_size 切换。
 */
pub fn ensure_lens_window(app: &AppHandle) -> Result<WebviewWindow, String> {
  if let Some(window) = app.get_webview_window("lens") {
    return Ok(window);
  }

  let window = WebviewWindowBuilder::new(
    app,
    "lens",
    WebviewUrl::App("index.html#lens".into()),
  )
  .title("Lens")
  .inner_size(600.0, 72.0)
  .always_on_top(true)
  .visible_on_all_workspaces(true)
  .resizable(true)
  .decorations(false)
  .shadow(false)
  .transparent(true)
  // 把 WebView2 / WKWebView 的默认背景设成全透明。Windows 上 WebView2 控件
  // 在 HTML/CSS 把 html、body、#root 设为 transparent 之前会用系统主题色（白）
  // 渲染首帧，导致全屏白闪 —— 设了 (0,0,0,0) 后默认背景本身就是透明的。
  // 文档：Windows 8+ 上 webview 层的 alpha=0 被尊重；macOS 上此调用是 no-op。
  .background_color(Color(0, 0, 0, 0))
  .skip_taskbar(true)
  .visible(false)
  .build()
  .map_err(|e| e.to_string())?;

  #[cfg(target_os = "macos")]
  apply_macos_workspace_behavior(&window);

  Ok(window)
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
