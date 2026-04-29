use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreBuilder;

// 设置存储文件名
const SETTINGS_STORE: &str = "settings.json";
// 系统钥匙串服务名（用于安全存储 API Key）
const KEYRING_SERVICE: &str = "com.zmair.keylingo";

/**
 * 生成提供商 API Key 在钥匙串中的条目名称
 */
fn provider_credential_name(provider_id: &str) -> String {
  format!("provider:{provider_id}")
}

/**
 * 一次性读取旧版 keyring 中的 API Key（仅用于升级迁移）
 * v2.3.x 及之前：API Key 存在系统钥匙串，settings.json 中 apiKey 字段留空。
 * 从 v2.4 起：API Key 直接存 settings.json，钥匙串不再写入。
 * 此函数仅在 settings.json 中没有 key 时用一次，迁移完成后旧条目可被清理。
 */
fn legacy_load_keyring_api_key(provider_id: &str) -> Option<String> {
  let entry =
    keyring::Entry::new(KEYRING_SERVICE, &provider_credential_name(provider_id)).ok()?;
  let raw = entry.get_password().ok()?;
  // v2.3.x 中 keyring 只存单 key（纯字符串）
  let trimmed = raw.trim().to_string();
  if trimmed.is_empty() { None } else { Some(trimmed) }
}

/**
 * 删除旧版 keyring 中的 API Key 条目（迁移完成后清理）
 */
fn legacy_clear_keyring_api_key(provider_id: &str) {
  if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, &provider_credential_name(provider_id)) {
    let _ = entry.delete_credential();
  }
}

/**
 * 从旧版 keyring 一次性迁移 API Key 到 settings.api_keys
 * 仅在 settings.json 中没有 key 时执行（保护用户不丢 key）
 * 迁移成功后立即清理 keyring 旧条目
 *
 * 幂等：settings.legacy_keyring_migrated == true 时直接跳过，
 * 防止用户在 v2.3.x ↔ v2.4 之间反复切换时每次启动都抹掉 keyring。
 * 标记会随用户下次保存设置写盘；即使没保存就退出，下次再跑也是 no-op（keyring 已被清）。
 */
fn migrate_legacy_keyring_keys(settings: &mut Settings) {
  if settings.legacy_keyring_migrated {
    return;
  }
  for provider in &mut settings.providers {
    if !provider.api_keys.is_empty() {
      // settings.json 已有 key，无需迁移；顺手清掉钥匙串里的残留
      legacy_clear_keyring_api_key(&provider.id);
      continue;
    }
    if let Some(legacy_key) = legacy_load_keyring_api_key(&provider.id) {
      provider.api_keys.push(legacy_key);
      legacy_clear_keyring_api_key(&provider.id);
      eprintln!(
        "Migrated legacy keyring API key for provider {} into settings.json",
        provider.id
      );
    }
  }
  settings.legacy_keyring_migrated = true;
}

// ========== 数据结构定义 ==========

/**
 * 旧版 OpenAI 配置（用于迁移兼容）
 */
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

/**
 * AI 模型提供商配置
 *
 * api_keys 支持多 key failover：第一个为主 key，后续为备用 key；
 * 当某个 key 触发配额/限流/鉴权失败时会自动切换到下一个。
 *
 * api_key_legacy 字段仅用于反序列化兼容旧版（v2.3.1 及之前）单 key 配置，
 * sanitize_settings 会把它合并到 api_keys[0]。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProvider {
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub api_keys: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none", rename = "apiKey")]
  pub api_key_legacy: Option<String>,
  pub base_url: String,
  #[serde(default)]
  pub available_models: Vec<String>,
  #[serde(default)]
  pub enabled_models: Vec<String>,
}

/**
 * 截图翻译功能配置
 */
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
  /// 是否启用思考模式（OCR 模型 + 翻译模型）。默认 false：截图翻译追求快，思考通常没必要。
  #[serde(default = "default_false")]
  pub thinking_enabled: bool,
  /// 是否流式输出 OCR + 翻译。默认 true：用户看着字逐步出现的体感比等"加载完"更顺。
  #[serde(default = "default_true")]
  pub stream_enabled: bool,
  #[serde(default)]
  pub prompt: Option<String>,
  // 旧版字段，用于迁移
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
      thinking_enabled: false,
      stream_enabled: true,
      prompt: None,
      openai: None,
    }
  }
}

