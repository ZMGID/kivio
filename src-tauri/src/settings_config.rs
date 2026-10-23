//! The original complete Settings schema, defaults and model-purpose resolution.
//! Persistence, migrations with side effects, and runtime integrations stay in the host.
use crate::settings::{
    ChatToolsConfig, ConnectorAuth, DefaultModelSelection, DefaultModelsConfig, ModelProvider,
    default_chat_max_output_tokens,
};
use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};

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
    /// 替换翻译独立热键（框选后在原位覆盖译文，固定 RapidOCR）。
    #[serde(default = "default_screenshot_translation_replace_hotkey")]
    pub replace_hotkey: String,
    #[serde(default = "default_true")]
    pub replace_enabled: bool,
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

    /// 快速翻译结果卡左右宽度（px）。截图翻译与选中文本翻译两种卡共用，保证宽度一致且可调。
    #[serde(default = "default_translate_card_width")]
    pub card_width: u32,
    /// 用平台本地 OCR 做文字识别，把识别出的文字喂给翻译模型（macOS Apple Vision / Windows OCR）。
    /// true → 系统 OCR + provider 文字翻译（provider 可是任意 OpenAI 兼容 endpoint）
    /// false → provider 必须是多模态模型，一次完成 OCR+翻译
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
    /// 截图(OCR/视觉)翻译自定义提示词。空 → 用内置截图模板。
    #[serde(default)]
    pub prompt: Option<String>,
    /// 选中文本翻译自定义提示词。空 → 用内置选中文本模板。独立于 `prompt`：
    /// 选中文本是干净结构化文本，与 OCR 噪声场景的提示词需求不同。
    #[serde(default)]
    pub text_prompt: Option<String>,
    /// 替换翻译自定义提示词（仅注入翻译规则块，JSON 输出契约固定）。空 → 用内置替换模板。
    #[serde(default)]
    pub replace_prompt: Option<String>,
    /// RapidOCR 模型档位:"standard"(默认,PP-OCRv5 mobile,速度优先) | "high"(PP-OCRv6 medium,精度优先)。
    /// 仅在 ocr_mode = RapidOcr 时生效;替换翻译（固定走 RapidOCR）跟随此字段。
    #[serde(default = "default_rapid_ocr_tier")]
    pub rapid_ocr_tier: String,
    // 旧版字段，用于迁移
    #[serde(skip_serializing_if = "Option::is_none")]
    pub openai: Option<OpenAIConfig>,
}

impl Default for ScreenshotTranslationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: default_screenshot_translation_hotkey(),
            text_hotkey: default_screenshot_translation_text_hotkey(),
            replace_hotkey: default_screenshot_translation_replace_hotkey(),
            replace_enabled: true,
            provider_id: "default-ocr".to_string(),
            model: "gpt-4o".to_string(),
            direct_translate: false,
            thinking_enabled: false,
            stream_enabled: true,
            keep_fullscreen_after_capture: true,
            card_width: default_translate_card_width(),
            use_system_ocr: false,
            ocr_mode: Some(OcrMode::CloudVision),
            prompt: None,
            text_prompt: None,
            replace_prompt: None,
            rapid_ocr_tier: default_rapid_ocr_tier(),
            openai: None,
        }
    }
}

/// RapidOCR 档位默认值,截图翻译用(要速度)。
pub(crate) fn default_rapid_ocr_tier() -> String {
    "standard".to_string()
}

/// 知识库文档处理默认走高精度(v6 medium):入库不在乎慢,要识别质量。
pub(crate) fn default_rapid_ocr_tier_high() -> String {
    "high".to_string()
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
    ExaMcp,
    Ollama,
    Grok,
    Deepseek,
    Brave,
    Serper,
    Bocha,
    Zhipu,
    Tinyfish,
    TinyfishMcp,
    Searxng,
    Kimi,
    /// 前端可能列出尚未接入后端的占位服务商；持久化时兜底为未知，避免旧值导致整份设置解析失败。
    #[serde(other)]
    Unknown,
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
    /// Independent `web_fetch` provider. `None` follows `provider` (legacy configs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_provider: Option<WebSearchProvider>,
    #[serde(default)]
    pub tavily_api_key: String,
    #[serde(default = "default_tavily_base_url")]
    pub tavily_base_url: String,
    #[serde(default)]
    pub exa_api_key: String,
    #[serde(default = "default_exa_base_url")]
    pub exa_base_url: String,
    #[serde(default = "default_exa_mcp_url")]
    pub exa_mcp_url: String,
    #[serde(default)]
    pub ollama_api_key: String,
    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,
    #[serde(default)]
    pub grok_api_key: String,
    #[serde(default = "default_grok_model")]
    pub grok_model: String,
    #[serde(default = "default_grok_base_url")]
    pub grok_base_url: String,
    #[serde(default = "default_grok_system_prompt")]
    pub grok_system_prompt: String,
    #[serde(default)]
    pub deepseek_api_key: String,
    #[serde(default = "default_deepseek_base_url")]
    pub deepseek_base_url: String,
    #[serde(default)]
    pub brave_api_key: String,
    #[serde(default = "default_brave_base_url")]
    pub brave_base_url: String,
    #[serde(default)]
    pub serper_api_key: String,
    #[serde(default = "default_serper_base_url")]
    pub serper_base_url: String,
    #[serde(default)]
    pub bocha_api_key: String,
    #[serde(default = "default_bocha_base_url")]
    pub bocha_base_url: String,
    #[serde(default)]
    pub zhipu_api_key: String,
    #[serde(default = "default_zhipu_base_url")]
    pub zhipu_base_url: String,
    #[serde(default)]
    pub tinyfish_api_key: String,
    #[serde(default = "default_tinyfish_base_url")]
    pub tinyfish_base_url: String,
    #[serde(default = "default_tinyfish_mcp_url")]
    pub tinyfish_mcp_url: String,
    /// TinyFish MCP 走 OAuth 2.1，不贴 API Key。授权结果存在这里，搜索时带 Authorization。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tinyfish_mcp_auth: Option<ConnectorAuth>,
    #[serde(default)]
    pub searxng_base_url: String,
    #[serde(default)]
    pub kimi_api_key: String,
    #[serde(default = "default_kimi_base_url")]
    pub kimi_base_url: String,
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
            fetch_provider: None,
            tavily_api_key: String::new(),
            tavily_base_url: default_tavily_base_url(),
            exa_api_key: String::new(),
            exa_base_url: default_exa_base_url(),
            exa_mcp_url: default_exa_mcp_url(),
            ollama_api_key: String::new(),
            ollama_base_url: default_ollama_base_url(),
            grok_api_key: String::new(),
            grok_model: default_grok_model(),
            grok_base_url: default_grok_base_url(),
            grok_system_prompt: default_grok_system_prompt(),
            deepseek_api_key: String::new(),
            deepseek_base_url: default_deepseek_base_url(),
            brave_api_key: String::new(),
            brave_base_url: default_brave_base_url(),
            serper_api_key: String::new(),
            serper_base_url: default_serper_base_url(),
            bocha_api_key: String::new(),
            bocha_base_url: default_bocha_base_url(),
            zhipu_api_key: String::new(),
            zhipu_base_url: default_zhipu_base_url(),
            tinyfish_api_key: String::new(),
            tinyfish_base_url: default_tinyfish_base_url(),
            tinyfish_mcp_url: default_tinyfish_mcp_url(),
            tinyfish_mcp_auth: None,
            searxng_base_url: String::new(),
            kimi_api_key: String::new(),
            kimi_base_url: default_kimi_base_url(),
            max_results: default_web_search_max_results(),
            search_depth: default_web_search_depth(),
        }
    }
}

