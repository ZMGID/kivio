//! HTTP 客户端、provider 凭据解析、retry / failover、OpenAI 兼容 chat completion 调用与 SSE 流。
//!
//! 本模块对外暴露：
//! - `ProviderConnectionInput` / `resolve_provider_credentials` —— 来自前端的 provider 临时配置或 settings.json 的解析。
//! - `build_http_client` —— 共享 reqwest Client 构造；只设置连接/读空闲超时。
//! - `with_standard_request_timeout` —— 为非流式请求显式加总超时。
//! - `effective_retry_attempts` —— 把 settings.retry_enabled + retry_attempts 折成实际尝试次数。
//! - `extract_status_code` / `is_failover_error` —— failover 判定（401/402/403 立即换 key；429 阈值化换 key）。
//! - `send_with_retry` —— 网络抖动 / 5xx / 429 退避重试。
//! - `send_with_failover` —— 在 api_keys 列表上轮换；401/402/403 立即换 key，429 达阈值且有备用 key 才换。
//! - `call_openai_text` / `call_openai_ocr` / `call_vision_api` —— chat completion 三类调用。
//! - `stream_chat_call` / `stream_translate_combined` / `stream_vision_response` —— SSE 流解析。
//! - `build_ocr_request_body` —— 视觉 + 流式 body 构造。

use std::{
    collections::HashSet,
    fs,
    future::Future,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose, Engine as _};
use reqwest::{header::HeaderMap, Client, RequestBuilder, StatusCode};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::chat::model::ModelUsage;
use crate::lens_commands::resolve_explain_image_path;
use crate::prompts::COMBINED_TRANSLATE_SEPARATOR;
use crate::settings::{
    self, default_lens_system_prompt, no_think_instruction, ExplainMessage, Settings,
};
use crate::state::AppState;
use crate::usage::{
    error_kind_from_message, model_usage_from_openai_value, model_usage_from_stream_value,
    record_model_call, UsageRecordInput,
};
use crate::utils::provider_supports_thinking_field;

// ===== Provider 凭据 =====

/// 供应商连接输入参数，用于测试连接或获取模型列表时临时传入
/// api_keys 优先；api_key 为兼容旧前端发的单 key 字段（v2.3.x 时的 ProviderConnectionInput）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConnectionInput {
    pub id: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

impl ProviderConnectionInput {
    /// 整理出非空 key 列表：优先 api_keys，回退到 api_key。
    pub fn merged_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .api_keys
            .iter()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect();
        if keys.is_empty() {
            if let Some(legacy) = self.api_key.as_deref() {
                let trimmed = legacy.trim().to_string();
                if !trimmed.is_empty() {
                    keys.push(trimmed);
                }
            }
        }
        keys
    }
}

/// 解析供应商的凭据信息（base_url + 多 key 列表）
/// 优先使用传入的 ProviderConnectionInput（如测试连接时），否则从 settings 中查找对应的供应商
pub fn resolve_provider_credentials(
    settings: &Settings,
    provider_id: &str,
    provider: Option<ProviderConnectionInput>,
) -> Result<(String, Vec<String>), String> {
    if let Some(input) = provider {
        let id_matches = input
            .id
            .as_ref()
            .map(|id| id.is_empty() || id == provider_id)
            .unwrap_or(true);

        if id_matches {
            return Ok((input.base_url.clone(), input.merged_keys()));
        }
    }

    let provider = settings
        .get_provider(provider_id)
        .ok_or_else(|| "Provider not found".to_string())?;
    Ok((provider.base_url.clone(), provider.api_keys.clone()))
}

/// 普通非流式 API 请求的总超时。
pub const STANDARD_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// 只限制 TCP/TLS 建连阶段，避免 DNS/握手长期卡住。
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// 流式响应的读空闲超时：持续有 SSE chunk 到达时不会触发。
const HTTP_READ_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// 为非流式请求设置总超时。流式请求不要用它，否则长回答会被总时长切断。
pub fn with_standard_request_timeout(request: RequestBuilder) -> RequestBuilder {
    request.timeout(STANDARD_HTTP_REQUEST_TIMEOUT)
}

/// 把 JSON body 挂到请求上：`gzip=false` 走普通 `.json()`；`gzip=true` 则序列化后
/// gzip 压缩并设置 `Content-Encoding: gzip`。用于绕开个别供应商前置 WAF 对明文请求体的
/// 误拦（详见 `ModelProvider::compress_request_body`）。压缩任一步失败都安全退回明文。
pub fn attach_json_body(
    request: RequestBuilder,
    body: &serde_json::Value,
    gzip: bool,
) -> RequestBuilder {
    use std::io::Write as _;
    if gzip {
        if let Ok(raw) = serde_json::to_vec(body) {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            if enc.write_all(&raw).is_ok() {
                if let Ok(gz) = enc.finish() {
                    return request
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .header(reqwest::header::CONTENT_ENCODING, "gzip")
                        .body(gz);
                }
            }
        }
    }
    request.json(body)
}

/// 构建 HTTP 客户端：不设置 total timeout，避免活跃 SSE 流在 60 秒处被砍掉。
pub fn build_http_client() -> Client {
    Client::builder()
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .read_timeout(HTTP_READ_IDLE_TIMEOUT)
        .build()
        .unwrap_or_else(|err| {
            eprintln!("Failed to build HTTP client: {err}");
            Client::new()
        })
}

// ===== Retry / Failover =====

/// 重试延迟基础值（毫秒）。暂时性错误起步退避 ~5s。
const RETRY_BASE_DELAY_MS: u64 = 5_000;
/// 重试延迟最大值（毫秒）。温和退避封顶 30s（Retry-After 可覆盖更大值）。
const RETRY_MAX_DELAY_MS: u64 = 30_000;
/// 同一个 key 上连续 429 退避重试达到该次数后，若存在未冷却的备用 key，
/// 则交回外层切 key（在新 key 上重新计数 / 重试）；无备用 key 时继续退避到总次数上限。
const RATE_LIMIT_KEY_SWITCH_THRESHOLD: usize = 2;
/// 限流（429）专用的最小重试次数。限流是「等一会就能恢复」的暂时性错误（QPM/配额按时间桶
/// 刷新），值得比通用 `retry_attempts` 更耐心地退避重试——对标 Claude Code 对 rate-limit 的
/// 多次退避。仅在**无备用 key**（无法换 key 规避）时生效；有备用 key 时仍按阈值优先换 key。
const RATE_LIMIT_MAX_ATTEMPTS: usize = 8;

/// 获取实际的重试次数
/// 如果重试功能被禁用，则返回 1（即只尝试一次）
pub fn effective_retry_attempts(settings: &Settings) -> usize {
    if settings.retry_enabled {
        settings.retry_attempts as usize
    } else {
        1
    }
}

/// 从响应头中解析 Retry-After 值（秒），转换为毫秒延迟
fn parse_retry_after(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

/// 判断 HTTP 状态码是否属于"立即换 key"错误（坏 / 失效 key）：
/// - 401 鉴权失败（key 被吊销 / 错误）
/// - 402 需要付费（账户欠费）
/// - 403 权限不足 / 被封禁
/// 这些与 key 直接相关、在同一 key 上重试永远失败 → 内层不重试，立即交外层换 key。
/// 注意：429 不在此列 —— 429 由内层退避重试，仅在达到阈值且有备用 key 时才换 key。
fn is_immediate_failover_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 402 | 403)
}

/// 判断请求错误是否可重试
/// 包括超时和连接错误
fn is_retryable_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

/// 计算重试延迟
/// 优先使用服务器返回的 Retry-After 头；否则使用指数退避策略
fn retry_delay_ms(attempt: usize, retry_after: Option<u64>) -> u64 {
    if let Some(seconds) = retry_after {
        return seconds.saturating_mul(1000);
    }

    let delay = RETRY_BASE_DELAY_MS.saturating_mul(2u64.saturating_pow((attempt - 1) as u32));
    delay.min(RETRY_MAX_DELAY_MS)
}

fn parse_leading_status_code(value: &str) -> Option<u16> {
    let end = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    if end == 0 {
        return None;
    }
    value[..end].parse().ok()
}

/// 从 HTTP 错误信息中提取状态码
/// 格式约定：`"{label} Error: {status} - {body}"`，
/// status 形如 `"429 Too Many Requests"`，第一段数字即可
/// 兼容少数防御性分支使用的 `"{label} HTTP {status}: {body}"`。
/// 网络错误（reqwest::Error 路径）格式为 `"{label} Error: <reqwest msg>"`，无前导数字 → 返回 None
pub fn extract_status_code(err_msg: &str) -> Option<u16> {
    if let Some(idx) = err_msg.find(" Error: ") {
        let rest = &err_msg[idx + " Error: ".len()..];
        if let Some(code) = parse_leading_status_code(rest) {
            return Some(code);
        }
    }

    if let Some(idx) = err_msg.find(" HTTP ") {
        let rest = &err_msg[idx + " HTTP ".len()..];
        return parse_leading_status_code(rest);
    }

    None
}