/**
 * 对话消息（Lens 多轮对话）
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainMessage {
  pub role: String,
  pub content: String,
}

/**
 * Lens 模式配置
 * 启用后可通过热键进入：屏幕高亮选择窗口/区域 → 截图 → 在悬浮对话栏内提问。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LensConfig {
  #[serde(default = "default_true")]
  pub enabled: bool,
  #[serde(default = "default_lens_hotkey")]
  pub hotkey: String,
  /// provider/model 留空时 fallback 到 translator_provider_id / translator_model
  #[serde(default)]
  pub provider_id: String,
  #[serde(default)]
  pub model: String,
  /// 响应语言（"zh"/"en"）。空字符串表示跟随 settings.target_lang，"auto" 则用 "zh"。
  #[serde(default)]
  pub default_language: String,
  /// 是否流式返回，默认 true。
  #[serde(default = "default_true")]
  pub stream_enabled: bool,
  /// 是否启用思考模式（推理链）。默认 true。
  /// false 时会向请求 body 注入各家厂商关闭思考的字段并集（不认识的会被 provider 忽略）。
  #[serde(default = "default_true")]
  pub thinking_enabled: bool,
  /// 自定义 system prompt。空字符串使用 default_system_prompt 模板。
  #[serde(default)]
  pub system_prompt: String,
  /// 自定义 question prompt。空字符串使用 default_question_prompt 模板。
  #[serde(default)]
  pub question_prompt: String,
  /// 消息排序："asc" 老到新（默认），"desc" 新到老
  #[serde(default = "default_message_order")]
  pub message_order: String,
}

fn default_message_order() -> String {
  "asc".to_string()
}

impl Default for LensConfig {
  fn default() -> Self {
    Self {
      enabled: true,
      hotkey: "CommandOrControl+Shift+G".to_string(),
      provider_id: String::new(),
      model: String::new(),
      default_language: String::new(),
      stream_enabled: true,
      thinking_enabled: true,
      system_prompt: String::new(),
      question_prompt: String::new(),
      message_order: "asc".to_string(),
    }
  }
}

/**
 * 应用完整设置
 */
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
  #[serde(default, alias = "cowork")]
  pub lens: LensConfig,
  #[serde(default = "default_settings_language")]
  pub settings_language: Option<String>,
  #[serde(default = "default_retry_enabled")]
  pub retry_enabled: bool,
  #[serde(default = "default_retry_attempts")]
  pub retry_attempts: u8,
  /// 一次性迁移标记：v2.3.x 钥匙串里的 key 已搬到 api_keys[0] 并清掉旧条目后置 true
  /// 防止 v2.3.x ↔ v2.4 反复切换时重复抹掉钥匙串
  #[serde(default)]
  pub legacy_keyring_migrated: bool,
  // 旧版字段，用于迁移
  #[serde(skip_serializing_if = "Option::is_none")]
  pub openai: Option<OpenAIConfig>,
}

impl Settings {
  /**
   * 根据 ID 查找提供商
   */
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
      lens: LensConfig::default(),
      settings_language: Some("zh".to_string()),
      retry_enabled: default_retry_enabled(),
      retry_attempts: default_retry_attempts(),
      legacy_keyring_migrated: false,
      openai: None,
    }
  }
}

/**
 * 设置数据清理与迁移
 *
 * 执行以下操作：
 * 1. 从旧版单提供商配置迁移到多提供商体系
 * 2. 确保空字段有默认值
 * 3. 确保当前使用的模型在 enabled_models 中
 * 4. 规范化快捷键字符串
 * 5. 确保必要字段不为空
 */
