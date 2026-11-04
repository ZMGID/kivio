//! Root-scoped `agent://` output and `history://` transcript reads.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncReadExt;

use super::jobs::JobManager;
use super::registry::{artifacts_dir, now_ms, AgentRecord, AgentRegistry, AgentStatus};
use super::task::valid_agent_id as valid_id;
use crate::state::AppState;

/// Read a published result, agent progress, or an agent transcript for one root conversation.
pub async fn read(app: &AppHandle, root: &str, url: &str) -> Result<String, String> {
    let dir = artifacts_dir(app, root)?;
    let state = app.state::<AppState>();
    read_in(&dir, &state.sub_agents.registry, &state.sub_agents.jobs, root, url).await
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Agent,
    History,
}

fn parse_url(url: &str) -> Result<(Scheme, &str), String> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("agent://") {
        (Scheme::Agent, rest)
    } else if let Some(rest) = url.strip_prefix("history://") {
        (Scheme::History, rest)
    } else {
        return Err("Expected an agent:// or history:// URL".into());
    };
    let id = rest.strip_suffix('/').unwrap_or(rest);
    if id.is_empty() {
        return if scheme == Scheme::History {
            Ok((scheme, id))
        } else {
            Err("agent:// URL requires an output ID: agent://<id>".into())
        };
    }
    if !valid_id(id) {
        return Err(format!("Invalid agent ID: {id}"));
    }
    if scheme == Scheme::Agent && id == "all" {
        return Err("agent://all is write-only; use it to broadcast a message.".into());
    }
    Ok((scheme, id))
}

async fn artifact(dir: &Path, id: &str, extension: &str) -> Result<Option<(String, Option<SystemTime>)>, String> {
    if !valid_id(id) {
        return Err(format!("Invalid agent ID: {id}"));
    }
    let path = dir.join(format!("{id}.{extension}"));
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not open agent artifact {id}: {error}")),
    };
    let metadata = file.metadata().await
        .map_err(|error| format!("Could not inspect agent artifact {id}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("Agent artifact {id} is not a regular file"));
    }
    let mut text = String::new();
    file.read_to_string(&mut text).await
        .map_err(|error| format!("Could not read agent artifact {id}: {error}"))?;
    Ok(Some((text, metadata.modified().ok())))
}

