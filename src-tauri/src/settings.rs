use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_store::StoreBuilder;

// 设置存储文件名
const SETTINGS_STORE: &str = "settings.json";
// 系统钥匙串服务名（用于安全存储 API Key）
const KEYRING_SERVICE: &str = "com.zmair.kivio";
// 旧版 service 名（v2.4.5 之前为 com.zmair.keylingo），仅用于 legacy 读 + 清理
const KEYRING_SERVICE_LEGACY: &str = "com.zmair.keylingo";
const APPLE_INTELLIGENCE_BASE_URL: &str = "applefoundation://local";

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
 * v2.4.5 (Kivio 重命名) 起：service 名从 com.zmair.keylingo → com.zmair.kivio，
 *   读取时同时尝试两个 service，确保从 KeyLingo 升级上来的用户 key 不丢。
 * 此函数仅在 settings.json 中没有 key 时用一次，迁移完成后旧条目可被清理。
 */
fn legacy_load_keyring_api_key(provider_id: &str) -> Option<String> {
    let cred = provider_credential_name(provider_id);
    for svc in [KEYRING_SERVICE, KEYRING_SERVICE_LEGACY] {
        let Ok(entry) = keyring::Entry::new(svc, &cred) else {
            continue;
        };
        let Ok(raw) = entry.get_password() else {
            continue;
        };
        let trimmed = raw.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

/**
 * 删除旧版 keyring 中的 API Key 条目（迁移完成后清理）
 * 同时清理新旧 service 名下的条目，避免有残留。
 */
fn legacy_clear_keyring_api_key(provider_id: &str) {
    let cred = provider_credential_name(provider_id);
    for svc in [KEYRING_SERVICE, KEYRING_SERVICE_LEGACY] {
        if let Ok(entry) = keyring::Entry::new(svc, &cred) {
            let _ = entry.delete_credential();
        }
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
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    /// 关闭后该供应商不会出现在模型选择器中，已引用它的功能会在保存时切到第一个启用的供应商。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// API 格式：`openai_chat`、`anthropic_messages` 或 `apple_local`。
    /// 旧值 `openai` / `anthropic` 会在 `sanitize_settings` 中归一化。
    #[serde(default = "default_api_format")]
    pub api_format: String,
    /// 用户自定义的模型参数覆盖（仅持久化用户显式修改的字段）
    #[serde(default)]
    pub model_overrides: std::collections::HashMap<String, ModelInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderApiFormat {
    OpenAiChat,
    AnthropicMessages,
    AppleLocal,
}

impl ProviderApiFormat {
    pub fn from_raw(raw: &str, base_url: &str) -> Self {
        if base_url == APPLE_INTELLIGENCE_BASE_URL {
            return Self::AppleLocal;
        }
        match raw.trim() {
            "anthropic" | "anthropic_messages" => Self::AnthropicMessages,
            "apple" | "apple_local" => Self::AppleLocal,
            _ => Self::OpenAiChat,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::AnthropicMessages => "anthropic_messages",
            Self::AppleLocal => "apple_local",
        }
    }
}

impl ModelProvider {
    pub fn api_format_kind(&self) -> ProviderApiFormat {
        ProviderApiFormat::from_raw(&self.api_format, &self.base_url)
    }
}

/**
 * 模型能力信息（来自内置数据库或用户自定义）
 */
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelInfo {
    pub display_name: Option<String>,
    pub context_window: Option<u64>,
    pub max_output: Option<u64>,
    pub capabilities: Option<ModelCapabilities>,
    pub pricing: Option<ModelPricing>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelCapabilities {
    pub vision: Option<bool>,
    pub function_calling: Option<bool>,
    pub reasoning: Option<bool>,
    pub streaming: Option<bool>,
    pub web_search: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelPricing {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cached_input: Option<f64>,
}

/**
 * OCR 引擎模式（截图翻译用）
 *
 * - CloudVision：发图给多模态 provider 一次完成 OCR+翻译（旧 use_system_ocr=false 等价行为）
 * - System：调用 macOS Apple Vision 或 Windows.Media.Ocr 识别文字，再交 provider 翻译
 * - RapidOcr：本地 RapidOCR (PaddleOCR ONNX) 识别文字，再交 provider 翻译。onnxruntime
 *   dylib 与模型文件均由用户在设置页面下载到 app data 目录，安装包不含。
 * - Legacy：反序列化兜底，吸收旧版本 settings.json 里的未知字符串（如 "tesseract"），
 *   sanitize_settings 会迁移到 RapidOcr，保留旧版离线 OCR 的隐私边界。
 *
 * 字段在 sanitize_settings 中由 use_system_ocr 自动迁移：true→System，false→CloudVision。
 * persist_settings 写盘时反向镜像到 use_system_ocr 维持降级到 v2.5.x 的兼容性。
 * RapidOcr 模式降级会落回 CloudVision（use_system_ocr=false），可接受。
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrMode {
    CloudVision,
    System,
    RapidOcr,
    #[serde(other)]
    Legacy,
}

impl Default for OcrMode {
    fn default() -> Self {
        OcrMode::CloudVision
    }
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
    #[serde(default = "default_screenshot_translation_text_hotkey")]
    pub text_hotkey: String,
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
    /// 截图后是否保留 lens 全屏覆盖。默认 true：选区高亮 + 译文卡同屏；false → lens 缩成浮动小窗，不挡下层 app。
    #[serde(default = "default_true")]
    pub keep_fullscreen_after_capture: bool,
    /// 用平台本地 OCR 做文字识别，把识别出的文字喂给翻译模型（macOS Apple Vision / Windows OCR）。
    /// true → 系统 OCR + provider 文字翻译（provider 可是任意 OpenAI 兼容 endpoint 或 Apple Intelligence）
    /// false → provider 必须是多模态模型，一次完成 OCR+翻译
    /// Apple Intelligence 选作 provider 时强制视为 true（Foundation Models 暂未开放图像输入）。
    ///
    /// 从 vNext 起，截图翻译路由实际走 ocr_mode 字段；本字段仅作降级镜像保留：
    /// - persist_settings 写盘时根据 ocr_mode 反向镜像到这里（System→true，其它→false），
    ///   让降级到 v2.5.x 的版本仍能从 useSystemOcr 字段读到对应行为。
    /// - sanitize_settings 在 ocr_mode 缺省时会从这里反推迁移。
    #[serde(default = "default_false")]
    pub use_system_ocr: bool,
    /// OCR 引擎选择（vNext+）。None 表示老版本数据，会在 sanitize_settings 中按 use_system_ocr 迁移。
    #[serde(default)]
    pub ocr_mode: Option<OcrMode>,
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
            text_hotkey: "CommandOrControl+Shift+T".to_string(),
            provider_id: "default-ocr".to_string(),
            model: "gpt-4o".to_string(),
            direct_translate: false,
            thinking_enabled: false,
            stream_enabled: true,
            keep_fullscreen_after_capture: true,
            use_system_ocr: false,
            ocr_mode: Some(OcrMode::CloudVision),
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
 * Lens 联网搜索提供商。
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchProvider {
    Tavily,
    Exa,
}

impl Default for WebSearchProvider {
    fn default() -> Self {
        WebSearchProvider::Tavily
    }
}

/**
 * Lens 联网搜索配置。
 *
 * 手动模式由前端在单次提问时传 web_search=true；后端仍会检查 enabled 和 key。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LensWebSearchConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    #[serde(default)]
    pub provider: WebSearchProvider,
    #[serde(default)]
    pub tavily_api_key: String,
    #[serde(default)]
    pub exa_api_key: String,
    #[serde(default = "default_web_search_max_results")]
    pub max_results: u8,
    #[serde(default = "default_web_search_depth")]
    pub search_depth: String,
}

impl Default for LensWebSearchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: WebSearchProvider::Tavily,
            tavily_api_key: String::new(),
            exa_api_key: String::new(),
            max_results: default_web_search_max_results(),
            search_depth: default_web_search_depth(),
        }
    }
}

fn default_web_search_max_results() -> u8 {
    5
}

fn default_web_search_depth() -> String {
    "basic".to_string()
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
    /// 进入截图选择态时是否显示顶部提示。默认 true，避免用户按下快捷键后看不出已进入截图模式。
    #[serde(default = "default_true")]
    pub show_capture_hint: bool,
    /// Windows 兼容模式：进入截图选择态前先抓取当前显示器冻结帧，再在覆盖层内显示和裁剪冻结帧。
    /// 默认 false，保留实时透明覆盖层行为；用于规避浏览器视频在透明置顶 WebView2 下变黑。
    #[serde(default = "default_false")]
    pub windows_freeze_frame_selection: bool,
    #[serde(default)]
    pub web_search: LensWebSearchConfig,
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
            show_capture_hint: true,
            windows_freeze_frame_selection: false,
            web_search: LensWebSearchConfig::default(),
        }
    }
}

/**
 * AI 客户端（Chat）行为配置：与 Lens 分离，避免截图问答与对话客户端共用开关。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatConfig {
    #[serde(default = "default_true")]
    pub stream_enabled: bool,
    #[serde(default = "default_true")]
    pub thinking_enabled: bool,
    /// 响应语言（"zh"/"en" 等）。空字符串表示跟随 Lens 默认语言，再跟随 target_lang。
    #[serde(default)]
    pub default_language: String,
    /// 自定义 system prompt；空则使用内置 Chat 模板。
    #[serde(default)]
    pub system_prompt: String,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            stream_enabled: true,
            thinking_enabled: true,
            default_language: String::new(),
            system_prompt: String::new(),
        }
    }
}

/**
 * 可选模型选择：provider_id 为空表示未单独设置。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DefaultModelSelection {
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub model: String,
}

impl Default for DefaultModelSelection {
    fn default() -> Self {
        Self {
            provider_id: String::new(),
            model: String::new(),
        }
    }
}

impl DefaultModelSelection {
    fn is_configured(&self) -> bool {
        !self.provider_id.trim().is_empty()
    }
}

/**
 * 默认模型配置。
 *
 * chat：新建 Chat 对话的全局默认模型；为空时沿用 Lens → 输入翻译的兜底链路。
 * vision：图片附件分析副任务使用；为空时保持 Chat 主模型直接处理图片。
 * title_summary：标题总结副任务使用；为空时继承有效 Chat 默认模型。
 * compression：上下文/历史对话压缩副任务使用；为空时继承有效 Chat 默认模型。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DefaultModelsConfig {
    #[serde(default)]
    pub chat: DefaultModelSelection,
    #[serde(default)]
    pub vision: DefaultModelSelection,
    #[serde(default)]
    pub title_summary: DefaultModelSelection,
    #[serde(default)]
    pub compression: DefaultModelSelection,
}

impl Default for DefaultModelsConfig {
    fn default() -> Self {
        Self {
            chat: DefaultModelSelection::default(),
            vision: DefaultModelSelection::default(),
            title_summary: DefaultModelSelection::default(),
            compression: DefaultModelSelection::default(),
        }
    }
}

/// 解析 Chat 使用的响应语言代码。
pub fn resolve_chat_language(settings: &Settings) -> String {
    if !settings.chat.default_language.trim().is_empty() {
        return settings.chat.default_language.trim().to_string();
    }
    if !settings.lens.default_language.trim().is_empty() {
        return settings.lens.default_language.trim().to_string();
    }
    match settings.target_lang.as_str() {
        "en" => "en".to_string(),
        "zh-Hant" | "zh-TW" => "zh-Hant".to_string(),
        _ => "zh".to_string(),
    }
}

/**
 * Chat MCP stdio server 配置。
 *
 * settings.json 使用 camelCase；env 与 API keys 一样按本地明文设置策略保存。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatMcpServer {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub transport: String,
    pub url: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub headers: std::collections::HashMap<String, String>,
    pub cwd: Option<String>,
    pub enabled_tools: Vec<String>,
}

impl Default for ChatMcpServer {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            enabled: false,
            transport: "stdio".to_string(),
            url: String::new(),
            command: String::new(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatNativeToolsConfig {
    pub web_search: bool,
    #[serde(default)]
    pub web_fetch: bool,
    pub skill_runtime: bool,
    #[serde(default)]
    pub read_file: bool,
    #[serde(default)]
    pub write_file: bool,
    #[serde(default)]
    pub edit_file: bool,
    #[serde(default)]
    pub run_command: bool,
    #[serde(default)]
    pub run_python: bool,
    #[serde(default)]
    pub workspace_roots: Vec<String>,
}

impl ChatNativeToolsConfig {
    pub fn any_enabled(&self) -> bool {
        self.web_search
            || self.web_fetch
            || self.skill_runtime
            || self.read_file
            || self.write_file
            || self.edit_file
            || self.run_command
            || self.run_python
    }
}

impl Default for ChatNativeToolsConfig {
    fn default() -> Self {
        Self {
            web_search: false,
            web_fetch: false,
            skill_runtime: true,
            read_file: false,
            write_file: false,
            edit_file: false,
            run_command: false,
            run_python: false,
            workspace_roots: Vec::new(),
        }
    }
}

fn default_skill_auto_match() -> bool {
    true
}

fn default_skill_fallback_mode() -> String {
    "progressive".to_string()
}

fn default_skill_script_allowlist() -> Vec<String> {
    vec![
        "python3".to_string(),
        "bash".to_string(),
        "sh".to_string(),
        "node".to_string(),
    ]
}

fn default_chat_max_tool_rounds() -> u8 {
    10
}

pub const CHAT_TOOL_MIN_TIMEOUT_MS: u64 = 1_000;
pub const CHAT_TOOL_MAX_TIMEOUT_MS: u64 = 300_000;
pub const SKILL_SCRIPT_MIN_TIMEOUT_MS: u64 = 120_000;

fn default_chat_tool_timeout_ms() -> u64 {
    60_000
}

fn default_chat_max_tool_output_chars() -> usize {
    12_000
}

fn default_chat_approval_policy() -> String {
    "readonly_auto_sensitive_confirm".to_string()
}

/**
 * Chat 工具与 Skill 配置。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatToolsConfig {
    pub enabled: bool,
    pub servers: Vec<ChatMcpServer>,
    pub skill_scan_paths: Vec<String>,
    #[serde(default = "default_skill_auto_match")]
    pub skill_auto_match: bool,
    #[serde(default = "default_skill_fallback_mode")]
    pub skill_fallback_mode: String,
    #[serde(default = "default_skill_script_allowlist")]
    pub skill_script_allowlist: Vec<String>,
    /// Skill ids the user turned off in Settings. Omitted ids are enabled.
    #[serde(default)]
    pub disabled_skill_ids: Vec<String>,
    #[serde(default = "default_chat_max_tool_rounds")]
    pub max_tool_rounds: u8,
    #[serde(default = "default_chat_tool_timeout_ms")]
    pub tool_timeout_ms: u64,
    #[serde(default = "default_chat_max_tool_output_chars")]
    pub max_tool_output_chars: usize,
    #[serde(default = "default_chat_approval_policy")]
    pub approval_policy: String,
    pub native_tools: ChatNativeToolsConfig,
}

impl Default for ChatToolsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            servers: Vec::new(),
            skill_scan_paths: Vec::new(),
            skill_auto_match: default_skill_auto_match(),
            skill_fallback_mode: default_skill_fallback_mode(),
            skill_script_allowlist: default_skill_script_allowlist(),
            disabled_skill_ids: Vec::new(),
            max_tool_rounds: default_chat_max_tool_rounds(),
            tool_timeout_ms: default_chat_tool_timeout_ms(),
            max_tool_output_chars: default_chat_max_tool_output_chars(),
            approval_policy: default_chat_approval_policy(),
            native_tools: ChatNativeToolsConfig::default(),
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
    pub chat_provider_id: String,
    #[serde(default)]
    pub chat_model: String,
    #[serde(default)]
    pub default_models: DefaultModelsConfig,
    #[serde(default)]
    pub translator_prompt: Option<String>,
    #[serde(default)]
    pub providers: Vec<ModelProvider>,
    #[serde(default)]
    pub screenshot_translation: ScreenshotTranslationConfig,
    #[serde(default, alias = "cowork")]
    pub lens: LensConfig,
    #[serde(default)]
    pub chat: ChatConfig,
    #[serde(default)]
    pub chat_tools: ChatToolsConfig,
    /// 一次性：将 Lens 的流式/思考开关复制到独立的 Chat 配置（旧版共用 Lens 行为）。
    #[serde(default)]
    pub chat_behavior_migrated_from_lens: bool,
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
    /// 启动时静默检查 GitHub Releases 是否有新版（默认 true）
    /// 仅做"提示 + 跳转 GH 下载页"，不集成 auto-installer，避免签名密钥那套
    #[serde(default = "default_true")]
    pub auto_check_update: bool,
    /// 截图自动归档开关（默认 false）
    #[serde(default = "default_false")]
    pub image_archive_enabled: bool,
    /// 自动归档目标目录路径（空字符串表示未设置）
    #[serde(default)]
    pub image_archive_path: String,
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

    pub fn effective_chat_model(&self) -> (String, String) {
        if self.default_models.chat.is_configured() {
            return (
                self.default_models.chat.provider_id.clone(),
                self.default_models.chat.model.clone(),
            );
        }
        if !self.lens.provider_id.trim().is_empty() {
            return (self.lens.provider_id.clone(), self.lens.model.clone());
        }
        (
            self.translator_provider_id.clone(),
            self.translator_model.clone(),
        )
    }

    pub fn effective_title_summary_model(&self) -> (String, String) {
        if self.default_models.title_summary.is_configured() {
            return (
                self.default_models.title_summary.provider_id.clone(),
                self.default_models.title_summary.model.clone(),
            );
        }
        self.effective_chat_model()
    }

    pub fn has_explicit_vision_model(&self) -> bool {
        self.default_models.vision.is_configured()
    }

    pub fn effective_vision_model(&self) -> (String, String) {
        if self.default_models.vision.is_configured() {
            return (
                self.default_models.vision.provider_id.clone(),
                self.default_models.vision.model.clone(),
            );
        }
        self.effective_chat_model()
    }

    pub fn effective_compression_model(&self) -> (String, String) {
        if self.default_models.compression.is_configured() {
            return (
                self.default_models.compression.provider_id.clone(),
                self.default_models.compression.model.clone(),
            );
        }
        self.effective_chat_model()
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
            chat_provider_id: String::new(),
            chat_model: String::new(),
            default_models: DefaultModelsConfig::default(),
            translator_prompt: None,
            providers: vec![],
            screenshot_translation: ScreenshotTranslationConfig::default(),
            lens: LensConfig::default(),
            chat: ChatConfig::default(),
            chat_tools: ChatToolsConfig::default(),
            chat_behavior_migrated_from_lens: false,
            settings_language: Some("zh".to_string()),
            retry_enabled: default_retry_enabled(),
            retry_attempts: default_retry_attempts(),
            legacy_keyring_migrated: false,
            auto_check_update: true,
            image_archive_enabled: false,
            image_archive_path: String::new(),
            openai: None,
        }
    }
}

/**
 * 设置数据清理与迁移
 *
 * 执行以下操作：
 * 1. 从旧版单提供商配置迁移到多提供商体系
 * 2. 确保空 provider 字段有默认值
 * 3. 如果当前模型不在 enabled_models 中则清空或切到第一个启用模型
 * 4. 规范化快捷键字符串
 * 5. 确保必要字段不为空
 */
pub fn chat_native_tools_enabled(chat_tools: &ChatToolsConfig) -> bool {
    chat_tools.native_tools.any_enabled()
}

pub fn is_skill_enabled(chat_tools: &ChatToolsConfig, skill_id: &str) -> bool {
    let skill_id = skill_id.trim();
    if skill_id.is_empty() {
        return false;
    }
    !chat_tools
        .disabled_skill_ids
        .iter()
        .any(|disabled| disabled == skill_id)
}

fn sanitize_default_model_selection(
    selection: &mut DefaultModelSelection,
    providers: &[ModelProvider],
) {
    selection.provider_id = selection.provider_id.trim().to_string();
    selection.model = selection.model.trim().to_string();
    if selection.provider_id.is_empty() {
        selection.model.clear();
        return;
    }

    let Some(provider) = providers
        .iter()
        .find(|p| p.id == selection.provider_id && p.enabled)
    else {
        selection.provider_id.clear();
        selection.model.clear();
        return;
    };

    if !provider.enabled_models.is_empty() && !provider.enabled_models.contains(&selection.model) {
        selection.model = provider.enabled_models.first().cloned().unwrap_or_default();
    }
}

fn sync_legacy_chat_model_fields(settings: &mut Settings) {
    let (provider_id, model) = settings.effective_chat_model();
    settings.chat_provider_id = provider_id;
    settings.chat_model = model;
}

fn mirror_explicit_chat_default_for_persistence(settings: &mut Settings) {
    if settings.default_models.chat.is_configured() {
        settings.chat_provider_id = settings.default_models.chat.provider_id.clone();
        settings.chat_model = settings.default_models.chat.model.clone();
    } else {
        settings.chat_provider_id.clear();
        settings.chat_model.clear();
    }
}

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
                supports_tools: true,
                enabled: true,
                api_format: "openai".to_string(),
                model_overrides: std::collections::HashMap::new(),
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
                supports_tools: true,
                enabled: true,
                api_format: "openai".to_string(),
                model_overrides: std::collections::HashMap::new(),
            });
            settings.screenshot_translation.provider_id = "default-ocr".to_string();
            settings.screenshot_translation.model = old_ocr.model;
        }
    }

    // 1b. 单 key → 多 key 迁移（v2.3.1 → v2.4 升级路径）
    for provider in &mut settings.providers {
        if provider.base_url == APPLE_INTELLIGENCE_BASE_URL {
            provider.supports_tools = false;
        }
        provider.api_format = provider.api_format_kind().as_str().to_string();
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

    let provider_exists = |id: &str| settings.providers.iter().any(|p| p.id == id);
    let provider_selectable = |id: &str| settings.providers.iter().any(|p| p.id == id && p.enabled);
    let first_selectable_provider = || settings.providers.iter().find(|p| p.enabled);

    // 2. 为空字段设置默认值
    if settings.translator_provider_id.is_empty() {
        if let Some(first) = first_selectable_provider() {
            settings.translator_provider_id = first.id.clone();
        }
    }
    if settings.screenshot_translation.provider_id.is_empty() {
        if let Some(first) = first_selectable_provider() {
            settings.screenshot_translation.provider_id = first.id.clone();
        }
    }
    if !settings.chat_provider_id.trim().is_empty()
        && settings.default_models.chat.provider_id.trim().is_empty()
    {
        settings.default_models.chat.provider_id = settings.chat_provider_id.clone();
        settings.default_models.chat.model = settings.chat_model.clone();
    }

    if settings.providers.is_empty() {
        settings.translator_provider_id.clear();
        settings.default_models = DefaultModelsConfig::default();
        settings.screenshot_translation.provider_id.clear();
        settings.lens.provider_id.clear();
    } else {
        if !provider_selectable(&settings.translator_provider_id) {
            if let Some(first) = first_selectable_provider() {
                settings.translator_provider_id = first.id.clone();
                if let Some(model) = first.enabled_models.first() {
                    settings.translator_model = model.clone();
                }
            } else if !provider_exists(&settings.translator_provider_id) {
                settings.translator_provider_id.clear();
                settings.translator_model.clear();
            }
        }
        if !provider_selectable(&settings.screenshot_translation.provider_id) {
            if let Some(first) = first_selectable_provider() {
                settings.screenshot_translation.provider_id = first.id.clone();
                if let Some(model) = first.enabled_models.first() {
                    settings.screenshot_translation.model = model.clone();
                }
            } else if !provider_exists(&settings.screenshot_translation.provider_id) {
                settings.screenshot_translation.provider_id.clear();
                settings.screenshot_translation.model.clear();
            }
        }
        // lens provider 可空（空时 call_vision_api 走 translator_provider_id fallback）；
        // 但若用户填了一个不存在或已禁用的，重置为空让其走 fallback。
        if !settings.lens.provider_id.is_empty()
            && (!provider_exists(&settings.lens.provider_id)
                || !provider_selectable(&settings.lens.provider_id))
        {
            settings.lens.provider_id.clear();
            settings.lens.model.clear();
        }

        sanitize_default_model_selection(&mut settings.default_models.chat, &settings.providers);
        sanitize_default_model_selection(&mut settings.default_models.vision, &settings.providers);
        sanitize_default_model_selection(
            &mut settings.default_models.title_summary,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.compression,
            &settings.providers,
        );
    }

    // 3. 确保当前使用的模型确实在该 provider 的 enabled_models 中。
    // enabled_models 可以为空：预设 provider 不再自带模型。
    for provider in &mut settings.providers {
        if settings.translator_provider_id == provider.id
            && !provider.enabled_models.contains(&settings.translator_model)
        {
            settings.translator_model =
                provider.enabled_models.first().cloned().unwrap_or_default();
        }
        if settings.screenshot_translation.provider_id == provider.id
            && !provider
                .enabled_models
                .contains(&settings.screenshot_translation.model)
        {
            settings.screenshot_translation.model =
                provider.enabled_models.first().cloned().unwrap_or_default();
        }
        if !settings.lens.provider_id.is_empty()
            && settings.lens.provider_id == provider.id
            && !settings.lens.model.is_empty()
            && !provider.enabled_models.contains(&settings.lens.model)
        {
            settings.lens.model = provider.enabled_models.first().cloned().unwrap_or_default();
        }
    }

    // 非 macOS 上 Apple Intelligence provider 可能来自跨平台同步的 settings.json。
    // 保留 provider 记录本身，但当前功能不能继续选中它，否则运行时会尝试走端上 Apple 路由。
    #[cfg(not(target_os = "macos"))]
    {
        let apple_provider_ids: std::collections::HashSet<String> = settings
            .providers
            .iter()
            .filter(|p| p.base_url == APPLE_INTELLIGENCE_BASE_URL)
            .map(|p| p.id.clone())
            .collect();
        let fallback = settings
            .providers
            .iter()
            .find(|p| p.enabled && p.base_url != APPLE_INTELLIGENCE_BASE_URL)
            .map(|p| {
                (
                    p.id.clone(),
                    p.enabled_models.first().cloned().unwrap_or_default(),
                )
            });

        if apple_provider_ids.contains(&settings.translator_provider_id) {
            if let Some((id, model)) = fallback.as_ref() {
                settings.translator_provider_id = id.clone();
                settings.translator_model = model.clone();
            } else {
                settings.translator_provider_id.clear();
                settings.translator_model.clear();
            }
        }
        if apple_provider_ids.contains(&settings.screenshot_translation.provider_id) {
            if let Some((id, model)) = fallback.as_ref() {
                settings.screenshot_translation.provider_id = id.clone();
                settings.screenshot_translation.model = model.clone();
            } else {
                settings.screenshot_translation.provider_id.clear();
                settings.screenshot_translation.model.clear();
            }
        }
        if !settings.lens.provider_id.is_empty()
            && apple_provider_ids.contains(&settings.lens.provider_id)
        {
            settings.lens.provider_id.clear();
            settings.lens.model.clear();
        }
        for selection in [
            &mut settings.default_models.chat,
            &mut settings.default_models.vision,
            &mut settings.default_models.title_summary,
            &mut settings.default_models.compression,
        ] {
            if apple_provider_ids.contains(&selection.provider_id) {
                if let Some((id, model)) = fallback.as_ref() {
                    selection.provider_id = id.clone();
                    selection.model = model.clone();
                } else {
                    selection.provider_id.clear();
                    selection.model.clear();
                }
            }
        }
    }

    sync_legacy_chat_model_fields(&mut settings);

    // 4. 规范化快捷键字符串
    settings.hotkey = normalize_hotkey(&settings.hotkey);
    settings.screenshot_translation.hotkey =
        normalize_hotkey(&settings.screenshot_translation.hotkey);
    settings.screenshot_translation.text_hotkey =
        normalize_hotkey(&settings.screenshot_translation.text_hotkey);
    settings.lens.hotkey = normalize_hotkey(&settings.lens.hotkey);

    // 规范化提示词（去除首尾空白，空值转为 None）
    settings.translator_prompt = normalize_optional_prompt(settings.translator_prompt.take());
    settings.screenshot_translation.prompt =
        normalize_optional_prompt(settings.screenshot_translation.prompt.take());

    // 5. 其他字段验证
    if settings.lens.message_order != "asc" && settings.lens.message_order != "desc" {
        settings.lens.message_order = "asc".to_string();
    }
    settings.lens.web_search.tavily_api_key =
        settings.lens.web_search.tavily_api_key.trim().to_string();
    settings.lens.web_search.exa_api_key = settings.lens.web_search.exa_api_key.trim().to_string();
    settings.lens.web_search.max_results = settings.lens.web_search.max_results.clamp(1, 10);
    if !matches!(
        settings.lens.web_search.search_depth.as_str(),
        "ultra-fast" | "fast" | "basic" | "advanced"
    ) {
        settings.lens.web_search.search_depth = default_web_search_depth();
    }

    if !settings.chat_behavior_migrated_from_lens {
        settings.chat.stream_enabled = settings.lens.stream_enabled;
        settings.chat.thinking_enabled = settings.lens.thinking_enabled;
        if settings.lens.default_language.trim().is_empty() {
            // keep chat.default_language empty → inherit chain unchanged
        } else {
            settings.chat.default_language = settings.lens.default_language.clone();
        }
        settings.chat_behavior_migrated_from_lens = true;
    }
    if !matches!(
        settings.chat.default_language.trim(),
        "" | "zh" | "zh-Hant" | "en"
    ) {
        settings.chat.default_language.clear();
    }
    settings.chat.system_prompt = settings.chat.system_prompt.trim().to_string();

    settings.chat_tools.max_tool_rounds = settings.chat_tools.max_tool_rounds.clamp(1, 30);
    settings.chat_tools.tool_timeout_ms = settings
        .chat_tools
        .tool_timeout_ms
        .clamp(CHAT_TOOL_MIN_TIMEOUT_MS, CHAT_TOOL_MAX_TIMEOUT_MS);
    settings.chat_tools.max_tool_output_chars = settings
        .chat_tools
        .max_tool_output_chars
        .clamp(1_000, 50_000);
    if !matches!(
        settings.chat_tools.approval_policy.trim(),
        "readonly_auto_sensitive_confirm" | "always_confirm" | "auto"
    ) {
        settings.chat_tools.approval_policy = default_chat_approval_policy();
    }
    settings.chat_tools.skill_scan_paths = settings
        .chat_tools
        .skill_scan_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    if !matches!(
        settings.chat_tools.skill_fallback_mode.trim(),
        "progressive" | "skill_md_only" | "legacy_full_body"
    ) {
        settings.chat_tools.skill_fallback_mode = default_skill_fallback_mode();
    }
    settings.chat_tools.skill_script_allowlist = settings
        .chat_tools
        .skill_script_allowlist
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    if settings.chat_tools.skill_script_allowlist.is_empty() {
        settings.chat_tools.skill_script_allowlist = default_skill_script_allowlist();
    }
    settings.chat_tools.disabled_skill_ids = settings
        .chat_tools
        .disabled_skill_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    settings.chat_tools.native_tools.workspace_roots = settings
        .chat_tools
        .native_tools
        .workspace_roots
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    let mut seen_roots = std::collections::HashSet::new();
    settings
        .chat_tools
        .native_tools
        .workspace_roots
        .retain(|path| seen_roots.insert(path.clone()));
    for server in &mut settings.chat_tools.servers {
        server.id = server.id.trim().to_string();
        if server.id.is_empty() {
            server.id = format!("mcp-{}", uuid::Uuid::new_v4());
        }
        server.name = server.name.trim().to_string();
        if server.name.is_empty() {
            server.name = server.id.clone();
        }
        server.transport = server.transport.trim().to_ascii_lowercase();
        if server.transport == "http" || server.transport == "sse" {
            server.transport = "streamable_http".to_string();
        }
        if server.transport != "stdio" && server.transport != "streamable_http" {
            server.transport = "stdio".to_string();
        }
        server.url = server.url.trim().to_string();
        server.command = server.command.trim().to_string();
        server.args = server
            .args
            .iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect();
        server.env = server
            .env
            .iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some((key.to_string(), value.clone()))
                }
            })
            .collect();
        server.headers = server
            .headers
            .iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some((key.to_string(), value.trim().to_string()))
                }
            })
            .collect();
        server.cwd = server.cwd.take().and_then(|cwd| {
            let trimmed = cwd.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        server.enabled_tools = server
            .enabled_tools
            .iter()
            .map(|tool| tool.trim().to_string())
            .filter(|tool| !tool.is_empty())
            .collect();
    }

    // 清理归档目录路径（去除首尾空白）
    settings.image_archive_path = settings.image_archive_path.trim().to_string();

    settings.retry_attempts = clamp_retry_attempts(settings.retry_attempts);

    // 系统 OCR 依赖平台本地 OCR 能力（macOS Apple Vision / Windows.Media.Ocr）。其它平台
    // 同步来的旧配置必须关闭，否则截图翻译会误入不可用分支。
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        settings.screenshot_translation.use_system_ocr = false;
    }

    // OCR 引擎模式迁移（vNext+）：
    // 1. 反序列化兜底变体 OcrMode::Legacy（如旧版 "tesseract" 字符串）→ RapidOcr，
    //    保留用户此前选择离线 OCR 的隐私边界；模型未下载时由前端引导下载。
    // 2. 若 ocr_mode 缺省（老版本数据），按 use_system_ocr 反推：
    //    true→System，false→CloudVision
    // 3. Linux 不支持 System / RapidOcr，强制落回 CloudVision
    if matches!(
        settings.screenshot_translation.ocr_mode,
        Some(OcrMode::Legacy)
    ) {
        settings.screenshot_translation.ocr_mode = Some(OcrMode::RapidOcr);
    }
    if settings.screenshot_translation.ocr_mode.is_none() {
        settings.screenshot_translation.ocr_mode =
            Some(if settings.screenshot_translation.use_system_ocr {
                OcrMode::System
            } else {
                OcrMode::CloudVision
            });
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if matches!(
            settings.screenshot_translation.ocr_mode,
            Some(OcrMode::System) | Some(OcrMode::RapidOcr)
        ) {
            settings.screenshot_translation.ocr_mode = Some(OcrMode::CloudVision);
        }
    }

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
    // Keep legacy top-level chat fields from turning Lens/Translator fallback into
    // an explicit defaultModels.chat selection on the next load.
    mirror_explicit_chat_default_for_persistence(&mut to_persist);

    for provider in &mut to_persist.providers {
        if let Some(primary) = provider.api_keys.first() {
            if !primary.trim().is_empty() {
                provider.api_key_legacy = Some(primary.clone());
            }
        }
    }

    // 降级镜像：把 ocr_mode 投影回 use_system_ocr，让降级到 v2.5.x 的版本仍能从 useSystemOcr 字段
    // 读到对应行为。RapidOcr 模式镜像为 false（v2.5.x 没有 RapidOCR 概念，落回 CloudVision）。
    let ocr_mode = to_persist
        .screenshot_translation
        .ocr_mode
        .unwrap_or(OcrMode::CloudVision);
    to_persist.screenshot_translation.use_system_ocr = matches!(ocr_mode, OcrMode::System);
    to_persist.screenshot_translation.ocr_mode = Some(ocr_mode);

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
 * 一次性数据迁移：v2.4.5 把 identifier 从 com.zmair.keylingo 改为 com.zmair.kivio。
 * Tauri 的 app_data_dir 直接由 identifier 派生，改名后新目录是空的，
 * 老用户升级会丢失 settings.json / lens-history。这里在新目录还没数据时，
 * 把同级的旧目录整个递归拷贝过来。
 *
 * 幂等：新目录已存在 settings.json → 跳过；旧目录不存在 → 跳过（全新安装）。
 */