pub(crate) fn default_exa_mcp_url() -> String {
    "https://mcp.exa.ai/mcp".to_string()
}

fn default_tavily_base_url() -> String {
    "https://api.tavily.com".to_string()
}

fn default_exa_base_url() -> String {
    "https://api.exa.ai".to_string()
}

fn default_ollama_base_url() -> String {
    "https://ollama.com".to_string()
}

pub(crate) fn default_grok_model() -> String {
    "grok-4-1-fast-non-reasoning".to_string()
}

pub(crate) fn default_grok_base_url() -> String {
    "https://api.x.ai/v1".to_string()
}

pub(crate) fn default_deepseek_base_url() -> String {
    "https://api.deepseek.com".to_string()
}

fn default_brave_base_url() -> String {
    "https://api.search.brave.com".to_string()
}

fn default_serper_base_url() -> String {
    "https://google.serper.dev".to_string()
}

fn default_bocha_base_url() -> String {
    "https://api.bochaai.com".to_string()
}

fn default_zhipu_base_url() -> String {
    "https://open.bigmodel.cn/api/paas/v4".to_string()
}

fn default_tinyfish_base_url() -> String {
    "https://api.search.tinyfish.ai".to_string()
}

pub(crate) fn default_tinyfish_mcp_url() -> String {
    "https://agent.tinyfish.ai/mcp".to_string()
}

fn default_kimi_base_url() -> String {
    "https://api.kimi.com/coding/v1/search".to_string()
}

pub fn default_grok_system_prompt() -> String {
    "You are a helpful search assistant. Search the web to find accurate and up-to-date information for the user's query. Provide a comprehensive answer with citations."
        .to_string()
}

fn default_web_search_max_results() -> u8 {
    5
}

pub(crate) fn default_web_search_depth() -> String {
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
    /// Lens 提问默认发送到 AI 客户端。关闭后保留旧的 Lens 浮窗内回答。
    #[serde(default = "default_true")]
    pub send_to_chat: bool,
    /// 消息排序："asc" 老到新（默认），"desc" 新到老
    #[serde(default = "default_message_order")]
    pub message_order: String,
    /// 进入截图选择态时是否显示顶部提示。默认 true，避免用户按下快捷键后看不出已进入截图模式。
    #[serde(default = "default_true")]
    pub show_capture_hint: bool,
    #[serde(default)]
    pub web_search: LensWebSearchConfig,
}

fn default_message_order() -> String {
    "asc".to_string()
}

pub(crate) fn clamp_chat_max_output_tokens(_value: u32) -> u32 {
    // 对齐 Pi：maxTokens 写在模型目录上，没有用户档位。自定义模型没填时缺省 16384。
    // Kivio 有库用库；未收录一律发这个 16k，不按协议省略。
    default_chat_max_output_tokens()
}

impl Default for LensConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: default_lens_hotkey(),
            provider_id: String::new(),
            model: String::new(),
            default_language: String::new(),
            stream_enabled: true,
            thinking_enabled: true,
            system_prompt: String::new(),
            question_prompt: String::new(),
            send_to_chat: true,
            message_order: "asc".to_string(),
            show_capture_hint: true,
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
    /// Expose video analysis to the main agent. Never starts on attachment alone.
    #[serde(default = "default_true")]
    pub video_analysis_enabled: bool,
    #[serde(default = "default_true")]
    pub stream_enabled: bool,
    #[serde(default = "default_true")]
    pub thinking_enabled: bool,
    /// 未收录模型的输出上限兜底（对齐 Pi 自定义模型缺省 16384；有库/覆盖时用库值）。
    #[serde(default = "default_chat_max_output_tokens")]
    pub max_output_tokens: u32,
    /// 响应语言（"zh"/"en" 等）。空字符串表示跟随 Lens 默认语言，再跟随 target_lang。
    #[serde(default)]
    pub default_language: String,
    /// 自定义 system prompt；空则使用内置 Chat 模板（Kivio Agent 运行时）。
    #[serde(default)]
    pub system_prompt: String,
    /// 输入框「问题优化」的自定义系统提示词；空则使用内置优化提示词。
    #[serde(default)]
    pub prompt_optimize_prompt: String,
    /// Chat 侧栏显示的用户名；空则前端使用默认文案。
    #[serde(default)]
    pub user_display_name: String,
    /// 头像图片 URL 或 data URL；空则显示首字母占位头像。
    #[serde(default)]
    pub user_avatar: String,
    /// 新建对话默认 Agent 运行时（内置 loop 或外部 CLI）。
    #[serde(default)]
    pub default_agent_runtime: crate::chat::conversation_data::AgentRuntimeConfig,
    /// 本地 CLI Agent 的用户覆盖，key = agent id（claude/codex/…）。缺省 = 全默认。
    #[serde(default)]
    pub external_cli_agents: std::collections::HashMap<String, ExternalCliAgentConfig>,
    /// Kivio Chat 运行时专属设置（与 Kivio Agent 的工具/提示词分离）。
    #[serde(default)]
    pub chat_mode: ChatModeConfig,
}

