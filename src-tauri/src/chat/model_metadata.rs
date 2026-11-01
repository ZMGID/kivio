use serde_json::Value;

use crate::settings::{ModelInfo, ModelPricing, ModelProvider};

const FALLBACK_CONTEXT_WINDOW_TOKENS: usize = 200_000;
const MIN_TEMPERATURE: f64 = 0.0;
const MAX_TEMPERATURE: f64 = 2.0;

fn context_window_from_model_info(info: Option<&ModelInfo>) -> Option<usize> {
    info.and_then(|info| info.context_window)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn max_output_from_model_info(info: Option<&ModelInfo>) -> Option<u32> {
    info.and_then(|info| info.max_output)
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn valid_temperature(value: f64) -> Option<f64> {
    (value.is_finite() && (MIN_TEMPERATURE..=MAX_TEMPERATURE).contains(&value)).then_some(value)
}

fn temperature_from_model_info(info: Option<&ModelInfo>) -> Option<f64> {
    info.and_then(|info| info.temperature)
        .and_then(valid_temperature)
}

fn model_vision_from_model_info(info: Option<&ModelInfo>) -> Option<bool> {
    info.and_then(|info| info.capabilities.as_ref())
        .and_then(|capabilities| capabilities.vision)
}

fn model_image_generation_from_model_info(info: Option<&ModelInfo>) -> Option<bool> {
    info.and_then(|info| info.capabilities.as_ref())
        .and_then(|capabilities| capabilities.image_generation)
}

/// 版本分隔符归一化：数据库键用点号（`claude-sonnet-4.6`），不少 provider 返回连字符
/// （`claude-sonnet-4-6`）。与前端 `src/data/modelMatching.ts` 的 `normalizeSep` 同构。
fn normalize_model_sep(value: &str) -> String {
    value.replace('.', "-")
}

/// 版本延续判定：DB key 以数字结尾，且候选紧跟 1~2 位纯数字分段（`gpt-5` ← `gpt-5-6-luna`）
/// 时，不能退化到基础版本，否则会把未知次级版本错配成 `gpt-5`。
/// 日期/快照后缀（≥3 位数字段，如 `claude-opus-4-8-20260101`）不算版本延续。
/// 与前端 `isVersionContinuation` 同构。
fn is_version_continuation(key_norm: &str, candidate: &str, end_idx: usize) -> bool {
    let Some(last) = key_norm.chars().last() else {
        return false;
    };
    if !last.is_ascii_digit() {
        return false;
    }
    if candidate.as_bytes().get(end_idx) != Some(&b'-') {
        return false;
    }
    // `str::split` 至少产出一段（可为空），对齐前端 `[0]`。
    let next_segment = candidate[end_idx + 1..].split('-').next().unwrap();
    let len = next_segment.len();
    (1..=2).contains(&len) && next_segment.bytes().all(|b| b.is_ascii_digit())
}

/// 从内置模型库匹配条目。
///
/// 与前端 `matchModel` / `matchModelExact` **同构**（精确 → 归一化精确 → 最长前缀 →
/// 最长包含；均带版本延续保护）。两边必须命中同一条记录，否则 UI 展示的 efforts/定价
/// 会和真实请求侧不一致（例如 `claude-sonnet-4-6` 前端认 4.6、后端退化到 4）。
///
/// 不做生图命名启发式（那是前端 `matchKnownImageGenerationModel` 的展示层兜底）；
/// 也不为某个 CLI 别名（如 kimi 的 `k3`）放宽匹配——那会污染所有 provider。
fn model_database_entry(model: &str) -> Option<Value> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }

    let snapshot = super::model_database::snapshot();
    let entries = &snapshot.database;
    let name = model.to_ascii_lowercase();
    let stripped = name.rsplit('/').next().unwrap_or(name.as_str());

    // 1. 精确匹配（含 OpenRouter 风格 `provider/model` 去前缀）
    if let Some(entry) = entries.get(name.as_str()) {
        return Some(entry.clone());
    }
    if stripped != name.as_str() {
        if let Some(entry) = entries.get(stripped) {
            return Some(entry.clone());
        }
    }

    let index = &snapshot;

    // 1b. 分隔符归一化后的精确匹配（`claude-sonnet-4-6` → `claude-sonnet-4.6`）
    if let Some(orig) = index
        .exact_norm
        .get(&normalize_model_sep(&name))
        .or_else(|| index.exact_norm.get(&normalize_model_sep(stripped)))
    {
        return entries.get(orig).cloned();
    }

    let norm_name = normalize_model_sep(&name);
    let norm_stripped = normalize_model_sep(stripped);
    let candidates: Vec<&str> = if norm_name == norm_stripped {
        vec![norm_stripped.as_str()]
    } else {
        vec![norm_name.as_str(), norm_stripped.as_str()]
    };

    // 2. 前缀匹配（归一化后最长 key 优先，带版本延续保护）
    let mut best_prefix: Option<(&str, usize)> = None;
    for (orig, norm) in &index.pairs {
        let norm_len = norm.len();
        let hit = candidates.iter().any(|candidate| {
            candidate.starts_with(norm.as_str())
                && norm_len < candidate.len()
                && !is_version_continuation(norm, candidate, norm_len)
        });
        if hit && best_prefix.map_or(true, |(_, len)| norm_len > len) {
            best_prefix = Some((orig.as_str(), norm_len));
        }
    }
    if let Some((orig, _)) = best_prefix {
        return entries.get(orig).cloned();
    }

    // 3. 包含匹配（归一化后最长 key 优先，带版本延续保护）
    let mut best_contains: Option<(&str, usize)> = None;
    for (orig, norm) in &index.pairs {
        let norm_len = norm.len();
        let hit = candidates.iter().any(|candidate| {
            if norm.as_str() == *candidate {
                return false;
            }
            match candidate.find(norm.as_str()) {
                Some(idx) => !is_version_continuation(norm, candidate, idx + norm_len),
                None => false,
            }
        });
        if hit && best_contains.map_or(true, |(_, len)| norm_len > len) {
            best_contains = Some((orig.as_str(), norm_len));
        }
    }
    best_contains.and_then(|(orig, _)| entries.get(orig).cloned())
}

