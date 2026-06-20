use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

use crate::external_agents::context::parse_context_window_label;
use crate::external_agents::stream::usage_from_numbers;
use crate::external_agents::types::{
    default_model_option, ExternalCliSlashCommand, RuntimeModelOption, UnifiedAgentEvent,
};
use crate::proc::NoConsoleWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiRpcOutcome {
    Continue,
    AgentEnd,
}

/// Discover Pi slash commands via the RPC `get_commands` request.
/// Response shape: `{type:"response", command:"get_commands", data:{commands:[{name, description}]}}`.
pub async fn detect_pi_commands(
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

    let req = json!({ "id": 1, "type": "get_commands" }).to_string();
    stdin.write_all(format!("{req}\n").as_bytes()).await.ok()?;

    let started = std::time::Instant::now();
    let mut commands: Option<Vec<ExternalCliSlashCommand>> = None;
    loop {
        if started.elapsed() > Duration::from_secs(timeout_secs) {
            break;
        }
        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(l))) => l,
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
        let is_get_commands = value.get("type").and_then(|v| v.as_str()) == Some("response")
            && value.get("command").and_then(|v| v.as_str()) == Some("get_commands");
        if !is_get_commands {
            continue;
        }
        let list = value
            .get("data")
            .and_then(|d| d.get("commands"))
            .and_then(|v| v.as_array());
        if let Some(list) = list {
            let mut out = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for raw in list {
                let Some(name) = raw
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                if seen.insert(name.to_string()) {
                    out.push(ExternalCliSlashCommand {
                        slash: format!("/{name}"),
                        name: name.to_string(),
                        description: raw
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|d| d.trim().to_string())
                            .filter(|d| !d.is_empty()),
                        argument_hint: None,
                    });
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            commands = Some(out);
        }
        break;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
    commands.filter(|c| !c.is_empty())
}

const FIRE_AND_FORGET: &[&str] = &[
    "setStatus",
    "setWidget",
    "notify",
    "setTitle",
    "set_editor_text",
];

pub fn parse_pi_models(stderr: &str) -> Option<Vec<RuntimeModelOption>> {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if lines.len() <= 1 {
        return None;
    }
    let mut out = vec![default_model_option()];
    let mut seen = std::collections::HashSet::from(["default".to_string()]);
    for line in lines.iter().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let full_id = format!("{}/{}", parts[0], parts[1]);
        if seen.insert(full_id.clone()) {
            let context_window_tokens = parts.get(2).and_then(|label| parse_context_window_label(label));
            out.push(RuntimeModelOption {
                id: full_id.clone(),
                label: full_id,
                context_window_tokens,
            });
        }
    }
    if out.len() > 1 {
        Some(out)
    } else {
        None
    }
}

