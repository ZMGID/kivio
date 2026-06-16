//! `kivio-code` session storage (Phase 3).
//!
//! An append-only JSONL store for the terminal coding agent, modelled after PI's
//! `harness/session/*` (see `research/pi-runtime-session.md` §4): one `.jsonl`
//! file per session, grouped on disk by the session's working directory, with a
//! header line followed by one typed [`SessionRecord`] per line.
//!
//! ## On-disk layout
//!
//! ```text
//! <app_data_dir>/kivio-code/sessions/<cwd-slug>/<timestamp>_<uuid>.jsonl
//! ```
//!
//! - `<app_data_dir>` is the same Kivio per-app data dir the CLI settings loader
//!   resolves (`com.zmair.kivio` via the `directories` crate).
//! - `<cwd-slug>` encodes the session's working directory into a single folder
//!   name: the absolute path is lower-cased-free, leading separators stripped,
//!   and every `/`, `\`, `:` replaced with `-` (PI's `encodeCwd` scheme). The
//!   original `cwd` is also stored verbatim in the header, so the slug is purely
//!   a grouping/index key and never has to round-trip back to a path.
//! - The file name is `<timestamp>_<uuid>.jsonl`, where `<timestamp>` is the
//!   UTC creation time formatted `%Y%m%dT%H%M%S` (sortable, filename-safe) and
//!   `<uuid>` is the session id.
//!
//! ## File format
//!
//! - **Line 1** — a [`SessionRecord::Header`] (`type = "session"`): version,
//!   session id, cwd, `created_at`, and the model in use at creation.
//! - **Lines 2..** — one [`SessionRecord`] each (`message`, `tool_call`,
//!   `tool_result`, `compaction`, `model_change`).
//!
//! Each non-header record carries `id` and `parent_id` so a branching tree is
//! representable later; the MVP appends linearly, so the leaf is simply the last
//! appended record and each new record's `parent_id` is the previous leaf id.
//!
//! Loads tolerate a truncated/corrupt trailing line (interrupted write) by
//! skipping it instead of failing the whole session.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kivio_code::settings_loader::app_data_dir;

/// Current on-disk session format version. Bump when the record schema changes
/// in a backward-incompatible way.
pub const SESSION_VERSION: u32 = 1;

/// Sub-directory (under `<app_data_dir>`) that holds all kivio-code sessions.
const SESSIONS_SUBDIR: &str = "kivio-code/sessions";

/// One line of a session JSONL file. The first line is always
/// [`SessionRecord::Header`]; every subsequent line is one of the entry
/// variants. Tagged by a `type` field for forward-compatible parsing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionRecord {
    /// First line of every session file.
    #[serde(rename = "session")]
    Header {
        /// Format version (see [`SESSION_VERSION`]).
        version: u32,
        /// Session id (uuid v4), also embedded in the file name.
        id: String,
        /// The working directory this session operates in (verbatim).
        cwd: String,
        /// RFC3339 creation timestamp.
        created_at: String,
        /// Model in use at session creation (`providerId:model` or bare model).
        model: String,
    },
    /// A conversation message (system / user / assistant text).
    Message {
        /// Record id (uuid v4).
        id: String,
        /// Parent record id, or `None` for the first entry after the header.
        parent_id: Option<String>,
        /// RFC3339 timestamp.
        timestamp: String,
        /// `system` | `user` | `assistant`.
        role: String,
        /// The message text.
        content: String,
    },
    /// A tool invocation emitted by the assistant.
    ToolCall {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        /// Provider-assigned tool-call id (links to the matching [`ToolResult`]).
        ///
        /// [`ToolResult`]: SessionRecord::ToolResult
        call_id: String,
        /// Tool name (e.g. `read`, `bash`).
        name: String,
        /// JSON arguments object for the call.
        arguments: Value,
    },
    /// The result of a tool invocation.
    ToolResult {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        /// The `call_id` of the [`SessionRecord::ToolCall`] this answers.
        call_id: String,
        /// Tool name (mirrors the call for convenience).
        name: String,
        /// Result text shown to the model.
        content: String,
        /// Whether the tool reported an error.
        #[serde(default)]
        is_error: bool,
    },
    /// A compaction checkpoint: history before this point is replaced by
    /// `summary` when rebuilding the model context.
    Compaction {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        /// The summary text that stands in for the compacted history.
        summary: String,
    },
    /// Records that the active model changed mid-session.
    ModelChange {
        id: String,
        parent_id: Option<String>,
        timestamp: String,
        /// New model (`providerId:model` or bare model).
        model: String,
    },
}