async fn disk_ids(dir: &Path, extension: &str) -> Result<Vec<String>, String> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not list subagent artifacts: {error}")),
    };
    let mut ids = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("Could not list subagent artifacts: {error}"))?
    {
        let file_type = entry.file_type().await.map_err(|error| format!("Could not inspect agent artifact: {error}"))?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = name.strip_suffix(extension) else { continue };
        if valid_id(id) {
            ids.push(id.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

async fn find_artifact(
    dir: &Path,
    id: &str,
    extension: &str,
    registered: bool,
) -> Result<Option<(String, String, Option<SystemTime>)>, String> {
    // Registry resolution supplies the canonical spelling. A disk-only ID
    // needs the directory's spelling even on a case-insensitive filesystem.
    if !registered {
        let suffix = format!(".{extension}");
        let ids = disk_ids(dir, &suffix).await?;
        let actual = ids.iter().find(|candidate| candidate.as_str() == id)
            .or_else(|| ids.iter().find(|candidate| candidate.eq_ignore_ascii_case(id)));
        if let Some(actual) = actual {
            if let Some((text, modified)) = artifact(dir, actual, extension).await? {
                return Ok(Some((actual.clone(), text, modified)));
            }
        }
    }
    Ok(artifact(dir, id, extension).await?.map(|(text, modified)| (id.to_string(), text, modified)))
}

async fn not_found(dir: &Path, registry: &AgentRegistry, root: &str, id: &str, suffix: &str) -> Result<String, String> {
    let mut ids: Vec<String> = registry.list(root).into_iter().map(|agent| agent.id).collect();
    ids.extend(disk_ids(dir, suffix).await?);
    ids.sort_by_cached_key(|id| id.to_ascii_lowercase());
    ids.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    ids.retain(|candidate| !candidate.eq_ignore_ascii_case(id));
    let query = id.to_ascii_lowercase();
    ids.sort_by_key(|candidate| {
        let candidate = candidate.to_ascii_lowercase();
        let rank = if candidate.starts_with(&query) { 0 } else if candidate.contains(&query) { 1 } else { 2 };
        (rank, candidate.len().abs_diff(query.len()), candidate)
    });
    ids.truncate(5);
    let hint = if ids.is_empty() {
        "List agents with history://".to_string()
    } else {
        format!("Did you mean: {}", ids.join(", "))
    };
    Ok(format!("Not found: {id}\n{hint}"))
}

async fn read_in(
    dir: &Path,
    registry: &AgentRegistry,
    jobs: &JobManager,
    root: &str,
    url: &str,
) -> Result<String, String> {
    crate::chat::storage::validate_conversation_id(root)?;
    let (scheme, requested) = parse_url(url)?;
    if scheme == Scheme::History && requested.is_empty() {
        let agents = registry.list(root);
        let disk = disk_ids(dir, ".jsonl").await?;
        return Ok(format_index(&agents, &disk, now_ms()));
    }

    let agent = registry.resolve(root, requested);
    let id = agent.as_ref().map(|agent| agent.id.as_str()).unwrap_or(requested);
    let registered = agent.is_some();
    if scheme == Scheme::Agent {
        if let Some((actual, text, modified)) = find_artifact(dir, id, "md", registered).await? {
            let started_at = agent.as_ref()
                .filter(|agent| agent.status == AgentStatus::Running)
                .and_then(|agent| jobs.running(root, &agent.parent_id).into_iter()
                    .filter(|job| job.agent_id == agent.id)
                    .map(|job| job.started_at)
                    .max());
            return Ok(if let Some(modified) = modified.filter(|modified| published_before(*modified, started_at)) {
                format!(
                    "> `{actual}` is running a newer turn. The output below is from its PREVIOUS run (published {} ago), not the current one. Live progress: `history://{actual}`; its next result auto-delivers to its owner.\n\n{text}",
                    elapsed(modified.elapsed().unwrap_or_default().as_secs())
                )
            } else {
                text
            });
        }
        if let Some(agent) = agent {
            let messages = match artifact(dir, &agent.id, "jsonl").await? {
                Some((text, _)) => parse_jsonl(&text, agent.status == AgentStatus::Running)?,
                None => Vec::new(),
            };
            let live = if agent.status == AgentStatus::Running {
                jobs.running(root, &agent.parent_id)
                    .into_iter()
                    .filter(|job| job.agent_id == agent.id)
                    .max_by_key(|job| job.started_at)
                    .and_then(|job| job.progress)
                    .and_then(|progress| {
                        progress.get("recentOutput")?.as_array().map(|lines| {
                            lines.iter().rev().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")
                        })
                    })
                    .filter(|text| !text.trim().is_empty())
            } else {
                None
            };
            return Ok(format_progress(&agent, &messages, live.as_deref()));
        }
        return Err(not_found(dir, registry, root, requested, ".md").await?);
    }

    if let Some((actual, text, _)) = find_artifact(dir, id, "jsonl", registered).await? {
        let status = agent.as_ref().map(|agent| status_name(agent.status)).unwrap_or("on disk");
        let running = agent.as_ref().is_some_and(|agent| agent.status == AgentStatus::Running);
        return Ok(format_transcript(&actual, status, &parse_jsonl(&text, running)?));
    }
    if let Some(agent) = agent {
        if agent.status == AgentStatus::Running {
            return Ok(format!("# {} (running)\n\nNo transcript has been written yet.\n", agent.id));
        }
        return Err(format!("Agent {} has no transcript: session is gone and no session file was retained", agent.id));
    }
    Err(not_found(dir, registry, root, requested, ".jsonl").await?)
}

// shortcut: equal or coarse filesystem timestamps cannot prove turn provenance; persist publication generation if exact identification is needed.
fn published_before(modified: SystemTime, started_at: Option<i64>) -> bool {
    let Some(started_at) = started_at.filter(|start| *start >= 0) else { return false };
    modified.duration_since(UNIX_EPOCH)
        .is_ok_and(|time| time.as_millis() < started_at as u128)
}

fn status_name(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "running",
        AgentStatus::Idle => "idle",
        AgentStatus::Parked => "parked",
        AgentStatus::Aborted => "aborted",
    }
}

fn elapsed(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

fn format_index(agents: &[AgentRecord], disk: &[String], now: i64) -> String {
    let mut lines = vec!["# Agents".to_string(), String::new()];
    if agents.is_empty() && disk.is_empty() {
        lines.push("No agents registered.".into());
    } else {
        lines.push("| id | status | kind | parent | last activity |".into());
        lines.push("|---|---|---|---|---|".into());
        for agent in agents {
            let age = now.saturating_sub(agent.last_activity).max(0) as u64 / 1_000;
            lines.push(format!("| {} | {} | sub | {} | {} ago |", agent.id, status_name(agent.status), agent.parent_id, elapsed(age)));
        }
        for id in disk {
            if !agents.iter().any(|agent| agent.id.eq_ignore_ascii_case(id)) {
                lines.push(format!("| {id} | on disk | — | — | — |"));
            }
        }
        lines.push(String::new());
        lines.push("Read a transcript with `read history://<id>`.".into());
    }
    format!("{}\n", lines.join("\n"))
}

fn parse_jsonl(text: &str, running: bool) -> Result<Vec<Value>, String> {
    let mut messages = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    while let Some((index, line)) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(message) => messages.push(message),
            Err(error) if running && lines.peek().is_none() && !text.ends_with('\n') && error.is_eof() => break,
            Err(error) => return Err(format!("Invalid agent transcript on line {}: {error}", index + 1)),
        }
    }
    Ok(messages)
}

fn content_text(content: &Value, include_images: bool) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content.as_array().into_iter().flatten().filter_map(|block| {
        match block.get("type").and_then(Value::as_str) {
            Some("text" | "input_text" | "output_text") => block.get("text").and_then(Value::as_str),
            Some("image" | "image_url" | "input_image") if include_images => Some("[image]"),
            _ => None,
        }
    }).collect::<Vec<_>>().join("\n")
}

