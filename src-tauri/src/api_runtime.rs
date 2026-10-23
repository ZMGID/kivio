//! Shared HTTP retry/failover execution. Protocol policy lives here for both desktops.
use super::transport::{
    format_reqwest_error, is_failover_error, is_permanent_provider_error, is_retryable_error,
    parse_retry_after, retry_delay_ms,
};
use reqwest::StatusCode;
use std::{collections::HashSet, future::Future, time::Duration};
tokio::task_local! {
    pub(crate) static RETRY_PROGRESS: tokio::sync::mpsc::UnboundedSender<crate::chat::protocol::ChatRetryStatusPayload>;
    static RETRY_BUDGET: RetryBudget;
}

/// One model request owns one budget, including HTTP failures, key switches and
/// interrupted streams. Task-local scope carries it through provider adapters;
/// separate requests (and concurrent subagents) never share their counters.
#[derive(Clone)]
pub(crate) struct RetryBudget(std::sync::Arc<parking_lot::Mutex<(usize, usize)>>);

impl RetryBudget {
    pub(crate) fn current_or_new(attempts: usize) -> Self {
        RETRY_BUDGET.try_with(Clone::clone).unwrap_or_else(|_| {
            Self(std::sync::Arc::new(parking_lot::Mutex::new((
                0,
                attempts.max(1) - 1,
            ))))
        })
    }

    pub(crate) fn schedule(
        &self,
        retry_after: Option<u64>,
        reason: &str,
        immediate: bool,
    ) -> Option<(crate::chat::protocol::ChatRetryStatusPayload, u64)> {
        let mut state = self.0.lock();
        if state.0 >= state.1 {
            return None;
        }
        state.0 += 1;
        let delay_ms = if immediate {
            0
        } else {
            retry_delay_ms(state.0, retry_after)
        };
        let retry = crate::chat::protocol::ChatRetryStatusPayload {
            attempt: state.0 as u32,
            max_retries: state.1 as u32,
            retry_at_ms: (chrono::Utc::now().timestamp_millis().max(0) as u64)
                .saturating_add(delay_ms),
            reason: reason.to_string(),
        };
        Some((retry, delay_ms))
    }

    fn attempts(&self) -> (usize, usize) {
        let state = self.0.lock();
        (state.0 + 1, state.1 + 1)
    }
}

pub(crate) async fn with_retry_budget<T>(attempts: usize, future: impl Future<Output = T>) -> T {
    RETRY_BUDGET
        .scope(RetryBudget::current_or_new(attempts), future)
        .await
}

fn report_retry_progress(retry: crate::chat::protocol::ChatRetryStatusPayload) {
    let _ = RETRY_PROGRESS.try_with(|tx| {
        let _ = tx.send(retry);
    });
}

/// A second 429 on the same key hands control back to key failover.
pub(super) const RATE_LIMIT_KEY_SWITCH_THRESHOLD: usize = 2;

pub async fn send_with_failover<F, Fut>(
    runtime: &crate::chat::provider_runtime::ProviderRequestRuntime,
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
        runtime,
        label,
        attempts,
        provider_id,
        api_keys,
        || false,
        send,
    )
    .await
}

pub(super) async fn send_with_failover_cancelable<F, Fut, C>(
    runtime: &crate::chat::provider_runtime::ProviderRequestRuntime,
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
    with_retry_budget(
        attempts,
        send_with_failover_budgeted(
            runtime,
            label,
            attempts,
            provider_id,
            api_keys,
            is_cancelled,
            send,
        ),
    )
    .await
}

