use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::external_agents::session::live::SessionCommand;
use crate::external_agents::stream::usage_from_numbers;
use crate::external_agents::types::{
    ExternalCliSlashCommand, RuntimeModelOption, UnifiedAgentEvent, default_model_option,
};
use crate::proc::NoConsoleWindow;

const ACP_PROTOCOL_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct AcpMcpServer {
    pub server_type: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

fn build_session_new_params(cwd: &Path, mcp_servers: &[AcpMcpServer]) -> Value {
    let servers: Vec<Value> = mcp_servers
        .iter()
        .map(|s| {
            json!({
                "type": s.server_type,
                "name": s.name,
                "command": s.command,
                "args": s.args,
                "env": s.env.iter().map(|(name, value)| json!({ "name": name, "value": value })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "cwd": cwd.to_string_lossy(),
        "mcpServers": servers,
    })
}

async fn write_rpc(
    stdin: &mut tokio::process::ChildStdin,
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
    stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())
}

async fn write_rpc_result(
    stdin: &mut tokio::process::ChildStdin,
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
    stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())
}

fn rpc_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if let Some(message) = error.get("message").and_then(|v| v.as_str()) {
        return Some(message.to_string());
    }
    error
        .get("code")
        .map(|c| c.to_string())
}

fn normalize_models(result: &Value) -> Vec<RuntimeModelOption> {
    let mut out = vec![default_model_option()];
    let mut seen = HashSet::from(["default".to_string()]);

    if let Some(config_options) = result.get("configOptions").and_then(|v| v.as_array()) {
        for raw_option in config_options {
            let option = match raw_option.as_object() {
                Some(o) => o,
                None => continue,
            };
            let config_id = option.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if config_id != "model" && option.get("category").and_then(|v| v.as_str()) != Some("model") {
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
                    out.push(RuntimeModelOption {
                        id: id.to_string(),
                        label: if name != id {
                            format!("{name} ({id})")
                        } else {
                            id.to_string()
                        },
                        context_window_tokens: None,
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
                let id = model
                    .get("modelId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if id.is_empty() || !seen.insert(id.to_string()) {
                    continue;
                }
                let name = model.get("name").and_then(|v| v.as_str()).unwrap_or(id);
                out.push(RuntimeModelOption {
                    id: id.to_string(),
                    label: if name != id {
                        format!("{name} ({id})")
                    } else {
                        id.to_string()
                    },
                    context_window_tokens: None,
                });
            }
        }
    }

    out
}

pub async fn detect_acp_models(
    bin: &Path,
    args: &[&str],
    cwd: &Path,
    timeout_secs: u64,
) -> Option<Vec<RuntimeModelOption>> {
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout).lines();

    let mut expected_id: u64 = 1;
    let mut next_id: u64 = 2;
    let mut models: Option<Vec<RuntimeModelOption>> = None;
    let deadline = Duration::from_secs(timeout_secs);

    write_rpc(&mut stdin, 1, "initialize", json!({
        "protocolVersion": ACP_PROTOCOL_VERSION,
        "clientCapabilities": { "terminal": false },
        "clientInfo": { "name": "kivio", "version": "external-agents" },
    }))
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
        let value: Value = serde_json::from_str(line.trim()).ok()?;
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
                build_session_new_params(cwd, &[]),
            )
            .await
            .ok()?;
            next_id += 1;
            continue;
        }
        if expected_id == 2 {
            models = Some(normalize_models(result));
            let _ = child.start_kill();
            break;
        }
    }

    models.filter(|m| m.len() > 1)
}

