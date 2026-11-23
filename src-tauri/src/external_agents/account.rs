//! Native account discovery and explicit Codex usage resets. Never starts a conversation or returns credentials.
#[cfg(unix)]
mod antigravity_status;
mod grok_identity;
mod kimi_binding;
mod persisted;
mod snapshot;
pub(super) use persisted::PersistedQuota;
pub(crate) use snapshot::{AccountSnapshotMode, AccountSnapshots};

use crate::{
    external_agents::{
        spawn::{
            agent_cli_command, agent_probe_command, join_stderr_tail, spawn_stderr_tail,
            AgentChild, CliProcessOutput,
        },
        types::{AccountProbe, AccountProtocol, RuntimeAgentDef},
    },
    proc::NoConsoleWindow,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    path::Path,
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;
use uuid::Uuid;

type AccountOutput = tokio::io::Lines<BufReader<super::diagnostics::AgentStdout>>;

fn explicit_command_env(command: &Command, key: &str) -> Option<Option<OsString>> {
    command.as_std().get_envs().find_map(|(name, value)| {
        let matches = if cfg!(windows) {
            name.to_string_lossy().eq_ignore_ascii_case(key)
        } else {
            name == OsStr::new(key)
        };
        matches.then(|| value.map(OsStr::to_os_string))
    })
}

fn command_env(command: &Command, key: &str) -> Option<OsString> {
    explicit_command_env(command, key)
        .unwrap_or_else(|| std::env::var_os(key))
        .filter(|value| !value.to_string_lossy().trim().is_empty())
}

#[derive(Clone, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexResetTarget {
    account_key: String,
    credit_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexResetCredits {
    available_count: u64,
    next_expires_at: Option<i64>,
    account_key: Option<String>,
    cards: Vec<CodexResetCard>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexResetCard {
    id: String,
    title: Option<String>,
    description: Option<String>,
    reset_type: Option<String>,
    granted_at: Option<i64>,
    expires_at: Option<i64>,
    status: CodexResetCardStatus,
}

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
enum CodexResetCardStatus {
    Available,
    Redeeming,
    Redeemed,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodexResetOutcome {
    Reset,
    NothingToReset,
    NoCredit,
    AlreadyRedeemed,
}

pub(super) struct ProbeReading {
    snapshot: Value,
    definitively_unavailable: bool,
}

pub(super) fn unavailable_snapshot(agent: &str, reason: &str) -> Value {
    json!({"agentId":agent,"account":null,"windows":[],"credits":null,
        "unavailable":reason,"checkedAt":chrono::Utc::now().timestamp()})
}

fn reading(
    agent: &str,
    protocol: AccountProtocol,
    account: &Value,
    usage: &Value,
    error: Option<String>,
) -> ProbeReading {
    let definitively_unavailable = match protocol {
        AccountProtocol::CodexAppServer => account.is_null() || account["type"] == "apiKey",
        AccountProtocol::ClaudeControl => usage["rate_limits_available"] == false,
        AccountProtocol::KimiWeb => {
            let data = &usage["data"];
            data["status"] == 401
                || data["message"].as_str().is_some_and(|message| {
                    message.starts_with("No token for ")
                        && message.ends_with("Run /login to authenticate.")
                })
        }
        _ => false,
    };
    ProbeReading {
        snapshot: normalize(agent, protocol, account, usage, error),
        definitively_unavailable,
    }
}

async fn exchange(
    input: &mut super::diagnostics::AgentStdin,
    output: &mut AccountOutput,
    request: Value,
    claude: bool,
) -> Result<Value, String> {
    let id = if claude {
        &request["request_id"]
    } else {
        &request["id"]
    };
    let mut line = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    line.push(b'\n');
    input.write_all(&line).await.map_err(|e| e.to_string())?;
    input.flush().await.map_err(|e| e.to_string())?;
    while let Some(line) = output.next_line().await.map_err(|e| e.to_string())? {
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if claude && frame["type"] == "control_response" && frame["response"]["request_id"] == *id {
            if frame["response"]["subtype"] != "success" {
                return Err("CLI 不支持此账号查询".into());
            }
            return Ok(frame["response"]["response"].clone());
        }
        if !claude && frame.get("method").is_none() && frame.get("id") == Some(id) {
            if frame.get("error").is_some() {
                if request["method"] == "account/rateLimitResetCredit/consume" {
                    return Err(if frame["error"]["code"] == -32601 {
                        "当前 Codex CLI 不支持使用额度重置卡"
                    } else {
                        "Codex 未能确认额度重置结果"
                    }
                    .into());
                }
                return Err("CLI 暂未提供账号或额度信息".into());
            }
            return Ok(frame["result"].clone());
        }
        // Discovery has no approval host. A reverse request must never wait indefinitely.
        let rejection = if frame["type"] == "control_request" {
            Some(
                json!({"type":"control_response","response":{"subtype":"error","request_id":frame["request_id"],"error":"Read-only account probe"}}),
            )
        } else if frame.get("method").is_some() && frame.get("id").is_some() {
            Some(
                json!({"id":frame["id"],"error":{"code":-32601,"message":"Read-only account probe"}}),
            )
        } else {
            None
        };
        if let Some(rejection) = rejection {
            input
                .write_all(format!("{rejection}\n").as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            input.flush().await.map_err(|e| e.to_string())?;
        }
    }
    Err("CLI 在账号查询期间退出".into())
}

pub(super) async fn probe(
    def: &RuntimeAgentDef,
    probe: AccountProbe,
    bin: &Path,
    cwd: &Path,
) -> Result<ProbeReading, String> {
    let protocol = probe.protocol;
    match protocol {
        AccountProtocol::KimiWeb => {
            let mut command = agent_probe_command(def, bin);
            command.args(probe.args).current_dir(cwd);
            kimi_account(command, def.id).await
        }
        AccountProtocol::AntigravityUsage => {
            let mut command = agent_probe_command(def, bin);
            command.args(probe.args).current_dir(cwd);
            let (usage, unavailable) = match antigravity_usage(command).await {
                Ok(usage) => (usage, None),
                Err(error) => (Value::Null, Some(error)),
            };
            #[cfg(unix)]
            let account =
                if antigravity_quota_groups(&usage).is_some_and(|groups| !groups.is_empty()) {
                    let mut identity_command = agent_probe_command(def, bin);
                    identity_command.current_dir(cwd);
                    antigravity_status::identity(identity_command)
                        .await
                        .unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
            #[cfg(not(unix))]
            let account = Value::Null;
            Ok(reading(def.id, protocol, &account, &usage, unavailable))
        }
        AccountProtocol::ClaudeControl
        | AccountProtocol::CodexAppServer
        | AccountProtocol::GrokBilling => probe_stdio(def, probe, bin, cwd).await,
    }
}

async fn antigravity_usage(mut command: Command) -> Result<Value, String> {
    let output = timeout(Duration::from_secs(25), command.group_output())
        .await
        .map_err(|_| "Antigravity 额度查询超时")?
        .map_err(|_| "Antigravity 启动失败")?;
    if !output.status.success() {
        return Err("Antigravity 额度查询失败，请确认本地 CLI 已登录".into());
    }
    serde_json::from_slice(&output.stdout).map_err(|_| "Antigravity 未返回结构化额度报告".into())
}

fn antigravity_quota_groups(usage: &Value) -> Option<&[Value]> {
    let command = &usage["command"];
    if usage["status"] != "SUCCESS" || usage["num_turns"] != 0 || command["name"] != "usage" {
        return None;
    }
    command["data"]["groups"].as_array().map(Vec::as_slice)
}

fn account_plan<'a>(
    protocol: AccountProtocol,
    account: &'a Value,
    usage: &'a Value,
) -> Option<&'a str> {
    let saved = account["planType"]
        .as_str()
        .or_else(|| account["subscriptionType"].as_str());
    if protocol == AccountProtocol::GrokBilling {
        return usage["subscription_tier"].as_str().or(saved);
    }
    if protocol == AccountProtocol::ClaudeControl {
        return saved.or_else(|| usage["subscription_type"].as_str());
    }
    if protocol != AccountProtocol::CodexAppServer || account["type"] != "chatgpt" {
        return saved;
    }
    let reported = [
        &usage["rateLimitsByLimitId"]["codex"]["planType"],
        &usage["rateLimits"]["planType"],
    ]
    .into_iter()
    .find_map(|value| {
        value
            .as_str()
            .filter(|plan| !plan.is_empty() && *plan != "unknown")
    });
    match reported.or(saved)? {
        "pro" => Some("Pro 20×"),
        "prolite" => Some("Pro 100"),
        "promax" => Some("Pro 500"),
        "plus" => Some("Plus"),
        "free" => Some("Free"),
        "go" => Some("Go"),
        plan => Some(plan),
    }
}

async fn kimi_account(mut command: Command, agent: &str) -> Result<ProbeReading, String> {
    // The CLI owns OAuth and token refresh. This client only talks to the
    // loopback server started for this probe, including on Kimi Code 2.6.2.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法连接本地 Kimi Code 服务")?;
    let mut child = AgentChild::spawn(command.stdout(Stdio::piped()).stderr(Stdio::null()))
        .map_err(|_| "Kimi Code 启动失败")?;
    let stdout = child.stdout.take().ok_or("无法读取 Kimi Code 启动状态")?;
    let mut lines = BufReader::new(stdout).lines();
    let (port, token) = timeout(Duration::from_secs(15), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(ready) = kimi_web_ready(&line) {
                return Ok(ready);
            }
        }
        Err("Kimi Code 未启动本地额度服务")
    })
    .await
    .map_err(|_| "Kimi Code 本地额度服务启动超时")??;
    let api = format!("http://127.0.0.1:{port}/api/v1");
    let get = |endpoint: &str| {
        client
            .get(format!("{api}/oauth/{endpoint}"))
            .bearer_auth(&token)
            .timeout(Duration::from_secs(8))
    };
    let (usage, unavailable) = match kimi_web_get(get("usage")).await {
        Ok(usage) => (usage, None),
        Err(_) => (Value::Null, Some("Kimi Code 额度接口暂不可用".into())),
    };
    let profile = kimi_web_get(get("userinfo")).await.unwrap_or(Value::Null);
    let account = kimi_identity(&profile);

    // Let the CLI close its own Web server and unregister it. AgentChild
    // terminates the process tree if an older or unresponsive CLI stays alive.
    let _ = client
        .post(format!("{api}/shutdown"))
        .bearer_auth(&token)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
    Ok(reading(
        agent,
        AccountProtocol::KimiWeb,
        &account,
        &usage,
        unavailable,
    ))
}

async fn kimi_web_get(request: reqwest::RequestBuilder) -> Result<Value, reqwest::Error> {
    request.send().await?.error_for_status()?.json().await
}

fn display_text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn kimi_identity(profile: &Value) -> Value {
    let data = &profile["data"];
    if profile["code"] != 0 || data["kind"] != "ok" {
        return Value::Null;
    }
    let user = &data["userInfo"];
    let name = display_text(&user["username"]).or_else(|| display_text(&user["nickname"]));
    json!({"type":"kimiCode","email":display_text(&user["email"]),"name":name,
        "planType":display_text(&user["userLevelName"])})
}

fn kimi_web_ready(line: &str) -> Option<(u16, String)> {
    let url = line.trim().strip_prefix("Kimi server: http://127.0.0.1:")?;
    let (port, token) = url.split_once("/#token=")?;
    let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
    if token.is_empty()
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    Some((port, token.to_owned()))
}

async fn with_account_stdio<T, F>(
    def: &RuntimeAgentDef,
    probe: AccountProbe,
    bin: &Path,
    cwd: &Path,
    run: impl FnOnce(super::diagnostics::AgentStdin, AccountOutput) -> F,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    let mut command = agent_cli_command(def, bin);
    command.args(probe.args);
    if probe.protocol == AccountProtocol::ClaudeControl {
        command.args(super::provider_profile::claude_overlay_args());
    }
    let mut child = AgentChild::spawn(
        command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .no_console_window()
            .kill_on_drop(true),
    )
    .map_err(|e| e.to_string())?;
    let tail = spawn_stderr_tail(child.stderr.take());
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        let input = child.stdin.take().ok_or("CLI stdin unavailable")?;
        let output = BufReader::new(child.stdout.take().ok_or("CLI stdout unavailable")?).lines();
        run(input, output).await
    })
    .await
    .unwrap_or_else(|_| Err("CLI 账号操作超时".into()));
    let _ = join_stderr_tail(&mut child, tail).await;
    let _ = child.wait().await;
    result
}