fn migrate_legacy_app_data(app: &AppHandle) {
    use tauri::Manager;
    let new_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(err) => {
            eprintln!("[migrate-app-data] app_data_dir unavailable: {err}");
            return;
        }
    };
    if new_dir.join(SETTINGS_STORE).exists() {
        return;
    }

    let Some(parent) = new_dir.parent() else {
        return;
    };
    // 旧 identifier 的目录名就是 identifier 本身（macOS / Windows / Linux 都一致）
    let legacy_dir = parent.join("com.zmair.keylingo");
    if !legacy_dir.is_dir() {
        return;
    }

    if let Err(err) = std::fs::create_dir_all(&new_dir) {
        eprintln!("[migrate-app-data] mkdir new dir failed: {err}");
        return;
    }

    match copy_dir_recursive(&legacy_dir, &new_dir) {
        Ok(()) => eprintln!(
            "[migrate-app-data] copied legacy app data: {} → {}",
            legacy_dir.display(),
            new_dir.display()
        ),
        Err(err) => eprintln!("[migrate-app-data] copy failed: {err}"),
    }
}

fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if src.is_file() && !dst.exists() {
            // 不覆盖已有目标文件：避免与用户在新路径下手动建/写过的内容冲突
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/**
 * 从存储文件加载设置
 * 执行清理迁移；若 settings.json 中无 API Key，则从旧版 keyring 一次性迁移
 */
pub fn load_settings(app: &AppHandle) -> Settings {
    // 入口先把旧 identifier 目录的数据搬到新目录（幂等）
    migrate_legacy_app_data(app);
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
/// Local system clock for Chat date/time questions. Models must not guess dates from training data.
pub fn chat_current_datetime_context(language: &str) -> String {
    let now = Local::now();
    let weekday = weekday_label(language, now.weekday());
    if language.starts_with("zh") {
        format!(
            "\n\n当前本地时间（系统时钟；回答今天/明天/星期几等日期时间问题必须以此为准，禁止凭记忆臆测）：{}年{}月{}日 {} {:02}:{:02}。",
            now.year(),
            now.month(),
            now.day(),
            weekday,
            now.hour(),
            now.minute()
        )
    } else {
        format!(
            "\n\nCurrent local time (system clock; use for today/tomorrow/weekday questions—never guess from training data): {}-{:02}-{:02} {} {:02}:{:02}.",
            now.year(),
            now.month(),
            now.day(),
            weekday,
            now.hour(),
            now.minute()
        )
    }
}

fn weekday_label(language: &str, weekday: chrono::Weekday) -> &'static str {
    if language.starts_with("zh") {
        match weekday {
            chrono::Weekday::Mon => "星期一",
            chrono::Weekday::Tue => "星期二",
            chrono::Weekday::Wed => "星期三",
            chrono::Weekday::Thu => "星期四",
            chrono::Weekday::Fri => "星期五",
            chrono::Weekday::Sat => "星期六",
            chrono::Weekday::Sun => "星期日",
        }
    } else {
        match weekday {
            chrono::Weekday::Mon => "Monday",
            chrono::Weekday::Tue => "Tuesday",
            chrono::Weekday::Wed => "Wednesday",
            chrono::Weekday::Thu => "Thursday",
            chrono::Weekday::Fri => "Friday",
            chrono::Weekday::Sat => "Saturday",
            chrono::Weekday::Sun => "Sunday",
        }
    }
}

/// Lens 默认系统提示（含截图翻译后的视觉问答）：输出紧凑，尽量不输出空行。
pub fn default_lens_system_prompt(language: &str, has_image: bool) -> String {
    match (language.starts_with("zh"), has_image) {
        (true, true) => "你是一位智能助手，能够看到用户分享的截图。请将其作为视觉上下文来理解和回答，可以涉及信息提取、概念解释、操作协助或任何相关话题。保持回答简洁直接，自然流畅，不用小标题和编号。输出必须紧凑：不要输出空行；只有在真正需要分隔段落、列表项、表格行、代码块或数学公式时才换行；列表项之间不要留空行。数学公式用 LaTeX（$...$ 或 $$...$$）。思考保持简洁，避免反复重述。".to_string(),
        (true, false) => "你是一位智能助手。直接给出答案，回答简洁、自然流畅，不要小标题或编号。输出必须紧凑：不要输出空行；只有在真正需要分隔段落、列表项、表格行、代码块或数学公式时才换行；列表项之间不要留空行。数学公式用 LaTeX（$...$ 或 $$...$$）。思考保持简洁，避免反复重述。".to_string(),
        (_, true) => "You are a helpful assistant that can see the user's screenshot. Use it as visual context to understand and answer, whether extracting information, explaining concepts, assisting with tasks, or any relevant topic. Keep responses short and natural, with no headings or bullet points unless a list is genuinely useful. Keep output compact: do not output blank lines; use a single newline only when needed for clear paragraph boundaries, list items, table rows, code blocks, or math; never put empty lines between list items. Use LaTeX ($...$ or $$...$$) for math. Think briefly; avoid repeating yourself.".to_string(),
        (_, false) => "You are a helpful assistant. Answer directly. Keep responses short and natural, with no headings or bullet points unless a list is genuinely useful. Keep output compact: do not output blank lines; use a single newline only when needed for clear paragraph boundaries, list items, table rows, code blocks, or math; never put empty lines between list items. Use LaTeX ($...$ or $$...$$) for math. Think briefly; avoid repeating yourself.".to_string(),
    }
}

/// Chat 客户端默认系统提示：允许正常 Markdown（含表格），不强制「不要空行」。
pub fn default_chat_system_prompt(language: &str, has_image: bool) -> String {
    match (language.starts_with("zh"), has_image) {
        (true, true) => "你是一位智能助手，可结合用户提供的图片作答。回答清晰、有条理；可使用 Markdown（表格、列表、代码块等，表格每行单独一行）。数学公式用 LaTeX（$...$ 或 $$...$$）。思考保持简洁。".to_string(),
        (true, false) => "你是一位智能助手。直接、清晰地回答用户问题；可使用 Markdown（表格、列表、代码块等，表格每行单独一行）。数学公式用 LaTeX（$...$ 或 $$...$$）。思考保持简洁。".to_string(),
        (_, true) => "You are a helpful assistant that can use images the user provides. Answer clearly; Markdown is welcome (tables, lists, code blocks—each table row on its own line). Use LaTeX ($...$ or $$...$$) for math. Think briefly.".to_string(),
        (_, false) => "You are a helpful assistant. Answer clearly and directly; Markdown is welcome (tables, lists, code blocks—each table row on its own line). Use LaTeX ($...$ or $$...$$) for math. Think briefly.".to_string(),
    }
}

/// 兼容旧调用：等同于 [`default_lens_system_prompt`]。
pub fn default_system_prompt(language: &str, has_image: bool) -> String {
    default_lens_system_prompt(language, has_image)
}

/**
 * Lens：关闭思考模式时附加到系统提示词末尾（含紧凑输出要求）。
 */
pub fn no_think_instruction(language: &str) -> &'static str {
    if language.starts_with("zh") {
        "\n\n严格要求：直接给出最终答案，不要输出任何思考过程、推理步骤或 <think> 内容。保持输出紧凑，不要输出空行。"
    } else {
        "\n\nStrict requirement: output only the final answer; do NOT include any thinking, reasoning steps, or <think> content. Keep output compact; do not output blank lines."
    }
}

