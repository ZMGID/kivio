// Shared durable usage ledger and reports. Platform commands remain in usage.rs.
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::{Datelike, Duration as ChronoDuration, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{chat::model_metadata, settings::ModelProvider};

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
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
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

fn default_range() -> String {
    "7d".to_string()
}

pub(crate) fn record_model_call_in(
    ledger: &UsageLedger,
    dir: &Path,
    input: crate::usage::UsageRecordInput<'_>,
) {
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
        reasoning_tokens,
        reasoning_effort: input.reasoning_effort,
        cost_usd: None,
        cost_source: "unavailable".to_string(),
        conversation_id: input.conversation_id,
        message_id: input.message_id,
        error_kind: input.error_kind,
    };
    apply_cost(input.provider, &mut record);
    if let Err(err) = ledger.append(dir, &record) {
        eprintln!("Failed to record usage: {err}");
    }
}

fn append_record(dir: &Path, record: &UsageRecord) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("create usage dir: {e}"))?;
    let path = dir.join(monthly_file_name(record.created_at));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open usage file: {e}"))?;
    let line = serde_json::to_string(record).map_err(|e| format!("serialize usage: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("write usage file: {e}"))
}

fn monthly_file_name(timestamp: i64) -> String {
    let date = Local
        .timestamp_opt(timestamp, 0)
        .single()
        .unwrap_or_else(Local::now);
    format!("usage-{:04}-{:02}.jsonl", date.year(), date.month())
}

fn apply_cost(provider: &ModelProvider, record: &mut UsageRecord) {
    let Some((pricing, source)) = model_metadata::pricing_for_model(Some(provider), &record.model)
    else {
        return;
    };
    let input_price = pricing.input.unwrap_or(0.0);
    let output_price = pricing.output.unwrap_or(0.0);
    let cached_price = pricing.cached_input.unwrap_or(input_price);
    let input_tokens = record.input_tokens.unwrap_or(0);
    let output_tokens = record.output_tokens.unwrap_or(0);
    // Claude cache writes have a separate tariff: 5m = 1.25x, 1h = 2x.
    // This remains a configured-rate estimate, not the gateway's actual invoice.
    // Other Anthropic-compatible providers must not inherit Claude pricing.
    let write_multiplier = if record.model.to_ascii_lowercase().contains("claude") {
        if provider.cache_retention() == crate::settings::CacheRetention::Long {
            2.0
        } else {
            1.25
        }
    } else {
        1.0
    };
    let cost = if record.api_format == "anthropic_messages" {
        let cache_read = record.cached_input_tokens.unwrap_or(0);
        let cache_creation = record.cache_creation_input_tokens.unwrap_or(0);
        (input_tokens as f64 * input_price
            + cache_read as f64 * cached_price
            + cache_creation as f64 * input_price * write_multiplier
            + output_tokens as f64 * output_price)
            / 1_000_000.0
    } else {
        let cached_tokens = record.cached_input_tokens.unwrap_or(0).min(input_tokens);
        let uncached_input_tokens = input_tokens.saturating_sub(cached_tokens);
        (uncached_input_tokens as f64 * input_price
            + cached_tokens as f64 * cached_price
            + output_tokens as f64 * output_price)
            / 1_000_000.0
    };
    record.cost_usd = Some(cost);
    record.cost_source = source;
}

fn read_records(dir: &Path, start: Option<i64>) -> (Vec<UsageRecord>, usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return (Vec::new(), 0);
    };
    let mut records = Vec::new();
    let mut skipped = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        if usage_file_is_before_start(&path, start) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            skipped = skipped.saturating_add(1);
            continue;
        };
        for line in content.lines().filter(|line| !line.trim().is_empty()) {
            match serde_json::from_str::<UsageRecord>(line) {
                Ok(record) => records.push(record),
                Err(_) => skipped = skipped.saturating_add(1),
            }
        }
    }
    (records, skipped)
}

fn usage_file_is_before_start(path: &Path, start: Option<i64>) -> bool {
    let Some(start) = start else {
        return false;
    };
    let Some(next_month_start) = usage_file_next_month_start(path) else {
        return false;
    };
    next_month_start <= start
}

