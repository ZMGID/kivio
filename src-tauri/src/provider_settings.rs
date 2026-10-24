//! Shared provider configuration, wire defaults and normalization.
//!
//! Desktop hosts deserialize the same model configuration and call sanitize_provider;
//! persistence, credential access and application-wide selection remain in their owners.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthConfig {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    /// Runtime-only account project, loaded from the local JSON credential store.
    #[serde(skip)]
    pub project_id: Option<String>,
}

/// 供应商级自定义请求头的一条。key/value 的合法性由 `provider_request` 校验，
/// 非法或命中保留名单的条目在 `sanitize_settings` 里丢弃（设置文件可被手改，后端必须自己拦）。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProviderCustomHeader {
    pub key: String,
    pub value: String,
}

/// 供应商级「请求配置」：自定义请求头 / 代理 / prompt 缓存 / CLI 身份伪装。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderRequestConfig {
    /// OAuth metadata only; secrets live in the local JSON credential store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthConfig>,
    /// 附加到该供应商所有请求上的自定义头。同名时覆盖 CLI 身份预设。
    pub custom_headers: Vec<ProviderCustomHeader>,
    /// 是否跟随系统代理。默认 true —— 与加这个开关之前的行为一致；关掉才走直连。
    pub use_system_proxy: bool,
    /// 遗留 on/off。sanitize 时迁移进 `prompt_cache_retention` 后清空，新配置不再写入。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_caching: Option<bool>,
    /// Prompt 缓存策略（对齐 pi `CacheRetention`）：
    /// - `none`：不发客户端缓存字段
    /// - `short`（默认）：OpenAI 发 `prompt_cache_key`；Anthropic 打 ephemeral 断点
    /// - `long`：在 short 上叠加长保留（Anthropic `ttl:1h` / OpenAI `prompt_cache_retention:24h`）
    pub prompt_cache_retention: String,
    /// CLI 身份伪装：`""` 关闭 | `claude_code` | `codex` | `grok`。
    pub cli_identity: String,
    /// 身份版本号，空则用内置常量。
    pub cli_identity_version: String,
}

impl Default for ProviderRequestConfig {
    fn default() -> Self {
        Self {
            oauth: None,
            custom_headers: Vec::new(),
            use_system_proxy: true,
            prompt_caching: None,
            prompt_cache_retention: "short".to_string(),
            cli_identity: String::new(),
            cli_identity_version: String::new(),
        }
    }
}