fn parse_available_commands(update: &serde_json::Map<String, Value>) -> Vec<ExternalCliSlashCommand> {
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
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let mut reader = BufReader::new(stdout).lines();

    let mut expected_id: u64 = 1;
    let mut next_id: u64 = 2;
    let mut commands: Option<Vec<ExternalCliSlashCommand>> = None;
    let deadline = Duration::from_secs(timeout_secs);

    write_rpc(&mut stdin, 1, "initialize", json!({
        "protocolVersion": ACP_PROTOCOL_VERSION,
        "clientCapabilities": { "terminal": false },
        "clientInfo": { "name": "kivio", "version": "external-agents" },
    }))
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
                build_session_new_params(cwd, &[]),
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

fn choose_permission_outcome(options: Option<&Value>) -> Option<String> {
    let list = options.and_then(|v| v.as_array())?;
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
    for item in list {
        if item.get("kind").and_then(|v| v.as_str()) == Some("allow_once") {
            if let Some(id) = item.get("optionId").and_then(|v| v.as_str()) {
                return Some(id.to_string());
            }
        }
    }
    None
}

fn format_acp_usage(usage: &Value) -> Option<crate::chat::model::ModelUsage> {
    let input = usage.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = usage.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
    if input == 0 && output == 0 {
        None
    } else {
        Some(usage_from_numbers(input, output))
    }
}

fn acp_update_status(update: &serde_json::Map<String, Value>) -> Option<String> {
    update.get("status").and_then(|v| v.as_str()).map(|status| {
        status
            .trim()
            .to_lowercase()
            .replace([' ', '-'], "_")
    })
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
    matches!(
        status,
        "completed" | "complete" | "succeeded" | "success"
    )
}

fn acp_is_terminal_failure(status: &str) -> bool {
    matches!(
        status,
        "failed" | "failure" | "error" | "cancelled" | "canceled"
    )
}

fn acp_result_content(update: &serde_json::Map<String, Value>) -> String {
    update
        .get("content")
        .or_else(|| update.get("output"))
        .or_else(|| update.get("result"))
        .map(|value| {
            if let Some(text) = value.as_str() {
                text.to_string()
            } else {
                value.to_string()
            }
        })
        .unwrap_or_else(|| acp_tool_name(update))
}

fn apply_acp_session_update(
    update: &serde_json::Map<String, Value>,
    emitted_tool_ids: &mut HashSet<String>,
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
            if !emitted_tool_ids.contains(&id) {
                emitted_tool_ids.insert(id.clone());
                sink(UnifiedAgentEvent::ToolUse {
                    id: id.clone(),
                    name: acp_tool_name(update),
                    input: Value::Object(update.clone()),
                });
            }
            if let Some(status) = acp_update_status(update) {
                if acp_is_terminal_success(&status) || acp_is_terminal_failure(&status) {
                    sink(UnifiedAgentEvent::ToolResult {
                        tool_use_id: id,
                        content: acp_result_content(update),
                        is_error: acp_is_terminal_failure(&status),
                    });
                }
            }
            true
        }
        _ => false,
    }
}

