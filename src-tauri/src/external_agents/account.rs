//! Read-only native account/usage discovery. Never starts a conversation or returns credentials.
use crate::{
    external_agents::{
        registry::get_agent_def,
        spawn::{agent_cli_command, join_stderr_tail, spawn_stderr_tail, AgentChild},
    },
    proc::NoConsoleWindow,
};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn exchange(
    input: &mut tokio::process::ChildStdin,
    output: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
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

pub async fn probe(agent_id: &str, bin: &Path, cwd: &Path) -> Result<Value, String> {
    let def = get_agent_def(agent_id).ok_or("未知 CLI")?;
    probe_with_command(agent_id, agent_cli_command(def, bin), cwd).await
}

/// A host may supply the configured environment without changing the read-only protocol.
pub async fn probe_with_command(agent_id: &str, mut command: tokio::process::Command, cwd: &Path) -> Result<Value, String> {
    if agent_id == "antigravity" {
        return probe_antigravity(command, cwd).await;
    }
    if !matches!(agent_id, "codex" | "claude" | "grok") {
        return Err("此 CLI 未提供账号额度查询".into());
    }
    let claude = agent_id == "claude";
    if claude {
        command.args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--replay-user-messages",
        ]);
    } else if agent_id == "grok" {
        command.args(["--no-auto-update", "agent", "--no-leader", "stdio"]);
    } else {
        command.arg("app-server");
    }
    let mut child = AgentChild::from(
        command
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .no_console_window()
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let tail = spawn_stderr_tail(child.stderr.take());
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        let mut input = child.stdin.take().ok_or("CLI stdin unavailable")?;
        let mut output = BufReader::new(child.stdout.take().ok_or("CLI stdout unavailable")?).lines();
        let init = if claude {
            json!({"type":"control_request","request_id":"account-init","request":{"subtype":"initialize"}})
        } else if agent_id == "grok" {
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"kivio","version":env!("CARGO_PKG_VERSION")}}})
        } else {
            json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"kivio","version":env!("CARGO_PKG_VERSION")}}})
        };
        let init = exchange(&mut input, &mut output, init, claude).await?;
        if agent_id == "grok" {
            let billing = exchange(&mut input, &mut output, json!({"jsonrpc":"2.0","id":2,"method":"_x.ai/billing","params":{}}), false).await?;
            return Ok(normalize_grok(&billing));
        }
        let account = if claude { init["account"].clone() } else {
            input.write_all(b"{\"method\":\"initialized\",\"params\":{}}\n").await.map_err(|e| e.to_string())?;
            exchange(&mut input, &mut output, json!({"id":2,"method":"account/read","params":{"refreshToken":false}}), false).await?["account"].clone()
        };
        let request = if claude {
            json!({"type":"control_request","request_id":"account-usage","request":{"subtype":"get_usage","skip_behaviors":true}})
        } else { json!({"id":3,"method":"account/rateLimits/read","params":{}}) };
        // Optional usage has its own budget; losing it must not discard a valid identity.
        let usage = tokio::time::timeout(Duration::from_secs(8), exchange(&mut input, &mut output, request, claude)).await;
        let (usage, unavailable) = match usage {
            Ok(Ok(usage)) => (usage, None),
            Ok(Err(error)) => (Value::Null, Some(error)),
            Err(_) => (Value::Null, Some("额度查询超时，可稍后刷新".to_string())),
        };
        Ok(normalize(agent_id, &account, &usage, unavailable))
    }).await.unwrap_or_else(|_| Err("账号查询超时，请确认 CLI 可正常启动".into()));
    let _ = join_stderr_tail(&mut child, tail).await;
    let _ = child.wait().await;
    result
}