/**
 * AI 模型提供商配置
 *
 * api_keys 是密钥池；`active_key_index` 是用户点选的当前 Key。
 * 鉴权/配额失败时仍会自动切到池里其它 Key（进程内 `active_key_idx`），
 * 重启或用户再次点选后回到这里保存的索引。
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
    /// 关闭后该供应商不会出现在模型选择器中，已引用它的功能会在保存时切到第一个启用的供应商。
    #[serde(default = "default_provider_enabled")]
    pub enabled: bool,
    /// API 格式：`openai_chat` 或 `anthropic_messages`。
    /// 旧值 `openai` / `anthropic` 会在 `sanitize_settings` 中归一化。
    #[serde(default = "default_api_format")]
    pub api_format: String,
    /// 用户自定义的模型参数覆盖（仅持久化用户显式修改的字段）
    #[serde(default)]
    pub model_overrides: std::collections::HashMap<String, ModelInfo>,
    /// 是否对请求体做 gzip 压缩再发送。默认关。
    /// 用于个别供应商前置的 Cloudflare WAF 会扫明文请求体、把 agent 工具/系统提示里的
    /// shell 命令、文件路径、SQL 等文本误判为攻击而返回 403「Blocked」的情况——
    /// gzip 后 WAF 不解析压缩体即可放行（后端需接受 gzip 请求，多数 OpenAI 兼容网关支持）。
    /// 不接受 gzip 的供应商（如官方 DeepSeek）请保持关闭，否则会 400。
    #[serde(default)]
    pub compress_request_body: bool,
    /// 「请求配置」：自定义头 / 代理 / prompt 缓存 / CLI 身份。
    #[serde(default)]
    pub request: ProviderRequestConfig,
    /// 用户点选的当前 Key 下标。缺省 / 越界时按 0 处理。
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub active_key_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderApiFormat {
    OpenAiChat,
    AnthropicMessages,
    /// OpenAI Responses API (`POST /v1/responses`). Used by Codex / Responses-native
    /// models and proxies that only emit tool-call arguments over this protocol.
    OpenAiResponses,
    /// Google Gemini native `generateContent` protocol. Avoids the OpenAI-compat
    /// endpoint's strict rejection of unknown fields (e.g. `promptCacheKey` 400).
    Gemini,
    /// xAI (Grok) 的 Responses 端点。**线协议与 `OpenAiResponses` 相同**，走同一个适配器；
    /// 区别是 xAI 严格拒收一批 OpenAI 专有字段（`store` / `prompt_cache_key` /
    /// `instructions` / `metadata` / `stream_options` …），思考档位也是它自己的一套
    /// （`none|low|medium|high|xhigh`）。做成独立协议而不是按 base_url 猜：中转站可以把
    /// grok 挂在任意域名上，靠域名判断必然漏。
    XaiResponses,
}

impl ProviderApiFormat {
    pub fn from_raw(raw: &str) -> Self {
        match raw.trim() {
            "anthropic" | "anthropic_messages" => Self::AnthropicMessages,
            "openai_responses" | "responses" => Self::OpenAiResponses,
            "gemini" | "google" | "gemini_generate" => Self::Gemini,
            "xai" | "xai_responses" | "grok" => Self::XaiResponses,
            _ => Self::OpenAiChat,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChat => "openai_chat",
            Self::AnthropicMessages => "anthropic_messages",
            Self::OpenAiResponses => "openai_responses",
            Self::Gemini => "gemini",
            Self::XaiResponses => "xai_responses",
        }
    }
}

impl ModelProvider {
    pub fn is_opencode_free(&self) -> bool {
        self.request.oauth.is_none()
            && self.api_format_kind() == ProviderApiFormat::OpenAiChat
            && crate::opencode_free::is_endpoint(&self.base_url)
            && self.api_keys.iter().all(|key| key.trim().is_empty())
    }

    /// API-key requests may be anonymous; OAuth still requires a saved login.
    pub fn authentication_ready(&self) -> bool {
        self.request
            .oauth
            .as_ref()
            .is_none_or(|auth| auth.credential_id.is_some())
    }

    pub fn api_format_kind(&self) -> ProviderApiFormat {
        ProviderApiFormat::from_raw(&self.api_format)
    }

    /// 夹到现有密钥池范围内；空池为 0。
    pub fn clamped_active_key_index(&self) -> usize {
        match self.api_keys.len() {
            0 => 0,
            n => self.active_key_index.min(n - 1),
        }
    }

    /// 当前点选的 Key；该槽为空时退回池里第一条非空。
    pub fn preferred_api_key(&self) -> Option<&str> {
        let idx = self.clamped_active_key_index();
        self.api_keys
            .get(idx)
            .map(String::as_str)
            .filter(|k| !k.trim().is_empty())
            .or_else(|| {
                self.api_keys
                    .iter()
                    .map(String::as_str)
                    .find(|k| !k.trim().is_empty())
            })
    }

    /// Prompt 缓存策略。非法/空串视为 `short`（与 sanitize 一致）。
    pub fn cache_retention(&self) -> CacheRetention {
        CacheRetention::parse(&self.request.prompt_cache_retention)
    }

    /// 是否发送客户端缓存字段（`retention != none`）。
    /// Gemini / xAI 适配器本身不发字段；此处只表示用户策略。
    pub fn prompt_caching_enabled(&self) -> bool {
        !matches!(self.cache_retention(), CacheRetention::None)
    }
}

/// 对齐 pi：`none | short | long`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRetention {
    None,
    Short,
    Long,
}

impl CacheRetention {
    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "none" => Self::None,
            "long" => Self::Long,
            _ => Self::Short,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Short => "short",
            Self::Long => "long",
        }
    }
}

/**
 * 模型能力信息（来自内置数据库或用户自定义）
 */
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelInfo {
    pub display_name: Option<String>,
    /// Latest upstream capability; never takes precedence over an explicit override.
    pub advertised_video_input: Option<bool>,
    pub context_window: Option<u64>,
    pub max_output: Option<u64>,
    /// 模型级采样温度；None 表示请求默认不发送 temperature。
    pub temperature: Option<f64>,
    /// 用户显式清空温度时阻止回落到模型库默认值。仅用于 model_overrides。
    pub omit_temperature: Option<bool>,
    pub capabilities: Option<ModelCapabilities>,
    pub pricing: Option<ModelPricing>,
    /// 每模型「思考等级」白名单，覆盖模型库的 `reasoningEfforts`。
    /// `None` = 跟随模型库；`Some([])` = 该模型没有 effort 旋钮（请求不带任何等级字段）；
    /// `Some([..])` = 只这几档可选可下发。仅用于 model_overrides。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<String>>,
    /// 每模型额外请求体字段（原样 merge 进 chat/completions body 根部）。
    /// 用于给严格 OpenAI-compat 端点塞标准 schema 之外的私有旋钮，例如 NVIDIA NIM /
    /// vLLM / SGLang 的 `chat_template_kwargs`、GLM 的 thinking 开关等。仅用于 model_overrides。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelCapabilities {
    pub vision: Option<bool>,
    pub video_input: Option<bool>,
    pub function_calling: Option<bool>,
    pub reasoning: Option<bool>,
    pub streaming: Option<bool>,
    pub web_search: Option<bool>,
    pub image_generation: Option<bool>,
    pub video_generation: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelPricing {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cached_input: Option<f64>,
}

/// Canonicalize one provider using the same migration rules as the full settings store.
pub fn sanitize_provider(provider: &mut ModelProvider) {
    provider.api_format = provider.api_format_kind().as_str().to_string();
    if let Some(legacy) = provider.api_key_legacy.take() {
        let trimmed = legacy.trim().to_string();
        if !trimmed.is_empty() && !provider.api_keys.contains(&trimmed) {
            provider.api_keys.insert(0, trimmed);
            provider.active_key_index = provider.active_key_index.saturating_add(1);
        }
    }
    // 去重 + 去空，并跟着挪 active_key_index（删掉当前槽前面的空/重复项时不能还指着旧下标）。
    let old_active = provider.active_key_index;
    let mut seen = std::collections::HashSet::new();
    let mut kept: Vec<String> = Vec::with_capacity(provider.api_keys.len());
    let mut mapped_active: Option<usize> = None;
    for (i, k) in provider.api_keys.iter().enumerate() {
        let trimmed = k.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        if i == old_active {
            mapped_active = Some(kept.len());
        }
        kept.push(k.clone());
    }
    provider.api_keys = kept;
    provider.active_key_index = match provider.api_keys.len() {
        0 => 0,
        n => mapped_active
            .unwrap_or_else(|| old_active.min(n - 1))
            .min(n - 1),
    };

    // 请求配置归一：settings.json 是用户可以手改的文件，非法头名会让 reqwest
    // 构造请求时直接失败，头值里的 CR/LF 是 header 注入，这里一并丢掉。
    provider.request.custom_headers.retain(is_usable_header);
    // Prompt 缓存：合法 none|short|long 优先；旧 bool 仅在 retention 缺失/非法时迁移。
    let retention_ok = matches!(
        provider.request.prompt_cache_retention.as_str(),
        "none" | "short" | "long"
    );
    if let Some(enabled) = provider.request.prompt_caching.take() {
        if !retention_ok {
            provider.request.prompt_cache_retention = if enabled {
                "short".to_string()
            } else {
                "none".to_string()
            };
        }
        // retention 已合法：保留用户/新字段，只丢掉遗留 bool。
    } else if !retention_ok {
        provider.request.prompt_cache_retention = "short".to_string();
    }
    if !matches!(
        provider.request.cli_identity.as_str(),
        "" | "claude_code" | "codex" | "grok"
    ) {
        provider.request.cli_identity = String::new();
    }
    // 版本号会被拼进 User-Agent，非法值清空（`identity_version` 会退回内置版本）。
    // 先 trim 再判，与 `identity_version` 同口径：粘贴常带尾换行，那不该算非法。
    let trimmed_version = provider.request.cli_identity_version.trim().to_string();
    provider.request.cli_identity_version = if is_valid_header_value(&trimmed_version) {
        trimmed_version
    } else {
        String::new()
    };
}

/// 由 Kivio 自己管理、不允许用户覆盖的头。放开会让鉴权/路由错乱。
const RESERVED_HEADER_KEYS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "host",
    "content-length",
    "content-encoding",
    "content-type",
    // 适配器已经发了 `Accept-Encoding: identity`，而 reqwest 的 `.header()` 是 append 不是
    // 覆盖 —— 用户再填一条 gzip 会变成 `identity, gzip`，客户端没开 gzip 解码，
    // 认这条头的供应商回来的 SSE 就是一堆二进制垃圾。
    "accept-encoding",
    "anthropic-version",
];

