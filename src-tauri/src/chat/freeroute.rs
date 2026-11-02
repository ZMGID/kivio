//! FreeRoute: aggregate official free-tier model vendors and fail over between them.
//!
//! Routing and per-error cooldown ideas are adapted from the MIT-licensed
//! [dsh-freeroute](https://github.com/dushaobindoudou/dsh-freeroute) project
//! (studied at `/tmp/dfr`). This is an independent implementation that fits
//! Kivio's provider/model abstractions.
//!
//! Constraints:
//! - Only official free tiers (OpenRouter `:free`, OpenCode Zen free models,
//!   SenseNova's verified free model, Groq's official free API).
//! - Exactly one API key per vendor (no multi-key rotation).
//! - Keys live in `Settings.free_route` using the same settings.json persistence
//!   as `ModelProvider.api_keys`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::model::{
    generate_with_vendor_provider, stream_with_vendor_provider, GenerateOutput, GenerateRequest,
    ModelError, StreamPart, StreamSink,
};
use crate::settings::{
    FreeRouteConfig, FreeRouteVendorConfig, ModelProvider, ProviderApiFormat, Settings,
};
use crate::state::AppState;

pub const FREEROUTE_BASE_URL: &str = "kivio://freeroute";
pub const AUTO_MODEL: &str = "auto";

/// Auth / config failures cool the vendor for 10 minutes.
pub const AUTH_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// Rate limits cool the vendor for 60 seconds.
pub const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
const OTHER_BACKOFF_BASE: Duration = Duration::from_secs(30);
const OTHER_BACKOFF_CAP: Duration = Duration::from_secs(10 * 60);
/// Abort and fail over if the vendor emits no visible token in this window.
pub const FIRST_TOKEN_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailKind {
    Auth,
    RateLimit,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub struct VendorSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub signup_url: &'static str,
    /// Empty key is allowed (OpenCode Zen anonymous).
    pub allows_anonymous: bool,
    pub default_model: &'static str,
    pub models: &'static [VendorModel],
}

#[derive(Debug, Clone, Copy)]
pub struct VendorModel {
    pub id: &'static str,
    pub name: &'static str,
}

/// Built-in official free-tier vendors. B.AI is omitted: its free quota expired
/// (dsh-freeroute 2026-09-16 live check).
pub const VENDORS: &[VendorSpec] = &[
    VendorSpec {
        id: "opencode",
        name: "OpenCode Zen",
        base_url: "https://opencode.ai/zen/v1",
        signup_url: "https://opencode.ai/zen",
        allows_anonymous: true,
        default_model: "big-pickle",
        models: &[
            VendorModel {
                id: "big-pickle",
                name: "Big Pickle",
            },
            VendorModel {
                id: "deepseek-v4-flash-free",
                name: "DeepSeek V4 Flash (free)",
            },
            VendorModel {
                id: "muse-spark-1.3-contributor-free",
                name: "Muse Spark 1.3 Contributor (free)",
            },
            VendorModel {
                id: "mimo-v2.5-free",
                name: "MiMo v2.5 (free)",
            },
            VendorModel {
                id: "ling-3.0-flash-fin-free",
                name: "Ling 3.0 Flash Fin (free)",
            },
            VendorModel {
                id: "nemotron-3.5-lightning-free",
                name: "Nemotron 3.5 Lightning (free)",
            },
            VendorModel {
                id: "nemotron-3-ultra-free",
                name: "Nemotron 3 Ultra (free)",
            },
        ],
    },
    VendorSpec {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        signup_url: "https://openrouter.ai/keys",
        allows_anonymous: false,
        default_model: "openrouter/free",
        models: &[
            VendorModel {
                id: "openrouter/free",
                name: "OpenRouter Free Router",
            },
            VendorModel {
                id: "stealth/union-alpha",
                name: "Stealth: Union Alpha (free)",
            },
            VendorModel {
                id: "inclusionai/ling-3.0-flash-vl:free",
                name: "Ling 3.0 Flash VL (free)",
            },
            VendorModel {
                id: "google/gemma-4-31b-it:free",
                name: "Gemma 4 31B (free)",
            },
            VendorModel {
                id: "nvidia/nemotron-3-super-120b-a12b:free",
                name: "Nemotron 3 Super 120B (free)",
            },
            VendorModel {
                id: "nvidia/nemotron-3.5-lightning:free",
                name: "Nemotron 3.5 Lightning (free)",
            },
            VendorModel {
                id: "cohere/north-mini-code:free",
                name: "North Mini Code (free)",
            },
            VendorModel {
                id: "poolside/laguna-s-2.1:free",
                name: "Laguna S 2.1 (free)",
            },
        ],
    },
    VendorSpec {
        id: "sensenova",
        name: "SenseNova",
        base_url: "https://token.sensenova.ai/v1",
        signup_url: "https://platform.sensenova.ai",
        allows_anonymous: false,
        default_model: "sensenova-6.8-flash-lite",
        models: &[VendorModel {
            id: "sensenova-6.8-flash-lite",
            name: "SenseNova 6.8 Flash Lite",
        }],
    },
    VendorSpec {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        signup_url: "https://console.groq.com/keys",
        allows_anonymous: false,
        default_model: "llama-3.1-8b-instant",
        models: &[
            VendorModel {
                id: "llama-3.1-8b-instant",
                name: "Llama 3.1 8B Instant",
            },
            VendorModel {
                id: "llama-3.3-70b-versatile",
                name: "Llama 3.3 70B Versatile",
            },
            VendorModel {
                id: "gemma2-9b-it",
                name: "Gemma 2 9B IT",
            },
        ],
    },
];

