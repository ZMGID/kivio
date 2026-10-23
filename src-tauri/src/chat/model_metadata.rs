#[path = "model_parameters.rs"]
mod parameters;
use parameters::*;
pub(crate) use parameters::{
    chat_max_output_tokens_for_model, chat_max_output_tokens_on_wire, claude_thinking_profile,
    gemini_thinking_budget, gemini_thinking_profile, is_image_output_model, model_supports_video,
    normalize_model_name, pricing_for_model, temperature_for_request, ClaudeThinkingKind,
    ClaudeThinkingProfile,
};
use serde_json::Value;

use crate::settings::{ModelInfo, ModelProvider};

const FALLBACK_CONTEXT_WINDOW_TOKENS: usize = 200_000;

fn context_window_from_model_info(info: Option<&ModelInfo>) -> Option<usize> {
    info.and_then(|info| info.context_window)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn model_vision_from_model_info(info: Option<&ModelInfo>) -> Option<bool> {
    info.and_then(|info| info.capabilities.as_ref())
        .and_then(|capabilities| capabilities.vision)
}

fn model_image_generation_from_model_info(info: Option<&ModelInfo>) -> Option<bool> {
    info.and_then(|info| info.capabilities.as_ref())
        .and_then(|capabilities| capabilities.image_generation)
}

fn model_database_context_window(model: &str) -> Option<usize> {
    context_window_from_database_entry(model_database_entry(model).as_ref())
}

fn context_window_from_database_entry(entry: Option<&Value>) -> Option<usize> {
    entry?
        .get("contextWindow")
        .and_then(Value::as_u64)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn model_database_vision(model: &str) -> Option<bool> {
    model_database_entry(model)?
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("vision"))
        .and_then(Value::as_bool)
}

fn model_database_image_generation(model: &str) -> Option<bool> {
    model_database_entry(model)?
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("imageGeneration"))
        .and_then(Value::as_bool)
}

/// 某模型支持的「思考等级」(reasoning effort) 列表，供前端的等级选择器决定显示哪些档。
/// 三层优先级：
/// 1. provider 的 `model_overrides[model].reasoningEfforts`（模型详情抽屉里手填，改完即生效）；
/// 2. 模型库 `reasoningEfforts` 显式列表（各家支持不构成单调子集，故逐模型列举）；
/// 3. 都没有才按家族兜底：能认出的 Claude 代际用官方档位表；认不出的 Anthropic 给
///    安全子集 low/medium/high（不要发明 xhigh/max）；其余同样是 low/medium/high。
///
/// 前两层的**显式空数组 = 该模型没有思考深度旋钮**（GLM-4.7 / Kimi K2.x / 通义 / Claude 3.x
/// 等），上游 `resolve_thinking` 据此不下发任何等级字段。Claude 4 的 UI 档位由 Anthropic
/// 适配器映射：4.5 及更早 → `budget_tokens`（Opus 4.5 另带 `output_config.effort`），
/// 4.6+ → adaptive + effort。始终只保留已知合法值并去重，避免脏数据进入请求。
pub fn reasoning_efforts_for_model(provider: Option<&ModelProvider>, model: &str) -> Vec<String> {
    // Antigravity OAuth catalog variants already select the effort through the model ID.
    if provider.is_some_and(crate::provider_oauth::antigravity::is_provider)
        && crate::provider_oauth::antigravity::model_includes_effort(model)
    {
        return vec![];
    }
    if let Some(list) = provider
        .and_then(|provider| override_model_info(provider, model))
        .and_then(|info| info.reasoning_efforts.as_deref())
    {
        return sanitize_efforts(list.iter().map(String::as_str));
    }
    if let Some(list) = model_database_entry(provider_model_database_id(provider, model))
        .as_ref()
        .and_then(|entry| entry.get("reasoningEfforts"))
        .and_then(Value::as_array)
    {
        return sanitize_efforts(list.iter().filter_map(Value::as_str));
    }
    let id = normalize_model_name(provider_model_database_id(provider, model));
    if id.contains("gemini-3.1-flash-lite-image") {
        return vec!["high".into()];
    }
    if id.contains("gemini-3-pro") {
        return vec!["low".into(), "high".into()];
    }
    if provider.map(ModelProvider::api_format_kind)
        == Some(crate::settings::ProviderApiFormat::AnthropicMessages)
    {
        if let Some(profile) = claude_thinking_profile(model) {
            return profile.ui_effort_levels();
        }
    }
    vec!["low".into(), "medium".into(), "high".into()]
}

/// Whether an Off choice can actually be honored, independently of effort levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ThinkingOffMode {
    Supported,
    UpfrontOnly,
    NotApplicable,
    Unsupported,
    Unknown,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThinkingCapabilities {
    pub levels: Vec<String>,
    pub off_mode: ThinkingOffMode,
}

