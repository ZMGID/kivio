use super::{antigravity, header_pairs, http, now, resolve_provider, KIMI_BASE};
use crate::{settings::ModelProvider, state::AppState};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    label: String,
    used_percent: Option<f64>,
    used: Option<f64>,
    limit: Option<f64>,
    resets_at: Option<i64>,
    reset_hint: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUsage {
    plan: Option<String>,
    windows: Vec<UsageWindow>,
    fetched_at: i64,
    /// Prepaid account money, for API keys that report a balance instead of plan windows.
    balances: Vec<Balance>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    label: String,
    amount: f64,
    currency: String,
}
fn number(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str()?.parse().ok())
        .filter(|n| n.is_finite() && *n >= 0.0)
}
fn reset(v: &Value, fetched: i64) -> Option<i64> {
    for key in ["reset_at", "resetAt", "reset_time", "resetTime"] {
        if let Some(n) = number(&v[key]) {
            return Some(if n > 1e12 {
                (n / 1000.0) as i64
            } else {
                n as i64
            });
        }
        if let Some(t) = v[key]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        {
            return Some(t.timestamp());
        }
    }
    ["reset_after_seconds", "reset_in", "resetIn", "ttl"]
        .iter()
        .find_map(|key| number(&v[*key]))
        .map(|n| fetched.saturating_add(n as i64))
}
fn kimi_window(v: &Value, label: String, fetched: i64) -> Option<UsageWindow> {
    let limit = number(&v["limit"]);
    let used = number(&v["used"]).or_else(|| Some((limit? - number(&v["remaining"])?).max(0.0)));
    if used.is_none() && limit.is_none() {
        return None;
    }
    Some(UsageWindow {
        label: v["name"]
            .as_str()
            .or(v["title"].as_str())
            .unwrap_or(&label)
            .to_owned(),
        used_percent: used
            .zip(limit)
            .filter(|(_, l)| *l > 0.0)
            .map(|(u, l)| (100.0 * u / l).clamp(0.0, 100.0)),
        used,
        limit,
        resets_at: reset(v, fetched),
        reset_hint: None,
    })
}
fn parse_usage(kind: &str, value: &Value, fetched: i64) -> AccountUsage {
    let mut windows = Vec::new();
    if kind == "antigravity" {
        windows = antigravity_windows(value, fetched);
    } else if kind == "kimi" {
        if let Some(row) = kimi_window(&value["usage"], "Weekly".into(), fetched) {
            windows.push(row);
        }
        if let Some(limits) = value["limits"].as_array() {
            for (i, item) in limits.iter().enumerate() {
                let detail = item.get("detail").filter(|v| v.is_object()).unwrap_or(item);
                let duration = number(&item["window"]["duration"]);
                let unit = item["window"]["timeUnit"].as_str().unwrap_or("");
                let label = item["name"]
                    .as_str()
                    .or(item["title"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| match duration {
                        Some(n) if unit.contains("MINUTE") && n % 60.0 == 0.0 => {
                            format!("{}h", n / 60.0)
                        }
                        Some(n) if unit.contains("MINUTE") => format!("{n}m"),
                        Some(n) if unit.contains("HOUR") => format!("{n}h"),
                        Some(n) if unit.contains("DAY") => format!("{n}d"),
                        _ => format!("Limit {}", i + 1),
                    });
                if let Some(mut row) = kimi_window(detail, label, fetched) {
                    row.resets_at = row.resets_at.or_else(|| reset(item, fetched));
                    windows.push(row);
                }
            }
        }
    } else if kind == "codex" {
        for (group, prefix) in [
            ("rate_limit", ""),
            ("code_review_rate_limit", "Code review · "),
        ] {
            for (key, fallback) in [
                ("primary_window", "Session"),
                ("secondary_window", "Weekly"),
            ] {
                let row = &value[group][key];
                if !row.is_object() {
                    continue;
                }
                let label = match number(&row["limit_window_seconds"]) {
                    Some(seconds) if seconds >= 3600.0 && seconds % 3600.0 == 0.0 => {
                        format!("{}h", seconds / 3600.0)
                    }
                    _ => fallback.into(),
                };
                windows.push(UsageWindow {
                    label: format!("{prefix}{label}"),
                    used_percent: number(&row["used_percent"]).map(|n| n.clamp(0.0, 100.0)),
                    used: None,
                    limit: None,
                    resets_at: reset(row, fetched),
                    reset_hint: None,
                });
            }
        }
    }
    AccountUsage {
        plan: value["plan_type"].as_str().map(str::to_owned),
        windows,
        fetched_at: fetched,
        balances: Vec::new(),
    }
}

fn antigravity_windows(value: &Value, fetched: i64) -> Vec<UsageWindow> {
    let value = value
        .get("response")
        .filter(|v| v.is_object())
        .unwrap_or(value);
    let mut windows = Vec::new();
    let mut add = |bucket: &Value, group: &str, index: usize| {
        let remaining = bucket
            .get("remaining")
            .filter(|v| v.is_object())
            .unwrap_or(bucket);
        let fraction = number(&remaining["remainingFraction"]).filter(|n| *n <= 1.0);
        let name = bucket["displayName"]
            .as_str()
            .or(bucket["modelId"].as_str())
            .or(bucket["bucketId"].as_str())
            .or(bucket["window"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Quota {}", index + 1));
        let label = if group.is_empty() {
            name
        } else {
            format!("{group} · {name}")
        };
        windows.push(UsageWindow {
            label,
            used_percent: fraction.map(|n| (1.0 - n) * 100.0),
            used: None,
            limit: None,
            resets_at: reset(bucket, fetched).or_else(|| reset(remaining, fetched)),
            reset_hint: bucket["description"].as_str().map(str::to_owned),
        });
    };
    if let Some(groups) = value["groups"].as_array() {
        for group in groups {
            if let Some(buckets) = group["buckets"].as_array() {
                for (i, bucket) in buckets.iter().filter(|v| v.is_object()).enumerate() {
                    add(bucket, group["displayName"].as_str().unwrap_or(""), i);
                }
            }
        }
    }
    if let Some(buckets) = value["buckets"].as_array() {
        for (i, bucket) in buckets.iter().filter(|v| v.is_object()).enumerate() {
            add(bucket, "", i);
        }
    }
    windows
}

async fn antigravity_usage(provider: &ModelProvider) -> Result<AccountUsage, String> {
    let client = http()?;
    let token = provider.preferred_api_key().ok_or("OAuth token missing")?;
    let project = provider
        .request
        .oauth
        .as_ref()
        .and_then(|a| a.project_id.as_deref())
        .ok_or("Antigravity project missing")?;
    let body = serde_json::json!({"project": project});
    // Dedicated quota endpoints only: catalog availability can masquerade as 100% remaining.
    if let Ok(value) =
        antigravity::control(&client, token, "retrieveUserQuotaSummary", body.clone()).await
    {
        let result = parse_usage("antigravity", &value, now());
        if result.windows.iter().any(|w| w.used_percent.is_some()) {
            return Ok(result);
        }
    }
    let value = antigravity::control(&client, token, "retrieveUserQuota", body).await?;
    Ok(parse_usage("antigravity", &value, now()))
}

/// GLM Coding Plan quota lives next to the coding endpoint the provider is configured for.
/// Pay-as-you-go GLM endpoints have no plan quota, so only the coding paths qualify.
fn glm_coding_plan_quota_url(base_url: &str) -> Option<&'static str> {
    let url = url::Url::parse(base_url.trim()).ok()?;
    if url.scheme() != "https" || !url.path().starts_with("/api/coding/paas") {
        return None;
    }
    match url.host_str()? {
        "open.bigmodel.cn" => Some("https://open.bigmodel.cn/api/monitor/usage/quota/limit"),
        "api.z.ai" => Some("https://api.z.ai/api/monitor/usage/quota/limit"),
        _ => None,
    }
}

fn glm_window(item: &Value, label: String, fetched: i64) -> UsageWindow {
    let used = number(&item["currentValue"]);
    let limit = number(&item["usage"]).filter(|n| *n > 0.0);
    let used_percent = number(&item["percentage"])
        .or_else(|| Some(100.0 * used? / limit?))
        .map(|n| n.clamp(0.0, 100.0));
    UsageWindow {
        label,
        used_percent,
        used: used.filter(|_| limit.is_some()),
        limit: limit.filter(|_| used.is_some()),
        resets_at: number(&item["nextResetTime"])
            .map(|n| {
                if n > 1e12 {
                    (n / 1000.0) as i64
                } else {
                    n as i64
                }
            })
            .filter(|t| *t > fetched),
        reset_hint: None,
    }
}

/// Model windows (5h, weekly) come first; MCP tool calls are a separate monthly allowance.
fn parse_glm_usage(data: &Value, fetched: i64) -> AccountUsage {
    let mut windows = Vec::new();
    let mut tools = Vec::new();
    for (i, item) in data["limits"].as_array().into_iter().flatten().enumerate() {
        let count = number(&item["number"]).unwrap_or(1.0);
        let span = match (number(&item["unit"]).map(|u| u as i64), count) {
            (Some(3), n) => Some(format!("{n}h")),
            (Some(6), n) if n == 1.0 => Some("Weekly".into()),
            (Some(5), n) if n == 1.0 => Some("Monthly".into()),
            _ => None,
        };
        match item["type"].as_str().unwrap_or("") {
            "TIME_LIMIT" => {
                let label = span.map_or_else(|| "MCP".into(), |s| format!("MCP · {s}"));
                tools.push(glm_window(item, label, fetched));
            }
            "TOKENS_LIMIT" | "CREDIT_LIMIT" => {
                let label = span.unwrap_or_else(|| format!("Limit {}", i + 1));
                windows.push(glm_window(item, label, fetched));
            }
            _ => {}
        }
    }
    windows.extend(tools);
    let plan = ["level", "planName", "plan", "packageName"]
        .iter()
        .find_map(|key| data[*key].as_str().map(str::trim).filter(|s| !s.is_empty()))
        .map(|plan| {
            let mut chars = plan.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        });
    AccountUsage {
        plan,
        windows,
        fetched_at: fetched,
        balances: Vec::new(),
    }
}

/// OpenCode Go keys report their plan meters at one documented endpoint.
/// Both the legacy `zen/go` and current `inference/go` bases qualify; Zen free does not.
fn is_opencode_go(base_url: &str) -> bool {
    url::Url::parse(base_url.trim()).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("opencode.ai")
            && ["/zen/go", "/inference/go"].iter().any(|prefix| {
                url.path() == *prefix || url.path().starts_with(&format!("{prefix}/"))
            })
    })
}

