//! ScreenCaptureKit 截图封装（macOS 14+）
//!
//! 取代 `screencapture` CLI。优势：
//!   - 同步 API 直接拿 `CGImage`，省去子进程冷启动（几十–几百 ms → 个位数 ms）
//!   - GPU 后端、原生 HiDPI、可排除指定窗口（截 cowork 区域时不必再 hide+sleep 自己）
//!
//! Phase 1 (PoC): 只暴露 `capture_window`。区域截图 / prewarm 在后续 Phase 加。
//!
//! 接口契约：返回 `PathBuf` (写到 temp_dir 下的 PNG 文件)，与原 `cowork::capture_window` 一致 →
//! 上层 `images_lock` / `resolve_explain_image_path` 等图片管线零改动。

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use uuid::Uuid;

use screencapturekit::{
  cg::CGRect,
  screenshot_manager::SCScreenshotManager,
  shareable_content::{SCShareableContent, SCWindow},
  stream::{configuration::SCStreamConfiguration, content_filter::SCContentFilter},
};

/// 异步预热 SCShareableContent 缓存：首次 `SCShareableContent::get()` 要查 WindowServer，
/// 实测 30–80 ms。在 cowork 进入 select 态时调一次，用户瞄准目标的几百毫秒里把这块开销摊掉，
/// 真正按下截图时直接走快路径（实测稳定值降到 < 30 ms）。
pub fn prewarm() {
  std::thread::spawn(|| {
    let started = std::time::Instant::now();
    match SCShareableContent::get() {
      Ok(_) => eprintln!("[sck] prewarm done in {}ms", started.elapsed().as_millis()),
      Err(e) => eprintln!("[sck] prewarm failed: {:?}", e),
    }
  });
}

/// 截取指定 windowID 的单窗口画面，写入 `temp_dir/cowork-<uuid>.png`，返回路径。
pub fn capture_window(window_id: u32) -> Result<PathBuf, String> {
  let started = std::time::Instant::now();

  let content = SCShareableContent::get()
    .map_err(|e| format!("SCShareableContent::get failed: {:?}", e))?;

  let windows = content.windows();
  let target = windows
    .iter()
    .find(|w| w.window_id() == window_id)
    .ok_or_else(|| format!("Window {window_id} not found in shareable content"))?;

  let frame = target.frame();
  let filter = SCContentFilter::create().with_window(target).build();
  let scale = filter.point_pixel_scale().max(1.0) as f64;

  // SCK 的 width/height 是输出物理像素；frame 是 logical points
  let pixel_w = ((frame.width * scale).round() as u32).max(1);
  let pixel_h = ((frame.height * scale).round() as u32).max(1);

  let config = SCStreamConfiguration::new()
    .with_width(pixel_w)
    .with_height(pixel_h);

  let image = SCScreenshotManager::capture_image(&filter, &config)
    .map_err(|e| format!("SCScreenshotManager::capture_image failed: {:?}", e))?;

  let path = std::env::temp_dir().join(format!("cowork-{}.png", Uuid::new_v4()));
  let path_str = path
    .to_str()
    .ok_or_else(|| "Path contains invalid UTF-8".to_string())?;

  image
    .save_png(path_str)
    .map_err(|e| format!("save_png failed: {:?}", e))?;

  let elapsed = started.elapsed();
  eprintln!(
    "[sck] capture_window id={} {}x{} took {}ms",
    window_id,
    pixel_w,
    pixel_h,
    elapsed.as_millis()
  );

  Ok(path)
}

/// 截取屏幕全局 logical 坐标 (x, y, width, height) 内的矩形区域。
///
/// `exclude_self_pid`：传 `Some(pid)` 时，从截图里排除该 PID 拥有的所有窗口（典型用法：
/// cowork webview 自己 → 这样无需先 hide+sleep 60ms 让 NSWindow.orderOut 生效再截，
/// SCK 在 GPU compositor 阶段直接抹掉这些 layer）。传 `None` 则不排除。
pub fn capture_region(
  x: f64,
  y: f64,
  width: f64,
  height: f64,
  exclude_self_pid: Option<i32>,
) -> Result<PathBuf, String> {
  if width <= 0.0 || height <= 0.0 {
    return Err(format!("Invalid region size {width}x{height}"));
  }
  let started = std::time::Instant::now();

  let content = SCShareableContent::get()
    .map_err(|e| format!("SCShareableContent::get failed: {:?}", e))?;

  // 找包含选区中心点的 display
  let center_x = x + width / 2.0;
  let center_y = y + height / 2.0;
  let displays = content.displays();
  let display = displays
    .iter()
    .find(|d| {
      let f = d.frame();
      center_x >= f.x && center_x < f.x + f.width && center_y >= f.y && center_y < f.y + f.height
    })
    .or_else(|| displays.first())
    .ok_or_else(|| "No display available".to_string())?;
  let display_frame = display.frame();

  // 构造 SCWindow 排除集：拥有 self pid 的所有窗口（cowork webview 自己），保 owned 引用避免悬空
  let windows = content.windows();
  let excluded_owned: Vec<SCWindow> = match exclude_self_pid {
    Some(pid) => windows
      .iter()
      .filter(|w| {
        w.owning_application()
          .map(|a| a.process_id() == pid)
          .unwrap_or(false)
      })
      .cloned()
      .collect(),
    None => Vec::new(),
  };
  let excluded_refs: Vec<&SCWindow> = excluded_owned.iter().collect();

  let filter = SCContentFilter::create()
    .with_display(display)
    .with_excluding_windows(&excluded_refs)
    .build();

  // SCStreamConfiguration source_rect: relative to source content (display frame origin) in logical points
  let scale = filter.point_pixel_scale().max(1.0) as f64;
  let local = CGRect::new(x - display_frame.x, y - display_frame.y, width, height);
  let pixel_w = ((width * scale).round() as u32).max(1);
  let pixel_h = ((height * scale).round() as u32).max(1);

  let config = SCStreamConfiguration::new()
    .with_source_rect(local)
    .with_width(pixel_w)
    .with_height(pixel_h);

  let image = SCScreenshotManager::capture_image(&filter, &config)
    .map_err(|e| format!("SCScreenshotManager::capture_image failed: {:?}", e))?;

  let path = std::env::temp_dir().join(format!("cowork-region-{}.png", Uuid::new_v4()));
  let path_str = path
    .to_str()
    .ok_or_else(|| "Path contains invalid UTF-8".to_string())?;
  image
    .save_png(path_str)
    .map_err(|e| format!("save_png failed: {:?}", e))?;

  let elapsed = started.elapsed();
  eprintln!(
    "[sck] capture_region rect=({},{},{}x{}) display={} excluded={} {}x{} took {}ms",
    x as i32,
    y as i32,
    width as i32,
    height as i32,
    display.display_id(),
    excluded_owned.len(),
    pixel_w,
    pixel_h,
    elapsed.as_millis()
  );

  Ok(path)
}
