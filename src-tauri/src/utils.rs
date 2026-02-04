use std::time::Duration;

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

pub fn has_chinese(text: &str) -> bool {
  text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

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

pub fn current_timestamp() -> i64 {
  let now = std::time::SystemTime::now();
  let since_epoch = now
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_else(|_| Duration::from_secs(0));
  since_epoch.as_millis() as i64
}
