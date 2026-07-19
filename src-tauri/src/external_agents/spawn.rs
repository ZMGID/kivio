use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

use crate::external_agents::types::{PromptInputFormat, RuntimeAgentDef};
use crate::proc::NoConsoleWindow;

pub struct SpawnedAgent {
    pub child: Child,
    pub resolved_bin: PathBuf,
}

/// Concurrently drain the child's stderr into a JoinHandle so a CLI that reports failures on
/// stderr doesn't (a) block on a full pipe while we read stdout, and (b) fail silently. Blank
/// lines are dropped and the buffer is capped at `STDERR_CAP_CHARS` (keeping the tail — the last
/// lines are usually the actual error). Call before the stdout read loop; await after `wait()`.
pub fn drain_stderr(child: &mut Child) -> tokio::task::JoinHandle<String> {
    const STDERR_CAP_CHARS: usize = 8192;
    let stderr = child.stderr.take();
    tokio::spawn(async move {
        let Some(stderr) = stderr else {
            return String::new();
        };
        let mut reader = BufReader::new(stderr).lines();
        let mut out = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line);
            if out.chars().count() > STDERR_CAP_CHARS {
                out = tail_chars(&out, STDERR_CAP_CHARS);
            }
        }
        out
    })
}

/// Keep the last `max_chars` characters of `value` (char-boundary safe).
pub fn tail_chars(value: &str, max_chars: usize) -> String {
    let chars: Vec<char> = value.chars().collect();
    let start = chars.len().saturating_sub(max_chars);
    chars[start..].iter().collect()
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
    images: &[crate::external_agents::attachments::ImageBlock],
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
            let content = stream_json_user_content(prompt, images);
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

fn stream_json_user_content(
    prompt: &str,
    images: &[crate::external_agents::attachments::ImageBlock],
) -> serde_json::Value {
    if prompt.trim_start().starts_with('/') {
        serde_json::Value::String(prompt.to_string())
    } else {
        // Anthropic content array: text block first, then a base64 image block per attached image.
        let mut content = vec![serde_json::json!({ "type": "text", "text": prompt })];
        for img in images {
            content.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": img.mime,
                    "data": img.data_base64,
                },
            }));
        }
        serde_json::Value::Array(content)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_json_user_content_uses_string_for_slash_commands() {
        let slash = stream_json_user_content("/compact", &[]);
        assert_eq!(slash, serde_json::json!("/compact"));
        let text = stream_json_user_content("hello", &[]);
        assert_eq!(
            text,
            serde_json::json!([{ "type": "text", "text": "hello" }])
        );
    }

    #[test]
    fn stream_json_user_content_appends_image_blocks() {
        let img = crate::external_agents::attachments::ImageBlock {
            data_base64: "AAAA".to_string(),
            mime: "image/png".to_string(),
            path: std::path::PathBuf::from("/tmp/a.png"),
        };
        let content = stream_json_user_content("look", std::slice::from_ref(&img));
        let arr = content.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], serde_json::json!("text"));
        assert_eq!(arr[1]["type"], serde_json::json!("image"));
        assert_eq!(arr[1]["source"]["media_type"], serde_json::json!("image/png"));
        assert_eq!(arr[1]["source"]["data"], serde_json::json!("AAAA"));
    }

    #[test]
    fn stream_json_slash_ignores_images() {
        let img = crate::external_agents::attachments::ImageBlock {
            data_base64: "AAAA".to_string(),
            mime: "image/png".to_string(),
            path: std::path::PathBuf::from("/tmp/a.png"),
        };
        let content = stream_json_user_content("/compact", std::slice::from_ref(&img));
        assert_eq!(content, serde_json::json!("/compact"));
    }
}
