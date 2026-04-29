// Lens 模式：枚举屏幕上可见应用窗口（hover 高亮 + 标签）+ 整窗截图。
// macOS：CGWindowListCopyWindowInfo（Quartz）；Windows MVP：返回空列表，整窗截图返回 Err。

use serde::{Deserialize, Serialize};

/// 屏幕上一个应用窗口的元信息。坐标为全局逻辑坐标（macOS Quartz：原点左上，含 menubar，跨 monitor 全局）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
  pub id: u32,
  pub owner: String,
  pub title: String,
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

#[cfg(target_os = "macos")]
pub fn list_windows() -> Vec<WindowInfo> {
  use core_foundation::array::{CFArray, CFArrayRef};
  use core_foundation::base::{CFType, TCFType};
  use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
  use core_graphics::window::{
    kCGNullWindowID, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly,
    CGWindowListCopyWindowInfo,
  };

  let info_ref: CFArrayRef = unsafe {
    CGWindowListCopyWindowInfo(
      kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
      kCGNullWindowID,
    )
  };
  if info_ref.is_null() {
    return Vec::new();
  }
  // 数组元素类型为 untyped CFType；每个元素本身是一个 CFDictionary。
  let array: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(info_ref) };

  let total = array.len();
  eprintln!("[lens] CGWindowList returned {} entries", total);

  let mut out = Vec::new();
  let mut filtered_summary: Vec<String> = Vec::new();
  for item in array.iter() {
    let dict_ref = item.as_CFTypeRef() as CFDictionaryRef;
    if dict_ref.is_null() {
      continue;
    }
    let dict: CFDictionary = unsafe { CFDictionary::wrap_under_get_rule(dict_ref) };

    let layer = read_dict_i64(&dict, "kCGWindowLayer").unwrap_or(-1);
    let alpha = read_dict_f64(&dict, "kCGWindowAlpha").unwrap_or(1.0);
    let id = read_dict_i64(&dict, "kCGWindowNumber").unwrap_or(0);
    let owner = read_dict_string(&dict, "kCGWindowOwnerName").unwrap_or_default();
    let title = read_dict_string(&dict, "kCGWindowName").unwrap_or_default();

    let bounds_dict = read_dict_subdict(&dict, "kCGWindowBounds");
    let (bx, by, bw, bh) = if let Some(b) = bounds_dict {
      (
        read_dict_f64(&b, "X").unwrap_or(0.0),
        read_dict_f64(&b, "Y").unwrap_or(0.0),
        read_dict_f64(&b, "Width").unwrap_or(0.0),
        read_dict_f64(&b, "Height").unwrap_or(0.0),
      )
    } else {
      (0.0, 0.0, 0.0, 0.0)
    };

    let mut reason: Option<&str> = None;
    if id <= 0 {
      reason = Some("no-id");
    } else if layer != 0 {
      reason = Some("layer!=0");
    } else if alpha < 0.05 {
      reason = Some("alpha~0");
    } else if owner == "KeyLingo" || owner == "keylingo" {
      reason = Some("self");
    } else if bw < 60.0 || bh < 40.0 {
      reason = Some("too-small");
    }

    filtered_summary.push(format!(
      "  layer={} alpha={:.2} id={} {}x{} owner={:?} title={:?} -> {}",
      layer,
      alpha,
      id,
      bw as i32,
      bh as i32,
      owner,
      title,
      reason.unwrap_or("KEEP"),
    ));

    if reason.is_some() {
      continue;
    }
    out.push(WindowInfo {
      id: id as u32,
      owner,
      title,
      x: bx,
      y: by,
      width: bw,
      height: bh,
    });
  }
  // 打印每个候选窗口的最终归类，方便调试
  for line in &filtered_summary {
    eprintln!("{}", line);
  }
  eprintln!("[lens] kept {} windows", out.len());
  out
}

#[cfg(target_os = "macos")]
fn read_dict_value(
  dict: &core_foundation::dictionary::CFDictionary,
  key: &str,
) -> Option<core_foundation::base::CFType> {
  use core_foundation::base::{CFType, TCFType};
  use core_foundation::string::CFString;
  let cfk = CFString::new(key);
  unsafe {
    let raw = dict.find(cfk.as_CFTypeRef() as *const _);
    raw.map(|r| CFType::wrap_under_get_rule(*r))
  }
}

#[cfg(target_os = "macos")]
fn read_dict_i64(dict: &core_foundation::dictionary::CFDictionary, key: &str) -> Option<i64> {
  use core_foundation::number::CFNumber;
  read_dict_value(dict, key)
    .and_then(|v| v.downcast::<CFNumber>())
    .and_then(|n| n.to_i64())
}

#[cfg(target_os = "macos")]
fn read_dict_f64(dict: &core_foundation::dictionary::CFDictionary, key: &str) -> Option<f64> {
  use core_foundation::number::CFNumber;
  read_dict_value(dict, key)
    .and_then(|v| v.downcast::<CFNumber>())
    .and_then(|n| n.to_f64())
}

#[cfg(target_os = "macos")]
fn read_dict_string(dict: &core_foundation::dictionary::CFDictionary, key: &str) -> Option<String> {
  use core_foundation::string::CFString;
  read_dict_value(dict, key)
    .and_then(|v| v.downcast::<CFString>())
    .map(|s| s.to_string())
}

#[cfg(target_os = "macos")]
fn read_dict_subdict(
  dict: &core_foundation::dictionary::CFDictionary,
  key: &str,
) -> Option<core_foundation::dictionary::CFDictionary> {
  use core_foundation::base::TCFType;
  use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
  let v = read_dict_value(dict, key)?;
  let r = v.as_CFTypeRef() as CFDictionaryRef;
  if r.is_null() {
    return None;
  }
  Some(unsafe { CFDictionary::wrap_under_get_rule(r) })
}

#[cfg(not(target_os = "macos"))]
pub fn list_windows() -> Vec<WindowInfo> {
  Vec::new()
}

/// 单窗口截图（macOS 14+）：走 ScreenCaptureKit (SCScreenshotManager)。
/// 取代旧的 `screencapture -l` CLI 调用：消除几十–几百 ms 子进程冷启动 + 消除屏幕白闪。
#[cfg(target_os = "macos")]
pub fn capture_window(window_id: u32) -> Result<std::path::PathBuf, String> {
  crate::sck::capture_window(window_id)
}

#[cfg(not(target_os = "macos"))]
pub fn capture_window(_window_id: u32) -> Result<std::path::PathBuf, String> {
  Err("Window capture not supported on this platform".to_string())
}