pub fn map_pi_rpc_event(value: &Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) -> PiRpcOutcome {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return PiRpcOutcome::Continue,
    };
    let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match kind {
        "agent_start" => {
            sink(UnifiedAgentEvent::Status {
                label: "working".to_string(),
                model: None,
            });
        }
        "agent_end" => return PiRpcOutcome::AgentEnd,
        "turn_start" => {
            sink(UnifiedAgentEvent::Status {
                label: "thinking".to_string(),
                model: None,
            });
        }
        "turn_end" => {
            if let Some(message) = obj.get("message").and_then(|v| v.as_object()) {
                if let Some(usage) = message.get("usage").and_then(|v| v.as_object()) {
                    let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
                    let output = usage.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
                    if input > 0 || output > 0 {
                        sink(UnifiedAgentEvent::Usage {
                            usage: usage_from_numbers(input, output),
                        });
                    }
                }
                if message.get("stopReason").and_then(|v| v.as_str()) == Some("error") {
                    let message_text = message
                        .get("errorMessage")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Pi agent error");
                    sink(UnifiedAgentEvent::Error {
                        message: message_text.to_string(),
                        code: None,
                    });
                }
            }
        }
        "message_update" => {
            if let Some(ev) = obj.get("assistantMessageEvent").and_then(|v| v.as_object()) {
                let ev_type = ev.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match ev_type {
                    "text_delta" => {
                        if let Some(delta) = ev.get("delta").and_then(|v| v.as_str()) {
                            sink(UnifiedAgentEvent::TextDelta {
                                delta: delta.to_string(),
                            });
                        }
                    }
                    "thinking_delta" => {
                        if let Some(delta) = ev.get("delta").and_then(|v| v.as_str()) {
                            sink(UnifiedAgentEvent::ThinkingDelta {
                                delta: delta.to_string(),
                            });
                        }
                    }
                    "error" => {
                        let message = ev
                            .get("reason")
                            .or_else(|| ev.get("delta"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Agent error");
                        sink(UnifiedAgentEvent::Error {
                            message: message.to_string(),
                            code: None,
                        });
                    }
                    _ => {}
                }
            }
        }
        "tool_execution_start" => {
            let id = obj
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = obj
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let input = obj.get("args").cloned().unwrap_or(Value::Null);
            if !id.is_empty() && !name.is_empty() {
                sink(UnifiedAgentEvent::ToolUse { id, name, input });
            }
        }
        "tool_execution_end" => {
            let tool_use_id = obj
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let result = obj.get("result").and_then(|v| v.as_object());
            let content = result
                .and_then(|r| r.get("content"))
                .map(|c| match c {
                    Value::String(s) => s.clone(),
                    _ => c.to_string(),
                })
                .unwrap_or_default();
            let is_error = obj.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
            if !tool_use_id.is_empty() {
                sink(UnifiedAgentEvent::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                });
            }
        }
        "extension_error" => {
            let message = obj
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Extension error");
            sink(UnifiedAgentEvent::Error {
                message: message.to_string(),
                code: None,
            });
        }
        "auto_retry_end" if obj.get("success").and_then(|v| v.as_bool()) == Some(false) => {
            let message = obj
                .get("finalError")
                .and_then(|v| v.as_str())
                .unwrap_or("Auto-retry exhausted");
            sink(UnifiedAgentEvent::Error {
                message: message.to_string(),
                code: None,
            });
        }
        "compaction_start" => {
            sink(UnifiedAgentEvent::Status {
                label: "compacting".to_string(),
                model: None,
            });
        }
        "auto_retry_start" => {
            sink(UnifiedAgentEvent::Status {
                label: "retrying".to_string(),
                model: None,
            });
        }
        _ => {}
    }
    PiRpcOutcome::Continue
}