/// Kivio Chat runtime settings — conversational tools + optional custom prompt.
/// Independent from Agent native-tool toggles (write/shell/skills stay Agent-only).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatModeConfig {
    /// Optional extra Chat instructions. Empty → no Chat identity essay;
    /// runtime still injects date plus conversation context (assistant / set /
    /// memory / knowledge base). File/shell limits are the tool filter, not prompt.
    pub system_prompt: String,
    pub web_search: bool,
    pub web_fetch: bool,
    pub knowledge_search: bool,
    /// `memory_read` / `memory_search` in Chat runtime.
    pub memory_tools: bool,
    /// Allow MCP tools that pass `is_read_only_tool()`.
    pub mcp_read_only: bool,
}

impl Default for ChatModeConfig {
    fn default() -> Self {
        Self {
            system_prompt: String::new(),
            web_search: true,
            web_fetch: true,
            knowledge_search: true,
            memory_tools: true,
            mcp_read_only: true,
        }
    }
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            video_analysis_enabled: true,
            stream_enabled: true,
            thinking_enabled: true,
            max_output_tokens: default_chat_max_output_tokens(),
            default_language: String::new(),
            system_prompt: String::new(),
            prompt_optimize_prompt: String::new(),
            user_display_name: String::new(),
            user_avatar: String::new(),
            default_agent_runtime: crate::chat::conversation_data::AgentRuntimeConfig::default(),
            external_cli_agents: std::collections::HashMap::new(),
            chat_mode: ChatModeConfig::default(),
        }
    }
}

/// 单个本地 CLI Agent 的用户覆盖（设置页「本地 CLI Agent」）。全字段可缺省 = 保持内置行为。
///
/// `PartialEq`：`persist_settings` 靠比较前后两份 `external_cli_agents` 决定要不要重新物化
/// 原生 CLI 配置（写用户家目录），不相等才写。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExternalCliAgentConfig {
    /// 停用后不出现在 Chat 的运行时选择器里。已绑定该 CLI 的旧会话不受影响——
    /// 一 agent 一对话，停用是「别再新建」而不是「把历史会话弄坏」。
    pub disabled: bool,
    /// 自定义可执行文件路径；空 = 走 PATH 探测。
    pub path: String,
    /// 注入该 CLI 子进程的环境变量（ANTHROPIC_BASE_URL / OPENAI_API_KEY 之类）。
    pub env: Vec<CliEnvVar>,
    /// 用户手填的模型，合并进探测出来的模型下拉。
    pub custom_models: Vec<CliCustomModel>,
    /// 该 CLI 的第三方供应商（中转站）列表。每个 CLI 各自一份，同 ccgui 的分桶方式。
    pub providers: Vec<ExternalCliProvider>,
    /// 当前默认供应商 id；空 = 使用 CLI 自己的默认配置。
    /// Pi / OpenCode / dsh 的 providers 会全部并存，此字段只决定未显式选模型时的默认项。
    pub current_provider: String,
}

/// 一个第三方供应商（中转站）。**各 CLI 用到的字段不同**：
/// - claude / gemini / 其余 env 系：只用 `env`（`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` …）；
///   claude 聊天选模按 cc-switch 写入 `~/.claude/settings.json` 的 `model` / `env.ANTHROPIC_MODEL`
/// - codex：只用 `config_toml` / `auth_json`，物化成一个私有 `CODEX_HOME`；聊天选模写入
///   `~/.codex/config.toml` 顶层 `model`（文件已存在时）以及 CLI 正在读的那份
/// - grok：只用 `config_toml`，把其中的 `[models]` / `[model.*]` 合并进 `~/.grok/config.toml`
/// - opencode / pi：用 `config_json` / `auth_json` / `default_model` 合并进 CLI 原生全局配置
/// - dsh：用 `config_json` 在 Kivio 私有 profile 中挂载 `llm-pi-ai`，Key 通过 `env` 注入
/// - pi：另用 `default_reasoning` 写入 `settings.json.defaultThinkingLevel`
///
/// 扁平结构而不是 tagged enum：settings.json 是用户可手改的文件，enum 的 tag 写错整条读不出来。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExternalCliProvider {
    /// 从 cc-switch 导入时**保留原 id**，这样二次导入走更新而不是新增一条重复的。
    pub id: String,
    pub name: String,
    /// 仅对支持并存的 CLI 生效；false = 启用（兼容旧配置），true = 不物化、不注入。
    pub disabled: bool,
    pub remark: String,
    pub env: Vec<CliEnvVar>,
    /// codex：私有 CODEX_HOME 里 config.toml 的全文；grok：写入原生 `~/.grok/config.toml` 的片段（至少含 models / model）。
    pub config_toml: String,
    /// opencode / pi：单个原生 provider 对象的 JSON（不含 provider id 外层）。
    pub config_json: String,
    /// codex：私有 CODEX_HOME 的完整 auth.json；opencode / pi：单个原生凭据对象。
    pub auth_json: String,
    /// Kivio 自用的模型覆盖状态；不写入 CLI 原生配置。
    pub model_metadata_json: String,
    /// opencode / pi / dsh：模型引用使用的稳定供应商 id（dsh wire 为 `provider:model`）。
    pub native_provider_id: String,
    /// opencode / pi / dsh：启用该供应商时使用的默认模型 id（不含 provider 前缀）。
    pub default_model: String,
    /// pi：终端独立启动时使用的默认 thinking 档位。
    pub default_reasoning: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CliEnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CliCustomModel {
    pub id: String,
    /// 显示名；空则前端回落显示 id。
    pub label: String,
}