pub fn is_freeroute_url(base_url: &str) -> bool {
    base_url.trim().trim_end_matches('/') == FREEROUTE_BASE_URL
}

pub fn vendor_spec(id: &str) -> Option<&'static VendorSpec> {
    VENDORS.iter().find(|v| v.id == id)
}

/// Official free-tier filter used when probing a vendor catalog.
pub fn is_vendor_free_model(vendor_id: &str, model: &str) -> bool {
    let model = model.trim();
    if model.is_empty() {
        return false;
    }
    match vendor_id {
        "opencode" => crate::opencode_free::is_free_model(model),
        "openrouter" => {
            model.ends_with(":free") || model == "stealth/union-alpha" || model == "openrouter/free"
        }
        "sensenova" => model == "sensenova-6.8-flash-lite",
        "groq" => true,
        _ => false,
    }
}

/// Keep the first non-empty token so a paste of several keys cannot become rotation.
pub fn single_api_key(raw: &str) -> String {
    raw.split(['\n', '\r', ','])
        .map(str::trim)
        .find(|part| !part.is_empty())
        .unwrap_or("")
        .to_string()
}

pub fn default_vendor_configs() -> Vec<FreeRouteVendorConfig> {
    VENDORS
        .iter()
        .map(|v| FreeRouteVendorConfig {
            id: v.id.to_string(),
            enabled: true,
            api_key: String::new(),
        })
        .collect()
}

pub fn sanitize_config(config: FreeRouteConfig) -> FreeRouteConfig {
    let mut kept: Vec<FreeRouteVendorConfig> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for vendor in config.vendors {
        let id = vendor.id.trim().to_string();
        if vendor_spec(&id).is_none() || !seen.insert(id.clone()) {
            continue;
        }
        kept.push(FreeRouteVendorConfig {
            id,
            enabled: vendor.enabled,
            api_key: single_api_key(&vendor.api_key),
        });
    }
    for spec in VENDORS {
        if !seen.contains(spec.id) {
            kept.push(FreeRouteVendorConfig {
                id: spec.id.to_string(),
                enabled: true,
                api_key: String::new(),
            });
        }
    }
    FreeRouteConfig { vendors: kept }
}