pub async fn run_acp_session(
    child: &mut Child,
    prompt: &str,
    cwd: &Path,
    model: Option<&str>,
    mcp_servers: &[AcpMcpServer],
    mut sink: impl FnMut(UnifiedAgentEvent),
    cancel_check: impl Fn() -> bool,
) -> Result<(), String> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout unavailable".to_string())?;

    let mut expected_id: u64 = 1;
    let mut next_id: u64 = 2;
    let mut session_id: Option<String> = None;
    let mut prompt_request_id: Option<u64> = None;
    let mut set_model_request_id: Option<u64> = None;
    let mut model_config_id: Option<String> = None;
    let mut emitted_text = String::new();
    let mut emitted_acp_tools = HashSet::new();
    let mut finished = false;

    write_rpc(
        &mut stdin,
        1,
        "initialize",
        json!({
            "protocolVersion": ACP_PROTOCOL_VERSION,
            "clientCapabilities": { "terminal": false },
            "clientInfo": { "name": "kivio", "version": "external-agents" },
        }),
    )
    .await?;

    let mut reader = BufReader::new(stdout).lines();

    while !finished {
        if cancel_check() {
            if let Some(ref sid) = session_id {
                let _ = write_rpc(&mut stdin, next_id, "session/cancel", json!({ "sessionId": sid })).await;
            }
            let _ = stdin.shutdown().await;
            let _ = child.start_kill();
            return Err("cancelled".to_string());
        }

        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => {
                if !finished {
                    return Err("ACP session exited before completion".to_string());
                }
                break;
            }
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(line.trim())
            .map_err(|e| format!("invalid ACP json: {e}"))?;

        if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
            if method == "session/request_permission" {
                let option_id = choose_permission_outcome(value.get("params").and_then(|p| p.get("options")));
                if let (Some(id), Some(option_id)) = (value.get("id"), option_id) {
                    write_rpc_result(
                        &mut stdin,
                        id,
                        json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
                    )
                    .await?;
                }
                continue;
            }
            if method == "session/update" {
                if let Some(update) = value
                    .get("params")
                    .and_then(|p| p.get("update"))
                    .and_then(|v| v.as_object())
                {
                    let session_update = update
                        .get("sessionUpdate")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if session_update == "agent_thought_chunk" {
                        if let Some(text) = update
                            .get("content")
                            .and_then(|c| c.get("text"))
                            .and_then(|v| v.as_str())
                        {
                            if !text.is_empty() {
                                sink(UnifiedAgentEvent::ThinkingDelta {
                                    delta: text.to_string(),
                                });
                            }
                        }
                        continue;
                    }
                    if session_update == "agent_message_chunk" {
                        if let Some(text) = update
                            .get("content")
                            .and_then(|c| c.get("text"))
                            .and_then(|v| v.as_str())
                        {
                            if !text.is_empty() {
                                let delta = if text.starts_with(&emitted_text) {
                                    text[emitted_text.len()..].to_string()
                                } else {
                                    text.to_string()
                                };
                                if !delta.is_empty() {
                                    emitted_text.push_str(&delta);
                                    sink(UnifiedAgentEvent::TextDelta { delta });
                                }
                            }
                        }
                        continue;
                    }
                    if apply_acp_session_update(update, &mut emitted_acp_tools, &mut sink) {
                        continue;
                    }
                    if session_update != "agent_message_chunk"
                        && session_update != "agent_thought_chunk"
                    {
                        sink(UnifiedAgentEvent::Status {
                            label: session_update.to_string(),
                            model: None,
                        });
                    }
                }
                continue;
            }
        }

        if let Some(err) = rpc_error_message(&value) {
            if value.get("id").and_then(|v| v.as_u64()) != Some(expected_id) {
                continue;
            }
            return Err(err);
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
                build_session_new_params(cwd, mcp_servers),
            )
            .await?;
            next_id += 1;
            continue;
        }

        if expected_id == 2 {
            session_id = result
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if let Some(config_options) = result.get("configOptions").and_then(|v| v.as_array()) {
                for raw_option in config_options {
                    if let Some(option) = raw_option.as_object() {
                        let id = option.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if id == "model" || option.get("category").and_then(|v| v.as_str()) == Some("model") {
                            model_config_id = Some(id.to_string());
                            break;
                        }
                    }
                }
            }

            let chosen = model.filter(|m| !m.is_empty() && *m != "default");
            if session_id.is_some() && chosen.is_some() {
                set_model_request_id = Some(next_id);
                expected_id = next_id;
                let sid = session_id.clone().unwrap();
                let chosen = chosen.unwrap();
                if model_config_id.is_some() {
                    write_rpc(
                        &mut stdin,
                        next_id,
                        "session/set_config_option",
                        json!({ "sessionId": sid, "configId": model_config_id, "value": chosen }),
                    )
                    .await?;
                } else {
                    write_rpc(
                        &mut stdin,
                        next_id,
                        "session/set_model",
                        json!({ "sessionId": sid, "modelId": chosen }),
                    )
                    .await?;
                }
                next_id += 1;
                continue;
            }

            if session_id.is_none() {
                return Err("invalid session/new response".to_string());
            }

            prompt_request_id = Some(next_id);
            expected_id = next_id;
            write_rpc(
                &mut stdin,
                next_id,
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": prompt }],
                }),
            )
            .await?;
            next_id += 1;
            sink(UnifiedAgentEvent::Status {
                label: "waiting_for_first_output".to_string(),
                model: None,
            });
            continue;
        }

        if set_model_request_id.is_some() && value.get("id").and_then(|v| v.as_u64()) == set_model_request_id {
            set_model_request_id = None;
            prompt_request_id = Some(next_id);
            expected_id = next_id;
            write_rpc(
                &mut stdin,
                next_id,
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": prompt }],
                }),
            )
            .await?;
            next_id += 1;
            sink(UnifiedAgentEvent::Status {
                label: "waiting_for_first_output".to_string(),
                model: None,
            });
            continue;
        }

        if prompt_request_id.is_some() && value.get("id").and_then(|v| v.as_u64()) == prompt_request_id {
            if let Some(usage) = result.get("usage").and_then(format_acp_usage) {
                sink(UnifiedAgentEvent::Usage { usage });
            }
            finished = true;
            let _ = stdin.shutdown().await;
        }
    }

    Ok(())
}

