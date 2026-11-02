use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::external_agents::spawn::AgentChild as Child;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::acp_terminal::{AcpTerminalHost, TerminalReply};
use crate::external_agents::session::live::{
    ApprovalAsk, ApprovalBridge, ApprovalDecision, SessionCommand,
};
use crate::external_agents::stream::{usage_from_parts, CliUsageParts};
use crate::external_agents::types::{
    default_model_option, ExternalCliSlashCommand, RuntimeModelOption, UnifiedAgentEvent,
};
use crate::proc::NoConsoleWindow;

const ACP_PROTOCOL_VERSION: i64 = 1;

fn cli_protocol_cwd(bin: &Path, cwd: &Path) -> PathBuf {
    crate::external_agents::wsl::path_for_cli(bin, cwd)
}

/// ACP 模型探测结果：模型列表 + CLI 当前配置的模型/推理等级（用于胶囊回填，见 R "同步 CLI 当前配置"）。
pub struct AcpModelsProbe {
    pub models: Vec<RuntimeModelOption>,
    pub current_model: Option<String>,
    pub current_reasoning: Option<String>,
    /// CLI 在 `configOptions` 里自报的推理档位选项（如 kimi 的 `thinking`：low/high/max）。
    /// 空 = 该 CLI 不暴露档位，前端退回 `def.reasoning_options` 静态表。
    pub reasoning_options: Vec<RuntimeModelOption>,
}

/// Handshake timeouts (缺陷 4 / R3): Paseo uses 60s; desktop starts at 30s. `initialize` and
/// `session/new` each get their own budget so a slow one doesn't starve the other.
const ACP_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const ACP_SESSION_NEW_TIMEOUT: Duration = Duration::from_secs(30);

use crate::external_agents::spawn::{fold_stderr, join_stderr_tail as join_tail};

#[derive(Debug, Clone)]
pub enum AcpMcpServer {
    Stdio {
        name: String,
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    },
    Http {
        name: String,
        url: String,
        headers: Vec<(String, String)>,
    },
}

fn supported_mcp_servers<'a>(
    servers: &'a [AcpMcpServer],
    initialize_result: &Value,
    is_wsl: bool,
) -> Vec<&'a AcpMcpServer> {
    let http = !is_wsl
        && initialize_result
            .pointer("/agentCapabilities/mcpCapabilities/http")
            .and_then(Value::as_bool)
            == Some(true);
    // WSL loopback may not reach the Windows-hosted server.
    servers
        .iter()
        .filter(|server| http || matches!(server, AcpMcpServer::Stdio { .. }))
        .collect()
}

fn build_session_new_params(
    cwd: &Path,
    mcp_servers: &[&AcpMcpServer],
    additional_directories: Option<&[String]>,
) -> Value {
    let servers: Vec<Value> = mcp_servers
        .iter()
        .map(|server| match server {
            AcpMcpServer::Stdio { name, command, args, env } => json!({
                "type": "stdio",
                "name": name,
                "command": command,
                "args": args,
                "env": env.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect::<Vec<_>>(),
            }),
            AcpMcpServer::Http { name, url, headers } => json!({
                "type": "http",
                "name": name,
                "url": url,
                "headers": headers.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect::<Vec<_>>(),
            }),
        })
        .collect();
    let mut params = json!({
        "cwd": cwd.to_string_lossy(),
        "mcpServers": servers,
    });
    if let Some(dirs) = additional_directories {
        params["additionalDirectories"] = json!(dirs);
    }
    params
}

fn build_session_load_params(
    cwd: &Path,
    session_id: &str,
    mcp_servers: &[&AcpMcpServer],
    additional_directories: Option<&[String]>,
) -> Value {
    let mut params = build_session_new_params(cwd, mcp_servers, additional_directories);
    params["sessionId"] = json!(session_id);
    params
}

/// `session/load` 的目标会话在 CLI 那边已经不存在。握手超时 / Method not found 不是这个。
pub fn is_missing_acp_session_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if lower.contains("timeout")
        || lower.contains("method not found")
        || lower.contains("-32601")
        || lower.contains("exited during handshake")
    {
        return false;
    }
    let mentions_session =
        lower.contains("session") || lower.contains("sessionid") || lower.contains("session/load");
    let gone = lower.contains("not found")
        || lower.contains("no such")
        || lower.contains("unknown session")
        || lower.contains("does not exist")
        || lower.contains("is gone");
    mentions_session && gone
}

async fn write_rpc(
    stdin: &mut (impl AsyncWrite + Unpin),
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let mut line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

async fn write_rpc_result(
    stdin: &mut (impl AsyncWrite + Unpin),
    id: &Value,
    result: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let mut line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

async fn write_rpc_error(
    stdin: &mut (impl AsyncWrite + Unpin),
    id: &Value,
    code: i64,
    message: &str,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    });
    let mut line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

fn agent_supports_additional_directories(initialize_result: &Value) -> bool {
    if initialize_result
        .pointer("/agentCapabilities/sessionCapabilities/additionalDirectories")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return true;
    }
    initialize_result
        .pointer("/capabilities/session/additionalDirectories")
        .and_then(Value::as_bool)
        == Some(true)
}