/// 判断错误信息是否触发 key failover（外层换 key 决策）
/// 严格按 HTTP 状态码：401/402/403/429 才换 key —— 与 key 直接相关的错误：
/// - 401 鉴权失败（key 被吊销 / 错误）→ 内层不重试，立即换 key
/// - 402 需要付费（账户欠费）→ 立即换 key
/// - 403 权限不足 / 被封禁 → 立即换 key
/// - 429 限流（key 维度配额耗尽）→ 内层先退避重试；达到阈值且有备用 key 时，
///   429 错误冒泡到外层触发换 key（在新 key 上重新计数）
/// 其它 4xx（如 400 malformed body）属于请求本身问题，换 key 也无济于事 → 不触发
/// 5xx 由内层退避重试，正常不会到这里（除非耗尽次数；耗尽后非 key 问题，不换 key）
/// 网络错误（timeout / connect 失败）非 key 问题，extract_status_code 返回 None → 不触发
pub fn is_failover_error(err_msg: &str) -> bool {
    matches!(extract_status_code(err_msg), Some(401 | 402 | 403 | 429))
}

/// 多 key failover 包装：在 api_keys 列表上依次尝试，遇到 failover-eligible 错误自动切下一 key。
///
/// 错误分类（内层 vs 外层换 key）：
/// - **401/402/403（坏 / 失效 key）**：内层不重试，立即冒泡 → 外层换 key。
/// - **429（限流）**：内层在当前 key 退避重试；只有当**同一 key 连续 429 达到阈值**
///   `RATE_LIMIT_KEY_SWITCH_THRESHOLD` **且存在未冷却备用 key** 时，才让 429 冒泡 → 外层换 key
///   （换后在新 key 上重新计数 / 重试）；无备用 key 时继续退避到总次数上限。
/// - **5xx / timeout / connect（暂时性）**：内层退避重试，不换 key（不是 key 的问题）。
/// - **400 / 404 / 422 等确定性客户端错误**：不重试，快速失败。
///
/// 始终优先尊重 Retry-After。所有 key 用尽后返回最后一次错误（最终失败路径不变）。
pub async fn send_with_failover<F, Fut>(
    state: &AppState,
    label: &str,
    attempts: usize,
    provider_id: &str,
    api_keys: &[String],
    send: F,
) -> Result<reqwest::Response, String>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    send_with_failover_cancelable(
        state,
        label,
        attempts,
        provider_id,
        api_keys,
        || false,
        send,
    )
    .await
}

async fn send_with_failover_cancelable<F, Fut, C>(
    state: &AppState,
    label: &str,
    attempts: usize,
    provider_id: &str,
    api_keys: &[String],
    is_cancelled: C,
    send: F,
) -> Result<reqwest::Response, String>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
    C: Fn() -> bool + Send + Sync,
{
    let total = api_keys.len();
    if total == 0 {
        return Err(format!("{} Error: No API key configured", label));
    }

    let mut tried: HashSet<usize> = HashSet::new();
    let mut last_err: Option<String> = None;

    while tried.len() < total {
        if is_cancelled() {
            return Err(format!("{} cancelled", label));
        }
        let idx = match state.pick_active_key(provider_id, total, &tried) {
            Some(i) => i,
            None => break,
        };
        tried.insert(idx);
        let key = api_keys[idx].as_str();

        // 是否还有未试过的备用 key —— 决定 429 是否在阈值处提前交回外层换 key。
        let has_backup_key = state.pick_active_key(provider_id, total, &tried).is_some();
        let rate_limit_cap = if has_backup_key {
            Some(RATE_LIMIT_KEY_SWITCH_THRESHOLD)
        } else {
            None
        };

        match send_with_retry_for_failover(label, attempts, rate_limit_cap, &is_cancelled, || {
            send(key)
        })
        .await
        {
            Ok(resp) => {
                state.mark_key_ok(provider_id, idx);
                return Ok(resp);
            }
            Err(err_msg) => {
                if is_failover_error(&err_msg) && tried.len() < total {
                    state.mark_key_failed(provider_id, idx);
                    eprintln!(
                        "[failover] {} key #{}/{} failed, switching to next: {}",
                        label,
                        idx + 1,
                        total,
                        err_msg
                    );
                    last_err = Some(err_msg);
                    continue;
                }
                // 非 failover 错误（或已穷举所有 key）→ 直接返回
                if is_failover_error(&err_msg) {
                    state.mark_key_failed(provider_id, idx);
                }
                return Err(err_msg);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| format!("{} Error: all {} keys exhausted", label, total)))
}

/// 带重试机制的 HTTP 发送函数
/// 对可重试的错误（限流、服务器错误、超时、连接失败）进行指数退避重试
///
/// 职责边界:这里只做**传输层重试**(429 / 5xx / 网络超时连接错误的退避;坏 key 换 key)。
/// **语义级恢复**(上下文超长 overflow 的「压缩后重试」、内容审核去敏重试、确定性兜底)
/// 不在此处——归 `chat/agent/recovery.rs`(分类 + 策略中枢)+ `chat/agent/synthesis.rs`
/// (执行)。不要在这里加 overflow / 去敏判定,避免与上层语义恢复重复退避、放大延迟。
pub async fn send_with_retry<F, Fut>(
    label: &str,
    attempts: usize,
    mut send: F,
) -> Result<reqwest::Response, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let never_cancelled = || false;
    send_with_retry_status_policy(
        label,
        attempts,
        &mut send,
        FailoverRetryPolicy {
            rate_limit_cap: None,
        },
        &never_cancelled,
    )
    .await
}

/// failover 内层重试策略：
/// - 401/402/403：不重试，立即冒泡（外层换 key）。
/// - 429：退避重试；若 `rate_limit_cap` 为 Some(N)（有备用 key），同一 key 上第 N 次 429
///   后冒泡（外层换 key）；为 None（无备用 key）则退避到总次数上限。
/// - 5xx / timeout / connect：退避重试，不换 key。
/// - 其它确定性 4xx：不重试，快速失败。
async fn send_with_retry_for_failover<F, Fut>(
    label: &str,
    attempts: usize,
    rate_limit_cap: Option<usize>,
    is_cancelled: &(dyn Fn() -> bool + Send + Sync),
    mut send: F,
) -> Result<reqwest::Response, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    send_with_retry_status_policy(
        label,
        attempts,
        &mut send,
        FailoverRetryPolicy { rate_limit_cap },
        is_cancelled,
    )
    .await
}

/// 内层重试的状态分类策略。
#[derive(Clone, Copy)]
struct FailoverRetryPolicy {
    /// 429 退避重试的次数上限：Some(N) 表示有备用 key，同一 key 上第 N 次 429 后停止重试
    /// 并冒泡（让外层换 key）；None 表示无备用 key，429 与 5xx 一样退避到总次数上限。
    rate_limit_cap: Option<usize>,
}

impl FailoverRetryPolicy {
    /// 判断在第 `rate_limit_attempts` 次 429（含本次）后，是否还应继续在当前 key 上退避重试。
    /// - 有备用 key 且已达阈值 → false（停止重试 → 冒泡换 key）。
    /// - 无备用 key → true（继续退避，受总次数上限约束）。
    fn should_retry_rate_limit(&self, rate_limit_attempts: usize) -> bool {
        match self.rate_limit_cap {
            Some(cap) => rate_limit_attempts < cap,
            None => true,
        }
    }
}