// ===========================================================================================
// Persistent ACP session (Phase 2): keep the agent process alive across turns. Reuses the
// same `apply_acp_session_update` mapping + permission/usage helpers as the one-shot driver.
// ===========================================================================================

/// A live ACP connection: one `session/new` (or `session/load`) + `set_model`, then many
/// `session/prompt` turns over the same process. Owned exclusively by its actor task.
pub struct AcpSession {
    child: Child,
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    session_id: String,
    next_id: u64,
}

impl AcpSession {
    pub async fn connect(
        resolved_bin: &Path,
        args: &[String],
        cwd: &Path,
        model: Option<&str>,
        mcp_servers: &[AcpMcpServer],
        resume_session: Option<&str>,
    ) -> Result<Self, String> {
        let mut child = Command::new(resolved_bin)
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .no_console_window()
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("spawn acp agent: {e}"))?;
        let mut stdin = child.stdin.take().ok_or_else(|| "stdin unavailable".to_string())?;
        let stdout = child.stdout.take().ok_or_else(|| "stdout unavailable".to_string())?;
        let mut reader = BufReader::new(stdout).lines();

        write_rpc(
            &mut stdin,
            1,
            "initialize",
            json!({
                "protocolVersion": ACP_PROTOCOL_VERSION,
                "clientCapabilities": { "terminal": false },
                "clientInfo": { "name": "kivio", "version": "external-agents" },
            }),
        )
        .await?;
        acp_read_until_id(&mut reader, &mut stdin, 1, Duration::from_secs(15)).await?;

        // session/new for a fresh session, session/load to resume a prior one.
        let mut next_id: u64 = 2;
        let (method, params) = match resume_session.filter(|s| !s.is_empty()) {
            Some(sid) => {
                let mut p = build_session_new_params(cwd, mcp_servers);
                p["sessionId"] = json!(sid);
                ("session/load", p)
            }
            None => ("session/new", build_session_new_params(cwd, mcp_servers)),
        };
        write_rpc(&mut stdin, next_id, method, params).await?;
        let result =
            acp_read_until_id(&mut reader, &mut stdin, next_id, Duration::from_secs(20)).await?;
        next_id += 1;