async fn reply_extension_ui(
    stdin: &mut tokio::process::ChildStdin,
    raw: &Value,
) -> Result<(), String> {
    let id = raw.get("id").cloned();
    if id.is_none() {
        return Ok(());
    }
    if let Some(method) = raw.get("method").and_then(|v| v.as_str()) {
        if FIRE_AND_FORGET.contains(&method) {
            return Ok(());
        }
    }
    let result = if raw.get("method").and_then(|v| v.as_str()) == Some("confirm") {
        json!({ "confirmed": true })
    } else {
        let opts = raw
            .get("params")
            .and_then(|p| p.get("options"))
            .or_else(|| raw.get("options"))
            .and_then(|v| v.as_array());
        if let Some(opts) = opts {
            if let Some(first) = opts.first() {
                let value = first
                    .as_str()
                    .map(|s| s.to_string())
                    .or_else(|| {
                        first
                            .as_object()
                            .and_then(|o| o.get("label").or_else(|| o.get("value")))
                            .and_then(|v| v.as_str())
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                json!({ "value": value })
            } else {
                json!({ "cancelled": true })
            }
        } else {
            json!({ "cancelled": true })
        }
    };
    let mut payload = json!({ "type": "extension_ui_response", "id": id });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(result_obj) = result.as_object() {
            for (k, v) in result_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    let mut line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin.write_all(line.as_bytes()).await.map_err(|e| e.to_string())
}

pub async fn run_pi_rpc_session(
    child: &mut Child,
    prompt: &str,
    model: Option<&str>,
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

    if let Some(model) = model.filter(|m| !m.is_empty() && *m != "default") {
        sink(UnifiedAgentEvent::Status {
            label: "initializing".to_string(),
            model: Some(model.to_string()),
        });
    } else {
        sink(UnifiedAgentEvent::Status {
            label: "initializing".to_string(),
            model: None,
        });
    }

    let prompt_line = {
        let payload = json!({ "id": 1, "type": "prompt", "message": prompt });
        let mut line = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        line.push('\n');
        line
    };
    stdin
        .write_all(prompt_line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stdout).lines();
    let mut finished = false;

    loop {
        if cancel_check() {
            let _ = child.start_kill();
            return Err("cancelled".to_string());
        }
        if finished {
            break;
        }

        let line = match timeout(Duration::from_millis(200), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => break,
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

        if value.get("type").and_then(|v| v.as_str()) == Some("extension_ui_request") {
            reply_extension_ui(&mut stdin, &value).await?;
            continue;
        }

        if value.get("type").and_then(|v| v.as_str()) == Some("response") {
            if value.get("success").and_then(|v| v.as_bool()) == Some(false) {
                let err = value
                    .get("error")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "prompt rejected".to_string());
                return Err(err);
            }
            continue;
        }

        if map_pi_rpc_event(&value, &mut sink) == PiRpcOutcome::AgentEnd {
            finished = true;
            let _ = stdin.shutdown().await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires live pi CLI on PATH"]
    async fn live_detect_pi_commands() {
        let bin = std::process::Command::new("which")
            .arg("pi")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
            .expect("pi on PATH");
        let cmds = detect_pi_commands(&bin, &["--mode", "rpc"], &std::env::temp_dir(), 10)
            .await
            .expect("pi get_commands");
        eprintln!("pi commands: {}", cmds.len());
        for c in cmds.iter().take(8) {
            eprintln!("  {}", c.slash);
        }
        assert!(!cmds.is_empty());
    }

    #[test]
    fn parse_pi_models_from_tsv() {
        let stderr = "provider model context\nanthropic claude-sonnet-4-5 200K\nopenai gpt-5 128K";
        let models = parse_pi_models(stderr).unwrap();
        assert!(models.iter().any(|m| m.id == "anthropic/claude-sonnet-4-5"));
        assert!(models.iter().any(|m| m.id == "openai/gpt-5"));
        let claude = models
            .iter()
            .find(|m| m.id == "anthropic/claude-sonnet-4-5")
            .unwrap();
        assert_eq!(claude.context_window_tokens, Some(200_000));
    }

    #[test]
    fn map_pi_text_delta() {
        let raw = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"hi"}}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        let mut events = Vec::new();
        map_pi_rpc_event(&value, &mut |e| events.push(e));
        assert!(matches!(
            events.first(),
            Some(UnifiedAgentEvent::TextDelta { delta }) if delta == "hi"
        ));
    }

    #[test]
    fn map_pi_agent_end() {
        let raw = r#"{"type":"agent_end"}"#;
        let value: Value = serde_json::from_str(raw).unwrap();
        assert_eq!(
            map_pi_rpc_event(&value, &mut |_| {}),
            PiRpcOutcome::AgentEnd
        );
    }

    #[test]
    fn parse_pi_models_real_aligned_table() {
        // Real `pi --list-models` output: header + 6 space-aligned columns.
        let out = "provider          model          context  max-out  thinking  images\n\
                   zmfooogreencloud  mimo-v2.5-pro  128K     8.2K     no        no\n\
                   zmfooogreencloud  minimax-m2.7   128K     8.2K     no        no";
        let models = parse_pi_models(out).unwrap();
        assert!(models.iter().any(|m| m.id == "zmfooogreencloud/mimo-v2.5-pro"));
        assert!(models.iter().any(|m| m.id == "zmfooogreencloud/minimax-m2.7"));
        // Generic provider models must NOT appear (those were the bogus fallback).
        assert!(!models.iter().any(|m| m.id.starts_with("anthropic/")));
    }
}