fn normalize_grok(billing: &Value) -> Value {
    let config = &billing["config"];
    let used = config["creditUsagePercent"]
        .as_f64()
        .or_else(|| {
            let limit = config["monthlyLimit"]["val"].as_f64()?;
            (limit > 0.0).then(|| config["used"]["val"].as_f64().unwrap_or(0.0) / limit * 100.0)
        })
        .filter(|v| v.is_finite());
    let mut windows = Vec::new();
    if let Some(used) = used {
        let period = &config["currentPeriod"];
        let weekly = period["type"].as_str() == Some("USAGE_PERIOD_TYPE_WEEKLY");
        windows.push(json!({"name":if weekly { "Grok · 7 天" } else { "Grok" },
            "usedPercent":used.clamp(0.0,100.0),
            "resetsAt":period["end"].as_str().or_else(|| config["billingPeriodEnd"].as_str()),
            "windowDurationMins":if weekly { Some(10080) } else { None }}));
    }
    let credits = config.get("prepaidBalance").filter(|v| v.is_object()).map(|v| {
        json!({"label":"预付余额 (USD)","balance":format!("{:.2}", v["val"].as_f64().unwrap_or(0.0) / 100.0)})
    });
    json!({"agentId":"grok", "account":{"type":"Grok", "email":null,"plan":billing["subscription_tier"].as_str()},
        "windows":windows,"credits":credits,
        "unavailable":if windows.is_empty() { Some("当前 Grok 账号未提供额度窗口，团队账号可能由管理员统一管理") } else { None },
        "checkedAt":chrono::Utc::now().timestamp()})
}

async fn probe_antigravity(mut command: tokio::process::Command, cwd: &Path) -> Result<Value, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(35),
        command
            .args([
                "-p",
                "/usage",
                "--output-format",
                "json",
                "--print-timeout",
                "30s",
            ])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .no_console_window()
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "反重力额度查询超时，请先在终端运行 agy 完成登录".to_string())?
    .map_err(|e| e.to_string())?;
    let payload: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "反重力未返回额度信息，请先在终端运行 agy 完成登录".to_string())?;
    if !output.status.success() || payload["status"] != "SUCCESS" {
        return Err("反重力额度查询失败，请确认 CLI 已登录后重试".into());
    }
    normalize_antigravity(&payload)
}

fn normalize_antigravity(payload: &Value) -> Result<Value, String> {
    // Validate the native report envelope: never interpret an agent response as quota.
    if payload["command"]["name"] != "usage" || payload["num_turns"] != 0 {
        return Err("反重力未返回原生额度报告".into());
    }
    let groups = payload["command"]["data"]["groups"]
        .as_array()
        .ok_or("反重力额度报告格式不受支持")?;
    let mut windows = Vec::new();
    for group in groups {
        if let Some(buckets) = group["buckets"].as_array() {
            for bucket in buckets {
                if let Some(remaining) = bucket["remaining_fraction"]
                    .as_f64()
                    .filter(|v| v.is_finite())
                {
                    let duration = match bucket["window"].as_str() {
                        Some("weekly") => Some(10080),
                        Some("5h") => Some(300),
                        _ => None,
                    };
                    windows.push(json!({"name":format!("{} · {}", group["name"].as_str().unwrap_or("Antigravity"), bucket["name"].as_str().unwrap_or("Quota")),
                        "usedPercent":(1.0 - remaining.clamp(0.0, 1.0)) * 100.0,
                        "resetsAt":bucket["reset_time"].as_str(), "windowDurationMins":duration}));
                }
            }
        }
    }
    Ok(
        json!({"agentId":"antigravity", "account":null, "windows":windows, "credits":null,
        "unavailable":if windows.is_empty() { Some("CLI 未提供额度窗口") } else { None },
        "checkedAt":chrono::Utc::now().timestamp()}),
    )
}

