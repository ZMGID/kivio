use std::{
  fs,
  path::{Path, PathBuf},
  thread,
  time::Duration,
};

use uuid::Uuid;

#[cfg(target_os = "windows")]
use arboard::Clipboard;

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
    let previous_hash = clipboard.get_image().ok().map(image_hash);

    std::process::Command::new("explorer.exe")
      .arg("ms-screenclip:")
      .spawn()
      .map_err(|e| e.to_string())?;

    let start = std::time::Instant::now();
    loop {
      if start.elapsed() > Duration::from_secs(25) {
        return Err("Screenshot timeout".to_string());
      }

      if let Ok(image) = clipboard.get_image() {
        let hash = image_hash(&image);
        if previous_hash.map(|prev| prev != hash).unwrap_or(true) {
          let temp_path = temp_file_path("screenshot");
          write_image_to_png(&image, &temp_path)?;
          return Ok(temp_path);
        }
      }

      thread::sleep(Duration::from_millis(200));
    }
  }

  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  {
    Err("Screenshot not supported on this platform".to_string())
  }
}

pub fn cleanup_temp_file(path: &Path) {
  let _ = fs::remove_file(path);
}

fn temp_file_path(prefix: &str) -> PathBuf {
  let filename = format!("{}-{}.png", prefix, Uuid::new_v4());
  std::env::temp_dir().join(filename)
}

#[cfg(target_os = "windows")]
fn write_image_to_png(image: &arboard::ImageData, path: &Path) -> Result<(), String> {
  let width = image.width as u32;
  let height = image.height as u32;
  let mut rgba = Vec::with_capacity(image.bytes.len());

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

#[cfg(target_os = "windows")]
fn image_hash(image: &arboard::ImageData) -> u64 {
  use std::hash::{Hash, Hasher};
  let mut hasher = std::collections::hash_map::DefaultHasher::new();
  image.width.hash(&mut hasher);
  image.height.hash(&mut hasher);
  image.bytes.hash(&mut hasher);
  hasher.finish()
}
