use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::{Duration as ChronoDuration, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use crate::{
    chat::model::{GenerateRequest, ModelUsage},
    chat::model_metadata,
    settings::ModelProvider,
    state::AppState,
};

const USAGE_DIR_NAME: &str = "usage";
const DEFAULT_LOG_LIMIT: usize = 100;
const MAX_LOG_LIMIT: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRecord {
    pub id: String,
    pub created_at: i64,
    pub completed_at: i64,
    pub duration_ms: u64,
    #[serde(default)]
    pub first_token_ms: Option<u64>,
    pub source: String,
    pub operation: String,
    pub provider_id: String,
    pub provider_name: String,
    pub model: String,
    pub api_format: String,
    pub status: String,
    #[serde(default)]
    pub status_code: Option<u16>,
    pub usage_source: String,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
    #[serde(default)]
    pub cached_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub cache_creation_input_tokens_long: Option<u64>,
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    pub cost_source: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub error_kind: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatsQuery {
    #[serde(default = "default_range")]
    pub range: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub provider_search: Option<String>,
    #[serde(default)]
    pub model_search: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub session_count: u64,
    pub unpriced_requests: u64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub missing_usage_requests: u64,
    pub provider_reported_requests: u64,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_cost_usd: f64,
    pub average_duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTrendPoint {
    pub date: String,
    pub label: String,
    pub requests: u64,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroupStats {
    pub session_count: u64,
    pub unpriced_requests: u64,
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub request_count: u64,
    pub success_count: u64,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cost_usd: f64,
    pub average_duration_ms: Option<f64>,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStatsResponse {
    pub summary: UsageSummary,
    pub trend: Vec<UsageTrendPoint>,
    pub logs: Vec<UsageRecord>,
    pub provider_stats: Vec<UsageGroupStats>,
    pub model_stats: Vec<UsageGroupStats>,
    pub total_logs: usize,
    pub skipped_records: usize,
}

pub struct UsageRecordInput<'a> {
    pub provider: &'a ModelProvider,
    pub model: &'a str,
    pub source: &'a str,
    pub operation: &'a str,
    pub status: &'a str,
    pub status_code: Option<u16>,
    pub usage: Option<ModelUsage>,
    pub usage_source: &'a str,
    pub started_at: i64,
    pub duration_ms: u64,
    pub first_token_ms: Option<u64>,
    pub reasoning_effort: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub error_kind: Option<String>,
}

fn default_range() -> String {
    "7d".to_string()
}

pub fn usage_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir unavailable: {e}"))?
        .join(USAGE_DIR_NAME);
    fs::create_dir_all(&dir).map_err(|e| format!("create usage dir: {e}"))?;
    Ok(dir)
}

pub fn record_model_call(state: &AppState, input: UsageRecordInput<'_>) {
    let completed_at = Local::now().timestamp();
    let usage = input.usage;
    let input_tokens = usage.as_ref().and_then(|usage| usage.input_tokens);
    let output_tokens = usage.as_ref().and_then(|usage| usage.output_tokens);
    let total_tokens = usage
        .as_ref()
        .and_then(|usage| usage.total_tokens)
        .or_else(|| {
            input_tokens
                .zip(output_tokens)
                .map(|(a, b)| a.saturating_add(b))
        });
    let cached_input_tokens = usage.as_ref().and_then(|usage| usage.cached_input_tokens);
    let cache_creation_input_tokens = usage
        .as_ref()
        .and_then(|usage| usage.cache_creation_input_tokens);
    let reasoning_tokens = usage.as_ref().and_then(|usage| usage.reasoning_tokens);
    let mut record = UsageRecord {
        id: format!("usage_{}", Uuid::new_v4()),
        created_at: input.started_at,
        completed_at,
        duration_ms: input.duration_ms,
        first_token_ms: input.first_token_ms,
        source: input.source.to_string(),
        operation: input.operation.to_string(),
        provider_id: input.provider.id.clone(),
        provider_name: input.provider.name.clone(),
        model: input.model.to_string(),
        api_format: input.provider.api_format.clone(),
        status: input.status.to_string(),
        status_code: input.status_code,
        usage_source: if usage.is_some() {
            input.usage_source.to_string()
        } else {
            "missing".to_string()
        },
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
        cache_creation_input_tokens,
        cache_creation_input_tokens_long: usage
            .as_ref()
            .and_then(|usage| usage.cache_creation_input_tokens_long),
        reasoning_tokens,
        reasoning_effort: input.reasoning_effort,
        service_tier: None,
        cost_usd: None,
        cost_source: "unavailable".to_string(),
        conversation_id: input.conversation_id,
        message_id: input.message_id,
        error_kind: input.error_kind,
    };
    pricing::ensure_loaded(&state.usage_dir);
    apply_cost(input.provider, &mut record);
    if let Err(err) = state.usage_ledger.append(&state.usage_dir, &record) {
        eprintln!("Failed to record usage: {err}");
    }
}

/// Explicit effort sent for this request. `None` means the provider/model default was used.
pub fn reasoning_effort_for_request(request: &GenerateRequest) -> Option<String> {
    if request.options.thinking_enabled {
        request.options.thinking_level.clone()
    } else {
        Some("none".to_string())
    }
}

fn apply_cost(provider: &ModelProvider, record: &mut UsageRecord) {
    if let Some((cost, source)) = estimate_model_cost(
        provider,
        &record.model,
        &ModelUsage {
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cached_input_tokens: record.cached_input_tokens,
            cache_creation_input_tokens: record.cache_creation_input_tokens,
            cache_creation_input_tokens_long: record.cache_creation_input_tokens_long,
            ..Default::default()
        },
    ) {
        record.cost_usd = Some(cost);
        record.cost_source = source;
    }
}

/// Configured-rate estimate shared by the ledger and subagent budgets, not a vendor invoice.
pub(crate) fn estimate_model_cost(
    provider: &ModelProvider,
    model: &str,
    usage: &ModelUsage,
) -> Option<(f64, String)> {
    let configured = model_metadata::pricing_for_model(Some(provider), model)
        .filter(|(_, source)| source == "user_override");
    let Some((pricing, source)) = configured else {
        return pricing::estimate(model, usage, &provider.api_format, None)
            .map(|cost| (cost, "litellm".into()));
    };
    let input_price = pricing.input.unwrap_or(0.0);
    let output_price = pricing.output.unwrap_or(0.0);
    let cached_price = pricing.cached_input.unwrap_or(input_price);
    let model = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let (short_multiplier, long_multiplier) = if model.starts_with("claude-") {
        (1.25, 2.0)
    } else if kivio_agent_sdk::model::openai::supports_explicit_prompt_cache(&model) {
        (1.25, 2.5)
    } else {
        (1.0, 1.0)
    };
    let short_price = pricing
        .cache_write
        .unwrap_or(input_price * short_multiplier);
    let long_price = pricing
        .cache_write_long
        .unwrap_or(input_price * long_multiplier);
    let input = usage.input_tokens.unwrap_or(0);
    let mut read = usage.cached_input_tokens.unwrap_or(0);
    let mut write = usage.cache_creation_input_tokens.unwrap_or(0);
    let ordinary = if provider.api_format == "anthropic_messages" {
        input
    } else {
        read = read.min(input);
        write = write.min(input - read);
        input - read - write
    };
    let long = usage
        .cache_creation_input_tokens_long
        .unwrap_or_else(|| {
            if provider.cache_retention() == crate::settings::CacheRetention::Long
                && !(kivio_agent_sdk::model::openai::supports_explicit_prompt_cache(&model)
                    && provider.api_format == "openai_chat")
            {
                write
            } else {
                0
            }
        })
        .min(write);
    let cost = (ordinary as f64 * input_price
        + read as f64 * cached_price
        + (write - long) as f64 * short_price
        + long as f64 * long_price
        + usage.output_tokens.unwrap_or(0) as f64 * output_price)
        / 1_000_000.0;
    Some((cost, source))
}

/// A small read-only snapshot for the desktop companion; shares the statistics
/// page's local-day filter, token accounting and recorded cost values.
pub struct TodayUsage {
    pub date: chrono::NaiveDate,
    pub summary: UsageSummary,
    pub missing_cost_requests: usize,
    pub skipped_records: usize,
}

pub fn today_usage(dir: &Path) -> Result<TodayUsage, String> {
    store::today(dir)
}

#[tauri::command]
pub async fn usage_get_conversation_cost(
    app: AppHandle,
    conversation_id: String,
) -> Result<ConversationCost, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state
            .usage_ledger
            .conversation_cost(&state.usage_dir, &conversation_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCost {
    pub cost_usd: Option<f64>,
    pub unpriced_requests: usize,
    pub skipped_records: usize,
}

mod native;
mod pricing;
mod store;
pub(crate) use store::UsageLedger;

#[tauri::command]
pub async fn usage_get_stats(
    app: AppHandle,
    query: Option<UsageStatsQuery>,
) -> Result<UsageStatsResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state
            .usage_ledger
            .stats(&state.usage_dir, query.unwrap_or_default())
    })
    .await
    .map_err(|error| format!("usage_get_stats failed: {error}"))?
}

#[tauri::command]
pub async fn usage_sync_cli(
    app: AppHandle,
    force_prices: Option<bool>,
) -> Result<native::ScanReport, String> {
    let dir = app.state::<AppState>().usage_dir.clone();
    let price_error = pricing::refresh(&dir, force_prices.unwrap_or(false))
        .await
        .err();
    tauri::async_runtime::spawn_blocking(move || {
        let mut report = native::scan(&dir)?;
        pricing::reprice(&mut store::open(&dir)?)?;
        if let Some(error) = price_error {
            report.sources.push(native::SourceReport {
                id: "prices".into(),
                label: "LiteLLM".into(),
                errors: vec![error],
                ..Default::default()
            });
        }
        Ok(report)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn usage_clear(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        state.usage_ledger.clear(&state.usage_dir)
    })
    .await
    .map_err(|error| format!("usage_clear failed: {error}"))?
}

#[cfg(test)]
fn filter_records(mut records: Vec<UsageRecord>, query: &UsageStatsQuery) -> Vec<UsageRecord> {
    let start = range_start(&query.range);
    let source = normalized_filter(query.source.as_deref());
    let status = normalized_filter(query.status.as_deref());
    let provider_search = normalized_search(query.provider_search.as_deref());
    let model_search = normalized_search(query.model_search.as_deref());
    records.retain(|record| {
        if let Some(start) = start {
            if record.created_at < start {
                return false;
            }
        }
        if let Some(source) = source.as_deref() {
            if record.source != source {
                return false;
            }
        }
        if let Some(status) = status.as_deref() {
            if status == "missing_usage" {
                if record.usage_source != "missing" {
                    return false;
                }
            } else if record.status != status {
                return false;
            }
        }
        if let Some(search) = provider_search.as_deref() {
            let haystack =
                format!("{} {}", record.provider_id, record.provider_name).to_ascii_lowercase();
            if !haystack.contains(search) {
                return false;
            }
        }
        if let Some(search) = model_search.as_deref() {
            if !record.model.to_ascii_lowercase().contains(search) {
                return false;
            }
        }
        true
    });
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    records
}

fn range_start(range: &str) -> Option<i64> {
    let now = Local::now().timestamp();
    match range {
        // 「当天」= 本地日历日起点（今日 00:00），随后各档为滚动窗口。
        "today" => Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| Local.from_local_datetime(&naive).single())
            .map(|dt| dt.timestamp()),
        "1d" => Some(now.saturating_sub(86_400)),
        "30d" => Some(now.saturating_sub(30 * 86_400)),
        "90d" => Some(now.saturating_sub(90 * 86_400)),
        "365d" => Some(now.saturating_sub(365 * 86_400)),
        "all" => None,
        // default 7d
        _ => Some(now.saturating_sub(7 * 86_400)),
    }
}

fn normalized_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "all")
        .map(str::to_string)
}