pub fn catalog_model_ids(config: &FreeRouteConfig) -> Vec<String> {
    let mut ids = vec![AUTO_MODEL.to_string()];
    for vendor in ready_vendors(config) {
        let Some(spec) = vendor_spec(&vendor.id) else {
            continue;
        };
        for model in spec.models {
            ids.push(format!("{}/{}", spec.id, model.id));
        }
    }
    ids
}

pub fn vendor_is_ready(spec: &VendorSpec, vendor: &FreeRouteVendorConfig) -> bool {
    vendor.enabled && (spec.allows_anonymous || !vendor.api_key.trim().is_empty())
}

fn ready_vendors(config: &FreeRouteConfig) -> Vec<&FreeRouteVendorConfig> {
    config
        .vendors
        .iter()
        .filter(|vendor| {
            vendor_spec(&vendor.id).is_some_and(|spec| vendor_is_ready(spec, vendor))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCandidate {
    pub vendor_id: String,
    pub vendor_name: String,
    pub model: String,
}

/// Rank failover candidates. A pinned `vendor/model` is tried first; `auto`
/// walks each ready vendor's default model in settings order.
pub fn rank_candidates(config: &FreeRouteConfig, requested: &str) -> Vec<RouteCandidate> {
    let requested = requested.trim();
    let ready = ready_vendors(config);
    let mut out = Vec::new();
    let mut push = |vendor: &FreeRouteVendorConfig, model: &str| {
        let Some(spec) = vendor_spec(&vendor.id) else {
            return;
        };
        if !is_vendor_free_model(spec.id, model) {
            return;
        }
        let key = (spec.id.to_string(), model.to_string());
        if out
            .iter()
            .any(|c: &RouteCandidate| c.vendor_id == key.0 && c.model == key.1)
        {
            return;
        }
        out.push(RouteCandidate {
            vendor_id: spec.id.to_string(),
            vendor_name: spec.name.to_string(),
            model: model.to_string(),
        });
    };

    if !requested.is_empty() && requested != AUTO_MODEL {
        if let Some((vendor_id, model)) = split_pinned(requested) {
            if let Some(vendor) = ready.iter().find(|v| v.id == vendor_id) {
                push(vendor, model);
            }
        } else {
            for vendor in &ready {
                if vendor_spec(&vendor.id).is_some_and(|spec| {
                    spec.models.iter().any(|m| m.id == requested)
                        || is_vendor_free_model(spec.id, requested)
                }) {
                    push(vendor, requested);
                }
            }
        }
    }

    for vendor in &ready {
        if let Some(spec) = vendor_spec(&vendor.id) {
            push(vendor, spec.default_model);
        }
    }
    out
}

fn split_pinned(requested: &str) -> Option<(&str, &str)> {
    let (vendor, model) = requested.split_once('/')?;
    if vendor_spec(vendor).is_some() && !model.is_empty() {
        Some((vendor, model))
    } else {
        None
    }
}

pub fn classify_error(err: &str) -> FailKind {
    match crate::api::extract_status_code(err) {
        Some(401..=403) => FailKind::Auth,
        Some(429) => FailKind::RateLimit,
        _ => FailKind::Other,
    }
}

pub fn cooldown_for(kind: FailKind, consecutive_failures: u32) -> Duration {
    match kind {
        FailKind::Auth => AUTH_COOLDOWN,
        FailKind::RateLimit => RATE_LIMIT_COOLDOWN,
        FailKind::Other => {
            let shift = consecutive_failures.saturating_sub(1).min(8);
            let secs = OTHER_BACKOFF_BASE
                .as_secs()
                .saturating_mul(1u64 << shift)
                .min(OTHER_BACKOFF_CAP.as_secs());
            Duration::from_secs(secs)
        }
    }
}

pub fn synthetic_provider(
    aggregator: &ModelProvider,
    spec: &VendorSpec,
    api_key: &str,
) -> ModelProvider {
    let key = single_api_key(api_key);
    ModelProvider {
        id: format!("freeroute:{}", spec.id),
        name: spec.name.to_string(),
        api_keys: if key.is_empty() { Vec::new() } else { vec![key] },
        api_key_legacy: None,
        base_url: spec.base_url.to_string(),
        available_models: spec.models.iter().map(|m| m.id.to_string()).collect(),
        enabled_models: spec.models.iter().map(|m| m.id.to_string()).collect(),
        enabled: true,
        api_format: ProviderApiFormat::OpenAiChat.as_str().to_string(),
        model_overrides: aggregator.model_overrides.clone(),
        compress_request_body: aggregator.compress_request_body,
        request: aggregator.request.clone(),
        active_key_index: 0,
    }
}

struct FirstTokenWatchSink<'a> {
    inner: &'a mut (dyn StreamSink + Send),
    first: Arc<AtomicBool>,
}

impl StreamSink for FirstTokenWatchSink<'_> {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        if is_visible_first_token(&part) {
            self.first.store(true, Ordering::SeqCst);
        }
        self.inner.emit(part)
    }
}