pub fn sanitize_settings(mut settings: Settings) -> Settings {
  // 1. 从旧版配置迁移
  if settings.providers.is_empty() {
    // 迁移翻译提供商
    if let Some(old_openai) = settings.openai.take() {
      let legacy_key = old_openai.api_key.trim().to_string();
      let api_keys = if legacy_key.is_empty() {
        vec![]
      } else {
        vec![legacy_key]
      };
      settings.providers.push(ModelProvider {
        id: "default-translator".to_string(),
        name: "OpenAI (Translator)".to_string(),
        api_keys,
        api_key_legacy: None,
        base_url: old_openai.base_url,
        available_models: vec![],
        enabled_models: vec![old_openai.model.clone()],
      });
      settings.translator_provider_id = "default-translator".to_string();
      settings.translator_model = old_openai.model;
    }

    // 迁移 OCR 提供商
    if let Some(old_ocr) = settings.screenshot_translation.openai.take() {
      let legacy_key = old_ocr.api_key.trim().to_string();
      let api_keys = if legacy_key.is_empty() {
        vec![]
      } else {
        vec![legacy_key]
      };
      settings.providers.push(ModelProvider {
        id: "default-ocr".to_string(),
        name: "OpenAI (OCR)".to_string(),
        api_keys,
        api_key_legacy: None,
        base_url: old_ocr.base_url,
        available_models: vec![],
        enabled_models: vec![old_ocr.model.clone()],
      });
      settings.screenshot_translation.provider_id = "default-ocr".to_string();
      settings.screenshot_translation.model = old_ocr.model;
    }
  }

  // 1b. 单 key → 多 key 迁移（v2.3.1 → v2.4 升级路径）
  for provider in &mut settings.providers {
    if let Some(legacy) = provider.api_key_legacy.take() {
      let trimmed = legacy.trim().to_string();
      if !trimmed.is_empty() && !provider.api_keys.contains(&trimmed) {
        provider.api_keys.insert(0, trimmed);
      }
    }
    // 去重 + 去空
    let mut seen = std::collections::HashSet::new();
    provider.api_keys.retain(|k| {
      let trimmed = k.trim();
      !trimmed.is_empty() && seen.insert(trimmed.to_string())
    });
  }

  // 2. 为空字段设置默认值
  if settings.translator_model.is_empty() {
      settings.translator_model = "gpt-4o".to_string();
  }
  if settings.screenshot_translation.model.is_empty() {
      settings.screenshot_translation.model = "gpt-4o".to_string();
  }

  if settings.translator_provider_id.is_empty() && !settings.providers.is_empty() {
      settings.translator_provider_id = settings.providers[0].id.clone();
  }
  if settings.screenshot_translation.provider_id.is_empty() && !settings.providers.is_empty() {
      settings.screenshot_translation.provider_id = settings.providers[0].id.clone();
  }

  let provider_exists = |id: &str| settings.providers.iter().any(|p| p.id == id);
  if settings.providers.is_empty() {
      settings.translator_provider_id.clear();
      settings.screenshot_translation.provider_id.clear();
      settings.lens.provider_id.clear();
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
      // lens provider 可空（空时 call_vision_api 走 translator_provider_id fallback）；
      // 但若用户填了一个不存在的，重置为空让其走 fallback。
      if !settings.lens.provider_id.is_empty()
        && !provider_exists(&settings.lens.provider_id)
      {
          settings.lens.provider_id.clear();
          settings.lens.model.clear();
      }
  }

  // 3. 确保当前使用的模型在 enabled_models 列表中
  for provider in &mut settings.providers {
      if provider.enabled_models.is_empty() {
          // 如果该提供商被某个功能使用，添加对应模型
          if settings.translator_provider_id == provider.id {
              provider.enabled_models.push(settings.translator_model.clone());
          }
          if settings.screenshot_translation.provider_id == provider.id
            && !provider.enabled_models.contains(&settings.screenshot_translation.model)
          {
              provider.enabled_models.push(settings.screenshot_translation.model.clone());
          }
          if !settings.lens.provider_id.is_empty()
            && settings.lens.provider_id == provider.id
            && !settings.lens.model.is_empty()
            && !provider.enabled_models.contains(&settings.lens.model)
          {
              provider.enabled_models.push(settings.lens.model.clone());
          }
          // 如果仍然为空，添加默认模型
          if provider.enabled_models.is_empty() {
              provider.enabled_models.push("gpt-4o".to_string());
          }
      }

      // 确保当前使用的模型确实在该 provider 的 enabled_models 中
      if settings.translator_provider_id == provider.id && !provider.enabled_models.contains(&settings.translator_model) {
          settings.translator_model = provider.enabled_models[0].clone();
      }
      if settings.screenshot_translation.provider_id == provider.id && !provider.enabled_models.contains(&settings.screenshot_translation.model) {
          settings.screenshot_translation.model = provider.enabled_models[0].clone();
      }
      if !settings.lens.provider_id.is_empty()
        && settings.lens.provider_id == provider.id
        && !settings.lens.model.is_empty()
        && !provider.enabled_models.contains(&settings.lens.model)
      {
          settings.lens.model = provider.enabled_models[0].clone();
      }
  }

  // 4. 规范化快捷键字符串
  settings.hotkey = normalize_hotkey(&settings.hotkey);
  settings.screenshot_translation.hotkey =
    normalize_hotkey(&settings.screenshot_translation.hotkey);
  settings.lens.hotkey = normalize_hotkey(&settings.lens.hotkey);

  // 规范化提示词（去除首尾空白，空值转为 None）
  settings.translator_prompt = normalize_optional_prompt(settings.translator_prompt.take());
  settings.screenshot_translation.prompt =
    normalize_optional_prompt(settings.screenshot_translation.prompt.take());

  // 5. 确保必要字段不为空
  if settings.hotkey.is_empty() {
    settings.hotkey = "CommandOrControl+Alt+T".to_string();
  }
  if settings.screenshot_translation.hotkey.is_empty() {
    settings.screenshot_translation.hotkey = "CommandOrControl+Shift+A".to_string();
  }
  if settings.lens.hotkey.is_empty() {
    settings.lens.hotkey = "CommandOrControl+Shift+G".to_string();
  }
  if settings.lens.message_order != "asc" && settings.lens.message_order != "desc" {
    settings.lens.message_order = "asc".to_string();
  }

  settings.retry_attempts = clamp_retry_attempts(settings.retry_attempts);

  settings
}