/// Resolve Kimi Code aliases only for its own provider, without widening global matching.
fn provider_model_database_id<'a>(provider: Option<&ModelProvider>, model: &'a str) -> &'a str {
    let kimi = provider.is_some_and(|p| {
        p.request
            .oauth
            .as_ref()
            .is_some_and(|a| a.provider == "kimi")
            || reqwest::Url::parse(&p.base_url).ok().is_some_and(|url| {
                url.host_str() == Some("api.kimi.com")
                    && (url.path() == "/coding" || url.path().starts_with("/coding/"))
            })
    });
    if !kimi {
        return model;
    }
    match model.trim().to_ascii_lowercase().as_str() {
        "k3" => "kimi-code/k3",
        "k3-256k" => "kimi-code/k3-256k",
        "kimi-for-coding" => "kimi-code/kimi-for-coding",
        "kimi-for-coding-highspeed" => "kimi-code/kimi-for-coding-highspeed",
        _ => model,
    }
}

fn model_database_context_window(model: &str) -> Option<usize> {
    context_window_from_database_entry(model_database_entry(model).as_ref())
}

fn model_database_max_output(model: &str) -> Option<u32> {
    max_output_from_database_entry(model_database_entry(model).as_ref())
}

fn model_database_temperature(model: &str) -> Option<f64> {
    model_database_entry(model)?
        .get("temperature")
        .and_then(Value::as_f64)
        .and_then(valid_temperature)
}