impl SessionRecord {
    /// The record id, if any. The header has no `id` of its own (its identity is
    /// the session id), so it returns `None` here.
    pub fn id(&self) -> Option<&str> {
        match self {
            SessionRecord::Header { .. } => None,
            SessionRecord::Message { id, .. }
            | SessionRecord::ToolCall { id, .. }
            | SessionRecord::ToolResult { id, .. }
            | SessionRecord::Compaction { id, .. }
            | SessionRecord::ModelChange { id, .. } => Some(id),
        }
    }

    /// The parent record id, if any.
    pub fn parent_id(&self) -> Option<&str> {
        match self {
            SessionRecord::Header { .. } => None,
            SessionRecord::Message { parent_id, .. }
            | SessionRecord::ToolCall { parent_id, .. }
            | SessionRecord::ToolResult { parent_id, .. }
            | SessionRecord::Compaction { parent_id, .. }
            | SessionRecord::ModelChange { parent_id, .. } => parent_id.as_deref(),
        }
    }
}

/// A loaded / live session: the parsed header plus its append-only record log.
#[derive(Debug, Clone)]
pub struct Session {
    /// Session id (uuid v4).
    pub id: String,
    /// Working directory (verbatim, from the header).
    pub cwd: String,
    /// RFC3339 creation timestamp.
    pub created_at: String,
    /// Format version from the header.
    pub version: u32,
    /// Model recorded in the header.
    pub model: String,
    /// On-disk path of this session's `.jsonl` file.
    pub path: PathBuf,
    /// All non-header records, in append order.
    pub records: Vec<SessionRecord>,
}

/// Lightweight session description for listings, parsed from a session file's
/// header (+ a cheap scan for the first user message preview).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    /// Session id.
    pub id: String,
    /// On-disk path of the `.jsonl` file.
    pub path: PathBuf,
    /// RFC3339 creation timestamp (used for recent-first ordering).
    pub created_at: String,
    /// Working directory recorded in the header.
    pub cwd: String,
    /// Model recorded in the header.
    pub model: String,
    /// Preview of the first user message, if any (whitespace-collapsed,
    /// truncated to a short length for display).
    pub first_user_message: Option<String>,
}

/// Encode a working directory into a single filesystem-safe folder name,
/// mirroring PI's `encodeCwd`: strip a leading separator, then replace every
/// `/`, `\`, `:` with `-`. An empty/`.`-only result falls back to `root`.
pub fn encode_cwd(cwd: &Path) -> String {
    let raw = cwd.to_string_lossy();
    let trimmed = raw.trim_start_matches(['/', '\\']);
    let slug: String = trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '-',
            other => other,
        })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "root".to_string()
    } else {
        slug
    }
}

/// `<app_data_dir>/kivio-code/sessions` — the root of the session store. Falls
/// back to the system temp dir when no home/data dir can be resolved (so the CLI
/// still functions, just non-persistently across users).
pub fn sessions_root() -> PathBuf {
    match app_data_dir() {
        Some(dir) => dir.join(SESSIONS_SUBDIR),
        None => std::env::temp_dir().join("com.zmair.kivio").join(SESSIONS_SUBDIR),
    }
}

/// The per-cwd directory under the session root for a given working directory.
pub fn session_dir_for_cwd(cwd: &Path) -> PathBuf {
    sessions_root().join(encode_cwd(cwd))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn filename_timestamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Collapse whitespace and truncate to `max` chars (char-safe), appending `…`
/// when truncated. Used for the listing preview.
fn preview(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let head: String = collapsed.chars().take(max).collect();
        format!("{head}…")
    }
}