fn is_visible_first_token(part: &StreamPart) -> bool {
    match part {
        StreamPart::TextDelta { delta } | StreamPart::ReasoningDelta { delta } => !delta.is_empty(),
        StreamPart::ToolCallStart { .. } | StreamPart::ToolCallDelta { .. } => true,
        _ => false,
    }
}

fn first_token_timeout_error(vendor_id: &str, model: &str, timeout: Duration) -> ModelError {
    ModelError::new(format!(
        "FreeRoute vendor {vendor_id} model {model} produced no token within {}s",
        timeout.as_secs()
    ))
}

async fn stream_vendor_with_first_token_timeout(
    state: &AppState,
    provider: &ModelProvider,
    vendor_id: &str,
    model: &str,
    request: GenerateRequest,
    sink: &mut (dyn StreamSink + Send),
    timeout: Duration,
) -> (Result<GenerateOutput, ModelError>, bool) {
    let first = Arc::new(AtomicBool::new(false));
    let mut watch = FirstTokenWatchSink {
        inner: sink,
        first: first.clone(),
    };
    // Concrete endpoint. The top-level stream entry re-enters FreeRoute and
    // makes `dispatch` an infinitely sized async fn (E0733).
    let fut = stream_with_vendor_provider(state, provider, 1, request, &mut watch);
    tokio::pin!(fut);
    let sleep = tokio::time::sleep(timeout);
    tokio::pin!(sleep);
    // One select is enough: the stream arm completes the call, and the sleep
    // arm is disabled once a visible token has arrived. A loop would always
    // break on the first iteration (clippy::never_loop).
    let result = tokio::select! {
        biased;
        result = &mut fut => result,
        _ = &mut sleep, if !first.load(Ordering::SeqCst) => {
            Err(first_token_timeout_error(vendor_id, model, timeout))
        }
    };
    (result, first.load(Ordering::SeqCst))
}

fn record_attempt_outcome(
    state: &AppState,
    vendor_id: &str,
    result: &Result<GenerateOutput, ModelError>,
    produced: bool,
) {
    match result {
        Ok(_) => state.provider_runtime().freeroute_success(vendor_id),
        Err(err) if err.is_cancelled() => {}
        Err(_) if produced => {}
        Err(err) => {
            let kind = classify_error(&err.message);
            state.provider_runtime().freeroute_failure(vendor_id, kind);
        }
    }
}

fn skip_cooling<'a>(
    state: &AppState,
    candidates: &'a [RouteCandidate],
) -> Vec<&'a RouteCandidate> {
    let now = Instant::now();
    let ready: Vec<&RouteCandidate> = candidates
        .iter()
        .filter(|c| !state.provider_runtime().freeroute_cooling(&c.vendor_id, now))
        .collect();
    if ready.is_empty() {
        candidates.iter().collect()
    } else {
        ready
    }
}