async fn send_with_retry_status_policy<F, Fut>(
    label: &str,
    attempts: usize,
    send: &mut F,
    policy: FailoverRetryPolicy,
    is_cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<reqwest::Response, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<reqwest::Response, reqwest::Error>>,
{
    let attempts = attempts.max(1);
    // 限流（429）在无备用 key 时用更高的重试上限耐心退避；有备用 key 时维持通用上限
    // （阈值处会冒泡换 key，换 key 比干等更优）。其它错误（5xx/网络）仍走通用 `attempts`。
    let rate_limit_max = if policy.rate_limit_cap.is_none() {
        attempts.max(RATE_LIMIT_MAX_ATTEMPTS)
    } else {
        attempts
    };
    let loop_max = attempts.max(rate_limit_max);
    let mut last_error: Option<String> = None;
    // 同一 key 上累计的 429 次数，用于阈值化换 key。
    let mut rate_limit_attempts: usize = 0;

    for attempt in 1..=loop_max {
        if is_cancelled() {
            return Err(format!("{} cancelled", label));
        }
        match send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }

                // 401/402/403：坏 / 失效 key，内层不重试，立即冒泡（外层换 key）。
                if is_immediate_failover_status(status) {
                    let text = response.text().await.unwrap_or_default();
                    let err_msg = format!("{} Error: {} - {}", label, status, text);
                    return Err(format!("{} (attempt {}/{})", err_msg, attempt, attempts));
                }

                let is_rate_limit = status == StatusCode::TOO_MANY_REQUESTS;
                if is_rate_limit {
                    rate_limit_attempts += 1;
                }

                let retry_after = parse_retry_after(response.headers());
                let text = response.text().await.unwrap_or_default();
                let err_msg = format!("{} Error: {} - {}", label, status, text);

                // 429：受 key-switch 阈值约束 + 限流专用上限耐心退避；
                // 5xx：退避重试到通用次数；其它确定性 4xx：不重试快速失败。
                let (should_retry, shown_max) = if is_rate_limit {
                    (
                        policy.should_retry_rate_limit(rate_limit_attempts)
                            && attempt < rate_limit_max,
                        rate_limit_max,
                    )
                } else {
                    (status.is_server_error() && attempt < attempts, attempts)
                };

                if should_retry {
                    last_error = Some(err_msg);
                    let delay = retry_delay_ms(attempt, retry_after);
                    eprintln!(
                        "{} retrying in {}ms (attempt {}/{})",
                        label, delay, attempt, shown_max
                    );
                    if is_cancelled() {
                        return Err(format!("{} cancelled", label));
                    }
                    sleep_with_cancel(label, delay, is_cancelled).await?;
                    continue;
                }

                return Err(format!("{} (attempt {}/{})", err_msg, attempt, shown_max));
            }
            Err(err) => {
                let err_msg = format!("{} Error: {}", label, err);
                if is_retryable_error(&err) && attempt < attempts {
                    last_error = Some(err_msg);
                    let delay = retry_delay_ms(attempt, None);
                    eprintln!(
                        "{} retrying in {}ms (attempt {}/{})",
                        label, delay, attempt, attempts
                    );
                    if is_cancelled() {
                        return Err(format!("{} cancelled", label));
                    }
                    sleep_with_cancel(label, delay, is_cancelled).await?;
                    continue;
                }
                return Err(format!("{} (attempt {}/{})", err_msg, attempt, attempts));
            }
        }
    }

    Err(last_error
        .map(|msg| format!("{} (attempt {}/{})", msg, loop_max, loop_max))
        .unwrap_or_else(|| format!("{} Error: exceeded retry attempts ({})", label, loop_max)))
}