        let session_id = match resume_session.filter(|s| !s.is_empty()) {
            Some(sid) => sid.to_string(),
            None => result
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| "invalid session/new response".to_string())?,
        };

        // Optional model selection (set_config_option / set_model), mirroring run_acp_session.
        if let Some(chosen) = model.filter(|m| !m.is_empty() && *m != "default") {
            let mut model_config_id: Option<String> = None;
            if let Some(config_options) = result.get("configOptions").and_then(|v| v.as_array()) {
                for raw in config_options {
                    if let Some(option) = raw.as_object() {
                        let id = option.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if id == "model"
                            || option.get("category").and_then(|v| v.as_str()) == Some("model")
                        {
                            model_config_id = Some(id.to_string());
                            break;
                        }
                    }
                }
            }
            let (set_method, set_params) = match &model_config_id {
                Some(cfg) => (
                    "session/set_config_option",
                    json!({ "sessionId": session_id, "configId": cfg, "value": chosen }),
                ),
                None => (
                    "session/set_model",
                    json!({ "sessionId": session_id, "modelId": chosen }),
                ),
            };
            write_rpc(&mut stdin, next_id, set_method, set_params).await?;
            // Best-effort: wait for the ack but don't fail the session if the agent ignores it.
            let _ = acp_read_until_id(&mut reader, &mut stdin, next_id, Duration::from_secs(10)).await;
            next_id += 1;
        }

        Ok(Self {
            child,
            stdin,
            reader,
            session_id,
            next_id,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Run one prompt turn over the live session. Emits events into `events`; an incoming
    /// `Cancel` on `control` sends `session/cancel` without killing the process.
    pub async fn run_turn(
        &mut self,
        prompt: &str,
        events: &mpsc::Sender<UnifiedAgentEvent>,
        control: &mut mpsc::Receiver<SessionCommand>,
    ) -> Result<(), String> {
        let prompt_id = self.next_id;
        self.next_id += 1;
        write_rpc(
            &mut self.stdin,
            prompt_id,
            "session/prompt",
            json!({
                "sessionId": self.session_id,
                "prompt": [{ "type": "text", "text": prompt }],
            }),
        )
        .await?;
        let _ = events
            .send(UnifiedAgentEvent::Status {
                label: "waiting_for_first_output".to_string(),
                model: None,
            })
            .await;

        let mut emitted_text = String::new();
        let mut emitted_tools: HashSet<String> = HashSet::new();

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
                Ok(SessionCommand::RunTurn { done, .. }) => {
                    let _ = done.send(Err("session busy".to_string()));
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err("control channel closed".to_string())
                }
            }

            let line = match timeout(Duration::from_millis(200), self.reader.next_line()).await {
                Ok(Ok(Some(l))) => l,
                Ok(Ok(None)) => return Err("ACP session exited mid-turn".to_string()),
                Ok(Err(e)) => return Err(e.to_string()),
                Err(_) => continue,
            };
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
                if method == "session/request_permission" {
                    let option_id = choose_permission_outcome(
                        value.get("params").and_then(|p| p.get("options")),
                    );
                    if let (Some(id), Some(option_id)) = (value.get("id"), option_id) {
                        write_rpc_result(
                            &mut self.stdin,
                            id,
                            json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
                        )
                        .await?;
                    }
                    continue;
                }
                if method == "session/update" {
                    if let Some(update) = value
                        .get("params")
                        .and_then(|p| p.get("update"))
                        .and_then(|v| v.as_object())
                    {
                        let mut buf: Vec<UnifiedAgentEvent> = Vec::new();
                        acp_apply_turn_update(update, &mut emitted_text, &mut emitted_tools, &mut |e| {
                            buf.push(e)
                        });
                        for e in buf {
                            let _ = events.send(e).await;
                        }
                    }
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
                if let Some(usage) = value
                    .get("result")
                    .and_then(|r| r.get("usage"))
                    .and_then(format_acp_usage)
                {
                    let _ = events.send(UnifiedAgentEvent::Usage { usage }).await;
                }
                return Ok(());
            }
        }
    }

    pub async fn close(mut self) {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

/// Map one ACP `session/update` to events (text / thought / tool), shared by the persistent
/// turn loop. Mirrors the inline handling in `run_acp_session`.
fn acp_apply_turn_update(
    update: &serde_json::Map<String, Value>,
    emitted_text: &mut String,
    emitted_tools: &mut HashSet<String>,
    sink: &mut dyn FnMut(UnifiedAgentEvent),
) {
    let session_update = update.get("sessionUpdate").and_then(|v| v.as_str()).unwrap_or("");
    if session_update == "agent_thought_chunk" {
        if let Some(text) = update.get("content").and_then(|c| c.get("text")).and_then(|v| v.as_str())
        {
            if !text.is_empty() {
                sink(UnifiedAgentEvent::ThinkingDelta { delta: text.to_string() });
            }
        }
        return;
    }
    if session_update == "agent_message_chunk" {
        if let Some(text) = update.get("content").and_then(|c| c.get("text")).and_then(|v| v.as_str())
        {
            if !text.is_empty() {
                let delta = if text.starts_with(emitted_text.as_str()) {
                    text[emitted_text.len()..].to_string()
                } else {
                    text.to_string()
                };
                if !delta.is_empty() {
                    emitted_text.push_str(&delta);
                    sink(UnifiedAgentEvent::TextDelta { delta });
                }
            }
        }
        return;
    }
    if apply_acp_session_update(update, emitted_tools, &mut |e| sink(e)) {
        return;
    }
    sink(UnifiedAgentEvent::Status {
        label: session_update.to_string(),
        model: None,
    });
}

/// Read ACP JSON-RPC lines until the response for `target_id`, auto-answering permission
/// requests and skipping notifications.
async fn acp_read_until_id(
    reader: &mut Lines<BufReader<ChildStdout>>,
    stdin: &mut ChildStdin,
    target_id: u64,
    overall: Duration,
) -> Result<Value, String> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > overall {
            return Err("ACP handshake timeout".to_string());
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(l))) => l,
            Ok(Ok(None)) => return Err("ACP agent exited during handshake".to_string()),
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
            if method == "session/request_permission" {
                let option_id =
                    choose_permission_outcome(value.get("params").and_then(|p| p.get("options")));
                if let (Some(id), Some(option_id)) = (value.get("id"), option_id) {
                    write_rpc_result(
                        stdin,
                        id,
                        json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
                    )
                    .await?;
                }
            }
            continue; // notification or handled request
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
}