fn usage_file_next_month_start(path: &Path) -> Option<i64> {
    let file_name = path.file_name()?.to_str()?;
    let stem = file_name.strip_prefix("usage-")?.strip_suffix(".jsonl")?;
    let (year, month) = stem.split_once('-')?;
    let mut year = year.parse::<i32>().ok()?;
    let mut month = month.parse::<u32>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    if month == 12 {
        year = year.saturating_add(1);
        month = 1;
    } else {
        month += 1;
    }
    Local
        .with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .map(|date| date.timestamp())
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
    match fs::read_dir(dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Read usage directory: {error}")),
    }
    let date = Local::now().date_naive();
    let (records, skipped_records) = read_records(dir, range_start("today"));
    let records = filter_records(
        records,
        &UsageStatsQuery {
            range: "today".into(),
            ..UsageStatsQuery::default()
        },
    );
    Ok(TodayUsage {
        date,
        summary: summarize(&records),
        missing_cost_requests: records
            .iter()
            .filter(|record| record.cost_usd.is_none())
            .count(),
        skipped_records,
    })
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationCost {
    pub cost_usd: Option<f64>,
    pub unpriced_requests: usize,
    pub skipped_records: usize,
}

#[derive(Default)]
struct CostIndex {
    conversations: BTreeMap<String, ConversationCost>,
    skipped: usize,
}
impl CostIndex {
    fn add(&mut self, record: &UsageRecord) {
        let Some(id) = &record.conversation_id else {
            return;
        };
        let result = self.conversations.entry(id.clone()).or_default();
        match record
            .cost_usd
            .filter(|cost| cost.is_finite() && *cost >= 0.0)
            .filter(|_| record.usage_source != "missing")
        {
            Some(cost) => *result.cost_usd.get_or_insert(0.0) += cost,
            None => result.unpriced_requests += 1,
        }
    }
    fn recover(dir: &Path) -> Result<Self, String> {
        let mut index = Self::default();
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(index),
            Err(e) => return Err(e.to_string()),
        };
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            // Do not cache a partial aggregate after a transient I/O failure.
            // Leave initialization retryable; malformed records still count as skipped.
            let file = fs::File::open(&path)
                .map_err(|error| format!("open usage file {}: {error}", path.display()))?;
            // Recover once without retaining the global request history in memory.
            for line in BufReader::new(file).lines() {
                match line {
                    Ok(line) if line.trim().is_empty() => {}
                    Ok(line) => match serde_json::from_str::<UsageRecord>(&line) {
                        Ok(record) => index.add(&record),
                        Err(_) => index.skipped += 1,
                    },
                    Err(error) => {
                        return Err(format!("read usage file {}: {error}", path.display()));
                    }
                }
            }
        }
        Ok(index)
    }
}

/// Owns append, clear and the process-local aggregate under one lock. The JSONL
/// ledger stays authoritative; a restart rebuilds only on the first cost query.
#[derive(Default)]
pub(crate) struct UsageLedger(Mutex<Option<CostIndex>>, Mutex<Vec<StatsCache>>);