fn format_progress(agent: &AgentRecord, messages: &[Value], live: Option<&str>) -> String {
    let last = live.map(str::to_string).or_else(|| {
        messages.iter().rev()
            .filter(|message| message["role"] == "assistant")
            .map(|message| content_text(&message["content"], false))
            .find(|text| !text.trim().is_empty())
    });
    let body = match last {
        Some(text) if !text.trim().is_empty() => format!("## Latest assistant text\n\n{}", text.trim()),
        _ => "Nothing produced yet.".to_string(),
    };
    format!("# {} ({})\n\nNo final output yet: `agent://{}` shows progress until the run publishes its output. Full transcript: `history://{}`.\n\n{body}\n", agent.id, status_name(agent.status), agent.id, agent.id)
}

fn one_line(text: &str) -> String {
    let mut chars = text.split_whitespace().enumerate().flat_map(|(index, word)| {
        (index > 0).then_some(' ').into_iter().chain(word.chars())
    });
    let mut preview: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        preview.pop();
        preview.push('…');
    }
    preview
}

fn arg_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().filter(|text| !text.is_empty()) {
        return Some(text.to_string());
    }
    let array = value.as_array()?;
    if array.is_empty() || !array.iter().all(Value::is_string) {
        return None;
    }
    Some(array.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
}