async fn send_with_failover_budgeted<F, Fut, C>(
    runtime: &crate::chat::provider_runtime::ProviderRequestRuntime,
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
    if api_keys.iter().all(|key| key.trim().is_empty()) {
        return send_with_retry_status_policy(
            label,
            attempts,
            &mut || send(""),
            FailoverRetryPolicy {
                rate_limit_cap: None,
            },
            &is_cancelled,
        )
        .await;
    }

    let total = api_keys.len();
    let mut tried: HashSet<usize> = api_keys
        .iter()
        .enumerate()
        .filter_map(|(idx, key)| key.trim().is_empty().then_some(idx))
        .collect();
    let mut last_err: Option<String> = None;

    while tried.len() < total {
        if is_cancelled() {
            return Err(format!("{} cancelled", label));
        }
        let idx = match runtime.pick_active_key(provider_id, total, &tried) {
            Some(i) => i,
            None => break,
        };
        tried.insert(idx);
        let key = api_keys[idx].as_str();

        // 是否还有未试过的备用 key —— 决定 429 是否在阈值处提前交回外层换 key。
        let has_backup_key = runtime
            .pick_active_key(provider_id, total, &tried)
            .is_some();
        let rate_limit_cap = if has_backup_key {
            Some(RATE_LIMIT_KEY_SWITCH_THRESHOLD)
        } else {
            None
        };

        let mut attempt_send = || send(key);
        match send_with_retry_status_policy(
            label,
            attempts,
            &mut attempt_send,
            FailoverRetryPolicy { rate_limit_cap },
            &is_cancelled,
        )
        .await
        {
            Ok(resp) => {
                runtime.mark_key_ok(provider_id, idx);
                return Ok(resp);
            }
            Err(err_msg) => {
                if is_failover_error(&err_msg) && tried.len() < total {
                    runtime.mark_key_failed(provider_id, idx);
                    eprintln!(
                        "[failover] {} key #{}/{} failed, switching to next: {}",
                        label,
                        idx + 1,
                        total,
                        err_msg
                    );
                    let budget = RetryBudget::current_or_new(attempts);
                    let Some((retry, _)) = budget.schedule(None, "Switching API key", true) else {
                        return Err(err_msg);
                    };
                    report_retry_progress(retry);
                    last_err = Some(err_msg);
                    continue;
                }
                // 非 failover 错误（或已穷举所有 key）→ 直接返回
                if is_failover_error(&err_msg) {
                    runtime.mark_key_failed(provider_id, idx);
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
///
/// 内层重试的状态分类策略。
#[derive(Clone, Copy)]
pub(super) struct FailoverRetryPolicy {
    /// 429 退避重试的次数上限：Some(N) 表示有备用 key，同一 key 上第 N 次 429 后停止重试
    /// 并冒泡（让外层换 key）；None 表示无备用 key，429 与 5xx 一样退避到总次数上限。
    pub(super) rate_limit_cap: Option<usize>,
}

impl FailoverRetryPolicy {
    /// 判断在第 `rate_limit_attempts` 次 429（含本次）后，是否还应继续在当前 key 上退避重试。
    /// - 有备用 key 且已达阈值 → false（停止重试 → 冒泡换 key）。
    /// - 无备用 key → true（继续退避，受总次数上限约束）。
    pub(super) fn should_retry_rate_limit(&self, rate_limit_attempts: usize) -> bool {
        match self.rate_limit_cap {
            Some(cap) => rate_limit_attempts < cap,
            None => true,
        }
    }
}

pub(super) async fn send_with_retry_status_policy<F, Fut>(
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
    let budget = RetryBudget::current_or_new(attempts);
    let mut rate_limit_attempts = 0;
    loop {
        if is_cancelled() {
            return Err(format!("{label} cancelled"));
        }
        let (err_msg, retry_after, reason, retryable) = match send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }
                let retry_after = parse_retry_after(response.headers());
                let text = response.text().await.unwrap_or_default();
                let err_msg = format!("{label} Error: {status} - {text}");
                let is_rate_limit = status == StatusCode::TOO_MANY_REQUESTS;
                if is_rate_limit {
                    rate_limit_attempts += 1;
                }
                // Gate every status: relays may wrap deterministic upstream errors in 5xx.
                let retryable = !is_permanent_provider_error(&text)
                    && if is_rate_limit {
                        policy.should_retry_rate_limit(rate_limit_attempts)
                    } else {
                        status.is_server_error() || matches!(status.as_u16(), 408)
                    };
                let reason = match status.as_u16() {
                    429 => "Rate limited",
                    529 => "Server overloaded",
                    408 | 504 => "Request timed out",
                    _ => "Server error",
                };
                (err_msg, retry_after, reason, retryable)
            }
            Err(err) => {
                let retryable = is_retryable_error(&err);
                let reason = if err.is_timeout() {
                    "Request timed out"
                } else {
                    "Connection interrupted"
                };
                (
                    format!("{label} Error: {}", format_reqwest_error(&err)),
                    None,
                    reason,
                    retryable,
                )
            }
        };
        if retryable {
            if is_cancelled() {
                return Err(format!("{label} cancelled"));
            }
            if let Some((retry, delay_ms)) = budget.schedule(retry_after, reason, false) {
                eprintln!(
                    "{label} retry {}/{} in {delay_ms}ms: {reason}",
                    retry.attempt, retry.max_retries
                );
                report_retry_progress(retry);
                sleep_with_cancel(label, delay_ms, is_cancelled).await?;
                continue;
            }
        }
        let (attempt, max) = budget.attempts();
        return Err(format!("{err_msg} (attempt {attempt}/{max})"));
    }
}

pub(super) async fn sleep_with_cancel(
    label: &str,
    delay_ms: u64,
    is_cancelled: &(dyn Fn() -> bool + Send + Sync),
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(delay_ms);
    loop {
        if is_cancelled() {
            return Err(format!("{label} cancelled"));
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(250))).await;
    }
}
