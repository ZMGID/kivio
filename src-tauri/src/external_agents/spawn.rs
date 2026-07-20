use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
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
    spawn_stderr_tail(child.stderr.take())
}

/// Ring-buffer stderr drain for persistent sessions (N1): the CLI process is long-lived and its
/// stderr is `take()`n separately from stdout, so we can't use `drain_stderr(&mut Child)`. Spawns a
/// task that accumulates the tail (last `STDERR_CAP_CHARS`) until stderr hits EOF (i.e. the child
/// dies / is killed), then returns it. Join the handle on close / error to fold into diagnostics.
pub fn spawn_stderr_tail(stderr: Option<ChildStderr>) -> tokio::task::JoinHandle<String> {
    tokio::spawn(async move {
        match stderr {
            Some(stderr) => accumulate_tail(stderr, STDERR_CAP_CHARS).await,
            None => String::new(),
        }
    })
}

const STDERR_CAP_CHARS: usize = 8192;

/// Kill the child (so its stderr hits EOF) and join the drain task to recover the stderr tail.
/// Used by persistent sessions on a handshake/connect error path (N1 + R2).
pub async fn join_stderr_tail(
    child: &mut Child,
    stderr_tail: tokio::task::JoinHandle<String>,
) -> String {
    let _ = child.start_kill();
    stderr_tail.await.unwrap_or_default()
}

/// Append a drained stderr tail to an error message when non-empty (for R2 diagnostics).
pub fn fold_stderr(msg: String, stderr_tail: &str) -> String {
    if stderr_tail.trim().is_empty() {
        msg
    } else {
        format!("{msg}\nstderr: {}", stderr_tail.trim())
    }
}

/// Read lines from `reader` until EOF, keeping only the last `cap` characters (char-boundary safe).
/// Blank lines are dropped. Extracted from the drain tasks so the ring-buffer bound is unit-testable
/// without spawning a real process.
async fn accumulate_tail<R: AsyncRead + Unpin>(reader: R, cap: usize) -> String {
    let mut lines = BufReader::new(reader).lines();
    let mut out = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
        if out.chars().count() > cap {
            out = tail_chars(&out, cap);
        }
    }
    out
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
        assert_eq!(
            arr[1]["source"]["media_type"],
            serde_json::json!("image/png")
        );
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

    #[tokio::test]
    async fn accumulate_tail_keeps_only_the_tail_and_finishes_on_eof() {
        // 200 numbered lines far exceeding an 8-char cap → only the last chars survive, and the
        // task completes once the in-memory reader hits EOF.
        let mut input = String::new();
        for i in 0..200 {
            input.push_str(&format!("line{i}\n"));
        }
        let tail = accumulate_tail(input.as_bytes(), 8).await;
        assert!(
            tail.chars().count() <= 8,
            "tail should be capped, got {tail:?}"
        );
        assert!(
            tail.ends_with("line199"),
            "tail should keep the end, got {tail:?}"
        );
    }

    #[tokio::test]
    async fn accumulate_tail_drops_blank_lines() {
        let tail = accumulate_tail("\n\nhello\n\nworld\n".as_bytes(), 8192).await;
        assert_eq!(tail, "hello\nworld");
    }
}