pub(crate) fn thinking_capabilities_for_model(
    provider: Option<&ModelProvider>,
    model: &str,
) -> ThinkingCapabilities {
    let reasoning = provider
        .and_then(|p| override_model_info(p, model))
        .and_then(|info| info.capabilities.as_ref())
        .and_then(|caps| caps.reasoning)
        .or_else(|| {
            model_database_entry(provider_model_database_id(provider, model))
                .as_ref()
                .and_then(|entry| entry.get("capabilities"))
                .and_then(|caps| caps.get("reasoning"))
                .and_then(Value::as_bool)
        });
    if reasoning == Some(false) {
        return ThinkingCapabilities {
            levels: vec![],
            off_mode: ThinkingOffMode::NotApplicable,
        };
    }
    let levels = reasoning_efforts_for_model(provider, model);
    let id = normalize_model_name(provider_model_database_id(provider, model));
    let kimi_code = id.starts_with("kimi-code/");
    let id = id.rsplit('/').next().unwrap_or(&id);
    let off_mode = if provider.is_some_and(crate::provider_oauth::antigravity::is_provider)
        && crate::provider_oauth::antigravity::model_includes_effort(model)
    {
        ThinkingOffMode::Unsupported
    } else if let Some(mode) = model_database_entry(id)
        .as_ref()
        .and_then(|entry| entry.get("thinkingOff"))
        .and_then(Value::as_bool)
    {
        if mode {
            ThinkingOffMode::Supported
        } else {
            ThinkingOffMode::Unsupported
        }
    } else if let Some(profile) = claude_thinking_profile(id) {
        if profile.off_thinking_type == Some("between_tools") {
            ThinkingOffMode::UpfrontOnly
        } else if profile.kind == ClaudeThinkingKind::Unsupported
            || id.contains("fable")
            || id.contains("mythos")
            || normalize_model_sep(id).contains("opus-5-5")
        {
            ThinkingOffMode::Unsupported
        } else {
            ThinkingOffMode::Supported
        }
    } else if let Some(profile) = gemini_thinking_profile(id) {
        if profile.supports_off {
            ThinkingOffMode::Supported
        } else {
            ThinkingOffMode::Unsupported
        }
    } else if kimi_code
        || id.starts_with("kimi-k3")
        || id.starts_with("kimi-k2.7")
        || id.starts_with("kimi-k2-thinking")
        || id.starts_with("deepseek-r1")
        || id.starts_with("grok-")
        || id == "gpt-5"
        || id.starts_with("gpt-5-")
        || id.starts_with("gpt-5.4-pro")
        || id.starts_with("gpt-5.5-pro")
        || id.starts_with("gpt-6-astra")
        || id.starts_with("gpt-6.1-sol")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
    {
        ThinkingOffMode::Unsupported
    } else if id.starts_with("deepseek-")
        || id.starts_with("kimi-k2.5")
        || id.starts_with("kimi-k2.6")
        || id.starts_with("gpt-5.")
        || id.starts_with("gpt-6-")
    {
        ThinkingOffMode::Supported
    } else {
        ThinkingOffMode::Unknown
    };
    ThinkingCapabilities { levels, off_mode }
}

const REASONING_EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

fn sanitize_efforts<'a>(items: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in items {
        if REASONING_EFFORT_LEVELS.contains(&item) && !out.iter().any(|kept| kept == item) {
            out.push(item.to_string());
        }
    }
    out
}

// 思考等级不再在此做「协议级白名单 → 收敛」的二次映射。哪个模型认哪几档是**逐模型**的
// （xhigh 在 Chat Completions 和 Responses 两个端点都收，但 gpt-5-codex 传了就 400），
// 用 api_format 粒度去卡必然错，且静默改档会让用户在 UI 选了 xhigh 却发出 high，毫无信号。
// 唯一判据是模型库的 `reasoningEfforts`（见 `reasoning_efforts_for_model`），前端选择器只
// 渲染这个列表；值本身已被校验两遍（落盘 `update_conversation_settings` + 取用
// `resolve_thinking`），到适配器手上必是 `low|medium|high|xhigh|max`。选错档就吃 provider
// 的 400 —— 那是正确的报错，不是需要兜底的异常。

pub(crate) fn model_supports_vision(provider: Option<&ModelProvider>, model: &str) -> Option<bool> {
    let provider = provider?;
    model_vision_from_model_info(provider.model_overrides.get(model))
        .or_else(|| model_database_vision(provider_model_database_id(Some(provider), model)))
}

pub(crate) fn model_supports_image_generation(
    provider: Option<&ModelProvider>,
    model: &str,
) -> Option<bool> {
    let provider = provider?;
    override_image_generation(provider, model)
        .or_else(|| model_database_image_generation(model))
        .or_else(|| image_generation_model_name_heuristic(provider, model))
}

/// 读 `model_overrides` 的生图能力：先精确 `get(model)`，未命中再按**归一化名**遍历匹配
/// （小 HashMap，O(n) 可接受），消除大小写 / `models/` 前缀导致 override 静默失效。
fn override_image_generation(provider: &ModelProvider, model: &str) -> Option<bool> {
    model_image_generation_from_model_info(override_model_info(provider, model))
}

pub(crate) fn model_can_generate_images_directly(provider: &ModelProvider, model: &str) -> bool {
    model_supports_image_generation(Some(provider), model) == Some(true)
        && crate::chat::image_generation::has_known_direct_image_generation_route(provider, model)
}

/// 当前 provider 是否支持模型**原生内置联网搜索**（任务 07-23）。
/// 依 `api_format`：OpenAI Responses / Gemini / Anthropic Messages 支持；OpenAI Chat
/// Completions 一般不支持（gpt-5 在其上开 `web_search` 会 400）。例外：官方 DeepSeek
/// API（`api.deepseek.com`）的 Chat Completions 供应商仍可开内置——请求改走 Responses
/// 的服务端 `web_search`（官方 Chat Completions 只收 `function` 工具）。
/// 前端据此把「内置」选项置灰。
pub(crate) fn builtin_web_search_supported(provider: &ModelProvider) -> bool {
    if crate::provider_oauth::antigravity::is_provider(provider) {
        return false;
    }
    use crate::settings::ProviderApiFormat::{
        AnthropicMessages, Gemini, OpenAiChat, OpenAiResponses, XaiResponses,
    };
    // xAI 的 Responses 同样有服务端 web_search（还多 x_search），与 OpenAI Responses 同形。
    match provider.api_format_kind() {
        OpenAiResponses | XaiResponses | Gemini | AnthropicMessages => true,
        OpenAiChat => crate::utils::is_official_deepseek_api(&provider.base_url),
    }
}