impl Session {
    /// Create a brand-new session: build the per-cwd directory, choose a path,
    /// and write the header line. The returned [`Session`] is ready to
    /// [`append`](Session::append) to.
    pub fn create(cwd: &Path, model: &str) -> Result<Session, String> {
        let id = new_id();
        let created_at = now_rfc3339();
        let dir = session_dir_for_cwd(cwd);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create session dir {}: {e}", dir.display()))?;
        let file_name = format!("{}_{}.jsonl", filename_timestamp(), id);
        let path = dir.join(file_name);

        let header = SessionRecord::Header {
            version: SESSION_VERSION,
            id: id.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            created_at: created_at.clone(),
            model: model.to_string(),
        };
        let line = serde_json::to_string(&header)
            .map_err(|e| format!("failed to serialize session header: {e}"))?;
        let mut file = File::create(&path)
            .map_err(|e| format!("failed to create session file {}: {e}", path.display()))?;
        writeln!(file, "{line}")
            .map_err(|e| format!("failed to write session header: {e}"))?;
        file.flush()
            .map_err(|e| format!("failed to flush session header: {e}"))?;

        Ok(Session {
            id,
            cwd: cwd.to_string_lossy().into_owned(),
            created_at,
            version: SESSION_VERSION,
            model: model.to_string(),
            path,
            records: Vec::new(),
        })
    }

    /// The id of the current leaf record (last appended), or `None` when the
    /// session has only its header. New appends parent themselves to this.
    pub fn leaf_id(&self) -> Option<String> {
        self.records
            .iter()
            .rev()
            .find_map(|r| r.id().map(str::to_string))
    }

    /// Append a record line and flush. Records carrying `id`/`parent_id` fields
    /// have them auto-populated when blank: `id` gets a fresh uuid, `parent_id`
    /// is set to the current leaf when `None`. Already-populated fields are
    /// respected so callers can build explicit trees later.
    pub fn append(&mut self, mut record: SessionRecord) -> Result<String, String> {
        let leaf = self.leaf_id();
        let assigned_id = Self::ensure_ids(&mut record, leaf);

        let line = serde_json::to_string(&record)
            .map_err(|e| format!("failed to serialize session record: {e}"))?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("failed to open session file {}: {e}", self.path.display()))?;
        writeln!(file, "{line}")
            .map_err(|e| format!("failed to append session record: {e}"))?;
        file.flush()
            .map_err(|e| format!("failed to flush session record: {e}"))?;

