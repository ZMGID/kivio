//! Read-only native account/usage discovery. Never starts a conversation or returns credentials.
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
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

pub(super) struct ProbeReading {
    snapshot: Value,
    definitively_unavailable: bool,
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
    output: &mut tokio::io::Lines<BufReader<super::diagnostics::AgentStdout>>,
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
        AccountProtocol::AntigravityUsage | AccountProtocol::KimiWeb => {
            let mut command = agent_probe_command(def, bin);
            command.args(probe.args).current_dir(cwd);
            let usage = if protocol == AccountProtocol::AntigravityUsage {
                antigravity_usage(command).await
            } else {
                kimi_usage(command).await
            };
            let (usage, unavailable) = match usage {
                Ok(usage) => (usage, None),
                Err(error) => (Value::Null, Some(error)),
            };
            Ok(reading(def.id, protocol, &Value::Null, &usage, unavailable))
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

async fn kimi_usage(mut command: Command) -> Result<Value, String> {
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
    let usage = async {
        let response = client
            .get(format!("{api}/oauth/usage"))
            .bearer_auth(&token)
            .timeout(Duration::from_secs(8))
            .send()
            .await?
            .error_for_status()?;
        response.json::<Value>().await
    }
    .await
    .map_err(|_| "Kimi Code 额度接口暂不可用".into());

    // Let the CLI close its own Web server and unregister it. AgentChild
    // terminates the process tree if an older or unresponsive CLI stays alive.
    let _ = client
        .post(format!("{api}/shutdown"))
        .bearer_auth(&token)
        .timeout(Duration::from_secs(2))
        .send()
        .await;
    let _ = timeout(Duration::from_secs(2), child.wait()).await;
    usage
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

async fn probe_stdio(
    def: &RuntimeAgentDef,
    probe: AccountProbe,
    bin: &Path,
    cwd: &Path,
) -> Result<ProbeReading, String> {
    let protocol = probe.protocol;
    let claude = protocol == AccountProtocol::ClaudeControl;
    let grok = protocol == AccountProtocol::GrokBilling;
    let mut command = agent_cli_command(def, bin);
    command.args(probe.args);
    if claude {
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
        let mut input = child.stdin.take().ok_or("CLI stdin unavailable")?;
        let mut output = BufReader::new(child.stdout.take().ok_or("CLI stdout unavailable")?).lines();
        let init = if claude {
            json!({"type":"control_request","request_id":"account-init","request":{"subtype":"initialize"}})
        } else if grok {
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}})
        } else {
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"kivio","version":env!("CARGO_PKG_VERSION")}}})
        };
        let init = exchange(&mut input, &mut output, init, claude).await?;
        let account = if claude { init["account"].clone() } else if grok { Value::Null } else {
            input.write_all(b"{\"method\":\"initialized\",\"params\":{}}\n").await.map_err(|e| e.to_string())?;
            exchange(&mut input, &mut output, json!({"id":2,"method":"account/read","params":{"refreshToken":false}}), false).await?["account"].clone()
        };
        let request = if claude {
            json!({"type":"control_request","request_id":"account-usage","request":{"subtype":"get_usage","skip_behaviors":true}})
        } else if grok { json!({"jsonrpc":"2.0","id":2,"method":"_x.ai/billing","params":{}}) } else { json!({"id":3,"method":"account/rateLimits/read","params":{}}) };
        // Optional usage has its own budget; losing it must not discard a valid identity.
        let usage = tokio::time::timeout(Duration::from_secs(8), exchange(&mut input, &mut output, request, claude)).await;
        let (usage, unavailable) = match usage {
            Ok(Ok(usage)) => (usage, None),
            Ok(Err(error)) => (Value::Null, Some(error)),
            Err(_) => (Value::Null, Some("额度查询超时，可稍后刷新".to_string())),
        };
        Ok(reading(def.id, protocol, &account, &usage, unavailable))
    }).await.unwrap_or_else(|_| Err("账号查询超时，请确认 CLI 可正常启动".into()));
    let _ = join_stderr_tail(&mut child, tail).await;
    let _ = child.wait().await;
    result
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
    let account = if protocol == AccountProtocol::GrokBilling {
        json!({"planType": usage["subscription_tier"]})
    } else {
        account.clone()
    };
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
        let command = &usage["command"];
        if usage["status"] == "SUCCESS" && usage["num_turns"] == 0 && command["name"] == "usage" {
            if let Some(groups) = command["data"]["groups"].as_array() {
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
        let summary = &usage["rateLimitResetCredits"];
        if let Some(count) = summary["availableCount"].as_u64() {
            let next_expiry = summary["credits"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|credit| credit["status"] == "available")
                .filter_map(|credit| credit["expiresAt"].as_i64().filter(|expiry| *expiry > 0))
                .min();
            reset_credits = json!({"availableCount":count,"nextExpiresAt":next_expiry});
        }
    }
    if windows.is_empty() && reset_credits.is_null() && unavailable.is_none() {
        unavailable = Some("当前账号或 CLI 未提供额度窗口".into());
    }
    json!({"agentId":agent,"account": if account.is_object() { json!({
        "type":account["type"].as_str().or_else(|| account["apiProvider"].as_str()),
        "email":account["email"].as_str(),"plan":account["planType"].as_str().or_else(|| account["subscriptionType"].as_str())
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
            &Value::Null,
            &payload,
            None,
        );
        let windows = status["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0]["usedPercent"], 0.0);
        assert_eq!(windows[1]["usedPercent"], 100.0);
        assert_eq!(windows[1]["resetsAt"], "2026-11-28T00:00:00Z");
        assert_eq!(windows[2]["usedPercent"], 25.0);

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
        assert!(!status.to_string().contains("SECRET"));
        assert!(status["unavailable"].is_null());
        let status = normalize(
            "claude",
            AccountProtocol::ClaudeControl,
            &json!({"subscriptionType":"max"}),
            &json!({
                "rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":80,"resets_at":"2026-10-23T00:00:00Z"},
                    "seven_day":{"utilization":0,"resets_at":null}, "extra_usage":{"is_enabled":true,"used_credits":7,"monthly_limit":100}}
            }),
            None,
        );
        assert_eq!(status["windows"].as_array().unwrap().len(), 2);
        assert_eq!(status["credits"]["used"], 7.0);
        let grok = normalize(
            "grok",
            AccountProtocol::GrokBilling,
            &Value::Null,
            &json!({"config":{"creditUsagePercent":12.5,"currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY"},
                "billingPeriodStart":"2026-11-01T00:00:00+00:00","billingPeriodEnd":"2026-11-08T00:00:00+00:00"},
                "subscription_tier":"SuperGrok Plus"}),
            None,
        );
        assert_eq!(grok["windows"][0]["usedPercent"], 12.5);
        assert_eq!(grok["windows"][0]["windowDurationMins"], 7 * 24 * 60);
        assert_eq!(grok["account"]["plan"], "SuperGrok Plus");
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
                        {"status":"consumed","expiresAt":100},
                        {"status":"available","expiresAt":300,"id":"internal-card-id"},
                        {"status":"available","expiresAt":200}
                    ]
                }
            }),
            None,
        );
        assert_eq!(status["windows"], json!([]));
        assert_eq!(status["credits"]["balance"], "42");
        assert_eq!(
            status["resetCredits"],
            json!({"availableCount":4,"nextExpiresAt":200})
        );
        assert!(status["unavailable"].is_null());
    }
}