pub(crate) fn image_generation_model_for_session(
    settings: &crate::settings::Settings,
    session: Option<crate::settings::SessionModel<'_>>,
) -> Option<(String, String)> {
    if !settings
        .default_models
        .image_generation
        .provider_id
        .trim()
        .is_empty()
        && !settings
            .default_models
            .image_generation
            .model
            .trim()
            .is_empty()
    {
        return Some((
            settings.default_models.image_generation.provider_id.clone(),
            settings.default_models.image_generation.model.clone(),
        ));
    }
    let session = session.filter(|session| session.is_set())?;
    let provider = settings.get_provider(session.provider_id)?;
    if model_can_generate_images_directly(provider, session.model) {
        Some((session.provider_id.to_string(), session.model.to_string()))
    } else {
        None
    }
}

fn image_generation_model_name_heuristic(provider: &ModelProvider, model: &str) -> Option<bool> {
    let descriptor = format!(
        "{} {} {} {}",
        provider.name,
        provider.base_url,
        provider.api_format,
        normalize_model_name(model)
    )
    .to_ascii_lowercase();
    let known_image_model = [
        "gpt-image",
        "dall-e",
        "grok-imagine-image",
        "gemini-3.1-flash-image",
        "gemini-3.1-flash-lite-image",
        "gemini-3-pro-image",
        "gemini-2.5-flash-image",
        "nano-banana",
        "qwen-image",
        "glm-image",
        "hy-image",
        "seedream",
        "krea",
        "muse-image",
        "mai-image",
        "flux",
        "recraft",
        "riverflow",
        "stable-diffusion",
        "sdxl",
        "ideogram",
        "imagen",
        "image-generation",
        "image_generation",
    ]
    .iter()
    .any(|needle| descriptor.contains(needle));
    if known_image_model {
        Some(true)
    } else {
        None
    }
}