        self.records.push(record);
        Ok(assigned_id)
    }

    /// Fill in `id` (when empty) and `parent_id` (when `None`) on a record,
    /// returning the effective id. The header is left untouched. Returns an
    /// empty string for the header variant.
    fn ensure_ids(record: &mut SessionRecord, leaf: Option<String>) -> String {
        macro_rules! fill {
            ($id:expr, $parent:expr) => {{
                if $id.is_empty() {
                    *$id = new_id();
                }
                if $parent.is_none() {
                    *$parent = leaf.clone();
                }
                $id.clone()
            }};
        }
        match record {
            SessionRecord::Header { .. } => String::new(),
            SessionRecord::Message { id, parent_id, .. }
            | SessionRecord::ToolCall { id, parent_id, .. }
            | SessionRecord::ToolResult { id, parent_id, .. }
            | SessionRecord::Compaction { id, parent_id, .. }
            | SessionRecord::ModelChange { id, parent_id, .. } => fill!(id, parent_id),
        }
    }

    /// Load a session from a `.jsonl` file: parse the header and every
    /// subsequent record. A corrupt/partial trailing line is skipped (treated as
    /// an interrupted write) rather than failing the load; a missing/garbled
    /// header is a hard error.
    pub fn load(path: &Path) -> Result<Session, String> {
        let file = File::open(path)
            .map_err(|e| format!("failed to open session file {}: {e}", path.display()))?;
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("failed to read session file {}: {e}", path.display()))?;

        let mut iter = lines.iter().enumerate();
        let header = loop {
            match iter.next() {
                Some((_, line)) if line.trim().is_empty() => continue,
                Some((_, line)) => {
                    break serde_json::from_str::<SessionRecord>(line)
                        .map_err(|e| format!("invalid session header in {}: {e}", path.display()))?;
                }
                None => return Err(format!("empty session file {}", path.display())),
            }
        };

        let (version, id, cwd, created_at, model) = match header {
            SessionRecord::Header {
                version,
                id,
                cwd,
                created_at,
                model,
            } => (version, id, cwd, created_at, model),
            _ => {
                return Err(format!(
                    "first line of {} is not a session header",
                    path.display()
                ))
            }
        };

        let mut records = Vec::new();
        let remaining: Vec<(usize, &String)> = iter.collect();
        let last_idx = remaining.len().saturating_sub(1);
        for (pos, (_, line)) in remaining.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SessionRecord>(line) {
                Ok(record) => records.push(record),
                Err(e) => {
                    // Tolerate only a corrupt *trailing* line (interrupted write).
                    // A corrupt line in the middle indicates real corruption.
                    if pos == last_idx {
                        break;
                    }
                    return Err(format!(
                        "corrupt record in {} (line {}): {e}",
                        path.display(),
                        pos + 2
                    ));
                }
            }
        }

        Ok(Session {
            id,
            cwd,
            created_at,
            version,
            model,
            path: path.to_path_buf(),
            records,
        })
    }

    /// Walk the leaf→root chain (following `parent_id`) and reverse it to root→
    /// leaf order. The MVP appends linearly so this is the full record list, but
    /// the walk is parent-id-driven so a future tree only emits the active
    /// branch. The header is not part of the chain.
    pub fn branch_records(&self) -> Vec<&SessionRecord> {
        use std::collections::HashMap;
        let by_id: HashMap<&str, &SessionRecord> = self
            .records
            .iter()
            .filter_map(|r| r.id().map(|id| (id, r)))
            .collect();

        let mut chain: Vec<&SessionRecord> = Vec::new();
        let mut cursor = self.leaf_id();
        while let Some(id) = cursor {
            match by_id.get(id.as_str()) {
                Some(record) => {
                    chain.push(*record);
                    cursor = record.parent_id().map(str::to_string);
                }
                None => break,
            }
        }
        chain.reverse();
        chain
    }

    /// Reconstruct the leaf→root record chain into the `runtime_messages` shape
    /// the agent loop consumes: a `Vec` of OpenAI-style message objects
    /// (`{role, content}`, assistant `tool_calls`, and `{role:"tool", ...}`
    /// results). A [`SessionRecord::Compaction`] on the path replaces the
    /// history before it with a single summary user message.
    pub fn to_runtime_messages(&self) -> Vec<Value> {
        let chain = self.branch_records();

        // Honor the latest compaction on the path: drop everything before it and
        // lead with the summary as a user message (matches PI's buildContext).
        let start = chain
            .iter()
            .rposition(|r| matches!(r, SessionRecord::Compaction { .. }))
            .unwrap_or(0);

        let mut messages: Vec<Value> = Vec::new();
        for record in &chain[start..] {
            match record {
                SessionRecord::Message { role, content, .. } => {
                    messages.push(serde_json::json!({ "role": role, "content": content }));
                }
                SessionRecord::ToolCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => {
                    // Each tool call is an assistant message carrying one
                    // tool_call (OpenAI tool-calling shape).
                    let arguments_str = if arguments.is_string() {
                        arguments.as_str().unwrap_or("").to_string()
                    } else {
                        arguments.to_string()
                    };
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": Value::Null,
                        "tool_calls": [{
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": arguments_str },
                        }],
                    }));
                }
                SessionRecord::ToolResult {
                    call_id, content, ..
                } => {
                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": content,
                    }));
                }
                SessionRecord::Compaction { summary, .. } => {
                    messages.push(serde_json::json!({
                        "role": "user",
                        "content": format!("<summary>\n{summary}\n</summary>"),
                    }));
                }
                SessionRecord::ModelChange { .. } | SessionRecord::Header { .. } => {}
            }
        }
        messages
    }
}

/// List the sessions stored for a given working directory, most-recent first
/// (by header `created_at`, falling back to file name). Missing directory → an
/// empty list. Files that fail to parse a header are skipped.
pub fn list_sessions(cwd: &Path) -> Vec<SessionSummary> {
    let dir = session_dir_for_cwd(cwd);
    let mut summaries = read_summaries_in_dir(&dir);
    summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    summaries
}