async fn sleep_with_cancel(
    label: &str,
    delay_ms: u64,
    is_cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<(), String> {
    const CANCEL_POLL_MS: u64 = 250;
    let mut remaining = delay_ms;
    while remaining > 0 {
        if is_cancelled() {
            return Err(format!("{} cancelled", label));
        }
        let step = remaining.min(CANCEL_POLL_MS);
        tokio::time::sleep(Duration::from_millis(step)).await;
        remaining -= step;
    }
    if is_cancelled() {
        return Err(format!("{} cancelled", label));
    }
    Ok(())
}

enum ApiUsageOutcome<'a> {
    Success(Option<ModelUsage>),
    Failure(&'a str),
    Cancelled(Option<ModelUsage>),
}

#[allow(clippy::too_many_arguments)]
fn record_api_usage(
    state: &AppState,
    provider: &settings::ModelProvider,
    model: &str,
    source: &str,
    operation: &str,
    started_at: i64,
    started: Instant,
    outcome: ApiUsageOutcome<'_>,
) {
    let (status, status_code, usage, usage_source, error_kind) = match outcome {
        ApiUsageOutcome::Success(usage) => ("success", Some(200), usage, "provider_reported", None),
        ApiUsageOutcome::Failure(error) => (
            "error",
            extract_status_code(error),
            None,
            "missing",
            Some(error_kind_from_message(error)),
        ),
        ApiUsageOutcome::Cancelled(usage) => (
            "cancelled",
            None,
            usage,
            "provider_reported",
            Some("cancelled".to_string()),
        ),
    };
    record_model_call(
        state,
        UsageRecordInput {
            provider,
            model,
            source,
            operation,
            status,
            status_code,
            usage,
            usage_source,
            started_at,
            duration_ms: started.elapsed().as_millis() as u64,
            conversation_id: None,
            message_id: None,
            error_kind,
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn record_api_usage_success(
    state: &AppState,
    provider: &settings::ModelProvider,
    model: &str,
    source: &str,
    operation: &str,
    started_at: i64,
    started: Instant,
    usage: Option<ModelUsage>,
) {
    record_api_usage(
        state,
        provider,
        model,
        source,
        operation,
        started_at,
        started,
        ApiUsageOutcome::Success(usage),
    );
}

#[allow(clippy::too_many_arguments)]
fn record_api_usage_failure(
    state: &AppState,
    provider: &settings::ModelProvider,
    model: &str,
    source: &str,
    operation: &str,
    started_at: i64,
    started: Instant,
    error: &str,
) {
    record_api_usage(
        state,
        provider,
        model,
        source,
        operation,
        started_at,
        started,
        ApiUsageOutcome::Failure(error),
    );
}

#[allow(clippy::too_many_arguments)]
fn record_api_usage_cancelled(
    state: &AppState,
    provider: &settings::ModelProvider,
    model: &str,
    source: &str,
    operation: &str,
    started_at: i64,
    started: Instant,
    usage: Option<ModelUsage>,
) {
    record_api_usage(
        state,
        provider,
        model,
        source,
        operation,
        started_at,
        started,
        ApiUsageOutcome::Cancelled(usage),
    );
}

// ===== Chat completion 调用 =====

/// 调用 OpenAI 兼容的文本聊天接口
/// 发送单轮 user 消息，temperature 设为 0.2,返回模型生成的文本内容
pub async fn call_openai_text(
    state: &State<'_, AppState>,
    config: &settings::ModelProvider,
    model: &str,
    prompt: String,
    retry_attempts: usize,
    thinking_enabled: bool,
    usage_source: &str,
    usage_operation: &str,
) -> Result<String, String> {
    if model.trim().is_empty() {
        return Err("Please select a model first".to_string());
    }
    let started_at = chrono::Local::now().timestamp();
    let started = Instant::now();
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
    let mut body = serde_json::json!({
      "model": model,
      "messages": [{ "role": "user", "content": prompt }],
      "temperature": 0.2
    });
    if !thinking_enabled && provider_supports_thinking_field(&config.base_url) {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    }

    let response = send_with_failover(
        state,
        "OpenAI API",
        retry_attempts,
        &config.id,
        &config.api_keys,
        |key| {
            with_standard_request_timeout(state.http.post(url.clone()).bearer_auth(key).json(&body))
                .send()
        },
    )
    .await
    .map_err(|err| {
        record_api_usage_failure(
            state.inner(),
            config,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;

    let value: serde_json::Value = response.json().await.map_err(|e| {
        let err = e.to_string();
        record_api_usage_failure(
            state.inner(),
            config,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;
    let usage = model_usage_from_openai_value(&value);
    let content = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| {
            let err = "Invalid response".to_string();
            record_api_usage_failure(
                state.inner(),
                config,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                &err,
            );
            err
        })?;

    record_api_usage_success(
        state.inner(),
        config,
        model,
        usage_source,
        usage_operation,
        started_at,
        started,
        usage,
    );
    Ok(content.trim().to_string())
}

/// 调用 OpenAI 兼容的 OCR/视觉接口
/// 将图片转为 Base64 后作为 image_url 类型内容发送，temperature 设为 0 以提高识别稳定性
pub async fn call_openai_ocr(
    state: &State<'_, AppState>,
    config: &settings::ModelProvider,
    model: &str,
    image_path: &Path,
    prompt: &str,
    retry_attempts: usize,
    thinking_enabled: bool,
    usage_source: &str,
    usage_operation: &str,
) -> Result<String, String> {
    if model.trim().is_empty() {
        return Err("Please select a model first".to_string());
    }
    let started_at = chrono::Local::now().timestamp();
    let started = Instant::now();
    let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
    let base64 = general_purpose::STANDARD.encode(bytes);
    let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

    // 与 lens 的 vision body 对齐：image 在 text 前、显式 max_tokens。
    // thinking 按调用方传入：截图翻译默认 false（节省时间），lens 默认 true。
    let mut body = serde_json::json!({
      "model": model,
      "messages": [
        {
          "role": "user",
          "content": [
            {
              "type": "image_url",
              "image_url": { "url": format!("data:image/png;base64,{base64}") }
            },
            {
              "type": "text",
              "text": prompt
            }
          ]
        }
      ],
      "temperature": 0.2,
      "max_tokens": 2000
    });
    if !thinking_enabled && provider_supports_thinking_field(&config.base_url) {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    }

    let response = send_with_failover(
        state,
        "OpenAI OCR",
        retry_attempts,
        &config.id,
        &config.api_keys,
        |key| {
            with_standard_request_timeout(state.http.post(url.clone()).bearer_auth(key).json(&body))
                .send()
        },
    )
    .await
    .map_err(|err| {
        record_api_usage_failure(
            state.inner(),
            config,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;

    // 显式检查 HTTP 状态：非 2xx 把原始 body 文本带回，避免后续 .json() 抛出含糊的 "error decoding response body"
    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let snippet: String = body_text.chars().take(500).collect();
        let err = format!("OCR HTTP {}: {}", status.as_u16(), snippet);
        record_api_usage_failure(
            state.inner(),
            config,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        return Err(err);
    }

    let raw = response.text().await.map_err(|e| {
        let err = format!("OCR read body: {}", e);
        record_api_usage_failure(
            state.inner(),
            config,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        let err = format!(
            "OCR parse JSON: {} (body: {})",
            e,
            raw.chars().take(500).collect::<String>()
        );
        record_api_usage_failure(
            state.inner(),
            config,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;
    let usage = model_usage_from_openai_value(&value);
    let content = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| {
            let err = format!(
                "Invalid OCR response: {}",
                raw.chars().take(500).collect::<String>()
            );
            record_api_usage_failure(
                state.inner(),
                config,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                &err,
            );
            err
        })?;

    record_api_usage_success(
        state.inner(),
        config,
        model,
        usage_source,
        usage_operation,
        started_at,
        started,
        usage,
    );
    Ok(content.trim().to_string())
}

/// 调用视觉 API（截图解释 / Lens 共用）
/// 支持流式输出：如果 stream 为 true，通过 stream_vision_response 逐段 emit `event_name` 事件。
/// `provider_id_override` 非空时使用指定 provider/model（用于 lens 选择独立模型）；空则走 explain 配置。
#[allow(clippy::too_many_arguments)]
pub async fn call_vision_api(
    app: &AppHandle,
    state: &State<'_, AppState>,
    image_id: &str,
    messages: Vec<ExplainMessage>,
    language: &str,
    retry_attempts: usize,
    stream: bool,
    stream_kind: &str,
    event_name: &str,
    provider_id_override: Option<&str>,
    model_override: Option<&str>,
    system_prompt_override: Option<&str>,
    thinking_enabled: bool,
    usage_source: &str,
    usage_operation: &str,
) -> Result<String, String> {
    let settings = state.settings_read().clone();
    let provider_id = provider_id_override
        .filter(|s| !s.is_empty())
        .unwrap_or(&settings.translator_provider_id);
    let provider = settings
        .get_provider(provider_id)
        .ok_or_else(|| "Vision provider not found".to_string())?;

    // image_id 为空 → 走纯文本对话路径（不附图）
    let has_image = !image_id.is_empty();

    // 优先用调用方传入的 system_prompt_override；否则用默认模板（区分有/无图片）
    // 关闭思考时在 system 末尾追加显式禁止指令，作为参数层不生效时的兜底
    let system_prompt_to_use = {
        let base = match system_prompt_override.filter(|s| !s.is_empty()) {
            Some(s) => s.to_string(),
            None => default_lens_system_prompt(language, has_image),
        };
        if !thinking_enabled {
            format!("{}{}", base, no_think_instruction(language))
        } else {
            base
        }
    };

    let mut api_messages = Vec::new();
    api_messages.push(serde_json::json!({
      "role": "system",
      "content": system_prompt_to_use
    }));

    if has_image {
        let image_path = resolve_explain_image_path(app, state, image_id)?;
        let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
        let base64 = general_purpose::STANDARD.encode(bytes);
        if let Some(first) = messages.first() {
            api_messages.push(serde_json::json!({
        "role": "user",
        "content": [
          { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{base64}") } },
          { "type": "text", "text": first.content }
        ]
      }));
            for message in messages.iter().skip(1) {
                api_messages.push(serde_json::json!({
                  "role": message.role,
                  "content": message.content
                }));
            }
        }
    } else {
        // 纯文本：每条 message 直接 push（无图）
        for message in messages.iter() {
            api_messages.push(serde_json::json!({
              "role": message.role,
              "content": message.content,
            }));
        }
    }

    let model = model_override
        .filter(|s| !s.is_empty())
        .unwrap_or(&settings.translator_model);
    if model.trim().is_empty() {
        return Err("Please select a model first".to_string());
    }
    let started_at = chrono::Local::now().timestamp();
    let started = Instant::now();
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let mut body = serde_json::json!({
      "model": model,
      "messages": api_messages,
      "temperature": 0.7,
      "max_tokens": 2000
    });
    if stream {
        body["stream"] = serde_json::json!(true);
    }

    // 关闭思考模式：仅塞 DeepSeek/Kimi 官方文档约定的 thinking={type:"disabled"} 字段。
    // 不注入 chat_template_kwargs / enable_thinking —— 这俩是 vLLM/Qwen 私有字段，第三方代理
    // （如 OpenRouter / 反代）做严格校验会以 400 拒绝整个请求（实测 DeepSeek 路径上 chat_template_kwargs
    // 直接报错）。注：reasoning_effort 是 OpenAI 标准参数（非私有），chat 路径的「思考等级」按需注入；
    // 这里是 lens/翻译路径，保持仅 thinking:disabled。提示词层的 no-think 指令是更稳的兜底。
    if !thinking_enabled && provider_supports_thinking_field(&provider.base_url) {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    }

    let response = send_with_failover(
        state,
        "Vision API",
        retry_attempts,
        &provider.id,
        &provider.api_keys,
        |key| {
            let request = state.http.post(url.clone()).bearer_auth(key).json(&body);
            let request = if stream {
                request
            } else {
                with_standard_request_timeout(request)
            };
            request.send()
        },
    )
    .await
    .map_err(|err| {
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;

    // 先检查 HTTP 状态：非 2xx 直接读出 body 文本作为错误，避免后续 .json() / chunk() 拿到非预期格式时抛出含糊的 "error decoding response body"。
    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let snippet = body_text.chars().take(500).collect::<String>();
        let err = format!("Vision API HTTP {}: {}", status.as_u16(), snippet);
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        return Err(err);
    }

    if stream {
        // 启动新流：递增代号，存到本流持有的快照里；后续 chunk 循环只要发现全局代号 != 自己的快照就退出。
        let generation = state
            .explain_stream_generation
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let stream_result = stream_vision_response(
            app,
            response,
            image_id,
            stream_kind,
            event_name,
            &state.explain_stream_generation,
            generation,
        )
        .await
        .map_err(|err| {
            record_api_usage_failure(
                state.inner(),
                provider,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                &err,
            );
            err
        })?;
        if stream_result.cancelled {
            record_api_usage_cancelled(
                state.inner(),
                provider,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                stream_result.usage,
            );
        } else {
            record_api_usage_success(
                state.inner(),
                provider,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                stream_result.usage,
            );
        }
        return Ok(stream_result.text);
    }

    // 非流式：先读 raw text，再 parse JSON，把原始 body 作为错误信息便于诊断。
    let raw = response.text().await.map_err(|e| {
        let err = format!("Vision API read body: {}", e);
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            if let Some(content) = parse_sse_chat_content(&raw) {
                record_api_usage_success(
                    state.inner(),
                    provider,
                    model,
                    usage_source,
                    usage_operation,
                    started_at,
                    started,
                    None,
                );
                return Ok(content);
            }
            let err = format!(
                "Vision API parse JSON: {} (body: {})",
                err,
                raw.chars().take(500).collect::<String>()
            );
            record_api_usage_failure(
                state.inner(),
                provider,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                &err,
            );
            return Err(err);
        }
    };
    let usage = model_usage_from_openai_value(&value);
    let content = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| {
            let err = format!(
                "Invalid vision response: {}",
                raw.chars().take(500).collect::<String>()
            );
            record_api_usage_failure(
                state.inner(),
                provider,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                &err,
            );
            err
        })?;

    record_api_usage_success(
        state.inner(),
        provider,
        model,
        usage_source,
        usage_operation,
        started_at,
        started,
        usage,
    );
    Ok(content.trim().to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamTextMode {
    Delta,
    Snapshot,
}

#[derive(Debug, Clone)]
struct StreamCallResult {
    text: String,
    usage: Option<ModelUsage>,
    cancelled: bool,
}

fn extract_sse_chat_text(value: &serde_json::Value) -> Option<(&str, StreamTextMode)> {
    let choice = value.get("choices").and_then(|choices| choices.get(0))?;

    if let Some(content) = choice
        .get("delta")
        .and_then(|delta| delta.get("content"))
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
    {
        return Some((content, StreamTextMode::Delta));
    }

    choice
        .get("message")
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .filter(|content| !content.is_empty())
        .map(|content| (content, StreamTextMode::Snapshot))
}

fn append_stream_text(full: &mut String, text: &str, mode: StreamTextMode) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    match mode {
        StreamTextMode::Delta => {
            full.push_str(text);
            Some(text.to_string())
        }
        StreamTextMode::Snapshot => {
            if text == full || full.starts_with(text) {
                return None;
            }
            if text.starts_with(full.as_str()) {
                let delta = text[full.len()..].to_string();
                full.clear();
                full.push_str(text);
                return if delta.is_empty() { None } else { Some(delta) };
            }

            full.push_str(text);
            Some(text.to_string())
        }
    }
}

fn parse_sse_chat_content(raw: &str) -> Option<String> {
    let mut full = String::new();
    for line in raw.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let data = line.trim_start_matches("data:").trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if let Some((content, mode)) = extract_sse_chat_text(&value) {
            let _ = append_stream_text(&mut full, content, mode);
        }
    }
    let full = full.trim();
    if full.is_empty() {
        None
    } else {
        Some(full.to_string())
    }
}

// ===== SSE 流 =====

/// 构造带 image 的 OCR/视觉请求 body（model 由调用方注入），开启 stream
pub fn build_ocr_request_body(
    image_path: &Path,
    prompt: &str,
    thinking_enabled: bool,
    provider_base_url: &str,
) -> Result<serde_json::Value, String> {
    let bytes = fs::read(image_path).map_err(|e| e.to_string())?;
    let base64 = general_purpose::STANDARD.encode(bytes);
    let mut body = serde_json::json!({
      "messages": [{
        "role": "user",
        "content": [
          { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{base64}") } },
          { "type": "text", "text": prompt }
        ]
      }],
      "temperature": 0.2,
      "max_tokens": 2000,
      "stream": true
    });
    if !thinking_enabled && provider_supports_thinking_field(provider_base_url) {
        body["thinking"] = serde_json::json!({ "type": "disabled" });
    }
    Ok(body)
}

/// 通用流式 chat 调用：发送 body（model 在外层注入）→ 解析 SSE → 通过 stream_vision_response emit。
/// 复用 explain_stream_generation 作取消代号（lens-stream / lens-translate-stream 都共用）。
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat_call(
    app: &AppHandle,
    state: &State<'_, AppState>,
    provider: &settings::ModelProvider,
    model: &str,
    mut body: serde_json::Value,
    retry_attempts: usize,
    image_id: &str,
    kind: &str,
    event_name: &str,
    usage_source: &str,
    usage_operation: &str,
) -> Result<String, String> {
    if model.trim().is_empty() {
        return Err("Please select a model first".to_string());
    }
    let started_at = chrono::Local::now().timestamp();
    let started = Instant::now();
    body["model"] = serde_json::json!(model);
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let generation = state
        .explain_stream_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;

    let response = send_with_failover_cancelable(
        state,
        "Stream chat",
        retry_attempts,
        &provider.id,
        &provider.api_keys,
        || state.explain_stream_generation.load(Ordering::SeqCst) != generation,
        |key| {
            state
                .http
                .post(url.clone())
                .bearer_auth(key)
                .json(&body)
                .send()
        },
    )
    .await
    .map_err(|err| {
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let snippet: String = body_text.chars().take(500).collect();
        let err = format!("Stream HTTP {}: {}", status.as_u16(), snippet);
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        return Err(err);
    }

    let stream_result = stream_vision_response(
        app,
        response,
        image_id,
        kind,
        event_name,
        &state.explain_stream_generation,
        generation,
    )
    .await
    .map_err(|err| {
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;
    if stream_result.cancelled {
        record_api_usage_cancelled(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            stream_result.usage,
        );
    } else {
        record_api_usage_success(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            stream_result.usage,
        );
    }
    Ok(stream_result.text)
}

/// 截图翻译合并模式流：单次调用模型，按 `<<<ORIGINAL>>>` 分隔符把 SSE delta 拆成两段。
/// 分隔符前的 chunk emit kind="translated"；分隔符后的 chunk emit kind="original"。
/// 返回 (translated, original) 完整文本。
///
/// 关键点：
/// - 分隔符可能跨 SSE chunk 边界 → 用 tail 缓冲住末尾 (SEPARATOR.len()-1) 字节防止把分隔符前缀当成译文 emit 出去
/// - tail 切片必须落在 UTF-8 char boundary，否则 String::drain 会 panic（用户截图常含 CJK，每字 3 字节）
#[allow(clippy::too_many_arguments)]
pub async fn stream_translate_combined(
    app: &AppHandle,
    state: &State<'_, AppState>,
    provider: &settings::ModelProvider,
    model: &str,
    mut body: serde_json::Value,
    retry_attempts: usize,
    image_id: &str,
    event_name: &str,
    usage_source: &str,
    usage_operation: &str,
) -> Result<(String, String), String> {
    if model.trim().is_empty() {
        return Err("Please select a model first".to_string());
    }
    let started_at = chrono::Local::now().timestamp();
    let started = Instant::now();
    body["model"] = serde_json::json!(model);
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    let my_gen = state
        .explain_stream_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;

    let mut response = send_with_failover_cancelable(
        state,
        "Stream translate combined",
        retry_attempts,
        &provider.id,
        &provider.api_keys,
        || state.explain_stream_generation.load(Ordering::SeqCst) != my_gen,
        |key| {
            state
                .http
                .post(url.clone())
                .bearer_auth(key)
                .json(&body)
                .send()
        },
    )
    .await
    .map_err(|err| {
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        err
    })?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let snippet: String = body_text.chars().take(500).collect();
        let err = format!("Stream HTTP {}: {}", status.as_u16(), snippet);
        record_api_usage_failure(
            state.inner(),
            provider,
            model,
            usage_source,
            usage_operation,
            started_at,
            started,
            &err,
        );
        return Err(err);
    }

    let sep = COMBINED_TRANSLATE_SEPARATOR;
    let sep_len = sep.len();

    let mut sse_buf = String::new();
    let mut tail = String::new();
    let mut streamed_content = String::new();
    let mut translated = String::new();
    let mut original = String::new();
    let mut sep_seen = false;
    let mut usage: Option<ModelUsage> = None;

    let emit_done = |reason: &str| {
        let _ = app.emit(
            event_name,
            serde_json::json!({
              "imageId": image_id, "delta": "", "done": true, "reason": reason,
            }),
        );
    };

    loop {
        if state.explain_stream_generation.load(Ordering::SeqCst) != my_gen {
            // 取消：把 tail 当作 translated flush（避免末尾几个字符丢失），再 emit done
            if !tail.is_empty() && !sep_seen {
                translated.push_str(&tail);
                let _ = app.emit(
                    event_name,
                    serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
                );
            }
            emit_done("cancelled");
            record_api_usage_cancelled(
                state.inner(),
                provider,
                model,
                usage_source,
                usage_operation,
                started_at,
                started,
                usage,
            );
            return Ok((translated, original));
        }

        let chunk = match response.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                emit_done("error");
                let err = e.to_string();
                record_api_usage_failure(
                    state.inner(),
                    provider,
                    model,
                    usage_source,
                    usage_operation,
                    started_at,
                    started,
                    &err,
                );
                return Err(err);
            }
        };

        let text = String::from_utf8_lossy(&chunk);
        sse_buf.push_str(&text);

        while let Some(pos) = sse_buf.find('\n') {
            let line: String = sse_buf.drain(..=pos).collect();
            let line = line.trim();
            if !line.starts_with("data:") {
                continue;
            }
            let data = line.trim_start_matches("data:").trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                // flush tail
                if !sep_seen && !tail.is_empty() {
                    translated.push_str(&tail);
                    let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
          );
                }
                emit_done("done");
                record_api_usage_success(
                    state.inner(),
                    provider,
                    model,
                    usage_source,
                    usage_operation,
                    started_at,
                    started,
                    usage,
                );
                return Ok((translated, original));
            }

            let value: serde_json::Value = match serde_json::from_str(data) {
                Ok(val) => val,
                Err(_) => continue,
            };
            if let Some(next_usage) = model_usage_from_stream_value(&value) {
                usage = Some(next_usage);
            }

            let delta_obj = value
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("delta"));

            // 推理链 emit（恒定 kind="translated"，前端在主面板渲染）
            if let Some(r) = delta_obj
                .and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning")))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                let _ = app.emit(
                    event_name,
                    serde_json::json!({
                      "imageId": image_id, "kind": "translated", "delta": "", "reasoningDelta": r,
                    }),
                );
            }

            let Some((content, mode)) = extract_sse_chat_text(&value) else {
                continue;
            };
            let Some(content_delta) = append_stream_text(&mut streamed_content, content, mode)
            else {
                continue;
            };
            let c = content_delta.as_str();

            if sep_seen {
                original.push_str(c);
                let _ = app.emit(
                    event_name,
                    serde_json::json!({ "imageId": image_id, "kind": "original", "delta": c }),
                );
                continue;
            }

            tail.push_str(c);
            if let Some(idx) = tail.find(sep) {
                // 分隔符命中：拆 before / after，trim 掉分隔符相邻的换行，分别发出
                let before: String = tail.drain(..idx).collect();
                // 移除分隔符本身
                tail.drain(..sep_len);
                let after: String = std::mem::take(&mut tail);

                let before_emit = before.trim_end_matches('\n').to_string();
                if !before_emit.is_empty() {
                    translated.push_str(&before_emit);
                    let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": before_emit }),
          );
                }
                sep_seen = true;
                let after_emit = after.trim_start_matches('\n').to_string();
                if !after_emit.is_empty() {
                    original.push_str(&after_emit);
                    let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "original", "delta": after_emit }),
          );
                }
            } else {
                // 没命中：emit 安全前缀（保留末尾 sep_len-1 字节防止跨 chunk 分隔符被切碎）
                let max_emit = tail.len().saturating_sub(sep_len.saturating_sub(1));
                if max_emit == 0 {
                    continue;
                }
                // 找一个合法 char boundary（CJK 字符多字节，不能切到字符中间）
                let mut safe = max_emit;
                while safe > 0 && !tail.is_char_boundary(safe) {
                    safe -= 1;
                }
                if safe == 0 {
                    continue;
                }
                let to_emit: String = tail.drain(..safe).collect();
                translated.push_str(&to_emit);
                let _ = app.emit(
          event_name,
          serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": to_emit }),
        );
            }
        }
    }

    // SSE 流结束（连接关闭）但没收到 [DONE]：flush tail
    if !sep_seen && !tail.is_empty() {
        translated.push_str(&tail);
        let _ = app.emit(
            event_name,
            serde_json::json!({ "imageId": image_id, "kind": "translated", "delta": tail }),
        );
    }
    emit_done("done");
    record_api_usage_success(
        state.inner(),
        provider,
        model,
        usage_source,
        usage_operation,
        started_at,
        started,
        usage,
    );
    Ok((translated, original))
}

/// 流式解析视觉 API 的 SSE 响应
/// 逐 chunk 读取响应体，解析 "data:" 行，提取 delta 中的 content 并通过 `event_name` emit。
/// 支持取消：调用方持有 `my_generation`，全局代号 `generation_atom` 一旦变化即视为被新流或外部取消作废。
async fn stream_vision_response(
    app: &AppHandle,
    mut response: reqwest::Response,
    image_id: &str,
    kind: &str,
    event_name: &str,
    generation_atom: &AtomicU64,
    my_generation: u64,
) -> Result<StreamCallResult, String> {
    let mut buffer = String::new();
    let mut full = String::new();
    let mut reasoning_full = String::new();
    let mut usage: Option<ModelUsage> = None;

    let emit_done = |reason: &str, full_text: &str| {
        let _ = app.emit(
            event_name,
            serde_json::json!({
              "imageId": image_id,
              "kind": kind,
              "delta": "",
              "done": true,
              "reason": reason,
              "full": full_text,
            }),
        );
    };

    loop {
        if generation_atom.load(Ordering::SeqCst) != my_generation {
            emit_done("cancelled", full.trim());
            return Ok(StreamCallResult {
                text: full.trim().to_string(),
                usage,
                cancelled: true,
            });
        }

        let chunk = match response.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                emit_done("error", full.trim());
                return Err(e.to_string());
            }
        };

        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);

        while let Some(pos) = buffer.find('\n') {
            let line: String = buffer.drain(..=pos).collect();
            let line = line.trim();
            if !line.starts_with("data:") {
                continue;
            }
            let data = line.trim_start_matches("data:").trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                emit_done("done", full.trim());
                return Ok(StreamCallResult {
                    text: full.trim().to_string(),
                    usage,
                    cancelled: false,
                });
            }

            let value: serde_json::Value = match serde_json::from_str(data) {
                Ok(val) => val,
                Err(_) => continue,
            };
            if let Some(next_usage) = model_usage_from_stream_value(&value) {
                usage = Some(next_usage);
            }

            let delta_obj = value
                .get("choices")
                .and_then(|choices| choices.get(0))
                .and_then(|choice| choice.get("delta"));

            // 推理模型（DeepSeek-R1 / Kimi 等）把链路放在 delta.reasoning_content
            // 部分实现用 delta.reasoning。两种字段都尝试取，只要有就 emit。
            let reasoning = delta_obj
                .and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning")))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());

            if let Some(r) = reasoning {
                let Some(reasoning_delta) =
                    append_stream_text(&mut reasoning_full, r, StreamTextMode::Delta)
                else {
                    continue;
                };
                let _ = app.emit(
                    event_name,
                    serde_json::json!({
                      "imageId": image_id,
                      "kind": kind,
                      "delta": "",
                      "reasoningDelta": reasoning_delta,
                    }),
                );
            }

            if let Some((content, mode)) = extract_sse_chat_text(&value) {
                let Some(delta) = append_stream_text(&mut full, content, mode) else {
                    continue;
                };
                let _ = app.emit(
                    event_name,
                    serde_json::json!({ "imageId": image_id, "kind": kind, "delta": delta }),
                );
            }
        }
    }

    emit_done("done", full.trim());
    Ok(StreamCallResult {
        text: full.trim().to_string(),
        usage,
        cancelled: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== attach_json_body (gzip) =====

    #[test]
    fn attach_json_body_plain_when_gzip_off() {
        let client = Client::new();
        let body =
            serde_json::json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
        let req = attach_json_body(client.post("http://x.invalid/v1"), &body, false)
            .build()
            .expect("build");
        assert!(req
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_none());
        let sent = req
            .body()
            .and_then(|b| b.as_bytes())
            .expect("in-memory body");
        let parsed: serde_json::Value = serde_json::from_slice(sent).expect("plain json");
        assert_eq!(parsed, body);
    }

    #[test]
    fn attach_json_body_gzips_and_round_trips_when_gzip_on() {
        use std::io::Read as _;
        let client = Client::new();
        let body = serde_json::json!({"model": "m", "cmd": "rm -rf /tmp/x && cat /etc/passwd"});
        let req = attach_json_body(client.post("http://x.invalid/v1"), &body, true)
            .build()
            .expect("build");
        assert_eq!(
            req.headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .unwrap(),
            "gzip"
        );
        let gz = req
            .body()
            .and_then(|b| b.as_bytes())
            .expect("in-memory body");
        // 压缩体必须能解回原始 JSON。
        let mut dec = flate2::read::GzDecoder::new(gz);
        let mut raw = Vec::new();
        dec.read_to_end(&mut raw).expect("gunzip");
        let parsed: serde_json::Value = serde_json::from_slice(&raw).expect("round-trip json");
        assert_eq!(parsed, body);
    }

    #[test]
    fn extract_status_code_parses_typical_send_with_retry_format() {
        // send_with_retry 拼出来的标准格式
        let s = "OpenAI API Error: 429 Too Many Requests - {\"error\":\"rate_limit\"}";
        assert_eq!(extract_status_code(s), Some(429));
    }

    #[test]
    fn extract_status_code_handles_each_failover_status() {
        assert_eq!(
            extract_status_code("X Error: 401 Unauthorized - body"),
            Some(401)
        );
        assert_eq!(
            extract_status_code("X Error: 402 Payment Required - body"),
            Some(402)
        );
        assert_eq!(
            extract_status_code("X Error: 403 Forbidden - body"),
            Some(403)
        );
        assert_eq!(
            extract_status_code("X Error: 429 Too Many Requests - body"),
            Some(429)
        );
    }

    #[test]
    fn extract_status_code_handles_defensive_http_format() {
        assert_eq!(
            extract_status_code("Stream HTTP 429: rate limited"),
            Some(429)
        );
        assert_eq!(
            extract_status_code("Stream HTTP 401: unauthorized"),
            Some(401)
        );
        assert_eq!(
            extract_status_code("Vision API HTTP 403: forbidden"),
            Some(403)
        );
    }

    #[test]
    fn extract_status_code_handles_non_failover_status() {
        assert_eq!(
            extract_status_code("X Error: 400 Bad Request - body"),
            Some(400)
        );
        assert_eq!(
            extract_status_code("X Error: 500 Internal Server Error - body"),
            Some(500)
        );
    }

    #[test]
    fn extract_status_code_returns_none_for_network_error() {
        // reqwest::Error 路径无前导数字
        let s = "Stream chat Error: error sending request: connection refused (attempt 3/3)";
        assert_eq!(extract_status_code(s), None);
    }

    #[test]
    fn extract_status_code_returns_none_when_marker_missing() {
        assert_eq!(extract_status_code("just some message"), None);
        assert_eq!(extract_status_code(""), None);
    }

    // ===== is_failover_error =====

    #[test]
    fn is_failover_error_only_triggers_on_auth_quota_codes() {
        assert!(is_failover_error("X Error: 401 - body"));
        assert!(is_failover_error("X Error: 402 - body"));
        assert!(is_failover_error("X Error: 403 - body"));
        assert!(is_failover_error("X Error: 429 - body"));
        assert!(is_failover_error("Stream HTTP 429: rate limited"));
        assert!(is_failover_error("Stream HTTP 401: unauthorized"));
    }

    #[test]
    fn is_failover_error_does_not_trigger_on_400_or_5xx() {
        // 400 是请求 body 问题，不应换 key
        assert!(!is_failover_error("X Error: 400 Bad Request - body"));
        assert!(!is_failover_error("Stream HTTP 400: bad request"));
        // 500 由 send_with_retry 内部退避重试，不应到 failover 层
        assert!(!is_failover_error(
            "X Error: 500 Internal Server Error - body"
        ));
        assert!(!is_failover_error(
            "X Error: 503 Service Unavailable - body"
        ));
    }

    #[test]
    fn is_failover_error_does_not_trigger_on_network_failure() {
        // 网络问题不是 key 的锅
        assert!(!is_failover_error(
            "Stream Error: error sending request: timed out"
        ));
        assert!(!is_failover_error("X Error: connection closed"));
    }

    #[test]
    fn is_failover_error_does_not_trigger_on_body_keywords_alone() {
        // 旧版宽泛匹配 body 含 "billing" / "quota" 会误触发；现版严格按状态码
        assert!(!is_failover_error(
            "X Error: 400 - {\"message\":\"billing issue\"}"
        ));
        assert!(!is_failover_error(
            "X Error: 500 - {\"message\":\"quota exceeded\"}"
        ));
    }

    #[test]
    fn is_failover_error_still_triggers_on_429() {
        // 429 仍是 failover-eligible：内层退避到阈值后冒泡，外层据此换 key。
        assert!(is_failover_error("X Error: 429 Too Many Requests - body"));
    }

    // ===== 错误分类（is_immediate_failover_status / FailoverRetryPolicy） =====

    #[test]
    fn immediate_failover_status_covers_auth_codes_only() {
        assert!(is_immediate_failover_status(StatusCode::UNAUTHORIZED)); // 401
        assert!(is_immediate_failover_status(StatusCode::PAYMENT_REQUIRED)); // 402
        assert!(is_immediate_failover_status(StatusCode::FORBIDDEN)); // 403
                                                                      // 429 不是 immediate failover —— 由内层退避重试。
        assert!(!is_immediate_failover_status(StatusCode::TOO_MANY_REQUESTS));
        // 5xx / 4xx 确定性错误也不是 immediate failover。
        assert!(!is_immediate_failover_status(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(!is_immediate_failover_status(StatusCode::BAD_REQUEST));
        assert!(!is_immediate_failover_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn rate_limit_policy_caps_at_threshold_when_backup_key_available() {
        let policy = FailoverRetryPolicy {
            rate_limit_cap: Some(RATE_LIMIT_KEY_SWITCH_THRESHOLD),
        };
        // 阈值 N=2：第 1 次 429 后继续重试，第 N 次后停止（冒泡换 key）。
        assert!(policy.should_retry_rate_limit(1));
        assert!(!policy.should_retry_rate_limit(RATE_LIMIT_KEY_SWITCH_THRESHOLD));
        assert!(!policy.should_retry_rate_limit(RATE_LIMIT_KEY_SWITCH_THRESHOLD + 1));
    }

    #[test]
    fn rate_limit_policy_retries_indefinitely_without_backup_key() {
        let policy = FailoverRetryPolicy {
            rate_limit_cap: None,
        };
        // 无备用 key：429 一直可重试（受外层总次数上限约束）。
        assert!(policy.should_retry_rate_limit(1));
        assert!(policy.should_retry_rate_limit(5));
        assert!(policy.should_retry_rate_limit(99));
    }

    #[test]
    fn append_stream_text_emits_delta_chunks_as_is() {
        let mut full = String::new();

        assert_eq!(
            append_stream_text(&mut full, "你", StreamTextMode::Delta),
            Some("你".to_string())
        );
        assert_eq!(
            append_stream_text(&mut full, "好", StreamTextMode::Delta),
            Some("好".to_string())
        );
        assert_eq!(full, "你好");
    }

    #[test]
    fn append_stream_text_converts_snapshots_to_suffix_deltas() {
        let mut full = String::new();

        assert_eq!(
            append_stream_text(&mut full, "你", StreamTextMode::Snapshot),
            Some("你".to_string())
        );
        assert_eq!(
            append_stream_text(&mut full, "你好", StreamTextMode::Snapshot),
            Some("好".to_string())
        );
        assert_eq!(
            append_stream_text(&mut full, "你好", StreamTextMode::Snapshot),
            None
        );
        assert_eq!(full, "你好");
    }

    #[test]
    fn parse_sse_chat_content_handles_cumulative_message_snapshots() {
        let raw = r#"
data: {"choices":[{"message":{"content":"你"}}]}
data: {"choices":[{"message":{"content":"你好"}}]}
data: {"choices":[{"message":{"content":"你好，世界"}}]}
data: [DONE]
"#;

        assert_eq!(parse_sse_chat_content(raw), Some("你好，世界".to_string()));
    }

    // ===== retry_delay_ms / parse_retry_after =====

    #[test]
    fn retry_delay_starts_around_five_seconds_and_caps() {
        // 起步 ~5s（RETRY_BASE_DELAY_MS）；指数退避封顶 RETRY_MAX_DELAY_MS（30s）。
        assert_eq!(retry_delay_ms(1, None), RETRY_BASE_DELAY_MS);
        assert_eq!(retry_delay_ms(2, None), RETRY_BASE_DELAY_MS * 2);
        // 第 4 次本应 5s*8=40s，被 cap 到 30s。
        assert_eq!(retry_delay_ms(4, None), RETRY_MAX_DELAY_MS);
        assert_eq!(retry_delay_ms(10, None), RETRY_MAX_DELAY_MS);
    }

    #[test]
    fn retry_delay_prefers_retry_after_over_backoff() {
        // Retry-After 优先：哪怕退避会算出别的值，也用服务器给的秒数。
        assert_eq!(retry_delay_ms(1, Some(7)), 7_000);
        assert_eq!(retry_delay_ms(5, Some(2)), 2_000);
    }

    #[test]
    fn parse_retry_after_reads_seconds_header() {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "12".parse().unwrap());
        assert_eq!(parse_retry_after(&headers), Some(12));

        let empty = HeaderMap::new();
        assert_eq!(parse_retry_after(&empty), None);
    }

    // ===== send_with_retry_status_policy / send_with_failover 行为（mock send 闭包） =====

    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;

    /// 用一个 reqwest::Error 模拟网络层错误（timeout/connect）。
    /// 通过对一个不可路由地址发起极短超时请求来获得真实的 reqwest::Error。
    async fn make_network_error() -> reqwest::Error {
        // 192.0.2.0/24 是 TEST-NET-1，保证不可路由 → connect/timeout 错误。
        Client::builder()
            .connect_timeout(Duration::from_millis(1))
            .build()
            .unwrap()
            .get("http://192.0.2.1:9/")
            .timeout(Duration::from_millis(1))
            .send()
            .await
            .expect_err("expected a network error")
    }

    /// 构造一个带指定状态码与可选 retry-after 的 reqwest::Response（不走网络）。
    fn make_response(status: u16, retry_after: Option<u64>) -> reqwest::Response {
        let mut builder = http::Response::builder().status(status);
        if let Some(secs) = retry_after {
            builder = builder.header("retry-after", secs.to_string());
        }
        let http_resp = builder.body("body").expect("build http response");
        reqwest::Response::from(http_resp)
    }

    fn test_never_cancelled() -> bool {
        false
    }

    #[tokio::test(start_paused = true)]
    async fn server_error_retries_up_to_attempt_limit() {
        let attempts = 5;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_inner = Arc::clone(&calls);

        let result = send_with_retry_status_policy(
            "Test",
            attempts,
            &mut || {
                let calls = Arc::clone(&calls_inner);
                async move {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(make_response(500, None))
                }
            },
            FailoverRetryPolicy {
                rate_limit_cap: None,
            },
            &test_never_cancelled,
        )
        .await;

        assert!(result.is_err());
        // 5xx 一直重试到 attempts 次。
        assert_eq!(calls.load(AtomicOrdering::SeqCst), attempts);
    }

    #[tokio::test(start_paused = true)]
    async fn network_error_retries_up_to_attempt_limit() {
        let attempts = 5;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_inner = Arc::clone(&calls);

        let result = send_with_retry_status_policy(
            "Test",
            attempts,
            &mut || {
                let calls = Arc::clone(&calls_inner);
                async move {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    Err(make_network_error().await)
                }
            },
            FailoverRetryPolicy {
                rate_limit_cap: None,
            },
            &test_never_cancelled,
        )
        .await;

        assert!(result.is_err());
        // timeout/connect 网络错误也重试到上限。
        assert_eq!(calls.load(AtomicOrdering::SeqCst), attempts);
    }

    #[tokio::test(start_paused = true)]
    async fn deterministic_client_error_does_not_retry() {
        for status in [400u16, 404, 422] {
            let calls = Arc::new(AtomicUsize::new(0));
            let calls_inner = Arc::clone(&calls);

            let result = send_with_retry_status_policy(
                "Test",
                5,
                &mut || {
                    let calls = Arc::clone(&calls_inner);
                    async move {
                        calls.fetch_add(1, AtomicOrdering::SeqCst);
                        Ok(make_response(status, None))
                    }
                },
                FailoverRetryPolicy {
                    rate_limit_cap: None,
                },
                &test_never_cancelled,
            )
            .await;

            assert!(result.is_err());
            // 确定性 4xx 快速失败，只发一次。
            assert_eq!(
                calls.load(AtomicOrdering::SeqCst),
                1,
                "status {status} should not retry"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn immediate_failover_status_does_not_retry_inner() {
        for status in [401u16, 402, 403] {
            let calls = Arc::new(AtomicUsize::new(0));
            let calls_inner = Arc::clone(&calls);

            // 即便给了 rate_limit_cap，401/403 也不重试 —— 立即冒泡换 key。
            let result = send_with_retry_status_policy(
                "Test",
                5,
                &mut || {
                    let calls = Arc::clone(&calls_inner);
                    async move {
                        calls.fetch_add(1, AtomicOrdering::SeqCst);
                        Ok(make_response(status, None))
                    }
                },
                FailoverRetryPolicy {
                    rate_limit_cap: Some(RATE_LIMIT_KEY_SWITCH_THRESHOLD),
                },
                &test_never_cancelled,
            )
            .await;

            let err = result.expect_err("auth error should fail");
            assert!(
                is_failover_error(&err),
                "status {status} should be failover"
            );
            assert_eq!(
                calls.load(AtomicOrdering::SeqCst),
                1,
                "status {status} must not retry inner"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_backs_off_on_same_key_when_no_backup() {
        // 无备用 key：429 在同一 key 上退避重试到**限流专用上限**（耐心重试，不受较小的
        // 通用 attempts 限制），不提前停。
        let attempts = 5;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_inner = Arc::clone(&calls);

        let result = send_with_retry_status_policy(
            "Test",
            attempts,
            &mut || {
                let calls = Arc::clone(&calls_inner);
                async move {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(make_response(429, None))
                }
            },
            FailoverRetryPolicy {
                rate_limit_cap: None,
            },
            &test_never_cancelled,
        )
        .await;

        assert!(result.is_err());
        assert_eq!(calls.load(AtomicOrdering::SeqCst), RATE_LIMIT_MAX_ATTEMPTS);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_bubbles_at_threshold_when_backup_available() {
        // 有备用 key：429 退避到阈值 N 后停止重试并冒泡（让外层换 key）。
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_inner = Arc::clone(&calls);

        let result = send_with_retry_status_policy(
            "Test",
            10, // 总次数远大于阈值，验证是阈值而非总次数封顶
            &mut || {
                let calls = Arc::clone(&calls_inner);
                async move {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(make_response(429, None))
                }
            },
            FailoverRetryPolicy {
                rate_limit_cap: Some(RATE_LIMIT_KEY_SWITCH_THRESHOLD),
            },
            &test_never_cancelled,
        )
        .await;

        let err = result.expect_err("429 at threshold should bubble");
        assert!(is_failover_error(&err));
        // 第 N 次 429 后停止 → 共发 N 次。
        assert_eq!(
            calls.load(AtomicOrdering::SeqCst),
            RATE_LIMIT_KEY_SWITCH_THRESHOLD
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_respects_retry_after_header() {
        // Retry-After 优先：429 带 retry-after，仍退避重试（这里验证不快速失败、能继续）。
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_inner = Arc::clone(&calls);

        let result = send_with_retry_status_policy(
            "Test",
            3,
            &mut || {
                let calls = Arc::clone(&calls_inner);
                async move {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(make_response(429, Some(2)))
                }
            },
            FailoverRetryPolicy {
                rate_limit_cap: None,
            },
            &test_never_cancelled,
        )
        .await;

        assert!(result.is_err());
        // 退避重试到限流专用上限（paused 时钟让 retry-after 的 sleep 瞬时跳过）。
        assert_eq!(calls.load(AtomicOrdering::SeqCst), RATE_LIMIT_MAX_ATTEMPTS);
    }

    #[tokio::test(start_paused = true)]
    async fn retry_backoff_stops_when_cancelled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_inner = Arc::clone(&calls);
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled_check = Arc::clone(&cancelled);

        let mut task = tokio::spawn(async move {
            send_with_retry_status_policy(
                "Test",
                5,
                &mut || {
                    let calls = Arc::clone(&calls_inner);
                    async move {
                        calls.fetch_add(1, AtomicOrdering::SeqCst);
                        Ok(make_response(500, None))
                    }
                },
                FailoverRetryPolicy {
                    rate_limit_cap: None,
                },
                &move || cancelled_check.load(AtomicOrdering::SeqCst),
            )
            .await
        });

        tokio::task::yield_now().await;
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);

        cancelled.store(true, AtomicOrdering::SeqCst);
        tokio::time::advance(Duration::from_millis(250)).await;

        let result = (&mut task).await.expect("retry task should finish");
        assert!(matches!(result, Err(err) if err == "Test cancelled"));
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn failover_switches_key_after_429_threshold_with_backup() {
        // 两把 key：key#0 一直 429 → 达阈值换 key#1（key#1 成功）。
        let state = crate::state::test_app_state();
        let keys = vec!["key0".to_string(), "key1".to_string()];
        let key0_calls = Arc::new(AtomicUsize::new(0));
        let key1_calls = Arc::new(AtomicUsize::new(0));
        let k0 = Arc::clone(&key0_calls);
        let k1 = Arc::clone(&key1_calls);

        let result = send_with_failover(&state, "Test", 5, "prov", &keys, |key| {
            let k0 = Arc::clone(&k0);
            let k1 = Arc::clone(&k1);
            let key = key.to_string();
            async move {
                if key == "key0" {
                    k0.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(make_response(429, None))
                } else {
                    k1.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(make_response(200, None))
                }
            }
        })
        .await;

        assert!(result.is_ok());
        // key#0 退避到阈值 N 次，然后换到 key#1 成功一次。
        assert_eq!(
            key0_calls.load(AtomicOrdering::SeqCst),
            RATE_LIMIT_KEY_SWITCH_THRESHOLD
        );
        assert_eq!(key1_calls.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn failover_switches_immediately_on_auth_error() {
        // key#0 返回 401 → 立即换 key（不重试），key#1 成功。
        let state = crate::state::test_app_state();
        let keys = vec!["key0".to_string(), "key1".to_string()];
        let key0_calls = Arc::new(AtomicUsize::new(0));
        let k0 = Arc::clone(&key0_calls);

        let result = send_with_failover(&state, "Test", 5, "prov", &keys, |key| {
            let k0 = Arc::clone(&k0);
            let key = key.to_string();
            async move {
                if key == "key0" {
                    k0.fetch_add(1, AtomicOrdering::SeqCst);
                    Ok(make_response(401, None))
                } else {
                    Ok(make_response(200, None))
                }
            }
        })
        .await;

        assert!(result.is_ok());
        // 401 不重试，只发一次就换 key。
        assert_eq!(key0_calls.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn failover_with_single_key_429_backs_off_no_switch() {
        // 只有一把 key：429 在该 key 上退避到总次数上限，没有可换的 key。
        let state = crate::state::test_app_state();
        let keys = vec!["only".to_string()];
        let calls = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&calls);

        let result = send_with_failover(&state, "Test", 5, "prov", &keys, |_key| {
            let c = Arc::clone(&c);
            async move {
                c.fetch_add(1, AtomicOrdering::SeqCst);
                Ok(make_response(429, None))
            }
        })
        .await;

        assert!(result.is_err());
        // 无备用 key → 退避到限流专用上限。
        assert_eq!(calls.load(AtomicOrdering::SeqCst), RATE_LIMIT_MAX_ATTEMPTS);
    }
}
