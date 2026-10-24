//! Shared model parameter lookup against the same official/user database.
use crate::settings::{ModelInfo, ModelPricing, ModelProvider};
use serde_json::Value;
const MIN_TEMPERATURE: f64 = 0.0;
const MAX_TEMPERATURE: f64 = 2.0;
pub(super) fn max_output_from_model_info(info: Option<&ModelInfo>) -> Option<u32> {
    info.and_then(|info| info.max_output)
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

pub(super) fn valid_temperature(value: f64) -> Option<f64> {
    (value.is_finite() && (MIN_TEMPERATURE..=MAX_TEMPERATURE).contains(&value)).then_some(value)
}

pub(super) fn temperature_from_model_info(info: Option<&ModelInfo>) -> Option<f64> {
    info.and_then(|info| info.temperature)
        .and_then(valid_temperature)
}

/// 版本分隔符归一化：数据库键用点号（`claude-sonnet-4.6`），不少 provider 返回连字符
/// （`claude-sonnet-4-6`）。与前端 `src/data/modelMatching.ts` 的 `normalizeSep` 同构。
pub(super) fn normalize_model_sep(value: &str) -> String {
    value.replace('.', "-")
}

/// 版本延续判定：DB key 以数字结尾，且候选紧跟 1~2 位纯数字分段（`gpt-5` ← `gpt-5-6-luna`）
/// 时，不能退化到基础版本，否则会把未知次级版本错配成 `gpt-5`。
/// 日期/快照后缀（≥3 位数字段，如 `claude-opus-4-8-20260101`）不算版本延续。
/// 与前端 `isVersionContinuation` 同构。
pub(super) fn is_version_continuation(key_norm: &str, candidate: &str, end_idx: usize) -> bool {
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
pub(super) fn model_database_entry(model: &str) -> Option<Value> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }

    let snapshot = crate::chat::model_database::snapshot();
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
pub(super) fn provider_model_database_id<'a>(
    provider: Option<&ModelProvider>,
    model: &'a str,
) -> &'a str {
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

pub(super) fn model_database_max_output(model: &str) -> Option<u32> {
    max_output_from_database_entry(model_database_entry(model).as_ref())
}

pub(super) fn model_database_temperature(model: &str) -> Option<f64> {
    model_database_entry(model)?
        .get("temperature")
        .and_then(Value::as_f64)
        .and_then(valid_temperature)
}

pub(super) fn max_output_from_database_entry(entry: Option<&Value>) -> Option<u32> {
    entry?
        .get("maxOutput")
        .and_then(Value::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
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

/// 取该模型的用户覆盖：先精确 `get(model)`，未命中再按**归一化名**遍历匹配
/// （小 HashMap，O(n) 可接受），消除大小写 / `models/` 前缀导致 override 静默失效。
pub(super) fn override_model_info<'a>(
    provider: &'a ModelProvider,
    model: &str,
) -> Option<&'a ModelInfo> {
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
    pub(super) fn ui_effort_levels(self) -> Vec<String> {
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

pub fn model_supports_video(provider: &ModelProvider, model: &str) -> Option<bool> {
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

/// Capabilities used by native model-purpose selectors. Uses the same model
/// database matching and explicit provider overrides as request metadata.
pub fn capabilities_for_model(
    provider: &ModelProvider,
    model: &str,
) -> crate::settings::ModelCapabilities {
    let defaults = model_database_entry(provider_model_database_id(Some(provider), model))
        .and_then(|entry| entry.get("capabilities").cloned())
        .and_then(|value| serde_json::from_value::<crate::settings::ModelCapabilities>(value).ok())
        .unwrap_or_default();
    let info = provider.model_overrides.get(model);
    let overrides = info
        .and_then(|info| info.capabilities.clone())
        .unwrap_or_default();
    crate::settings::ModelCapabilities {
        vision: overrides.vision.or(defaults.vision),
        video_input: overrides
            .video_input
            .or_else(|| info.and_then(|info| info.advertised_video_input))
            .or(defaults.video_input),
        function_calling: overrides.function_calling.or(defaults.function_calling),
        reasoning: overrides.reasoning.or(defaults.reasoning),
        streaming: overrides.streaming.or(defaults.streaming),
        web_search: overrides.web_search.or(defaults.web_search),
        image_generation: overrides.image_generation.or(defaults.image_generation),
        video_generation: overrides.video_generation.or(defaults.video_generation),
    }
}

/// 判定一个模型是否为**出图**模型（其响应会带图片）。用于 Gemini 原生 `generateContent`：
/// 生图模型才追加 `responseModalities:["TEXT","IMAGE"]`（普通文本模型加了会 400）。
/// 判据轻量：归一化名（小写 + 去可选 `models/` 前缀）含 `image` 或以 `imagen` 开头。
pub fn is_image_output_model(model: &str) -> bool {
    let name = normalize_model_name(model);
    name.contains("image") || name.starts_with("imagen")
}

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

/// Model editor defaults use the same provider-aware lookup as requests.
pub fn database_model_info(provider: &ModelProvider, model: &str) -> ModelInfo {
    model_database_entry(provider_model_database_id(Some(provider), model))
        .and_then(|entry| serde_json::from_value(entry).ok()).unwrap_or_default()
}

/// Whether an Off choice can actually be honored, independently of effort levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingOffMode {
    Supported,
    UpfrontOnly,
    NotApplicable,
    Unsupported,
    Unknown,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingCapabilities {
    pub levels: Vec<String>,
    pub off_mode: ThinkingOffMode,
}

pub fn thinking_capabilities_for_model(
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

pub(super) const FALLBACK_CONTEXT_WINDOW_TOKENS: usize = 200_000;
pub(super) fn context_window_from_model_info(info: Option<&ModelInfo>) -> Option<usize> {
    info.and_then(|info| info.context_window)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

pub(super) fn model_vision_from_model_info(info: Option<&ModelInfo>) -> Option<bool> {
    info.and_then(|info| info.capabilities.as_ref())
        .and_then(|capabilities| capabilities.vision)
}

pub(super) fn model_database_context_window(model: &str) -> Option<usize> {
    context_window_from_database_entry(model_database_entry(model).as_ref())
}

pub(super) fn context_window_from_database_entry(entry: Option<&Value>) -> Option<usize> {
    entry?
        .get("contextWindow")
        .and_then(Value::as_u64)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

pub(super) fn model_database_vision(model: &str) -> Option<bool> {
    model_database_entry(model)?
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("vision"))
        .and_then(Value::as_bool)
}

pub(crate) fn model_supports_vision(provider: Option<&ModelProvider>, model: &str) -> Option<bool> {
    let provider = provider?;
    model_vision_from_model_info(provider.model_overrides.get(model))
        .or_else(|| model_database_vision(provider_model_database_id(Some(provider), model)))
}

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