fn parse_opencode_go_usage(value: &Value, fetched: i64) -> AccountUsage {
    let windows = [
        ("rolling", "5h"),
        ("weekly", "Weekly"),
        ("monthly", "Monthly"),
    ]
    .into_iter()
    .filter(|(key, _)| value["usage"][*key].is_object())
    .map(|(key, label)| {
        let row = &value["usage"][key];
        let limited = row["status"].as_str() == Some("rate-limited");
        UsageWindow {
            label: label.into(),
            used_percent: number(&row["percent"])
                .or(limited.then_some(100.0))
                .map(|n| n.clamp(0.0, 100.0)),
            used: None,
            limit: None,
            resets_at: reset(row, fetched),
            reset_hint: None,
        }
    })
    .collect();
    AccountUsage {
        plan: None,
        windows,
        fetched_at: fetched,
        balances: Vec::new(),
    }
}

/// MiniMax Token Plan `/v1/token_plan/remains` reports what remains per bucket.
/// `general` is the model allowance; other buckets (video, image) are metered apart.
/// Status 3 is a window the plan lacks, status 2 is used up. A pay-as-you-go key has no
/// buckets, which yields no windows rather than an error.
fn parse_minimax_usage(value: &Value, fetched: i64) -> AccountUsage {
    let at = |n: f64| {
        if n > 1e12 {
            (n / 1000.0) as i64
        } else {
            n as i64
        }
    };
    let mut windows = Vec::new();
    let mut extras = Vec::new();
    for bucket in value["model_remains"].as_array().into_iter().flatten() {
        let name = bucket["model_name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let general = name.eq_ignore_ascii_case("general");
        for (left, status, start, end, week) in [
            (
                "current_interval_remaining_percent",
                "current_interval_status",
                "start_time",
                "end_time",
                false,
            ),
            (
                "current_weekly_remaining_percent",
                "current_weekly_status",
                "weekly_start_time",
                "weekly_end_time",
                true,
            ),
        ] {
            let status = number(&bucket[status]).map(|n| n as i64);
            let left = number(&bucket[left]);
            if status == Some(3) || (left.is_none() && status != Some(2)) {
                continue;
            }
            let used = if status == Some(2) {
                100.0
            } else {
                100.0 - left.unwrap_or(0.0)
            };
            let (start, end) = (number(&bucket[start]).map(at), number(&bucket[end]).map(at));
            let span = match (week, start.zip(end).map(|(s, e)| e - s).filter(|s| *s > 0)) {
                (true, _) => "Weekly".to_owned(),
                (_, Some(s)) if s % 86_400 == 0 && s > 86_400 => format!("{}d", s / 86_400),
                (_, Some(86_400)) => "Daily".to_owned(),
                (_, Some(s)) if s % 3600 == 0 => format!("{}h", s / 3600),
                _ => "Limit".to_owned(),
            };
            let row = UsageWindow {
                label: if general {
                    span
                } else {
                    let mut chars = name.chars();
                    let title: String = chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect())
                        .unwrap_or_default();
                    format!("{title} · {span}")
                },
                used_percent: Some(used.clamp(0.0, 100.0)),
                used: None,
                limit: None,
                resets_at: end.filter(|t| *t > fetched),
                reset_hint: None,
            };
            if general {
                windows.push(row)
            } else {
                extras.push(row)
            }
        }
    }
    windows.extend(extras);
    AccountUsage {
        plan: None,
        windows,
        fetched_at: fetched,
        balances: Vec::new(),
    }
}