fn remember_served(state: &AppState, request: &GenerateRequest, candidate: &RouteCandidate) {
    let Some(conversation_id) = request.metadata.conversation_id.as_deref() else {
        return;
    };
    let Some(message_id) = request.metadata.message_id.as_deref() else {
        return;
    };
    state.provider_runtime().remember_freeroute_served(
        conversation_id,
        message_id,
        &candidate.vendor_id,
        &candidate.vendor_name,
        &candidate.model,
    );
}

pub fn apply_served_attribution(state: &AppState, conversation_id: &str, message: &mut crate::chat::ChatMessage) {
    let Some(served) = state
        .provider_runtime()
        .take_freeroute_served(conversation_id, &message.id)
    else {
        return;
    };
    message.model = Some(served.model);
    message.served_vendor = Some(served.vendor_name);
}

pub async fn generate(
    state: &AppState,
    aggregator: &ModelProvider,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    let settings = state.settings_read().clone();
    dispatch(state, aggregator, &settings, request, None).await
}

pub async fn stream(
    state: &AppState,
    aggregator: &ModelProvider,
    request: GenerateRequest,
    sink: &mut (dyn StreamSink + Send),
) -> Result<GenerateOutput, ModelError> {
    let settings = state.settings_read().clone();
    dispatch(state, aggregator, &settings, request, Some(sink)).await
}