fn primary_arg(name: &str, args: &Value) -> String {
    let parsed;
    let args = if let Some(text) = args.as_str() {
        match serde_json::from_str::<Value>(text) {
            Ok(value) => { parsed = value; &parsed }
            Err(_) => return one_line(text),
        }
    } else {
        args
    };
    let Some(object) = args.as_object() else { return String::new() };
    let get = |key: &str| object.get(key).and_then(arg_value);
    if name == "grep" {
        let pattern = get("pattern");
        let path = get("path").or_else(|| get("paths"));
        if let (Some(pattern), Some(path)) = (&pattern, &path) {
            return one_line(&format!("{pattern} @ {path}"));
        }
        if let Some(value) = pattern.or(path) { return one_line(&value); }
    }
    if name == "glob" {
        if let Some(value) = get("path").or_else(|| get("paths")) { return one_line(&value); }
    }
    if name == "ast_grep" {
        if let Some(value) = get("pat") { return one_line(&value); }
    }
    for key in ["path", "file_path", "filePath", "command", "cmd", "pattern", "url", "query", "prompt", "assignment", "note", "message", "op", "name", "id"] {
        if let Some(value) = get(key) { return one_line(&value); }
    }
    for (key, value) in object {
        if key != "i" {
            if let Some(text) = value.as_str().filter(|text| !text.is_empty()) { return one_line(text); }
        }
    }
    let rest: serde_json::Map<String, Value> = object.iter().filter(|(key, _)| key.as_str() != "i").map(|(key, value)| (key.clone(), value.clone())).collect();
    one_line(&Value::Object(rest).to_string())
}

fn tool_line(name: &str, args: &Value, result: Option<&Value>) -> String {
    let head = format!("→ {name}({})", primary_arg(name, args));
    let Some(result) = result else { return format!("{head} ⇒ pending") };
    let text = content_text(&result["content"], true);
    let count = if text.is_empty() { 0 } else { text.bytes().filter(|byte| *byte == b'\n').count() + 1 };
    let error = result.get("is_error").or_else(|| result.get("isError")).and_then(Value::as_bool);
    let status = match error { Some(true) => "error", Some(false) => "ok", None => "unknown" };
    let mut line = format!("{head} ⇒ {status} · {count} {}", if count == 1 { "line" } else { "lines" });
    if error == Some(true) {
        let preview = one_line(text.lines().next().unwrap_or_default());
        if !preview.is_empty() { line.push_str(&format!(" — {preview}")); }
    }
    line
}