async fn initialize_codex_account(
    input: &mut super::diagnostics::AgentStdin,
    output: &mut AccountOutput,
) -> Result<Value, String> {
    exchange(input, output, json!({
        "id":1,"method":"initialize","params":{"clientInfo":{"name":"kivio","version":env!("CARGO_PKG_VERSION")}}
    }), false).await?;
    input
        .write_all(b"{\"method\":\"initialized\",\"params\":{}}\n")
        .await
        .map_err(|error| error.to_string())?;
    Ok(exchange(
        input,
        output,
        json!({
            "id":2,"method":"account/read","params":{"refreshToken":false}
        }),
        false,
    )
    .await?["account"]
        .take())
}

fn codex_rate_limits_request() -> Value {
    json!({"id":3,"method":"account/rateLimits/read","params":{}})
}

async fn probe_stdio(
    def: &RuntimeAgentDef,
    probe: AccountProbe,
    bin: &Path,
    cwd: &Path,
) -> Result<ProbeReading, String> {
    let protocol = probe.protocol;
    let claude = protocol == AccountProtocol::ClaudeControl;
    let grok = protocol == AccountProtocol::GrokBilling;
    with_account_stdio(def, probe, bin, cwd, |mut input, mut output| async move {
        let account = if protocol == AccountProtocol::CodexAppServer {
            initialize_codex_account(&mut input, &mut output).await?
        } else {
        let init = if claude {
            json!({"type":"control_request","request_id":"account-init","request":{"subtype":"initialize"}})
        } else {
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}})
        };
        let init = exchange(&mut input, &mut output, init, claude).await?;
        if claude { init["account"].clone() } else { Value::Null }
        };
        let request = if claude {
            json!({"type":"control_request","request_id":"account-usage","request":{"subtype":"get_usage","skip_behaviors":true}})
        } else if grok { json!({"jsonrpc":"2.0","id":2,"method":"_x.ai/billing","params":{}}) } else { codex_rate_limits_request() };
        // Optional usage has its own budget; losing it must not discard a valid identity.
        let usage = tokio::time::timeout(Duration::from_secs(8), exchange(&mut input, &mut output, request, claude)).await;
        let (usage, unavailable) = match usage {
            Ok(Ok(usage)) => (usage, None),
            Ok(Err(error)) => (Value::Null, Some(error)),
            Err(_) => (Value::Null, Some("额度查询超时，可稍后刷新".to_string())),
        };
        let account = if grok {
            let email = grok_identity::email(&agent_cli_command(def, bin), bin, cwd);
            json!({"type":email.as_ref().map(|_| "grok"),"email":email})
        } else {
            account
        };
        Ok(reading(def.id, protocol, &account, &usage, unavailable))
    }).await
}