/**
 * Chat 记忆系统配置。
 *
 * 记忆正文不存 settings.json；这里只保存运行开关。正文保存在 app data 的 chat-memory/L1.md
 * 与 chat-memory/L2.md 中。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatMemoryConfig {
    #[serde(default = "default_false")]
    pub enabled: bool,
    /// 已废弃：memory 工具现均无需用户确认；保留字段仅作旧配置兼容。
    #[serde(default = "default_false")]
    pub tool_write_confirm: bool,
}

impl Default for ChatMemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tool_write_confirm: false,
        }
    }
}

/// 当前 Chat 会话的主模型（顶栏选择），用于混音器 auto 时解析副任务路由。
#[derive(Debug, Clone, Copy)]
pub struct SessionModel<'a> {
    pub provider_id: &'a str,
    pub model: &'a str,
}

impl<'a> SessionModel<'a> {
    pub fn is_set(self) -> bool {
        !self.provider_id.trim().is_empty() && !self.model.trim().is_empty()
    }
}

fn resolve_mixer_side_model(
    selection: &DefaultModelSelection,
    session: Option<SessionModel<'_>>,
    settings: &Settings,
) -> (String, String) {
    if selection.is_configured() {
        return (selection.provider_id.clone(), selection.model.clone());
    }
    if let Some(session) = session.filter(|session| session.is_set()) {
        return (session.provider_id.to_string(), session.model.to_string());
    }
    settings.effective_chat_model()
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

/// 第三方文档解析服务（MinerU / Doc2X / LlamaParse / 自定义端点）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DocProcessorProvider {
    pub id: String,
    pub name: String,
    /// "mineru" | "doc2x" | "llamaparse" | "custom"
    pub kind: String,
    pub api_keys: Vec<String>,
    pub base_url: String,
    pub enabled: bool,
}

/// 知识库文档处理：Kivio 内置解析 + 可选第三方解析服务及路由策略。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DocumentProcessingConfig {
    /// 图片/可 OCR 内容用的引擎: "off"(默认) | "system" | "rapid_ocr"
    pub ocr_engine: String,
    /// RapidOCR 模型档位:"standard"(PP-OCRv5 mobile) | "high"(默认,PP-OCRv6 medium)。
    /// 知识库入库不在乎慢、要识别质量,故默认高精度。仅在 ocr_engine = "rapid_ocr" 时生效。
    #[serde(default = "default_rapid_ocr_tier_high")]
    pub rapid_ocr_tier: String,
    /// PDF 处理: "text"(默认,文字层) | "force_ocr"(扫描版重扫——内置未启用,会报错)
    pub pdf_strategy: String,
    /// "" = Kivio 内置（本地 Rust）；否则为某第三方 provider id
    pub active_processor: String,
    /// 内置解析失败（如扫描版 PDF）时自动回退到第一个启用的第三方服务。
    pub fallback_to_third_party: bool,
    pub providers: Vec<DocProcessorProvider>,
}

impl Default for DocumentProcessingConfig {
    fn default() -> Self {
        Self {
            ocr_engine: "off".into(),
            rapid_ocr_tier: default_rapid_ocr_tier_high(),
            pdf_strategy: "text".into(),
            active_processor: String::new(),
            fallback_to_third_party: false,
            providers: Vec::new(),
        }
    }
}

/// 知识库检索配置：hybrid(向量+关键词 RRF) 权重 + 可选全局 rerank。
/// 只配 embedding 即可用：hybrid 免配可关，rerank 留空即关、失败降级。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KnowledgeBaseConfig {
    /// 是否启用关键词(BM25)+向量 hybrid 融合（关掉=纯向量）。
    pub hybrid_enabled: bool,
    /// RRF 融合权重（hybrid 开启时生效）。
    pub weight_vector: f32,
    pub weight_keyword: f32,
    /// 全局 rerank：留空即关闭。provider 引用 providers[]，model 为该 provider 的 rerank 模型。
    pub rerank_provider_id: String,
    pub rerank_model: String,
    /// 入库分块目标 tokens（使用处 clamp 到 256..=8192；只影响新导入/重建）。
    pub chunk_tokens: u32,
    /// knowledge_search 默认返回片段数（使用处 clamp 到 1..=20；工具入参可覆盖）。
    /// 这是最终进入上下文的片段数（contextTopK）。
    pub top_k: u32,
    /// 每库融合候选池大小（clamp 20..=200）。召回→融合的候选数，越大召回越全、
    /// 本地检索成本略增；不影响送 rerank 的数量。
    pub candidate_k: u32,
    /// 送 rerank 的候选数（clamp 5..=50）。只在 rerank 开启时生效，直接决定
    /// rerank 网络调用的文档数。
    pub rerank_top_k: u32,
    /// 相关性阈值（D5，0..=1；0 = 关闭，保守默认不误杀）。rerank 开启时对齐
    /// rerank relevance 分数；关闭时为向量-only 命中的余弦相似度下限（词法命中恒过）。
    pub min_score: f32,
}

impl Default for KnowledgeBaseConfig {
    fn default() -> Self {
        Self {
            hybrid_enabled: true,
            weight_vector: 1.0,
            weight_keyword: 1.0,
            rerank_provider_id: String::new(),
            rerank_model: String::new(),
            chunk_tokens: 480,
            top_k: 5,
            candidate_k: 60,
            rerank_top_k: 20,
            min_score: 0.0,
        }
    }
}

impl KnowledgeBaseConfig {
    /// Per-library fused candidate pool size, clamped to safe bounds.
    pub fn candidate_k_clamped(&self) -> usize {
        (self.candidate_k as usize).clamp(20, 200)
    }
    /// How many top candidates to send to the reranker, clamped to safe bounds.
    pub fn rerank_top_k_clamped(&self) -> usize {
        (self.rerank_top_k as usize).clamp(5, 50)
    }
}

/**
 * 独立截图标注功能配置（截图 → 箭头/矩形/马赛克标注 → 复制/保存）
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScreenshotAnnotateConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_screenshot_annotate_hotkey")]
    pub hotkey: String,
}

fn default_screenshot_annotate_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Shift+S".to_string()
}

impl Default for ScreenshotAnnotateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: default_screenshot_annotate_hotkey(),
        }
    }
}

pub(crate) const BUILTIN_THEME_IDS: &[&str] = &[
    "neutral",
    "warm",
    "cool",
    "graphite",
    "blossom",
    "grove",
    "ocean",
    "ember",
    "iris",
    "white",
    "nord",
    "solarized",
];
/// `crypto.randomUUID()` is 36 ASCII characters: 8-4-4-4-12 hex.
const CUSTOM_THEME_ID_LEN: usize = 36;
/// Trimmed name length, counted in UTF-16 code units to match the UI
/// `name.trim().length` / `<input maxLength={80}>` check of 1..=80.
const CUSTOM_THEME_NAME_MAX_UTF16: usize = 80;

/// 自定义主题的一套语义色。十七个字段都只能是 `#rrggbb`。
/// 短色、函数、空白或任何能逃出 CSS 声明的片段都会让整条主题被丢弃。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ThemePalette {
    pub surface: String,
    pub surface_soft: String,
    pub surface_muted: String,
    pub surface_hover: String,
    pub surface_active: String,
    pub surface_titlebar: String,
    pub border: String,
    pub border_strong: String,
    pub text: String,
    pub text_muted: String,
    pub text_faint: String,
    pub accent: String,
    pub accent_hover: String,
    pub accent_soft: String,
    pub on_accent: String,
    pub danger: String,
    pub danger_soft: String,
}

/// 用户保存的主题。`id` 是 `themeColor` 可以指向的选择值，不能占用内置 id。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ThemeDefinition {
    pub id: String,
    pub name: String,
    pub light: ThemePalette,
    pub dark: ThemePalette,
}

fn canonical_hex_color(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return None;
    }
    if !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn canonical_palette(palette: &ThemePalette) -> Option<ThemePalette> {
    Some(ThemePalette {
        surface: canonical_hex_color(&palette.surface)?,
        surface_soft: canonical_hex_color(&palette.surface_soft)?,
        surface_muted: canonical_hex_color(&palette.surface_muted)?,
        surface_hover: canonical_hex_color(&palette.surface_hover)?,
        surface_active: canonical_hex_color(&palette.surface_active)?,
        surface_titlebar: canonical_hex_color(&palette.surface_titlebar)?,
        border: canonical_hex_color(&palette.border)?,
        border_strong: canonical_hex_color(&palette.border_strong)?,
        text: canonical_hex_color(&palette.text)?,
        text_muted: canonical_hex_color(&palette.text_muted)?,
        text_faint: canonical_hex_color(&palette.text_faint)?,
        accent: canonical_hex_color(&palette.accent)?,
        accent_hover: canonical_hex_color(&palette.accent_hover)?,
        accent_soft: canonical_hex_color(&palette.accent_soft)?,
        on_accent: canonical_hex_color(&palette.on_accent)?,
        danger: canonical_hex_color(&palette.danger)?,
        danger_soft: canonical_hex_color(&palette.danger_soft)?,
    })
}

/// 自定义 id 只接受 `crypto.randomUUID()` 的原始 36 字符。
/// trim 后大小写不敏感，落盘为小写；长度不是 36 或不是 8-4-4-4-12 十六进制的一律拒绝。
/// 内置 id 不是 UUID，因此不可能被自定义主题占用。
fn canonical_custom_theme_id(raw: &str) -> Option<String> {
    let id = raw.trim().to_ascii_lowercase();
    let bytes = id.as_bytes();
    if bytes.len() != CUSTOM_THEME_ID_LEN {
        return None;
    }
    let hex = |range: std::ops::Range<usize>| bytes[range].iter().all(u8::is_ascii_hexdigit);
    if bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && hex(0..8)
        && hex(9..13)
        && hex(14..18)
        && hex(19..23)
        && hex(24..36)
    {
        Some(id)
    } else {
        None
    }
}

/// 名称只按 UI 规则：trim 后 UTF-16 长度 1..=80。不再额外拒绝控制字符，
/// 否则一次合法保存会在规范化里被丢掉，调用方仍收到成功。
fn canonical_theme_name(raw: &str) -> Option<String> {
    let name = raw.trim();
    let len = name.encode_utf16().count();
    if (1..=CUSTOM_THEME_NAME_MAX_UTF16).contains(&len) {
        Some(name.to_string())
    } else {
        None
    }
}

/// 缺键走 `default`；显式 `null`、非数组、或单条无法反序列化的元素都变成跳过，
/// 避免一条坏主题让整个 settings.json 解析失败从而丢掉其它设置。
fn deserialize_custom_themes<'de, D>(deserializer: D) -> Result<Vec<ThemeDefinition>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(Vec::new());
    };
    let serde_json::Value::Array(items) = value else {
        return Ok(Vec::new());
    };
    Ok(items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect())
}

pub(crate) fn sanitize_custom_themes(themes: &mut Vec<ThemeDefinition>) {
    // 没有条数上限。重复 id 保留最后一条：设置页保存时把正在编辑的主题追加在末尾，
    // 丢掉后者会让这次保存看起来成功，实际写回的仍是旧主题。
    let mut accepted: Vec<ThemeDefinition> = Vec::with_capacity(themes.len());
    for theme in themes.drain(..) {
        let Some(id) = canonical_custom_theme_id(&theme.id) else {
            continue;
        };
        let Some(name) = canonical_theme_name(&theme.name) else {
            continue;
        };
        let Some(light) = canonical_palette(&theme.light) else {
            continue;
        };
        let Some(dark) = canonical_palette(&theme.dark) else {
            continue;
        };
        if let Some(index) = accepted.iter().position(|item| item.id == id) {
            accepted.remove(index);
        }
        accepted.push(ThemeDefinition {
            id,
            name,
            light,
            dark,
        });
    }
    *themes = accepted;
}

pub(crate) fn canonicalize_theme_color(raw: &str, themes: &[ThemeDefinition]) -> String {
    let trimmed = raw.trim();
    if let Some(builtin) = BUILTIN_THEME_IDS
        .iter()
        .copied()
        .find(|id| trimmed.eq_ignore_ascii_case(id))
    {
        return builtin.to_string();
    }
    if let Some(theme) = themes
        .iter()
        .find(|theme| theme.id.eq_ignore_ascii_case(trimmed))
    {
        return theme.id.clone();
    }
    default_theme_color()
}

/**
 * 应用完整设置
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// 打开 AI 客户端（chat 窗口）的全局热键。
    #[serde(default = "default_chat_hotkey")]
    pub chat_hotkey: String,
    /// 关闭 AI 客户端（chat 窗口）的全局热键。
    #[serde(default = "default_close_chat_hotkey")]
    pub close_chat_hotkey: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_theme_color")]
    pub theme_color: String,
    /// 用户自定义主题。缺键或 null 都是空数组，旧 settings 的其它字段不受影响。
    /// 保存时总是写出完整数组。非法条目在 `sanitize_settings` 里整条丢弃。
    #[serde(default, deserialize_with = "deserialize_custom_themes")]
    pub custom_themes: Vec<ThemeDefinition>,
    #[serde(default = "default_false")]
    pub translucent_sidebar: bool,
    /// UI 整体缩放（作用于聊天窗口根元素 zoom），范围 0.8–1.4，1.0 为默认。
    #[serde(default = "default_ui_font_scale")]
    pub ui_font_scale: f32,
    /// 自定义 UI 字体名（系统已装字体，拼到字体栈最前）。空串 = 系统默认。
    #[serde(default)]
    pub ui_font_family: String,
    /// 自定义代码/等宽字体名（作用于 --font-mono）。空串 = 系统默认。
    #[serde(default)]
    pub ui_font_mono: String,
    #[serde(default = "default_target_lang")]
    pub target_lang: String,
    /// 阅读目标语言：划词、截图与替换翻译共用；缺失时从旧目标语言迁移。
    #[serde(default)]
    pub reading_target_lang: String,
    #[serde(default = "default_true")]
    pub auto_paste: bool,
    #[serde(default = "default_false")]
    pub launch_at_startup: bool,
    /// 启动后不打开聊天窗口，进程留在托盘。开机自启用户常开这个，避免每次开机弹出界面。
    /// `--from-autostart` 仍会单独跳过弹窗（插件参数偶发丢失时靠本开关兜底）。
    #[serde(default = "default_false")]
    pub launch_minimized_to_tray: bool,
    /// 关闭聊天窗口时隐藏复用（默认 false = 销毁 WebView 回收内存）。
    /// 高频开关 / 低配机可避免每次冷创建的启动延迟和风扇起来。
    #[serde(default = "default_false")]
    pub keep_chat_window_alive: bool,
    /// 回复完成时发送系统通知；默认关闭，开启后正在查看该对话时不提醒。
    #[serde(default = "default_false")]
    pub chat_completion_notifications: bool,
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
    #[serde(default)]
    pub screenshot_annotate: ScreenshotAnnotateConfig,
    #[serde(default, alias = "cowork")]
    pub lens: LensConfig,
    #[serde(default)]
    pub chat: ChatConfig,
    #[serde(default)]
    pub chat_memory: ChatMemoryConfig,
    #[serde(default)]
    pub chat_tools: ChatToolsConfig,
    #[serde(default)]
    pub document_processing: DocumentProcessingConfig,
    #[serde(default)]
    pub knowledge_base: KnowledgeBaseConfig,
    /// 供应商自定义图标：provider id → 图标 key（前端 PROVIDER_BRANDS 的键）。
    /// ponytail: 单独一张表而不是 ModelProvider 上加字段——那个结构体有 50 处字面量构造，
    /// 而且只有设置界面关心图标。
    #[serde(default)]
    pub provider_icons: std::collections::HashMap<String, String>,
    /// 一次性：将 Lens 的流式/思考开关复制到独立的 Chat 配置（旧版共用 Lens 行为）。
    #[serde(default)]
    pub chat_behavior_migrated_from_lens: bool,
    #[serde(default = "default_settings_language")]
    pub settings_language: Option<String>,
    #[serde(default = "default_retry_enabled")]
    pub retry_enabled: bool,
    /// Total attempts, including the first request. UI shows this minus one.
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: u8,
    /// 一次性迁移标记：内置专家（写作/编程/研究/数据）已 seed 进 assistants.json 后置 true。
    /// 该迁移会清空整个助手索引（含用户自建——用户明确选择）再装入这 4 个内置专家，
    /// 仅在首次启动跑一次；之后用户新建/删除专家不受影响。
    #[serde(default)]
    pub builtin_assistants_seeded_v1: bool,
    /// 一次性迁移标记（v2，非破坏性）：按 id upsert 新一批内置专家（升级 4 个 + 新增前端/翻译/文档），
    /// 更新旧内置、补齐新增，**保留用户自建**。已 seed v1 的老用户靠它拿到新专家；置 true 后不再跑。
    #[serde(default)]
    pub builtin_assistants_seeded_v2: bool,
    /// 一次性迁移标记（v3，非破坏性）：按 id upsert 补齐产品/法务/财务/教学/审查/求职。
    /// 已 seed v2 的老用户靠它拿到新专家；置 true 后不再跑。
    #[serde(default)]
    pub builtin_assistants_seeded_v3: bool,
    /// 一次性迁移标记：把 pre-green-light 安装（原生工具默认全关 + 旧 approval_policy）
    /// 带到新默认——原生文件/命令工具置 true，不改变已有审批策略（包括用户显式选择的 auto）。
    /// 幂等：置 true 后不再翻转，尊重用户此后手动关闭某工具或改 policy 的选择。
    #[serde(default)]
    pub chat_tools_greenlit_v1: bool,
    /// 首次使用引导状态：`pending` | `completed` | `skipped`。
    /// 缺省为空字符串：老版本无此字段时由 `normalize_onboarding_status` 按是否已有 provider 决定。
    #[serde(default)]
    pub onboarding_status: String,
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
    /// 收藏并置顶的模型键（"providerId:model"）；列表顺序即置顶顺序。
    /// 只在 chat 模型选择器里展示为顶部"收藏"组；失效项（provider 删/禁用/模型没了）展示时过滤。
    #[serde(default)]
    pub favorite_models: Vec<String>,
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

    pub fn effective_title_summary_model_for_session(
        &self,
        session: Option<SessionModel<'_>>,
    ) -> (String, String) {
        resolve_mixer_side_model(&self.default_models.title_summary, session, self)
    }

    pub fn has_explicit_vision_model(&self) -> bool {
        self.default_models.vision.is_configured()
    }

    pub fn effective_vision_model(&self) -> (String, String) {
        self.effective_vision_model_for_session(None)
    }

    pub fn effective_vision_model_for_session(
        &self,
        session: Option<SessionModel<'_>>,
    ) -> (String, String) {
        resolve_mixer_side_model(&self.default_models.vision, session, self)
    }

    pub fn effective_compression_model_for_session(
        &self,
        session: Option<SessionModel<'_>>,
    ) -> (String, String) {
        resolve_mixer_side_model(&self.default_models.compression, session, self)
    }

    pub fn effective_prompt_optimize_model_for_session(
        &self,
        session: Option<SessionModel<'_>>,
    ) -> (String, String) {
        resolve_mixer_side_model(&self.default_models.prompt_optimize, session, self)
    }

    pub fn image_generation_model(&self) -> Option<(String, String)> {
        if self.default_models.image_generation.is_configured()
            && !self.default_models.image_generation.model.trim().is_empty()
        {
            Some((
                self.default_models.image_generation.provider_id.clone(),
                self.default_models.image_generation.model.clone(),
            ))
        } else {
            None
        }
    }

    /// Advisor model (executor-advisor pattern): the `advisor` tool is exposed
    /// only when both provider and model are set. No inheritance — blank = off.
    pub fn advisor_model(&self) -> Option<(String, String)> {
        if self.default_models.advisor.is_configured()
            && !self.default_models.advisor.model.trim().is_empty()
        {
            Some((
                self.default_models.advisor.provider_id.clone(),
                self.default_models.advisor.model.clone(),
            ))
        } else {
            None
        }
    }

    /// Extra OAuth tokens stored outside `chat_tools.servers`, keyed by MCP resource URL.
    /// The MCP manager persists a refreshed token to every slot bound to that URL
    /// without knowing each product name.
    pub(crate) fn detached_oauth_auth_for_url_mut(
        &mut self,
        resource_url: &str,
    ) -> Option<&mut Option<ConnectorAuth>> {
        fn url_matches(left: &str, right: &str) -> bool {
            left.trim().trim_end_matches('/') == right.trim().trim_end_matches('/')
        }
        if url_matches(self.lens.web_search.tinyfish_mcp_url.trim(), resource_url) {
            return Some(&mut self.lens.web_search.tinyfish_mcp_auth);
        }
        None
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            chat_hotkey: default_chat_hotkey(),
            close_chat_hotkey: default_close_chat_hotkey(),
            theme: "system".to_string(),
            theme_color: default_theme_color(),
            custom_themes: Vec::new(),
            translucent_sidebar: false,
            ui_font_scale: default_ui_font_scale(),
            ui_font_family: String::new(),
            ui_font_mono: String::new(),
            target_lang: "auto".to_string(),
            reading_target_lang: "zh".to_string(),
            auto_paste: true,
            launch_at_startup: false,
            launch_minimized_to_tray: false,
            keep_chat_window_alive: false,
            chat_completion_notifications: false,
            translator_provider_id: "default-translator".to_string(),
            translator_model: "gpt-4o".to_string(),
            chat_provider_id: String::new(),
            chat_model: String::new(),
            default_models: DefaultModelsConfig::default(),
            translator_prompt: None,
            providers: vec![],
            screenshot_translation: ScreenshotTranslationConfig::default(),
            screenshot_annotate: ScreenshotAnnotateConfig::default(),
            lens: LensConfig::default(),
            chat: ChatConfig::default(),
            chat_memory: ChatMemoryConfig::default(),
            chat_tools: ChatToolsConfig::default(),
            document_processing: DocumentProcessingConfig::default(),
            knowledge_base: KnowledgeBaseConfig::default(),
            provider_icons: std::collections::HashMap::new(),
            chat_behavior_migrated_from_lens: false,
            settings_language: Some("zh".to_string()),
            retry_enabled: default_retry_enabled(),
            retry_attempts: default_retry_attempts(),
            builtin_assistants_seeded_v1: false,
            builtin_assistants_seeded_v2: false,
            builtin_assistants_seeded_v3: false,
            chat_tools_greenlit_v1: false,
            onboarding_status: default_onboarding_status(),
            auto_check_update: !crate::app_data::is_dev(),
            image_archive_enabled: false,
            image_archive_path: String::new(),
            favorite_models: Vec::new(),
            openai: None,
        }
    }
}

// ========== 默认提示词生成 ==========

/**
 * 获取默认系统提示词
 * has_image=true 时为视觉助手；为 false 时为通用对话助手（不假设有图片）
 * 风格统一：简短直答、无小标题、思考过程尽量精简
 */