pub(crate) fn context_window_for_model(
    provider: Option<&ModelProvider>,
    model: &str,
) -> (usize, bool) {
    if let Some(tokens) = context_window_from_model_info(
        provider.and_then(|provider| provider.model_overrides.get(model)),
    ) {
        return (tokens, false);
    }
    if let Some(tokens) = model_database_context_window(provider_model_database_id(provider, model))
    {
        return (tokens, false);
    }

    let lower = model.to_ascii_lowercase();
    let known = [
        ("1m", 1_000_000usize),
        // 256k：`kimi-code/k3-256k` 这类名字里明写窗口的模型，缺这一项会一路掉到
        // FALLBACK_CONTEXT_WINDOW_TOKENS(200K)。值取 2^18（对齐 modelDatabase 里 kimi 系列）。
        ("256k", 262_144usize),
        ("200k", 200_000usize),
        ("128k", 128_000usize),
        ("100k", 100_000usize),
        ("64k", 64_000usize),
        ("32k", 32_000usize),
        ("16k", 16_000usize),
        ("8k", 8_000usize),
    ];
    for (needle, tokens) in known {
        if lower.contains(needle) {
            return (tokens, false);
        }
    }
    if lower.contains("claude") {
        return (200_000, false);
    }
    if lower.contains("gpt-4o")
        || lower.contains("gpt-4.1")
        || lower.contains("gpt-5")
        || lower.contains("deepseek")
        || lower.contains("qwen")
        || lower.contains("gemini")
    {
        return (128_000, true);
    }
    (FALLBACK_CONTEXT_WINDOW_TOKENS, true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn thinking_capabilities_separate_toggle_effort_and_upfront_only() {
        use ThinkingOffMode::*;
        for (model, mode) in [
            ("gpt-4o", NotApplicable),
            ("kimi-k2.6", Supported),
            ("kimi-k3", Unsupported),
            ("kimi-k2.7-code", Unsupported),
            ("gemini-2.5-flash", Supported),
            ("gemini-2.5-pro", Unsupported),
            ("gemini-3.8-flash", Unsupported),
            ("claude-opus-4.7", Supported),
            ("claude-opus-5.5", Unsupported),
            ("claude-sonnet-5.5", UpfrontOnly),
            ("claude-haiku-5.5", Supported),
            ("gpt-6-astra", Unsupported),
            ("gpt-6.1-sol", Unsupported),
            ("gpt-5.4-pro", Unsupported),
            ("gpt-5.5-pro", Unsupported),
            ("gpt-6-sol", Supported),
            ("grok-4.6", Unsupported),
        ] {
            assert_eq!(
                thinking_capabilities_for_model(None, model).off_mode,
                mode,
                "{model}"
            );
        }
        assert!(thinking_capabilities_for_model(None, "kimi-k2.6")
            .levels
            .is_empty());
        let mut provider = test_provider_with_overrides(Default::default());
        provider.base_url = "https://api.kimi.com/coding/v1".into();
        assert_eq!(
            thinking_capabilities_for_model(Some(&provider), "k3").off_mode,
            Unsupported
        );
        assert_eq!(
            thinking_capabilities_for_model(None, "k3").off_mode,
            Unknown
        );
    }

    use std::collections::HashMap;

    use crate::settings::{ModelInfo, ModelProvider};

    use super::*;

    #[test]
    fn imported_video_models_use_database_defaults() {
        let provider = test_provider_with_overrides(HashMap::new());
        for model in [
            "kimi-k2.7-code",
            "kimi-k2.7-code-highspeed",
            "moonshotai/kimi-k2.7-code",
            "gemini-3-pro-preview",
            "models/gemini-3-pro-preview",
            "gemini-3.1-pro-preview",
            "gemini-3.8-flash",
        ] {
            assert_eq!(
                model_supports_video(&provider, model),
                Some(true),
                "{model}"
            );
        }
        for model in [
            "gemini-3-pro-image-preview",
            "gemini-embedding-001",
            "kimi-k2",
            "kimi-k2.7",
        ] {
            assert_ne!(
                model_supports_video(&provider, model),
                Some(true),
                "{model}"
            );
        }
    }

    fn db_display_name(model: &str) -> Option<String> {
        model_database_entry(model)
            .as_ref()
            .and_then(|entry| entry.get("displayName"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// 与前端 `src/data/modelMatching.test.ts` 对齐：连字符版本 / 版本延续 / 日期后缀。
    #[test]
    fn model_database_matching_is_isomorphic_with_frontend() {
        // 空白 → 无匹配
        assert!(model_database_entry("").is_none());
        assert!(model_database_entry("   ").is_none());

        // 点号键 ↔ 连字符 id
        assert_eq!(
            db_display_name("claude-sonnet-4-6").as_deref(),
            Some("Claude Sonnet 4.6")
        );
        assert_eq!(
            db_display_name("claude-opus-4-8").as_deref(),
            Some("Claude Opus 4.8")
        );
        assert_eq!(
            db_display_name("claude-opus-4-7").as_deref(),
            Some("Claude Opus 4.7")
        );
        assert_eq!(
            db_display_name("claude-haiku-4-5").as_deref(),
            Some("Claude Haiku 4.5")
        );
        assert_eq!(
            db_display_name("claude-opus-4-5").as_deref(),
            Some("Claude Opus 4.5")
        );
        assert_eq!(
            db_display_name("claude-opus-4-5-20251101").as_deref(),
            Some("Claude Opus 4.5")
        );
        assert_eq!(db_display_name("kimi-k2-7").as_deref(), Some("Kimi K2.7"));

        // 主版本本身仍命中自己的条目，不能被次级版本抢
        assert_eq!(
            db_display_name("claude-sonnet-5").as_deref(),
            Some("Claude Sonnet 5")
        );
        assert_eq!(
            db_display_name("claude-opus-5").as_deref(),
            Some("Claude Opus 5")
        );

        // 日期快照后缀：最长归一化前缀，仍认 4.8（数字段 ≥3 不算版本延续）
        assert_eq!(
            db_display_name("claude-opus-4-8-20260101").as_deref(),
            Some("Claude Opus 4.8")
        );

        // 版本延续保护：未知次级版本不能退化到 gpt-5
        assert!(model_database_entry("gpt-5.7-nebula").is_none());
        // 连字符写法的已知 5.6 变体要认到 5.6 条目，不能落到 gpt-5
        assert_eq!(
            db_display_name("gpt-5-6-luna").as_deref(),
            Some("GPT-5.6 Luna")
        );
        assert_eq!(
            db_display_name("gpt-5.6-luna").as_deref(),
            Some("GPT-5.6 Luna")
        );
        assert_eq!(
            db_display_name("gpt-5.6-sol").as_deref(),
            Some("GPT-5.6 Sol")
        );
        assert_eq!(
            db_display_name("gpt-5.6-terra").as_deref(),
            Some("GPT-5.6 Terra")
        );
        assert_eq!(db_display_name("gpt-5.5").as_deref(), Some("GPT-5.5"));
        assert_eq!(db_display_name("gpt-5").as_deref(), Some("GPT-5"));

        // provider 前缀（含连字符版本 id）+ 点号键本身
        assert_eq!(db_display_name("openai/gpt-4o"), db_display_name("gpt-4o"));
        assert_eq!(
            db_display_name("anthropic/claude-sonnet-4-6").as_deref(),
            Some("Claude Sonnet 4.6")
        );
        assert_eq!(
            db_display_name("claude-sonnet-4.6").as_deref(),
            Some("Claude Sonnet 4.6")
        );
        assert_eq!(db_display_name("ox-alpha").as_deref(), Some("Ox Alpha"));
        assert_eq!(
            db_display_name("stealth/ox-alpha").as_deref(),
            Some("Ox Alpha")
        );
        assert_eq!(
            db_display_name("hy4-preview").as_deref(),
            Some("Hy4 Preview")
        );
        assert_eq!(db_display_name("hy4").as_deref(), Some("Hy4 Preview"));
        assert_eq!(
            db_display_name("tencent/hy4-preview").as_deref(),
            Some("Hy4 Preview")
        );
        assert_eq!(db_display_name("hy3").as_deref(), Some("Hy3"));
        assert_eq!(
            db_display_name("hy3-preview").as_deref(),
            Some("Hy3 Preview")
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "hy3"),
            vec!["low", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "hy4-preview"),
            vec!["high"]
        );
        assert_eq!(
            db_display_name("doubao-seed-2.1-pro").as_deref(),
            Some("Doubao Seed 2.1 Pro")
        );
        assert_eq!(
            db_display_name("doubao-seed-2-1-pro-260628").as_deref(),
            Some("Doubao Seed 2.1 Pro")
        );
        assert_eq!(
            db_display_name("bytedance-seed/seed-2.0-code").as_deref(),
            Some("Doubao Seed 2.0 Code")
        );
        assert_eq!(
            db_display_name("doubao-seed-2.0-pro").as_deref(),
            Some("Doubao Seed 2.0 Pro")
        );
        assert_eq!(db_display_name("glm-4.7").as_deref(), Some("GLM-4.7"));
        assert_eq!(
            db_display_name("glm-4.7-flash").as_deref(),
            Some("GLM-4.7 Flash")
        );
        assert_eq!(
            db_display_name("glm-4.5-airx").as_deref(),
            Some("GLM-4.5 AirX")
        );
        assert_eq!(
            db_display_name("deepseek-v4-flash-vision-exp").as_deref(),
            Some("DeepSeek V4.1 Flash (Legacy Vision Alias)")
        );
        assert_eq!(
            db_display_name("qwen3.5-plus").as_deref(),
            Some("Qwen3.5 Plus")
        );

        // 包含匹配路径：带 tag 的变体（`gemma4:31b`）靠前缀/包含吃到 `gemma4`
        assert_eq!(db_display_name("gemma4:31b").as_deref(), Some("Gemma 4"));

        // 生图：Grok Imagine 各档必须是独立条目，不能前缀塌到 quality / 基线
        assert_eq!(
            db_display_name("grok-imagine-image").as_deref(),
            Some("Grok Imagine Image")
        );
        assert_eq!(
            db_display_name("grok-imagine-image-2.0").as_deref(),
            Some("Grok Imagine Image 2.0")
        );
        assert_eq!(
            db_display_name("x-ai/grok-imagine-image-2.0").as_deref(),
            Some("Grok Imagine Image 2.0")
        );
        assert_eq!(
            db_display_name("grok-imagine-image-quality").as_deref(),
            Some("Grok Imagine Image Quality")
        );
        assert_eq!(
            model_database_image_generation("grok-imagine-image"),
            Some(true)
        );
        assert_eq!(
            model_database_image_generation("gemini-3.1-flash-lite-image"),
            Some(true)
        );
        assert_eq!(
            db_display_name("qwen-image-3").as_deref(),
            Some("Qwen Image 3")
        );

        // 未知模型
        assert!(model_database_entry("totally-unknown-model-xyz-9999").is_none());

        // 有意不放宽：kimi CLI 短别名 `k3` 不得命中 `kimi-k3`（见 external_agents/context.rs）
        assert!(model_database_entry("k3").is_none());
    }

    /// 连字符 id 必须吃到正确条目的 efforts，不能退化到旧主版本的 `[]`。
    #[test]
    fn dash_versioned_ids_resolve_correct_reasoning_efforts() {
        // 4.6 有 low..max；`claude-sonnet-4` 走 extended 档位 low/medium/high。
        assert_eq!(
            reasoning_efforts_for_model(None, "claude-sonnet-4-6"),
            vec!["low", "medium", "high", "max"]
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "anthropic/claude-sonnet-4-6"),
            vec!["low", "medium", "high", "max"]
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "claude-sonnet-4"),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "claude-haiku-4-5"),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "claude-haiku-4.5"),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "claude-opus-4-5"),
            vec!["low", "medium", "high"]
        );
    }

    #[test]
    fn user_model_database_overlay_merges_fields_and_normalizes_keys() {
        let mut base = serde_json::json!({
            "_meta": { "version": "builtin" },
            "gpt-5.6": { "contextWindow": 256000, "pricing": { "input": 5.0 } },
        })
        .as_object()
        .unwrap()
        .clone();
        let user = serde_json::json!({
            "_meta": { "version": "user" },
            "GPT-5.6": { "reasoningEfforts": ["max"] },
            "My-New-Model": { "contextWindow": 999 },
        })
        .as_object()
        .unwrap()
        .clone();

        super::super::model_database::merge_user_entries(&mut base, user);

        // 已有 key：按顶层字段合并，pricing 不被抹掉；key 大小写归一。
        let gpt = &base["gpt-5.6"];
        assert_eq!(gpt["pricing"]["input"], 5.0);
        assert_eq!(gpt["contextWindow"], 256000);
        assert_eq!(gpt["reasoningEfforts"], serde_json::json!(["max"]));
        // 新 key 直接新增，且键名已小写（否则匹配不到）。
        assert_eq!(base["my-new-model"]["contextWindow"], 999);
        // _meta 不接受覆盖。
        assert_eq!(base["_meta"]["version"], "builtin");
    }

    #[test]
    fn reasoning_efforts_resolve_from_db_family_and_default() {
        // 原生档位独立于兼容映射：DeepSeek 的 xhigh 映射到 high，不是另一档。
        // 库里能查到时 provider 无关，传 None。
        let ds = reasoning_efforts_for_model(None, "DeepSeek-V4-Flash");
        assert_eq!(ds, vec!["low", "high", "max"]);
        // GPT-5：有 xhigh、无 max（max 是 5.6 一代才加的）。
        let gpt = reasoning_efforts_for_model(None, "gpt-5.5");
        assert!(
            gpt.contains(&"xhigh".to_string()) && !gpt.contains(&"max".to_string()),
            "{gpt:?}"
        );
        // gpt-5.6 全家（含 sol/terra/luna）的 reasoning_options 含 max。
        for model in ["gpt-5.6", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert_eq!(
                reasoning_efforts_for_model(None, model),
                vec!["low", "medium", "high", "xhigh", "max"],
                "{model}"
            );
        }
        // Gemma：有 max、无 xhigh（非单调子集）。
        let gemma = reasoning_efforts_for_model(None, "gemma4:31b");
        assert!(
            gemma.contains(&"max".to_string()) && !gemma.contains(&"xhigh".to_string()),
            "{gemma:?}"
        );
        // 库里没有 + 非 Anthropic → 安全子集 low/medium/high。
        let unknown = reasoning_efforts_for_model(None, "some-random-model");
        assert_eq!(unknown, vec!["low", "medium", "high"]);
        // 认不出的 Anthropic 模型同样走安全子集，不要发明 xhigh/max。
        let mut anthropic = test_provider_with_overrides(HashMap::new());
        for raw in ["anthropic", "anthropic_messages"] {
            anthropic.api_format = raw.to_string();
            assert_eq!(
                reasoning_efforts_for_model(Some(&anthropic), "whatever"),
                vec!["low", "medium", "high"],
                "{raw}"
            );
        }
        // Kimi K3：reasoning_effort 只认 low/high/max（无 medium/xhigh）。
        assert_eq!(
            reasoning_efforts_for_model(None, "kimi-k3"),
            vec!["low", "high", "max"]
        );
    }

    #[test]
    fn model_override_reasoning_efforts_wins_over_database() {
        // 模型详情抽屉里手填的档位优先于模型库；脏值被过滤、重复被去掉；
        // 大小写 / `models/` 前缀不同也要命中（走 normalize_model_name）。
        let mut overrides = HashMap::new();
        overrides.insert(
            "GPT-5.6".to_string(),
            ModelInfo {
                reasoning_efforts: Some(vec![
                    "low".into(),
                    "low".into(),
                    "bogus".into(),
                    "max".into(),
                ]),
                ..Default::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);
        assert_eq!(
            reasoning_efforts_for_model(Some(&provider), "gpt-5.6"),
            vec!["low", "max"]
        );
        // 未被覆盖的模型仍走模型库。
        assert_eq!(
            reasoning_efforts_for_model(Some(&provider), "kimi-k3"),
            vec!["low", "high", "max"]
        );

        // 显式空数组 = 用户宣布这个模型没有 effort 旋钮，压过模型库的非空列表。
        let mut muted = HashMap::new();
        muted.insert(
            "gpt-5.6".to_string(),
            ModelInfo {
                reasoning_efforts: Some(Vec::new()),
                ..Default::default()
            },
        );
        assert!(reasoning_efforts_for_model(
            Some(&test_provider_with_overrides(muted)),
            "models/GPT-5.6"
        )
        .is_empty());
    }

    #[test]
    fn explicit_empty_list_means_no_effort_knob() {
        // 显式 `[]` 必须原样返回空，不能掉进家族兜底 —— 上游 `resolve_thinking` 靠它判定
        // 「这个模型没有思考深度旋钮」。Claude 4.5 走 budget_tokens，库里是 low/medium/high。
        let mut anthropic = test_provider_with_overrides(HashMap::new());
        anthropic.api_format = "anthropic".to_string();
        for model in [
            "claude-3.5-haiku",
            "glm-4.7",     // 思考是模式控制，没有 effort
            "kimi-k2.7",   // 思考强制开，无 effort
            "qwen3.7-max", // 用 thinking_budget 数值预算
            "minimax-m3",
        ] {
            assert!(
                reasoning_efforts_for_model(Some(&anthropic), model).is_empty(),
                "{model} 应为空档位"
            );
        }
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-opus-4"),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-sonnet-4"),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-sonnet-4.5"),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-haiku-4.5"),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-3-5-sonnet-20241022"),
            Vec::<String>::new()
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-3-haiku-20240307"),
            Vec::<String>::new()
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "opus-4.5"),
            vec!["low", "medium", "high"]
        );
        // 4.6 与 4.7 的分界：4.6 止步 max，没有 xhigh。
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-opus-4.6"),
            vec!["low", "medium", "high", "max"]
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), "claude-opus-4.7"),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn xhigh_gated_per_model_not_per_protocol() {
        // 删掉 reasoning_effort_wire 的协议级白名单后，模型库是**唯一**门控：适配器原样下发
        // 用户所选档位，选错就吃 provider 的 400。所以这份数据必须准，尤其是 xhigh 这一档
        // ——它随 gpt-5.1-codex-max 引入，之前的 gpt-5 / gpt-5-codex 传了会 400。
        for model in ["gpt-5", "gpt-5-pro", "gpt-5-codex"] {
            assert!(
                !reasoning_efforts_for_model(None, model).contains(&"xhigh".to_string()),
                "{model} 不支持 xhigh，不能出现在选择器里"
            );
        }
        for model in ["gpt-5.4", "gpt-5.6", "grok-4.6"] {
            assert!(
                reasoning_efforts_for_model(None, model).contains(&"xhigh".to_string()),
                "{model} 支持 xhigh"
            );
        }
        // Gemini thinkingLevel 只有 minimal/low/medium/high，无 xhigh/max；库里无条目，
        // 走非 Anthropic 兜底 low/medium/high 即正确。
        let mut gemini = test_provider_with_overrides(HashMap::new());
        gemini.api_format = "gemini".to_string();
        assert_eq!(
            reasoning_efforts_for_model(Some(&gemini), "gemini-3.2-pro"),
            vec!["low", "medium", "high"]
        );
    }

    fn test_provider_with_overrides(model_overrides: HashMap<String, ModelInfo>) -> ModelProvider {
        ModelProvider {
            id: "provider".to_string(),
            name: "Provider".to_string(),
            api_keys: vec!["sk-test".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides,
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        }
    }

    #[test]
    fn builtin_web_search_supported_by_api_format() {
        let mut p = test_provider_with_overrides(HashMap::new());
        // Chat Completions（含别名/空）⇒ 不支持内置搜索（gpt-5 在其上会 400）。
        for fmt in ["openai_chat", "openai", ""] {
            p.api_format = fmt.into();
            assert!(
                !builtin_web_search_supported(&p),
                "should be unsupported: {fmt:?}"
            );
        }
        // Responses / Gemini / Anthropic（含各别名）⇒ 支持。
        for fmt in [
            "openai_responses",
            "responses",
            "gemini",
            "google",
            "anthropic",
            "anthropic_messages",
        ] {
            p.api_format = fmt.into();
            assert!(
                builtin_web_search_supported(&p),
                "should be supported: {fmt:?}"
            );
        }

        p.api_format = "openai_chat".into();
        p.base_url = "https://api.deepseek.com/v1".into();
        assert!(
            builtin_web_search_supported(&p),
            "official DeepSeek Chat Completions can host-search via Responses"
        );
        p.base_url = "https://relay.example/v1".into();
        assert!(
            !builtin_web_search_supported(&p),
            "relays must not inherit DeepSeek hosted search"
        );
    }

    #[test]
    fn kimi_code_aliases_are_provider_scoped() {
        let mut provider = test_provider_with_overrides(HashMap::new());
        provider.base_url = "https://api.kimi.com/coding/v1".into();
        assert_eq!(
            context_window_for_model(Some(&provider), "k3-256k"),
            (262144, false)
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&provider), "k3"),
            vec!["low", "high", "max"]
        );
        assert!(pricing_for_model(Some(&provider), "k3").is_none());
        assert_eq!(model_supports_vision(Some(&provider), "k3"), Some(true));
        provider.base_url = "https://example.com/v1".into();
        assert_eq!(provider_model_database_id(Some(&provider), "k3"), "k3");
        assert!(model_database_entry("k3").is_none());
    }

    #[test]
    fn antigravity_web_search_uses_third_party_and_preserves_off() {
        use crate::chat::types::WebSearchMode;
        let mut provider = test_provider_with_overrides(HashMap::new());
        provider.api_format = "gemini".into();
        assert!(builtin_web_search_supported(&provider));
        assert_eq!(
            WebSearchMode::Builtin.for_provider(&provider),
            WebSearchMode::Builtin
        );
        provider.request.oauth = Some(
            serde_json::from_value(serde_json::json!({
                "provider": "antigravity"
            }))
            .unwrap(),
        );
        assert!(!builtin_web_search_supported(&provider));
        assert_eq!(
            WebSearchMode::Builtin.for_provider(&provider),
            WebSearchMode::ThirdParty
        );
        assert_eq!(
            WebSearchMode::Off.for_provider(&provider),
            WebSearchMode::Off
        );
        assert_eq!(
            WebSearchMode::ThirdParty.for_provider(&provider),
            WebSearchMode::ThirdParty
        );
    }

    #[test]
    fn antigravity_oauth_effort_variants_hide_knob_and_ignore_request_overrides() {
        use crate::provider_oauth::antigravity;
        let mut provider = test_provider_with_overrides(HashMap::new());
        provider.api_format = "gemini".into();
        let model = "gemini-3.8-flash-low";
        assert!(!reasoning_efforts_for_model(Some(&provider), model).is_empty());
        provider.request.oauth = Some(
            serde_json::from_value(serde_json::json!({
                "provider": "antigravity"
            }))
            .unwrap(),
        );
        for model in [
            "gemini-3.8-flash-low",
            "gemini-3.8-flash-high",
            "gpt-oss-120b-medium",
        ] {
            assert!(reasoning_efforts_for_model(Some(&provider), model).is_empty());
            let request = antigravity::wrap_request(
                &provider,
                model,
                serde_json::json!({
                    "generationConfig": {"thinkingConfig": {
                        "includeThoughts": true, "thinkingLevel": "HIGH", "thinkingBudget": 8192
                    }}
                }),
            );
            let thinking = &request["request"]["generationConfig"]["thinkingConfig"];
            assert!(thinking.get("thinkingLevel").is_none());
            assert!(thinking.get("thinkingBudget").is_none());
            if model.starts_with("gemini") {
                assert_eq!(thinking["includeThoughts"], true);
            }
        }
        assert!(!reasoning_efforts_for_model(Some(&provider), "gemini-3.8-flash").is_empty());
    }

    #[test]
    fn context_window_uses_model_override_before_name_heuristic() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "deepseek-v4-flash".to_string(),
            ModelInfo {
                context_window: Some(1_048_576),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            context_window_for_model(Some(&provider), "deepseek-v4-flash"),
            (1_048_576, false)
        );
    }

    #[test]
    fn is_image_output_model_matches_image_and_imagen() {
        assert!(is_image_output_model("gemini-3.1-flash-image"));
        assert!(is_image_output_model("models/gemini-2.5-flash-image"));
        assert!(is_image_output_model("Imagen-4"));
        assert!(is_image_output_model("models/imagen-3.0"));
        // 普通文本模型不判为出图（红线：非出图模型不会追加 responseModalities）。
        assert!(!is_image_output_model("gemini-3.1-flash-lite"));
        assert!(!is_image_output_model("models/gemini-2.5-pro"));
        assert!(!is_image_output_model("gpt-5"));
    }

    #[test]
    fn normalize_model_name_lowercases_strips_models_prefix_and_trims() {
        assert_eq!(
            normalize_model_name("Gemini-3.1-Flash-Image"),
            "gemini-3.1-flash-image"
        );
        assert_eq!(
            normalize_model_name("models/Gemini-3.1-Flash-Image"),
            "gemini-3.1-flash-image"
        );
        assert_eq!(normalize_model_name("  MODELS/imagen-4  "), "imagen-4");
        // 裸名与带前缀 / 大小写归一到同一结果。
        assert_eq!(
            normalize_model_name("gemini-3.1-flash-image"),
            normalize_model_name("models/Gemini-3.1-Flash-Image")
        );
    }

    #[test]
    fn override_image_generation_matches_by_normalized_name() {
        // override key 用带前缀 + 大小写变体，仍应命中查询的裸名（此前精确匹配会静默失效）。
        let mut overrides = HashMap::new();
        overrides.insert(
            "models/Gemini-3.1-Flash-Image".to_string(),
            ModelInfo {
                capabilities: Some(crate::settings::ModelCapabilities {
                    image_generation: Some(true),
                    ..Default::default()
                }),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        // 归一化后命中 override 的生图开关。
        assert_eq!(
            override_image_generation(&provider, "gemini-3.1-flash-image"),
            Some(true)
        );
        assert_eq!(
            override_image_generation(&provider, "GEMINI-3.1-FLASH-IMAGE"),
            Some(true)
        );
        assert_eq!(
            model_supports_image_generation(Some(&provider), "gemini-3.1-flash-image"),
            Some(true)
        );
    }

    #[test]
    fn context_window_uses_builtin_model_database_defaults() {
        assert_eq!(
            context_window_for_model(None, "deepseek-v4-flash"),
            (1_048_576, false)
        );
    }

    #[test]
    fn chat_max_output_uses_builtin_model_database_defaults() {
        assert_eq!(
            chat_max_output_tokens_for_model(None, "deepseek-v4-flash"),
            Some(384_000)
        );
        assert_eq!(
            chat_max_output_tokens_for_model(None, "kimi-k3"),
            Some(1_048_576)
        );
    }

    #[test]
    fn chat_max_output_uses_model_override_before_database() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "deepseek-v4-flash".to_string(),
            ModelInfo {
                max_output: Some(65_536),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            chat_max_output_tokens_for_model(Some(&provider), "deepseek-v4-flash"),
            Some(65_536)
        );
    }

    #[test]
    fn chat_max_output_is_absent_when_metadata_is_missing() {
        assert_eq!(chat_max_output_tokens_for_model(None, "custom-model"), None);
        assert_eq!(
            chat_max_output_tokens_for_model(None, "totally-unknown-model-xyz-9999"),
            None
        );
    }

    #[test]
    fn chat_max_output_uses_glm_53_database_cap() {
        assert_eq!(
            chat_max_output_tokens_for_model(None, "glm-5.3"),
            Some(131_072)
        );
        assert_eq!(
            chat_max_output_tokens_for_model(None, "glm-5.3-flash"),
            Some(131_072)
        );
    }

    #[test]
    fn chat_max_output_on_wire_uses_fallback_for_unlisted_models() {
        let provider = test_provider_with_overrides(HashMap::new());
        assert_eq!(
            chat_max_output_tokens_on_wire(Some(&provider), "custom-model", 16_384),
            16_384
        );
        assert_eq!(
            chat_max_output_tokens_on_wire(Some(&provider), "glm-5.3", 16_384),
            131_072
        );
        for model in [
            "[临时测试自有算力]Qwen/Qwen3.8-27B-Uncensored",
            "[自有算力]Qwen/Qwen3.5-9B-Uncensored-HauhauCS-Aggressive",
        ] {
            assert_eq!(
                chat_max_output_tokens_on_wire(Some(&provider), model, 32_768),
                16_384,
                "{model}: self-hosted variants must not inherit a commercial output cap"
            );
        }
    }

    #[test]
    fn chat_max_output_on_wire_keeps_fallback_for_anthropic_too() {
        let mut provider = test_provider_with_overrides(HashMap::new());
        provider.api_format = "anthropic_messages".into();
        assert_eq!(
            chat_max_output_tokens_on_wire(Some(&provider), "claude-mystery", 16_384),
            16_384
        );
    }

    #[test]
    fn temperature_is_absent_when_metadata_is_missing() {
        assert_eq!(temperature_for_model(None, "custom-model"), None);
    }

    #[test]
    fn temperature_uses_model_override() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "custom-model".to_string(),
            ModelInfo {
                temperature: Some(0.4),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            temperature_for_model(Some(&provider), "custom-model"),
            Some(0.4)
        );
    }

    #[test]
    fn temperature_explicit_omit_wins_over_override_value() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "custom-model".to_string(),
            ModelInfo {
                temperature: Some(0.4),
                omit_temperature: Some(true),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(temperature_for_model(Some(&provider), "custom-model"), None);
    }

    #[test]
    fn temperature_ignores_invalid_or_out_of_range_values() {
        for invalid in [f64::NAN, f64::INFINITY, -0.1, 2.1] {
            let mut overrides = HashMap::new();
            overrides.insert(
                "custom-model".to_string(),
                ModelInfo {
                    temperature: Some(invalid),
                    ..ModelInfo::default()
                },
            );
            let provider = test_provider_with_overrides(overrides);
            assert_eq!(
                temperature_for_model(Some(&provider), "custom-model"),
                None,
                "invalid temperature {invalid:?} should be ignored"
            );
        }
    }

    #[test]
    fn request_temperature_is_validated_and_wins_when_valid() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "custom-model".to_string(),
            ModelInfo {
                temperature: Some(0.4),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            temperature_for_request(Some(1.2), Some(&provider), "custom-model"),
            Some(1.2)
        );
        assert_eq!(
            temperature_for_request(Some(3.0), Some(&provider), "custom-model"),
            Some(0.4)
        );
    }

    #[test]
    fn context_window_keeps_name_heuristic_when_metadata_missing() {
        assert_eq!(
            context_window_for_model(None, "custom-200k"),
            (200_000, false)
        );
        assert_eq!(
            context_window_for_model(None, "custom-deepseek-model"),
            (128_000, true)
        );
    }

    #[test]
    fn context_window_recognizes_256k_in_model_name() {
        // 关键词表缺 256k 时这两个会掉到 200K 兜底（kimi k3-256k 的真实症状）。
        assert_eq!(
            context_window_for_model(None, "kimi-code/k3-256k"),
            (262_144, false)
        );
        assert_eq!(
            context_window_for_model(None, "some-model-256K"),
            (262_144, false)
        );
    }
}