fn normalize(
    agent: &str,
    account: &Value,
    usage: &Value,
    mut unavailable: Option<String>,
) -> Value {
    let mut windows = Vec::new();
    let mut credits = Value::Null;
    let mut add_window = |name: String, window: &Value, claude: bool| {
        let used = window[if claude { "utilization" } else { "usedPercent" }].as_f64();
        if let Some(used) = used.filter(|value| value.is_finite()) {
            let reset = &window[if claude { "resets_at" } else { "resetsAt" }];
            windows.push(json!({"name":name,"usedPercent":used.clamp(0.0,100.0),
                "resetsAt": if reset.is_string() || reset.is_number() { reset.clone() } else { Value::Null },
                "windowDurationMins":window["windowDurationMins"].as_u64()}));
        }
    };
    if agent == "claude" {
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
    }
    if windows.is_empty() && unavailable.is_none() {
        unavailable = Some("当前账号或 CLI 未提供额度窗口".into());
    }
    json!({"agentId":agent,"account": if account.is_object() { json!({
        "type":account["type"].as_str().or_else(|| account["apiProvider"].as_str()),
        "email":account["email"].as_str(),"plan":account["planType"].as_str().or_else(|| account["subscriptionType"].as_str())
    }) } else { Value::Null }, "windows":windows,"credits":credits,"unavailable":unavailable,
    "checkedAt":chrono::Utc::now().timestamp()})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn antigravity_native_quota_report_preserves_groups_and_rejects_model_turns() {
        let mut report = json!({"command":{"name":"usage","data":{"groups":[
            {"name":"Gemini Models","buckets":[{"name":"Weekly","window":"weekly","remaining_fraction":0.75,"reset_time":"2026-10-30T00:00:00Z"}]},
            {"name":"Claude and GPT models","buckets":[{"name":"Five Hour","window":"5h","remaining_fraction":0.5}]}]}}, "num_turns":0});
        let status = normalize_antigravity(&report).unwrap();
        assert_eq!(status["windows"].as_array().unwrap().len(), 2);
        assert_eq!(status["windows"][0]["usedPercent"], 25.0);
        assert_eq!(status["windows"][0]["resetsAt"], "2026-10-30T00:00:00Z");
        assert_eq!(status["windows"][1]["windowDurationMins"], 300);
        report["num_turns"] = json!(1);
        assert!(normalize_antigravity(&report).is_err());
    }

    #[test]
    fn grok_billing_prefers_native_percent_and_converts_prepaid_cents() {
        let status = normalize_grok(&json!({"subscription_tier":"SuperGrok", "config":{
            "creditUsagePercent":12.5, "monthlyLimit":{"val":100}, "used":{"val":99},
            "currentPeriod":{"type":"USAGE_PERIOD_TYPE_WEEKLY","end":"2026-10-30T00:00:00Z"},
            "prepaidBalance":{"val":1250}}}));
        assert_eq!(status["windows"][0]["usedPercent"], 12.5);
        assert_eq!(status["credits"]["balance"], "12.50");
        assert_eq!(status["account"]["plan"], "SuperGrok");
        let status = normalize_grok(&json!({"config":null}));
        assert!(status["windows"].as_array().unwrap().is_empty());
        assert!(status["unavailable"].is_string());
    }

    #[test]
    fn native_account_payloads_keep_all_windows_without_exposing_credentials() {
        let account = json!({"type":"chatgpt","email":"a@example.test","planType":"pro","accessToken":"SECRET"});
        let status = normalize(
            "codex",
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
            &json!({"subscriptionType":"max"}),
            &json!({
                "rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":80,"resets_at":"2026-10-23T00:00:00Z"},
                    "seven_day":{"utilization":0,"resets_at":null}, "extra_usage":{"is_enabled":true,"used_credits":7,"monthly_limit":100}}
            }),
            None,
        );
        assert_eq!(status["windows"].as_array().unwrap().len(), 2);
        assert_eq!(status["credits"]["used"], 7.0);
        let unavailable = normalize("claude", &Value::Null, &Value::Null, None);
        assert!(unavailable["windows"].as_array().unwrap().is_empty());
        assert!(unavailable["unavailable"].is_string());
    }
}