fn context_window_from_database_entry(entry: Option<&Value>) -> Option<usize> {
    entry?
        .get("contextWindow")
        .and_then(Value::as_u64)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn max_output_from_database_entry(entry: Option<&Value>) -> Option<u32> {
    entry?
        .get("maxOutput")
        .and_then(Value::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok())
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
    let database_id = provider_model_database_id(provider, model);
    let id = normalize_model_name(database_id);
    let kimi_code = id.starts_with("kimi-code/");
    let id = id.rsplit('/').next().unwrap_or(&id);
    let off_mode = if provider.is_some_and(crate::provider_oauth::antigravity::is_provider)
        && crate::provider_oauth::antigravity::model_includes_effort(model)
    {
        ThinkingOffMode::Unsupported
    } else if let Some(mode) = model_database_entry(database_id)
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct GeminiThinkingProfile {
    pub supports_off: bool,
    pub budget_max: Option<i64>,
}

pub(crate) fn gemini_thinking_profile(model: &str) -> Option<GeminiThinkingProfile> {
    let id = normalize_model_name(model);
    if id.contains("gemini-2.5-pro") {
        Some(GeminiThinkingProfile {
            supports_off: false,
            budget_max: Some(32768),
        })
    } else if id.contains("gemini-2.5-flash") {
        Some(GeminiThinkingProfile {
            supports_off: true,
            budget_max: Some(24576),
        })
    } else if id.contains("gemini-3") {
        Some(GeminiThinkingProfile {
            supports_off: false,
            budget_max: None,
        })
    } else {
        None
    }
}

// Product mapping for the shared Low/Medium/High control, within Google's 2.5 budgets.
// These are application presets, not native Gemini effort levels.
pub(crate) fn gemini_thinking_budget(model: &str, level: &str) -> Option<i64> {
    let max = gemini_thinking_profile(model)?.budget_max?;
    match level {
        "low" => Some(1024),
        "medium" => Some(8192),
        "high" => Some(max),
        _ => None,
    }
}

/// Claude 思考线格式。对照官方 per-model 表：
/// https://platform.claude.com/docs/en/build-with-claude/thinking-troubleshooting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaudeThinkingKind {
    /// Claude 3 / 3.5：没有 thinking，发任何 type 都会 400。
    Unsupported,
    /// 4.5 及更早（含 3.7 / Opus 4 / Sonnet 4）：`type: "enabled"` + `budget_tokens`。
    Extended,
    /// 4.6+：`type: "adaptive"` + `output_config.effort`。4.7+ 发 `enabled` 会 400。
    Adaptive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClaudeThinkingProfile {
    pub kind: ClaudeThinkingKind,
    /// 官方 `output_config.effort`。Opus 4.5 是 extended-only 里唯一认 effort 的模型。
    pub supports_output_effort: bool,
    supports_xhigh: bool,
    supports_max: bool,
    /// Off 的 wire 类型：Sonnet 5.5 用 between_tools，5 用 disabled；始终思考的模型省略。
    pub(crate) off_thinking_type: Option<&'static str>,
    /// Fable/Mythos/Preview / Opus 4.7+ / Sonnet 5：非默认 temperature 一律 400。
    pub(crate) forbid_temperature: bool,
}

impl ClaudeThinkingProfile {
    fn ui_effort_levels(self) -> Vec<String> {
        if self.kind == ClaudeThinkingKind::Unsupported {
            return Vec::new();
        }
        let mut out = vec!["low".into(), "medium".into(), "high".into()];
        if self.supports_xhigh {
            out.push("xhigh".into());
        }
        if self.supports_max {
            out.push("max".into());
        }
        out
    }

    pub(crate) fn clamp_effort(self, level: &str) -> &str {
        match level {
            "xhigh" if !self.supports_xhigh => {
                if self.supports_max {
                    "max"
                } else {
                    "high"
                }
            }
            "max" if !self.supports_max => "high",
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeFamily {
    Opus,
    Sonnet,
    Haiku,
    Fable,
    Mythos,
}

impl ClaudeFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Sonnet => "sonnet",
            Self::Haiku => "haiku",
            Self::Fable => "fable",
            Self::Mythos => "mythos",
        }
    }
}

/// 从模型 id 识别 Claude 代际。覆盖官方 id、日期快照、Bedrock 点分 id、2api 短名（`opus-4.5`）。
/// 认不出则返回 None（适配器按 adaptive 兜底；UI 档位走安全子集）。
pub(crate) fn claude_thinking_profile(model: &str) -> Option<ClaudeThinkingProfile> {
    let id = normalize_claude_model_id(model);
    if id.contains("mythos-preview") {
        return Some(ClaudeThinkingProfile {
            kind: ClaudeThinkingKind::Adaptive,
            supports_output_effort: true,
            supports_xhigh: false,
            supports_max: true,
            off_thinking_type: None,
            forbid_temperature: true,
        });
    }
    if id.contains("3-7-sonnet") || id.contains("sonnet-3-7") {
        return Some(extended_budget_only());
    }
    if is_legacy_claude3(&id) {
        return Some(unsupported_thinking());
    }
    let (family, major, minor) = parse_claude_family_version(&id)?;
    if major < 4 {
        return Some(unsupported_thinking());
    }
    Some(profile_for_claude_version(family, major, minor))
}

fn is_legacy_claude3(id: &str) -> bool {
    id.contains("3-5-")
        || id.contains("sonnet-3-5")
        || id.contains("haiku-3-5")
        || id.contains("3-haiku")
        || id.contains("3-opus")
        || id.contains("3-sonnet")
}

fn unsupported_thinking() -> ClaudeThinkingProfile {
    ClaudeThinkingProfile {
        kind: ClaudeThinkingKind::Unsupported,
        supports_output_effort: false,
        supports_xhigh: false,
        supports_max: false,
        off_thinking_type: None,
        forbid_temperature: false,
    }
}

fn extended_budget_only() -> ClaudeThinkingProfile {
    ClaudeThinkingProfile {
        kind: ClaudeThinkingKind::Extended,
        supports_output_effort: false,
        supports_xhigh: false,
        supports_max: false,
        off_thinking_type: None,
        forbid_temperature: false,
    }
}

fn normalize_claude_model_id(model: &str) -> String {
    let name = normalize_model_name(model);
    let stripped = name.rsplit('/').next().unwrap_or(name.as_str());
    let stripped = stripped
        .strip_prefix("anthropic.")
        .unwrap_or(stripped)
        .strip_prefix("anthropic-")
        .unwrap_or(stripped);
    normalize_model_sep(stripped).replace(':', "-")
}

fn parse_claude_family_version(id: &str) -> Option<(ClaudeFamily, u32, u32)> {
    for family in [
        ClaudeFamily::Opus,
        ClaudeFamily::Sonnet,
        ClaudeFamily::Haiku,
        ClaudeFamily::Fable,
        ClaudeFamily::Mythos,
    ] {
        let key = family.as_str();
        let rest = if let Some(stripped) = id.strip_prefix(&format!("{key}-")) {
            stripped
        } else if let Some(idx) = id.find(&format!("-{key}-")) {
            &id[idx + key.len() + 2..]
        } else {
            continue;
        };
        let mut parts = rest.split('-');
        let major = version_segment(parts.next()?)?;
        let minor = parts.next().and_then(version_segment).unwrap_or(0);
        return Some((family, major, minor));
    }
    None
}

fn version_segment(part: &str) -> Option<u32> {
    if (1..=2).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

fn version_at_least(major: u32, minor: u32, req_major: u32, req_minor: u32) -> bool {
    major > req_major || (major == req_major && minor >= req_minor)
}

/// 档位名单来自官方 Effort 页：
/// xhigh = Fable 5 / Mythos 5 / Opus 5 / Opus 4.8 / Opus 4.7 / Sonnet 5
/// max = 上述 + Mythos Preview / Opus 4.6 / Sonnet 4.6
/// effort 本体 = 上述 + Opus 4.5
fn profile_for_claude_version(
    family: ClaudeFamily,
    major: u32,
    minor: u32,
) -> ClaudeThinkingProfile {
    // Haiku 5.5 adds adaptive thinking and output effort; older Haiku uses budgets.
    if family == ClaudeFamily::Haiku && !version_at_least(major, minor, 5, 5) {
        return extended_budget_only();
    }
    let adaptive = match family {
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
        ClaudeFamily::Opus | ClaudeFamily::Sonnet => version_at_least(major, minor, 4, 6),
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
    };
    let kind = if adaptive {
        ClaudeThinkingKind::Adaptive
    } else {
        ClaudeThinkingKind::Extended
    };
    let supports_output_effort = match family {
        ClaudeFamily::Opus => version_at_least(major, minor, 4, 5),
        ClaudeFamily::Sonnet => version_at_least(major, minor, 4, 6),
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
    };
    let supports_xhigh = match family {
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
        ClaudeFamily::Opus => version_at_least(major, minor, 4, 7),
        ClaudeFamily::Sonnet => major >= 5,
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
    };
    let supports_max = match family {
        ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
        ClaudeFamily::Opus | ClaudeFamily::Sonnet => version_at_least(major, minor, 4, 6),
        ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
    };
    ClaudeThinkingProfile {
        kind,
        supports_output_effort,
        supports_xhigh,
        supports_max,
        off_thinking_type: match family {
            ClaudeFamily::Haiku if version_at_least(major, minor, 5, 5) => Some("disabled"),
            ClaudeFamily::Sonnet if version_at_least(major, minor, 5, 5) => Some("between_tools"),
            ClaudeFamily::Sonnet if major >= 5 => Some("disabled"),
            ClaudeFamily::Opus if major >= 5 && !version_at_least(major, minor, 5, 5) => {
                Some("disabled")
            }
            _ => None,
        },
        forbid_temperature: match family {
            ClaudeFamily::Fable | ClaudeFamily::Mythos => major >= 5,
            ClaudeFamily::Opus => version_at_least(major, minor, 4, 7),
            ClaudeFamily::Sonnet => major >= 5,
            ClaudeFamily::Haiku => version_at_least(major, minor, 5, 5),
        },
    }
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

fn model_pricing_from_model_info(info: Option<&ModelInfo>) -> Option<ModelPricing> {
    info.and_then(|info| info.pricing.clone())
}

fn model_database_pricing(model: &str) -> Option<ModelPricing> {
    let entry = model_database_entry(model)?;
    let pricing = entry.get("pricing")?;
    Some(ModelPricing {
        input: pricing.get("input").and_then(Value::as_f64),
        output: pricing.get("output").and_then(Value::as_f64),
        cached_input: pricing.get("cachedInput").and_then(Value::as_f64),
    })
}

pub(crate) fn model_supports_vision(provider: Option<&ModelProvider>, model: &str) -> Option<bool> {
    let provider = provider?;
    model_vision_from_model_info(provider.model_overrides.get(model))
        .or_else(|| model_database_vision(provider_model_database_id(Some(provider), model)))
}

pub(crate) fn model_supports_video(provider: &ModelProvider, model: &str) -> Option<bool> {
    provider
        .model_overrides
        .get(model)
        .and_then(|info| info.capabilities.as_ref())
        .and_then(|caps| caps.video_input)
        .or_else(|| provider.model_overrides.get(model)?.advertised_video_input)
        .or_else(|| {
            model_database_entry(provider_model_database_id(Some(provider), model))?
                .get("capabilities")?
                .get("videoInput")?
                .as_bool()
        })
}

/// 归一化模型名：小写 + 去 `models/` 前缀 + trim。出图路由 / override 生图能力判定 /
/// `is_image_output_model` / 名字启发式统一走这里，消除「换大小写/加 `models/` 前缀就路由错、
/// override 精确匹配静默失效」三类脆弱。
pub(crate) fn normalize_model_name(model: &str) -> String {
    let lower = model.trim().to_ascii_lowercase();
    lower
        .strip_prefix("models/")
        .unwrap_or(&lower)
        .trim()
        .to_string()
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

/// 取该模型的用户覆盖：先精确 `get(model)`，未命中再按**归一化名**遍历匹配
/// （小 HashMap，O(n) 可接受），消除大小写 / `models/` 前缀导致 override 静默失效。
fn override_model_info<'a>(provider: &'a ModelProvider, model: &str) -> Option<&'a ModelInfo> {
    if let Some(info) = provider.model_overrides.get(model) {
        return Some(info);
    }
    let normalized = normalize_model_name(model);
    provider
        .model_overrides
        .iter()
        .find(|(key, _)| normalize_model_name(key) == normalized)
        .map(|(_, info)| info)
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

/// 判定一个模型是否为**出图**模型（其响应会带图片）。用于 Gemini 原生 `generateContent`：
/// 生图模型才追加 `responseModalities:["TEXT","IMAGE"]`（普通文本模型加了会 400）。
/// 判据轻量：归一化名（小写 + 去可选 `models/` 前缀）含 `image` 或以 `imagen` 开头。
pub fn is_image_output_model(model: &str) -> bool {
    let name = normalize_model_name(model);
    name.contains("image") || name.starts_with("imagen")
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

/// 模型级最大输出。覆盖 → 内置模型库；都没有就返回 `None`。
pub(crate) fn chat_max_output_tokens_for_model(
    provider: Option<&ModelProvider>,
    model: &str,
) -> Option<u32> {
    max_output_from_model_info(provider.and_then(|provider| provider.model_overrides.get(model)))
        .or_else(|| model_database_max_output(provider_model_database_id(provider, model)))
}

/// 写入请求体的输出上限。有模型元数据就用；没有就用 `fallback`。
/// 对齐 Pi：自定义模型没填 maxTokens 时一律 16384，不按协议省略。
pub(crate) fn chat_max_output_tokens_on_wire(
    provider: Option<&ModelProvider>,
    model: &str,
    fallback: u32,
) -> u32 {
    chat_max_output_tokens_for_model(provider, model).unwrap_or(fallback)
}

/// 解析模型级 temperature。用户显式清空优先于数据库值；所有来源都缺省时不发送。
pub(crate) fn temperature_for_model(provider: Option<&ModelProvider>, model: &str) -> Option<f64> {
    if let Some(info) = provider.and_then(|provider| provider.model_overrides.get(model)) {
        if info.omit_temperature == Some(true) {
            return None;
        }
        if let Some(temperature) = temperature_from_model_info(Some(info)) {
            return Some(temperature);
        }
    }
    model_database_temperature(model)
}

/// 单次请求显式值优先；非法显式值不会进入请求体，并回退到模型级配置。
pub(crate) fn temperature_for_request(
    explicit: Option<f64>,
    provider: Option<&ModelProvider>,
    model: &str,
) -> Option<f64> {
    explicit
        .and_then(valid_temperature)
        .or_else(|| temperature_for_model(provider, model))
}

pub(crate) fn pricing_for_model(
    provider: Option<&ModelProvider>,
    model: &str,
) -> Option<(ModelPricing, String)> {
    if let Some(pricing) = model_pricing_from_model_info(
        provider.and_then(|provider| provider.model_overrides.get(model)),
    ) {
        return Some((pricing, "user_override".to_string()));
    }
    model_database_pricing(provider_model_database_id(provider, model))
        .map(|pricing| (pricing, "model_pricing".to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn codex_defaults_apply_to_all_providers_and_preserve_overrides() {
        let snapshot = super::super::model_database::snapshot();
        let mut provider = test_provider_with_overrides(Default::default());
        for model in ["gpt-6-astra", "gpt-6.1-sol", "gpt-6.1-terra", "gpt-5.6-sol"] {
            let entry = &snapshot.database[model];
            let context = usize::try_from(entry["contextWindow"].as_u64().unwrap()).unwrap();
            let max_output = entry["maxOutput"]
                .as_u64()
                .and_then(|tokens| u32::try_from(tokens).ok())
                .filter(|tokens| *tokens > 0);
            let namespaced = format!("openai/{model}");
            let namespaced_context = usize::try_from(
                snapshot.database[namespaced.as_str()]["contextWindow"]
                    .as_u64()
                    .unwrap(),
            )
            .unwrap();
            for oauth in [
                None,
                Some(serde_json::from_value(serde_json::json!({"provider": "codex"})).unwrap()),
            ] {
                provider.request.oauth = oauth;
                assert_eq!(
                    context_window_for_model(Some(&provider), model),
                    (context, false)
                );
                assert_eq!(
                    context_window_for_model(Some(&provider), &namespaced),
                    (namespaced_context, false)
                );
                assert_eq!(
                    chat_max_output_tokens_for_model(Some(&provider), model),
                    max_output
                );
                assert_eq!(
                    reasoning_efforts_for_model(Some(&provider), model),
                    catalog_efforts(entry)
                );
            }
            assert_eq!(context_window_for_model(None, model), (context, false));
        }
        let override_context =
            if snapshot.database["gpt-6.1-sol"]["contextWindow"].as_u64() == Some(8192) {
                8193
            } else {
                8192
            };
        provider.model_overrides.insert(
            "gpt-6.1-sol".into(),
            ModelInfo {
                context_window: Some(override_context),
                ..Default::default()
            },
        );
        assert_eq!(
            context_window_for_model(Some(&provider), "gpt-6.1-sol"),
            (override_context as usize, false)
        );
        assert_eq!(
            provider_model_database_id(Some(&provider), "private-model"),
            "private-model"
        );
    }

    #[test]
    fn thinking_capabilities_separate_toggle_effort_and_upfront_only() {
        use ThinkingOffMode::*;
        let mut provider = test_provider_with_overrides(Default::default());
        provider.model_overrides.insert(
            "synthetic-thinking-model".into(),
            ModelInfo {
                capabilities: Some(crate::settings::ModelCapabilities {
                    reasoning: Some(false),
                    ..Default::default()
                }),
                reasoning_efforts: Some(vec!["high".into()]),
                ..Default::default()
            },
        );
        let not_applicable =
            thinking_capabilities_for_model(Some(&provider), "synthetic-thinking-model");
        assert_eq!(not_applicable.off_mode, NotApplicable);
        assert!(not_applicable.levels.is_empty());

        for (model, mode) in [
            ("claude-opus-5.5.99-synthetic", Unsupported),
            ("claude-sonnet-5.99-synthetic", UpfrontOnly),
            ("claude-haiku-5.99-synthetic", Supported),
            ("kimi-k3.99-synthetic", Unsupported),
            ("totally-unknown-model-xyz-9999", Unknown),
        ] {
            assert!(model_database_entry(model).is_none(), "{model}");
            assert_eq!(
                thinking_capabilities_for_model(None, model).off_mode,
                mode,
                "{model}"
            );
        }
        assert!(
            gemini_thinking_profile("gemini-2.5-flash")
                .unwrap()
                .supports_off
        );
        assert!(
            !gemini_thinking_profile("gemini-2.5-pro")
                .unwrap()
                .supports_off
        );
        let snapshot = super::super::model_database::snapshot();
        for model in ["kimi-k2.6", "kimi-k3"] {
            let entry = &snapshot.database[model];
            let expected_mode = if entry["capabilities"]["reasoning"].as_bool() == Some(false) {
                NotApplicable
            } else {
                match entry["thinkingOff"].as_bool() {
                    Some(true) => Supported,
                    Some(false) => Unsupported,
                    None if model == "kimi-k2.6" => Supported,
                    None => Unsupported,
                }
            };
            let capabilities = thinking_capabilities_for_model(None, model);
            assert_eq!(capabilities.off_mode, expected_mode, "{model}");
            let levels = if expected_mode == NotApplicable {
                Vec::new()
            } else {
                catalog_efforts(entry)
            };
            assert_eq!(capabilities.levels, levels, "{model}");
        }

        provider.base_url = "https://api.kimi.com/coding/v1".into();
        let alias = thinking_capabilities_for_model(Some(&provider), "k3");
        let canonical = thinking_capabilities_for_model(Some(&provider), "kimi-code/k3");
        assert_eq!(alias.off_mode, canonical.off_mode);
        assert_eq!(alias.levels, canonical.levels);
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
        let snapshot = super::super::model_database::snapshot();
        let mut provider = test_provider_with_overrides(HashMap::new());
        for (model, key) in [
            ("kimi-k2.7-code", "kimi-k2.7-code"),
            ("kimi-k2.7-code-highspeed", "kimi-k2.7-code-highspeed"),
            ("moonshotai/kimi-k2.7-code", "moonshotai/kimi-k2.7-code"),
            ("gemini-3-pro-preview", "gemini-3-pro-preview"),
            ("models/gemini-3-pro-preview", "gemini-3-pro-preview"),
            ("gemini-3.1-pro-preview", "gemini-3.1-pro-preview"),
            ("gemini-3.8-flash", "gemini-3.8-flash"),
            ("gemini-3-pro-image-preview", "gemini-3-pro-image"),
            ("gemini-embedding-001", "gemini-embedding-001"),
            ("kimi-k2", "kimi-k2"),
            ("kimi-k2.7", "kimi-k2.7"),
        ] {
            let expected = snapshot.database[key]["capabilities"]["videoInput"].as_bool();
            assert_eq!(model_supports_video(&provider, model), expected, "{model}");
        }

        provider.model_overrides.insert(
            "synthetic-video".into(),
            ModelInfo {
                advertised_video_input: Some(true),
                capabilities: Some(crate::settings::ModelCapabilities {
                    video_input: Some(false),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        assert_eq!(
            model_supports_video(&provider, "synthetic-video"),
            Some(false)
        );
        provider
            .model_overrides
            .get_mut("synthetic-video")
            .unwrap()
            .capabilities = None;
        assert_eq!(
            model_supports_video(&provider, "synthetic-video"),
            Some(true)
        );
    }

    fn catalog_efforts(entry: &Value) -> Vec<String> {
        let mut levels: Vec<String> = Vec::new();
        for level in entry["reasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
        {
            if matches!(level, "low" | "medium" | "high" | "xhigh" | "max")
                && !levels.iter().any(|known| known.as_str() == level)
            {
                levels.push(level.to_owned());
            }
        }
        levels
    }

    /// 与前端 `src/data/modelMatching.test.ts` 对齐：连字符版本 / 版本延续 / 日期后缀。
    #[test]
    fn model_database_matching_is_isomorphic_with_frontend() {
        let snapshot = super::super::model_database::snapshot();
        for (model, key) in [
            ("claude-sonnet-4-6", "claude-sonnet-4-6"),
            ("claude-sonnet-4.6", "claude-sonnet-4.6"),
            ("gpt-5", "gpt-5"),
            ("gpt-5-6-luna", "gpt-5.6-luna"),
            ("gpt-5-6-luna-20260101", "gpt-5.6-luna"),
            ("provider/gpt-5-6-luna", "gpt-5.6-luna"),
            (
                "bytedance-seed/seed-2.0-code",
                "bytedance-seed/seed-2.0-code",
            ),
            ("private/seed-2.0-code", "seed-2.0-code"),
            ("kimi-k2-7", "kimi-k2.7"),
            ("hy3", "hy3"),
            ("hy3-preview", "hy3-preview"),
            ("grok-imagine-image", "grok-imagine-image"),
            ("grok-imagine-image-2.0", "grok-imagine-image-2.0"),
            (
                "grok-imagine-image-quality-draft",
                "grok-imagine-image-quality",
            ),
            ("vendor-tag-gemma4:31b", "gemma4:31b"),
        ] {
            assert_eq!(
                model_database_entry(model).as_ref(),
                Some(snapshot.database.get(key).unwrap()),
                "{model} should match {key}"
            );
        }
        assert!(model_database_entry("").is_none());
        assert!(model_database_entry("   ").is_none());
        assert!(model_database_entry("gpt-5.7-nebula").is_none());
        assert!(model_database_entry("totally-unknown-model-xyz-9999").is_none());
        assert!(model_database_entry("k3").is_none());
    }

    /// 连字符 id 必须吃到正确条目的 efforts，不能退化到旧主版本的 `[]`。
    #[test]
    fn dash_versioned_ids_resolve_correct_reasoning_efforts() {
        let snapshot = super::super::model_database::snapshot();
        for model in [
            "claude-sonnet-4-6",
            "anthropic/claude-sonnet-4-6",
            "claude-haiku-4-5",
            "claude-haiku-4.5",
            "claude-opus-4-5",
        ] {
            assert_eq!(
                reasoning_efforts_for_model(None, model),
                catalog_efforts(&snapshot.database[model]),
                "{model}"
            );
        }
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
        let snapshot = super::super::model_database::snapshot();
        assert_eq!(
            reasoning_efforts_for_model(None, "DeepSeek-V4-Flash"),
            catalog_efforts(&snapshot.database["deepseek-v4-flash"])
        );
        assert_eq!(
            reasoning_efforts_for_model(None, "some-random-model"),
            vec!["low", "medium", "high"]
        );
        let mut anthropic = test_provider_with_overrides(HashMap::new());
        for raw in ["anthropic", "anthropic_messages"] {
            anthropic.api_format = raw.to_string();
            assert_eq!(
                reasoning_efforts_for_model(Some(&anthropic), "whatever"),
                vec!["low", "medium", "high"],
                "{raw}"
            );
        }
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
        let snapshot = super::super::model_database::snapshot();
        assert_eq!(
            reasoning_efforts_for_model(Some(&provider), "kimi-k3"),
            catalog_efforts(&snapshot.database["kimi-k3"])
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
        let model = "claude-sonnet-5.99-synthetic";
        assert!(model_database_entry(model).is_none());
        let mut anthropic = test_provider_with_overrides(HashMap::new());
        anthropic.api_format = "anthropic".to_string();
        anthropic.model_overrides.insert(
            model.into(),
            ModelInfo {
                reasoning_efforts: Some(Vec::new()),
                ..Default::default()
            },
        );
        assert!(reasoning_efforts_for_model(Some(&anthropic), model).is_empty());
        anthropic.model_overrides.clear();
        assert_eq!(
            reasoning_efforts_for_model(Some(&anthropic), model),
            vec!["low", "medium", "high", "xhigh", "max"]
        );

        for (model, expected) in [
            ("opus-4.5", vec!["low", "medium", "high"]),
            ("opus-4.6", vec!["low", "medium", "high", "max"]),
            ("opus-4.7", vec!["low", "medium", "high", "xhigh", "max"]),
        ] {
            assert!(model_database_entry(model).is_none(), "{model}");
            assert_eq!(
                reasoning_efforts_for_model(Some(&anthropic), model),
                expected,
                "{model}"
            );
        }
        for model in ["claude-3-5-sonnet-20241022", "claude-3-haiku-20240307"] {
            assert!(model_database_entry(model).is_none(), "{model}");
            assert!(
                reasoning_efforts_for_model(Some(&anthropic), model).is_empty(),
                "{model}"
            );
        }
    }

    #[test]
    fn xhigh_gated_per_model_not_per_protocol() {
        let mut provider = test_provider_with_overrides(HashMap::new());
        for (model, levels) in [
            ("synthetic-basic", vec!["low", "high"]),
            ("synthetic-extended", vec!["low", "high", "xhigh"]),
        ] {
            provider.model_overrides.insert(
                model.into(),
                ModelInfo {
                    reasoning_efforts: Some(levels.iter().map(|level| (*level).into()).collect()),
                    ..Default::default()
                },
            );
            for format in ["openai_chat", "openai_responses"] {
                provider.api_format = format.into();
                assert_eq!(
                    reasoning_efforts_for_model(Some(&provider), model),
                    levels,
                    "{format}"
                );
            }
        }
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
        let snapshot = super::super::model_database::snapshot();
        let entry = &snapshot.database["kimi-code/k3"];
        let mut provider = test_provider_with_overrides(HashMap::new());
        provider.base_url = "https://api.kimi.com/coding/v1".into();
        let context = snapshot.database["kimi-code/k3-256k"]["contextWindow"]
            .as_u64()
            .unwrap();
        assert_eq!(
            context_window_for_model(Some(&provider), "k3-256k"),
            (usize::try_from(context).unwrap(), false)
        );
        assert_eq!(
            reasoning_efforts_for_model(Some(&provider), "k3"),
            catalog_efforts(entry)
        );
        let expected_pricing = entry.get("pricing").map(|pricing| {
            (
                pricing["input"].as_f64(),
                pricing["output"].as_f64(),
                pricing["cachedInput"].as_f64(),
                "model_pricing".to_string(),
            )
        });
        let actual_pricing = pricing_for_model(Some(&provider), "k3")
            .map(|(pricing, source)| (pricing.input, pricing.output, pricing.cached_input, source));
        assert_eq!(actual_pricing, expected_pricing);
        assert_eq!(
            model_supports_vision(Some(&provider), "k3"),
            entry["capabilities"]["vision"].as_bool()
        );
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
        for (model, level) in [
            ("gemini-3.8-flash-low", "high"),
            ("gemini-3.8-flash", "low"),
        ] {
            provider.model_overrides.insert(
                model.into(),
                ModelInfo {
                    reasoning_efforts: Some(vec![level.into()]),
                    ..Default::default()
                },
            );
        }
        assert_eq!(
            reasoning_efforts_for_model(Some(&provider), "gemini-3.8-flash-low"),
            vec!["high"]
        );
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
        assert_eq!(
            reasoning_efforts_for_model(Some(&provider), "gemini-3.8-flash"),
            vec!["low"]
        );
    }

    #[test]
    fn context_window_uses_model_override_before_name_heuristic() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "synthetic-128k".to_string(),
            ModelInfo {
                context_window: Some(12_345),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            context_window_for_model(Some(&provider), "synthetic-128k"),
            (12_345, false)
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
    }

    #[test]
    fn override_image_generation_matches_by_normalized_name() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "models/Synthetic-Image-Generation".to_string(),
            ModelInfo {
                capabilities: Some(crate::settings::ModelCapabilities {
                    image_generation: Some(false),
                    ..Default::default()
                }),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            override_image_generation(&provider, "synthetic-image-generation"),
            Some(false)
        );
        assert_eq!(
            override_image_generation(&provider, "SYNTHETIC-IMAGE-GENERATION"),
            Some(false)
        );
        assert_eq!(
            model_supports_image_generation(Some(&provider), "synthetic-image-generation"),
            Some(false)
        );
    }

    #[test]
    fn chat_max_output_uses_model_override_before_database() {
        let snapshot = super::super::model_database::snapshot();
        let override_max =
            if snapshot.database["deepseek-v4-flash"]["maxOutput"].as_u64() == Some(1) {
                2
            } else {
                1
            };
        let mut overrides = HashMap::new();
        overrides.insert(
            "deepseek-v4-flash".to_string(),
            ModelInfo {
                max_output: Some(override_max),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            chat_max_output_tokens_for_model(Some(&provider), "deepseek-v4-flash"),
            Some(override_max as u32)
        );
    }

    #[test]
    fn chat_max_output_is_absent_when_metadata_is_missing() {
        assert_eq!(chat_max_output_tokens_for_model(None, "custom-model"), None);
    }

    #[test]
    fn chat_max_output_on_wire_uses_fallback_for_unlisted_models() {
        let snapshot = super::super::model_database::snapshot();
        let mut provider = test_provider_with_overrides(HashMap::new());
        assert_eq!(
            chat_max_output_tokens_on_wire(Some(&provider), "custom-model", 16_384),
            16_384
        );
        provider.model_overrides.insert(
            "synthetic-capped".into(),
            ModelInfo {
                max_output: Some(4096),
                ..Default::default()
            },
        );
        assert_eq!(
            chat_max_output_tokens_on_wire(Some(&provider), "synthetic-capped", 16_384),
            4096
        );
        for (model, key) in [
            (
                "[临时测试自有算力]Qwen/Qwen3.8-27B-Uncensored",
                "qwen3.8-27b-uncensored",
            ),
            (
                "[自有算力]Qwen/Qwen3.5-9B-Uncensored-HauhauCS-Aggressive",
                "qwen3.5-9b-uncensored-hauhaucs-aggressive",
            ),
        ] {
            let max_output = snapshot.database[key]["maxOutput"]
                .as_u64()
                .and_then(|tokens| u32::try_from(tokens).ok())
                .filter(|tokens| *tokens > 0)
                .unwrap_or(32_768);
            assert_eq!(
                chat_max_output_tokens_on_wire(Some(&provider), model, 32_768),
                max_output,
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
        // This was missing for names such as kimi k3-256k.
        assert_eq!(
            context_window_for_model(None, "some-model-256K"),
            (262_144, false)
        );
    }
}