/// List sessions across *all* cwd directories under the session root, most
/// recent first.
pub fn list_all_sessions() -> Vec<SessionSummary> {
    let root = sessions_root();
    let mut summaries = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                summaries.extend(read_summaries_in_dir(&entry.path()));
            }
        }
    }
    summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    summaries
}

fn read_summaries_in_dir(dir: &Path) -> Vec<SessionSummary> {
    let mut summaries = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return summaries,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(summary) = read_summary(&path) {
            summaries.push(summary);
        }
    }
    summaries
}

/// Parse a session file's header (+ scan for the first user message) into a
/// [`SessionSummary`] without loading the whole record vector eagerly. Returns
/// `None` when the header can't be parsed.
fn read_summary(path: &Path) -> Option<SessionSummary> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // First non-empty line must be the header.
    let mut header: Option<SessionRecord> = None;
    for line in lines.by_ref() {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        header = serde_json::from_str::<SessionRecord>(&line).ok();
        break;
    }
    let (id, created_at, cwd, model) = match header? {
        SessionRecord::Header {
            id,
            created_at,
            cwd,
            model,
            ..
        } => (id, created_at, cwd, model),
        _ => return None,
    };

    // Cheap scan for the first user message preview.
    let mut first_user_message = None;
    for line in lines {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(SessionRecord::Message { role, content, .. }) =
            serde_json::from_str::<SessionRecord>(&line)
        {
            if role == "user" {
                first_user_message = Some(preview(&content, 80));
                break;
            }
        }
    }

    Some(SessionSummary {
        id,
        path: path.to_path_buf(),
        created_at,
        cwd,
        model,
        first_user_message,
    })
}