/// Chat：关闭思考模式时的附加指令（不要求紧凑、不禁止空行）。
pub fn chat_no_think_instruction(language: &str) -> &'static str {
    if language.starts_with("zh") {
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
    if language.starts_with("zh") {
        "用户分享了这张截图，请结合其中的视觉信息来理解和回答：".to_string()
    } else {
        "The user shared this screenshot. Use the visual context to understand and answer:"
            .to_string()
    }
}

// ========== 默认值辅助函数 ==========

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_api_format() -> String {
    "openai_chat".to_string()
}

fn default_hotkey() -> String {
    "CommandOrControl+Alt+T".to_string()
}

fn default_screenshot_translation_hotkey() -> String {
    "CommandOrControl+Shift+A".to_string()
}

fn default_screenshot_translation_text_hotkey() -> String {
    "CommandOrControl+Shift+T".to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    // ===== normalize_hotkey =====

    #[test]
    fn normalize_hotkey_canonicalizes_aliases() {
        // 仅规范修饰键名（cmd/ctrl/opt/super/meta），按键名 case 透传
        assert_eq!(normalize_hotkey("cmd+shift+a"), "CommandOrControl+Shift+a");
        assert_eq!(normalize_hotkey("Command+Alt+T"), "CommandOrControl+Alt+T");
        assert_eq!(normalize_hotkey("ctrl+shift+G"), "Control+Shift+G");
        assert_eq!(normalize_hotkey("opt+space"), "Alt+space");
        assert_eq!(normalize_hotkey("option+x"), "Alt+x");
        assert_eq!(normalize_hotkey("super+L"), "Super+L");
        assert_eq!(normalize_hotkey("meta+L"), "Super+L");
    }

    #[test]
    fn normalize_hotkey_preserves_key_case() {
        // 按键名大小写不被改动（Tauri 全局快捷键大小写敏感）
        assert_eq!(normalize_hotkey("cmd+a"), "CommandOrControl+a");
        assert_eq!(normalize_hotkey("cmd+A"), "CommandOrControl+A");
    }

    #[test]
    fn normalize_hotkey_trims_whitespace() {
        assert_eq!(
            normalize_hotkey(" cmd + shift + a "),
            "CommandOrControl+Shift+a"
        );
    }

    #[test]
    fn normalize_hotkey_filters_empty_parts() {
        assert_eq!(normalize_hotkey("cmd++a"), "CommandOrControl+a");
        assert_eq!(normalize_hotkey("+cmd+a+"), "CommandOrControl+a");
    }

    #[test]
    fn normalize_hotkey_preserves_unknown_keys_verbatim() {
        // F1, Backspace 等键名直接透传，不做 case 转换
        assert_eq!(normalize_hotkey("cmd+F1"), "CommandOrControl+F1");
        assert_eq!(normalize_hotkey("ctrl+Backspace"), "Control+Backspace");
    }

    // ===== sanitize_settings =====

    #[test]
    fn sanitize_settings_clamps_retry_attempts() {
        let mut s = Settings::default();
        s.retry_attempts = 0;
        let s = sanitize_settings(s);
        assert!((1..=5).contains(&s.retry_attempts));

        let mut s = Settings::default();
        s.retry_attempts = 99;
        let s = sanitize_settings(s);
        assert!((1..=5).contains(&s.retry_attempts));
    }

    #[test]
    fn sanitize_settings_normalizes_hotkeys() {
        let mut s = Settings::default();
        s.hotkey = "cmd+alt+T".to_string();
        s.screenshot_translation.hotkey = "ctrl+shift+A".to_string();
        s.screenshot_translation.text_hotkey = "cmd+shift+T".to_string();
        s.lens.hotkey = "cmd+shift+G".to_string();
        let s = sanitize_settings(s);
        assert_eq!(s.hotkey, "CommandOrControl+Alt+T");
        assert_eq!(s.screenshot_translation.hotkey, "Control+Shift+A");
        assert_eq!(
            s.screenshot_translation.text_hotkey,
            "CommandOrControl+Shift+T"
        );
        assert_eq!(s.lens.hotkey, "CommandOrControl+Shift+G");
    }

    #[test]
    fn sanitize_settings_preserves_empty_hotkeys() {
        let mut s = Settings::default();
        s.hotkey = String::new();
        s.screenshot_translation.hotkey = String::new();
        s.screenshot_translation.text_hotkey = String::new();
        s.lens.hotkey = String::new();
        let s = sanitize_settings(s);
        assert_eq!(s.hotkey, "");
        assert_eq!(s.screenshot_translation.hotkey, "");
        assert_eq!(s.screenshot_translation.text_hotkey, "");
        assert_eq!(s.lens.hotkey, "");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn sanitize_settings_disables_system_ocr_on_unsupported_platforms() {
        let mut s = Settings::default();
        s.screenshot_translation.ocr_mode = Some(OcrMode::System);
        let s = sanitize_settings(s);
        assert_eq!(
            s.screenshot_translation.ocr_mode,
            Some(OcrMode::CloudVision)
        );
    }

    #[test]
    fn sanitize_settings_migrates_use_system_ocr_true_to_system_mode() {
        // 老版本数据：useSystemOcr=true 但没有 ocr_mode 字段
        let mut s = Settings::default();
        s.screenshot_translation.use_system_ocr = true;
        s.screenshot_translation.ocr_mode = None;
        let s = sanitize_settings(s);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(s.screenshot_translation.ocr_mode, Some(OcrMode::System));
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(
            s.screenshot_translation.ocr_mode,
            Some(OcrMode::CloudVision)
        );
    }

    #[test]
    fn sanitize_settings_migrates_use_system_ocr_false_to_cloud_vision_mode() {
        let mut s = Settings::default();
        s.screenshot_translation.use_system_ocr = false;
        s.screenshot_translation.ocr_mode = None;
        let s = sanitize_settings(s);
        assert_eq!(
            s.screenshot_translation.ocr_mode,
            Some(OcrMode::CloudVision)
        );
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn sanitize_settings_preserves_rapidocr_mode() {
        let mut s = Settings::default();
        s.screenshot_translation.ocr_mode = Some(OcrMode::RapidOcr);
        let s = sanitize_settings(s);
        assert_eq!(s.screenshot_translation.ocr_mode, Some(OcrMode::RapidOcr));
    }

    #[test]
    fn sanitize_settings_migrates_legacy_tesseract_to_rapidocr() {
        // 旧版本 settings.json 含 "ocrMode": "tesseract"——序列化后落到 OcrMode::Legacy
        // 兜底变体,sanitize_settings 把它迁移到 RapidOcr,避免从本地 OCR 静默变成云端视觉。
        let json = r#"{"ocrMode":"tesseract"}"#;
        let cfg: ScreenshotTranslationConfig =
            serde_json::from_str(json).expect("legacy variant should deserialize");
        assert_eq!(cfg.ocr_mode, Some(OcrMode::Legacy));

        let mut s = Settings::default();
        s.screenshot_translation.ocr_mode = Some(OcrMode::Legacy);
        let s = sanitize_settings(s);
        assert_eq!(s.screenshot_translation.ocr_mode, Some(OcrMode::RapidOcr));
    }

    #[test]
    fn ocr_mode_serializes_with_snake_case() {
        // ocrMode 在 settings.json 里是 snake_case 字符串(cloud_vision / system / rapid_ocr)。
        // 前端 union type 'cloud_vision' | 'system' | 'rapid_ocr' 直接对齐。
        let modes = [
            (OcrMode::CloudVision, "\"cloud_vision\""),
            (OcrMode::System, "\"system\""),
            (OcrMode::RapidOcr, "\"rapid_ocr\""),
        ];
        for (mode, expected) in modes {
            assert_eq!(serde_json::to_string(&mode).unwrap(), expected);
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn sanitize_settings_unselects_apple_provider_on_non_macos() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "apple".to_string(),
            name: "Apple Intelligence".to_string(),
            api_keys: vec!["__on_device__".to_string()],
            api_key_legacy: None,
            base_url: APPLE_INTELLIGENCE_BASE_URL.to_string(),
            available_models: vec![],
            enabled_models: vec!["apple-foundation".to_string()],
            supports_tools: false,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "cloud".to_string(),
            name: "Cloud".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4o".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "apple".to_string();
        s.translator_model = "apple-foundation".to_string();
        s.screenshot_translation.provider_id = "apple".to_string();
        s.screenshot_translation.model = "apple-foundation".to_string();
        s.lens.provider_id = "apple".to_string();
        s.lens.model = "apple-foundation".to_string();

        let s = sanitize_settings(s);
        assert_eq!(s.translator_provider_id, "cloud");
        assert_eq!(s.screenshot_translation.provider_id, "cloud");
        assert_eq!(s.lens.provider_id, "");
        assert_eq!(
            s.providers
                .iter()
                .find(|provider| provider.id == "apple")
                .map(|provider| provider.supports_tools),
            Some(false),
        );
    }

    #[test]
    fn sanitize_settings_forces_apple_provider_tools_off() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "apple".to_string(),
            name: "Apple Intelligence".to_string(),
            api_keys: vec!["__on_device__".to_string()],
            api_key_legacy: None,
            base_url: APPLE_INTELLIGENCE_BASE_URL.to_string(),
            available_models: vec![],
            enabled_models: vec!["apple-foundation".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });

        let s = sanitize_settings(s);
        assert_eq!(
            s.providers
                .iter()
                .find(|provider| provider.id == "apple")
                .map(|provider| provider.supports_tools),
            Some(false),
        );
    }

    #[test]
    fn sanitize_settings_migrates_legacy_apikey_to_apikeys() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec![],
            api_key_legacy: Some("sk-legacy".to_string()),
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(p.api_keys, vec!["sk-legacy".to_string()]);
        assert!(p.api_key_legacy.is_none(), "legacy field should be drained");
    }

    #[test]
    fn sanitize_settings_dedupes_apikey_legacy_against_apikeys() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["sk-1".to_string(), "sk-2".to_string()],
            api_key_legacy: Some("sk-1".to_string()), // 已在 api_keys 中
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(
            p.api_keys.len(),
            2,
            "duplicate legacy key should not be inserted"
        );
    }

    #[test]
    fn sanitize_settings_filters_empty_apikeys() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["sk-1".to_string(), "  ".to_string(), String::new()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(p.api_keys, vec!["sk-1".to_string()]);
    }

    #[test]
    fn chat_tools_default_max_rounds_is_ten() {
        assert_eq!(ChatToolsConfig::default().max_tool_rounds, 10);

        let cfg: ChatToolsConfig =
            serde_json::from_str("{}").expect("empty chat tools config should load");
        assert_eq!(cfg.max_tool_rounds, 10);
    }

    #[test]
    fn sanitize_settings_keeps_empty_models_for_unfetched_provider() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec![],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "p".to_string();
        s.screenshot_translation.provider_id = "p".to_string();

        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert!(p.available_models.is_empty());
        assert!(p.enabled_models.is_empty());
        assert!(s.translator_model.is_empty());
        assert!(s.screenshot_translation.model.is_empty());
    }

    #[test]
    fn sanitize_settings_defaults_chat_to_lens_then_translator() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "translator".to_string(),
            name: "Translator".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4o".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "lens".to_string(),
            name: "Lens".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["vision-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "translator".to_string();
        s.translator_model = "gpt-4o".to_string();
        s.lens.provider_id = "lens".to_string();
        s.lens.model = "vision-model".to_string();

        let s = sanitize_settings(s);
        assert_eq!(s.chat_provider_id, "lens");
        assert_eq!(s.chat_model, "vision-model");
        assert!(
            s.default_models.chat.provider_id.is_empty(),
            "Lens fallback should not become an explicit Chat default slot"
        );
    }

    #[test]
    fn unset_auxiliary_models_inherit_effective_chat_model() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "translator".to_string(),
            name: "Translator".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4o".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "lens".to_string(),
            name: "Lens".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["vision-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "translator".to_string();
        s.translator_model = "gpt-4o".to_string();
        s.lens.provider_id = "lens".to_string();
        s.lens.model = "vision-model".to_string();

        let s = sanitize_settings(s);

        assert_eq!(
            s.effective_chat_model(),
            ("lens".to_string(), "vision-model".to_string())
        );
        assert!(!s.has_explicit_vision_model());
        assert_eq!(s.effective_vision_model(), s.effective_chat_model());
        assert_eq!(s.effective_title_summary_model(), s.effective_chat_model());
        assert_eq!(s.effective_compression_model(), s.effective_chat_model());
        assert!(s.default_models.vision.provider_id.is_empty());
        assert!(s.default_models.title_summary.provider_id.is_empty());
        assert!(s.default_models.compression.provider_id.is_empty());
    }

    #[test]
    fn sanitize_settings_keeps_valid_chat_model() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "chat".to_string(),
            name: "Chat".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m1".to_string(), "m2".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.chat_provider_id = "chat".to_string();
        s.chat_model = "m2".to_string();

        let s = sanitize_settings(s);
        assert_eq!(s.chat_provider_id, "chat");
        assert_eq!(s.chat_model, "m2");
        assert_eq!(s.default_models.chat.provider_id, "chat");
        assert_eq!(s.default_models.chat.model, "m2");
    }

    #[test]
    fn explicit_default_model_slots_are_independent() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "chat".to_string(),
            name: "Chat".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["chat-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "vision".to_string(),
            name: "Vision".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["vision-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "title".to_string(),
            name: "Title".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["title-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "compression".to_string(),
            name: "Compression".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["compression-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "chat".to_string();
        s.translator_model = "chat-model".to_string();
        s.default_models.chat.provider_id = "chat".to_string();
        s.default_models.chat.model = "chat-model".to_string();
        s.default_models.vision.provider_id = "vision".to_string();
        s.default_models.vision.model = "vision-model".to_string();
        s.default_models.title_summary.provider_id = "title".to_string();
        s.default_models.title_summary.model = "title-model".to_string();
        s.default_models.compression.provider_id = "compression".to_string();
        s.default_models.compression.model = "compression-model".to_string();

        let s = sanitize_settings(s);

        assert_eq!(
            s.effective_chat_model(),
            ("chat".to_string(), "chat-model".to_string())
        );
        assert_eq!(
            s.effective_title_summary_model(),
            ("title".to_string(), "title-model".to_string())
        );
        assert!(s.has_explicit_vision_model());
        assert_eq!(
            s.effective_vision_model(),
            ("vision".to_string(), "vision-model".to_string())
        );
        assert_eq!(
            s.effective_compression_model(),
            ("compression".to_string(), "compression-model".to_string())
        );
    }

    #[test]
    fn sanitize_settings_repairs_invalid_default_model_slots() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "chat".to_string(),
            name: "Chat".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m1".to_string(), "m2".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "chat".to_string();
        s.translator_model = "m1".to_string();
        s.default_models.chat.provider_id = "chat".to_string();
        s.default_models.chat.model = "removed".to_string();
        s.default_models.vision.provider_id = "chat".to_string();
        s.default_models.vision.model = String::new();
        s.default_models.title_summary.provider_id = "deleted-provider".to_string();
        s.default_models.title_summary.model = "ghost".to_string();
        s.default_models.compression.provider_id = "chat".to_string();
        s.default_models.compression.model = String::new();

        let s = sanitize_settings(s);

        assert_eq!(s.default_models.chat.provider_id, "chat");
        assert_eq!(s.default_models.chat.model, "m1");
        assert_eq!(s.default_models.vision.provider_id, "chat");
        assert_eq!(s.default_models.vision.model, "m1");
        assert!(s.default_models.title_summary.provider_id.is_empty());
        assert!(s.default_models.title_summary.model.is_empty());
        assert_eq!(s.default_models.compression.provider_id, "chat");
        assert_eq!(s.default_models.compression.model, "m1");
        assert_eq!(s.chat_provider_id, "chat");
        assert_eq!(s.chat_model, "m1");
    }

    #[test]
    fn persistence_mirror_keeps_unset_chat_default_unset() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "translator".to_string(),
            name: "Translator".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4o".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "lens".to_string(),
            name: "Lens".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["vision-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "translator".to_string();
        s.translator_model = "gpt-4o".to_string();
        s.lens.provider_id = "lens".to_string();
        s.lens.model = "vision-model".to_string();

        let mut s = sanitize_settings(s);
        assert_eq!(s.chat_provider_id, "lens");
        assert_eq!(s.chat_model, "vision-model");
        assert!(s.default_models.chat.provider_id.is_empty());

        mirror_explicit_chat_default_for_persistence(&mut s);

        assert!(s.chat_provider_id.is_empty());
        assert!(s.chat_model.is_empty());
        assert!(s.default_models.chat.provider_id.is_empty());
    }

    #[test]
    fn default_models_serialize_as_structured_camel_case_settings() {
        let mut s = Settings::default();
        s.default_models.vision.provider_id = "vision-provider".to_string();
        s.default_models.vision.model = "vision-model".to_string();
        s.default_models.title_summary.provider_id = "title-provider".to_string();
        s.default_models.title_summary.model = "title-model".to_string();
        let value = serde_json::to_value(&s).expect("settings should serialize");

        assert_eq!(
            value["defaultModels"]["vision"]["providerId"],
            "vision-provider"
        );
        assert_eq!(value["defaultModels"]["vision"]["model"], "vision-model");
        assert_eq!(
            value["defaultModels"]["titleSummary"]["providerId"],
            "title-provider"
        );
        assert_eq!(
            value["defaultModels"]["titleSummary"]["model"],
            "title-model"
        );
        assert!(value["defaultModels"]["chat"]["providerId"]
            .as_str()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sanitize_settings_preserves_streamable_http_mcp_server() {
        let mut s = Settings::default();
        let mut headers = std::collections::HashMap::new();
        headers.insert(" Authorization ".to_string(), " Bearer token ".to_string());
        s.chat_tools.servers.push(ChatMcpServer {
            id: " http-server ".to_string(),
            name: " Remote ".to_string(),
            enabled: true,
            transport: "sse".to_string(),
            url: " https://example.com/mcp ".to_string(),
            command: " ignored ".to_string(),
            args: vec![" ".to_string(), "--unused".to_string()],
            env: std::collections::HashMap::new(),
            headers,
            cwd: None,
            enabled_tools: vec![" fetch ".to_string(), "".to_string()],
        });

        let s = sanitize_settings(s);
        let server = &s.chat_tools.servers[0];
        assert_eq!(server.id, "http-server");
        assert_eq!(server.name, "Remote");
        assert_eq!(server.transport, "streamable_http");
        assert_eq!(server.url, "https://example.com/mcp");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Bearer token"),
        );
        assert_eq!(server.enabled_tools, vec!["fetch".to_string()]);
    }

    #[test]
    fn sanitize_settings_resets_unknown_mcp_transport_to_stdio() {
        let mut s = Settings::default();
        s.chat_tools.servers.push(ChatMcpServer {
            id: "mcp-1".to_string(),
            name: "Local".to_string(),
            enabled: false,
            transport: "websocket".to_string(),
            url: String::new(),
            command: " npx ".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
        });

        let s = sanitize_settings(s);
        let server = &s.chat_tools.servers[0];
        assert_eq!(server.transport, "stdio");
        assert_eq!(server.command, "npx");
    }

    #[test]
    fn sanitize_settings_clamps_unknown_message_order() {
        let mut s = Settings::default();
        s.lens.message_order = "garbage".to_string();
        let s = sanitize_settings(s);
        assert_eq!(s.lens.message_order, "asc");
    }

    #[test]
    fn lens_capture_hint_defaults_to_enabled() {
        let s = Settings::default();
        assert!(s.lens.show_capture_hint);

        let cfg: LensConfig = serde_json::from_str("{}").expect("empty lens config should load");
        assert!(cfg.show_capture_hint);
    }

    #[test]
    fn lens_windows_freeze_frame_selection_defaults_to_disabled() {
        let s = Settings::default();
        assert!(!s.lens.windows_freeze_frame_selection);

        let cfg: LensConfig = serde_json::from_str("{}").expect("empty lens config should load");
        assert!(!cfg.windows_freeze_frame_selection);
    }

    #[test]
    fn sanitize_settings_resets_lens_provider_when_pointing_to_nonexistent() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "real".to_string(),
            name: "Real".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.lens.provider_id = "nonexistent".to_string();
        s.lens.model = "ghost-model".to_string();
        let s = sanitize_settings(s);
        // 不存在的 provider_id 应被清空 → fallback 到 translator provider/model
        assert_eq!(s.lens.provider_id, "");
        assert_eq!(s.lens.model, "");
    }

    #[test]
    fn sanitize_settings_reassigns_disabled_provider_selections() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "disabled".to_string(),
            name: "Disabled".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://disabled.example/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["off-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: false,
            model_overrides: std::collections::HashMap::new(),
        });
        s.providers.push(ModelProvider {
            id: "active".to_string(),
            name: "Active".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://active.example/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["live-model".to_string()],
            supports_tools: true,
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
        });
        s.translator_provider_id = "disabled".to_string();
        s.translator_model = "off-model".to_string();
        s.screenshot_translation.provider_id = "disabled".to_string();
        s.screenshot_translation.model = "off-model".to_string();
        s.lens.provider_id = "disabled".to_string();
        s.lens.model = "off-model".to_string();
        s.default_models.chat.provider_id = "disabled".to_string();
        s.default_models.chat.model = "off-model".to_string();

        let s = sanitize_settings(s);

        assert_eq!(s.translator_provider_id, "active");
        assert_eq!(s.translator_model, "live-model");
        assert_eq!(s.screenshot_translation.provider_id, "active");
        assert_eq!(s.screenshot_translation.model, "live-model");
        assert_eq!(s.lens.provider_id, "");
        assert_eq!(s.lens.model, "");
        assert_eq!(s.default_models.chat.provider_id, "");
        assert_eq!(s.default_models.chat.model, "");
    }

    #[test]
    fn chat_current_datetime_context_uses_local_clock() {
        let now = Local::now();
        let zh = chat_current_datetime_context("zh");
        assert!(zh.contains("系统时钟"));
        assert!(zh.contains(&format!("{}年", now.year())));
        let en = chat_current_datetime_context("en");
        assert!(en.contains("system clock"));
        assert!(en.contains(&format!("{}-", now.year())));
    }
}