fn balance(label: &str, amount: Option<f64>, currency: &str) -> Option<Balance> {
    Some(Balance {
        label: label.into(),
        amount: amount?,
        currency: currency.into(),
    })
}

/// Balance endpoints differ per vendor; each reader keeps only the spendable total.
fn parse_balance(vendor: Vendor, value: &Value, fetched: i64) -> AccountUsage {
    let balances = match vendor {
        Vendor::DeepSeek => value["balance_infos"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| {
                balance(
                    "Balance",
                    number(&row["total_balance"]),
                    row["currency"].as_str().unwrap_or("CNY"),
                )
            })
            .collect(),
        Vendor::Moonshot(currency) => balance(
            "Balance",
            number(&value["data"]["available_balance"]),
            currency,
        )
        .into_iter()
        .collect(),
        Vendor::SiliconFlow(currency) => {
            balance("Balance", number(&value["data"]["totalBalance"]), currency)
                .into_iter()
                .collect()
        }
        Vendor::OpenRouter => {
            let data = &value["data"];
            match number(&data["limit_remaining"]) {
                Some(left) => balance("Remaining", Some(left), "USD"),
                None => balance("Used", number(&data["usage"]), "USD"),
            }
            .into_iter()
            .collect()
        }
        _ => Vec::new(),
    };
    AccountUsage {
        plan: None,
        windows: Vec::new(),
        fetched_at: fetched,
        balances,
    }
}

