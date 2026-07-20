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
    default_model_option, ExternalCliSlashCommand, RuntimeModelOption, UnifiedAgentEvent,
};
use crate::proc::NoConsoleWindow;

const ACP_PROTOCOL_VERSION: i64 = 1;

/// Handshake timeouts (缺陷 4 / R3): Paseo uses 60s; desktop starts at 30s. `initialize` and
/// `session/new` each get their own budget so a slow one doesn't starve the other.
const ACP_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);
const ACP_SESSION_NEW_TIMEOUT: Duration = Duration::from_secs(30);
/// Wall-clock guard for the one-shot `run_acp_session` prompt phase (A3): external CLIs run long
/// legitimate tasks, so this only trips on a truly hung stdout — not a normal long turn.
const ACP_ONESHOT_WALL_CLOCK: Duration = Duration::from_secs(600);

use crate::external_agents::spawn::{fold_stderr, join_stderr_tail as join_tail};

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
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
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
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

fn rpc_error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if let Some(message) = error.get("message").and_then(|v| v.as_str()) {
        return Some(message.to_string());
    }
    error.get("code").map(|c| c.to_string())
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
                let id = model.get("modelId").and_then(|v| v.as_str()).unwrap_or("");
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
    let input = usage
        .get("inputTokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("outputTokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if input == 0 && output == 0 {
        None
    } else {
        Some(usage_from_numbers(input, output))
    }
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

/// 一次 ACP turn 的去重状态:正文与思考各持一个消息级游标 + 已发工具 id 集合。
#[derive(Default)]
struct AcpUpdateState {
    text: AcpTextAssembler,
    thought: AcpTextAssembler,
    emitted_tools: HashSet<String>,
}

fn acp_update_text(update: &serde_json::Map<String, Value>) -> &str {
    update
        .get("content")
        .and_then(|c| c.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
}

/// 把一条 ACP `session/update` 映射为事件(text / thought / tool),一次性驱动
/// (`run_acp_session`)与持久驱动(`AcpSession::run_turn`)共用同一份去重逻辑。
fn acp_apply_session_update(
    update: &serde_json::Map<String, Value>,
    state: &mut AcpUpdateState,
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
        _ => {
            // tool_call / tool_call_update 是消息边界:发出工具事件后,重置正文与思考游标,
            // 使其后到来的累积快照被识别为新消息起点。
            if apply_acp_session_update(update, &mut state.emitted_tools, &mut |e| sink(e)) {
                state.text.on_boundary();
                state.thought.on_boundary();
            }
        }
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
    let mut update_state = AcpUpdateState::default();
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

    // A3: wall-clock guard so a hung stdout (prompt response never arrives / EOF never comes)
    // can't spin the loop forever and later block `child.wait()`.
    let wall_clock_start = std::time::Instant::now();

    while !finished {
        if cancel_check() {
            if let Some(ref sid) = session_id {
                let _ = write_rpc(
                    &mut stdin,
                    next_id,
                    "session/cancel",
                    json!({ "sessionId": sid }),
                )
                .await;
            }
            let _ = stdin.shutdown().await;
            let _ = child.start_kill();
            return Err("cancelled".to_string());
        }

        if wall_clock_start.elapsed() > ACP_ONESHOT_WALL_CLOCK {
            let _ = stdin.shutdown().await;
            let _ = child.start_kill();
            return Err("ACP session timed out (wall-clock)".to_string());
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

        let value: Value =
            serde_json::from_str(line.trim()).map_err(|e| format!("invalid ACP json: {e}"))?;

        if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
            if method == "session/request_permission" {
                let option_id =
                    choose_permission_outcome(value.get("params").and_then(|p| p.get("options")));
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
                    acp_apply_session_update(update, &mut update_state, &mut sink);
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
                        if id == "model"
                            || option.get("category").and_then(|v| v.as_str()) == Some("model")
                        {
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
            continue;
        }

        if set_model_request_id.is_some()
            && value.get("id").and_then(|v| v.as_u64()) == set_model_request_id
        {
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
            continue;
        }

        if prompt_request_id.is_some()
            && value.get("id").and_then(|v| v.as_u64()) == prompt_request_id
        {
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
                    || category == "thought")
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

        // Fallible handshake, isolated so an error path can kill the child and fold in stderr.
        let handshake = async {
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
            .await
            .map_err(|e| format!("initialize: {e}"))?;
            acp_read_until_id(&mut reader, &mut stdin, 1, ACP_INITIALIZE_TIMEOUT)
                .await
                .map_err(|e| format!("initialize: {e}"))?;

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
            write_rpc(&mut stdin, next_id, method, params)
                .await
                .map_err(|e| format!("session-new: {e}"))?;
            let result =
                acp_read_until_id(&mut reader, &mut stdin, next_id, ACP_SESSION_NEW_TIMEOUT)
                    .await
                    .map_err(|e| format!("session-new: {e}"))?;
            next_id += 1;

            let session_id = match resume_session.filter(|s| !s.is_empty()) {
                Some(sid) => sid.to_string(),
                None => result
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| "session-new: invalid session/new response".to_string())?,
            };

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
                let _ =
                    acp_read_until_id(&mut reader, &mut stdin, next_id, Duration::from_secs(10))
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
                    current_reasoning: normalize_opt(reasoning),
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
        events: &mpsc::Sender<UnifiedAgentEvent>,
        control: &mut mpsc::Receiver<SessionCommand>,
    ) -> Result<(), String> {
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
                        acp_apply_session_update(update, &mut update_state, &mut |e| buf.push(e));
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
        // Child is dead → its stderr hit EOF → the drain task finishes; join it so the task ends.
        let _ = self.stderr_tail.await;
    }
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
                            &events,
                            &mut rx,
                        )
                        .await;
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
        let session = AcpSession::connect(&bin, &["acp".to_string()], &cwd, None, None, &[], None)
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
    fn assembler_passes_through_incremental_deltas() {
        let mut a = AcpTextAssembler::default();
        assert_eq!(a.push_chunk("Hello"), Some("Hello".to_string()));
        assert_eq!(a.push_chunk(" world"), Some(" world".to_string()));
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
    fn assembler_resets_on_boundary_for_new_message() {
        let mut a = AcpTextAssembler::default();
        assert_eq!(a.push_chunk("Hello"), Some("Hello".to_string()));
        a.on_boundary();
        // 新消息累积快照,不以旧全文为前缀 → 视为新消息重置游标。
        assert_eq!(a.push_chunk("Bye"), Some("Bye".to_string()));
        assert_eq!(a.push_chunk("Byebye"), Some("bye".to_string()));
    }

    #[test]
    fn assembler_whole_turn_snapshot_stays_backward_compatible() {
        let mut a = AcpTextAssembler::default();
        assert_eq!(a.push_chunk("Hello"), Some("Hello".to_string()));
        a.on_boundary();
        // 整轮累积语义:边界后的快照仍以旧全文开头 → 不重置,只裁出增量(不重不漏)。
        assert_eq!(a.push_chunk("Hello world"), Some(" world".to_string()));
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
            UnifiedAgentEvent::ThinkingDelta { .. } => "ThinkingDelta",
            UnifiedAgentEvent::ToolUse { .. } => "ToolUse",
            UnifiedAgentEvent::ToolResult { .. } => "ToolResult",
            UnifiedAgentEvent::Usage { .. } => "Usage",
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