fn normalized_search(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

#[cfg(test)]
fn summarize(records: &[UsageRecord]) -> UsageSummary {
    let mut summary = UsageSummary {
        session_count: records
            .iter()
            .filter_map(|r| r.conversation_id.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len() as u64,
        unpriced_requests: records.iter().filter(|r| r.cost_usd.is_none()).count() as u64,
        ..Default::default()
    };
    let mut duration_total = 0u64;
    let mut duration_count = 0u64;
    for record in records {
        summary.total_requests = summary.total_requests.saturating_add(1);
        if record.status == "success" {
            summary.successful_requests = summary.successful_requests.saturating_add(1);
        } else {
            summary.failed_requests = summary.failed_requests.saturating_add(1);
        }
        if record.usage_source == "missing" {
            summary.missing_usage_requests = summary.missing_usage_requests.saturating_add(1);
        }
        if record.usage_source == "provider_reported" {
            summary.provider_reported_requests =
                summary.provider_reported_requests.saturating_add(1);
        }
        summary.total_tokens = summary
            .total_tokens
            .saturating_add(record_total_tokens(record));
        summary.input_tokens = summary
            .input_tokens
            .saturating_add(record_effective_input_tokens(record));
        summary.output_tokens = summary
            .output_tokens
            .saturating_add(record.output_tokens.unwrap_or(0));
        summary.cached_input_tokens = summary
            .cached_input_tokens
            .saturating_add(record.cached_input_tokens.unwrap_or(0));
        summary.cache_creation_input_tokens = summary
            .cache_creation_input_tokens
            .saturating_add(record.cache_creation_input_tokens.unwrap_or(0));
        summary.reasoning_tokens = summary
            .reasoning_tokens
            .saturating_add(record.reasoning_tokens.unwrap_or(0));
        summary.total_cost_usd += record.cost_usd.unwrap_or(0.0);
        duration_total = duration_total.saturating_add(record.duration_ms);
        duration_count = duration_count.saturating_add(1);
    }
    if duration_count > 0 {
        summary.average_duration_ms = Some(duration_total as f64 / duration_count as f64);
    }
    summary
}

/// 统一「输入 tokens」口径（对齐 crate::chat::model::context_usage::anchor_total_tokens，commit
/// 85a3056）：Anthropic 的 input_tokens 是**非缓存**部分，全量输入 = input +
/// cache_read + cache_creation（三者不相交）；OpenAI 系 input(=prompt_tokens)
/// 已含 cached，直接用。聚合（summary/trend/分组）一律走这里，缓存命中率
/// cached/input 才对两种口径同时成立。
#[cfg(test)]
fn record_effective_input_tokens(record: &UsageRecord) -> u64 {
    let input = record.input_tokens.unwrap_or(0);
    if record.api_format == "anthropic_messages" {
        input
            .saturating_add(record.cached_input_tokens.unwrap_or(0))
            .saturating_add(record.cache_creation_input_tokens.unwrap_or(0))
    } else {
        input
    }
}

#[cfg(test)]
fn record_total_tokens(record: &UsageRecord) -> u64 {
    // Anthropic：不能用落盘的 total_tokens（= input+output，漏 cache，见
    // model_usage_from_anthropic_value）——显式加回缓存部分。
    if record.api_format == "anthropic_messages" {
        return record_effective_input_tokens(record)
            .saturating_add(record.output_tokens.unwrap_or(0));
    }
    record.total_tokens.unwrap_or_else(|| {
        record
            .input_tokens
            .unwrap_or(0)
            .saturating_add(record.output_tokens.unwrap_or(0))
    })
}

fn empty_trend(range: &str) -> Vec<UsageTrendPoint> {
    let mut points: BTreeMap<String, UsageTrendPoint> = BTreeMap::new();
    // 「当天」按小时分桶（今日 00:00 至当前小时），其余档位按天分桶。
    let hourly = matches!(range, "today" | "1d");
    if range == "1d" {
        let now = Local::now();
        for offset in (0..=24).rev() {
            let dt = now - ChronoDuration::hours(offset);
            let key = dt.format("%Y-%m-%d %H").to_string();
            points.insert(
                key.clone(),
                UsageTrendPoint {
                    date: key,
                    label: dt.format("%H:00").to_string(),
                    ..Default::default()
                },
            );
        }
    } else if hourly {
        let now = Local::now();
        let today = now.date_naive();
        for hour in 0..=now.hour() {
            let key = format!("{} {:02}", today.format("%Y-%m-%d"), hour);
            points.insert(
                key.clone(),
                UsageTrendPoint {
                    date: key,
                    label: format!("{:02}:00", hour),
                    ..UsageTrendPoint::default()
                },
            );
        }
    } else if let Some(days) = range_days(range) {
        let today = Local::now().date_naive();
        for offset in (0..days).rev() {
            let date = today - ChronoDuration::days(offset as i64);
            let key = date.format("%Y-%m-%d").to_string();
            points.insert(
                key.clone(),
                UsageTrendPoint {
                    date: key,
                    label: date.format("%m/%d").to_string(),
                    ..UsageTrendPoint::default()
                },
            );
        }
    }
    points.into_values().collect()
}

#[cfg(test)]
fn build_trend(records: &[UsageRecord], range: &str) -> Vec<UsageTrendPoint> {
    let hourly = matches!(range, "today" | "1d");
    let mut points: BTreeMap<_, _> = empty_trend(range)
        .into_iter()
        .map(|p| (p.date.clone(), p))
        .collect();
    for record in records {
        let dt = Local
            .timestamp_opt(record.created_at, 0)
            .single()
            .unwrap_or_else(Local::now);
        let (key, label) = if hourly {
            (
                format!("{} {:02}", dt.date_naive().format("%Y-%m-%d"), dt.hour()),
                format!("{:02}:00", dt.hour()),
            )
        } else {
            let date = dt.date_naive();
            (
                date.format("%Y-%m-%d").to_string(),
                date.format("%m/%d").to_string(),
            )
        };
        let point = points
            .entry(key.clone())
            .or_insert_with(|| UsageTrendPoint {
                date: key,
                label,
                ..UsageTrendPoint::default()
            });
        point.requests = point.requests.saturating_add(1);
        point.total_tokens = point
            .total_tokens
            .saturating_add(record_total_tokens(record));
        point.input_tokens = point
            .input_tokens
            .saturating_add(record_effective_input_tokens(record));
        point.output_tokens = point
            .output_tokens
            .saturating_add(record.output_tokens.unwrap_or(0));
        point.cached_input_tokens = point
            .cached_input_tokens
            .saturating_add(record.cached_input_tokens.unwrap_or(0));
        point.cache_creation_input_tokens = point
            .cache_creation_input_tokens
            .saturating_add(record.cache_creation_input_tokens.unwrap_or(0));
        point.cost_usd += record.cost_usd.unwrap_or(0.0);
    }
    points.into_values().collect()
}

fn range_days(range: &str) -> Option<usize> {
    match range {
        "today" | "1d" => Some(1),
        "30d" => Some(30),
        "90d" => Some(90),
        "365d" => Some(365),
        "all" => None,
        // default 7d
        _ => Some(7),
    }
}

#[cfg(test)]
fn group_provider_stats(records: &[UsageRecord]) -> Vec<UsageGroupStats> {
    let mut groups: BTreeMap<String, UsageGroupStatsAccumulator> = BTreeMap::new();
    for record in records {
        let entry = groups
            .entry(record.provider_id.clone())
            .or_insert_with(|| UsageGroupStatsAccumulator::new_provider(record));
        entry.add(record);
    }
    finalize_groups(groups)
}

#[cfg(test)]
fn group_model_stats(records: &[UsageRecord]) -> Vec<UsageGroupStats> {
    let mut groups: BTreeMap<String, UsageGroupStatsAccumulator> = BTreeMap::new();
    for record in records {
        let key = format!("{}::{}", record.provider_id, record.model);
        let entry = groups
            .entry(key)
            .or_insert_with(|| UsageGroupStatsAccumulator::new_model(record));
        entry.add(record);
    }
    finalize_groups(groups)
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct UsageGroupStatsAccumulator {
    stats: UsageGroupStats,
    duration_total: u64,
    duration_count: u64,
}

#[cfg(test)]
impl UsageGroupStatsAccumulator {
    fn new_provider(record: &UsageRecord) -> Self {
        Self {
            stats: UsageGroupStats {
                id: record.provider_id.clone(),
                label: if record.provider_name.trim().is_empty() {
                    record.provider_id.clone()
                } else {
                    record.provider_name.clone()
                },
                provider_id: Some(record.provider_id.clone()),
                provider_name: Some(record.provider_name.clone()),
                ..UsageGroupStats::default()
            },
            duration_total: 0,
            duration_count: 0,
        }
    }

    fn new_model(record: &UsageRecord) -> Self {
        Self {
            stats: UsageGroupStats {
                id: format!("{}::{}", record.provider_id, record.model),
                label: record.model.clone(),
                provider_id: Some(record.provider_id.clone()),
                provider_name: Some(record.provider_name.clone()),
                model: Some(record.model.clone()),
                ..UsageGroupStats::default()
            },
            duration_total: 0,
            duration_count: 0,
        }
    }

    fn add(&mut self, record: &UsageRecord) {
        self.stats.unpriced_requests += u64::from(record.cost_usd.is_none());
        self.stats.request_count = self.stats.request_count.saturating_add(1);
        if record.status == "success" {
            self.stats.success_count = self.stats.success_count.saturating_add(1);
        }
        self.stats.total_tokens = self
            .stats
            .total_tokens
            .saturating_add(record_total_tokens(record));
        self.stats.input_tokens = self
            .stats
            .input_tokens
            .saturating_add(record_effective_input_tokens(record));
        self.stats.output_tokens = self
            .stats
            .output_tokens
            .saturating_add(record.output_tokens.unwrap_or(0));
        self.stats.cached_input_tokens = self
            .stats
            .cached_input_tokens
            .saturating_add(record.cached_input_tokens.unwrap_or(0));
        self.stats.cache_creation_input_tokens = self
            .stats
            .cache_creation_input_tokens
            .saturating_add(record.cache_creation_input_tokens.unwrap_or(0));
        self.stats.cost_usd += record.cost_usd.unwrap_or(0.0);
        self.duration_total = self.duration_total.saturating_add(record.duration_ms);
        self.duration_count = self.duration_count.saturating_add(1);
        self.stats.last_used_at = Some(
            self.stats
                .last_used_at
                .unwrap_or(record.created_at)
                .max(record.created_at),
        );
    }
}

#[cfg(test)]
fn finalize_groups(groups: BTreeMap<String, UsageGroupStatsAccumulator>) -> Vec<UsageGroupStats> {
    let mut stats = groups
        .into_values()
        .map(|mut group| {
            if group.duration_count > 0 {
                group.stats.average_duration_ms =
                    Some(group.duration_total as f64 / group.duration_count as f64);
            }
            group.stats
        })
        .collect::<Vec<_>>();
    stats.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
    stats
}

pub use kivio_agent_sdk::model::usage::{
    model_usage_from_anthropic_value, model_usage_from_openai_value,
};

pub fn chat_usage_source_for_label(label: &str) -> String {
    let lower = label.to_ascii_lowercase();
    if lower.contains("title") {
        "chat_title_summary".to_string()
    } else if lower.contains("compression") {
        "chat_compression".to_string()
    } else if lower.contains("auxiliary vision") {
        "chat_aux_vision".to_string()
    } else if lower.contains("auxiliary video") {
        "chat_aux_video".to_string()
    } else if lower.contains("image generation") {
        "chat_image_generation".to_string()
    } else if lower.contains("prompt optimize") {
        "chat_prompt_optimize".to_string()
    } else {
        "chat".to_string()
    }
}

pub fn operation_from_label(label: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in label.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_sep = false;
        } else if !last_was_sep && !out.is_empty() {
            out.push('_');
            last_was_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "model_call".to_string()
    } else {
        out
    }
}

pub fn error_kind_from_message(message: &str) -> String {
    if let Some(code) = crate::api::extract_status_code(message) {
        return format!("http_{code}");
    }
    let lower = message.to_ascii_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        "timeout".to_string()
    } else if lower.contains("cancelled") || lower.contains("canceled") {
        "cancelled".to_string()
    } else if lower.contains("parse") || lower.contains("json") {
        "parse_error".to_string()
    } else if lower.contains("read body") || lower.contains("stream") {
        "stream_error".to_string()
    } else {
        "request_error".to_string()
    }
}

/// 失败记录的 usage status：错误文案命中取消语义时记 `"cancelled"`，否则 `"error"`。
/// 四个模型适配器的 `record_usage_failure` 签名只有错误字符串（不带 `ModelError`），
/// 故在共享层按与 `error_kind_from_message` 同一口径判定，避免改四处签名。
pub fn failure_status_from_message(message: &str) -> &'static str {
    if error_kind_from_message(message) == "cancelled" {
        "cancelled"
    } else {
        "error"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_cost_uses_protocol_counts_ttl_buckets_and_configured_prices() {
        let mut provider: ModelProvider = serde_json::from_value(serde_json::json!({
            "id":"p", "name":"test", "baseUrl":"https://example.test", "apiFormat":"openai_chat",
            "modelOverrides": {
                "claude-test": {"pricing":{"input":2.0,"output":10.0,"cachedInput":0.2}},
                "gpt-5.6": {"pricing":{"input":2.0,"output":10.0,"cachedInput":0.2}},
                "custom": {"pricing":{"input":2.0,"output":10.0,"cachedInput":0.2,"cacheWrite":3.0,"cacheWriteLong":7.0}}
            }
        })).unwrap();
        for (model, api, input, long, expected) in [
            ("claude-test", "openai_chat", 100, None, 0.000111),
            ("claude-test", "anthropic_messages", 10, None, 0.000111),
            ("claude-test", "openai_chat", 100, Some(4), 0.000117),
            ("gpt-5.6", "openai_responses", 100, Some(4), 0.000121),
            ("custom", "openai_chat", 100, Some(4), 0.000132),
        ] {
            provider.api_format = api.into();
            let usage = ModelUsage {
                input_tokens: Some(input),
                output_tokens: Some(5),
                cached_input_tokens: Some(80),
                cache_creation_input_tokens: Some(10),
                cache_creation_input_tokens_long: long,
                ..Default::default()
            };
            let (cost, _) = estimate_model_cost(&provider, model, &usage).unwrap();
            assert!((cost - expected).abs() < 1e-12, "{model}/{api}: {cost}");
            let mut record = base_record(api);
            record.model = model.into();
            record.input_tokens = usage.input_tokens;
            record.output_tokens = usage.output_tokens;
            record.cached_input_tokens = usage.cached_input_tokens;
            record.cache_creation_input_tokens = usage.cache_creation_input_tokens;
            record.cache_creation_input_tokens_long = long;
            apply_cost(&provider, &mut record);
            assert_eq!(record.cost_usd, Some(cost));
        }
        provider
            .model_overrides
            .get_mut("claude-test")
            .unwrap()
            .pricing
            .as_mut()
            .unwrap()
            .cache_write = Some(3.0);
        let mixed = ModelUsage {
            input_tokens: Some(100),
            cache_creation_input_tokens: Some(20),
            cache_creation_input_tokens_long: Some(10),
            ..Default::default()
        };
        assert!(
            (estimate_model_cost(&provider, "claude-test", &mixed)
                .unwrap()
                .0
                - 0.00023)
                .abs()
                < 1e-12
        );
        provider
            .model_overrides
            .get_mut("claude-test")
            .unwrap()
            .pricing
            .as_mut()
            .unwrap()
            .cache_write = None;
        provider.request.prompt_cache_retention = "long".into();
        let usage = ModelUsage {
            input_tokens: Some(100),
            cache_creation_input_tokens: Some(20),
            cache_creation_input_tokens_long: Some(0),
            ..Default::default()
        };
        assert!(
            (estimate_model_cost(&provider, "claude-test", &usage)
                .unwrap()
                .0
                - 0.00021)
                .abs()
                < 1e-12
        );
    }

    #[test]
    fn claude_cache_write_cost_uses_requested_retention_without_affecting_other_models() {
        let mut provider: ModelProvider = serde_json::from_value(serde_json::json!({
            "id":"p", "name":"test", "baseUrl":"https://example.test", "apiFormat":"anthropic_messages",
            "modelOverrides": {
                "claude-test": {"pricing":{"input":2.0,"output":10.0,"cachedInput":0.2}},
                "deepseek-test": {"pricing":{"input":2.0,"output":10.0,"cachedInput":0.2}}
            }
        })).unwrap();
        for (model, retention, expected) in [
            ("claude-test", "short", 0.003285),
            ("claude-test", "long", 0.00336),
            ("deepseek-test", "short", 0.00326),
        ] {
            provider.request.prompt_cache_retention = retention.into();
            let mut record = base_record("anthropic_messages");
            record.model = model.into();
            apply_cost(&provider, &mut record);
            assert!(
                (record.cost_usd.unwrap() - expected).abs() < 1e-10,
                "{model}/{retention}: {:?}",
                record.cost_usd
            );
        }
    }

    #[test]
    fn parses_openai_cached_and_reasoning_usage() {
        let value = serde_json::json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 20,
                "total_tokens": 120,
                "prompt_tokens_details": { "cached_tokens": 80 },
                "completion_tokens_details": { "reasoning_tokens": 5 }
            }
        });
        let usage = model_usage_from_openai_value(&value).expect("usage");
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.cached_input_tokens, Some(80));
        assert_eq!(usage.reasoning_tokens, Some(5));
    }

    #[test]
    fn filter_missing_usage_status() {
        let mut record = UsageRecord {
            id: format!("usage_{}", Uuid::new_v4()),
            created_at: Local::now().timestamp(),
            completed_at: Local::now().timestamp(),
            duration_ms: 1,
            first_token_ms: None,
            source: "chat".to_string(),
            operation: "plain".to_string(),
            provider_id: "p".to_string(),
            provider_name: "Provider".to_string(),
            model: "model".to_string(),
            api_format: "openai_chat".to_string(),
            status: "success".to_string(),
            status_code: Some(200),
            usage_source: "missing".to_string(),
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cached_input_tokens: None,
            cache_creation_input_tokens: None,
            cache_creation_input_tokens_long: None,
            reasoning_tokens: None,
            reasoning_effort: None,
            service_tier: None,
            cost_usd: None,
            cost_source: "unavailable".to_string(),
            conversation_id: None,
            message_id: None,
            error_kind: None,
        };
        let query = UsageStatsQuery {
            status: Some("missing_usage".to_string()),
            ..UsageStatsQuery::default()
        };
        assert_eq!(filter_records(vec![record.clone()], &query).len(), 1);
        record.usage_source = "provider_reported".to_string();
        assert!(filter_records(vec![record], &query).is_empty());
    }

    fn base_record(api_format: &str) -> UsageRecord {
        UsageRecord {
            id: format!("usage_{}", Uuid::new_v4()),
            created_at: Local::now().timestamp(),
            completed_at: Local::now().timestamp(),
            duration_ms: 1,
            first_token_ms: None,
            source: "chat".to_string(),
            operation: "plain".to_string(),
            provider_id: "p".to_string(),
            provider_name: "Provider".to_string(),
            model: "model".to_string(),
            api_format: api_format.to_string(),
            status: "success".to_string(),
            status_code: Some(200),
            usage_source: "provider_reported".to_string(),
            input_tokens: Some(1_000),
            output_tokens: Some(100),
            total_tokens: Some(1_100),
            cached_input_tokens: Some(800),
            cache_creation_input_tokens: Some(50),
            cache_creation_input_tokens_long: None,
            reasoning_tokens: None,
            reasoning_effort: None,
            service_tier: None,
            cost_usd: None,
            cost_source: "unavailable".to_string(),
            conversation_id: None,
            message_id: None,
            error_kind: None,
        }
    }

    #[test]
    fn stats_pagination_follows_append_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let query = |offset| UsageStatsQuery {
            range: "all".into(),
            limit: Some(1),
            offset: Some(offset),
            ..Default::default()
        };
        for i in 0..3 {
            let mut record = base_record("openai_responses");
            record.id = format!("record-{i}");
            record.created_at += i;
            ledger.append(dir.path(), &record).unwrap();
        }
        let first = ledger.stats(dir.path(), query(0)).unwrap();
        let second = ledger.stats(dir.path(), query(1)).unwrap();
        assert_eq!(first.total_logs, 3);
        assert_eq!(first.summary.total_requests, second.summary.total_requests);
        assert_ne!(first.logs[0].id, second.logs[0].id);
        ledger
            .append(dir.path(), &base_record("openai_responses"))
            .unwrap();
        assert_eq!(ledger.stats(dir.path(), query(0)).unwrap().total_logs, 4);
        ledger.clear(dir.path()).unwrap();
        let empty = ledger.stats(dir.path(), query(0)).unwrap();
        assert_eq!(empty.total_logs, 0);
        assert!(empty.logs.is_empty());
    }

    #[test]
    fn conversation_cost_tracks_appends_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let usage_dir = dir.path().join("usage");
        for (id, cost) in [
            ("parent", Some(0.125)),
            ("subagent-parent-child", Some(0.25)),
            ("subagent-parent-child", Some(0.5)),
            ("subagent-parent-child", None),
            ("subagent-other-child", Some(99.0)),
        ] {
            let mut record = base_record("openai_chat");
            record.conversation_id = Some(id.into());
            record.cost_usd = cost;
            ledger.append(&usage_dir, &record).unwrap();
        }
        let cost = ledger.conversation_cost(&usage_dir, "parent").unwrap();
        assert_eq!(cost.cost_usd, Some(0.875));
        assert_eq!(cost.unpriced_requests, 1);
        let mut extra = base_record("openai_chat");
        extra.conversation_id = Some("subagent-parent-child-2".into());
        extra.cost_usd = Some(0.125);
        ledger.append(&usage_dir, &extra).unwrap();
        assert_eq!(
            ledger
                .conversation_cost(&usage_dir, "parent")
                .unwrap()
                .cost_usd,
            Some(1.0)
        );
        let restart = UsageLedger::default();
        assert_eq!(
            restart
                .conversation_cost(&usage_dir, "parent")
                .unwrap()
                .cost_usd,
            Some(1.0)
        );
        ledger.clear(&usage_dir).unwrap();
        assert_eq!(
            ledger
                .conversation_cost(&usage_dir, "parent")
                .unwrap()
                .cost_usd,
            None
        );
        ledger.append(&usage_dir, &extra).unwrap();
        assert_eq!(
            ledger
                .conversation_cost(&usage_dir, "parent")
                .unwrap()
                .cost_usd,
            Some(0.125)
        );
    }

    #[test]
    fn conversation_cost_recovers_root_prefix_nested_and_continued_usage() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let root = "conv_00000000-0000-4000-8000-000000000001";
        let other = "conv_00000000-0000-4000-8000-000000000002";
        let similar = format!("{root}2");
        let child = format!("subagent-{root}-Worker");
        let nested = format!("{child}.Nested");
        let next_child = format!("{child}-2");
        let other_child = format!("subagent-{other}-Worker");
        let similar_child = format!("subagent-{similar}-Worker");
        let missing_separator = format!("subagent-{root}");
        // Query before writing, then verify both existing and restarted readers.
        ledger.conversation_cost(dir.path(), root).unwrap();
        for (id, cost) in [
            (root, Some(1.0)),
            (child.as_str(), Some(0.25)),
            (child.as_str(), Some(0.5)),
            (nested.as_str(), Some(0.125)),
            (nested.as_str(), Some(0.0625)),
            (next_child.as_str(), Some(0.03125)),
            (child.as_str(), None),
            (other, Some(4.0)),
            (other_child.as_str(), Some(8.0)),
            (similar.as_str(), Some(16.0)),
            (similar_child.as_str(), Some(32.0)),
            (missing_separator.as_str(), Some(64.0)),
        ] {
            let mut record = base_record("openai_chat");
            record.conversation_id = Some(id.into());
            record.cost_usd = cost;
            ledger.append(dir.path(), &record).unwrap();
        }
        let restart = UsageLedger::default();
        for index in [&ledger, &restart] {
            let cost = index.conversation_cost(dir.path(), root).unwrap();
            assert_eq!(cost.cost_usd, Some(1.96875));
            assert_eq!(cost.unpriced_requests, 1);
            assert_eq!(cost.skipped_records, 0);
            assert_eq!(
                index.conversation_cost(dir.path(), other).unwrap().cost_usd,
                Some(12.0)
            );
        }
    }

    #[test]
    fn concurrent_appends_and_failed_writes_preserve_the_cost_aggregate() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        ledger.conversation_cost(dir.path(), "parent").unwrap();
        let mut record = base_record("openai_chat");
        record.conversation_id = Some("parent".into());
        record.cost_usd = Some(0.125);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..16 {
                        let mut next = record.clone();
                        next.id = Uuid::new_v4().to_string();
                        ledger.append(dir.path(), &next).unwrap();
                    }
                });
            }
        });
        assert_eq!(
            ledger
                .conversation_cost(dir.path(), "parent")
                .unwrap()
                .cost_usd,
            Some(16.0)
        );
        assert_eq!(
            UsageLedger::default()
                .conversation_cost(dir.path(), "parent")
                .unwrap()
                .cost_usd,
            Some(16.0)
        );
        let invalid = dir.path().join("not-a-directory");
        fs::write(&invalid, "").unwrap();
        assert!(ledger.append(&invalid, &record).is_err());
        assert!(ledger.conversation_cost(&invalid, "parent").is_err());
    }

    #[test]
    fn conversation_cost_covers_all_recorded_requests_without_conflating_unknown_and_free() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        for (id, age_days, cost, usage_source) in [
            ("a", 400, Some(0.125), "provider_reported"),
            ("a", 0, Some(0.25), "provider_reported"),
            ("a", 0, None, "provider_reported"),
            ("a", 0, Some(0.0), "missing"),
            ("other", 0, Some(999.0), "provider_reported"),
            ("free", 0, Some(0.0), "provider_reported"),
        ] {
            let mut record = base_record("openai_chat");
            record.conversation_id = Some(id.into());
            record.created_at -= age_days * 86400;
            record.cost_usd = cost;
            record.usage_source = usage_source.into();
            ledger.append(dir.path(), &record).unwrap();
        }
        let cost = ledger.conversation_cost(dir.path(), "a").unwrap();
        assert_eq!(cost.cost_usd, Some(0.375));
        assert_eq!(cost.unpriced_requests, 2);
        assert_eq!(cost.skipped_records, 0);
        assert_eq!(
            ledger
                .conversation_cost(dir.path(), "free")
                .unwrap()
                .cost_usd,
            Some(0.0)
        );
        assert_eq!(
            ledger
                .conversation_cost(dir.path(), "absent")
                .unwrap()
                .cost_usd,
            None
        );
    }

    #[test]
    fn today_companion_snapshot_matches_local_day_and_preserves_missing_costs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("unreadable.jsonl"), "{broken record}\n").unwrap();
        let mut old = base_record("openai_chat");
        old.created_at = range_start("today").unwrap() - 1;
        old.cost_usd = Some(900.0);
        UsageLedger::default().append(dir.path(), &old).unwrap();
        let mut known = base_record("anthropic_messages");
        known.cost_usd = Some(0.125);
        UsageLedger::default().append(dir.path(), &known).unwrap();
        UsageLedger::default()
            .append(dir.path(), &base_record("openai_chat"))
            .unwrap();
        let snapshot = today_usage(dir.path()).unwrap();
        assert_eq!(snapshot.summary.total_requests, 2);
        assert_eq!(snapshot.summary.total_tokens, 3_050);
        assert_eq!(snapshot.summary.total_cost_usd, 0.125);
        assert_eq!(snapshot.missing_cost_requests, 1);
        assert_eq!(snapshot.skipped_records, 1);
        fs::write(dir.path().join("unreadable.jsonl"), "{broken record}\n").unwrap();
        assert_eq!(today_usage(dir.path()).unwrap().skipped_records, 1);
        let not_directory = dir.path().join("file");
        fs::write(&not_directory, "").unwrap();
        assert!(today_usage(&not_directory).is_err());
    }

    /// 口径对齐 crate::chat::model::context_usage::anchor_total_tokens（85a3056）：Anthropic 的
    /// input 是非缓存部分，全量要显式加 cache_read + cache_creation；OpenAI 的
    /// input 已含 cached，直接用。
    #[test]
    fn effective_input_and_total_disambiguate_by_api_format() {
        let anthropic = base_record("anthropic_messages");
        // 1000(非缓存) + 800(cache_read) + 50(cache_creation) = 1850
        assert_eq!(record_effective_input_tokens(&anthropic), 1_850);
        // 全量 = 1850 + 100(out)，不能用漏 cache 的落盘 total_tokens(1100)。
        assert_eq!(record_total_tokens(&anthropic), 1_950);

        let openai = base_record("openai_chat");
        assert_eq!(record_effective_input_tokens(&openai), 1_000);
        assert_eq!(record_total_tokens(&openai), 1_100); // 落盘 total 优先
    }

    #[test]
    fn usage_record_new_fields_are_backward_compatible_and_camel_case() {
        let mut value = serde_json::to_value(base_record("openai_chat")).expect("serialize");
        let object = value.as_object_mut().expect("record object");
        object.remove("firstTokenMs");
        object.remove("reasoningEffort");

        let old_record: UsageRecord = serde_json::from_value(value).expect("old record");
        assert_eq!(old_record.first_token_ms, None);
        assert_eq!(old_record.reasoning_effort, None);

        let mut current = base_record("openai_responses");
        current.first_token_ms = Some(1_250);
        current.reasoning_effort = Some("high".to_string());
        let serialized = serde_json::to_value(current).expect("serialize current record");
        assert_eq!(serialized["firstTokenMs"], 1_250);
        assert_eq!(serialized["reasoningEffort"], "high");
    }

    #[test]
    fn sqlite_reports_match_existing_accounting_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let mut records = Vec::new();
        for i in 0..24 {
            let mut record = base_record(if i % 2 == 0 {
                "anthropic_messages"
            } else {
                "openai_chat"
            });
            record.created_at -= i * 3700;
            record.provider_id = format!("provider-{}", i % 3);
            record.provider_name = format!("Name {}", i % 3);
            record.model = format!("model_{}%", i % 4);
            record.source = if i % 3 == 0 { "knowledge_base" } else { "chat" }.into();
            record.status = if i % 5 == 0 { "error" } else { "success" }.into();
            record.usage_source = if i % 7 == 0 {
                "missing"
            } else {
                "provider_reported"
            }
            .into();
            record.cost_usd = (i % 7 != 0).then_some(i as f64 / 8.0);
            record.duration_ms = i as u64 + 1;
            record.total_tokens = None;
            ledger.append(dir.path(), &record).unwrap();
            records.push(record);
        }
        for query in [
            UsageStatsQuery {
                range: "all".into(),
                ..Default::default()
            },
            UsageStatsQuery {
                range: "today".into(),
                ..Default::default()
            },
            UsageStatsQuery {
                source: Some("chat".into()),
                ..Default::default()
            },
            UsageStatsQuery {
                status: Some("missing_usage".into()),
                ..Default::default()
            },
            UsageStatsQuery {
                status: Some("error".into()),
                ..Default::default()
            },
            UsageStatsQuery {
                provider_search: Some("NAME 2".into()),
                model_search: Some("_2%".into()),
                ..Default::default()
            },
            UsageStatsQuery {
                model_search: Some("absent".into()),
                ..Default::default()
            },
        ] {
            let filtered = filter_records(records.clone(), &query);
            let actual = ledger.stats(dir.path(), query.clone()).unwrap();
            assert_eq!(
                serde_json::to_value(&actual.summary).unwrap(),
                serde_json::to_value(summarize(&filtered)).unwrap()
            );
            assert_eq!(
                serde_json::to_value(&actual.trend).unwrap(),
                serde_json::to_value(build_trend(&filtered, &query.range)).unwrap()
            );
            let canonical = |mut groups: Vec<UsageGroupStats>| {
                groups.sort_by(|a, b| a.id.cmp(&b.id));
                serde_json::to_value(groups).unwrap()
            };
            assert_eq!(
                canonical(actual.provider_stats),
                canonical(group_provider_stats(&filtered))
            );
            assert_eq!(
                canonical(actual.model_stats),
                canonical(group_model_stats(&filtered))
            );
            assert_eq!(actual.total_logs, filtered.len());
        }
    }

    #[test]
    fn migration_is_atomic_idempotent_and_clear_does_not_resurrect_backups() {
        let dir = tempfile::tempdir().unwrap();
        let record = base_record("anthropic_messages");
        let json = serde_json::to_string(&record).unwrap();
        let legacy = dir.path().join("usage-2026-01.jsonl");
        fs::write(&legacy, format!("{json}\n{json}\ninvalid\n")).unwrap();
        let unreadable = dir.path().join("unreadable.jsonl");
        fs::create_dir(&unreadable).unwrap();
        let ledger = UsageLedger::default();
        let query = UsageStatsQuery {
            range: "all".into(),
            ..Default::default()
        };
        assert!(ledger.stats(dir.path(), query.clone()).is_err());
        fs::remove_dir(&unreadable).unwrap();
        let report = ledger.stats(dir.path(), query.clone()).unwrap();
        assert_eq!(report.total_logs, 1);
        assert_eq!(report.skipped_records, 1);
        assert_eq!(report.summary.total_tokens, 1950);
        assert_eq!(
            serde_json::to_value(&report.logs[0]).unwrap(),
            serde_json::to_value(&record).unwrap()
        );
        ledger.append(dir.path(), &record).unwrap();
        assert_eq!(
            ledger.stats(dir.path(), query.clone()).unwrap().total_logs,
            1
        );
        assert!(legacy.exists());
        ledger.clear(dir.path()).unwrap();
        let restarted = UsageLedger::default().stats(dir.path(), query).unwrap();
        assert_eq!(restarted.total_logs, 0);
        assert_eq!(restarted.skipped_records, 0);
    }

    #[test]
    fn failed_database_write_and_clear_preserve_committed_records() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        ledger
            .append(dir.path(), &base_record("openai_chat"))
            .unwrap();
        let db = rusqlite::Connection::open(dir.path().join("usage.sqlite3")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_insert BEFORE INSERT ON records BEGIN SELECT RAISE(ABORT,'test failure'); END; CREATE TRIGGER reject_clear BEFORE DELETE ON records BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(ledger
            .append(dir.path(), &base_record("openai_chat"))
            .is_err());
        assert!(ledger.clear(dir.path()).is_err());
        assert_eq!(
            ledger
                .stats(dir.path(), UsageStatsQuery::default())
                .unwrap()
                .total_logs,
            1
        );
        db.execute_batch("DROP TRIGGER reject_insert; DROP TRIGGER reject_clear;")
            .unwrap();
        ledger.clear(dir.path()).unwrap();
        assert_eq!(
            ledger
                .stats(dir.path(), UsageStatsQuery::default())
                .unwrap()
                .total_logs,
            0
        );
    }
    #[test]
    fn pagination_is_stable_for_equal_timestamps_and_handles_empty_pages() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::default();
        let mut record = base_record("openai_chat");
        for id in ["b", "c", "a"] {
            record.id = id.into();
            ledger.append(dir.path(), &record).unwrap();
        }
        for (offset, expected) in [
            (0, Some("c")),
            (1, Some("b")),
            (2, Some("a")),
            (3, None),
            (usize::MAX, None),
        ] {
            let report = ledger
                .stats(
                    dir.path(),
                    UsageStatsQuery {
                        range: "all".into(),
                        limit: Some(1),
                        offset: Some(offset),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(report.total_logs, 3);
            assert_eq!(report.logs.first().map(|r| r.id.as_str()), expected);
        }
    }
}
