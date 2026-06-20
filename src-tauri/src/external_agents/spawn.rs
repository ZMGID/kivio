use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

use crate::external_agents::types::{PromptInputFormat, RuntimeAgentDef, UnifiedAgentEvent};
use crate::proc::NoConsoleWindow;

pub struct SpawnedAgent {
    pub child: Child,
    pub resolved_bin: PathBuf,
}

pub async fn resolve_binary(def: &RuntimeAgentDef) -> Option<PathBuf> {
    for candidate in std::iter::once(def.bin).chain(def.fallback_bins.iter().copied()) {
        if let Some(path) = which_binary(candidate).await {
            return Some(path);
        }
    }
    None
}

async fn which_binary(name: &str) -> Option<PathBuf> {
    let output = Command::new(if cfg!(windows) { "where" } else { "which" })
        .arg(name)
        .no_console_window()
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if line.is_empty() {
        None
    } else {
        Some(PathBuf::from(line))
    }
}

pub async fn spawn_agent(
    def: &RuntimeAgentDef,
    resolved_bin: &Path,
    args: &[String],
    cwd: &Path,
    extra_env: &HashMap<String, String>,
) -> Result<SpawnedAgent, String> {
    let mut command = Command::new(resolved_bin);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_console_window()
        .kill_on_drop(true);
    for (key, value) in def.env {
        command.env(key, value);
    }
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", def.id))?;
    Ok(SpawnedAgent {
        child,
        resolved_bin: resolved_bin.to_path_buf(),
    })
}

pub async fn write_prompt_stdin(
    child: &mut Child,
    def: &RuntimeAgentDef,
    prompt: &str,
) -> Result<(), String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let mut stdin = stdin;
    match def.prompt_input_format {
        PromptInputFormat::Text => {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            stdin.shutdown().await.map_err(|e| e.to_string())?;
        }
        PromptInputFormat::StreamJson => {
            let content = stream_json_user_content(prompt);
            let line = serde_json::json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": content
                },
                "parent_tool_use_id": null
            });
            let mut payload = serde_json::to_string(&line).map_err(|e| e.to_string())?;
            payload.push('\n');
            stdin
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Minimal stdin write to elicit Claude `system/init` during slash-command probing.
pub async fn write_probe_stdin(child: &mut Child) -> Result<(), String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let mut stdin = stdin;
    let line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": "."
        },
        "parent_tool_use_id": null
    });
    let mut payload = serde_json::to_string(&line).map_err(|e| e.to_string())?;
    payload.push('\n');
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn stream_json_user_content(prompt: &str) -> serde_json::Value {
    if prompt.trim_start().starts_with('/') {
        serde_json::Value::String(prompt.to_string())
    } else {
        serde_json::json!([{ "type": "text", "text": prompt }])
    }
}

pub async fn read_stdout_lines<F>(
    child: &mut Child,
    mut on_line: F,
    cancel_check: impl Fn() -> bool,
) -> Result<(), String>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout unavailable".to_string())?;
    let mut reader = BufReader::new(stdout).lines();
    loop {
        if cancel_check() {
            let _ = child.start_kill();
            return Err("cancelled".to_string());
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
        on_line(&line)?;
    }
    Ok(())
}

pub fn parse_json_line(line: &str) -> Option<serde_json::Value> {
    serde_json::from_str(line.trim()).ok()
}

pub fn emit_from_value(value: &serde_json::Value, sink: &mut dyn FnMut(UnifiedAgentEvent)) {
    if let Some(event) = crate::external_agents::stream::map_json_value(value) {
        sink(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_json_user_content_uses_string_for_slash_commands() {
        let slash = stream_json_user_content("/compact");
        assert_eq!(slash, serde_json::json!("/compact"));
        let text = stream_json_user_content("hello");
        assert_eq!(
            text,
            serde_json::json!([{ "type": "text", "text": "hello" }])
        );
    }
}