/// RFC 7230 token 字符集。
pub fn is_valid_header_key(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().all(|b| {
            b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// 只允许可见 ASCII 与水平制表符：CR/LF 是 header 注入，非 ASCII 会让部分网关直接 400。
pub fn is_valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
}

pub fn is_reserved_header_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    RESERVED_HEADER_KEYS.contains(&normalized.as_str())
}

/// 一条自定义头是否可用（合法且非保留）。
pub fn is_usable_header(header: &ProviderCustomHeader) -> bool {
    is_valid_header_key(&header.key)
        && is_valid_header_value(&header.value)
        && !is_reserved_header_key(&header.key)
}

fn default_provider_enabled() -> bool {
    true
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

fn default_api_format() -> String {
    "openai_chat".to_string()
}

pub fn default_chat_max_output_tokens() -> u32 {
    16_384
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
    pub(crate) fn is_configured(&self) -> bool {
        !self.provider_id.trim().is_empty()
    }
}

/**
 * 默认模型配置。
 *
 * chat：新建 Chat 对话的全局默认模型；为空时沿用 Lens → 输入翻译的兜底链路。
 * vision：图片附件分析副任务使用；为空时继承当前会话主模型（无会话时回退有效 Chat 默认）。
 * title_summary：标题总结副任务使用；为空时继承当前会话主模型（无会话时回退有效 Chat 默认）。
 * compression：上下文/历史对话压缩副任务使用；为空时继承当前会话主模型（无会话时回退有效 Chat 默认）。
 * image_generation：生图副任务使用；为空时若当前会话主模型支持直接生图则继承该模型。
 * prompt_optimize：输入框问题优化副任务使用；为空时继承当前会话主模型。
 * advisor：顾问模型（executor-advisor 模式）——主循环模型可用 `advisor` 工具向它
 *   单次咨询；为空 = 功能关闭（工具不注册），没有继承语义。
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DefaultModelsConfig {
    #[serde(default)]
    pub chat: DefaultModelSelection,
    #[serde(default)]
    pub vision: DefaultModelSelection,
    /// Video-capable auxiliary model; empty selects an enabled capable model automatically.
    #[serde(default)]
    pub video_analysis: DefaultModelSelection,
    #[serde(default)]
    pub title_summary: DefaultModelSelection,
    #[serde(default)]
    pub compression: DefaultModelSelection,
    #[serde(default)]
    pub image_generation: DefaultModelSelection,
    #[serde(default)]
    pub prompt_optimize: DefaultModelSelection,
    #[serde(default)]
    pub advisor: DefaultModelSelection,
}

impl Default for DefaultModelsConfig {
    fn default() -> Self {
        Self {
            chat: DefaultModelSelection::default(),
            vision: DefaultModelSelection::default(),
            video_analysis: DefaultModelSelection::default(),
            title_summary: DefaultModelSelection::default(),
            compression: DefaultModelSelection::default(),
            image_generation: DefaultModelSelection::default(),
            prompt_optimize: DefaultModelSelection::default(),
            advisor: DefaultModelSelection::default(),
        }
    }
}

pub fn sanitize_default_model_selection(
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

pub fn format_chat_login_required_error(provider_name: &str) -> String {
    let provider = provider_name.trim();
    if provider.is_empty() {
        "Chat 模型供应商需要账号登录，请到设置 > 模型中登录后再发送。".to_string()
    } else {
        format!("Chat 模型供应商「{provider}」需要账号登录，请到设置 > 模型中登录后再发送。")
    }
}