fn acp_initialize_params(terminal: bool) -> Value {
    // Live chat sessions must advertise `terminal: true` and implement `terminal/*`.
    // Probe/import processes never handle those methods, so they keep it false.
    let mut client_capabilities = json!({ "terminal": terminal });
    if terminal {
        // Grok 1.0.17+ can forward MCP form elicitation over ACP. Probes do not
        // own an interaction host, so they must not advertise the capability.
        client_capabilities["elicitation"] = json!({ "form": {} });
    }
    json!({
        "protocolVersion": ACP_PROTOCOL_VERSION,
        "clientCapabilities": client_capabilities,
        "clientInfo": {
            "name": "kivio",
            "title": "Kivio",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn rpc_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if let Some(message) = error.get("message").and_then(|v| v.as_str()) {
        return Some(message.to_string());
    }
    error.get("code").map(|c| c.to_string())
}

/// 从 ACP modelId 的方括号参数里抠出上下文窗口提示。
///
/// cursor 实测形态（`session/new` 的 `models.availableModels[]`）：
/// ```text
/// claude-opus-5[thinking=true,context=300k,effort=high,fast=false]
/// gpt-5.6-sol[context=272k,reasoning=medium,fast=false]
/// composer-2.5[fast=true]        <- 无 context，返回 None
/// ```
/// 32 个模型中 13 个带 `context=`，且**全部没有** `_meta.totalContextTokens`
/// （那是 grok 的形态）——不解析这里 cursor 的分母就永远没来源。
///
/// 复用 `context::parse_context_window_label`（"300k" → 300000，支持 K/M 后缀与浮点）。
/// 没有方括号、没有 `context=`、值解析不出来时一律 `None`——**不猜**。
fn context_window_from_model_id_params(model_id: &str) -> Option<u32> {
    let start = model_id.find('[')?;
    let end = model_id[start + 1..].find(']')? + start + 1;
    model_id[start + 1..end]
        .split(',')
        .filter_map(|param| param.split_once('='))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case("context"))
        .and_then(|(_, value)| {
            crate::external_agents::context::parse_context_window_label(value.trim())
        })
}

fn normalize_models(result: &Value) -> Vec<RuntimeModelOption> {
    let mut out = vec![default_model_option()];
    let mut seen = HashSet::from(["default".to_string()]);

    // 有 currentModelId（如 grok 的 models.currentModelId）时，把 default 占位标成
    // "Default (<真实模型名>)" —— 否则单模型 CLI 的胶囊上永远只见 "Default"，
    // 用户无从知道探测其实成功了（与 claude 的 label_for_claude_model 呈现一致）。
    let current_id = result
        .get("models")
        .and_then(|m| m.get("currentModelId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    if let Some(config_options) = result.get("configOptions").and_then(|v| v.as_array()) {
        for raw_option in config_options {
            let option = match raw_option.as_object() {
                Some(o) => o,
                None => continue,
            };
            let config_id = option.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if config_id != "model"
                && option.get("category").and_then(|v| v.as_str()) != Some("model")
            {
                continue;
            }
            if let Some(values) = option.get("options").and_then(|v| v.as_array()) {
                for raw_value in values {
                    let value = match raw_value.as_object() {
                        Some(o) => o,
                        None => continue,
                    };
                    let id = value
                        .get("value")
                        .or_else(|| value.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if id.is_empty() || !seen.insert(id.to_string()) {
                        continue;
                    }
                    let name = value.get("name").and_then(|v| v.as_str()).unwrap_or(id);
                    // cursor 的模型走的是**这条** configOptions 分支（不是下面的
                    // models.availableModels），且它把窗口写在 modelId 的方括号里
                    // （`claude-opus-5[thinking=true,context=300k,...]`）——实测 33 个模型
                    // 里 13 个带 `context=`，全都没有 `_meta.totalContextTokens`。
                    // 这里若留 None，cursor 的分母就永远没来源、只能吃兜底。
                    // `_meta.totalContextTokens` 仍优先（显式字段比字符串提示可靠）。
                    let window = value
                        .get("_meta")
                        .and_then(|m| m.get("totalContextTokens"))
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32)
                        .or_else(|| context_window_from_model_id_params(id));
                    if current_id == Some(id) {
                        out[0].label = format!("Default ({name})");
                        out[0].context_window_tokens = window;
                    }
                    out.push(RuntimeModelOption {
                        capabilities: None,
                        id: id.to_string(),
                        label: if name != id {
                            format!("{name} ({id})")
                        } else {
                            id.to_string()
                        },
                        context_window_tokens: window,
                    });
                }
            }
            if out.len() > 1 {
                return out;
            }
        }
    }

    if let Some(models) = result.get("models").and_then(|v| v.as_object()) {
        if let Some(available) = models.get("availableModels").and_then(|v| v.as_array()) {
            for model in available {
                let id = model.get("modelId").and_then(|v| v.as_str()).unwrap_or("");
                if id.is_empty() || !seen.insert(id.to_string()) {
                    continue;
                }
                let name = model.get("name").and_then(|v| v.as_str()).unwrap_or(id);
                // 窗口优先级：`_meta.totalContextTokens`（显式字段，grok）> modelId 方括号里的
                // `context=300k`（字符串提示，cursor）> None。显式字段比字符串提示可靠。
                let window = model
                    .get("_meta")
                    .and_then(|m| m.get("totalContextTokens"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .or_else(|| context_window_from_model_id_params(id));
                if current_id == Some(id) {
                    out[0].label = format!("Default ({name})");
                    out[0].context_window_tokens = window;
                }
                out.push(RuntimeModelOption {
                    capabilities: None,
                    id: id.to_string(),
                    label: if name != id {
                        format!("{name} ({id})")
                    } else {
                        id.to_string()
                    },
                    // grok 等在 _meta.totalContextTokens 报真实窗口（如 500000）；
                    // cursor 把它写在 modelId 的 `context=300k` 里；都没有则留空。
                    context_window_tokens: window,
                });
            }
        }
    }

    out
}

/// 把某个 ACP `result`/`update` 里解析出的模型并入累加器（去重）。default 占位不进 `acc`，
/// 但若其标签被 `normalize_models` 用 currentModelId 富化过（"Default (Grok 4.5)"），
/// 则更新 `default_slot`，最终装配时替换普通占位。
fn merge_acp_models(
    acc: &mut Vec<RuntimeModelOption>,
    seen: &mut HashSet<String>,
    default_slot: &mut RuntimeModelOption,
    value: &Value,
) {
    for model in normalize_models(value) {
        if model.id == "default" {
            if model.label != "Default" {
                *default_slot = model;
            }
            continue;
        }
        if seen.insert(model.id.clone()) {
            acc.push(model);
        }
    }
}

/// 测试替身：复刻 `detect_acp_models` 读循环的行处理（跳空行 / 跳非 JSON banner / 合并
/// result 与异步 session/update 推送），不启子进程。生产循环与此共用 `merge_acp_models`。
#[cfg(test)]
fn collect_acp_models_from_lines(lines: &[&str]) -> Option<Vec<RuntimeModelOption>> {
    let mut collected: Vec<RuntimeModelOption> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut default_slot = default_model_option();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value.get("method").and_then(|v| v.as_str()) == Some("session/update") {
            if let Some(update) = value.get("params").and_then(|p| p.get("update")) {
                merge_acp_models(&mut collected, &mut seen, &mut default_slot, update);
            }
            continue;
        }
        if let Some(result) = value.get("result") {
            merge_acp_models(&mut collected, &mut seen, &mut default_slot, result);
        }
    }
    if collected.is_empty() {
        return None;
    }
    let mut out = vec![default_slot];
    out.extend(collected);
    Some(out)
}

/// 从一个 ACP `result`/`update` 里抽取 CLI 当前配置：`models.currentModelId` 作为当前模型，
/// 当前模型条目的 `_meta.reasoningEfforts` 里 `default=true` 的 id 作为当前推理等级（grok 实测 high）。
/// 结构未知时保守返回 None（不破坏 default 富化路径）。
/// 从 ACP `configOptions` 里抽出**推理档位**那一项的选项列表 + 当前值。
///
/// kimi 实测形态（`session/new` 的 result）：
/// ```json
/// {"type":"select","id":"thinking","category":"thought_level","currentValue":"high",
///  "options":[{"value":"low","name":"Low"},{"value":"high","name":"High"},
///             {"value":"max","name":"Max"}]}
/// ```
/// 此前只有「设置」侧认这个（`acp_config_ids` 能发 set_config_option），
/// **发现侧完全没读** —— 于是胶囊上没有档位可选。
///
/// 识别口径与 `acp_config_ids` 保持一致（id 含 reasoning/thought/thinking/effort，
/// 或 category 为 reasoning/thought/thought_level），避免两处判据分叉。
/// 是否是无意义的开关型档位（kimi always_thinking 模型：options 只有 `On`）。
/// 这种不能当 effort 胶囊用——用户会看到莫名其妙的「On」。
fn is_boolean_toggle_efforts(options: &[RuntimeModelOption]) -> bool {
    if options.is_empty() {
        return false;
    }
    options.iter().all(|o| {
        matches!(
            o.id.to_lowercase().as_str(),
            "on" | "off" | "true" | "false" | "enabled" | "disabled" | "1" | "0" | "yes" | "no"
        )
    })
}

fn extract_acp_reasoning(value: &Value) -> (Vec<RuntimeModelOption>, Option<String>) {
    let Some(config_options) = value.get("configOptions").and_then(|v| v.as_array()) else {
        return (Vec::new(), None);
    };
    // 先收集所有候选，优先取多档 effort（low/high/max），跳过只有 On/Off 的开关。
    let mut best: Option<(Vec<RuntimeModelOption>, Option<String>)> = None;
    for raw in config_options {
        let Some(option) = raw.as_object() else {
            continue;
        };
        let id_l = option
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let category = option
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let is_reasoning = id_l.contains("reasoning")
            || id_l.contains("thought")
            || id_l.contains("thinking")
            || id_l.contains("effort")
            || category == "reasoning"
            || category == "thought"
            || category == "thought_level";
        if !is_reasoning {
            continue;
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for raw_value in option
            .get("options")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(entry) = raw_value.as_object() else {
                continue;
            };
            let id = entry
                .get("value")
                .or_else(|| entry.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if id.is_empty() || !seen.insert(id.to_string()) {
                continue;
            }
            let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            out.push(RuntimeModelOption {
                capabilities: None,
                id: id.to_string(),
                label: name.to_string(),
                context_window_tokens: None,
            });
        }
        let current = option
            .get("currentValue")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // 只有 On/Off 的开关：不当作 effort 档位（kimi K2.7 always_thinking）。
        if is_boolean_toggle_efforts(&out) {
            continue;
        }
        // 多档优先；同档数取先出现的。
        let take = match &best {
            None => true,
            Some((prev, _)) => out.len() > prev.len(),
        };
        if take {
            best = Some((out, current));
        }
    }
    best.unwrap_or((Vec::new(), None))
}

fn extract_acp_current(value: &Value) -> (Option<String>, Option<String>) {
    let models = value.get("models");
    let current_id = models
        .and_then(|m| m.get("currentModelId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let mut current_reasoning = None;
    if let (Some(models_obj), Some(cur)) = (models, current_id.as_deref()) {
        if let Some(available) = models_obj.get("availableModels").and_then(|v| v.as_array()) {
            for model in available {
                if model.get("modelId").and_then(|v| v.as_str()) != Some(cur) {
                    continue;
                }
                let meta = model.get("_meta");
                // 首选 default=true 的 reasoningEfforts 条目；退而求其次读标量 reasoningEffort。
                if let Some(efforts) = meta
                    .and_then(|m| m.get("reasoningEfforts"))
                    .and_then(|v| v.as_array())
                {
                    for effort in efforts {
                        if effort.get("default").and_then(|v| v.as_bool()) == Some(true) {
                            current_reasoning = effort
                                .get("id")
                                .or_else(|| effort.get("value"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(str::to_string);
                            break;
                        }
                    }
                }
                if current_reasoning.is_none() {
                    current_reasoning = meta
                        .and_then(|m| m.get("reasoningEffort"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                }
                break;
            }
        }
    }
    (current_id, current_reasoning)
}

pub async fn detect_acp_models(
    bin: &Path,
    args: &[&str],
    cwd: &Path,
    timeout_secs: u64,
) -> Option<AcpModelsProbe> {
    let mut child = crate::external_agents::spawn::cli_command(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .map(crate::external_agents::spawn::AgentChild::from)
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout).lines();

    let mut expected_id: u64 = 1;
    let mut next_id: u64 = 2;
    let mut collected: Vec<RuntimeModelOption> = Vec::new();
    let mut default_slot = default_model_option();
    let mut seen: HashSet<String> = HashSet::new();
    let mut current_model: Option<String> = None;
    let mut current_reasoning: Option<String> = None;
    let mut reasoning_options: Vec<RuntimeModelOption> = Vec::new();
    let deadline = Duration::from_secs(timeout_secs);

    write_rpc(&mut stdin, 1, "initialize", acp_initialize_params(false))
        .await
        .ok()?;

    let started = std::time::Instant::now();
    // 收到 session/new 结果后再留一个短窗口（1.5s）收异步 session/update 推送的模型（N4）——
    // 部分 CLI 在 session/new 之后才异步推模型列表（参照 detect_acp_commands 的通知处理）。
    let mut post_window: Option<std::time::Instant> = None;
    loop {
        if started.elapsed() > deadline {
            let _ = child.start_kill();
            break;
        }
        if let Some(since) = post_window {
            if since.elapsed() > Duration::from_millis(1500) {
                let _ = child.start_kill();
                break;
            }
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        // banner / 日志等非 JSON 行跳过（对齐同文件其他 reader），别因一行噪声放弃整个探测（缺陷 3）。
        let value: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // 异步模型推送：session/update 通知里携带的模型（N4）。
        if value.get("method").and_then(|v| v.as_str()) == Some("session/update") {
            if let Some(update) = value.get("params").and_then(|p| p.get("update")) {
                merge_acp_models(&mut collected, &mut seen, &mut default_slot, update);
                let (cm, cr) = extract_acp_current(update);
                current_model = current_model.or(cm);
                current_reasoning = current_reasoning.or(cr);
            }
            continue;
        }

        if rpc_error_message(&value).is_some() {
            if value.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
                continue;
            }
            // 匹配 id 的错误：结束探测；已收到的部分模型仍返回，否则判失败。
            let _ = child.start_kill();
            break;
        }
        if value.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
            continue;
        }
        let result = match value.get("result") {
            Some(r) => r,
            None => continue,
        };
        if expected_id == 1 {
            expected_id = next_id;
            write_rpc(
                &mut stdin,
                next_id,
                "session/new",
                build_session_new_params(&cli_protocol_cwd(bin, cwd), &[], None),
            )
            .await
            .ok()?;
            next_id += 1;
            continue;
        }
        if expected_id == 2 {
            merge_acp_models(&mut collected, &mut seen, &mut default_slot, result);
            let (cm, cr) = extract_acp_current(result);
            current_model = current_model.or(cm);
            current_reasoning = current_reasoning.or(cr);
            // configOptions 里的推理档位（kimi 的 `thinking`）——与 models._meta 那条
            // 并列，两种形态各占一家 CLI，都要认。
            let (opts, cur) = extract_acp_reasoning(result);
            if !opts.is_empty() && reasoning_options.is_empty() {
                reasoning_options = opts;
            }
            current_reasoning = current_reasoning.or(cur);
            // 不立即退出：再留 1.5s 收异步推送的模型。若此后无推送则窗口到点结束。
            post_window = Some(std::time::Instant::now());
            continue;
        }
    }

    if collected.is_empty() {
        return None;
    }
    let mut out = vec![default_slot];
    out.extend(collected);
    Some(AcpModelsProbe {
        models: out,
        current_model,
        current_reasoning,
        reasoning_options,
    })
}

fn parse_available_commands(
    update: &serde_json::Map<String, Value>,
) -> Vec<ExternalCliSlashCommand> {
    let list = update
        .get("availableCommands")
        .or_else(|| update.get("available_commands"))
        .and_then(|v| v.as_array());
    let Some(list) = list else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for raw in list {
        let Some(obj) = raw.as_object() else {
            continue;
        };
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(name) = name else {
            continue;
        };
        let description = obj
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        out.push(ExternalCliSlashCommand {
            slash: format!("/{name}"),
            name: name.to_string(),
            description,
            argument_hint: None,
        });
    }
    out
}

/// Discover an ACP agent's slash commands. Mirrors `detect_acp_models`: run `initialize`
/// → `session/new`, then keep reading `session/update` *notifications* and capture the one
/// whose `sessionUpdate == "available_commands_update"` (cursor pushes this asynchronously,
/// up to ~10s after the session is created). Returns the deduped, sorted command list.
pub async fn detect_acp_commands(
    bin: &Path,
    args: &[&str],
    cwd: &Path,
    timeout_secs: u64,
) -> Option<Vec<ExternalCliSlashCommand>> {
    let mut child = crate::external_agents::spawn::cli_command(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .map(crate::external_agents::spawn::AgentChild::from)
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout).lines();

    let mut expected_id: u64 = 1;
    let mut next_id: u64 = 2;
    let mut commands: Option<Vec<ExternalCliSlashCommand>> = None;
    let deadline = Duration::from_secs(timeout_secs);

    write_rpc(&mut stdin, 1, "initialize", acp_initialize_params(false))
        .await
        .ok()?;

    let started = std::time::Instant::now();
    loop {
        if started.elapsed() > deadline {
            let _ = child.start_kill();
            break;
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Capture the asynchronously-pushed available_commands_update notification.
        if value.get("method").and_then(|v| v.as_str()) == Some("session/update") {
            if let Some(update) = value
                .get("params")
                .and_then(|p| p.get("update"))
                .and_then(|v| v.as_object())
            {
                let session_update = update
                    .get("sessionUpdate")
                    .or_else(|| update.get("availableCommandsUpdate"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if session_update == "available_commands_update"
                    || session_update == "availableCommandsUpdate"
                    || update.contains_key("availableCommands")
                    || update.contains_key("available_commands")
                {
                    let parsed = parse_available_commands(update);
                    if !parsed.is_empty() {
                        commands = Some(parsed);
                        let _ = child.start_kill();
                        break;
                    }
                }
            }
            continue;
        }

        if rpc_error_message(&value).is_some() {
            if value.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
                continue;
            }
            let _ = child.start_kill();
            return None;
        }
        if value.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
            continue;
        }
        let result = value.get("result")?;
        if expected_id == 1 {
            expected_id = next_id;
            write_rpc(
                &mut stdin,
                next_id,
                "session/new",
                build_session_new_params(&cli_protocol_cwd(bin, cwd), &[], None),
            )
            .await
            .ok()?;
            next_id += 1;
            continue;
        }
        if expected_id == 2 {
            // session/new acknowledged; some agents include commands inline in the result.
            if let Some(update) = result.as_object() {
                let parsed = parse_available_commands(update);
                if !parsed.is_empty() {
                    commands = Some(parsed);
                    let _ = child.start_kill();
                    break;
                }
            }
            // Otherwise keep reading notifications until the agent pushes them or we time out.
            expected_id = 0; // no further responses expected
            continue;
        }
    }

    commands.map(|mut cmds| {
        cmds.sort_by(|a, b| a.name.cmp(&b.name));
        cmds.dedup_by(|a, b| a.name == b.name);
        cmds
    })
}

/// 选一个「批准」选项回给 ACP agent。
///
/// **优先挑 `allow_once`**：审批卡上的「允许一次」必须名副其实。以前优先挑
/// `approve_for_session` / `allow_always`，等于用户点「允许」就被静默升级成永久放行。
/// 「总是允许」由 Kivio 自己那张 `chat_tool_always_allow` 表兜住（后续同名工具不再弹卡，
/// 每次照样以 allow_once 回给 CLI），不需要把三态透传下来。
/// 只在 CLI 压根没给 once 选项时才退回 session/always（否则这一轮就卡死了）。
fn choose_permission_outcome(options: Option<&Value>) -> Option<String> {
    let list = options.and_then(|v| v.as_array())?;
    for item in list {
        if item.get("kind").and_then(|v| v.as_str()) == Some("allow_once") {
            if let Some(id) = item.get("optionId").and_then(|v| v.as_str()) {
                return Some(id.to_string());
            }
        }
    }
    for item in list {
        if item.get("optionId").and_then(|v| v.as_str()) == Some("approve_for_session") {
            return Some("approve_for_session".to_string());
        }
    }
    for item in list {
        if item.get("kind").and_then(|v| v.as_str()) == Some("allow_always") {
            if let Some(id) = item.get("optionId").and_then(|v| v.as_str()) {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// 解析 ACP `PromptResponse.usage`（官方标记 UNSTABLE，字段缺失一律按 0，不报错）。
///
/// **CLI 自报了 `totalTokens`（>0）就信它**，与 grok `_meta` 路同一原则：各 CLI 的
/// cache 与 input 包含口径不同，客户端按分量猜口径必错一家。分量仍如实记录（成本面板用）。
///
/// `totalTokens` 缺失时才按「四项并列」分量累加兜底（opencode 旧实测口径：
/// `inputTokens 11685 + outputTokens 4 + thoughtTokens 11 + cachedReadTokens 1792 = 13492`
/// 恰等于其自报 totalTokens，两路一致；`thoughtTokens` 不含在 `outputTokens` 内，与
/// pi/codex 相反，见 `CliUsageParts::reasoning` 注释）。hermes 实测（openai-codex 代理）属 OpenAI 口径
/// `cachedReadTokens ⊆ inputTokens`，并列累加会把 cache 双算，见
/// `prompt_response_usage_trusts_cli_reported_total` 的对账样本。
///
/// 全零时返回 `None`（保留既有行为：没报就是没报，不要造出一个 0 用量覆盖真实值）。
fn format_acp_usage(usage: &Value) -> Option<crate::chat::model::ModelUsage> {
    let field = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let reported_total = field("totalTokens");
    let parts = CliUsageParts {
        input: field("inputTokens"),
        output: field("outputTokens"),
        cache_read: field("cachedReadTokens"),
        cache_creation: field("cachedWriteTokens"),
        reasoning: field("thoughtTokens"),
        // opencode 实测 `cachedReadTokens` 与 `inputTokens` 不相交；自报 total 优先。
        // 其他 CLI 如 hermes 的 cache 可能是 input 子集，不能一概按分量重算。
        cache_included_in_input: false,
        // PromptResponse.usage 不带窗口——窗口只在 `usage_update.size` 里给。
        // 这里留 None 由 run.rs 的窗口粘滞保住先到的 usage_update.size。
        context_window: None,
    };
    if parts.input == 0
        && parts.output == 0
        && parts.cache_read == 0
        && parts.cache_creation == 0
        && parts.reasoning == 0
        && reported_total == 0
    {
        None
    } else {
        let mut model_usage = usage_from_parts(parts);
        if reported_total > 0 {
            model_usage.total_tokens = Some(reported_total);
        }
        Some(model_usage)
    }
}

/// OpenCode v2 的 `result.usage` 按 prompt 累计；grok 的 `_meta` 顶层是末次上下文。
/// https://opencode.ai/v2/docs/cli/acp/#usage
/// shortcut: OpenCode 1.18.x 只报末次请求，升级 OpenCode v2 才能取得整轮计费。
fn usage_from_prompt_result(
    result: &Value,
    latest_context: Option<u64>,
) -> Option<crate::chat::model::ModelUsage> {
    if let Some(mut usage) = result.get("usage").and_then(format_acp_usage) {
        usage.context_tokens = latest_context;
        return Some(usage);
    }
    let meta = result.get("_meta")?;
    let totals = meta.get("usage");
    let field = |key: &str| totals.and_then(|v| v.get(key)).and_then(Value::as_u64);
    let usage = crate::chat::model::ModelUsage {
        input_tokens: field("inputTokens"),
        output_tokens: field("outputTokens"),
        total_tokens: field("totalTokens"),
        cached_input_tokens: field("cachedReadTokens").filter(|v| *v > 0),
        cache_creation_input_tokens: field("cacheCreationTokens")
            .or_else(|| field("cachedWriteTokens"))
            .filter(|v| *v > 0),
        reasoning_tokens: field("reasoningTokens").filter(|v| *v > 0),
        context_tokens: meta
            .get("totalTokens")
            .and_then(Value::as_u64)
            .filter(|v| *v > 0)
            .or(latest_context),
        ..Default::default()
    };
    // grok 的缓存是 input 的子集，total 信其自报值，不用并列分量重新计算。
    (usage.context_tokens.is_some() || !crate::external_agents::run::usage_tokens_all_zero(&usage))
        .then_some(usage)
}

/// 解析 ACP `session/update` 的 `usage_update` 变体（官方 RFD「Session Usage and Context
/// Status」）。字段**平铺在 `update` 下**，不嵌套在 `usage` 对象里：
///
/// ```json
/// {"sessionUpdate":"usage_update","used":13477,"size":200000,
///  "cost":{"amount":0,"currency":"USD"}}
/// ```
///
/// `used` 是当前上下文占用，不是计费输入；`size` 是上下文窗口总大小。
/// `cost` 暂不解析——Kivio 目前没有成本展示位。
///
/// `used`/`size` 都缺时返回 `None`（不是这个变体，或上游没给数据）。
fn parse_acp_usage_update(
    update: &serde_json::Map<String, Value>,
) -> Option<crate::chat::model::ModelUsage> {
    let used = update.get("used").and_then(|v| v.as_u64());
    let size = update
        .get("size")
        .and_then(|v| v.as_u64())
        .filter(|v| *v > 0);
    if used.is_none() && size.is_none() {
        return None;
    }
    Some(crate::chat::model::ModelUsage {
        context_tokens: used,
        context_window_tokens: size,
        ..Default::default()
    })
}

fn acp_update_status(update: &serde_json::Map<String, Value>) -> Option<String> {
    update
        .get("status")
        .and_then(|v| v.as_str())
        .map(|status| status.trim().to_lowercase().replace([' ', '-'], "_"))
}

fn acp_tool_call_id(update: &serde_json::Map<String, Value>) -> Option<String> {
    update
        .get("toolCallId")
        .or_else(|| update.get("tool_call_id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn acp_tool_name(update: &serde_json::Map<String, Value>) -> String {
    update
        .get("title")
        .or_else(|| update.get("toolName"))
        .or_else(|| update.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("tool")
        .to_string()
}

fn acp_is_terminal_success(status: &str) -> bool {
    matches!(status, "completed" | "complete" | "succeeded" | "success")
}

fn acp_is_terminal_failure(status: &str) -> bool {
    matches!(
        status,
        "failed" | "failure" | "error" | "cancelled" | "canceled"
    )
}

fn acp_result_content(
    update: &serde_json::Map<String, Value>,
    terminals: Option<&AcpTerminalHost>,
) -> Option<String> {
    let value = update
        .get("content")
        .or_else(|| update.get("output"))
        .or_else(|| update.get("result"))?;
    let text = flatten_acp_tool_blocks(value, terminals);
    (!text.is_empty()).then(|| explain_acp_agent_policy_error(text))
}

/// Pull readable text out of ACP `tool_call` content (string, text blocks, or a
/// `{type:"terminal"}` block whose stdout lives on the host).
pub(crate) fn flatten_acp_tool_content(content: &Value) -> String {
    flatten_acp_tool_blocks(content, None)
}

fn flatten_acp_tool_blocks(content: &Value, terminals: Option<&AcpTerminalHost>) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| acp_tool_block_text(item, terminals))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => acp_tool_block_text(content, terminals).unwrap_or_default(),
        _ => String::new(),
    }
}

fn acp_tool_block_text(block: &Value, terminals: Option<&AcpTerminalHost>) -> Option<String> {
    if let Some(text) = block.as_str() {
        return Some(text.to_string());
    }
    match block.get("type").and_then(Value::as_str).unwrap_or("") {
        "terminal" => {
            let id = block
                .get("terminalId")
                .or_else(|| block.get("terminal_id"))
                .and_then(Value::as_str)?;
            terminals
                .and_then(|host| host.preview_output(id))
                .filter(|output| !output.is_empty())
        }
        "content" => block
            .get("content")
            .and_then(|inner| acp_tool_block_text(inner, terminals)),
        "diff" => None,
        _ => block
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Kimi 0.37 在广告 `terminal` 后把全部 spawn 换成宿主终端，又只放行 `bash -c`。
/// 原文看起来像 Kivio 没接好，其实是 agent 策略。
pub(crate) fn explain_acp_agent_policy_error<'a>(
    raw: impl Into<std::borrow::Cow<'a, str>>,
) -> String {
    let raw = raw.into();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return raw.into_owned();
    }
    if trimmed.contains("Kimi 在 ACP 下") || trimmed.contains("Kimi 需要宿主提供 ACP 终端")
    {
        return raw.into_owned();
    }
    if trimmed.contains("ACP runtime only supports interactive Bash tool processes") {
        return format!(
            "Kimi 在 ACP 下只用 Bash 执行命令，不会把 Glob/Grep 交给宿主。可用 Bash 查找或搜索。\n\n{trimmed}"
        );
    }
    if trimmed.contains("ACP terminal capability is unavailable") {
        return format!(
            "Kimi 需要宿主提供 ACP 终端才能跑命令。当前会话没有广告该能力，Bash 也会失败。\n\n{trimmed}"
        );
    }
    raw.into_owned()
}

fn apply_acp_session_update(
    update: &serde_json::Map<String, Value>,
    emitted_tool_ids: &mut HashSet<String>,
    sink: &mut impl FnMut(UnifiedAgentEvent),
) -> bool {
    apply_acp_session_update_ex(update, emitted_tool_ids, None, sink)
}

fn apply_acp_session_update_ex(
    update: &serde_json::Map<String, Value>,
    emitted_tool_ids: &mut HashSet<String>,
    terminals: Option<&AcpTerminalHost>,
    sink: &mut impl FnMut(UnifiedAgentEvent),
) -> bool {
    let session_update = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match session_update {
        "tool_call" => {
            let Some(id) = acp_tool_call_id(update) else {
                return true;
            };
            if emitted_tool_ids.insert(id.clone()) {
                sink(UnifiedAgentEvent::ToolUse {
                    id,
                    name: acp_tool_name(update),
                    input: Value::Object(update.clone()),
                });
            }
            true
        }
        "tool_call_update" => {
            let Some(id) = acp_tool_call_id(update) else {
                return true;
            };
            if emitted_tool_ids.insert(id.clone())
                || update.get("rawInput").is_some_and(|input| !input.is_null())
                || ["title", "toolName", "name"].iter().any(|key| {
                    update
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.contains("html_render"))
                })
            {
                sink(UnifiedAgentEvent::ToolUse {
                    id: id.clone(),
                    name: acp_tool_name(update),
                    input: Value::Object(update.clone()),
                });
            }
            let status = acp_update_status(update);
            let is_error = status.as_deref().is_some_and(acp_is_terminal_failure);
            if is_error || status.as_deref().is_some_and(acp_is_terminal_success) {
                sink(UnifiedAgentEvent::ToolResult {
                    tool_use_id: id,
                    content: acp_result_content(update, terminals)
                        .unwrap_or_else(|| acp_tool_name(update)),
                    is_error,
                });
            } else if let Some(output) = acp_result_content(update, terminals) {
                sink(UnifiedAgentEvent::ToolPartialOutput {
                    tool_use_id: id,
                    output,
                });
            }
            true
        }
        "usage_update" => {
            // 有状态的分发器会先匹配 usage_update，以便保留本轮最近的上下文。
            if let Some(usage) = parse_acp_usage_update(update) {
                sink(UnifiedAgentEvent::Usage { usage });
            }
            true
        }
        _ => false,
    }
}

/// 按消息边界维护助手正文/思考的累积游标,替代旧的全局 `emitted_text` 前缀裁剪(缺陷 2 / N7)。
///
/// 上游三种语义统一在一个分支里:
/// - 纯增量 delta(每条 chunk 是新片段)——`starts_with(current)` 除首条(current 为空)外不命中,
///   整段作为 delta 追加。
/// - 按消息累积快照(chunk 以本消息已发文本为前缀)——前缀裁剪出增量。
/// - 整轮累积快照(旧行为:快照以整轮全文为前缀)——边界事件后快照仍以旧前缀开头,`on_boundary`
///   置位的 `boundary_pending` 因 `starts_with` 命中而不触发重置,行为与旧全局前缀裁剪完全一致。
#[derive(Default)]
struct AcpTextAssembler {
    current: String,
    boundary_pending: bool,
}

impl AcpTextAssembler {
    /// 见到 tool_call / thought 等边界事件时置位。只置位,不立即清空——由下一条 chunk 的
    /// `starts_with` 检查决定是否真是新消息起点(保证整轮累积语义向后兼容)。
    fn on_boundary(&mut self) {
        self.boundary_pending = true;
    }

    /// 返回本条 chunk 应发出的增量;无新增内容时返回 `None`。
    fn push_chunk(&mut self, text: &str) -> Option<String> {
        // 边界后的第一条 chunk 若不再以当前消息累积文本为前缀,视为新消息:重置游标。
        if self.boundary_pending && !text.starts_with(self.current.as_str()) {
            self.current.clear();
        }
        self.boundary_pending = false;
        let delta = if text.starts_with(self.current.as_str()) {
            text[self.current.len()..].to_string()
        } else {
            text.to_string()
        };
        if delta.is_empty() {
            return None;
        }
        self.current.push_str(&delta);
        Some(delta)
    }
}

/// 一次 ACP turn 的文本游标、已发工具 id 和最近的上下文快照。
#[derive(Default)]
struct AcpUpdateState {
    text: AcpTextAssembler,
    thought: AcpTextAssembler,
    emitted_tools: HashSet<String>,
    context_tokens: Option<u64>,
}

fn acp_update_text(update: &serde_json::Map<String, Value>) -> &str {
    update
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

fn map_acp_plan_entries(raw: Option<&Value>) -> Vec<Value> {
    raw.and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let content = entry.get("content")?.as_str()?.trim();
                    let content = if content.is_empty() {
                        format!("Step {}", index + 1)
                    } else {
                        content.to_string()
                    };
                    let status = match entry.get("status").and_then(Value::as_str) {
                        Some("in_progress") => "in_progress",
                        Some("completed") | Some("cancelled") => "completed",
                        _ => "pending",
                    };
                    Some(json!({
                        "id": format!("acp-plan-{}", index + 1),
                        "content": content,
                        "status": status,
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 把一条 ACP `session/update` 映射为事件并维护本轮去重与上下文状态。
fn acp_apply_session_update(
    update: &serde_json::Map<String, Value>,
    state: &mut AcpUpdateState,
    sink: &mut dyn FnMut(UnifiedAgentEvent),
) {
    acp_apply_session_update_ex(update, state, None, sink);
}

fn acp_apply_session_update_ex(
    update: &serde_json::Map<String, Value>,
    state: &mut AcpUpdateState,
    terminals: Option<&AcpTerminalHost>,
    sink: &mut dyn FnMut(UnifiedAgentEvent),
) {
    let session_update = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match session_update {
        "agent_thought_chunk" => {
            // 思考块开始意味着上一条正文消息结束——为正文游标标记边界。
            state.text.on_boundary();
            let text = acp_update_text(update);
            if !text.is_empty() {
                if let Some(delta) = state.thought.push_chunk(text) {
                    sink(UnifiedAgentEvent::ThinkingDelta { delta });
                }
            }
        }
        "agent_message_chunk" => {
            // 正文块开始意味着上一条思考消息结束——为思考游标标记边界。
            state.thought.on_boundary();
            let text = acp_update_text(update);
            if !text.is_empty() {
                if let Some(delta) = state.text.push_chunk(text) {
                    sink(UnifiedAgentEvent::TextDelta { delta });
                }
            }
        }
        // Plan snapshots do not interrupt the current text or thought stream.
        "plan" => {
            let todos = map_acp_plan_entries(update.get("entries"));
            sink(UnifiedAgentEvent::TodoWrite {
                todos: json!({ "todos": todos }),
            });
        }
        "plan_update" => {
            if let Some(entries) = update
                .get("plan")
                .filter(|plan| plan.get("type").and_then(Value::as_str) == Some("items"))
                .and_then(|plan| plan.get("entries"))
                .filter(|entries| entries.is_array())
            {
                let todos = map_acp_plan_entries(Some(entries));
                sink(UnifiedAgentEvent::TodoWrite {
                    todos: json!({ "todos": todos }),
                });
            }
        }
        "plan_removed" => sink(UnifiedAgentEvent::TodoWrite {
            todos: json!({ "todos": [] }),
        }),
        "usage_update" => {
            // **必须在 `_` 分支之前单独匹配**：usage 通知不是消息边界。若落进 `_`，
            // `apply_acp_session_update` 返回 true 会触发 text/thought 的 `on_boundary()`，
            // 正在流式的正文游标被重置后，后续累积快照的 starts_with 判断失效
            // → 整段正文重复发一遍。两处分发共用 `parse_acp_usage_update`（不是两份拷贝）。
            if let Some(mut usage) = parse_acp_usage_update(update) {
                if usage.context_tokens.is_some() {
                    state.context_tokens = usage.context_tokens;
                } else {
                    usage.context_tokens = state.context_tokens;
                }
                sink(UnifiedAgentEvent::Usage { usage });
            }
        }
        _ => {
            // tool_call / tool_call_update 是消息边界:发出工具事件后,重置正文与思考游标,
            // 使其后到来的累积快照被识别为新消息起点。
            if apply_acp_session_update_ex(update, &mut state.emitted_tools, terminals, &mut |e| {
                sink(e)
            }) {
                state.text.on_boundary();
                state.thought.on_boundary();
            }
        }
    }
}

/// grok 的 `retry_state` 通知 → 状态行上的一句短话（不进正文）。
///
/// **为什么必须接**：上游 503 / 429 时 grok 在**静默重试**（实测 `max_retries: 15`，退避到
/// 后面单次要等半分钟），界面上一个字都没有 —— 与 claude 的 `api_retry` 完全同一类问题，
/// 复用同一个 `StatusNote` 出口和同一套文案格式。这条通知走的是厂商私有方法
/// `_x.ai/session_notification`，不是标准 `session/update`。
///
/// 形状（grok 1.0.3 实测）：
/// `{sessionUpdate:"retry_state", type:"retrying", attempt:1, max_retries:15, reason:"API error …"}`
/// `type` 非 `retrying`（重试结束）时不发 —— 状态行是瞬态的，本轮继续流就自然被覆盖。
fn acp_retry_state_note(update: &serde_json::Map<String, Value>) -> Option<String> {
    if update.get("sessionUpdate").and_then(|v| v.as_str())? != "retry_state" {
        return None;
    }
    if update.get("type").and_then(|v| v.as_str()) != Some("retrying") {
        return None;
    }
    let attempt = update.get("attempt").and_then(|v| v.as_u64())?;
    let of_max = update
        .get("max_retries")
        .and_then(|v| v.as_u64())
        .map(|max| format!("/{max}"))
        .unwrap_or_default();
    let cause = update
        .get("reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        // reason 是一整句英文错误（含 body 片段），状态行放不下也没必要 —— 截到能认出
        // 「是什么故障」为止。
        .map(|s| head_chars(s, 80));
    Some(match cause {
        Some(text) => format!("retry {attempt}{of_max} · {text}"),
        None => format!("retry {attempt}{of_max}"),
    })
}

/// 取前 n 个字符（按字符而非字节，避免切裂多字节）。
fn head_chars(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let head: String = s.chars().take(n).collect();
    format!("{head}…")
}

// ===========================================================================================
// Persistent ACP session: keep the agent process alive across turns.
// ===========================================================================================

/// A live ACP connection: one `session/new` (or `session/load`) + `set_model`, then many
/// `session/prompt` turns over the same process. Owned exclusively by its actor task.
pub struct AcpSession {
    child: Child,
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    session_id: String,
    next_id: u64,
    /// Ring-buffered stderr tail (N1): drained for the process lifetime, joined on close / error
    /// so a silent handshake/turn failure surfaces the CLI's stderr.
    stderr_tail: tokio::task::JoinHandle<String>,
    /// The `configOptions` id for the model selector, if this agent exposes one (used by
    /// `session/set_config_option`); `None` → fall back to `session/set_model`.
    model_config_id: Option<String>,
    /// The `configOptions` id for reasoning/thinking level, if any. `None` (e.g. grok, whose
    /// reasoning is a launch flag) → a reasoning change forces a reconnect instead.
    reasoning_config_id: Option<String>,
    /// Normalized model/reasoning the live session currently reflects, for mid-turn change
    /// detection (N3). `None` = agent default.
    current_model: Option<String>,
    current_reasoning: Option<String>,
    terminals: AcpTerminalHost,
}

/// Sentinel returned by `run_turn` when a config change (reasoning without a config option) can
/// only take effect by relaunching the CLI with new args — `run_persistent_turn` reconnects fresh.
pub const NEEDS_RECONNECT: &str = "__needs_reconnect__";

fn normalize_opt(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "default")
        .map(str::to_string)
}

/// Scan an ACP `session/new` result's `configOptions` for the model + reasoning selector ids.
fn find_config_ids(result: &Value) -> (Option<String>, Option<String>) {
    let mut model_id = None;
    let mut reasoning_id = None;
    if let Some(config_options) = result.get("configOptions").and_then(|v| v.as_array()) {
        for raw in config_options {
            let Some(option) = raw.as_object() else {
                continue;
            };
            let id = option.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let category = option
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let id_l = id.to_lowercase();
            if model_id.is_none() && (id == "model" || category == "model") {
                model_id = Some(id.to_string());
            } else if reasoning_id.is_none()
                && (id_l.contains("reasoning")
                    || id_l.contains("thought")
                    || id_l.contains("thinking")
                    || id_l.contains("effort")
                    || category == "reasoning"
                    || category == "thought"
                    || category == "thought_level")
            {
                reasoning_id = Some(id.to_string());
            }
        }
    }
    (model_id, reasoning_id)
}

/// Build the ACP request to switch the session model: `set_config_option` when the agent exposes a
/// model config id, else `set_model`. Pure so the per-turn model-switch (N3) is unit-testable.
fn model_set_rpc(
    session_id: &str,
    model_config_id: Option<&str>,
    chosen: &str,
) -> (&'static str, Value) {
    match model_config_id {
        Some(cfg) => (
            "session/set_config_option",
            json!({ "sessionId": session_id, "configId": cfg, "value": chosen }),
        ),
        None => (
            "session/set_model",
            json!({ "sessionId": session_id, "modelId": chosen }),
        ),
    }
}

/// 会话模式（OpenCode 的 build / plan）。T3 `AcpSessionRuntime.setMode` 同款：优先
/// `category:"mode"` 的 select configOption（也兼容缺失 type），否则 ACP v1 的
/// `modes` → `session/set_mode`。只发 CLI 自报过的取值：其他 ACP CLI 的 sandbox 档
/// （grok 的 ask / strict）不会被误当成模式发出去；已是该模式也不发。
fn mode_set_rpc(
    session_id: &str,
    result: &Value,
    mode: Option<&str>,
) -> Option<(&'static str, Value)> {
    let mode = mode.map(str::trim).filter(|m| !m.is_empty())?;
    let offers = |list: Option<&Value>, key: &str| {
        list.and_then(Value::as_array).is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get(key).and_then(Value::as_str) == Some(mode))
        })
    };
    let config = result
        .get("configOptions")
        .and_then(Value::as_array)
        .and_then(|options| {
            options.iter().find(|option| {
                option.get("category").and_then(Value::as_str) == Some("mode")
                    && option
                        .get("type")
                        .map_or(true, |kind| kind.as_str() == Some("select"))
            })
        });
    if let Some(option) = config {
        let id = option.get("id").and_then(Value::as_str)?;
        if option.get("currentValue").and_then(Value::as_str) == Some(mode)
            || !offers(option.get("options"), "value")
        {
            return None;
        }
        return Some((
            "session/set_config_option",
            json!({ "sessionId": session_id, "configId": id, "value": mode }),
        ));
    }
    let modes = result.get("modes")?;
    if modes.get("currentModeId").and_then(Value::as_str) == Some(mode)
        || !offers(modes.get("availableModes"), "id")
    {
        return None;
    }
    Some((
        "session/set_mode",
        json!({ "sessionId": session_id, "modeId": mode }),
    ))
}

/// What a mid-turn reasoning change needs (N3): nothing, an in-session `set_config_option`, or a
/// full relaunch (agents whose reasoning is a launch flag, e.g. grok).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReasoningAction {
    NoChange,
    SetConfig { config_id: String, value: String },
    Reconnect,
}

fn reasoning_action(
    current: &Option<String>,
    desired: &Option<String>,
    config_id: &Option<String>,
) -> ReasoningAction {
    if current == desired {
        return ReasoningAction::NoChange;
    }
    match config_id {
        Some(cfg) => ReasoningAction::SetConfig {
            config_id: cfg.clone(),
            value: desired.clone().unwrap_or_else(|| "default".to_string()),
        },
        None => ReasoningAction::Reconnect,
    }
}

impl AcpSession {
    #[allow(clippy::too_many_arguments)]
    pub async fn connect(
        resolved_bin: &Path,
        args: &[String],
        cwd: &Path,
        model: Option<&str>,
        reasoning: Option<&str>,
        // 会话模式（OpenCode build / plan），取自权限胶囊；CLI 没自报该值时忽略。
        mode: Option<&str>,
        mcp_servers: &[AcpMcpServer],
        resume_session: Option<&str>,
        additional_directories: &[String],
    ) -> Result<Self, String> {
        let mut command = crate::external_agents::spawn::cli_command(resolved_bin);
        crate::external_agents::spawn::bypass_loopback_proxy(&mut command);
        let mut child = command
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .no_console_window()
            .kill_on_drop(true)
            .spawn()
            .map(crate::external_agents::spawn::AgentChild::from)
            .map_err(|e| format!("spawn: {e}"))?;
        // N1: drain stderr for the process lifetime.
        let stderr_tail = crate::external_agents::spawn::spawn_stderr_tail(child.stderr.take());
        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let tail = join_tail(&mut child, stderr_tail).await;
                return Err(fold_stderr("spawn: stdin unavailable".to_string(), &tail));
            }
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let tail = join_tail(&mut child, stderr_tail).await;
                return Err(fold_stderr("spawn: stdout unavailable".to_string(), &tail));
            }
        };
        let mut reader = BufReader::new(stdout).lines();
        let mut terminals = AcpTerminalHost::new(cwd.to_path_buf());

        // Fallible handshake, isolated so an error path can kill the child and fold in stderr.
        let handshake = async {
            write_rpc(&mut stdin, 1, "initialize", acp_initialize_params(true))
                .await
                .map_err(|e| format!("initialize: {e}"))?;
            let init_result = acp_read_until_id(
                &mut reader,
                &mut stdin,
                &mut terminals,
                1,
                ACP_INITIALIZE_TIMEOUT,
            )
            .await
            .map_err(|e| format!("initialize: {e}"))?;
            let mcp_servers = supported_mcp_servers(
                mcp_servers,
                &init_result,
                crate::external_agents::wsl::is_wsl_target(resolved_bin),
            );
            let additional_directories = agent_supports_additional_directories(&init_result)
                .then_some(additional_directories);

            // session/new for a fresh session, session/load to resume a prior one.
            // One Kivio conversation ↔ one native session id: load failure must
            // surface (caller only starts fresh when the CLI says the id is gone).
            let mut next_id: u64 = 2;
            let (result, session_id) = match resume_session.filter(|s| !s.is_empty()) {
                Some(sid) => {
                    let mut last_err = None;
                    let mut loaded = None;
                    for cwd_try in
                        crate::external_agents::wsl::cwd_spellings_for_cli(resolved_bin, cwd)
                    {
                        let params = build_session_load_params(
                            &cwd_try,
                            sid,
                            &mcp_servers,
                            additional_directories,
                        );
                        write_rpc(&mut stdin, next_id, "session/load", params)
                            .await
                            .map_err(|e| format!("session/load: {e}"))?;
                        match acp_read_until_id(
                            &mut reader,
                            &mut stdin,
                            &mut terminals,
                            next_id,
                            ACP_SESSION_NEW_TIMEOUT,
                        )
                        .await
                        {
                            Ok(result) => {
                                loaded = Some(result);
                                next_id += 1;
                                break;
                            }
                            Err(err) if is_missing_acp_session_error(&err) => {
                                last_err = Some(format!("session/load: {err}"));
                                next_id += 1;
                            }
                            Err(err) => {
                                return Err(format!("session/load: {err}"));
                            }
                        }
                    }
                    let result = loaded.ok_or_else(|| {
                        last_err.unwrap_or_else(|| format!("session/load: session {sid} not found"))
                    })?;
                    (result, sid.to_string())
                }
                None => {
                    write_rpc(
                        &mut stdin,
                        next_id,
                        "session/new",
                        build_session_new_params(
                            &cli_protocol_cwd(resolved_bin, cwd),
                            &mcp_servers,
                            additional_directories,
                        ),
                    )
                    .await
                    .map_err(|e| format!("session-new: {e}"))?;
                    let result = acp_read_until_id(
                        &mut reader,
                        &mut stdin,
                        &mut terminals,
                        next_id,
                        ACP_SESSION_NEW_TIMEOUT,
                    )
                    .await
                    .map_err(|e| format!("session-new: {e}"))?;
                    next_id += 1;
                    let session_id = result
                        .get("sessionId")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .ok_or_else(|| "session-new: invalid session/new response".to_string())?;
                    (result, session_id)
                }
            };

            terminals.set_session_id(session_id.clone());

            let (model_config_id, reasoning_config_id) = find_config_ids(&result);

            // Optional model selection (set_config_option / set_model), mirroring run_acp_session.
            let chosen_model = normalize_opt(model);
            if let Some(chosen) = chosen_model.as_deref() {
                let (set_method, set_params) =
                    model_set_rpc(&session_id, model_config_id.as_deref(), chosen);
                write_rpc(&mut stdin, next_id, set_method, set_params)
                    .await
                    .map_err(|e| format!("session-new: {e}"))?;
                // Best-effort: wait for the ack but don't fail the session if the agent ignores it.
                let _ = acp_read_until_id(
                    &mut reader,
                    &mut stdin,
                    &mut terminals,
                    next_id,
                    Duration::from_secs(10),
                )
                .await;
                next_id += 1;
            }
            if let Some((set_method, set_params)) = mode_set_rpc(&session_id, &result, mode) {
                write_rpc(&mut stdin, next_id, set_method, set_params)
                    .await
                    .map_err(|e| format!("session-new: {e}"))?;
                // Best-effort like the model switch: an ignored mode keeps the CLI default.
                let _ = acp_read_until_id(
                    &mut reader,
                    &mut stdin,
                    &mut terminals,
                    next_id,
                    Duration::from_secs(10),
                )
                .await;
                next_id += 1;
            }

            Ok::<_, String>((
                session_id,
                next_id,
                model_config_id,
                reasoning_config_id,
                chosen_model,
            ))
        }
        .await;

        match handshake {
            Ok((session_id, next_id, model_config_id, reasoning_config_id, current_model)) => {
                // In-session effort config (OpenCode / kimi): start as "not yet applied" so the
                // first `run_turn` sends the chosen level. Launch-flag effort (grok) is already
                // in argv, so it counts as applied.
                let current_reasoning = if reasoning_config_id.is_some() {
                    None
                } else {
                    normalize_opt(reasoning)
                };
                Ok(Self {
                    child,
                    stdin,
                    reader,
                    session_id,
                    next_id,
                    stderr_tail,
                    model_config_id,
                    reasoning_config_id,
                    current_model,
                    current_reasoning,
                    terminals,
                })
            }
            Err(msg) => {
                let tail = join_tail(&mut child, stderr_tail).await;
                Err(fold_stderr(msg, &tail))
            }
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 常驻子进程的 pid。只作为注册表元数据（诊断 / 「两轮是不是同一个进程」），
    /// 关停一律走 actor 的 `Close`，绝不按 pid 杀。
    pub fn child_pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Run one prompt turn over the live session. Emits events into `events`; an incoming
    /// `Cancel` on `control` sends `session/cancel` without killing the process.
    ///
    /// `model`/`reasoning` are re-applied per turn (N3): a model change goes through
    /// `session/set_config_option` / `session/set_model` in-session; a reasoning change with no
    /// config option (grok's launch-flag reasoning) returns `Err(NEEDS_RECONNECT)` so the caller
    /// relaunches the CLI with new args.
    pub async fn run_turn(
        &mut self,
        prompt: &str,
        model: Option<&str>,
        reasoning: Option<&str>,
        images: &[crate::external_agents::attachments::ImageBlock],
        extra_writable_roots: &[String],
        events: &mpsc::Sender<UnifiedAgentEvent>,
        control: &mut mpsc::Receiver<SessionCommand>,
        approvals: &mut Option<ApprovalBridge>,
    ) -> Result<(), String> {
        self.terminals.set_extra_roots(extra_writable_roots);
        // Apply mid-session config changes before sending the prompt (N3).
        let desired_reasoning = normalize_opt(reasoning);
        match reasoning_action(
            &self.current_reasoning,
            &desired_reasoning,
            &self.reasoning_config_id,
        ) {
            ReasoningAction::NoChange => {}
            ReasoningAction::SetConfig { config_id, value } => {
                let id = self.next_id;
                self.next_id += 1;
                let _ = write_rpc(
                    &mut self.stdin,
                    id,
                    "session/set_config_option",
                    json!({ "sessionId": self.session_id, "configId": config_id, "value": value }),
                )
                .await;
                let _ = acp_read_until_id(
                    &mut self.reader,
                    &mut self.stdin,
                    &mut self.terminals,
                    id,
                    Duration::from_secs(10),
                )
                .await;
                self.current_reasoning = desired_reasoning;
            }
            // Reasoning is a launch flag (grok) — only a relaunch with new args applies it.
            ReasoningAction::Reconnect => return Err(NEEDS_RECONNECT.to_string()),
        }

        let desired_model = normalize_opt(model);
        if desired_model != self.current_model {
            if let Some(chosen) = desired_model.as_deref() {
                let id = self.next_id;
                self.next_id += 1;
                let (method, params) =
                    model_set_rpc(&self.session_id, self.model_config_id.as_deref(), chosen);
                let _ = write_rpc(&mut self.stdin, id, method, params).await;
                let _ = acp_read_until_id(
                    &mut self.reader,
                    &mut self.stdin,
                    &mut self.terminals,
                    id,
                    Duration::from_secs(10),
                )
                .await;
            }
            self.current_model = desired_model;
        }

        let prompt_id = self.next_id;
        self.next_id += 1;
        write_rpc(
            &mut self.stdin,
            prompt_id,
            "session/prompt",
            json!({
                "sessionId": self.session_id,
                "prompt": acp_prompt_blocks(prompt, images),
            }),
        )
        .await?;

        let mut update_state = AcpUpdateState::default();

        loop {
            match control.try_recv() {
                Ok(SessionCommand::Cancel) => {
                    let cid = self.next_id;
                    self.next_id += 1;
                    let _ = write_rpc(
                        &mut self.stdin,
                        cid,
                        "session/cancel",
                        json!({ "sessionId": self.session_id }),
                    )
                    .await;
                    return Err("cancelled".to_string());
                }
                Ok(SessionCommand::Close) => return Err("closed".to_string()),
                // ACP 没有「往在飞的 prompt 追加输入」的原语 —— 全部动作只有 `session/prompt`
                // 与 `session/cancel`。回 false，这条消息留在前端队列里等轮末自动发送。
                Ok(SessionCommand::Steer { accepted, .. }) => {
                    let _ = accepted.send(false);
                }
                Ok(SessionCommand::RunTurn { done, .. }) => {
                    let _ = done.send(Err("session busy".to_string()));
                }
                // ACP 无后台任务协议（stop_task 是 claude 专属），忽略。
                Ok(SessionCommand::StopTask { .. }) => {}
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err("control channel closed".to_string())
                }
            }

            let value = match acp_next_message(
                &mut self.reader,
                &mut self.stdin,
                &mut self.terminals,
                Duration::from_millis(200),
                approvals.is_some(),
            )
            .await?
            {
                AcpNext::Value(value) => value,
                AcpNext::Eof => return Err("ACP session exited mid-turn".to_string()),
                AcpNext::Idle => continue,
            };

            if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
                if method == "session/update" {
                    if let Some(update) = value
                        .get("params")
                        .and_then(|p| p.get("update"))
                        .and_then(|v| v.as_object())
                    {
                        let mut buf: Vec<UnifiedAgentEvent> = Vec::new();
                        acp_apply_session_update_ex(
                            update,
                            &mut update_state,
                            Some(&self.terminals),
                            &mut |e| buf.push(e),
                        );
                        for e in buf {
                            let _ = events.send(e).await;
                        }
                    }
                    continue;
                }
                // 厂商私有通知。目前只取 grok 的 `retry_state`：上游 503/429 的静默重试是
                // 「怎么卡住了」的头号成因，不接的话界面在整个重试期（可达数分钟）毫无输出。
                // 其余 `_x.ai/*`（mcp 进度 / 队列 / 会话列表变更）与本轮回答无关，继续忽略。
                if method == "_x.ai/session_notification" {
                    if let Some(update) = value
                        .get("params")
                        .and_then(|p| p.get("update"))
                        .and_then(|v| v.as_object())
                    {
                        if let Some(text) = acp_retry_state_note(update) {
                            let _ = events.send(UnifiedAgentEvent::StatusNote { text }).await;
                        }
                    }
                    continue;
                }
                if method.starts_with("cursor/")
                    || method == "session/request_permission"
                    || method == "elicitation/create"
                {
                    handle_cursor_extension(
                        &mut self.stdin,
                        &value,
                        method,
                        events,
                        control,
                        approvals.as_mut(),
                        &mut self.next_id,
                        &self.session_id,
                    )
                    .await?;
                    continue;
                }
                continue;
            }

            if let Some(err) = rpc_error_message(&value) {
                if value.get("id").and_then(|v| v.as_u64()) == Some(prompt_id) {
                    return Err(err);
                }
                continue;
            }

            if value.get("id").and_then(|v| v.as_u64()) == Some(prompt_id) {
                if let Some(usage) = value.get("result").and_then(|result| {
                    usage_from_prompt_result(result, update_state.context_tokens)
                }) {
                    let _ = events.send(UnifiedAgentEvent::Usage { usage }).await;
                }
                return Ok(());
            }
        }
    }

    pub async fn close(mut self) {
        self.terminals.close_all();
        let _ = flush_terminal_side_effects(&mut self.stdin, &mut self.terminals).await;
        let _ = self.stdin.shutdown().await;
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        // Child is dead → its stderr hit EOF → the drain task finishes; join it so the task ends.
        let _ = self.stderr_tail.await;
    }
}

/// Read ACP JSON-RPC lines until the response for `target_id`, auto-answering permission
/// and `terminal/*` requests and skipping notifications.
async fn acp_read_until_id(
    reader: &mut Lines<impl tokio::io::AsyncBufRead + Unpin>,
    stdin: &mut (impl AsyncWrite + Unpin),
    terminals: &mut AcpTerminalHost,
    target_id: u64,
    overall: Duration,
) -> Result<Value, String> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > overall {
            return Err("ACP handshake timeout".to_string());
        }
        match acp_next_message(reader, stdin, terminals, Duration::from_millis(200), false).await? {
            AcpNext::Value(value) => {
                if let Some(method) = value.get("method").and_then(Value::as_str) {
                    if method.starts_with("cursor/") {
                        if let Some(id) = value.get("id") {
                            let params = value.get("params").unwrap_or(&Value::Null);
                            write_rpc_result(stdin, id, cursor_handshake_result(method, params))
                                .await?;
                        }
                        continue;
                    }
                }
                if let Some(err) = rpc_error_message(&value) {
                    if value.get("id").and_then(|v| v.as_u64()) == Some(target_id) {
                        return Err(err);
                    }
                    continue;
                }
                if value.get("id").and_then(|v| v.as_u64()) == Some(target_id) {
                    return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                }
            }
            AcpNext::Eof => return Err("ACP agent exited during handshake".to_string()),
            AcpNext::Idle => {}
        }
    }
}

enum AcpNext {
    Value(Value),
    Idle,
    Eof,
}

async fn acp_next_message(
    reader: &mut Lines<impl tokio::io::AsyncBufRead + Unpin>,
    stdin: &mut (impl AsyncWrite + Unpin),
    terminals: &mut AcpTerminalHost,
    slice: Duration,
    host_permissions: bool,
) -> Result<AcpNext, String> {
    flush_terminal_side_effects(stdin, terminals).await?;
    let line = match timeout(slice, reader.next_line()).await {
        Ok(Ok(Some(raw))) => raw,
        Ok(Ok(None)) => return Ok(AcpNext::Eof),
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => return Ok(AcpNext::Idle),
    };
    if line.trim().is_empty() {
        return Ok(AcpNext::Idle);
    }
    let value: Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(_) => return Ok(AcpNext::Idle),
    };
    if host_permissions {
        let method = value.get("method").and_then(Value::as_str);
        if matches!(
            method,
            Some("session/request_permission" | "elicitation/create")
        ) {
            return Ok(AcpNext::Value(value));
        }
    }
    if handle_agent_to_client_request(&value, stdin, terminals).await? {
        return Ok(AcpNext::Idle);
    }
    Ok(AcpNext::Value(value))
}

async fn flush_terminal_side_effects(
    stdin: &mut (impl AsyncWrite + Unpin),
    terminals: &mut AcpTerminalHost,
) -> Result<(), String> {
    while let Ok((id, exit)) = terminals.exit_rx().try_recv() {
        for (rpc_id, result) in terminals.on_exit(id, exit) {
            write_rpc_result(stdin, &rpc_id, result).await?;
        }
    }
    for (rpc_id, result) in terminals.take_completed_waits() {
        write_rpc_result(stdin, &rpc_id, result).await?;
    }
    for (rpc_id, code, message) in terminals.take_aborted_waits() {
        write_rpc_error(stdin, &rpc_id, code, &message).await?;
    }
    Ok(())
}

fn jsonrpc_id_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

fn acp_json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn cursor_extension_is_blocking(method: &str) -> bool {
    method == "cursor/ask_question"
        || method == "cursor/create_plan"
        || method == "session/request_permission"
        || method == "elicitation/create"
}

fn cursor_cancelled_result() -> Value {
    json!({ "outcome": { "outcome": "cancelled" } })
}

fn host_request_declined_result(method: &str) -> Value {
    if method == "elicitation/create" {
        json!({ "action": "decline" })
    } else {
        cursor_cancelled_result()
    }
}

fn host_request_cancelled_result(method: &str) -> Value {
    if method == "elicitation/create" {
        json!({ "action": "cancel" })
    } else {
        cursor_cancelled_result()
    }
}

/// 未知的带 `id` 请求：回 cancelled 结果而不是 `-32601`。
/// 部分 CLI 把 Method not found 当协议失败，这一轮会挂死。
fn unknown_blocking_rpc_result() -> Value {
    cursor_cancelled_result()
}

fn cursor_handshake_result(method: &str, params: &Value) -> Value {
    if cursor_extension_is_blocking(method) {
        cursor_cancelled_result()
    } else {
        cursor_notification_ack(method, params)
    }
}

fn cursor_notification_ack(method: &str, params: &Value) -> Value {
    match method {
        "cursor/update_todos" => json!({
            "outcome": {
                "outcome": "accepted",
                "todos": params.get("todos").cloned().unwrap_or_else(|| json!([])),
            }
        }),
        "cursor/task" => json!({
            "outcome": {
                "outcome": "completed",
                "agentId": params.get("agentId").cloned(),
                "durationMs": params.get("durationMs").cloned(),
            }
        }),
        "cursor/generate_image" => match acp_json_str(params, "filePath") {
            Some(path) => json!({
                "outcome": { "outcome": "generated", "filePath": path }
            }),
            None => cursor_cancelled_result(),
        },
        _ => cursor_cancelled_result(),
    }
}

fn map_cursor_todos(raw: Option<&Value>) -> Vec<Value> {
    raw.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let content = acp_json_str(item, "content")?;
                    let id = acp_json_str(item, "id").unwrap_or(content);
                    let status = match item.get("status").and_then(Value::as_str) {
                        Some("in_progress") => "in_progress",
                        Some("completed") | Some("cancelled") => "completed",
                        _ => "pending",
                    };
                    Some(json!({ "id": id, "content": content, "status": status }))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn cursor_extension_events(method: &str, params: &Value) -> Vec<UnifiedAgentEvent> {
    match method {
        "cursor/update_todos" => {
            let todos = map_cursor_todos(params.get("todos"));
            if todos.is_empty() {
                Vec::new()
            } else {
                vec![UnifiedAgentEvent::TodoWrite {
                    todos: json!({ "todos": todos }),
                }]
            }
        }
        "cursor/task" => {
            let task_id = acp_json_str(params, "agentId")
                .or_else(|| acp_json_str(params, "toolCallId"))
                .unwrap_or("cursor-task")
                .to_string();
            let preview = acp_json_str(params, "description")
                .or_else(|| acp_json_str(params, "prompt"))
                .unwrap_or("")
                .to_string();
            vec![UnifiedAgentEvent::SubagentProgress {
                task_id,
                status: "completed".to_string(),
                preview,
                steps: Vec::new(),
            }]
        }
        "cursor/generate_image" => {
            let id = acp_json_str(params, "toolCallId")
                .unwrap_or("cursor-image")
                .to_string();
            let path = acp_json_str(params, "filePath").unwrap_or("");
            let mut events = vec![UnifiedAgentEvent::ToolUse {
                id: id.clone(),
                name: "image_generation".to_string(),
                input: params.clone(),
            }];
            if !path.is_empty() {
                events.push(UnifiedAgentEvent::ToolResult {
                    tool_use_id: id,
                    content: path.to_string(),
                    is_error: false,
                });
            }
            events
        }
        _ => Vec::new(),
    }
}

fn cursor_decision_result(method: &str, decision: &ApprovalDecision) -> Value {
    if method == "elicitation/create" {
        return if decision.approved {
            decision
                .updated_input
                .clone()
                .filter(Value::is_object)
                .unwrap_or_else(|| host_request_declined_result(method))
        } else {
            host_request_declined_result(method)
        };
    }
    if method == "cursor/create_plan" {
        return if decision.approved {
            json!({ "outcome": { "outcome": "accepted" } })
        } else {
            json!({ "outcome": { "outcome": "rejected" } })
        };
    }
    if decision.approved {
        decision
            .updated_input
            .clone()
            .filter(Value::is_object)
            .unwrap_or_else(cursor_cancelled_result)
    } else {
        json!({ "outcome": { "outcome": "skipped" } })
    }
}

/// 用户批准 → ACP 选项。「允许」→ `allow_once`；「总是允许」→ `allow_always`（T3
/// `acceptForSession` → `allow_always` 同款），CLI 没给 always 时退回 once。绝不把一次
/// 「允许」升级成持久授权：没有 once 选项就取消。
fn acp_permission_result(params: &Value, approved: bool, always: bool) -> Value {
    let options = params.get("options").and_then(Value::as_array);
    let pick = |kind: &str| {
        options?
            .iter()
            .find(|option| option.get("kind").and_then(Value::as_str) == Some(kind))?
            .get("optionId")
            .and_then(Value::as_str)
    };
    let option = match (approved, always) {
        (false, _) => None,
        (true, true) => pick("allow_always").or_else(|| pick("allow_once")),
        (true, false) => pick("allow_once"),
    };
    match option {
        Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
        None => cursor_cancelled_result(),
    }
}

async fn handle_cursor_extension(
    stdin: &mut (impl AsyncWrite + Unpin),
    value: &Value,
    method: &str,
    events: &mpsc::Sender<UnifiedAgentEvent>,
    control: &mut mpsc::Receiver<SessionCommand>,
    approvals: Option<&mut ApprovalBridge>,
    next_id: &mut u64,
    session_id: &str,
) -> Result<(), String> {
    let params = value.get("params").cloned().unwrap_or(Value::Null);
    for event in cursor_extension_events(method, &params) {
        let _ = events.send(event).await;
    }
    let Some(id) = value.get("id") else {
        return Ok(());
    };
    if !cursor_extension_is_blocking(method) {
        return write_rpc_result(stdin, id, cursor_notification_ack(method, &params)).await;
    }

    let Some(bridge) = approvals else {
        return write_rpc_result(stdin, id, host_request_declined_result(method)).await;
    };
    let request_id = jsonrpc_id_key(id);
    let permission = method == "session/request_permission";
    let tool = params.get("toolCall").unwrap_or(&params);
    let tool_call_id = acp_json_str(tool, "toolCallId")
        .or_else(|| acp_json_str(&params, "elicitationId"))
        .or_else(|| acp_json_str(&params, "elicitation_id"))
        .map(str::to_string)
        .unwrap_or_else(|| format!("cursor-{request_id}"));
    let tool_name = if method == "elicitation/create" {
        "elicitation/create"
    } else if permission {
        acp_json_str(tool, "title").unwrap_or("ACP tool")
    } else if method == "cursor/create_plan" {
        "cursor/create_plan"
    } else {
        "cursor/ask_question"
    };
    let ask = ApprovalAsk {
        request_id: request_id.clone(),
        tool_call_id,
        tool_name: tool_name.to_string(),
        input: if permission {
            tool.clone()
        } else {
            params.clone()
        },
        requires_user_interaction: method == "cursor/ask_question"
            || method == "elicitation/create",
        requires_manual_approval: false,
        permission_suggestions: None,
    };
    if bridge.requests.send(ask).await.is_err() {
        return write_rpc_result(stdin, id, host_request_declined_result(method)).await;
    }
    loop {
        match control.try_recv() {
            Ok(SessionCommand::Cancel) => {
                let _ = write_rpc_result(stdin, id, host_request_cancelled_result(method)).await;
                let cid = *next_id;
                *next_id += 1;
                let _ = write_rpc(
                    stdin,
                    cid,
                    "session/cancel",
                    json!({ "sessionId": session_id }),
                )
                .await;
                return Err("cancelled".to_string());
            }
            Ok(SessionCommand::Close) => {
                let _ = write_rpc_result(stdin, id, host_request_cancelled_result(method)).await;
                return Err("closed".to_string());
            }
            Ok(SessionCommand::RunTurn { done, .. }) => {
                let _ = done.send(Err("session busy".to_string()));
            }
            Ok(SessionCommand::Steer { accepted, .. }) => {
                let _ = accepted.send(false);
            }
            Ok(SessionCommand::StopTask { .. }) => {}
            Err(mpsc::error::TryRecvError::Empty) => {}
            Err(mpsc::error::TryRecvError::Disconnected) => {
                let _ = write_rpc_result(stdin, id, host_request_cancelled_result(method)).await;
                return Err("control channel closed".to_string());
            }
        }
        match timeout(Duration::from_millis(200), bridge.decisions.recv()).await {
            Ok(Some(decision)) if decision.request_id == request_id => {
                let result = if permission {
                    acp_permission_result(&params, decision.approved, decision.always)
                } else {
                    cursor_decision_result(method, &decision)
                };
                return write_rpc_result(stdin, id, result).await;
            }
            Ok(Some(_)) | Err(_) => continue,
            Ok(None) => {
                return write_rpc_result(stdin, id, host_request_cancelled_result(method)).await;
            }
        }
    }
}

async fn handle_agent_to_client_request(
    value: &Value,
    stdin: &mut (impl AsyncWrite + Unpin),
    terminals: &mut AcpTerminalHost,
) -> Result<bool, String> {
    let Some(method) = value.get("method").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    if method == "session/request_permission" {
        let option_id =
            choose_permission_outcome(value.get("params").and_then(|p| p.get("options")));
        if let Some(id) = value.get("id") {
            let result = match option_id {
                Some(option_id) => {
                    json!({ "outcome": { "outcome": "selected", "optionId": option_id } })
                }
                None => cursor_cancelled_result(),
            };
            write_rpc_result(stdin, id, result).await?;
        }
        return Ok(true);
    }
    if method == "elicitation/create" {
        if let Some(id) = value.get("id") {
            write_rpc_result(stdin, id, host_request_declined_result(method)).await?;
        }
        return Ok(true);
    }
    if method.starts_with("terminal/") {
        let Some(id) = value.get("id") else {
            return Ok(true);
        };
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        match terminals.handle(method, &params, id) {
            TerminalReply::Result(result) => write_rpc_result(stdin, id, result).await?,
            TerminalReply::Pending => {}
            TerminalReply::Error { code, message } => {
                write_rpc_error(stdin, id, code, &message).await?;
            }
        }
        flush_terminal_side_effects(stdin, terminals).await?;
        return Ok(true);
    }
    if method == "session/update"
        || method == "_x.ai/session_notification"
        || method.starts_with("cursor/")
    {
        return Ok(false);
    }
    if let Some(id) = value.get("id") {
        write_rpc_result(stdin, id, unknown_blocking_rpc_result()).await?;
        return Ok(true);
    }
    Ok(true)
}

/// Build the ACP `session/prompt` content array: text block first, then a native image block
/// (`{type:"image", data, mimeType}`) per attached image. Empty `images` → just the text block.
fn acp_prompt_blocks(
    prompt: &str,
    images: &[crate::external_agents::attachments::ImageBlock],
) -> Vec<serde_json::Value> {
    let mut blocks = vec![json!({ "type": "text", "text": prompt })];
    for img in images {
        blocks.push(json!({
            "type": "image",
            "data": img.data_base64,
            "mimeType": img.mime,
        }));
    }
    blocks
}

/// Spawn the actor task owning a connected ACP session.
pub fn spawn_acp_session_actor(mut session: AcpSession) -> mpsc::Sender<SessionCommand> {
    let (tx, mut rx) = mpsc::channel::<SessionCommand>(8);
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                SessionCommand::RunTurn {
                    prompt,
                    model,
                    reasoning,
                    images,
                    events,
                    done,
                    // cursor-agent 的 `cursor/ask_question` / `cursor/create_plan` 是阻塞
                    // JSON-RPC：必须回结果，否则这一轮挂死。通道由 `ask_user::needs_host`
                    // 打开；其它 ACP CLI 仍是 `None`。
                    mut approvals,
                    extra_writable_roots,
                } => {
                    // Invariant (A4): `run_turn` sends all its `events` before returning, and mpsc
                    // preserves order, so every event is already queued when `done` fires — the
                    // caller's post-`done` `try_recv` drain sees them all. Keep `done.send` LAST.
                    let result = session
                        .run_turn(
                            &prompt,
                            model.as_deref(),
                            reasoning.as_deref(),
                            &images,
                            &extra_writable_roots,
                            &events,
                            &mut rx,
                            &mut approvals,
                        )
                        .await;
                    let _ = done.send(result);
                }
                // 轮次之间没有可注入的对象：回 false 让前端把这条留在队列里、
                // 轮末按普通消息发出去（绝不静默吞掉）。
                SessionCommand::Steer { accepted, .. } => {
                    let _ = accepted.send(false);
                }
                SessionCommand::Cancel => {}
                // ACP 无后台任务协议，忽略。
                SessionCommand::StopTask { .. } => {}
                SessionCommand::Close => {
                    session.close().await;
                    return;
                }
            }
        }
        session.close().await;
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handshake_declines_elicitation_before_reading_its_target_response() {
        // Exercise the production dispatch chain, not cursor_handshake_result:
        // acp_read_until_id -> acp_next_message(host_permissions=false)
        // -> handle_agent_to_client_request. A peer may reuse our numeric id
        // for its request; that request must not be mistaken for our response.
        for request_id in [json!(7), json!("elicitation-7")] {
            let (host, agent) = tokio::io::duplex(4096);
            let (host_read, mut host_write) = tokio::io::split(host);
            let (agent_read, mut agent_write) = tokio::io::split(agent);
            let mut reader = BufReader::new(host_read).lines();
            let mut responses = BufReader::new(agent_read).lines();
            let mut terminals = AcpTerminalHost::new(std::env::temp_dir());
            let handshake = acp_read_until_id(
                &mut reader,
                &mut host_write,
                &mut terminals,
                7,
                Duration::from_secs(2),
            );
            let peer = async {
                let request = json!({
                    "jsonrpc": "2.0", "id": request_id, "method": "elicitation/create",
                    "params": {
                        "mode": "form", "message": "Select environment",
                        "requestedSchema": {"type": "object", "properties": {}}
                    }
                });
                agent_write
                    .write_all(format!("{request}\n").as_bytes())
                    .await
                    .unwrap();
                let line = responses.next_line().await.unwrap().unwrap();
                let response: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(
                    response,
                    json!({
                        "jsonrpc": "2.0", "id": request_id, "result": {"action": "decline"}
                    })
                );
                // The agent only completes initialize after receiving decline.
                write_rpc_result(&mut agent_write, &json!(7), json!({"protocolVersion": 1}))
                    .await
                    .unwrap();
            };
            let (result, ()) = timeout(Duration::from_secs(3), async {
                tokio::join!(handshake, peer)
            })
            .await
            .expect("handshake must answer elicitation without an interaction host");
            assert_eq!(result.unwrap(), json!({"protocolVersion": 1}));
            drop(reader);
            drop(host_write);
            assert!(
                responses.next_line().await.unwrap().is_none(),
                "respond exactly once"
            );
        }
    }

    #[tokio::test]
    async fn live_elicitation_dispatch_reaches_host_without_handshake_decline() {
        let (host, mut agent) = tokio::io::duplex(4096);
        let (host_read, mut host_write) = tokio::io::split(host);
        let mut reader = BufReader::new(host_read).lines();
        let mut terminals = AcpTerminalHost::new(std::env::temp_dir());
        let request = json!({
            "jsonrpc": "2.0", "id": "elicitation-live", "method": "elicitation/create",
            "params": {"mode": "form", "requestedSchema": {"type": "object", "properties": {}}}
        });
        agent
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let message = acp_next_message(
            &mut reader,
            &mut host_write,
            &mut terminals,
            Duration::from_secs(1),
            true,
        )
        .await
        .unwrap();
        assert!(matches!(message, AcpNext::Value(value) if value == request));
        drop(reader);
        drop(host_write);
        assert!(BufReader::new(agent)
            .lines()
            .next_line()
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn permission_request_waits_for_host_and_echoes_wire_id() {
        for approved in [true, false] {
            let (mut writer, reader) = tokio::io::duplex(4096);
            let (asks, mut requests) = mpsc::channel(1);
            let (decisions, replies) = mpsc::channel(1);
            let mut bridge = ApprovalBridge {
                requests: asks,
                decisions: replies,
            };
            let (_commands, mut control) = mpsc::channel(1);
            let (events, _events_rx) = mpsc::channel(1);
            let value = json!({"id": 42, "method": "session/request_permission", "params": {
                "toolCall": {"toolCallId": "tool-7", "title": "Write file", "rawInput": {"path": "a.txt"}},
                "options": [{"kind": "allow_once", "optionId": "approve"}]
            }});
            let mut next_id = 50;
            let handler = handle_cursor_extension(
                &mut writer,
                &value,
                "session/request_permission",
                &events,
                &mut control,
                Some(&mut bridge),
                &mut next_id,
                "session-1",
            );
            let user = async {
                let ask = requests.recv().await.expect("host request");
                assert_eq!(ask.tool_call_id, "tool-7");
                assert_eq!(ask.tool_name, "Write file");
                decisions
                    .send(ApprovalDecision {
                        request_id: ask.request_id,
                        approved,
                        updated_input: None,
                        set_permission_mode: None,
                        updated_permissions: None,
                        always: false,
                    })
                    .await
                    .unwrap();
            };
            let (result, ()) = timeout(Duration::from_secs(2), async {
                tokio::join!(handler, user)
            })
            .await
            .expect("approval must settle");
            result.unwrap();
            let line = BufReader::new(reader)
                .lines()
                .next_line()
                .await
                .unwrap()
                .unwrap();
            let response: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(response["id"], 42);
            assert_eq!(
                response["result"]["outcome"]["outcome"],
                if approved { "selected" } else { "cancelled" }
            );
        }
    }

    #[tokio::test]
    async fn permission_request_cancels_when_host_is_missing_or_turn_is_cancelled() {
        for cancel in [false, true] {
            let (mut writer, reader) = tokio::io::duplex(4096);
            let (asks, _requests) = mpsc::channel(1);
            let (_decisions, replies) = mpsc::channel(1);
            let mut bridge = ApprovalBridge {
                requests: asks,
                decisions: replies,
            };
            let (commands, mut control) = mpsc::channel(1);
            let (events, _events_rx) = mpsc::channel(1);
            if cancel {
                commands.send(SessionCommand::Cancel).await.unwrap();
            }
            let value =
                json!({"id": "req-1", "method": "session/request_permission", "params": {}});
            let mut next_id = 50;
            let result = timeout(
                Duration::from_secs(2),
                handle_cursor_extension(
                    &mut writer,
                    &value,
                    "session/request_permission",
                    &events,
                    &mut control,
                    cancel.then_some(&mut bridge),
                    &mut next_id,
                    "session-1",
                ),
            )
            .await
            .expect("must not hang");
            assert_eq!(result.is_err(), cancel);
            let mut lines = BufReader::new(reader).lines();
            let response: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(response["id"], "req-1");
            assert_eq!(response["result"], cursor_cancelled_result());
            if cancel {
                let response: Value =
                    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(response["method"], "session/cancel");
            }
        }
    }

    #[test]
    fn permission_decisions_never_escalate_to_persistent_grants() {
        let params = json!({"options": [
            {"optionId": "forever", "kind": "allow_always"},
            {"optionId": "once", "kind": "allow_once"}
        ]});
        assert_eq!(
            acp_permission_result(&params, true, false)["outcome"]["optionId"],
            "once"
        );
        assert_eq!(
            acp_permission_result(&params, true, true)["outcome"]["optionId"],
            "forever"
        );
        assert_eq!(
            acp_permission_result(&params, false, true),
            cursor_cancelled_result()
        );
        let always_only = json!({"options": [{"optionId": "forever", "kind": "allow_always"}]});
        assert_eq!(
            acp_permission_result(&always_only, true, false),
            cursor_cancelled_result()
        );
        let once_only = json!({"options": [{"optionId": "once", "kind": "allow_once"}]});
        assert_eq!(
            acp_permission_result(&once_only, true, true)["outcome"]["optionId"],
            "once"
        );
        assert_eq!(
            acp_permission_result(&Value::Null, true, true),
            cursor_cancelled_result()
        );
    }

    #[test]
    fn mode_is_set_only_to_values_the_cli_offers() {
        // OpenCode 2.0.32 `session/new` 实测形状（截取）。
        let opencode = json!({"configOptions": [
            {"id": "model", "category": "model", "currentValue": "opencode/big-pickle", "options": []},
            {"id": "mode", "category": "mode", "currentValue": "build",
             "options": [{"value": "build", "name": "build"}, {"value": "plan", "name": "plan"}]}
        ]});
        assert_eq!(
            mode_set_rpc("s", &opencode, Some("plan")),
            Some((
                "session/set_config_option",
                json!({"sessionId": "s", "configId": "mode", "value": "plan"})
            ))
        );
        assert_eq!(mode_set_rpc("s", &opencode, Some("build")), None);
        assert_eq!(mode_set_rpc("s", &opencode, Some("ask")), None);
        assert_eq!(mode_set_rpc("s", &opencode, None), None);
        let v1 = json!({"modes": {"currentModeId": "default",
            "availableModes": [{"id": "default"}, {"id": "plan"}]}});
        assert_eq!(
            mode_set_rpc("s", &v1, Some("plan")),
            Some((
                "session/set_mode",
                json!({"sessionId": "s", "modeId": "plan"})
            ))
        );
        assert_eq!(mode_set_rpc("s", &json!({}), Some("plan")), None);
    }

    #[test]
    fn mode_config_requires_select_or_missing_type() {
        let mut result = json!({"configOptions": [{
            "id": "wrong", "category": "mode", "type": "boolean",
            "currentValue": "build", "options": [{"value": "plan"}]
        }]});
        assert_eq!(mode_set_rpc("s", &result, Some("plan")), None);
        result["configOptions"][0]["type"] = Value::Null;
        assert_eq!(mode_set_rpc("s", &result, Some("plan")), None);
        result["configOptions"].as_array_mut().unwrap().push(json!({
            "id": "mode", "category": "mode", "type": "select",
            "currentValue": "build", "options": [{"value": "plan"}]
        }));
        assert_eq!(
            mode_set_rpc("s", &result, Some("plan")),
            Some((
                "session/set_config_option",
                json!({"sessionId": "s", "configId": "mode", "value": "plan"})
            ))
        );
        result["configOptions"].as_array_mut().unwrap().pop();
        result["modes"] = json!({
            "currentModeId": "build", "availableModes": [{"id": "plan"}]
        });
        assert_eq!(
            mode_set_rpc("s", &result, Some("plan")),
            Some((
                "session/set_mode",
                json!({"sessionId": "s", "modeId": "plan"})
            ))
        );
    }

    #[test]
    fn acp_plan_update_becomes_todo_card() {
        let update = json!({"sessionUpdate": "plan", "entries": [
            {"content": " 读文件 ", "priority": "high", "status": "in_progress"},
            {"content": " \n ", "status": "completed"},
            {"content": "改代码", "status": "cancelled"},
            {"content": "改代码", "status": "unexpected"}
        ]});
        let mut events = Vec::new();
        acp_apply_session_update(
            update.as_object().unwrap(),
            &mut AcpUpdateState::default(),
            &mut |e| events.push(e),
        );
        let [UnifiedAgentEvent::TodoWrite { todos }] = events.as_slice() else {
            panic!("expected one TodoWrite, got {}", events.len());
        };
        assert_eq!(
            todos["todos"],
            json!([
                {"id": "acp-plan-1", "content": "读文件", "status": "in_progress"},
                {"id": "acp-plan-2", "content": "Step 2", "status": "completed"},
                {"id": "acp-plan-3", "content": "改代码", "status": "completed"},
                {"id": "acp-plan-4", "content": "改代码", "status": "pending"}
            ])
        );
        let state = crate::external_agents::session::dsh_jsonrpc::todo_state_from_write(todos)
            .expect("ACP plan must produce todo state");
        assert_eq!(state.items.len(), 4);
    }

    #[test]
    fn acp_v2_items_plan_maps_entries_and_ignores_other_types() {
        let items = json!({"sessionUpdate": "plan_update", "plan": {
            "type": "items", "planId": "plan-1", "entries": [
                {"content": " Inspect schema ", "status": "completed"},
                {"content": "", "status": "in_progress"},
                {"content": " Ship "},
                null,
                {"content": true, "status": "pending"}
            ]
        }});
        let markdown = json!({"sessionUpdate": "plan_update", "plan": {
            "type": "markdown", "planId": "plan-1", "content": "# Plan"
        }});
        let missing_entries = json!({"sessionUpdate": "plan_update", "plan": {
            "type": "items", "planId": "plan-1"
        }});
        let (_, events) = run_updates(&[
            items.as_object().unwrap().clone(),
            markdown.as_object().unwrap().clone(),
            missing_entries.as_object().unwrap().clone(),
        ]);
        let [UnifiedAgentEvent::TodoWrite { todos }] = events.as_slice() else {
            panic!("expected one TodoWrite, got {}", events.len());
        };
        assert_eq!(
            todos["todos"],
            json!([
                {"id": "acp-plan-1", "content": "Inspect schema", "status": "completed"},
                {"id": "acp-plan-2", "content": "Step 2", "status": "in_progress"},
                {"id": "acp-plan-3", "content": "Ship", "status": "pending"}
            ])
        );
    }

    #[test]
    fn acp_plan_removed_clears_todo_state() {
        let plan = json!({"sessionUpdate": "plan_update", "plan": {
            "type": "items", "planId": "plan-1", "entries": [
                {"content": "Ship", "status": "in_progress"}
            ]
        }});
        let removed = json!({"sessionUpdate": "plan_removed", "planId": "plan-1"});
        let (_, events) = run_updates(&[
            plan.as_object().unwrap().clone(),
            removed.as_object().unwrap().clone(),
        ]);
        let [UnifiedAgentEvent::TodoWrite { todos: before }, UnifiedAgentEvent::TodoWrite { todos }] =
            events.as_slice()
        else {
            panic!("expected plan and removal, got {}", events.len());
        };
        assert_eq!(before["todos"].as_array().unwrap().len(), 1);
        assert_eq!(todos, &json!({"todos": []}));
        let state = crate::external_agents::session::dsh_jsonrpc::todo_state_from_write(todos)
            .expect("empty plan must clear todo state");
        assert!(state.items.is_empty());
    }

    /// grok 上游 503 时的静默重试必须变成一行可见状态。样本取自本机 grok 1.0.3
    /// `_x.ai/session_notification` 的原样 update。
    #[test]
    fn retry_state_becomes_status_note() {
        let update = json!({
            "sessionUpdate": "retry_state",
            "type": "retrying",
            "attempt": 3,
            "max_retries": 15,
            "reason": "API error (status 503 Service Unavailable): api_error: Service temporarily unavailable"
        });
        let note = acp_retry_state_note(update.as_object().unwrap()).expect("retry note");
        assert!(note.starts_with("retry 3/15 · "), "got: {note}");
        assert!(note.contains("503"), "原因要能看出是什么故障: {note}");
    }

    /// 重试结束（非 `retrying`）不发状态行；别的 update 也不能误命中这条路。
    #[test]
    fn retry_state_note_ignores_non_retrying_and_other_updates() {
        let finished = json!({ "sessionUpdate": "retry_state", "type": "succeeded", "attempt": 4 });
        assert!(acp_retry_state_note(finished.as_object().unwrap()).is_none());
        let other = json!({ "sessionUpdate": "model_changed", "model_id": "grok-4.6" });
        assert!(acp_retry_state_note(other.as_object().unwrap()).is_none());
    }

    #[test]
    fn cursor_todos_and_image_emit_existing_event_kinds() {
        let todos = json!({
            "todos": [
                { "id": "1", "content": "Set up", "status": "completed" },
                { "id": "2", "content": "Auth", "status": "in_progress" },
                { "id": "3", "content": "Skip", "status": "cancelled" }
            ]
        });
        let events = cursor_extension_events("cursor/update_todos", &todos);
        let UnifiedAgentEvent::TodoWrite { todos } = &events[0] else {
            panic!("expected TodoWrite");
        };
        assert_eq!(todos["todos"][2]["status"], json!("completed"));

        let task = json!({
            "toolCallId": "call_126",
            "description": "Explore codebase",
            "prompt": "Find auth",
            "subagentType": "explore"
        });
        let events = cursor_extension_events("cursor/task", &task);
        assert!(matches!(
            &events[0],
            UnifiedAgentEvent::SubagentProgress { task_id, preview, .. }
                if task_id == "call_126" && preview == "Explore codebase"
        ));

        let image = json!({
            "toolCallId": "call_127",
            "description": "icon",
            "filePath": "/tmp/icon.png"
        });
        let events = cursor_extension_events("cursor/generate_image", &image);
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::ToolUse { name, .. } if name == "image_generation"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::ToolResult { content, .. } if content == "/tmp/icon.png"
        )));
        assert!(cursor_extension_is_blocking("cursor/ask_question"));
        assert!(cursor_extension_is_blocking("cursor/create_plan"));
        assert!(cursor_extension_is_blocking("elicitation/create"));
        assert!(!cursor_extension_is_blocking("cursor/update_todos"));
        assert_eq!(
            cursor_decision_result(
                "cursor/create_plan",
                &ApprovalDecision {
                    request_id: "1".to_string(),
                    approved: true,
                    updated_input: None,
                    set_permission_mode: None,
                    updated_permissions: None,
                    always: false,
                }
            ),
            json!({ "outcome": { "outcome": "accepted" } })
        );
        assert_eq!(
            unknown_blocking_rpc_result(),
            json!({ "outcome": { "outcome": "cancelled" } })
        );
        assert_eq!(
            cursor_decision_result(
                "elicitation/create",
                &ApprovalDecision {
                    request_id: "e1".to_string(),
                    approved: true,
                    updated_input: Some(json!({
                        "action": "accept",
                        "content": { "environment": "production" }
                    })),
                    set_permission_mode: None,
                    updated_permissions: None,
                    always: false,
                }
            ),
            json!({
                "action": "accept",
                "content": { "environment": "production" }
            })
        );
        assert_eq!(
            host_request_declined_result("elicitation/create"),
            json!({ "action": "decline" })
        );
        assert_eq!(
            host_request_cancelled_result("elicitation/create"),
            json!({ "action": "cancel" })
        );
    }

    #[test]
    fn session_load_params_carry_session_id_and_cwd() {
        let params = build_session_load_params(Path::new("/mnt/e/proj"), "sess-1", &[], None);
        assert_eq!(params["sessionId"], json!("sess-1"));
        assert_eq!(params["cwd"], json!("/mnt/e/proj"));
        assert_eq!(params["mcpServers"], json!([]));
        assert!(params.get("additionalDirectories").is_none());
    }

    #[test]
    fn http_mcp_requires_capability_and_native_loopback() {
        let servers = vec![
            AcpMcpServer::Stdio {
                name: "existing".into(),
                command: "server".into(),
                args: vec![],
                env: vec![],
            },
            AcpMcpServer::Http {
                name: "kivio".into(),
                url: "http://127.0.0.1:1234/mcp".into(),
                headers: vec![("Authorization".into(), "Bearer secret".into())],
            },
        ];
        let capable = json!({"agentCapabilities":{"mcpCapabilities":{"http":true}}});
        let wsl =
            crate::external_agents::wsl::is_wsl_target(Path::new(r"\\wsl$\Ubuntu\usr\bin\kimi"));
        assert!(wsl);
        for (init, wsl, include_http) in [
            (capable.clone(), false, true),
            (capable, wsl, false),
            (
                json!({"agentCapabilities":{"mcpCapabilities":{"http":false}}}),
                false,
                false,
            ),
            (json!({}), false, false),
        ] {
            let accepted = supported_mcp_servers(&servers, &init, wsl);
            let new = build_session_new_params(Path::new("/project"), &accepted, None);
            let load = build_session_load_params(Path::new("/project"), "session", &accepted, None);
            assert_eq!(new["mcpServers"], load["mcpServers"]);
            assert_eq!(new["mcpServers"][0]["type"], "stdio");
            assert_eq!(
                new["mcpServers"].as_array().unwrap().len(),
                if include_http { 2 } else { 1 }
            );
            if include_http {
                assert_eq!(
                    new["mcpServers"][1],
                    json!({
                        "type": "http", "name": "kivio", "url": "http://127.0.0.1:1234/mcp",
                        "headers": [{"name": "Authorization", "value": "Bearer secret"}]
                    })
                );
            }
        }
    }

    #[test]
    fn session_new_omits_additional_directories_unless_advertised() {
        let params = build_session_new_params(Path::new("/tmp"), &[], None);
        assert!(params.get("additionalDirectories").is_none());
        let params = build_session_new_params(
            Path::new("/tmp"),
            &[],
            Some(&["/home/me/biz-a".to_string()]),
        );
        assert_eq!(params["additionalDirectories"], json!(["/home/me/biz-a"]));
        let params = build_session_new_params(Path::new("/tmp"), &[], Some(&[]));
        assert_eq!(params["additionalDirectories"], json!([]));
    }

    #[test]
    fn additional_directories_capability_reads_v1_and_v2() {
        assert!(agent_supports_additional_directories(&json!({
            "agentCapabilities": {
                "sessionCapabilities": { "additionalDirectories": true }
            }
        })));
        assert!(agent_supports_additional_directories(&json!({
            "capabilities": { "session": { "additionalDirectories": true } }
        })));
        assert!(!agent_supports_additional_directories(&json!({
            "agentCapabilities": { "loadSession": true }
        })));
        assert!(!agent_supports_additional_directories(&json!({})));
    }

    #[test]
    fn live_handshake_advertises_terminal_capability() {
        let params = acp_initialize_params(true);
        assert_eq!(params["clientCapabilities"]["terminal"], json!(true));
        assert!(params["clientCapabilities"]["elicitation"]["form"].is_object());
        assert_eq!(params["protocolVersion"], json!(ACP_PROTOCOL_VERSION));
        assert_eq!(params["clientInfo"]["name"], json!("kivio"));
        assert_eq!(params["clientInfo"]["title"], json!("Kivio"));
        assert_eq!(
            params["clientInfo"]["version"],
            json!(env!("CARGO_PKG_VERSION"))
        );
        let probe = acp_initialize_params(false);
        assert_eq!(probe["clientCapabilities"]["terminal"], json!(false));
        assert!(probe["clientCapabilities"].get("elicitation").is_none());
    }

    #[test]
    fn missing_acp_session_is_recognized() {
        assert!(is_missing_acp_session_error(
            "session/load: Session not found"
        ));
        assert!(is_missing_acp_session_error("unknown session id"));
        assert!(!is_missing_acp_session_error("ACP handshake timeout"));
        assert!(!is_missing_acp_session_error("Method not found"));
        assert!(!is_missing_acp_session_error(
            "ACP agent exited during handshake"
        ));
        assert!(!is_missing_acp_session_error("ACP session exited mid-turn"));
    }

    /// kimi 的 ACP 确实暴露推理档位——`configOptions` 里 `id="thinking"` /
    /// `category="thought_level"`。此前发现侧完全没读，胶囊上没有档位可选。
    /// 样本为本机 `kimi acp` 的 session/new 原样输出。
    #[test]
    fn reasoning_options_come_from_config_options() {
        let result = json!({
            "sessionId": "s1",
            "configOptions": [
                { "type": "select", "id": "model", "category": "model",
                  "currentValue": "kimi-code/k3-256k",
                  "options": [{ "value": "kimi-code/k3", "name": "K3" }] },
                { "type": "select", "id": "thinking", "category": "thought_level",
                  "currentValue": "high",
                  "options": [
                    { "value": "low", "name": "Low" },
                    { "value": "high", "name": "High" },
                    { "value": "max", "name": "Max" }
                  ] },
                { "type": "select", "id": "mode", "category": "mode",
                  "currentValue": "default",
                  "options": [{ "value": "default", "name": "Default" }] }
            ]
        });
        let (options, current) = extract_acp_reasoning(&result);
        assert_eq!(
            options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
            vec!["low", "high", "max"]
        );
        assert_eq!(options[1].label, "High");
        assert_eq!(current.as_deref(), Some("high"));
    }

    /// kimi always_thinking 模型（K2.7 Coding）ACP 只给 `options:[{on,On}]`——不能当 effort 显示。
    #[test]
    fn reasoning_options_skip_boolean_on_only_toggle() {
        let result = json!({
            "configOptions": [
                { "type": "select", "id": "thinking", "category": "thought_level",
                  "currentValue": "on",
                  "options": [{ "value": "on", "name": "On" }] }
            ]
        });
        let (options, current) = extract_acp_reasoning(&result);
        assert!(
            options.is_empty(),
            "On-only 不应成为 effort 档位: {options:?}"
        );
        assert!(current.is_none(), "On 当前值也不该回填: {current:?}");
    }

    #[test]
    fn reasoning_options_are_empty_when_the_cli_exposes_none() {
        // cursor/grok 那种只给 model+mode 的形态：不得凭空造出档位。
        let result = json!({
            "configOptions": [
                { "id": "model", "category": "model", "options": [] },
                { "id": "mode", "category": "mode", "options": [] }
            ]
        });
        let (options, current) = extract_acp_reasoning(&result);
        assert!(options.is_empty(), "不该造出档位：{options:?}");
        assert!(current.is_none());
    }

    #[test]
    fn reasoning_options_survive_a_missing_options_array() {
        // 结构变形（有 currentValue 但没 options 列表）时不 panic，当前值仍要能读出来。
        let result = json!({
            "configOptions": [
                { "id": "effort", "category": "reasoning", "currentValue": "medium" }
            ]
        });
        let (options, current) = extract_acp_reasoning(&result);
        assert!(options.is_empty());
        assert_eq!(current.as_deref(), Some("medium"));
    }

    #[test]
    fn acp_models_skip_banner_and_parse_json() {
        // 首行 banner（非 JSON）后跟 session/new 结果，仍应解析出模型。
        let lines = [
            "cursor-agent v1.2.3 starting…",
            "",
            r#"{"id":2,"result":{"models":{"availableModels":[{"modelId":"gpt-5","name":"GPT-5"},{"modelId":"o3"}]}}}"#,
        ];
        let models = collect_acp_models_from_lines(&lines).expect("models parsed past banner");
        assert!(models.iter().any(|m| m.id == "gpt-5"));
        assert!(models.iter().any(|m| m.id == "o3"));
        assert_eq!(models[0].id, "default");
    }

    #[test]
    fn acp_models_merge_async_session_update_push() {
        // session/new 结果给出一个模型，随后异步 session/update 推送更多模型 → 合并去重。
        let lines = [
            r#"{"id":2,"result":{"models":{"availableModels":[{"modelId":"gpt-5"}]}}}"#,
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"update":{"configOptions":[{"id":"model","options":[{"value":"claude-sonnet","name":"Claude Sonnet"},{"value":"gpt-5"}]}]}}}"#,
        ];
        let models = collect_acp_models_from_lines(&lines).expect("merged models");
        assert!(models.iter().any(|m| m.id == "gpt-5"));
        assert!(models.iter().any(|m| m.id == "claude-sonnet"));
        // gpt-5 出现在两处，去重后仅一次。
        assert_eq!(models.iter().filter(|m| m.id == "gpt-5").count(), 1);
    }

    #[test]
    fn acp_models_default_slot_enriched_by_current_model_id() {
        // grok 形态：session/new 结果带 models.currentModelId + availableModels（含 _meta 窗口）。
        // default 占位应富化为 "Default (Grok 4.5)" 并继承窗口，真实模型进列表。
        let lines = [
            r#"{"id":2,"result":{"sessionId":"s1","models":{"currentModelId":"grok-4.5","availableModels":[{"modelId":"grok-4.5","name":"Grok 4.5","_meta":{"totalContextTokens":500000}}]}}}"#,
        ];
        let models = collect_acp_models_from_lines(&lines).expect("models");
        assert_eq!(models[0].id, "default");
        assert_eq!(models[0].label, "Default (Grok 4.5)");
        assert_eq!(models[0].context_window_tokens, Some(500_000));
        let real = models
            .iter()
            .find(|m| m.id == "grok-4.5")
            .expect("real model");
        assert_eq!(real.context_window_tokens, Some(500_000));
    }

    #[test]
    fn acp_models_none_when_no_models_present() {
        let lines = [
            "banner line",
            r#"{"id":1,"result":{}}"#,
            r#"{"id":2,"result":{}}"#,
        ];
        assert!(collect_acp_models_from_lines(&lines).is_none());
    }

    #[test]
    fn acp_prompt_blocks_text_only_when_no_images() {
        let blocks = acp_prompt_blocks("hello", &[]);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], json!({ "type": "text", "text": "hello" }));
    }

    #[test]
    fn acp_prompt_blocks_appends_image_block() {
        let img = crate::external_agents::attachments::ImageBlock {
            data_base64: "AAAA".to_string(),
            mime: "image/png".to_string(),
            path: std::path::PathBuf::from("/tmp/a.png"),
        };
        let blocks = acp_prompt_blocks("look", std::slice::from_ref(&img));
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], json!("text"));
        assert_eq!(
            blocks[1],
            json!({ "type": "image", "data": "AAAA", "mimeType": "image/png" })
        );
    }

    /// Live cross-turn continuity over ACP: connect once to `cursor-agent acp`, run two prompt
    /// turns on the SAME process, and confirm turn 2 recalls a fact from turn 1 — proving the ACP
    /// session persists between turns (Phase 2). Requires a logged-in `cursor-agent` + network.
    #[tokio::test]
    #[ignore = "requires live cursor-agent login + network"]
    async fn acp_persistent_session_remembers_across_turns() {
        use crate::external_agents::session::live::SessionCommand;
        use tokio::sync::{mpsc, oneshot};

        let bin = which_bin("cursor-agent").expect("cursor-agent on PATH");
        let cwd = std::env::temp_dir();
        let session = AcpSession::connect(
            &bin,
            &["acp".to_string()],
            &cwd,
            None,
            None,
            None,
            &[],
            None,
            &[],
        )
        .await
        .expect("connect cursor-agent acp");
        let sid = session.session_id().to_string();
        assert!(!sid.is_empty());
        let control = spawn_acp_session_actor(session);

        async fn one_turn(control: &mpsc::Sender<SessionCommand>, prompt: &str) -> String {
            let (etx, mut erx) = mpsc::channel::<UnifiedAgentEvent>(64);
            let (dtx, drx) = oneshot::channel();
            control
                .send(SessionCommand::RunTurn {
                    prompt: prompt.to_string(),
                    model: None,
                    reasoning: None,
                    images: vec![],
                    extra_writable_roots: vec![],
                    events: etx,
                    done: dtx,
                    approvals: None,
                })
                .await
                .unwrap();
            let mut text = String::new();
            let mut drx = drx;
            loop {
                tokio::select! {
                    biased;
                    r = &mut drx => {
                        while let Ok(e) = erx.try_recv() {
                            if let UnifiedAgentEvent::TextDelta { delta } = e { text.push_str(&delta); }
                        }
                        r.unwrap().unwrap();
                        break;
                    }
                    ev = erx.recv() => {
                        if let Some(UnifiedAgentEvent::TextDelta { delta }) = ev { text.push_str(&delta); }
                    }
                }
            }
            text
        }

        let _t1 = one_turn(&control, "Remember this secret number: 42. Just reply OK.").await;
        let t2 = one_turn(
            &control,
            "What was the secret number I just gave you? Reply with only the digits.",
        )
        .await;
        eprintln!("acp turn2 reply: {t2:?}");
        assert!(
            t2.contains("42"),
            "turn 2 should recall 42 from turn 1, got: {t2:?}"
        );
        let _ = control.send(SessionCommand::Close).await;
    }

    fn which_bin(name: &str) -> Option<std::path::PathBuf> {
        let out = std::process::Command::new("which")
            .arg(name)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if p.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(p))
        }
    }

    #[test]
    fn apply_acp_session_update_emits_tool_use_and_result() {
        let started = serde_json::Map::from_iter([
            ("sessionUpdate".to_string(), json!("tool_call")),
            ("toolCallId".to_string(), json!("acp-1")),
            ("title".to_string(), json!("Write")),
        ]);
        let finished = serde_json::Map::from_iter([
            ("sessionUpdate".to_string(), json!("tool_call_update")),
            ("toolCallId".to_string(), json!("acp-1")),
            ("title".to_string(), json!("Write")),
            ("status".to_string(), json!("completed")),
            ("content".to_string(), json!("done")),
        ]);
        let mut emitted = HashSet::new();
        let mut events = Vec::new();
        assert!(apply_acp_session_update(
            &started,
            &mut emitted,
            &mut |event| {
                events.push(event);
            }
        ));
        assert!(apply_acp_session_update(
            &finished,
            &mut emitted,
            &mut |event| {
                events.push(event);
            }
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::ToolUse { id, name, .. } if id == "acp-1" && name == "Write"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::ToolResult { tool_use_id, content, is_error }
                if tool_use_id == "acp-1" && content == "done" && !*is_error
        )));
    }

    #[test]
    fn acp_forwards_late_render_input_before_its_terminal_result() {
        let mut emitted = HashSet::new();
        let mut events = Vec::new();
        for update in [
            json!({"sessionUpdate": "tool_call", "toolCallId": "a", "title": "tool"}),
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
                "title": "Call html_render", "rawInput": {"title": "Chart"},
                "status": "completed", "content": "published"}),
        ] {
            apply_acp_session_update(update.as_object().unwrap(), &mut emitted, &mut |event| {
                events.push(event)
            });
        }
        assert!(matches!(events.as_slice(), [
            UnifiedAgentEvent::ToolUse { .. },
            UnifiedAgentEvent::ToolUse { name, input, .. },
            UnifiedAgentEvent::ToolResult { is_error: false, .. }
        ] if name == "Call html_render" && input["rawInput"]["title"] == "Chart"));
    }

    #[test]
    fn acp_forwards_programmatic_name_even_with_a_different_title() {
        let mut emitted = HashSet::new();
        let mut events = Vec::new();
        for update in [
            json!({"sessionUpdate": "tool_call", "toolCallId": "a",
                "title": "Rendering chart", "rawInput": {"title": "Chart"}}),
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
                "title": "Rendering chart", "name": "kivio/html_render",
                "rawInput": null, "status": "completed", "content": "published"}),
        ] {
            apply_acp_session_update(update.as_object().unwrap(), &mut emitted, &mut |event| {
                events.push(event)
            });
        }
        assert!(matches!(events.as_slice(), [
            UnifiedAgentEvent::ToolUse { .. },
            UnifiedAgentEvent::ToolUse { name, input, .. },
            UnifiedAgentEvent::ToolResult { is_error: false, .. }
        ] if name == "Rendering chart" && input["name"] == "kivio/html_render"));
    }

    #[test]
    fn acp_in_progress_tool_updates_emit_snapshots_without_completing() {
        let mut emitted = HashSet::new();
        let mut events = Vec::new();
        for update in [
            json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "acp-1",
                "title": "Bash", "status": "in_progress",
                "content": [{"type": "content", "content": {"type": "text", "text": "first"}}]
            }),
            json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "acp-1",
                "status": "in_progress", "output": "first\nsecond"
            }),
            json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "acp-1",
                "result": "first\nsecond\nthird"
            }),
            json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "acp-1",
                "status": "in_progress", "content": ""
            }),
            json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "acp-1",
                "content": [{"type": "diff", "path": "file", "newText": "ignore"}]
            }),
            json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "acp-1"
            }),
        ] {
            assert!(apply_acp_session_update(
                update.as_object().unwrap(),
                &mut emitted,
                &mut |event| events.push(event),
            ));
        }
        assert!(matches!(
            events.as_slice(),
            [
                UnifiedAgentEvent::ToolUse { id, name, .. },
                UnifiedAgentEvent::ToolPartialOutput { tool_use_id: id1, output: first },
                UnifiedAgentEvent::ToolPartialOutput { tool_use_id: id2, output: second },
                UnifiedAgentEvent::ToolPartialOutput { tool_use_id: id3, output: third }
            ] if id == "acp-1" && name == "Bash"
                && id1 == "acp-1" && first == "first"
                && id2 == "acp-1" && second == "first\nsecond"
                && id3 == "acp-1" && third == "first\nsecond\nthird"
        ));
        let finished = json!({
            "sessionUpdate": "tool_call_update", "toolCallId": "acp-1",
            "status": "completed", "result": "done"
        });
        assert!(apply_acp_session_update(
            finished.as_object().unwrap(),
            &mut emitted,
            &mut |event| events.push(event),
        ));
        assert!(matches!(
            events.last(),
            Some(UnifiedAgentEvent::ToolResult { tool_use_id, content, is_error })
                if tool_use_id == "acp-1" && content == "done" && !*is_error
        ));
    }

    #[test]
    fn acp_in_progress_tool_update_resolves_terminal_output() {
        let mut host = AcpTerminalHost::new(std::env::temp_dir());
        host.seed_output("term_1", "current output");
        let update = json!({
            "sessionUpdate": "tool_call_update", "toolCallId": "bash-1",
            "status": "in_progress", "content": [
                {"type": "terminal", "terminalId": "term_1"},
                {"type": "diff", "path": "file", "newText": "ignore"}
            ]
        });
        let mut events = Vec::new();
        assert!(apply_acp_session_update_ex(
            update.as_object().unwrap(),
            &mut HashSet::new(),
            Some(&host),
            &mut |event| events.push(event),
        ));
        assert!(matches!(
            events.as_slice(),
            [
                UnifiedAgentEvent::ToolUse { id, .. },
                UnifiedAgentEvent::ToolPartialOutput { tool_use_id, output }
            ] if id == "bash-1" && tool_use_id == "bash-1" && output == "current output"
        ));
    }

    #[test]
    fn flatten_acp_tool_content_prefers_text_blocks_over_json() {
        assert_eq!(
            flatten_acp_tool_content(&json!([{
                "type": "content",
                "content": { "type": "text", "text": "hello" }
            }])),
            "hello"
        );
        assert_eq!(
            flatten_acp_tool_content(&json!([{
                "type": "terminal",
                "terminalId": "term_missing"
            }])),
            ""
        );
    }

    #[test]
    fn flatten_acp_tool_content_resolves_terminal_output() {
        let mut host = AcpTerminalHost::new(std::env::temp_dir());
        host.seed_output("term_1", "KIVIO_ACP_TERMINAL_OK\n");
        let finished = serde_json::Map::from_iter([
            ("sessionUpdate".to_string(), json!("tool_call_update")),
            ("toolCallId".to_string(), json!("bash-1")),
            ("title".to_string(), json!("Bash")),
            ("status".to_string(), json!("completed")),
            (
                "content".to_string(),
                json!([{ "type": "terminal", "terminalId": "term_1" }]),
            ),
        ]);
        let mut emitted = HashSet::new();
        let mut events = Vec::new();
        assert!(apply_acp_session_update_ex(
            &finished,
            &mut emitted,
            Some(&host),
            &mut |event| events.push(event),
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            UnifiedAgentEvent::ToolResult { tool_use_id, content, is_error }
                if tool_use_id == "bash-1"
                    && content.contains("KIVIO_ACP_TERMINAL_OK")
                    && !*is_error
        )));
    }

    #[test]
    fn kimi_glob_policy_error_is_explained() {
        let finished = serde_json::Map::from_iter([
            ("sessionUpdate".to_string(), json!("tool_call_update")),
            ("toolCallId".to_string(), json!("glob-1")),
            ("title".to_string(), json!("Glob")),
            ("status".to_string(), json!("failed")),
            (
                "content".to_string(),
                json!([{
                    "type": "content",
                    "content": {
                        "type": "text",
                        "text": "ACP runtime only supports interactive Bash tool processes"
                    }
                }]),
            ),
        ]);
        let mut emitted = HashSet::new();
        let mut events = Vec::new();
        assert!(apply_acp_session_update(
            &finished,
            &mut emitted,
            &mut |event| events.push(event),
        ));
        let content = events.iter().find_map(|event| match event {
            UnifiedAgentEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } if tool_use_id == "glob-1" && *is_error => Some(content.clone()),
            _ => None,
        });
        let content = content.expect("failed Glob result");
        assert!(
            content.contains("不会把 Glob/Grep 交给宿主"),
            "content={content}"
        );
        assert!(content.contains("ACP runtime only supports interactive Bash tool processes"));
    }

    #[test]
    fn explain_acp_agent_policy_error_is_idempotent() {
        let once = explain_acp_agent_policy_error(
            "ACP runtime only supports interactive Bash tool processes",
        );
        assert_eq!(explain_acp_agent_policy_error(&once), once);
        assert_eq!(
            explain_acp_agent_policy_error("normal output".to_string()),
            "normal output"
        );
    }

    // ---- AcpTextAssembler / shared update-dedup (Step 2) ----

    fn msg_chunk(text: &str) -> serde_json::Map<String, Value> {
        serde_json::Map::from_iter([
            ("sessionUpdate".to_string(), json!("agent_message_chunk")),
            ("content".to_string(), json!({ "text": text })),
        ])
    }

    fn thought_chunk(text: &str) -> serde_json::Map<String, Value> {
        serde_json::Map::from_iter([
            ("sessionUpdate".to_string(), json!("agent_thought_chunk")),
            ("content".to_string(), json!({ "text": text })),
        ])
    }

    fn tool_call_update(id: &str) -> serde_json::Map<String, Value> {
        serde_json::Map::from_iter([
            ("sessionUpdate".to_string(), json!("tool_call")),
            ("toolCallId".to_string(), json!(id)),
            ("title".to_string(), json!("Write")),
        ])
    }

    fn run_updates(updates: &[serde_json::Map<String, Value>]) -> (String, Vec<UnifiedAgentEvent>) {
        let mut state = AcpUpdateState::default();
        let mut events = Vec::new();
        for u in updates {
            acp_apply_session_update(u, &mut state, &mut |e| events.push(e));
        }
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                UnifiedAgentEvent::TextDelta { delta } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        (text, events)
    }

    #[test]
    fn acp_plan_updates_are_not_text_or_thought_boundaries() {
        for update in [
            json!({"sessionUpdate": "plan", "entries": []}),
            json!({"sessionUpdate": "plan_update", "plan": {"type": "items", "entries": []}}),
            json!({"sessionUpdate": "plan_update", "plan": {"type": "markdown", "content": "plan"}}),
            json!({"sessionUpdate": "plan_removed", "planId": "plan-1"}),
        ] {
            let update = update.as_object().unwrap().clone();
            let (text, _) = run_updates(&[
                msg_chunk("abc"),
                update.clone(),
                msg_chunk("xyz"),
                msg_chunk("abcxyzEND"),
            ]);
            assert_eq!(text, "abcxyzEND");
            let (_, events) = run_updates(&[
                thought_chunk("abc"),
                update,
                thought_chunk("xyz"),
                thought_chunk("abcxyzEND"),
            ]);
            let thought: String = events
                .iter()
                .filter_map(|event| match event {
                    UnifiedAgentEvent::ThinkingDelta { delta } => Some(delta.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(thought, "abcxyzEND");
        }
    }

    #[test]
    fn assembler_trims_per_message_accumulated_snapshots() {
        let mut a = AcpTextAssembler::default();
        assert_eq!(a.push_chunk("Hel"), Some("Hel".to_string()));
        assert_eq!(a.push_chunk("Hello"), Some("lo".to_string()));
        // 重复快照无新增内容。
        assert_eq!(a.push_chunk("Hello"), None);
    }

    #[test]
    fn driver_incremental_no_duplication() {
        let (text, _) = run_updates(&[msg_chunk("Hello"), msg_chunk(" there")]);
        assert_eq!(text, "Hello there");
    }

    #[test]
    fn driver_per_message_snapshots_with_tool_call_no_dup() {
        // msg1 按消息累积 → tool_call(边界)→ msg2 按消息累积。
        let (text, events) = run_updates(&[
            msg_chunk("Loo"),
            msg_chunk("Looking"),
            tool_call_update("t1"),
            msg_chunk("Don"),
            msg_chunk("Done"),
        ]);
        assert_eq!(text, "LookingDone");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, UnifiedAgentEvent::ToolUse { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn driver_whole_turn_snapshots_with_tool_call_no_dup_no_loss() {
        // 整轮累积:tool_call 后的快照仍以前一段正文为前缀,只应发出新尾巴。
        let (text, _) = run_updates(&[
            msg_chunk("Looking"),
            tool_call_update("t1"),
            msg_chunk("LookingDone"),
        ]);
        assert_eq!(text, "LookingDone");
    }

    #[test]
    fn driver_thought_then_message_are_separate_streams() {
        // 思考块与正文块互为边界,各自独立累积,互不裁剪对方内容。
        let (text, events) = run_updates(&[thought_chunk("plan"), msg_chunk("plan answer")]);
        // 正文以 "plan answer" 起头,思考游标不影响正文;正文应完整发出。
        assert_eq!(text, "plan answer");
        assert!(events
            .iter()
            .any(|e| matches!(e, UnifiedAgentEvent::ThinkingDelta { delta } if delta == "plan")));
    }

    // ---- ACP usage：usage_update（官方 RFD）与 PromptResponse.usage ----

    fn usage_update(fields: Value) -> serde_json::Map<String, Value> {
        let mut map =
            serde_json::Map::from_iter([("sessionUpdate".to_string(), json!("usage_update"))]);
        if let Some(obj) = fields.as_object() {
            for (k, v) in obj {
                map.insert(k.clone(), v.clone());
            }
        }
        map
    }

    #[test]
    fn usage_update_tolerates_missing_fields() {
        let only_used = usage_update(json!({ "used": 42 }));
        let parsed = parse_acp_usage_update(&only_used).expect("只有 used 也该解析");
        assert_eq!(parsed.input_tokens, None);
        assert_eq!(parsed.total_tokens, None);
        assert_eq!(parsed.context_tokens, Some(42));
        assert_eq!(parsed.context_window_tokens, None);

        let only_size = usage_update(json!({ "size": 262144 }));
        let parsed = parse_acp_usage_update(&only_size).expect("只有 size 也该解析");
        assert_eq!(parsed.input_tokens, None);
        assert_eq!(parsed.total_tokens, None);
        assert_eq!(parsed.context_tokens, None);
        assert_eq!(parsed.context_window_tokens, Some(262_144));

        let zero = parse_acp_usage_update(&usage_update(json!({ "used": 0 }))).unwrap();
        assert_eq!(zero.context_tokens, Some(0));
        assert_eq!(zero.input_tokens, None);

        // 两个都缺 → None，不 panic。
        assert!(parse_acp_usage_update(&usage_update(json!({}))).is_none());
        // 非数值/负数等异常形态也不得 panic。
        assert!(
            parse_acp_usage_update(&usage_update(json!({ "used": "many", "size": -1 }))).is_none()
        );
    }

    #[test]
    fn driver_emits_usage_event_for_usage_update() {
        let (_, events) = run_updates(&[usage_update(json!({ "used": 13477, "size": 200000 }))]);
        assert!(events.iter().any(|e| matches!(
            e,
            UnifiedAgentEvent::Usage { usage }
                if usage.input_tokens.is_none()
                    && usage.total_tokens.is_none()
                    && usage.context_tokens == Some(13_477)
                    && usage.context_window_tokens == Some(200_000)
        )));
    }

    #[test]
    fn usage_update_between_chunks_never_duplicates_text() {
        // 若 usage_update 被当成边界，游标会在下一条不同前缀的 chunk 处被清空，
        // 之后到来的整轮累积快照就不再以游标为前缀 → 整段正文重发一遍。
        let (text, _) = run_updates(&[
            msg_chunk("abc"),
            usage_update(json!({ "used": 100, "size": 200000 })),
            msg_chunk("xyz"),
            msg_chunk("abcxyzEND"),
        ]);
        assert_eq!(text, "abcxyzEND", "usage_update 不得重置正文游标");
    }

    #[test]
    fn usage_update_between_chunks_never_duplicates_thought() {
        let mut state = AcpUpdateState::default();
        let mut events = Vec::new();
        for u in [
            thought_chunk("abc"),
            usage_update(json!({ "used": 100 })),
            thought_chunk("xyz"),
            thought_chunk("abcxyzEND"),
        ] {
            acp_apply_session_update(&u, &mut state, &mut |e| events.push(e));
        }
        let thoughts: String = events
            .iter()
            .filter_map(|e| match e {
                UnifiedAgentEvent::ThinkingDelta { delta } => Some(delta.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(thoughts, "abcxyzEND", "usage_update 不得重置思考游标");
        // 用量事件仍要发出来。
        assert!(events
            .iter()
            .any(|e| matches!(e, UnifiedAgentEvent::Usage { .. })));
    }

    #[test]
    fn shared_update_mapper_also_emits_context_only_usage() {
        let update = usage_update(json!({ "used": 13477, "size": 200000 }));
        let mut emitted = HashSet::new();
        let mut events = Vec::new();
        assert!(apply_acp_session_update(&update, &mut emitted, &mut |e| {
            events.push(e);
        }));
        assert!(events.iter().any(|e| matches!(
            e,
            UnifiedAgentEvent::Usage { usage }
                if usage.input_tokens.is_none()
                    && usage.total_tokens.is_none()
                    && usage.context_tokens == Some(13_477)
                    && usage.context_window_tokens == Some(200_000)
        )));
    }

    #[test]
    fn prompt_response_usage_counts_cache_and_thought_tokens() {
        // opencode 本机实测样本；11685 + 4 + 11 + 1792 = 13492 = 其自报的 totalTokens。
        let usage = format_acp_usage(&json!({
            "inputTokens": 11685,
            "outputTokens": 4,
            "totalTokens": 13492,
            "thoughtTokens": 11,
            "cachedReadTokens": 1792
        }))
        .expect("有非零字段应产出用量");
        assert_eq!(usage.input_tokens, Some(11_685));
        assert_eq!(usage.output_tokens, Some(4));
        assert_eq!(usage.cached_input_tokens, Some(1_792));
        assert_eq!(usage.reasoning_tokens, Some(11));
        assert_eq!(usage.total_tokens, Some(13_492));
        // PromptResponse.usage 不带窗口——留 None 交给 run.rs 的窗口粘滞。
        assert_eq!(usage.context_window_tokens, None);
        assert_eq!(usage.context_tokens, None);
    }

    /// hermes 0.21.3 本机实测（2026-10-13，`openai-codex:gpt-6.1-sol` 经 chatgpt codex 代理）：
    /// 它的 `inputTokens` 是**全量 prompt（已含 cache）**、`cachedReadTokens ⊆ inputTokens`
    /// （OpenAI 口径，对账：其 state.db 累计非缓存 21448 + cache 175616 = 197064 = inputTokens）。
    /// 按「四项并列」累加会把 cache 双算成 373155（实机用量条 74.6%，真实 39.5%）。
    /// 与 grok `_meta` 路同理：CLI 自报了 `totalTokens` 就信它，不自己按分量猜口径。
    #[test]
    fn prompt_response_usage_trusts_cli_reported_total() {
        let usage = format_acp_usage(&json!({
            "inputTokens": 197064, "outputTokens": 466, "totalTokens": 197530,
            "thoughtTokens": 9, "cachedReadTokens": 175616
        }))
        .expect("hermes 上报的是真实用量");
        assert_eq!(
            usage.total_tokens,
            Some(197_530),
            "总量口径归 CLI 自报，不再按并列分量累加"
        );
        // 分量照实记录，成本面板还要用。
        assert_eq!(usage.input_tokens, Some(197_064));
        assert_eq!(usage.cached_input_tokens, Some(175_616));
        assert_eq!(usage.reasoning_tokens, Some(9));
    }

    #[test]
    fn prompt_response_usage_counts_cache_write() {
        let usage = format_acp_usage(&json!({ "cachedWriteTokens": 4096 }))
            .expect("只有 cache write 也是真实占用");
        assert_eq!(usage.cache_creation_input_tokens, Some(4_096));
        assert_eq!(usage.total_tokens, Some(4_096));
    }

    #[test]
    fn prompt_response_usage_is_none_when_empty_or_all_zero() {
        // ACP 标记 UNSTABLE，缺失/全零一律 None（不得报错，也不得造出 0 用量覆盖真实值）。
        assert!(format_acp_usage(&json!({})).is_none());
        assert!(format_acp_usage(&json!({
            "inputTokens": 0, "outputTokens": 0,
            "cachedReadTokens": 0, "cachedWriteTokens": 0, "thoughtTokens": 0
        }))
        .is_none());
    }

    /// grok 0.2.114 本机实测原文（2026-07-30）：`result` 里**没有** `usage`，用量全在 `_meta`。
    #[test]
    fn prompt_result_reads_grok_meta_usage() {
        let result = json!({
            "stopReason": "end_turn",
            "_meta": {
                "sessionId": "019fb3a1-b5d8-7a30-9a49-56c47a1c8df6",
                "totalTokens": 14893, "modelId": "grok-4.5",
                "inputTokens": 14870, "outputTokens": 23,
                "cachedReadTokens": 128, "reasoningTokens": 21,
                "usage": { "inputTokens": 14870, "outputTokens": 23, "totalTokens": 14893,
                           "cachedReadTokens": 128, "reasoningTokens": 21, "numTurns": 1 }
            }
        });
        let usage = usage_from_prompt_result(&result, None).expect("grok 的 _meta 必须被读到");
        // 信 grok 自报的 total：它的 cachedRead ⊆ input（14870 + 23 = 14893），
        // 按 opencode 那套并列口径累加会变成 15021（cache 双算）。
        assert_eq!(usage.total_tokens, Some(14_893));
        assert_eq!(usage.input_tokens, Some(14_870));
        assert_eq!(usage.cached_input_tokens, Some(128));
        assert_eq!(usage.context_tokens, Some(14_893));
    }

    /// grok 在每个 prompt 返回累计计费，顶层仍是末次模型请求的上下文。
    #[test]
    fn prompt_result_bills_grok_whole_turn_and_reports_last_request_context() {
        let previous = json!({
            "_meta": { "totalTokens": 22212, "usage": {
                "inputTokens": 22168, "outputTokens": 36, "totalTokens": 22204
            }}
        });
        let previous_usage = usage_from_prompt_result(&previous, None).unwrap();
        assert_eq!(previous_usage.total_tokens, Some(22_204));
        let result = json!({
            "stopReason": "end_turn",
            "_meta": {
                "totalTokens": 15137, "inputTokens": 15108, "outputTokens": 29,
                "cachedReadTokens": 14848, "reasoningTokens": 24,
                "usage": { "inputTokens": 30034, "outputTokens": 79, "totalTokens": 30113,
                           "cachedReadTokens": 29696, "cacheCreationTokens": 16,
                           "reasoningTokens": 50, "modelCalls": 2, "numTurns": 2 }
            }
        });
        let usage = usage_from_prompt_result(&result, Some(12_000)).expect("应读到用量");
        assert_eq!(usage.input_tokens, Some(30_034));
        assert_eq!(usage.output_tokens, Some(79));
        assert_eq!(usage.total_tokens, Some(30_113));
        assert_eq!(usage.cached_input_tokens, Some(29_696));
        assert_eq!(usage.cache_creation_input_tokens, Some(16));
        assert_eq!(usage.reasoning_tokens, Some(50));
        assert_eq!(usage.context_tokens, Some(15_137));
    }

    #[test]
    fn opencode_prompt_uses_turn_totals_and_latest_context_without_previous_turn() {
        let mut previous_state = AcpUpdateState::default();
        acp_apply_session_update(
            &usage_update(json!({ "used": 9000 })),
            &mut previous_state,
            &mut |_| {},
        );
        let previous = usage_from_prompt_result(
            &json!({ "usage": { "inputTokens": 8000, "outputTokens": 5, "totalTokens": 8005 } }),
            previous_state.context_tokens,
        )
        .unwrap();
        assert_eq!(previous.context_tokens, Some(9000));

        let mut state = AcpUpdateState::default();
        let mut events = Vec::new();
        for update in [
            usage_update(json!({ "used": 156, "size": 200000 })),
            usage_update(json!({ "used": 178 })),
            usage_update(json!({ "size": 200000 })),
        ] {
            acp_apply_session_update(&update, &mut state, &mut |event| events.push(event));
        }
        assert!(matches!(
            events.last(),
            Some(UnifiedAgentEvent::Usage { usage }) if usage.context_tokens == Some(178)
        ));
        // 两次请求的 input 100+120、output 4+5、cache 40+50 与 10+0。
        let result = json!({ "usage": {
            "inputTokens": 220, "outputTokens": 9, "cachedReadTokens": 90,
            "cachedWriteTokens": 10, "thoughtTokens": 5, "totalTokens": 334
        }});
        let usage = usage_from_prompt_result(&result, state.context_tokens).unwrap();
        assert_eq!(usage.input_tokens, Some(220));
        assert_eq!(usage.output_tokens, Some(9));
        assert_eq!(usage.cached_input_tokens, Some(90));
        assert_eq!(usage.cache_creation_input_tokens, Some(10));
        assert_eq!(usage.reasoning_tokens, Some(5));
        assert_eq!(usage.total_tokens, Some(334));
        assert_eq!(usage.context_tokens, Some(178));

        let next_state = AcpUpdateState::default();
        let without_update = usage_from_prompt_result(&result, next_state.context_tokens).unwrap();
        assert_eq!(without_update.context_tokens, None);
    }

    /// 官方 `result.usage` 优先于 `_meta`：opencode 那条路不能被 grok 分支改口径。
    #[test]
    fn prompt_result_prefers_official_usage_field() {
        let result = json!({
            "usage": { "inputTokens": 11685, "outputTokens": 4,
                       "thoughtTokens": 11, "cachedReadTokens": 1792 },
            "_meta": { "totalTokens": 999_999 }
        });
        let usage = usage_from_prompt_result(&result, Some(12_345)).expect("应读官方字段");
        assert_eq!(usage.total_tokens, Some(13_492)); // 四项并列相加，不是 _meta 的 999999
        assert_eq!(usage.context_tokens, Some(12_345));
    }

    #[test]
    fn prompt_result_without_usage_anywhere_is_none() {
        assert!(usage_from_prompt_result(&json!({ "stopReason": "end_turn" }), None).is_none());
        // `_meta` 存在但只有非用量字段（cursor/opencode 的 result 形状）——不得造出假用量。
        assert!(usage_from_prompt_result(
            &json!({
                "stopReason": "end_turn", "_meta": { "sessionId": "s1" }
            }),
            None
        )
        .is_none());
        // totalTokens 为 0 的空轮（斜杠命令）：不得覆盖真实值。
        assert!(
            usage_from_prompt_result(&json!({ "_meta": { "totalTokens": 0 } }), None).is_none()
        );
        let snapshot = usage_from_prompt_result(
            &json!({ "_meta": {
                "totalTokens": 120, "inputTokens": 110, "outputTokens": 10
            }}),
            None,
        )
        .unwrap();
        assert_eq!(snapshot.context_tokens, Some(120));
        assert_eq!(snapshot.input_tokens, None);
        assert_eq!(snapshot.total_tokens, None);
        let billing = usage_from_prompt_result(
            &json!({ "_meta": {
                "usage": { "inputTokens": 200, "outputTokens": 20, "totalTokens": 220 }
            }}),
            None,
        )
        .unwrap();
        assert_eq!(billing.total_tokens, Some(220));
        assert_eq!(billing.context_tokens, None);
    }

    #[test]
    fn normalize_models_from_available() {
        let result = json!({
            "models": {
                "availableModels": [
                    { "modelId": "grok-4.3", "name": "Grok 4.3" }
                ]
            }
        });
        let models = normalize_models(&result);
        assert!(models.iter().any(|m| m.id == "grok-4.3"));
    }

    #[test]
    fn cursor_model_id_params_carry_context_window() {
        // 本机实测 cursor 的 session/new 返回样本（2026-07-26）：窗口写在方括号参数里，
        // 且这些模型**没有** _meta.totalContextTokens。
        assert_eq!(
            context_window_from_model_id_params(
                "claude-opus-5[thinking=true,context=300k,effort=high,fast=false]"
            ),
            Some(300_000)
        );
        assert_eq!(
            context_window_from_model_id_params(
                "gpt-5.6-sol[context=272k,reasoning=medium,fast=false]"
            ),
            Some(272_000)
        );
        assert_eq!(
            context_window_from_model_id_params(
                "claude-sonnet-4-6[thinking=true,context=200k,effort=high,fast=false]"
            ),
            Some(200_000)
        );
        // 无 context= 提示 → None，不猜。
        assert_eq!(
            context_window_from_model_id_params("composer-2.5[fast=true]"),
            None
        );
        assert_eq!(context_window_from_model_id_params("default[]"), None);
        // 完全没有方括号（grok / kimi 形态）也不该 panic。
        assert_eq!(context_window_from_model_id_params("grok-4.5"), None);
        // 有 context= 但值解析不出来 → None。
        assert_eq!(
            context_window_from_model_id_params("weird[context=abc]"),
            None
        );
    }

    #[test]
    fn normalize_models_reads_cursor_context_hint_from_model_id() {
        let result = json!({
            "models": {
                "currentModelId": "composer-2.5[fast=true]",
                "availableModels": [
                    { "modelId": "claude-opus-5[thinking=true,context=300k,effort=high,fast=false]",
                      "name": "claude-opus-5" },
                    { "modelId": "gpt-5.6-sol[context=272k,reasoning=medium,fast=false]",
                      "name": "gpt-5.6-sol" },
                    { "modelId": "composer-2.5[fast=true]", "name": "composer-2.5" }
                ]
            }
        });
        let models = normalize_models(&result);
        let window_of = |id: &str| {
            models
                .iter()
                .find(|m| m.id == id)
                .unwrap_or_else(|| panic!("missing {id}"))
                .context_window_tokens
        };
        assert_eq!(
            window_of("claude-opus-5[thinking=true,context=300k,effort=high,fast=false]"),
            Some(300_000)
        );
        assert_eq!(
            window_of("gpt-5.6-sol[context=272k,reasoning=medium,fast=false]"),
            Some(272_000)
        );
        // 无提示的模型窗口留空——不许兜底成 200K。
        assert_eq!(window_of("composer-2.5[fast=true]"), None);
    }

    /// cursor 的模型实际是从 **configOptions** 分支出来的，不是 `models.availableModels`
    /// ——那条分支命中后会提前 return。真机实测 33 个模型全走 configOptions，
    /// 所以只给 availableModels 加窗口解析在生产里等于没做（实测 0/33 有窗口）。
    /// 这条测试钉住 configOptions 分支，防止再次回归。
    #[test]
    fn normalize_models_reads_context_hint_from_config_options_branch() {
        // 真机 `cursor-agent acp` 的 session/new 返回形状（含 mode 项，模型项排第二）。
        let result = json!({
            "configOptions": [
                { "id": "mode", "category": "mode", "options": [
                    { "value": "agent", "name": "Agent" }
                ]},
                { "id": "model", "category": "model",
                  "currentValue": "claude-opus-5[thinking=true,context=300k,effort=high,fast=false]",
                  "options": [
                    { "value": "default[]", "name": "Auto" },
                    { "value": "claude-opus-5[thinking=true,context=300k,effort=high,fast=false]",
                      "name": "claude-opus-5" },
                    { "value": "gpt-5.6-sol[context=272k,reasoning=medium,fast=false]",
                      "name": "gpt-5.6-sol" },
                    { "value": "composer-2.5[fast=true]", "name": "composer-2.5" }
                ]}
            ]
        });
        let models = normalize_models(&result);
        let window_of = |id: &str| {
            models
                .iter()
                .find(|m| m.id == id)
                .unwrap_or_else(|| panic!("missing {id}"))
                .context_window_tokens
        };
        assert_eq!(
            window_of("claude-opus-5[thinking=true,context=300k,effort=high,fast=false]"),
            Some(300_000)
        );
        assert_eq!(
            window_of("gpt-5.6-sol[context=272k,reasoning=medium,fast=false]"),
            Some(272_000)
        );
        // `default[]` 与无 context= 的模型都留空，不猜。
        assert_eq!(window_of("composer-2.5[fast=true]"), None);
        assert_eq!(window_of("default[]"), None);
    }

    #[test]
    fn normalize_models_prefers_meta_window_over_model_id_hint() {
        // 同时有 _meta.totalContextTokens 与 modelId 的 context= 时取 _meta（显式字段更可靠）。
        let result = json!({
            "models": {
                "availableModels": [
                    { "modelId": "x-model[context=300k]", "name": "X",
                      "_meta": { "totalContextTokens": 500000 } }
                ]
            }
        });
        let models = normalize_models(&result);
        assert_eq!(
            models
                .iter()
                .find(|m| m.id == "x-model[context=300k]")
                .expect("model")
                .context_window_tokens,
            Some(500_000)
        );
    }

    #[test]
    fn extract_acp_current_reads_current_model_and_reasoning() {
        // grok 形态：currentModelId + 当前模型的 _meta.reasoningEfforts 里 default=true。
        let result = json!({
            "models": {
                "currentModelId": "grok-4.5",
                "availableModels": [
                    { "modelId": "grok-4.5", "name": "Grok 4.5", "_meta": {
                        "reasoningEfforts": [
                            { "id": "low", "default": false },
                            { "id": "high", "default": true }
                        ]
                    }},
                    { "modelId": "grok-4-fast", "name": "Grok 4 Fast" }
                ]
            }
        });
        let (model, reasoning) = extract_acp_current(&result);
        assert_eq!(model.as_deref(), Some("grok-4.5"));
        assert_eq!(reasoning.as_deref(), Some("high"));
    }

    #[test]
    fn extract_acp_current_none_without_current() {
        let result = json!({
            "models": { "availableModels": [ { "modelId": "gpt-5" } ] }
        });
        let (model, reasoning) = extract_acp_current(&result);
        assert!(model.is_none());
        assert!(reasoning.is_none());
    }

    // ---- N3: per-turn model / reasoning change decisions (Step 5) ----

    #[test]
    fn model_set_rpc_uses_config_option_when_id_present() {
        let (method, params) = model_set_rpc("sess-1", Some("model"), "grok-4.5");
        assert_eq!(method, "session/set_config_option");
        assert_eq!(params["sessionId"], json!("sess-1"));
        assert_eq!(params["configId"], json!("model"));
        assert_eq!(params["value"], json!("grok-4.5"));
    }

    #[test]
    fn model_set_rpc_falls_back_to_set_model() {
        let (method, params) = model_set_rpc("sess-1", None, "sonnet-4");
        assert_eq!(method, "session/set_model");
        assert_eq!(params["sessionId"], json!("sess-1"));
        assert_eq!(params["modelId"], json!("sonnet-4"));
    }

    #[test]
    fn reasoning_action_no_change_when_equal() {
        let cur = Some("high".to_string());
        let want = Some("high".to_string());
        assert_eq!(
            reasoning_action(&cur, &want, &Some("reasoning".to_string())),
            ReasoningAction::NoChange
        );
    }

    #[test]
    fn reasoning_action_sets_config_when_option_available() {
        let cur = Some("low".to_string());
        let want = Some("high".to_string());
        assert_eq!(
            reasoning_action(&cur, &want, &Some("reasoning_effort".to_string())),
            ReasoningAction::SetConfig {
                config_id: "reasoning_effort".to_string(),
                value: "high".to_string(),
            }
        );
    }

    #[test]
    fn reasoning_action_reconnects_when_launch_flag_only() {
        // grok: reasoning is a launch flag → no config id → change forces a reconnect.
        let cur = Some("low".to_string());
        let want = Some("high".to_string());
        assert_eq!(
            reasoning_action(&cur, &want, &None),
            ReasoningAction::Reconnect
        );
    }

    #[test]
    fn find_config_ids_picks_model_and_reasoning() {
        let result = json!({
            "configOptions": [
                { "id": "model", "options": [] },
                { "id": "reasoning_effort", "options": [] },
            ]
        });
        let (model_id, reasoning_id) = find_config_ids(&result);
        assert_eq!(model_id.as_deref(), Some("model"));
        assert_eq!(reasoning_id.as_deref(), Some("reasoning_effort"));
    }

    #[test]
    fn find_config_ids_none_when_absent() {
        let (model_id, reasoning_id) = find_config_ids(&json!({}));
        assert!(model_id.is_none());
        assert!(reasoning_id.is_none());
    }

    fn event_variant(event: &UnifiedAgentEvent) -> &'static str {
        match event {
            UnifiedAgentEvent::TextDelta { .. } => "TextDelta",
            UnifiedAgentEvent::TextItemDelta { .. } => "TextItemDelta",
            UnifiedAgentEvent::TextItemCompleted { .. } => "TextItemCompleted",
            UnifiedAgentEvent::InteractionResolved { .. } => "InteractionResolved",
            UnifiedAgentEvent::ThinkingDelta { .. } => "ThinkingDelta",
            UnifiedAgentEvent::ToolUse { .. } => "ToolUse",
            UnifiedAgentEvent::ToolResult { .. } => "ToolResult",
            UnifiedAgentEvent::ToolPartialOutput { .. } => "ToolPartialOutput",
            UnifiedAgentEvent::Usage { .. } => "Usage",
            UnifiedAgentEvent::Error { .. } => "Error",
            UnifiedAgentEvent::Raw { .. } => "Raw",
            UnifiedAgentEvent::SlashCommands { .. } => "SlashCommands",
            UnifiedAgentEvent::CliCompacted { .. } => "CliCompacted",
            UnifiedAgentEvent::UserSteer { .. } => "UserSteer",
            UnifiedAgentEvent::UserFollowUp { .. } => "UserFollowUp",
            UnifiedAgentEvent::QueuedTextsRestored { .. } => "QueuedTextsRestored",
            UnifiedAgentEvent::StatusNote { .. } => "StatusNote",
            UnifiedAgentEvent::RetryStatus { .. } => "RetryStatus",
            UnifiedAgentEvent::BackgroundTask { .. } => "BackgroundTask",
            UnifiedAgentEvent::TodoWrite { .. } => "TodoWrite",
            UnifiedAgentEvent::SubagentProgress { .. } => "SubagentProgress",
        }
    }

    /// 一轮真机 ACP 对话，**走生产用的常驻 actor**（`AcpSession::connect` +
    /// `spawn_acp_session_actor`）。
    ///
    /// 此前这条测试驱动的是 `run_acp_session` —— 一个只被它自己吊着命的一次性驱动，
    /// 也就是同一个协议的第二份实现。改成驱动生产代码后那份实现被删掉了。
    #[tokio::test]
    #[ignore = "requires live cursor-agent login + network"]
    async fn cursor_acp_smoke() {
        let Some(bin) = which_bin("cursor-agent") else {
            eprintln!("SKIP: cursor-agent 不在 PATH 上");
            return;
        };
        let cwd = std::env::temp_dir();
        let session = match AcpSession::connect(
            &bin,
            &["acp".to_string()],
            &cwd,
            None,
            None,
            None,
            &[],
            None,
            &[],
        )
        .await
        {
            Ok(session) => session,
            Err(err) => {
                eprintln!("SKIP: 连接失败（未登录 / 网络？）：{err}");
                return;
            }
        };
        let control = spawn_acp_session_actor(session);
        let captured = run_one_live_turn(
            &control,
            "Reply with exactly the token SMOKE_OK and nothing else.",
        )
        .await;

        eprintln!("=== cursor ACP smoke: {} events ===", captured.len());
        for (i, ev) in captured.iter().enumerate() {
            eprintln!("[{i}] {ev:?}");
        }
        let seq: Vec<&str> = captured.iter().map(event_variant).collect();
        eprintln!("cursor sequence: {seq:?}");

        let got_text = captured
            .iter()
            .any(|e| matches!(e, UnifiedAgentEvent::TextDelta { .. }));
        let got_error = captured
            .iter()
            .any(|e| matches!(e, UnifiedAgentEvent::Error { .. }));
        assert!(
            got_text || got_error,
            "expected at least one TextDelta or an Error, got: {seq:?}"
        );
    }

    /// 把一轮跑完并收齐事件。90s 墙钟上限 —— 挂住就是失败，不是慢。
    async fn run_one_live_turn(
        control: &tokio::sync::mpsc::Sender<SessionCommand>,
        prompt: &str,
    ) -> Vec<UnifiedAgentEvent> {
        let (events_tx, mut events_rx) = tokio::sync::mpsc::channel::<UnifiedAgentEvent>(256);
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        control
            .send(SessionCommand::RunTurn {
                prompt: prompt.to_string(),
                model: None,
                reasoning: None,
                images: Vec::new(),
                extra_writable_roots: Vec::new(),
                events: events_tx,
                done: done_tx,
                approvals: None,
            })
            .await
            .expect("actor alive");
        let collector = tokio::spawn(async move {
            let mut out = Vec::new();
            while let Some(event) = events_rx.recv().await {
                out.push(event);
            }
            out
        });
        match timeout(Duration::from_secs(90), done_rx).await {
            Ok(Ok(Ok(()))) => eprintln!("turn: Ok"),
            Ok(Ok(Err(err))) => eprintln!("turn: Err({err})"),
            Ok(Err(_)) => eprintln!("turn: actor dropped the done channel"),
            Err(_) => panic!("session HUNG past the 90s wall-clock guard"),
        }
        collector.await.expect("collector task")
    }

    /// Live proof that L6 recovers cursor's context window from the modelId params.
    ///
    /// cursor 是唯一把窗口写在 modelId 方括号里的 CLI（实测 32 个模型中 13 个带
    /// `context=`，且全都没有 `_meta.totalContextTokens`）。这条跑真实 `cursor-agent acp`
    /// 探测，断言确实有模型解析出了窗口——改动前这些全是 None，分母只能吃 200K 兜底。
    #[tokio::test]
    #[ignore = "requires live cursor-agent login + network"]
    async fn cursor_models_expose_context_window_from_model_id() {
        let bin = which_bin("cursor-agent").expect("cursor-agent on PATH");
        let cwd = std::env::temp_dir();
        let probe = detect_acp_models(&bin, &["acp"], &cwd, 60)
            .await
            .expect("cursor acp model probe");

        let with_window: Vec<_> = probe
            .models
            .iter()
            .filter(|m| m.context_window_tokens.is_some())
            .collect();
        eprintln!(
            "cursor models: {} total, {} with a window",
            probe.models.len(),
            with_window.len()
        );
        for m in with_window.iter().take(8) {
            eprintln!("  {} -> {:?}", m.id, m.context_window_tokens);
        }
        assert!(
            !with_window.is_empty(),
            "no cursor model exposed a context window; modelId parsing regressed"
        );
        // 实测样本里 claude-opus-5 是 300k、gpt-5.6-sol 是 272k，都应落在合理区间。
        for m in &with_window {
            let w = m.context_window_tokens.unwrap();
            assert!(
                (100_000..=2_000_000).contains(&w),
                "implausible window {w} for {}",
                m.id
            );
        }
    }

    /// Live end-to-end proof that the ACP usage channel (L1/L2) works against a real CLI.
    ///
    /// opencode 会发 `usage_update`。分母来自 size，轮末计费事件也必须保留最近的上下文。
    /// 计费 total 仍遵守 opencode 的 cache 口径。
    #[tokio::test]
    #[ignore = "requires live opencode login + network"]
    async fn opencode_acp_reports_usage_and_context_window() {
        let Some(bin) = which_bin("opencode") else {
            eprintln!("SKIP: opencode 不在 PATH 上");
            return;
        };
        let cwd = std::env::temp_dir();
        // 走生产用的常驻 actor（此前驱动的是只被真机测试吊着命的一次性 `run_acp_session`）。
        let session = match AcpSession::connect(
            &bin,
            &["acp".to_string()],
            &cwd,
            None,
            None,
            None,
            &[],
            None,
            &[],
        )
        .await
        {
            Ok(session) => session,
            Err(err) => {
                eprintln!("SKIP: 连接失败（未登录 / 网络？）：{err}");
                return;
            }
        };
        let control = spawn_acp_session_actor(session);
        // 这里不用 `run_one_live_turn`：它的墙钟超时是 panic，而 opencode 密钥失效导致的
        // 挂起是**环境**问题不是本层回归，必须按 SKIP 处理。
        let (events_tx, mut events_rx) = tokio::sync::mpsc::channel::<UnifiedAgentEvent>(256);
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        control
            .send(SessionCommand::RunTurn {
                prompt: "Reply with exactly the token USAGE_OK and nothing else.".to_string(),
                model: None,
                reasoning: None,
                images: Vec::new(),
                extra_writable_roots: Vec::new(),
                events: events_tx,
                done: done_tx,
                approvals: None,
            })
            .await
            .expect("actor alive");
        let collector = tokio::spawn(async move {
            let mut out = Vec::new();
            while let Some(event) = events_rx.recv().await {
                out.push(event);
            }
            out
        });
        let result = timeout(Duration::from_secs(120), done_rx).await;
        let captured = collector.await.expect("collector task");
        if result.is_err() {
            eprintln!("SKIP: opencode ACP 会话在 120s 内没有完成（多为密钥失效/网络问题）。先跑 `opencode run hi` 确认 CLI 本身可用。");
            return;
        }

        let usages: Vec<&crate::chat::model::ModelUsage> = captured
            .iter()
            .filter_map(|e| match e {
                UnifiedAgentEvent::Usage { usage } => Some(usage),
                _ => None,
            })
            .collect();
        for u in &usages {
            eprintln!(
                "opencode usage: input={:?} output={:?} cache_read={:?} reasoning={:?} \
                 total={:?} context={:?} window={:?}",
                u.input_tokens,
                u.output_tokens,
                u.cached_input_tokens,
                u.reasoning_tokens,
                u.total_tokens,
                u.context_tokens,
                u.context_window_tokens,
            );
        }
        assert!(
            !usages.is_empty(),
            "opencode reported no usage at all — the ACP usage channel regressed \
             (若 `opencode run hi` 也报 Invalid API Key，先修密钥再看这条)"
        );

        // 分母：来自 usage_update.size。
        let window = usages.iter().find_map(|u| u.context_window_tokens);
        assert!(
            window.is_some_and(|w| w > 0),
            "no context window from usage_update.size; usages={usages:?}"
        );

        let final_billing = usages
            .iter()
            .rev()
            .find(|usage| usage.total_tokens.is_some())
            .expect("no prompt billing usage");
        assert!(
            final_billing.context_tokens.is_some_and(|used| used > 0),
            "prompt billing lost the context snapshot: {final_billing:?}"
        );

        // 计费 total 仍须保留 opencode 自报的 cache。
        let counts_cache = usages.iter().any(|u| {
            let cache = u.cached_input_tokens.unwrap_or(0);
            let plain = u.input_tokens.unwrap_or(0) + u.output_tokens.unwrap_or(0);
            cache > 0 && u.total_tokens.unwrap_or(0) > plain
        });
        assert!(
            counts_cache,
            "no usage report counted cache tokens into total; usages={usages:?}"
        );
    }
}

#[cfg(test)]
mod live_reasoning_tests {
    use super::*;

    /// 真机跑 `kimi acp`，确认档位选项与当前值都能探到。
    /// 单测喂构造样本，这条证明真实 CLI 也给 —— 而且是 defs 静态表拿不到的
    /// （`acp_def` 把 reasoning_options 写死成空，所有 ACP CLI 共用）。
    #[tokio::test]
    #[ignore = "requires a real kimi CLI on this machine"]
    async fn live_kimi_exposes_reasoning_options() {
        // kimi 常装在 ~/.kimi-code/bin（不一定在 PATH 上），两处都找。
        let bin = crate::external_agents::spawn::resolve_binary(
            crate::external_agents::registry::get_agent_def("kimi").expect("kimi def"),
        )
        .await
        .or_else(|| {
            let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
            let p = home.join(".kimi-code").join("bin").join("kimi");
            p.exists().then_some(p)
        })
        .expect("kimi on PATH or in ~/.kimi-code/bin");
        let probe = detect_acp_models(&bin, &["acp"], &std::env::temp_dir(), 60)
            .await
            .expect("kimi acp probe");
        eprintln!("current_model     = {:?}", probe.current_model);
        eprintln!("current_reasoning = {:?}", probe.current_reasoning);
        eprintln!(
            "reasoning_options = {:?}",
            probe
                .reasoning_options
                .iter()
                .map(|o| format!("{}/{}", o.id, o.label))
                .collect::<Vec<_>>()
        );
        assert!(
            !probe.reasoning_options.is_empty(),
            "kimi 应上报推理档位（实测 low/high/max），拿到空说明发现侧回归了"
        );
        assert!(
            probe.current_reasoning.is_some(),
            "kimi 的 configOptions 带 currentValue，当前档位不该是 None"
        );
    }
}

// -------------------------------------------------------------------------------------------
// 导入用：ACP 会话枚举
// -------------------------------------------------------------------------------------------

/// `session/list` 返回的一条会话（只取导入列表要用的字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct AcpSessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub title: Option<String>,
}

/// ACP `session/list` 探针：起进程 → `initialize` → 按 `cwd` 翻页拉会话 → 关掉。
///
/// **只在导入列表里用**，不参与聊天路径。
///
/// 返回 `None` 表示**该 agent 不支持导入**——要么 `initialize` 没声明 `loadSession`
/// （导进来也续不了聊，本机 gemini 就是这样），要么 `session/list` 回 `Method not found`。
/// 返回 `Some(vec![])` 是"支持但这个目录下没有会话"，两者含义不同，前端展示也不同。
///
/// `cwd` 必须传给 agent：不传的话它返回的是**全局**最近会话，翻页上限会在够到本目录的会话
/// 之前就截断（这个坑 paseo 也踩过并写在注释里）。
pub async fn probe_acp_sessions(
    bin: &Path,
    args: &[&str],
    cwd: &Path,
    timeout_secs: u64,
) -> Option<Vec<AcpSessionSummary>> {
    // ponytail: 每次列表都起一次进程握手（实测 opencode ~2s）。导入入口是低频动作，
    // 不做连接复用；真觉得慢再挂到 detection 的缓存上。
    let mut child = crate::external_agents::spawn::cli_command(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .map(crate::external_agents::spawn::AgentChild::from)
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout).lines();

    let deadline = Duration::from_secs(timeout_secs);
    let started = std::time::Instant::now();

    write_rpc(&mut stdin, 1, "initialize", acp_initialize_params(false))
        .await
        .ok()?;

    let init = read_rpc_response(&mut reader, 1, started, deadline).await?;
    let supports_load = init
        .get("result")
        .and_then(|r| r.get("agentCapabilities"))
        .and_then(|c| c.get("loadSession"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !supports_load {
        let _ = child.start_kill();
        return None;
    }

    let cwd_text = cli_protocol_cwd(bin, cwd).to_string_lossy().into_owned();
    let mut sessions = Vec::new();
    let mut cursor: Option<String> = None;
    let mut next_id: u64 = 2;
    // 翻页上限：防止 agent 返回一个永不为空的游标把我们困在这里。
    for _ in 0..20 {
        let mut params = json!({ "cwd": cwd_text });
        if let Some(c) = cursor.as_ref() {
            params["cursor"] = json!(c);
        }
        let id = next_id;
        next_id += 1;
        if write_rpc(&mut stdin, id, "session/list", params)
            .await
            .is_err()
        {
            break;
        }
        let Some(response) = read_rpc_response(&mut reader, id, started, deadline).await else {
            break;
        };
        if response.get("error").is_some() {
            // `Method not found` ⇒ 这个 agent 根本没有会话枚举，按"不支持导入"处理。
            let _ = child.start_kill();
            return None;
        }
        let Some(result) = response.get("result") else {
            break;
        };
        if let Some(items) = result.get("sessions").and_then(|v| v.as_array()) {
            for item in items {
                let Some(session_id) = item.get("sessionId").and_then(|v| v.as_str()) else {
                    continue;
                };
                sessions.push(AcpSessionSummary {
                    session_id: session_id.to_string(),
                    cwd: item
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&cwd_text)
                        .to_string(),
                    title: item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string),
                });
            }
        }
        cursor = result
            .get("nextCursor")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }

    let _ = child.start_kill();
    Some(sessions)
}

/// 读到指定 id 的 JSON-RPC 响应为止，途中的通知一律丢弃。超时或流结束返回 `None`。
async fn read_rpc_response(
    reader: &mut Lines<BufReader<ChildStdout>>,
    want_id: u64,
    started: std::time::Instant,
    deadline: Duration,
) -> Option<Value> {
    loop {
        if started.elapsed() > deadline {
            return None;
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) | Ok(Err(_)) => return None,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("id").and_then(|v| v.as_u64()) == Some(want_id) {
            return Some(value);
        }
    }
}

/// 用 ACP `session/load` 把一条既有会话的历史重放出来，收集期间推送的 `session/update`。
///
/// 只在**导入**时用。返回的是原始 update 负载，交给 `import_history::parse_acp_updates`
/// 转成 `ChatMessage`——协议细节留在本模块，消息形状的事归那边。
///
/// 返回 `None` = 加载失败或该 agent 不支持；`Some(vec![])` = 加载成功但**不重放历史**
/// （kimi 实测就是这样）。两者调用方要区别对待。
pub async fn probe_acp_session_history(
    bin: &Path,
    args: &[&str],
    cwd: &Path,
    session_id: &str,
    timeout_secs: u64,
) -> Option<Vec<Value>> {
    let mut child = crate::external_agents::spawn::cli_command(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .map(crate::external_agents::spawn::AgentChild::from)
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout).lines();

    let deadline = Duration::from_secs(timeout_secs);
    let started = std::time::Instant::now();

    write_rpc(&mut stdin, 1, "initialize", acp_initialize_params(false))
        .await
        .ok()?;
    read_rpc_response(&mut reader, 1, started, deadline).await?;

    write_rpc(
        &mut stdin,
        2,
        "session/load",
        json!({
            "sessionId": session_id,
            "cwd": cli_protocol_cwd(bin, cwd).to_string_lossy(),
            "mcpServers": [],
        }),
    )
    .await
    .ok()?;

    // 重放的通知**大部分在响应之前**到达，但不能只读到响应就停——实测部分 agent 会在
    // 响应之后再补推。所以收到响应后再多听一小段（tail window），否则会漏掉尾巴上的消息。
    let mut updates = Vec::new();
    let mut got_response = false;
    let mut tail_deadline: Option<std::time::Instant> = None;
    loop {
        if started.elapsed() > deadline {
            break;
        }
        if let Some(tail) = tail_deadline {
            if std::time::Instant::now() > tail {
                break;
            }
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) | Ok(Err(_)) => break,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if value.get("id").and_then(|v| v.as_u64()) == Some(2) {
            if value.get("error").is_some() {
                let _ = child.start_kill();
                return None;
            }
            got_response = true;
            tail_deadline = Some(std::time::Instant::now() + Duration::from_secs(3));
            continue;
        }
        if value.get("method").and_then(|v| v.as_str()) == Some("session/update") {
            if let Some(update) = value.get("params").and_then(|p| p.get("update")) {
                updates.push(update.clone());
            }
        }
    }

    let _ = child.start_kill();
    got_response.then_some(updates)
}