/**
 * 持久化设置到存储文件
 * 从 v2.4 起 API Key 直接保存在 settings.json 的 api_keys 数组中
 *
 * 降级兼容：写盘前把 api_keys[0] 镜像到 api_key_legacy（serde rename = "apiKey"）字段，
 * 这样老版本（v2.3.x）反序列化时仍能从 apiKey 字段读到主 key 不丢。
 * 新版加载时 sanitize_settings 会把 api_key_legacy.take() 合并回 api_keys 并去重，无副作用。
 */
pub fn persist_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
  let mut to_persist = settings.clone();
  for provider in &mut to_persist.providers {
    if let Some(primary) = provider.api_keys.first() {
      if !primary.trim().is_empty() {
        provider.api_key_legacy = Some(primary.clone());
      }
    }
  }

  let store = StoreBuilder::new(app, SETTINGS_STORE)
    .build()
    .map_err(|e| e.to_string())?;
  store.set(
    "settings".to_string(),
    serde_json::to_value(&to_persist).map_err(|e| e.to_string())?,
  );
  store.save().map_err(|e| e.to_string())
}

/**
 * 从存储文件加载设置
 * 执行清理迁移；若 settings.json 中无 API Key，则从旧版 keyring 一次性迁移
 */
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
  migrate_legacy_keyring_keys(&mut sanitized);
  sanitized
}

// ========== 默认提示词生成 ==========

/**
 * 获取默认系统提示词
 * has_image=true 时为视觉助手；为 false 时为通用对话助手（不假设有图片）
 * 风格统一：简短直答、无小标题、思考过程尽量精简
 */
pub fn default_system_prompt(language: &str, has_image: bool) -> String {
  match (language, has_image) {
    ("zh", true) => "你是图片分析助手。直接、简洁地回答用户关于图片的问题。回答尽量短、自然流畅，不要小标题或编号。数学公式用 LaTeX（$...$ 或 $$...$$）。思考保持简洁，避免反复重述。".to_string(),
    ("zh", false) => "你是简洁的对话助手。直接给出答案，回答尽量短、自然流畅，不要小标题或编号。数学公式用 LaTeX（$...$ 或 $$...$$）。思考保持简洁，避免反复重述。".to_string(),
    (_, true) => "You analyze images. Answer directly and concisely about what the user asks. Keep responses short and natural — no headings or bullet points. Use LaTeX ($...$ or $$...$$) for math. Think briefly; avoid repeating yourself.".to_string(),
    (_, false) => "You are a concise assistant. Answer directly. Keep responses short and natural — no headings or bullet points. Use LaTeX ($...$ or $$...$$) for math. Think briefly; avoid repeating yourself.".to_string(),
  }
}

/**
 * 关闭思考模式时附加到系统提示词末尾的指令。
 * 提示词层兜底：当 provider 不识别 thinking={type:"disabled"} 字段（如某些第三方代理）时，
 * 仍可让模型按指令省略思考过程。
 */
pub fn no_think_instruction(language: &str) -> &'static str {
  if language == "zh" {
    "\n\n严格要求：直接给出最终答案，不要输出任何思考过程、推理步骤或 <think> 内容。"
  } else {
    "\n\nStrict requirement: output only the final answer; do NOT include any thinking, reasoning steps, or <think> content."
  }
}

/**
 * 获取默认问答提示词
 * has_image=true 时让模型聚焦图片内容；has_image=false 时返回空串（不附加前缀，直接传用户原话）
 */
pub fn default_question_prompt(language: &str, has_image: bool) -> String {
  if !has_image {
    return String::new();
  }
  if language == "zh" {
    "基于这张图片回答用户问题。如果问题与图片无关，礼貌地引导回到图片内容。".to_string()
  } else {
    "Answer the user's question based on this image. If unrelated, politely steer back to the image.".to_string()
  }
}

// ========== 默认值辅助函数 ==========

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

fn default_lens_hotkey() -> String {
  "CommandOrControl+Shift+G".to_string()
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

/**
 * 规范化可选提示词：去除空白，空字符串转为 None
 */
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

/**
 * 规范化快捷键字符串：去除各部分首尾空白并过滤空部分
 */
fn normalize_hotkey(value: &str) -> String {
  value
    .split('+')
    .map(|part| {
      let trimmed = part.trim();
      match trimmed.to_lowercase().as_str() {
        "cmd" | "command" | "commandorcontrol" => "CommandOrControl".to_string(),
        "ctrl" | "control" => "Control".to_string(),
        "opt" | "option" | "alt" => "Alt".to_string(),
        "shift" => "Shift".to_string(),
        "super" | "meta" => "Super".to_string(),
        "plus" => "Plus".to_string(),
        _ => trimmed.to_string(),
      }
    })
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join("+")
}