struct StatsCache {
    key: String,
    files: Vec<(PathBuf, u64, Option<std::time::SystemTime>)>,
    start: Option<i64>,
    response: UsageStatsResponse,
    records: Vec<UsageRecord>,
}
impl UsageLedger {
    pub(crate) fn stats(
        &self,
        dir: &Path,
        mut query: UsageStatsQuery,
    ) -> Result<UsageStatsResponse, String> {
        let start = range_start(&query.range);
        let offset = query.offset.take().unwrap_or(0);
        let limit = query
            .limit
            .take()
            .unwrap_or(DEFAULT_LOG_LIMIT)
            .min(MAX_LOG_LIMIT);
        let key = format!(
            "{}:{}:{}",
            dir.display(),
            Local::now().format("%Y-%m-%d-%H"),
            serde_json::to_string(&query).map_err(|e| e.to_string())?
        );
        let mut files = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|v| v.to_str()) != Some("jsonl")
                    || usage_file_is_before_start(&path, start)
                {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                Some((path, metadata.len(), metadata.modified().ok()))
            })
            .collect::<Vec<_>>();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        // Separate from the append/cost lock: a slow report must not block recording model calls.
        let mut cached = self.1.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cached.iter().find(|entry| {
            entry.key == key
                && entry.files == files
                && start >= entry.start
                && !entry
                    .records
                    .last()
                    .is_some_and(|record| start.is_some_and(|start| record.created_at < start))
        }) {
            let mut response = entry.response.clone();
            response.logs = entry
                .records
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect();
            return Ok(response);
        }
        let (records, skipped_records) = read_records(dir, start);
        let filtered = filter_records(records, &query);
        let total_logs = filtered.len();
        let summary = summarize(&filtered);
        let trend = build_trend(&filtered, &query.range);
        let provider_stats = group_provider_stats(&filtered);
        let model_stats = group_model_stats(&filtered);
        let logs = filtered.iter().skip(offset).take(limit).cloned().collect();
        let response = UsageStatsResponse {
            summary,
            trend,
            logs: Vec::new(),
            provider_stats,
            model_stats,
            total_logs,
            skipped_records,
        };
        cached.retain(|entry| entry.key != key);
        if cached.len() >= 2 {
            cached.remove(0);
        }
        cached.push(StatsCache {
            key,
            files,
            start,
            response: response.clone(),
            records: filtered,
        });
        Ok(UsageStatsResponse { logs, ..response })
    }

    pub(crate) fn append(&self, dir: &Path, record: &UsageRecord) -> Result<(), String> {
        let mut index = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = append_record(dir, record) {
            // A partial append may have reached disk; rebuild rather than guess.
            *index = None;
            return Err(error);
        }
        if let Some(index) = index.as_mut() {
            index.add(record);
        }
        Ok(())
    }

    pub(crate) fn conversation_cost_for_ids(
        &self,
        dir: &Path,
        ids: &[String],
    ) -> Result<ConversationCost, String> {
        let mut cached = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if cached.is_none() {
            *cached = Some(CostIndex::recover(dir)?);
        }
        let index = cached.as_ref().unwrap();
        let mut result = ConversationCost {
            skipped_records: index.skipped,
            ..Default::default()
        };
        for id in ids {
            if let Some(cost) = index.conversations.get(id) {
                if let Some(value) = cost.cost_usd {
                    *result.cost_usd.get_or_insert(0.0) += value;
                }
                result.unpriced_requests += cost.unpriced_requests;
            }
        }
        Ok(result)
    }

    pub(crate) fn clear(&self, dir: &Path) -> Result<(), String> {
        let mut cached = self.0.lock().unwrap_or_else(|e| e.into_inner());
        // Invalidate first: a failed partial clear must never expose old totals.
        *cached = None;
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir).map_err(|e| format!("read usage dir: {e}"))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                fs::remove_file(&path).map_err(|e| format!("remove usage file: {e}"))?;
            }
        }
        *cached = Some(CostIndex::default());
        Ok(())
    }
}

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

fn summarize(records: &[UsageRecord]) -> UsageSummary {
    let mut summary = UsageSummary::default();
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

/// 统一「输入 tokens」口径（对齐 context_estimate::anchor_total_tokens，commit
/// 85a3056）：Anthropic 的 input_tokens 是**非缓存**部分，全量输入 = input +
/// cache_read + cache_creation（三者不相交）；OpenAI 系 input(=prompt_tokens)
/// 已含 cached，直接用。聚合（summary/trend/分组）一律走这里，缓存命中率
/// cached/input 才对两种口径同时成立。
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

fn build_trend(records: &[UsageRecord], range: &str) -> Vec<UsageTrendPoint> {
    let mut points: BTreeMap<String, UsageTrendPoint> = BTreeMap::new();
    // 「当天」按小时分桶（今日 00:00 至当前小时），其余档位按天分桶。
    let hourly = range == "today";
    if hourly {
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

#[derive(Debug, Clone)]
struct UsageGroupStatsAccumulator {
    stats: UsageGroupStats,
    duration_total: u64,
    duration_count: u64,
}

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

/// Report across per-conversation native ledgers using the same filtering,
/// accounting and grouping as the application-wide legacy ledger.
pub fn stats_in_directories(directories: &[PathBuf], query: UsageStatsQuery) -> UsageStatsResponse {
    let mut records = Vec::new();
    let mut skipped_records = 0;
    for directory in directories {
        let (next, skipped) = read_records(directory, range_start(&query.range));
        records.extend(next);
        skipped_records += skipped;
    }
    let filtered = filter_records(records, &query);
    UsageStatsResponse {
        summary: summarize(&filtered),
        trend: build_trend(&filtered, &query.range),
        provider_stats: group_provider_stats(&filtered),
        model_stats: group_model_stats(&filtered),
        total_logs: filtered.len(),
        logs: filtered.iter().skip(query.offset.unwrap_or(0))
            .take(query.limit.unwrap_or(DEFAULT_LOG_LIMIT).min(MAX_LOG_LIMIT)).cloned().collect(),
        skipped_records,
    }
}
