use std::time::Duration;

/**
 * 解析目标语言
 * 当设置为 "auto" 时，根据文本内容自动判断：
 * - 如果文本包含中文，则目标语言为英文
 * - 否则目标语言为中文
 */
pub fn resolve_target_lang(target: &str, text: &str) -> String {
  if target == "auto" {
    if has_chinese(text) {
      "en".to_string()
    } else {
      "zh".to_string()
    }
  } else {
    target.to_string()
  }
}

/**
 * 判断文本中是否包含中文字符
 */
pub fn has_chinese(text: &str) -> bool {
  text.chars().any(|c| ('\u{4e00}'..'\u{9fff}').contains(&c))
}

/**
 * 获取语言代码对应的显示名称
 */
pub fn language_name(code: &str) -> &'static str {
  match code {
    "zh" | "zh-Hans" => "Simplified Chinese",
    "en" => "English",
    "ja" => "Japanese",
    "ko" => "Korean",
    "fr" => "French",
    "de" => "German",
    _ => "English",
  }
}

/**
 * 获取当前时间戳（毫秒）
 */
pub fn current_timestamp() -> i64 {
  let now = std::time::SystemTime::now();
  let since_epoch = now
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_else(|_| Duration::from_secs(0));
  since_epoch.as_millis() as i64
}