/// Local system date for Chat date questions. Models must not guess dates from training data.
/// 只到日期级、不含时分：系统提示词是每轮请求的公共前缀，分钟级时钟会让同一对话每轮前缀
/// 都变——打穿 provider 的 prompt cache（前缀匹配），也让会话亲和型代理无法续会话。
/// 回答"今天/明天/星期几"日期粒度已足够。
pub fn chat_current_datetime_context(language: &str) -> String {
    let now = Local::now();
    let weekday = weekday_label(language, now.weekday());
    if language.starts_with("zh") {
        format!(
            "\n\n当前日期（系统时钟；回答今天/明天/星期几等日期问题必须以此为准，禁止凭记忆臆测）：{}年{}月{}日 {}。",
            now.year(),
            now.month(),
            now.day(),
            weekday
        )
    } else {
        format!(
            "\n\nToday's date (system clock; use for today/tomorrow/weekday questions—never guess from training data): {}-{:02}-{:02} {}.",
            now.year(),
            now.month(),
            now.day(),
            weekday
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

/// 内置 Agent 的完整默认提示词。运行时只补充能力、模式和上下文。
/// 用户自定义系统提示词替换此正文；应用信任边界仍由运行时统一提供。
pub fn default_chat_system_prompt(has_image: bool) -> String {
    let mut prompt = r#"You are the AI assistant inside Kivio. Help the user understand problems, make decisions, and complete their requests using the capabilities available in this conversation.

How you work:
Answer questions directly. For action requests, carry the work through to completion, including necessary supporting steps within the requested scope. Use available context to make reasonable routine decisions; ask a focused question when missing information materially affects the outcome. Incorporate the user's answers and corrections into the ongoing task; follow explicit requests to change the objective or stop.

Use the tools declared in this conversation when they help complete the task, following their descriptions and schemas. Ground action and completion claims in their results. During multi-step work, give brief updates about useful findings, obstacles, or changes of direction. Carry out independent steps while waiting for results needed elsewhere.

When editing files, inspect existing content and follow its conventions. Preserve the user's work, keep temporary files separate, and clean up only your own disposable files. Commit or push when the user requests it. Use absolute paths when passing local files to MCP tools.

Handle failures by using the error and current state to adjust your approach. Retry when a changed approach or new evidence gives a reason to expect progress. When blocked, explain what is complete, the concrete obstacle, and what is needed to continue.

Verify the deliverable against the request with checks proportional to the change and its risks. Fix issues found and recheck affected results. Once the requested outcome is achieved and checked, deliver it; present optional improvements as suggestions.

Response style:
Answer in the user's language with clear, natural prose and detail proportional to the task. Lead with the main point and develop the information the user needs. Prefer paragraphs; use lists for parallel items or steps, tables for comparisons, and headings for longer answers. Use emphasis sparingly and keep Markdown well formed. Use backticks for code and identifiers, LaTeX for math, and currency names or codes for amounts. Follow the user's requested format.

Make the final answer self-contained, even after clarification: lead with the result, then provide the information needed to understand, use, or continue the work. Keep internal reasoning in the dedicated reasoning channel.

Attachment presentation:
Choose placement by the attachment's purpose. When the user asks for an image, file, or other media as the main result, present it near the start of the final answer, with at most a brief introduction before it and any necessary usage notes after it. When an image or chart explains a particular point, place it immediately after the complete paragraph it supports. For reports, spreadsheets, archives, and other downloadable deliverables accompanying a written answer, finish the explanation first and group the files at the end. Follow any placement or ordering the user requests.

Place each attachment reference in its own paragraph, separated from surrounding text by blank lines, so previews and file cards have a clear place in the message. Keep explanatory sentences, lists, tables, and code blocks complete before inserting an attachment. For multiple attachments, arrange them in the order discussed and identify each with a short, meaningful name; distinguish versions or formats when useful. For visual comparisons, keep each option's label, image, and explanation together.

Present the selected result once per answer and refer to it by name elsewhere. Reuse the available attachment reference; use the relevant tool's documented syntax to render an image preview or file card. Include supporting files when they help the user use the result. Keep intermediate screenshots, extracted frames, drafts, and failed attempts in the working context unless the user needs to inspect them. Refer to user-uploaded attachments by name and present them again only when useful for the requested explanation or comparison.

Distinguish confirmed facts, inferences, and unverified points; state material uncertainty. Preserve source-provided citation formats and link web sources. Verify time-sensitive information with available tools; when verification is unavailable, state the information's time frame and uncertainty.

Instruction scope:
Work within application safety, tool availability, and the active mode. Follow the user's current request and apply project, assistant, set, and skill instructions to their relevant scope. Specific task requirements override general style defaults. If a material conflict remains, ask one focused question."#.to_string();
    if has_image {
        prompt.push_str("\n\nUse the user's images as visual context.");
    }
    prompt
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
pub fn chat_no_think_instruction() -> &'static str {
    "\n\nStrict requirement: output only the final answer; do NOT include any thinking, reasoning steps, or <think> content."
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

fn default_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Alt+T".to_string()
}

fn default_chat_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Shift+K".to_string()
}

fn default_close_chat_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Shift+W".to_string()
}

fn default_screenshot_translation_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Shift+A".to_string()
}

fn default_screenshot_translation_text_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Shift+T".to_string()
}

fn default_screenshot_translation_replace_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Shift+R".to_string()
}

/// 快速翻译结果卡默认宽度（px）。介于旧截图卡(~514)与选中文本卡(420)之间。
fn default_translate_card_width() -> u32 {
    480
}

fn default_lens_hotkey() -> String {
    if crate::app_data::is_dev() {
        return String::new();
    }
    "CommandOrControl+Shift+G".to_string()
}

pub(crate) fn default_theme() -> String {
    "system".to_string()
}

fn default_theme_color() -> String {
    "neutral".to_string()
}

pub(crate) fn default_ui_font_scale() -> f32 {
    1.0
}

fn default_target_lang() -> String {
    "auto".to_string()
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
    11 // First request + ten retries; persisted totals retain their original meaning.
}

fn default_retry_enabled() -> bool {
    true
}

fn default_onboarding_status() -> String {
    "pending".to_string()
}
