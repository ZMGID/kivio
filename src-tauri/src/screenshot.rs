#![allow(dead_code)]

use std::{
  fs,
  path::{Path, PathBuf},
};

use uuid::Uuid;

#[cfg(target_os = "windows")]
use arboard::Clipboard;

/**
 * 截图捕获函数
 *
 * macOS: 使用系统自带的 screencapture -i 命令进行交互式区域截图
 * Windows: 使用系统截图工具（ms-screenclip:）配合剪贴板轮询获取截图
 */
pub fn capture_screenshot() -> Result<PathBuf, String> {
  #[cfg(target_os = "macos")]
  {
    let temp_path = temp_file_path("screenshot");
    let status = std::process::Command::new("screencapture")
      .arg("-i")
      .arg(&temp_path)
      .status()
      .map_err(|e| e.to_string())?;

    if !status.success() || !temp_path.exists() {
      return Err("Screenshot cancelled".to_string());
    }

    return Ok(temp_path);
  }

  #[cfg(target_os = "windows")]
  {
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    // 记录当前剪贴板图片的哈希值，用于判断是否有新截图
    let previous_hash = clipboard
      .get_image()
      .ok()
      .map(|image| image_hash(&image));

    // 打开 Windows 系统截图工具
    std::process::Command::new("explorer.exe")
      .arg("ms-screenclip:")
      .spawn()
      .map_err(|e| e.to_string())?;

    let start = std::time::Instant::now();
    loop {
      // 超时 25 秒则取消
      if start.elapsed() > std::time::Duration::from_secs(25) {
        return Err("Screenshot timeout".to_string());
      }

      if let Ok(image) = clipboard.get_image() {
        let hash = image_hash(&image);
        // 如果剪贴板图片发生变化，则保存为新截图
        if previous_hash.map(|prev| prev != hash).unwrap_or(true) {
          let temp_path = temp_file_path("screenshot");
          write_image_to_png(&image, &temp_path)?;
          return Ok(temp_path);
        }
      }

      std::thread::sleep(std::time::Duration::from_millis(200));
    }
  }

  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  {
    Err("Screenshot not supported on this platform".to_string())
  }
}

/**
 * 清理临时截图文件
 */
pub fn cleanup_temp_file(path: &Path) {
  let _ = fs::remove_file(path);
}

/**
 * 生成临时文件路径
 */
fn temp_file_path(prefix: &str) -> PathBuf {
  let filename = format!("{}-{}.png", prefix, Uuid::new_v4());
  std::env::temp_dir().join(filename)
}

/**
 * Windows: 将剪贴板图片数据写入 PNG 文件
 * 注意：剪贴板数据为 BGRA 格式，需要转换为 RGBA
 */
#[cfg(target_os = "windows")]
fn write_image_to_png(image: &arboard::ImageData, path: &Path) -> Result<(), String> {
  let width = image.width as u32;
  let height = image.height as u32;
  let mut rgba = Vec::with_capacity(image.bytes.len());

  // BGRA -> RGBA 转换
  for chunk in image.bytes.chunks(4) {
    if chunk.len() == 4 {
      rgba.push(chunk[2]);
      rgba.push(chunk[1]);
      rgba.push(chunk[0]);
      rgba.push(chunk[3]);
    }
  }

  let buffer = image::RgbaImage::from_raw(width, height, rgba)
    .ok_or_else(|| "Failed to build image buffer".to_string())?;
  buffer.save(path).map_err(|e| e.to_string())
}

/**
 * Windows: 计算图片数据的哈希值
 * 用于检测剪贴板内容是否发生变化
 */
#[cfg(target_os = "windows")]
fn image_hash(image: &arboard::ImageData) -> u64 {
  use std::hash::{Hash, Hasher};
  let mut hasher = std::collections::hash_map::DefaultHasher::new();
  image.width.hash(&mut hasher);
  image.height.hash(&mut hasher);
  image.bytes.hash(&mut hasher);
  hasher.finish()
}