/// API-key accounts whose vendor publishes a plan-quota or balance endpoint.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Vendor {
    Glm,
    OpenCodeGo,
    Kimi,
    MiniMax,
    DeepSeek,
    Moonshot(&'static str),
    SiliconFlow(&'static str),
    OpenRouter,
}

/// Keep in step with `apiKeyUsageKind` in `src/settings/providerAccountUsage.ts`.
fn api_key_vendor(base_url: &str) -> Option<(Vendor, String)> {
    if let Some(url) = glm_coding_plan_quota_url(base_url) {
        return Some((Vendor::Glm, url.into()));
    }
    if is_opencode_go(base_url) {
        return Some((
            Vendor::OpenCodeGo,
            "https://opencode.ai/inference/go/v1/usage".into(),
        ));
    }
    let url = url::Url::parse(base_url.trim()).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?;
    let vendor = match host {
        "api.kimi.com" if url.path().starts_with("/coding") => Vendor::Kimi,
        "api.minimaxi.com" | "api.minimax.cn" | "api.minimax.io" => Vendor::MiniMax,
        "api.deepseek.com" => Vendor::DeepSeek,
        "api.moonshot.cn" => Vendor::Moonshot("CNY"),
        "api.moonshot.ai" => Vendor::Moonshot("USD"),
        "api.siliconflow.cn" => Vendor::SiliconFlow("CNY"),
        "api.siliconflow.com" => Vendor::SiliconFlow("USD"),
        "openrouter.ai" => Vendor::OpenRouter,
        _ => return None,
    };
    let endpoint = match vendor {
        Vendor::Kimi => format!("{KIMI_BASE}/usages"),
        Vendor::MiniMax => format!("https://{host}/v1/token_plan/remains"),
        Vendor::DeepSeek => "https://api.deepseek.com/user/balance".into(),
        Vendor::Moonshot(_) => format!("https://{host}/v1/users/me/balance"),
        Vendor::SiliconFlow(_) => format!("https://{host}/v1/user/info"),
        Vendor::OpenRouter => "https://openrouter.ai/api/v1/key".into(),
        Vendor::Glm | Vendor::OpenCodeGo => unreachable!(),
    };
    Some((vendor, endpoint))
}

fn vendor_message(value: &Value) -> Option<&str> {
    [
        &value["error"]["message"],
        &value["base_resp"]["status_msg"],
        &value["msg"],
        &value["message"],
    ]
    .into_iter()
    .find_map(|v| v.as_str().map(str::trim).filter(|s| !s.is_empty()))
}

async fn api_key_usage(provider: &ModelProvider) -> Result<AccountUsage, String> {
    let (vendor, url) = api_key_vendor(&provider.base_url)
        .ok_or("This provider does not expose account usage here")?;
    let key = provider
        .preferred_api_key()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or("API key missing")?;
    // MiniMax pay-as-you-go keys (`sk-api-`) share the plan's base URL but carry no plan windows.
    if vendor == Vendor::MiniMax && key.starts_with("sk-api-") {
        return Err("This provider does not expose account usage here".into());
    }
    let mut request = http()?.get(&url).header("Accept", "application/json");
    request = match vendor {
        // Zhipu's quota endpoint takes the plan key itself, without a Bearer prefix.
        Vendor::Glm => request
            .header("Authorization", key)
            .header("Accept-Language", "en-US,en"),
        Vendor::Kimi => request
            .bearer_auth(key)
            .header("User-Agent", "KimiCLI/1.0.0"),
        _ => request.bearer_auth(key),
    };
    let response = request
        .send()
        .await
        .map_err(|_| "Usage request failed; check your connection or proxy")?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    let fail = |value: &Value| match vendor_message(value) {
        Some(message) => format!("Usage request failed: {message}"),
        None => format!("Usage request failed (HTTP {})", status.as_u16()),
    };
    if !status.is_success() {
        return Err(fail(&value));
    }
    let fetched = now();
    match vendor {
        Vendor::Glm => {
            if value["success"].as_bool() == Some(false) || !value["data"].is_object() {
                return Err(fail(&value));
            }
            Ok(parse_glm_usage(&value["data"], fetched))
        }
        Vendor::OpenCodeGo if value["usage"].is_object() => {
            Ok(parse_opencode_go_usage(&value, fetched))
        }
        Vendor::Kimi if value.is_object() => Ok(parse_usage("kimi", &value, fetched)),
        Vendor::MiniMax => match number(&value["base_resp"]["status_code"]) {
            Some(code) if code == 0.0 => Ok(parse_minimax_usage(&value, fetched)),
            // MiniMax answers 200 with the refusal in base_resp; a key without a plan lands here too.
            Some(_) => Err(fail(&value)),
            None => Err("Invalid usage response".into()),
        },
        Vendor::Moonshot(_) | Vendor::SiliconFlow(_) if value["data"].is_object() => {
            Ok(parse_balance(vendor, &value, fetched))
        }
        Vendor::DeepSeek | Vendor::OpenRouter if value.is_object() => {
            Ok(parse_balance(vendor, &value, fetched))
        }
        _ => Err("Invalid usage response".into()),
    }
}

/// Account quota for OAuth sessions and API-key plans that publish a quota endpoint.
#[tauri::command]
pub async fn provider_oauth_usage(
    state: tauri::State<'_, AppState>,
    provider: ModelProvider,
) -> Result<AccountUsage, String> {
    if provider.request.oauth.is_none() {
        return api_key_usage(&provider).await;
    }
    let kind = provider
        .request
        .oauth
        .as_ref()
        .ok_or("OAuth account required")?
        .provider
        .clone();
    if kind == "antigravity" {
        let resolved = resolve_provider(&state, &provider).await?;
        return antigravity_usage(&resolved).await;
    }
    let url = match kind.as_str() {
        "kimi" => format!("{KIMI_BASE}/usages"),
        "codex" => "https://chatgpt.com/backend-api/wham/usage".into(),
        _ => return Err("This provider does not expose account usage here".into()),
    };
    let resolved = resolve_provider(&state, &provider).await?;
    let client = http()?;
    let mut request = client
        .get(url)
        .bearer_auth(resolved.preferred_api_key().ok_or("OAuth token missing")?)
        .header("Accept", "application/json");
    for (key, value) in header_pairs(&resolved) {
        request = request.header(key, value);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "Usage request failed; check your connection or proxy")?;
    if !response.status().is_success() {
        return Err(format!(
            "Usage request failed (HTTP {})",
            response.status().as_u16()
        ));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "Invalid usage response")?;
    Ok(parse_usage(&kind, &value, now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn codex_usage_preserves_the_plan_and_reset_windows() {
        let usage = parse_usage(
            "codex",
            &json!({"plan_type":"plus","rate_limit":{
                "primary_window":{"used_percent":80,"reset_at":200,"limit_window_seconds":18000},
                "secondary_window":{"used_percent":0,"reset_after_seconds":30}
            }}),
            100,
        );
        assert_eq!(usage.plan.as_deref(), Some("plus"));
        assert_eq!(usage.windows[0].label, "5h");
        assert_eq!(usage.windows[0].used_percent, Some(80.0));
        assert_eq!(usage.windows[0].resets_at, Some(200));
        assert_eq!(usage.windows[1].used_percent, Some(0.0));
        assert_eq!(usage.windows[1].resets_at, Some(130));
    }
    #[test]
    fn antigravity_grouped_and_model_quota() {
        let grouped = parse_usage(
            "antigravity",
            &json!({"response":{"groups":[{"displayName":"Gemini","buckets":[{"displayName":"Weekly","remaining":{"remainingFraction":0.25},"resetTime":"2026-09-06T00:00:00Z"},{"displayName":"Session","remainingFraction":0}]}]}}),
            100,
        );
        assert_eq!(grouped.windows.len(), 2);
        assert_eq!(grouped.windows[0].label, "Gemini · Weekly");
        assert_eq!(grouped.windows[0].used_percent, Some(75.0));
        assert!(grouped.windows[0].resets_at.is_some());
        assert_eq!(grouped.windows[1].used_percent, Some(100.0));
        let model = parse_usage(
            "antigravity",
            &json!({"buckets":[{"modelId":"gemini-3.8-flash","remainingFraction":1,"resetTime":"2026-09-06T00:00:00Z"}]}),
            100,
        );
        assert_eq!(model.windows[0].used_percent, Some(0.0));
    }
    #[test]
    fn antigravity_catalog_and_invalid_fractions_are_not_quota() {
        let catalog = json!({"models":{"gemini-3.8-flash":{"quotaInfo":{"remainingFraction":1}}}});
        assert!(parse_usage("antigravity", &catalog, 100).windows.is_empty());
        let result = parse_usage(
            "antigravity",
            &json!({"buckets":[{"modelId":"missing"},{"modelId":"invalid","remainingFraction":2},{"modelId":"negative","remainingFraction":-1}]}),
            100,
        );
        assert_eq!(result.windows.len(), 3);
        assert!(result.windows.iter().all(|w| w.used_percent.is_none()));
    }
    #[test]
    fn kimi_remaining_and_windows() {
        let value = json!({"usage":{"limit":"100","remaining":"25","resetTime":"2026-09-06T00:00:00Z"},"limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},"detail":{"limit":20,"used":0,"resetIn":60}}]});
        let result = parse_usage("kimi", &value, 100);
        assert_eq!(result.windows[0].used_percent, Some(75.0));
        assert!(result.windows[0].resets_at.unwrap() > 100);
        assert_eq!(result.windows[1].label, "5h");
        assert_eq!(result.windows[1].used_percent, Some(0.0));
        assert_eq!(result.windows[1].resets_at, Some(160));
    }
    #[test]
    fn glm_coding_plan_endpoints_only() {
        assert_eq!(
            glm_coding_plan_quota_url("https://open.bigmodel.cn/api/coding/paas/v4"),
            Some("https://open.bigmodel.cn/api/monitor/usage/quota/limit")
        );
        assert_eq!(
            glm_coding_plan_quota_url(" https://api.z.ai/api/coding/paas/v4/ "),
            Some("https://api.z.ai/api/monitor/usage/quota/limit")
        );
        for other in [
            "https://open.bigmodel.cn/api/paas/v4",
            "http://open.bigmodel.cn/api/coding/paas/v4",
            "https://open.bigmodel.cn.example.com/api/coding/paas/v4",
            "",
        ] {
            assert_eq!(glm_coding_plan_quota_url(other), None, "{other}");
        }
    }
    #[test]
    fn glm_quota_lists_model_windows_before_mcp() {
        let usage = parse_glm_usage(
            &json!({"level":"lite","limits":[
                {"type":"TIME_LIMIT","unit":5,"number":1,"usage":100,"currentValue":4,"remaining":96,"percentage":4,"nextResetTime":1798016731999i64},
                {"type":"TOKENS_LIMIT","unit":3,"number":5,"percentage":0},
                {"type":"CREDIT_LIMIT","unit":6,"number":1,"usage":60000,"currentValue":15000}
            ]}),
            100,
        );
        assert_eq!(usage.plan.as_deref(), Some("Lite"));
        let labels: Vec<_> = usage.windows.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(labels, ["5h", "Weekly", "MCP · Monthly"]);
        assert_eq!(usage.windows[0].used_percent, Some(0.0));
        assert_eq!(usage.windows[0].used, None);
        assert_eq!(usage.windows[1].used_percent, Some(25.0));
        assert_eq!(usage.windows[2].used, Some(4.0));
        assert_eq!(usage.windows[2].limit, Some(100.0));
        assert_eq!(usage.windows[2].resets_at, Some(1798016731));
    }
    #[test]
    fn missing_usage_is_not_zero_or_full() {
        let result = parse_usage("kimi", &json!({"usage":{"limit":100}}), 100);
        assert_eq!(result.windows[0].used_percent, None);
        assert!(parse_usage("kimi", &json!({}), 100).windows.is_empty());
    }
}