/// Open the most-recent session for `cwd`, if any (the `--continue` flow). The
/// returned session is fully loaded and ready to append to. `None` when there
/// are no sessions for that directory.
pub fn resume_recent(cwd: &Path) -> Option<Session> {
    let summary = list_sessions(cwd).into_iter().next()?;
    Session::load(&summary.path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point the session root at an isolated temp dir for the duration of a
    /// test by overriding the app-data resolution via `HOME`/`XDG_DATA_HOME`.
    /// Instead of fighting env vars, every test creates sessions through the
    /// real path scheme but inside a unique temp cwd, so directories never
    /// collide between tests. We then assert on the returned `Session.path`.
    fn unique_cwd(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kivio-code-sess-{tag}-{}", new_id()));
        std::fs::create_dir_all(&dir).expect("create temp cwd");
        dir
    }

    #[test]
    fn encode_cwd_slugifies_path() {
        assert_eq!(encode_cwd(Path::new("/Users/me/proj")), "Users-me-proj");
        assert_eq!(encode_cwd(Path::new("/")), "root");
        // Adjacent separators (`:` then `\`) each map to `-`, so a Windows-style
        // path yields a doubled dash there — fine for a grouping slug.
        assert_eq!(
            encode_cwd(Path::new("C:\\Users\\me\\proj")),
            "C--Users-me-proj"
        );
    }

    #[test]
    fn create_append_reload_roundtrip() {
        let cwd = unique_cwd("roundtrip");
        let mut session = Session::create(&cwd, "prov:model-x").expect("create");
        assert!(session.path.exists(), "header file written");
        assert_eq!(session.records.len(), 0);

        let m1 = session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "user".to_string(),
                content: "read main.rs".to_string(),
            })
            .expect("append user");
        let call = session
            .append(SessionRecord::ToolCall {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                call_id: "call_1".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({ "path": "main.rs" }),
            })
            .expect("append tool call");
        session
            .append(SessionRecord::ToolResult {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                call_id: "call_1".to_string(),
                name: "read".to_string(),
                content: "fn main() {}".to_string(),
                is_error: false,
            })
            .expect("append tool result");
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "assistant".to_string(),
                content: "It is an empty main.".to_string(),
            })
            .expect("append assistant");

        // Linear append → each record parents to the prior leaf.
        assert!(!m1.is_empty());
        assert_eq!(session.records.len(), 4);
        if let SessionRecord::ToolCall { parent_id, .. } = &session.records[1] {
            assert_eq!(parent_id.as_deref(), Some(m1.as_str()));
        } else {
            panic!("record 1 should be a tool call");
        }
        assert_eq!(session.leaf_id().is_some(), true);
        let _ = call;

        let reloaded = Session::load(&session.path).expect("reload");
        assert_eq!(reloaded.id, session.id);
        assert_eq!(reloaded.cwd, cwd.to_string_lossy());
        assert_eq!(reloaded.model, "prov:model-x");
        assert_eq!(reloaded.version, SESSION_VERSION);
        assert_eq!(reloaded.records, session.records);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }

    #[test]
    fn list_orders_recent_first() {
        let cwd = unique_cwd("listorder");
        // Create three sessions; force distinct created_at so ordering is
        // deterministic regardless of filename-timestamp granularity.
        let mut paths = Vec::new();
        for (i, ts) in ["2026-01-01T00:00:00Z", "2026-06-01T00:00:00Z", "2026-03-01T00:00:00Z"]
            .iter()
            .enumerate()
        {
            let mut s = Session::create(&cwd, &format!("m{i}")).expect("create");
            // Rewrite header with a controlled created_at by appending a marker
            // user message and patching the header in place is overkill; instead
            // we craft the file directly to control created_at.
            let header = SessionRecord::Header {
                version: SESSION_VERSION,
                id: s.id.clone(),
                cwd: cwd.to_string_lossy().into_owned(),
                created_at: ts.to_string(),
                model: format!("m{i}"),
            };
            let line = serde_json::to_string(&header).unwrap();
            std::fs::write(&s.path, format!("{line}\n")).unwrap();
            s.append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "user".to_string(),
                content: format!("hello {i}"),
            })
            .unwrap();
            paths.push((ts.to_string(), s.path.clone()));
        }

        let listed = list_sessions(&cwd);
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].created_at, "2026-06-01T00:00:00Z");
        assert_eq!(listed[1].created_at, "2026-03-01T00:00:00Z");
        assert_eq!(listed[2].created_at, "2026-01-01T00:00:00Z");
        // Preview captured the first user message.
        assert!(listed[0].first_user_message.as_deref().unwrap().starts_with("hello"));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }

    #[test]
    fn resume_recent_returns_latest() {
        let cwd = unique_cwd("resume");
        assert!(resume_recent(&cwd).is_none(), "no sessions yet");

        for (i, ts) in ["2026-01-01T00:00:00Z", "2026-09-01T00:00:00Z"].iter().enumerate() {
            let s = Session::create(&cwd, &format!("m{i}")).expect("create");
            let header = SessionRecord::Header {
                version: SESSION_VERSION,
                id: s.id.clone(),
                cwd: cwd.to_string_lossy().into_owned(),
                created_at: ts.to_string(),
                model: format!("m{i}"),
            };
            let line = serde_json::to_string(&header).unwrap();
            std::fs::write(&s.path, format!("{line}\n")).unwrap();
        }

        let resumed = resume_recent(&cwd).expect("resume");
        assert_eq!(resumed.created_at, "2026-09-01T00:00:00Z");
        assert_eq!(resumed.model, "m1");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }

    #[test]
    fn load_tolerates_corrupt_trailing_line() {
        let cwd = unique_cwd("corrupt");
        let mut session = Session::create(&cwd, "m").expect("create");
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "user".to_string(),
                content: "good line".to_string(),
            })
            .expect("append");

        // Simulate an interrupted write: append a truncated/garbage line.
        let mut file = OpenOptions::new().append(true).open(&session.path).unwrap();
        write!(file, "{{\"type\":\"message\",\"role\":\"assi").unwrap();
        file.flush().unwrap();

        let reloaded = Session::load(&session.path).expect("load tolerates trailing junk");
        // Only the one good message survives; the trailing partial is dropped.
        assert_eq!(reloaded.records.len(), 1);
        assert!(matches!(
            &reloaded.records[0],
            SessionRecord::Message { content, .. } if content == "good line"
        ));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }

    #[test]
    fn corrupt_middle_line_is_an_error() {
        let cwd = unique_cwd("midcorrupt");
        let session = Session::create(&cwd, "m").expect("create");
        let header_line = {
            let header = SessionRecord::Header {
                version: SESSION_VERSION,
                id: session.id.clone(),
                cwd: cwd.to_string_lossy().into_owned(),
                created_at: now_rfc3339(),
                model: "m".to_string(),
            };
            serde_json::to_string(&header).unwrap()
        };
        // header, garbage (middle), then a valid trailing record.
        let good = SessionRecord::Message {
            id: new_id(),
            parent_id: None,
            timestamp: now_rfc3339(),
            role: "user".to_string(),
            content: "tail".to_string(),
        };
        let good_line = serde_json::to_string(&good).unwrap();
        std::fs::write(
            &session.path,
            format!("{header_line}\n{{not json}}\n{good_line}\n"),
        )
        .unwrap();

        assert!(Session::load(&session.path).is_err());

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }

    #[test]
    fn to_runtime_messages_shape() {
        let cwd = unique_cwd("runtime");
        let mut session = Session::create(&cwd, "m").expect("create");
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "system".to_string(),
                content: "be helpful".to_string(),
            })
            .unwrap();
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "user".to_string(),
                content: "read main.rs".to_string(),
            })
            .unwrap();
        session
            .append(SessionRecord::ToolCall {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                call_id: "call_1".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({ "path": "main.rs" }),
            })
            .unwrap();
        session
            .append(SessionRecord::ToolResult {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                call_id: "call_1".to_string(),
                name: "read".to_string(),
                content: "fn main() {}".to_string(),
                is_error: false,
            })
            .unwrap();
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "assistant".to_string(),
                content: "done".to_string(),
            })
            .unwrap();

        let msgs = session.to_runtime_messages();
        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "be helpful");
        assert_eq!(msgs[1]["role"], "user");

        // Tool call → assistant message with tool_calls array.
        assert_eq!(msgs[2]["role"], "assistant");
        let call = &msgs[2]["tool_calls"][0];
        assert_eq!(call["id"], "call_1");
        assert_eq!(call["type"], "function");
        assert_eq!(call["function"]["name"], "read");
        // Arguments serialized to a JSON string per OpenAI tool-calling shape.
        let args: Value = serde_json::from_str(call["function"]["arguments"].as_str().unwrap())
            .expect("arguments is a JSON string");
        assert_eq!(args["path"], "main.rs");

        // Tool result → role:tool message keyed by tool_call_id.
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "call_1");
        assert_eq!(msgs[3]["content"], "fn main() {}");

        assert_eq!(msgs[4]["role"], "assistant");
        assert_eq!(msgs[4]["content"], "done");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }

    #[test]
    fn to_runtime_messages_folds_compaction() {
        let cwd = unique_cwd("compact");
        let mut session = Session::create(&cwd, "m").expect("create");
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "user".to_string(),
                content: "old turn one".to_string(),
            })
            .unwrap();
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "assistant".to_string(),
                content: "old answer one".to_string(),
            })
            .unwrap();
        session
            .append(SessionRecord::Compaction {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                summary: "user asked about X; resolved.".to_string(),
            })
            .unwrap();
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "user".to_string(),
                content: "new question".to_string(),
            })
            .unwrap();

        let msgs = session.to_runtime_messages();
        // History before the compaction is dropped; summary leads as a user msg.
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert!(msgs[0]["content"].as_str().unwrap().contains("<summary>"));
        assert!(msgs[0]["content"]
            .as_str()
            .unwrap()
            .contains("resolved"));
        assert_eq!(msgs[1]["content"], "new question");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }

    #[test]
    fn model_change_record_roundtrips_and_is_ignored_in_runtime() {
        let cwd = unique_cwd("modelchange");
        let mut session = Session::create(&cwd, "m0").expect("create");
        session
            .append(SessionRecord::ModelChange {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                model: "prov:m1".to_string(),
            })
            .unwrap();
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: now_rfc3339(),
                role: "user".to_string(),
                content: "hi".to_string(),
            })
            .unwrap();

        let reloaded = Session::load(&session.path).expect("reload");
        assert!(matches!(
            &reloaded.records[0],
            SessionRecord::ModelChange { model, .. } if model == "prov:m1"
        ));
        // ModelChange is settings-only: not emitted into runtime messages.
        let msgs = reloaded.to_runtime_messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(session_dir_for_cwd(&cwd));
    }
}