fn codex_reset_credits(account: &Value, usage: &Value) -> Option<CodexResetCredits> {
    let summary = &usage["rateLimitResetCredits"];
    let available_count = summary["availableCount"].as_u64()?;
    let mut cards: Vec<CodexResetCard> = summary["credits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|credit| CodexResetCard::deserialize(credit).ok())
        .filter(|card| !card.id.is_empty())
        .collect();
    cards.sort_by_key(|card| {
        (
            card.status != CodexResetCardStatus::Available,
            card.expires_at.unwrap_or(i64::MAX),
        )
    });
    let next_expires_at = cards
        .iter()
        .filter(|card| card.status == CodexResetCardStatus::Available)
        .filter_map(|card| card.expires_at.filter(|expiry| *expiry > 0))
        .min();
    let account_key = match (
        account["type"].as_str(),
        account["email"].as_str(),
        usage["accountId"].as_str(),
    ) {
        (Some("chatgpt"), Some(email), Some(account_id))
            if !email.is_empty() && !account_id.is_empty() =>
        {
            let mut hash = Sha256::new();
            hash.update(account_id.as_bytes());
            hash.update([0]);
            hash.update(email.to_ascii_lowercase());
            Some(format!("{:x}", hash.finalize()))
        }
        _ => None,
    };
    Some(CodexResetCredits {
        available_count,
        next_expires_at,
        account_key,
        cards,
    })
}

impl AccountSnapshots {
    pub(super) async fn redeem_codex_reset_credit(
        &self,
        def: &RuntimeAgentDef,
        probe: AccountProbe,
        bin: &Path,
        cwd: &Path,
        expected: CodexResetTarget,
        expected_count: u64,
    ) -> Result<CodexResetOutcome, String> {
        let mut pending = self
            .pending_codex_resets
            .try_lock()
            .map_err(|_| "已有额度重置正在处理")?;
        let launch = super::overrides::LaunchIdentity::capture(def, bin);
        let _resource = super::session::live::ResourceLease::pending(def.id)?;
        with_account_stdio(def, probe, bin, cwd, |mut input, mut output| async move {
            let account = initialize_codex_account(&mut input, &mut output).await?;
            let usage = timeout(
                Duration::from_secs(8),
                exchange(&mut input, &mut output, codex_rate_limits_request(), false),
            )
            .await
            .map_err(|_| "Codex 额度查询超时")??;
            let credits =
                codex_reset_credits(&account, &usage).ok_or("无法读取当前 Codex 额度重置卡")?;
            let current = credits.account_key.ok_or("无法确认当前 Codex 登录账号")?;
            if current != expected.account_key {
                return Err("Codex 登录账号已变更，请刷新后再试".into());
            }
            launch.ensure_current(def, bin)?;
            if !pending.contains_key(&expected) {
                if credits.available_count == 0 {
                    return Ok(CodexResetOutcome::NoCredit);
                }
                let still_available = match &expected.credit_id {
                    Some(id) => credits.cards.iter().any(|card| {
                        card.id == *id
                            && card.status == CodexResetCardStatus::Available
                            && card
                                .expires_at
                                .is_none_or(|at| at > chrono::Utc::now().timestamp())
                    }),
                    None => credits.available_count == expected_count,
                };
                if !still_available {
                    return Err("Codex 额度重置卡已变更，请刷新后再试".into());
                }
            }
            let key = pending.entry(expected.clone()).or_insert_with(Uuid::new_v4);
            let mut params = json!({"idempotencyKey":*key});
            if let Some(id) = &expected.credit_id {
                params["creditId"] = json!(id);
            }
            let result = timeout(
                Duration::from_secs(12),
                exchange(
                    &mut input,
                    &mut output,
                    json!({
                        "id":4,"method":"account/rateLimitResetCredit/consume","params":params
                    }),
                    false,
                ),
            )
            .await
            .map_err(|_| "Codex 未能确认额度重置结果，可重试同一次操作")??;
            let outcome = serde_json::from_value(result["outcome"].clone())
                .map_err(|_| "Codex 未能确认额度重置结果")?;
            pending.remove(&expected);
            Ok(outcome)
        })
        .await
    }
}

fn grok_usage_window(usage: &Value) -> Option<(&'static str, Value)> {
    let config = &usage["config"];
    let period = &config["currentPeriod"];
    let name = match period["type"].as_str() {
        Some("USAGE_PERIOD_TYPE_WEEKLY") => "Weekly",
        Some("USAGE_PERIOD_TYPE_MONTHLY") => "Monthly",
        _ => "Subscription",
    };
    let timestamp = |value: &Value| chrono::DateTime::parse_from_rfc3339(value.as_str()?).ok();
    let start = timestamp(&period["start"]);
    let end = timestamp(&period["end"]);
    let complete_period = start.zip(end).is_some_and(|(start, end)| start < end);
    // Grok omits a zero percentage while still reporting the subscription period.
    let used = config["creditUsagePercent"].as_f64().or_else(|| {
        (config["creditUsagePercent"].is_null() && name != "Subscription" && complete_period)
            .then_some(0.0)
    })?;
    let start = start.or_else(|| timestamp(&config["billingPeriodStart"]));
    let end = end.or_else(|| timestamp(&config["billingPeriodEnd"]));
    let duration = start.zip(end).and_then(|(start, end)| {
        let minutes = ((end - start).num_milliseconds() as f64 / 60_000.0).round() as i64;
        (minutes > 0).then_some(minutes)
    });
    Some((
        name,
        json!({
            "usedPercent": used,
            "resetsAt": end.map(|time| time.to_utc().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
            "windowDurationMins": duration,
        }),
    ))
}

fn normalize(
    agent: &str,
    protocol: AccountProtocol,
    account: &Value,
    usage: &Value,
    mut unavailable: Option<String>,
) -> Value {
    let mut windows = Vec::new();
    let mut credits = Value::Null;
    let mut reset_credits = Value::Null;
    let mut add_window = |name: String, window: &Value, claude: bool| {
        let used = window[if claude { "utilization" } else { "usedPercent" }].as_f64();
        if let Some(used) = used.filter(|value| value.is_finite()) {
            let reset = &window[if claude { "resets_at" } else { "resetsAt" }];
            windows.push(json!({"name":name,"usedPercent":used.clamp(0.0,100.0),
                "resetsAt": if reset.is_string() || reset.is_number() { reset.clone() } else { Value::Null },
                "windowDurationMins":window["windowDurationMins"].as_u64()}));
        }
    };
    if protocol == AccountProtocol::KimiWeb {
        let data = &usage["data"];
        if usage["code"] == 0 && data["kind"] == "ok" {
            let usages = &data["quota"]["usages"];
            for (key, name, duration) in [
                ("limit5h", "5-hour limit", Some(300)),
                ("limit7d", "Weekly limit", Some(10080)),
                ("monthTotal", "Monthly limit", None),
            ] {
                let entry = &usages[key];
                add_window(
                    name.into(),
                    &json!({"usedPercent": entry["usedRatio"].as_f64().map(|ratio| ratio * 100.0),
                            "resetsAt": entry["resetAt"], "windowDurationMins": duration}),
                    false,
                );
            }
        } else if unavailable.is_none() {
            unavailable = Some("Kimi Code 未返回订阅额度，请确认本地 CLI 已登录".into());
        }
    } else if protocol == AccountProtocol::AntigravityUsage {
        if let Some(groups) = antigravity_quota_groups(usage) {
            for group in groups {
                let Some(group_name) = group["name"].as_str() else {
                    continue;
                };
                let Some(buckets) = group["buckets"].as_array() else {
                    continue;
                };
                for bucket in buckets {
                    let (name, duration) = match bucket["window"].as_str() {
                        Some("5h") => (Some("5h"), Some(300)),
                        Some("weekly") => (Some("7d"), Some(10080)),
                        _ => (bucket["name"].as_str(), None),
                    };
                    let Some(name) = name else {
                        continue;
                    };
                    let remaining = bucket["remaining_fraction"]
                        .as_f64()
                        .filter(|value| (0.0..=1.0).contains(value));
                    add_window(
                        format!("{group_name} · {name}"),
                        &json!({"usedPercent": remaining.map(|ratio| (1.0 - ratio) * 100.0),
                                    "resetsAt": bucket["reset_time"], "windowDurationMins": duration}),
                        false,
                    );
                }
            }
        }
    } else if protocol == AccountProtocol::GrokBilling {
        if let Some((name, window)) = grok_usage_window(usage) {
            add_window(name.into(), &window, false);
        }
    } else if protocol == AccountProtocol::ClaudeControl {
        if let Some(limits) = usage["rate_limits"].as_object() {
            for (name, window) in limits {
                if name != "extra_usage" {
                    add_window(name.clone(), window, true);
                }
            }
            if let Some(extra) = limits
                .get("extra_usage")
                .filter(|extra| extra["is_enabled"] == true)
            {
                credits = json!({"label":"额外用量","used":extra["used_credits"].as_f64(),"limit":extra["monthly_limit"].as_f64(),"currency":extra["currency"].as_str()});
            }
        }
    } else {
        let mut collect = |name: &str, limits: &Value| {
            for kind in ["primary", "secondary"] {
                add_window(format!("{name} {kind}"), &limits[kind], false);
            }
            if let Some(value) = limits.get("credits").filter(|value| value.is_object()) {
                credits = json!({"label":"Credits","unlimited":value["unlimited"].as_bool(),"balance":value["balance"].as_str()});
            }
        };
        if let Some(limits) = usage["rateLimitsByLimitId"]
            .as_object()
            .filter(|limits| !limits.is_empty())
        {
            for (id, limits) in limits {
                collect(limits["limitName"].as_str().unwrap_or(id), limits);
            }
        } else {
            collect("Codex", &usage["rateLimits"]);
        }
        reset_credits = json!(codex_reset_credits(account, usage));
    }
    if windows.is_empty() && reset_credits.is_null() && unavailable.is_none() {
        unavailable = Some("当前账号或 CLI 未提供额度窗口".into());
    }
    let plan = account_plan(protocol, account, usage);
    json!({"agentId":agent,"account": if account.is_object() || plan.is_some() { json!({
        "type":account["type"].as_str().or_else(|| account["apiProvider"].as_str()),
        "email":display_text(&account["email"]),"name":display_text(&account["name"]),"plan":plan
    }) } else { Value::Null }, "windows":windows,"credits":credits,"resetCredits":reset_credits,
    "unavailable":unavailable,
    "checkedAt":chrono::Utc::now().timestamp()})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_keeps_zero_usage_for_complete_periods_and_hides_incomplete_reports() {
        let billing = |period| json!({"config": {"currentPeriod": period}});
        let report = |payload: Value| {
            normalize(
                "grok",
                AccountProtocol::GrokBilling,
                &Value::Null,
                &payload,
                None,
            )
        };
        for (kind, name, end, days) in [
            (
                "USAGE_PERIOD_TYPE_WEEKLY",
                "Weekly",
                "2026-02-08T00:00:00Z",
                7,
            ),
            (
                "USAGE_PERIOD_TYPE_MONTHLY",
                "Monthly",
                "2026-03-01T00:00:00Z",
                28,
            ),
        ] {
            let snapshot = report(billing(json!({
                "type": kind, "start": "2026-02-01T00:00:00Z", "end": end,
            })));
            let windows = snapshot["windows"].as_array().unwrap();
            assert_eq!(windows.len(), 1);
            assert_eq!(windows[0]["name"], name);
            assert_eq!(windows[0]["usedPercent"], 0.0);
            assert_eq!(windows[0]["windowDurationMins"], days * 24 * 60);
            assert_eq!(windows[0]["resetsAt"], end.replace('Z', ".000Z"));
        }
        for period in [
            Value::Null,
            json!({"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-02-08T00:00:00Z"}),
            json!({"type":"USAGE_PERIOD_TYPE_WEEKLY","start":"2026-02-30T00:00:00Z","end":"2026-03-08T00:00:00Z"}),
        ] {
            assert!(report(billing(period))["windows"]
                .as_array()
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn antigravity_preserves_grouped_windows_and_zero_remaining() {
        let report = json!({
            "status": "SUCCESS",
            "num_turns": 0,
            "command": {"name": "usage", "data": {"groups": [
                {"name": "Gemini Models", "buckets": [
                    {"name": "Five Hour Limit Remaining", "window": "5h", "remaining_fraction": 0.875,
                     "reset_time": "2026-11-21T09:00:00Z"},
                    {"name": "Missing quota"},
                ]},
                {"name": "Gemini 4 Argon", "buckets": [
                    {"name": "Gemini 4 Argon Weekly Limit", "window": "weekly", "remaining_fraction": 0},
                ]},
            ]}}
        });
        let status = normalize(
            "antigravity",
            AccountProtocol::AntigravityUsage,
            &Value::Null,
            &report,
            None,
        );
        let windows = status["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0]["name"], "Gemini Models · 5h");
        assert_eq!(windows[0]["usedPercent"], 12.5);
        assert_eq!(windows[0]["resetsAt"], "2026-11-21T09:00:00Z");
        assert_eq!(windows[1]["name"], "Gemini 4 Argon · 7d");
        assert_eq!(windows[1]["usedPercent"], 100.0);
    }

    #[test]
    fn kimi_reports_subscription_windows_and_omits_monthly_breakdown() {
        let payload = json!({
            "code": 0,
            "data": {"kind": "ok", "quota": {"usages": {
                "limit5h": {"usedRatio": 0},
                "limit7d": {"usedRatio": 1, "resetAt": "2026-11-28T00:00:00Z"},
                "monthTotal": {"usedRatio": 0.25},
                "monthCode": {"usedRatio": 0.125}
            }}}
        });
        let status = normalize(
            "kimi",
            AccountProtocol::KimiWeb,
            &kimi_identity(&json!({"code":0,"data":{"kind":"ok","userInfo":{
                "nickname":"Test account","userLevelName":"Andante","phone":{"number":"PRIVATE_PHONE"}
            }}})),
            &payload,
            None,
        );
        let windows = status["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0]["usedPercent"], 0.0);
        assert_eq!(windows[1]["usedPercent"], 100.0);
        assert_eq!(windows[1]["resetsAt"], "2026-11-28T00:00:00Z");
        assert_eq!(windows[2]["usedPercent"], 25.0);
        assert_eq!(status["account"]["name"], "Test account");
        assert_eq!(status["account"]["plan"], "Andante");
        assert!(!status.to_string().contains("PRIVATE_PHONE"));

        let unavailable = normalize(
            "kimi",
            AccountProtocol::KimiWeb,
            &Value::Null,
            &json!({"code": 0, "data": {"kind": "error", "message": "provider diagnostic"}}),
            None,
        );
        assert!(unavailable["windows"].as_array().unwrap().is_empty());
        assert!(unavailable["unavailable"].is_string());
        assert!(!unavailable.to_string().contains("provider diagnostic"));
    }

    #[test]
    fn kimi_accepts_only_authenticated_loopback_startup_urls() {
        assert_eq!(
            kimi_web_ready("Kimi server: http://127.0.0.1:12345/#token=abc-123_"),
            Some((12345, "abc-123_".into()))
        );
        for line in [
            "Kimi server: http://app.example.test:12345/#token=abc",
            "Kimi server: http://127.0.0.1:12345@elsewhere.test/#token=abc",
            "Kimi server: http://127.0.0.1:0/#token=abc",
            "Kimi server: http://127.0.0.1:12345/",
        ] {
            assert!(kimi_web_ready(line).is_none());
        }
    }

    #[test]
    fn native_account_payloads_keep_all_windows_without_exposing_credentials() {
        let account = json!({"type":"chatgpt","email":"a@example.test","planType":"pro","accessToken":"SECRET"});
        let status = normalize(
            "codex",
            AccountProtocol::CodexAppServer,
            &account,
            &json!({"rateLimits":{"primary":{"usedPercent":99}},
            "rateLimitsByLimitId":{
                "codex":{"primary":{"usedPercent":25,"windowDurationMins":300,"resetsAt":123},"credits":{"balance":"42","unlimited":false}},
                "review":{"secondary":{"usedPercent":100,"resetsAt":456},"credits":null}
            }}),
            None,
        );
        assert_eq!(status["windows"].as_array().unwrap().len(), 2);
        assert_eq!(status["windows"][0]["usedPercent"], 25.0);
        assert_eq!(status["credits"]["balance"], "42");
        assert_eq!(status["account"]["plan"], "Pro 20×");
        assert!(!status.to_string().contains("SECRET"));
        assert!(status["unavailable"].is_null());
        let status = normalize(
            "claude",
            AccountProtocol::ClaudeControl,
            &json!({"email":"claude@example.test"}),
            &json!({
                "subscription_type":"max",
                "rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":80,"resets_at":"2026-10-23T00:00:00Z"},
                    "seven_day":{"utilization":0,"resets_at":null}, "extra_usage":{"is_enabled":true,"used_credits":7,"monthly_limit":100}}
            }),
            None,
        );
        assert_eq!(status["windows"].as_array().unwrap().len(), 2);
        assert_eq!(status["credits"]["used"], 7.0);
        assert_eq!(status["account"]["email"], "claude@example.test");
        assert_eq!(status["account"]["plan"], "max");
        let grok = normalize(
            "grok",
            AccountProtocol::GrokBilling,
            &json!({"email":"grok@example.test"}),
            &json!({"config":{"creditUsagePercent":12.5,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY"},
                "billingPeriodStart":"2026-11-01T00:00:00+00:00","billingPeriodEnd":"2026-11-08T00:00:00+00:00"},
                "subscription_tier":"SuperGrok Plus"}),
            None,
        );
        assert_eq!(grok["windows"][0]["usedPercent"], 12.5);
        assert_eq!(grok["windows"][0]["windowDurationMins"], 7 * 24 * 60);
        assert_eq!(grok["account"]["plan"], "SuperGrok Plus");
        assert_eq!(grok["account"]["email"], "grok@example.test");
        let unavailable = normalize(
            "claude",
            AccountProtocol::ClaudeControl,
            &Value::Null,
            &Value::Null,
            None,
        );
        assert!(unavailable["windows"].as_array().unwrap().is_empty());
        assert!(unavailable["unavailable"].is_string());
    }

    #[test]
    fn codex_plan_prefers_the_current_codex_quota_over_the_login_token() {
        let snapshot = normalize(
            "codex",
            AccountProtocol::CodexAppServer,
            &json!({"type":"chatgpt","planType":"pro"}),
            &json!({"rateLimits":{"planType":"pro"},"rateLimitsByLimitId":{"codex":{"planType":"prolite"},"review":{"planType":"promax"}}}),
            None,
        );
        assert_eq!(snapshot["account"]["plan"], "Pro 100");
    }

    #[test]
    fn codex_reset_cards_are_read_independently_of_quota_windows() {
        let status = normalize(
            "codex",
            AccountProtocol::CodexAppServer,
            &json!({"type":"chatgpt"}),
            &json!({
                "rateLimits": {"credits": {"balance": "42", "unlimited": false}},
                "rateLimitResetCredits": {
                    "availableCount": 4,
                    "credits": [
                        {"status":"redeemed","expiresAt":100,"id":"used"},
                        {"status":"available","expiresAt":300,"id":"later"},
                        {"status":"available","expiresAt":200,"grantedAt":100,"id":"first",
                         "title":"Weekly reset","description":"Clears eligible windows","resetType":"codexRateLimits"},
                        {"status":"available","expiresAt":50}
                    ]
                }
            }),
            None,
        );
        assert_eq!(status["windows"], json!([]));
        assert_eq!(status["credits"]["balance"], "42");
        let summary = &status["resetCredits"];
        assert_eq!(summary["availableCount"], 4);
        assert_eq!(summary["nextExpiresAt"], 200);
        assert!(summary["accountKey"].is_null());
        assert_eq!(summary["cards"].as_array().unwrap().len(), 3);
        assert_eq!(summary["cards"][0]["id"], "first");
        assert_eq!(summary["cards"][0]["grantedAt"], 100);
        assert_eq!(
            summary["cards"][0]["description"],
            "Clears eligible windows"
        );
        assert_eq!(summary["cards"][2]["status"], "redeemed");
        assert!(status["unavailable"].is_null());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_retry_reuses_the_same_credit_and_key_after_an_unconfirmed_reset() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let fixture = tempfile::tempdir().unwrap();
        let cwd = fixture.path();
        let bin = cwd.join("codex-fixture");
        fs::write(&bin, r#"#!/bin/sh
while IFS= read -r frame; do
  case "$frame" in
    *'"method":"initialize"'*) echo '{"id":1,"result":{}}' ;;
    *'"method":"account/read"'*) echo '{"id":2,"result":{"account":{"type":"chatgpt","email":"fixture@example.test"}}}' ;;
    *'"method":"account/rateLimits/read"'*)
      if [ -f first ]; then
        echo '{"id":3,"result":{"accountId":"workspace","rateLimitResetCredits":{"availableCount":1,"credits":[{"status":"available","id":"first","expiresAt":4102444800}]}}}'
      else
        echo '{"id":3,"result":{"accountId":"workspace","rateLimitResetCredits":{"availableCount":2,"credits":[{"status":"available","id":"first","expiresAt":4102444800},{"status":"available","id":"later","expiresAt":4102444900}]}}}'
      fi ;;
    *'"method":"account/rateLimitResetCredit/consume"'*)
      if [ -f first ]; then
        printf '%s\n' "$frame" > retry
        echo '{"id":4,"result":{"outcome":"alreadyRedeemed"}}'
      else
        printf '%s\n' "$frame" > first
        echo '{"id":4,"error":{"code":-32000}}'
      fi ;;
  esac
done
"#).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o700)).unwrap();
        let def = super::super::registry::get_agent_def("codex").unwrap();
        let probe = def.account.unwrap();
        let snapshots = AccountSnapshots::default();
        let expected = |card: &str| CodexResetTarget {
            account_key: codex_reset_credits(
                &json!({"type":"chatgpt","email":"fixture@example.test"}),
                &json!({"accountId":"workspace","rateLimitResetCredits":{"availableCount":2}}),
            )
            .unwrap()
            .account_key
            .unwrap(),
            credit_id: Some(card.into()),
        };
        let mut wrong_account = expected("first");
        wrong_account.account_key = "different-workspace".into();
        assert!(snapshots
            .redeem_codex_reset_credit(def, probe, &bin, cwd, wrong_account, 2)
            .await
            .is_err());
        assert!(!cwd.join("first").exists());
        assert!(snapshots
            .redeem_codex_reset_credit(def, probe, &bin, cwd, expected("later"), 2)
            .await
            .is_err());
        let outcome = snapshots
            .redeem_codex_reset_credit(def, probe, &bin, cwd, expected("later"), 1)
            .await
            .unwrap();
        assert!(matches!(outcome, CodexResetOutcome::AlreadyRedeemed));
        let issued: Value = serde_json::from_slice(&fs::read(cwd.join("first")).unwrap()).unwrap();
        let retry: Value = serde_json::from_slice(&fs::read(cwd.join("retry")).unwrap()).unwrap();
        assert_eq!(issued["params"], retry["params"]);
        assert_eq!(issued["params"]["creditId"], "later");
        assert!(Uuid::parse_str(issued["params"]["idempotencyKey"].as_str().unwrap()).is_ok());
    }
}