fn format_transcript(id: &str, status: &str, messages: &[Value]) -> String {
    let mut pending: HashMap<&str, VecDeque<(usize, usize)>> = HashMap::new();
    let mut results: HashMap<(usize, usize), usize> = HashMap::new();
    let mut consumed = HashSet::new();
    for (message_index, message) in messages.iter().enumerate() {
        match message["role"].as_str() {
            Some("assistant") => {
                for (call_index, call) in message["tool_calls"].as_array().into_iter().flatten().enumerate() {
                    if let Some(id) = call["id"].as_str() {
                        pending.entry(id).or_default().push_back((message_index, call_index));
                    }
                }
            }
            Some("tool") => {
                if let Some(call) = message["tool_call_id"].as_str()
                    .and_then(|id| pending.get_mut(id))
                    .and_then(VecDeque::pop_front)
                {
                    results.insert(call, message_index);
                    consumed.insert(message_index);
                }
            }
            _ => {}
        }
    }
    let mut lines = vec![format!("# {id} ({status})"), String::new()];
    for (message_index, message) in messages.iter().enumerate() {
        match message["role"].as_str() {
            Some("user" | "developer") => {
                let text = content_text(&message["content"], true);
                if !text.trim().is_empty() {
                    lines.extend([format!("## {}", message["role"].as_str().unwrap_or_default()), String::new(), text, String::new()]);
                }
            }
            Some("assistant") => {
                let mut body = Vec::new();
                let text = content_text(&message["content"], false);
                if !text.trim().is_empty() { body.push(text); }
                for (call_index, call) in message["tool_calls"].as_array().into_iter().flatten().enumerate() {
                    let result = results.get(&(message_index, call_index)).map(|index| &messages[*index]);
                    let function = &call["function"];
                    let name = function["name"].as_str().unwrap_or("tool");
                    body.push(tool_line(name, &function["arguments"], result));
                }
                if !body.is_empty() {
                    lines.push("## assistant".into());
                    lines.push(String::new());
                    lines.extend(body);
                    lines.push(String::new());
                }
            }
            Some("tool") => {
                if !consumed.contains(&message_index) {
                    lines.push(tool_line(message["name"].as_str().unwrap_or("tool"), &Value::Null, Some(message)));
                    lines.push(String::new());
                }
            }
            _ => {}
        }
    }
    format!("{}\n", lines.join("\n").trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;

    fn record(root: &str, id: &str, status: AgentStatus) -> AgentRecord {
        AgentRecord {
            id: id.into(), root: root.into(), parent_id: "Main".into(), generation: 0,
            display_name: id.into(), agent_type: "scout".into(), status, depth: 1,
            child_conversation_id: format!("child-{id}"), run_id: None,
            session_file: PathBuf::new(), output_file: PathBuf::new(), last_activity: 0, activity: None,
        }
    }

    #[test]
    fn history_index_formats_registered_and_disk_only_agents() {
        let agents = [record("conv_root", "Scout", AgentStatus::Running), record("conv_root", "Scout.Child", AgentStatus::Parked)];
        let disk = vec!["Scout".into(), "Old".into()];
        let table = format_index(&agents, &disk, 125_000);
        assert!(table.contains("| id | status | kind | parent | last activity |"));
        assert!(table.contains("| Scout | running | sub | Main | 2m ago |"));
        assert!(table.contains("| Scout.Child | parked | sub | Main | 2m ago |"));
        assert!(table.contains("| Old | on disk | — | — | — |"));
        assert_eq!(table.matches("| Scout | running").count(), 1);
    }

    #[test]
    fn transcript_collapses_openai_tool_calls_and_retains_text() {
        let jsonl = concat!(
            "{\"role\":\"user\",\"content\":\"Inspect this\"}\n",
            "{\"role\":\"assistant\",\"content\":\"Checking\",\"tool_calls\":[{\"id\":\"one\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"src/main.rs\\\"}\"}},{\"id\":\"two\",\"function\":{\"name\":\"grep\",\"arguments\":\"{\\\"pattern\\\":\\\"needle\\\",\\\"path\\\":\\\"src\\\"}\"}},{\"id\":\"three\",\"function\":{\"name\":\"bash\",\"arguments\":\"{\\\"command\\\":\\\"pwd\\\"}\"}}]}\n",
            "{\"role\":\"tool\",\"tool_call_id\":\"one\",\"content\":\"line 1\\nline 2\",\"is_error\":false}\n",
            "{\"role\":\"tool\",\"tool_call_id\":\"two\",\"content\":\"permission denied\",\"is_error\":true}\n",
            "{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Done\"}]}\n"
        );
        let transcript = format_transcript("Scout", "running", &parse_jsonl(jsonl, true).unwrap());
        assert!(transcript.contains("## user\n\nInspect this"));
        assert!(transcript.contains("## assistant\n\nChecking"));
        assert!(transcript.contains("→ read(src/main.rs) ⇒ ok · 2 lines"));
        assert!(transcript.contains("→ grep(needle @ src) ⇒ error · 1 line — permission denied"));
        assert!(transcript.contains("→ bash(pwd) ⇒ pending"));
        assert!(transcript.contains("## assistant\n\nDone"));
        assert!(!transcript.contains("line 1"));
    }

    #[test]
    fn transcript_matches_reused_tool_ids_in_order() {
        let jsonl = concat!(
            "{\"role\":\"tool\",\"tool_call_id\":\"same\",\"content\":\"orphan\",\"is_error\":true}\n",
            "{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"same\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"first\\\"}\"}}]}\n",
            "{\"role\":\"tool\",\"tool_call_id\":\"same\",\"content\":\"ok\",\"is_error\":false}\n",
            "{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"same\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\":\\\"second\\\"}\"}}]}\n",
            "{\"role\":\"tool\",\"tool_call_id\":\"same\",\"content\":\"denied\",\"is_error\":true}\n"
        );
        let transcript = format_transcript("Scout", "idle", &parse_jsonl(jsonl, false).unwrap());
        assert!(transcript.contains("→ tool() ⇒ error · 1 line — orphan"));
        assert!(transcript.contains("→ read(first) ⇒ ok · 1 line"));
        assert!(transcript.contains("→ read(second) ⇒ error · 1 line — denied"));
        assert_eq!(transcript.matches("→ read(").count(), 2);
    }

    #[test]
    fn publication_must_precede_the_running_job_to_be_superseded() {
        let at_start = UNIX_EPOCH + Duration::from_millis(1_000);
        assert!(published_before(at_start - Duration::from_millis(1), Some(1_000)));
        assert!(!published_before(at_start, Some(1_000)));
        assert!(!published_before(at_start + Duration::from_nanos(500), Some(1_000)));
        assert!(!published_before(at_start, None));
    }

    #[tokio::test]
    async fn agent_progress_yields_to_published_output_and_is_root_scoped() {
        let temp = tempfile::tempdir().unwrap();
        let registry = AgentRegistry::default();
        let jobs = JobManager::default();
        registry.register(record("conv_root", "Scout", AgentStatus::Running));
        registry.register(record("conv_other", "Foreign", AgentStatus::Running));
        tokio::fs::write(temp.path().join("Scout.jsonl"), concat!(
            "{\"role\":\"assistant\",\"content\":\"first\"}\n",
            "{\"role\":\"assistant\",\"content\":\"latest\"}\n",
            "{\"role\":\"assistant\",\"content\":[{\"type\":\"image_url\"}]}\n"
        )).await.unwrap();
        let progress = read_in(temp.path(), &registry, &jobs, "conv_root", "agent://scout").await.unwrap();
        assert!(progress.contains("# Scout (running)"));
        assert!(progress.contains("## Latest assistant text\n\nlatest"));
        assert!(!progress.contains("\n\nfirst\n"));
        let output_path = temp.path().join("Scout.md");
        tokio::fs::write(&output_path, "published output\n").await.unwrap();
        assert_eq!(read_in(temp.path(), &registry, &jobs, "conv_root", "agent://Scout").await.unwrap(), "published output\n");
        let (job, _) = jobs.register("conv_root", "Main", "Scout", "work");
        let started_at = jobs.get(&job).unwrap().started_at as u64;
        let set_modified = |millis| {
            let file = std::fs::OpenOptions::new().write(true).open(&output_path).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_millis(millis))).unwrap();
        };
        set_modified(started_at + 5_000);
        assert_eq!(read_in(temp.path(), &registry, &jobs, "conv_root", "agent://Scout").await.unwrap(), "published output\n");
        set_modified(started_at - 5_000);
        let superseded = read_in(temp.path(), &registry, &jobs, "conv_root", "agent://Scout").await.unwrap();
        assert!(superseded.contains("running a newer turn"));
        assert!(superseded.ends_with("published output\n"));
        registry.set_status("conv_root", "Scout", AgentStatus::Idle);
        assert_eq!(read_in(temp.path(), &registry, &jobs, "conv_root", "agent://Scout").await.unwrap(), "published output\n");
        assert!(read_in(temp.path(), &registry, &jobs, "conv_root", "agent://Foreign").await.is_err());
        assert!(read_in(temp.path(), &registry, &jobs, "conv_root", "agent://../other/Foreign").await.is_err());
        assert!(read_in(temp.path(), &registry, &jobs, "..", "history://").await.is_err());
        let index = read_in(temp.path(), &registry, &jobs, "conv_root", "history://").await.unwrap();
        assert!(index.contains("| Scout | idle |"));
        assert!(!index.contains("Foreign"));
        let transcript = read_in(temp.path(), &registry, &jobs, "conv_root", "history://scout").await.unwrap();
        assert!(transcript.contains("# Scout (idle)\n\n## assistant\n\nfirst"));
        assert!(transcript.contains("## assistant\n\nlatest"));
    }

    #[tokio::test]
    async fn disk_only_history_and_live_progress_are_available() {
        let temp = tempfile::tempdir().unwrap();
        let registry = AgentRegistry::default();
        let jobs = JobManager::default();
        registry.register(record("conv_root", "Live", AgentStatus::Running));
        let empty = read_in(temp.path(), &registry, &jobs, "conv_root", "agent://Live").await.unwrap();
        assert!(empty.contains("# Live (running)") && empty.contains("Nothing produced yet."));
        let (job, _) = jobs.register("conv_root", "Main", "Live", "work");
        jobs.report_progress(&job, serde_json::json!({"recentOutput": ["second", "first"]}));
        let progress = read_in(temp.path(), &registry, &jobs, "conv_root", "agent://Live").await.unwrap();
        assert!(progress.contains("## Latest assistant text\n\nfirst\nsecond"));
        tokio::fs::write(temp.path().join("Old.jsonl"), "{\"role\":\"user\",\"content\":\"earlier\"}\n").await.unwrap();
        let index = read_in(temp.path(), &registry, &jobs, "conv_root", "history://").await.unwrap();
        assert!(index.contains("| Old | on disk |"));
        let transcript = read_in(temp.path(), &registry, &jobs, "conv_root", "history://old").await.unwrap();
        assert!(transcript.contains("# Old (on disk)\n\n## user\n\nearlier"));
        tokio::fs::write(temp.path().join("Old.md"), "saved output").await.unwrap();
        assert_eq!(read_in(temp.path(), &registry, &jobs, "conv_root", "agent://old").await.unwrap(), "saved output");
        registry.register(record("conv_root", "分析者.文件 核对", AgentStatus::Idle));
        tokio::fs::write(temp.path().join("分析者.文件 核对.md"), "unicode name").await.unwrap();
        assert_eq!(
            read_in(temp.path(), &registry, &jobs, "conv_root", "agent://分析者.文件 核对").await.unwrap(),
            "unicode name"
        );
    }

    #[tokio::test]
    async fn suggestions_and_results_are_scoped_to_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("conv_first");
        let second = temp.path().join("conv_second");
        tokio::fs::create_dir_all(&first).await.unwrap();
        tokio::fs::create_dir_all(&second).await.unwrap();
        let registry = AgentRegistry::default();
        let jobs = JobManager::default();
        for id in ["Alpha", "Alpine", "Albatross", "Alloy", "Almond", "Algebra"] {
            registry.register(record("conv_first", id, AgentStatus::Idle));
        }
        registry.register(record("conv_second", "Secret", AgentStatus::Idle));
        registry.register(record("conv_first", "Shared", AgentStatus::Idle));
        registry.register(record("conv_second", "Shared", AgentStatus::Idle));
        tokio::fs::write(first.join("Shared.md"), "first root").await.unwrap();
        tokio::fs::write(second.join("Shared.md"), "second root").await.unwrap();
        assert_eq!(read_in(&first, &registry, &jobs, "conv_first", "agent://Shared").await.unwrap(), "first root");
        assert_eq!(read_in(&second, &registry, &jobs, "conv_second", "agent://Shared").await.unwrap(), "second root");
        let error = read_in(&first, &registry, &jobs, "conv_first", "agent://Al").await.unwrap_err();
        let suggestions = error.split("Did you mean: ").nth(1).unwrap();
        assert_eq!(suggestions.split(", ").count(), 5);
        assert!(!error.contains("Secret"));
        assert!(read_in(&first, &registry, &jobs, "conv_first", "history://../conv_second/Secret").await.is_err());
    }


    #[test]
    fn tool_previews_collapse_whitespace_and_truncate_unicode() {
        assert_eq!(one_line("  a \n b  "), "a b");
        assert_eq!(one_line(&"测".repeat(120)), "测".repeat(120));
        assert_eq!(one_line(&"测".repeat(121)), format!("{}…", "测".repeat(119)));
    }

    #[test]
    fn incomplete_live_jsonl_is_tolerated_but_corruption_is_reported() {
        let text = "{\"role\":\"user\",\"content\":\"ok\"}\n{\"role\":\"assistant\"";
        assert_eq!(parse_jsonl(text, true).unwrap().len(), 1);
        assert!(parse_jsonl(text, false).is_err());
        assert!(parse_jsonl("{bad}\n", true).is_err());
    }
}
