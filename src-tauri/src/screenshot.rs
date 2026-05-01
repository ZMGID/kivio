use std::{fs, path::Path, time::{Duration, SystemTime}};

/**
 * 清理临时截图文件
 */
pub fn cleanup_temp_file(path: &Path) {
  let _ = fs::remove_file(path);
}

/// 启动时清理 temp_dir 下遗留的截图文件。
///
/// 我们在三个地方写 PNG 到 temp_dir：
///   - `lens-<uuid>.png`（macOS SCK 整窗截图）
///   - `lens-region-<uuid>.png`（macOS SCK 区域截图）
///   - `screenshot-<uuid>.png`（Windows xcap）
///
/// 正常路径里 lens_close 会删除 active image_id 对应的文件，但以下情况会留 orphan：
///   - 应用被强杀 / 崩溃前来不及清
///   - 历史记录引用的旧 image_id（应用重启后历史里指针还在但文件不再被 active 引用）
///   - 之前版本（v2.2 及更早）的旧文件
///
/// 这里只删 24 小时之前的文件，避免误删可能正在被另一个 Kivio 实例使用的新文件。
pub fn cleanup_orphan_temp_files() {
  const PREFIXES: &[&str] = &["lens-", "lens-region-", "screenshot-"];
  const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

  let temp_dir = std::env::temp_dir();
  let entries = match fs::read_dir(&temp_dir) {
    Ok(e) => e,
    Err(e) => {
      eprintln!("[orphan-cleanup] read_dir({:?}) err: {}", temp_dir, e);
      return;
    }
  };

  let now = SystemTime::now();
  let mut removed = 0u32;
  let mut bytes_freed = 0u64;
  for entry in entries.flatten() {
    let path = entry.path();
    let name = match path.file_name().and_then(|n| n.to_str()) {
      Some(n) => n,
      None => continue,
    };
    if !PREFIXES.iter().any(|p| name.starts_with(p)) || !name.ends_with(".png") {
      continue;
    }
    let metadata = match entry.metadata() {
      Ok(m) => m,
      Err(_) => continue,
    };
    let modified = match metadata.modified() {
      Ok(t) => t,
      Err(_) => continue,
    };
    let age = match now.duration_since(modified) {
      Ok(a) => a,
      Err(_) => continue, // 文件 mtime 在未来 → 跳过
    };
    if age < MAX_AGE {
      continue;
    }
    let size = metadata.len();
    if fs::remove_file(&path).is_ok() {
      removed += 1;
      bytes_freed += size;
    }
  }
  if removed > 0 {
    eprintln!(
      "[orphan-cleanup] removed {} stale screenshot file(s), freed {} KB",
      removed,
      bytes_freed / 1024
    );
  }
}
