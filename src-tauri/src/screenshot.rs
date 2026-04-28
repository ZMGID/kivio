use std::{fs, path::Path};

/**
 * 清理临时截图文件
 */
pub fn cleanup_temp_file(path: &Path) {
  let _ = fs::remove_file(path);
}