async fn dispatch(
    state: &AppState,
    aggregator: &ModelProvider,
    settings: &Settings,
    request: GenerateRequest,
    mut sink: Option<&mut (dyn StreamSink + Send)>,
) -> Result<GenerateOutput, ModelError> {
    let candidates = rank_candidates(&settings.free_route, &request.model);
    if candidates.is_empty() {
        return Err(ModelError::new(
            "FreeRoute has no ready vendor. Add an OpenRouter / Groq / SenseNova key, or use OpenCode Zen (no key).",
        ));
    }
    let ordered: Vec<RouteCandidate> = skip_cooling(state, &candidates)
        .into_iter()
        .cloned()
        .collect();
    let mut last_err: Option<ModelError> = None;
    let remaining = ordered.len();
    for (index, candidate) in ordered.iter().enumerate() {
        let Some(spec) = vendor_spec(&candidate.vendor_id) else {
            continue;
        };
        let Some(vendor) = settings
            .free_route
            .vendors
            .iter()
            .find(|v| v.id == candidate.vendor_id)
        else {
            continue;
        };
        let synthetic = synthetic_provider(aggregator, spec, &vendor.api_key);
        let mut hop = request.clone();
        hop.model = candidate.model.clone();
        hop.metadata.label = format!("FreeRoute {}", spec.name);
        let (result, produced_tokens) = if let Some(sink) = sink.as_mut() {
            stream_vendor_with_first_token_timeout(
                state,
                &synthetic,
                &candidate.vendor_id,
                &candidate.model,
                hop,
                *sink,
                FIRST_TOKEN_TIMEOUT,
            )
            .await
        } else {
            // Same constraint as the stream hop: do not re-enter FreeRoute.
            let result = generate_with_vendor_provider(state, &synthetic, 1, hop).await;
            let produced = result.is_ok();
            (result, produced)
        };
        record_attempt_outcome(state, &candidate.vendor_id, &result, produced_tokens);
        match result {
            Ok(output) => {
                remember_served(state, &request, candidate);
                return Ok(output);
            }
            Err(err) if err.is_cancelled() => return Err(err),
            Err(err) if produced_tokens => return Err(err),
            Err(err) => {
                last_err = Some(err);
                if index + 1 == remaining {
                    break;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| ModelError::new("FreeRoute exhausted every ready vendor")))
}

pub fn sanitize_freeroute_provider(provider: &mut ModelProvider) {
    if !is_freeroute_url(&provider.base_url) {
        return;
    }
    provider.base_url = FREEROUTE_BASE_URL.to_string();
    provider.api_format = ProviderApiFormat::OpenAiChat.as_str().to_string();
    provider.api_keys.clear();
    provider.api_key_legacy = None;
    provider.active_key_index = 0;
    if !provider.available_models.iter().any(|m| m == AUTO_MODEL) {
        provider.available_models.insert(0, AUTO_MODEL.to_string());
    }
    if !provider.enabled_models.iter().any(|m| m == AUTO_MODEL) {
        provider.enabled_models.insert(0, AUTO_MODEL.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(keys: &[(&str, bool, &str)]) -> FreeRouteConfig {
        let mut config = FreeRouteConfig {
            vendors: default_vendor_configs(),
        };
        for vendor in &mut config.vendors {
            vendor.enabled = false;
            vendor.api_key.clear();
        }
        for (id, enabled, key) in keys {
            if let Some(vendor) = config.vendors.iter_mut().find(|v| v.id == *id) {
                vendor.enabled = *enabled;
                vendor.api_key = (*key).to_string();
            }
        }
        config
    }

    #[test]
    fn only_official_free_models_pass_the_filter() {
        assert!(is_vendor_free_model("openrouter", "google/gemma-4-31b-it:free"));
        assert!(is_vendor_free_model("openrouter", "stealth/union-alpha"));
        assert!(!is_vendor_free_model("openrouter", "openai/gpt-4o"));
        assert!(is_vendor_free_model("opencode", "big-pickle"));
        assert!(is_vendor_free_model("opencode", "mimo-v2.5-free"));
        assert!(!is_vendor_free_model("opencode", "ox-alpha-free"));
        assert!(is_vendor_free_model(
            "sensenova",
            "sensenova-6.8-flash-lite"
        ));
        assert!(!is_vendor_free_model("sensenova", "sensenova-6.7-flash"));
        assert!(is_vendor_free_model("groq", "llama-3.1-8b-instant"));
    }

    #[test]
    fn single_api_key_keeps_the_first_non_empty_line() {
        assert_eq!(single_api_key("  sk-one  \nsk-two\n"), "sk-one");
        assert_eq!(single_api_key("sk-one,sk-two"), "sk-one");
        assert_eq!(single_api_key("\n  \n"), "");
        assert_eq!(single_api_key("sk-only"), "sk-only");
    }

    #[test]
    fn sanitize_drops_unknown_vendors_and_seeds_missing_ones() {
        let cleaned = sanitize_config(FreeRouteConfig {
            vendors: vec![
                FreeRouteVendorConfig {
                    id: "openrouter".into(),
                    enabled: false,
                    api_key: "sk-a\nsk-b".into(),
                },
                FreeRouteVendorConfig {
                    id: "not-a-vendor".into(),
                    enabled: true,
                    api_key: "x".into(),
                },
            ],
        });
        assert_eq!(cleaned.vendors.len(), VENDORS.len());
        let openrouter = cleaned
            .vendors
            .iter()
            .find(|v| v.id == "openrouter")
            .unwrap();
        assert!(!openrouter.enabled);
        assert_eq!(openrouter.api_key, "sk-a");
        assert!(cleaned.vendors.iter().any(|v| v.id == "opencode"));
        assert!(cleaned.vendors.iter().all(|v| v.id != "not-a-vendor"));
    }

    #[test]
    fn auto_chain_uses_ready_vendors_in_settings_order() {
        let config = config_with(&[
            ("openrouter", true, "sk-or"),
            ("opencode", true, ""),
            ("groq", true, ""),
            ("sensenova", true, "sk-sn"),
        ]);
        let ranked = rank_candidates(&config, "auto");
        assert_eq!(
            ranked
                .iter()
                .map(|c| c.vendor_id.as_str())
                .collect::<Vec<_>>(),
            vec!["opencode", "openrouter", "sensenova"]
        );
        assert_eq!(ranked[0].model, "big-pickle");
        assert_eq!(ranked[1].model, "openrouter/free");
    }

    #[test]
    fn pinned_model_is_tried_before_the_auto_chain() {
        let config = config_with(&[
            ("openrouter", true, "sk-or"),
            ("opencode", true, ""),
        ]);
        let ranked = rank_candidates(&config, "openrouter/google/gemma-4-31b-it:free");
        assert_eq!(ranked[0].vendor_id, "openrouter");
        assert_eq!(ranked[0].model, "google/gemma-4-31b-it:free");
        assert!(ranked.iter().any(|c| c.vendor_id == "opencode"));
    }

    #[test]
    fn paid_openrouter_pin_is_rejected() {
        let config = config_with(&[("openrouter", true, "sk-or")]);
        let ranked = rank_candidates(&config, "openrouter/openai/gpt-4o");
        assert!(ranked
            .iter()
            .all(|c| c.model != "openai/gpt-4o"));
    }

    #[test]
    fn classify_matches_status_codes() {
        assert_eq!(
            classify_error("Chat stream Error: 401 Unauthorized"),
            FailKind::Auth
        );
        assert_eq!(
            classify_error("Chat stream Error: 403 Forbidden"),
            FailKind::Auth
        );
        assert_eq!(
            classify_error("Chat stream Error: 429 Too Many Requests"),
            FailKind::RateLimit
        );
        assert_eq!(
            classify_error("Chat stream Error: 500 Internal"),
            FailKind::Other
        );
        assert_eq!(
            classify_error("connection reset"),
            FailKind::Other
        );
    }

    #[test]
    fn cooldown_durations_match_the_spec() {
        assert_eq!(cooldown_for(FailKind::Auth, 1), AUTH_COOLDOWN);
        assert_eq!(cooldown_for(FailKind::RateLimit, 8), RATE_LIMIT_COOLDOWN);
        assert_eq!(cooldown_for(FailKind::Other, 1), Duration::from_secs(30));
        assert_eq!(cooldown_for(FailKind::Other, 2), Duration::from_secs(60));
        assert_eq!(cooldown_for(FailKind::Other, 3), Duration::from_secs(120));
        assert_eq!(cooldown_for(FailKind::Other, 20), OTHER_BACKOFF_CAP);
    }

    #[test]
    fn anonymous_opencode_is_ready_without_a_key() {
        let config = config_with(&[("opencode", true, "")]);
        assert_eq!(rank_candidates(&config, "auto").len(), 1);
        let config = config_with(&[("openrouter", true, "")]);
        assert!(rank_candidates(&config, "auto").is_empty());
    }

    #[test]
    fn catalog_lists_auto_and_pinned_vendor_models() {
        let config = config_with(&[("openrouter", true, "sk-or")]);
        let ids = catalog_model_ids(&config);
        assert_eq!(ids[0], "auto");
        assert!(ids.iter().any(|id| id == "openrouter/openrouter/free"));
        assert!(!ids.iter().any(|id| id.starts_with("groq/")));
    }

    #[test]
    fn synthetic_provider_holds_exactly_one_key() {
        let aggregator = ModelProvider {
            id: "fr".into(),
            name: "FreeRoute".into(),
            api_keys: Vec::new(),
            api_key_legacy: None,
            base_url: FREEROUTE_BASE_URL.into(),
            available_models: vec!["auto".into()],
            enabled_models: vec!["auto".into()],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let spec = vendor_spec("openrouter").unwrap();
        let provider = synthetic_provider(&aggregator, spec, "sk-a\nsk-b");
        assert_eq!(provider.api_keys, vec!["sk-a".to_string()]);
        assert_eq!(provider.base_url, spec.base_url);
        assert!(!provider.is_opencode_free());
    }

    #[test]
    fn first_visible_token_ignores_empty_keepalives() {
        use crate::chat::model::StreamPart;
        assert!(super::is_visible_first_token(&StreamPart::TextDelta {
            delta: "hi".into(),
        }));
        assert!(super::is_visible_first_token(&StreamPart::ReasoningDelta {
            delta: "think".into(),
        }));
        assert!(!super::is_visible_first_token(&StreamPart::TextDelta {
            delta: "".into(),
        }));
        assert!(super::is_visible_first_token(&StreamPart::ToolCallStart {
            id: "c1".into(),
            name: "read".into(),
        }));
    }
}
