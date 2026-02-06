use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreBuilder;

const SETTINGS_STORE: &str = "settings.json";
const KEYRING_SERVICE: &str = "com.zmair.keylingo";

fn provider_credential_name(provider_id: &str) -> String {
  format!("provider:{provider_id}")
}

fn save_provider_api_key(provider_id: &str, api_key: &str) -> Result<(), String> {
  let entry = keyring::Entry::new(KEYRING_SERVICE, &provider_credential_name(provider_id))
    .map_err(|e| e.to_string())?;

  if api_key.trim().is_empty() {
    let _ = entry.delete_credential();
    return Ok(());
  }

  entry.set_password(api_key).map_err(|e| e.to_string())
}

fn load_provider_api_key(provider_id: &str) -> Option<String> {
  let entry = keyring::Entry::new(KEYRING_SERVICE, &provider_credential_name(provider_id)).ok()?;
  entry.get_password().ok()
}

fn persist_provider_api_keys(settings: &Settings) -> Result<(), String> {
  for provider in &settings.providers {
    save_provider_api_key(&provider.id, &provider.api_key)?;
  }
  Ok(())
}

fn hydrate_provider_api_keys(settings: &mut Settings) {
  for provider in &mut settings.providers {
    let inline_key = provider.api_key.trim().to_string();
    if !inline_key.is_empty() {
      if let Err(err) = save_provider_api_key(&provider.id, &inline_key) {
        eprintln!(
          "Failed to migrate API key for provider {} to keyring: {}",
          provider.id, err
        );
      }
      provider.api_key = inline_key;
      continue;
    }

    provider.api_key = load_provider_api_key(&provider.id).unwrap_or_default();
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenAIConfig {
  #[serde(default)]
  pub api_key: String,
  #[serde(default = "default_openai_base_url")]
  pub base_url: String,
  #[serde(default = "default_openai_model")]
  pub model: String,
}

impl Default for OpenAIConfig {
  fn default() -> Self {
    Self {
      api_key: "".to_string(),
      base_url: "https://api.openai.com/v1".to_string(),
      model: "gpt-4o".to_string(),
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProvider {
  pub id: String,
  pub name: String,
  pub api_key: String,
  pub base_url: String,
  #[serde(default)]
  pub available_models: Vec<String>,
  #[serde(default)]
  pub enabled_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenshotTranslationConfig {
  #[serde(default = "default_true")]
  pub enabled: bool,
  #[serde(default = "default_screenshot_translation_hotkey")]
  pub hotkey: String,
  #[serde(default)]
  pub provider_id: String,
  #[serde(default = "default_openai_model")]
  pub model: String,
  #[serde(default = "default_false")]
  pub direct_translate: bool,
  #[serde(default)]
  pub prompt: Option<String>,
  // Legacy field for migration
  #[serde(skip_serializing_if = "Option::is_none")]
  pub openai: Option<OpenAIConfig>,
}

impl Default for ScreenshotTranslationConfig {
  fn default() -> Self {
    Self {
      enabled: true,
      hotkey: "CommandOrControl+Shift+A".to_string(),
      provider_id: "default-ocr".to_string(),
      model: "gpt-4o".to_string(),
      direct_translate: false,
      prompt: None,
      openai: None,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPrompts {
  pub system_prompt: Option<String>,
  pub summary_prompt: Option<String>,
  pub question_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotExplainModel {
  pub provider: String,
  pub api_key: String,
  pub base_url: String,
  pub model_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenshotExplainConfig {
  #[serde(default = "default_true")]
  pub enabled: bool,
  #[serde(default = "default_screenshot_explain_hotkey")]
  pub hotkey: String,
  #[serde(default)]
  pub provider_id: String,
  #[serde(default = "default_openai_model")]
  pub model: String,
  #[serde(default = "default_language_zh")]
  pub default_language: String,
  #[serde(default)]
  pub stream_enabled: bool,
  #[serde(default)]
  pub custom_prompts: Option<CustomPrompts>,
  // Legacy field for migration
  #[serde(skip_serializing_if = "Option::is_none")]
  pub model_legacy: Option<ScreenshotExplainModel>,
}

impl Default for ScreenshotExplainConfig {
  fn default() -> Self {
    Self {
      enabled: true,
      hotkey: "CommandOrControl+Shift+E".to_string(),
      provider_id: "default-explain".to_string(),
      model: "gpt-4o".to_string(),
      default_language: "zh".to_string(),
      stream_enabled: false,
      custom_prompts: None,
      model_legacy: None,
    }
  }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainMessage {
  pub role: String,
  pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainHistoryRecord {
  pub id: String,
  pub timestamp: i64,
  pub messages: Vec<ExplainMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
  #[serde(default = "default_hotkey")]
  pub hotkey: String,
  #[serde(default = "default_theme")]
  pub theme: String,
  #[serde(default = "default_target_lang")]
  pub target_lang: String,
  #[serde(default = "default_source")]
  pub source: String,
  #[serde(default = "default_true")]
  pub auto_paste: bool,
  #[serde(default = "default_false")]
  pub launch_at_startup: bool,
  #[serde(default)]
  pub translator_provider_id: String,
  #[serde(default = "default_openai_model")]
  pub translator_model: String,
  #[serde(default)]
  pub translator_prompt: Option<String>,
  #[serde(default)]
  pub providers: Vec<ModelProvider>,
  #[serde(default)]
  pub screenshot_translation: ScreenshotTranslationConfig,
  #[serde(default)]
  pub screenshot_explain: ScreenshotExplainConfig,
  #[serde(default)]
  pub explain_history: Vec<ExplainHistoryRecord>,
  #[serde(default = "default_settings_language")]
  pub settings_language: Option<String>,
  #[serde(default = "default_retry_enabled")]
  pub retry_enabled: bool,
  #[serde(default = "default_retry_attempts")]
  pub retry_attempts: u8,
  // Legacy field for migration
  #[serde(skip_serializing_if = "Option::is_none")]
  pub openai: Option<OpenAIConfig>,
}

impl Settings {
  pub fn get_provider(&self, id: &str) -> Option<&ModelProvider> {
    self.providers.iter().find(|p| p.id == id)
  }
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      hotkey: "CommandOrControl+Alt+T".to_string(),
      theme: "system".to_string(),
      target_lang: "auto".to_string(),
      source: "openai".to_string(),
      auto_paste: true,
      launch_at_startup: false,
      translator_provider_id: "default-translator".to_string(),
      translator_model: "gpt-4o".to_string(),
      translator_prompt: None,
      providers: vec![],
      screenshot_translation: ScreenshotTranslationConfig::default(),
      screenshot_explain: ScreenshotExplainConfig::default(),
      explain_history: vec![],
      settings_language: Some("zh".to_string()),
      retry_enabled: default_retry_enabled(),
      retry_attempts: default_retry_attempts(),
      openai: None,
    }
  }
}

pub fn sanitize_settings(mut settings: Settings) -> Settings {
  // 1. Migration from old settings
  if settings.providers.is_empty() {
    // Migrate Translator provider
    if let Some(old_openai) = settings.openai.take() {
      settings.providers.push(ModelProvider {
        id: "default-translator".to_string(),
        name: "OpenAI (Translator)".to_string(),
        api_key: old_openai.api_key,
        base_url: old_openai.base_url,
        available_models: vec![],
        enabled_models: vec![old_openai.model.clone()],
      });
      settings.translator_provider_id = "default-translator".to_string();
      settings.translator_model = old_openai.model;
    }

    // Migrate OCR provider
    if let Some(old_ocr) = settings.screenshot_translation.openai.take() {
        settings.providers.push(ModelProvider {
            id: "default-ocr".to_string(),
            name: "OpenAI (OCR)".to_string(),
            api_key: old_ocr.api_key,
            base_url: old_ocr.base_url,
            available_models: vec![],
            enabled_models: vec![old_ocr.model.clone()],
        });
        settings.screenshot_translation.provider_id = "default-ocr".to_string();
        settings.screenshot_translation.model = old_ocr.model;
    }

    // Migrate Explain provider
    if let Some(old_explain) = settings.screenshot_explain.model_legacy.take() {
        settings.providers.push(ModelProvider {
            id: "default-explain".to_string(),
            name: "OpenAI (Explain)".to_string(),
            api_key: old_explain.api_key,
            base_url: old_explain.base_url,
            available_models: vec![],
            enabled_models: vec![old_explain.model_name.clone()],
        });
        settings.screenshot_explain.provider_id = "default-explain".to_string();
        settings.screenshot_explain.model = old_explain.model_name;
    }
  }

  // 2. Ensure defaults for empty fields
  if settings.translator_model.is_empty() {
      settings.translator_model = "gpt-4o".to_string();
  }
  if settings.screenshot_translation.model.is_empty() {
      settings.screenshot_translation.model = "gpt-4o".to_string();
  }
  if settings.screenshot_explain.model.is_empty() {
      settings.screenshot_explain.model = "gpt-4o".to_string();
  }

  if settings.translator_provider_id.is_empty() && !settings.providers.is_empty() {
      settings.translator_provider_id = settings.providers[0].id.clone();
  }
  if settings.screenshot_translation.provider_id.is_empty() && !settings.providers.is_empty() {
      settings.screenshot_translation.provider_id = settings.providers[0].id.clone();
  }
  if settings.screenshot_explain.provider_id.is_empty() && !settings.providers.is_empty() {
      settings.screenshot_explain.provider_id = settings.providers[0].id.clone();
  }

  let provider_exists = |id: &str| settings.providers.iter().any(|p| p.id == id);
  if settings.providers.is_empty() {
      settings.translator_provider_id.clear();
      settings.screenshot_translation.provider_id.clear();
      settings.screenshot_explain.provider_id.clear();
  } else {
      if !provider_exists(&settings.translator_provider_id) {
          let first = &settings.providers[0];
          settings.translator_provider_id = first.id.clone();
          if let Some(model) = first.enabled_models.first() {
              settings.translator_model = model.clone();
          }
      }
      if !provider_exists(&settings.screenshot_translation.provider_id) {
          let first = &settings.providers[0];
          settings.screenshot_translation.provider_id = first.id.clone();
          if let Some(model) = first.enabled_models.first() {
              settings.screenshot_translation.model = model.clone();
          }
      }
      if !provider_exists(&settings.screenshot_explain.provider_id) {
          let first = &settings.providers[0];
          settings.screenshot_explain.provider_id = first.id.clone();
          if let Some(model) = first.enabled_models.first() {
              settings.screenshot_explain.model = model.clone();
          }
      }
  }

  // 3. Ensure current models are in enabled_models
  for provider in &mut settings.providers {
      if provider.enabled_models.is_empty() {
          // If a feature is using this provider, add its model to enabled_models
          if settings.translator_provider_id == provider.id {
              provider.enabled_models.push(settings.translator_model.clone());
          }
          if settings.screenshot_translation.provider_id == provider.id {
              if !provider.enabled_models.contains(&settings.screenshot_translation.model) {
                  provider.enabled_models.push(settings.screenshot_translation.model.clone());
              }
          }
          if settings.screenshot_explain.provider_id == provider.id {
              if !provider.enabled_models.contains(&settings.screenshot_explain.model) {
                  provider.enabled_models.push(settings.screenshot_explain.model.clone());
              }
          }
          // Default fallback if still empty
          if provider.enabled_models.is_empty() {
              provider.enabled_models.push("gpt-4o".to_string());
          }
      }
  }

  // 4. Normalize hotkey strings
  settings.hotkey = normalize_hotkey(&settings.hotkey);
  settings.screenshot_translation.hotkey =
    normalize_hotkey(&settings.screenshot_translation.hotkey);
  settings.screenshot_explain.hotkey = normalize_hotkey(&settings.screenshot_explain.hotkey);

  settings.translator_prompt = normalize_optional_prompt(settings.translator_prompt.take());
  settings.screenshot_translation.prompt =
    normalize_optional_prompt(settings.screenshot_translation.prompt.take());
  if let Some(prompts) = &mut settings.screenshot_explain.custom_prompts {
    if sanitize_custom_prompts(prompts) {
      settings.screenshot_explain.custom_prompts = None;
    }
  }

  // 5. Ensure essential fields are not empty
  if settings.hotkey.is_empty() {
    settings.hotkey = "CommandOrControl+Alt+T".to_string();
  }
  if settings.screenshot_translation.hotkey.is_empty() {
    settings.screenshot_translation.hotkey = "CommandOrControl+Shift+A".to_string();
  }
  if settings.screenshot_explain.hotkey.is_empty() {
    settings.screenshot_explain.hotkey = "CommandOrControl+Shift+E".to_string();
  }

  settings.retry_attempts = clamp_retry_attempts(settings.retry_attempts);

  settings
}

pub fn persist_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
  persist_provider_api_keys(settings)?;

  let mut persisted_settings = settings.clone();
  for provider in &mut persisted_settings.providers {
    provider.api_key.clear();
  }

  let store = StoreBuilder::new(app, SETTINGS_STORE)
    .build()
    .map_err(|e| e.to_string())?;
  store.set(
    "settings".to_string(),
    serde_json::to_value(persisted_settings).map_err(|e| e.to_string())?,
  );
  store.save().map_err(|e| e.to_string())
}

pub fn load_settings(app: &AppHandle) -> Settings {
  let store = StoreBuilder::new(app, SETTINGS_STORE).build();
  let settings = match store {
    Ok(store) => store
      .get("settings")
      .and_then(|value| serde_json::from_value(value).ok())
      .unwrap_or_default(),
    Err(_) => Settings::default(),
  };
  let mut sanitized = sanitize_settings(settings);
  hydrate_provider_api_keys(&mut sanitized);
  sanitized
}

pub fn default_system_prompt(language: &str) -> String {
  if language == "zh" {
    "你是一个图片分析助手。请用自然流畅的语言回答，不要使用小标题、序号或分点列举。如果遇到数学公式，请使用 LaTeX 格式（如 $...$ 或 $$...$$）以确保正确渲染。\n\n".to_string()
  } else {
    "You are an image analysis assistant. Please respond naturally without headings, bullet points, or numbered lists. If you encounter mathematical formulas, please use LaTeX format (e.g., $...$ or $$...$$) to ensure they are rendered correctly.\n\n"
      .to_string()
  }
}

pub fn default_summary_prompt(language: &str) -> String {
  if language == "zh" {
    "你是一个图片分析助手。请简洁地总结这张图片的主要内容，不要使用小标题、序号或分点列举。如果遇到数学公式，请使用 LaTeX 格式（如 $...$ 或 $$...$$）。\n\n要求：\n- 用1-3句话概括图片核心内容\n- 语言自然流畅，像在和朋友描述\n- 突出最重要的信息\n- 不要使用\"图片显示...\"这样的开头\n\n请用中文回复。"
      .to_string()
  } else {
    "You are an image analysis assistant. Please provide a concise summary of this image's main content without using headings, bullet points, or numbered lists. If you encounter mathematical formulas, please use LaTeX format (e.g., $...$ or $$...$$).\n\nRequirements:\n- Summarize in 1-3 natural sentences\n- Write conversationally as if describing to a friend\n- Highlight the most important information\n- Don't start with \"The image shows...\"\n\nPlease respond in English."
      .to_string()
  }
}

pub fn default_question_prompt(language: &str) -> String {
  if language == "zh" {
    "你是一个图片分析助手。用户正在询问关于这张图片的问题。如果回答中包含数学公式，请务必使用 LaTeX 格式（如 $...$ 或 $$...$$）。\n\n要求：\n- 直接回答问题，不要使用小标题或分点列举\n- 语言自然、简洁\n- 基于图片内容回答\n- 如果问题与图片无关，礼貌地引导回到图片内容\n\n请用中文回复。"
      .to_string()
  } else {
    "You are an image analysis assistant. The user is asking a question about this image. If your answer contains mathematical formulas, please use LaTeX format (e.g., $...$ or $$...$$).\n\nRequirements:\n- Answer directly without headings or bullet points\n- Be natural and concise\n- Base your answer on the image content\n- If the question is unrelated to the image, politely guide back\n\nPlease respond in English."
      .to_string()
  }
}

fn default_true() -> bool {
  true
}

fn default_false() -> bool {
  false
}

fn default_hotkey() -> String {
  "CommandOrControl+Alt+T".to_string()
}

fn default_screenshot_translation_hotkey() -> String {
  "CommandOrControl+Shift+A".to_string()
}

fn default_screenshot_explain_hotkey() -> String {
  "CommandOrControl+Shift+E".to_string()
}

fn default_theme() -> String {
  "system".to_string()
}

fn default_target_lang() -> String {
  "auto".to_string()
}

fn default_source() -> String {
  "openai".to_string()
}

fn default_openai_base_url() -> String {
  "https://api.openai.com/v1".to_string()
}

fn default_openai_model() -> String {
  "gpt-4o".to_string()
}

fn default_language_zh() -> String {
  "zh".to_string()
}

fn default_settings_language() -> Option<String> {
  Some("zh".to_string())
}

fn default_retry_attempts() -> u8 {
  3
}

fn default_retry_enabled() -> bool {
  true
}

fn clamp_retry_attempts(value: u8) -> u8 {
  value.clamp(1, 5)
}

fn normalize_optional_prompt(value: Option<String>) -> Option<String> {
  value.and_then(|v| {
    let trimmed = v.trim();
    if trimmed.is_empty() {
      None
    } else {
      Some(trimmed.to_string())
    }
  })
}

fn sanitize_custom_prompts(prompts: &mut CustomPrompts) -> bool {
  prompts.system_prompt = normalize_optional_prompt(prompts.system_prompt.take());
  prompts.summary_prompt = normalize_optional_prompt(prompts.summary_prompt.take());
  prompts.question_prompt = normalize_optional_prompt(prompts.question_prompt.take());
  prompts.system_prompt.is_none()
    && prompts.summary_prompt.is_none()
    && prompts.question_prompt.is_none()
}

fn normalize_hotkey(value: &str) -> String {
  value
    .split('+')
    .map(|part| part.trim())
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("+")
}