/// Spawn the actor task owning a connected ACP session.
pub fn spawn_acp_session_actor(mut session: AcpSession) -> mpsc::Sender<SessionCommand> {
    let (tx, mut rx) = mpsc::channel::<SessionCommand>(8);
    tokio::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                SessionCommand::RunTurn {
                    prompt,
                    events,
                    done,
                    ..
                } => {
                    let result = session.run_turn(&prompt, &events, &mut rx).await;
                    let _ = done.send(result);
                }
                SessionCommand::Cancel => {}
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
        let session = AcpSession::connect(&bin, &["acp".to_string()], &cwd, None, &[], None)
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
                    events: etx,
                    done: dtx,
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
        assert!(t2.contains("42"), "turn 2 should recall 42 from turn 1, got: {t2:?}");
        let _ = control.send(SessionCommand::Close).await;
    }

    fn which_bin(name: &str) -> Option<std::path::PathBuf> {
        let out = std::process::Command::new("which").arg(name).output().ok()?;
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
        assert!(apply_acp_session_update(&started, &mut emitted, &mut |event| {
            events.push(event);
        }));
        assert!(apply_acp_session_update(&finished, &mut emitted, &mut |event| {
            events.push(event);
        }));
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

    fn event_variant(event: &UnifiedAgentEvent) -> &'static str {
        match event {
            UnifiedAgentEvent::Status { .. } => "Status",
            UnifiedAgentEvent::TextDelta { .. } => "TextDelta",
            UnifiedAgentEvent::ThinkingDelta { .. } => "ThinkingDelta",
            UnifiedAgentEvent::ToolUse { .. } => "ToolUse",
            UnifiedAgentEvent::ToolResult { .. } => "ToolResult",
            UnifiedAgentEvent::Usage { .. } => "Usage",
            UnifiedAgentEvent::TurnEnd { .. } => "TurnEnd",
            UnifiedAgentEvent::Error { .. } => "Error",
            UnifiedAgentEvent::Raw { .. } => "Raw",
            UnifiedAgentEvent::SlashCommands { .. } => "SlashCommands",
        }
    }

    #[tokio::test]
    #[ignore = "requires live cursor-agent login + network"]
    async fn cursor_acp_smoke() {
        let cwd = std::env::temp_dir();
        let mut child = Command::new("cursor-agent")
            .arg("acp")
            .current_dir(&cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("spawn cursor-agent acp");

        let events = std::cell::RefCell::new(Vec::<UnifiedAgentEvent>::new());
        let result = timeout(
            Duration::from_secs(90),
            run_acp_session(
                &mut child,
                "Reply with exactly the token SMOKE_OK and nothing else.",
                &cwd,
                None,
                &[],
                |event| events.borrow_mut().push(event),
                || false,
            ),
        )
        .await;

        let _ = child.start_kill();
        let captured = events.into_inner();
        eprintln!("=== cursor ACP smoke: {} events ===", captured.len());
        for (i, ev) in captured.iter().enumerate() {
            eprintln!("[{i}] {ev:?}");
        }
        let seq: Vec<&str> = captured.iter().map(event_variant).collect();
        eprintln!("cursor sequence: {seq:?}");
        match &result {
            Ok(Ok(())) => eprintln!("cursor run_acp_session: Ok"),
            Ok(Err(e)) => eprintln!("cursor run_acp_session: Err({e})"),
            Err(_) => panic!("cursor ACP session HUNG past 90s wall-clock guard"),
        }

        let got_text = captured
            .iter()
            .any(|e| matches!(e, UnifiedAgentEvent::TextDelta { .. }));
        let got_error = captured
            .iter()
            .any(|e| matches!(e, UnifiedAgentEvent::Error { .. }))
            || matches!(&result, Ok(Err(_)));
        assert!(
            got_text || got_error,
            "expected at least one TextDelta or an Error/Err round-trip, got: {seq:?}"
        );
    }
}
