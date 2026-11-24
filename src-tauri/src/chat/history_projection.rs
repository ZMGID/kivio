//! Rebuildable SQLite display projection of one atomically replaced conversation.
//! JSON remains the replay and edit authority. A reader verifies the source file
//! identity in the same transaction that supplies its metadata, rows and pages.
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{value::RawValue, Value};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use super::commands::{
    catalog,
    history_tool_results::{extract_results, DeferredToolResults},
};
use super::model::ModelArtifact;
use super::protocol::{
    ChatConversationEvent, ChatHistoryCursor, ChatHistoryReplay, ChatRevisionPatch,
    MAX_REPLAY_BYTES, MAX_REPLAY_EVENTS,
};
use super::{ChatMessage, Conversation};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const MAX_TURN_ITEMS: usize = 500;
const MAX_ITEMS: usize = 75;
const MAX_BYTES: usize = 1_048_576;
const SIBLINGS: usize = 4;
const INITIAL_TURNS: usize = 10;
const PAGE_TURNS: usize = 20;

/// Optional resume evidence. The projection already hashed the precise JSON message array
/// while committing this revision. Never rebuild a missing display cache just for a checkpoint.
pub(crate) fn settled_message_digest(app: &AppHandle, id: &str, revision: u64) -> Option<String> {
    let source = super::storage::conversation_file_path(app, id).ok()?;
    let source_stamp = stamp(&File::open(&source).ok()?).ok()?;
    let db = Connection::open_with_flags(
        database_path(&source),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    db.query_row("SELECT lower(hex(messages)) FROM projection_digests JOIN snapshot USING(singleton) WHERE source=?1 AND revision=?2",
        params![source_stamp, revision as i64], |r| r.get(0)).ok()
}

pub(crate) fn database_path(source: &Path) -> PathBuf {
    source.with_extension("history-v1.sqlite3")
}

const SIDECAR_SUFFIXES: [&str; 4] = [
    "history-v1.sqlite3",
    "history-v1.sqlite3-journal",
    "history-v1.sqlite3-wal",
    "history-v1.sqlite3-shm",
];

pub(crate) fn is_archive_sidecar(relative: &Path) -> bool {
    relative.parent() == Some(Path::new("conversations"))
        && relative
            .file_name()
            .and_then(|x| x.to_str())
            .is_some_and(|name| {
                SIDECAR_SUFFIXES.iter().any(|suffix| {
                    name.strip_suffix(suffix)
                        .is_some_and(|stem| stem.ends_with('.'))
                })
            })
}

pub(crate) fn remove_files(source: &Path) -> Vec<String> {
    SIDECAR_SUFFIXES
        .iter()
        .filter_map(|suffix| {
            let path = source.with_extension(suffix);
            match std::fs::remove_file(&path) {
                Ok(()) => None,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => Some(format!("无法清理会话历史索引 {}：{e}", path.display())),
            }
        })
        .collect()
}

pub(crate) fn stamp(file: &File) -> Result<String> {
    let metadata = file.metadata()?;
    let modified = metadata
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_nanos() as i128)
        .unwrap_or_else(|before| -(before.duration().as_nanos() as i128));
    #[cfg(unix)]
    let identity = std::os::unix::fs::MetadataExt::ino(&metadata);
    #[cfg(windows)]
    let identity = {
        let info = winapi_util::file::information(file)?;
        format!("{}-{}", info.volume_serial_number(), info.file_index())
    };
    Ok(format!("{identity}-{}-{modified}", metadata.len()))
}

fn open_database(path: &Path) -> Result<Connection> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // SQLite otherwise creates new files with 0644. Its rollback journal
        // inherits the database mode, so secure even an old file before opening it.
        let file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => File::open(path)?,
            Err(e) => return Err(e.into()),
        };
        if file.metadata()?.permissions().mode() & 0o777 != 0o600 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let db = Connection::open(path)?;
    schema(&db)?;
    Ok(db)
}

fn schema(db: &Connection) -> Result<()> {
    if db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))? == 4 {
        return Ok(());
    }
    db.execute_batch("BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS snapshot (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1), source TEXT NOT NULL,
            revision INTEGER NOT NULL, total INTEGER NOT NULL, turns INTEGER NOT NULL, metadata TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS items (
            ordinal INTEGER PRIMARY KEY, id TEXT NOT NULL, role TEXT NOT NULL, group_id TEXT,
            title TEXT, bytes INTEGER NOT NULL, inline_bytes INTEGER NOT NULL, digest BLOB NOT NULL
        );
        CREATE INDEX IF NOT EXISTS by_message ON items(id);
        CREATE INDEX IF NOT EXISTS turns ON items(ordinal) WHERE role='user';
        CREATE TABLE IF NOT EXISTS messages (ordinal INTEGER PRIMARY KEY, payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS results (
            ordinal INTEGER NOT NULL, tool TEXT NOT NULL, digest TEXT NOT NULL, preview TEXT NOT NULL,
            PRIMARY KEY(ordinal, tool)
        );
        CREATE TABLE IF NOT EXISTS artifacts (
            ordinal INTEGER NOT NULL, id TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(ordinal,id)
        );
        CREATE INDEX IF NOT EXISTS by_artifact ON artifacts(id, ordinal);
        CREATE TABLE IF NOT EXISTS boundaries (ordinal INTEGER NOT NULL, payload TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS by_boundary ON boundaries(ordinal);
        CREATE TABLE IF NOT EXISTS projection_digests (
            singleton INTEGER PRIMARY KEY CHECK(singleton=1), messages BLOB NOT NULL, boundaries BLOB NOT NULL
        );
        CREATE TABLE IF NOT EXISTS metadata_replay (
            revision INTEGER PRIMARY KEY, previous_source TEXT NOT NULL, source TEXT NOT NULL, patch TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS message_usage (
            ordinal INTEGER PRIMARY KEY, provider_id TEXT, input REAL, cache_read REAL,
            cache_write REAL, output REAL
        );
        DELETE FROM message_usage;
        INSERT INTO message_usage
            SELECT ordinal, json_extract(payload,'$.provider_id'), json_extract(payload,'$.usage.input_tokens'),
                json_extract(payload,'$.usage.cached_input_tokens'), json_extract(payload,'$.usage.cache_creation_input_tokens'),
                json_extract(payload,'$.usage.output_tokens')
            FROM messages WHERE json_type(payload,'$.usage')='object';
        PRAGMA user_version=4;
        COMMIT;")?;
    Ok(())
}

pub(crate) struct View<'a> {
    db: &'a Connection,
    pub total: usize,
    pub turn_count: usize,
    pub revision: u64,
    pub source: String,
}

pub(crate) fn read<T>(
    app: &AppHandle,
    id: &str,
    operation: impl Fn(&View<'_>) -> Result<T>,
) -> std::result::Result<T, String> {
    let source = super::storage::conversation_file_path(app, id)?;
    File::open(&source).map_err(|e| format!("读取对话失败（{id}）：{e}"))?;
    let result = open_database(&database_path(&source))
        .and_then(|mut db| with_snapshot(&mut db, &source, &operation));
    match result {
        Ok(value) => Ok(value),
        Err(error)
            if error.is::<rusqlite::Error>()
                || error.is::<serde_json::Error>()
                || error.is::<std::io::Error>() =>
        {
            // A read-only volume or damaged cache must not hide the source. Never
            // remove an open database under another reader or writer.
            eprintln!("history projection unavailable for {id}: {error}");
            let fallback = || -> Result<T> {
                let mut db = Connection::open_in_memory()?;
                schema(&db)?;
                with_snapshot(&mut db, &source, operation)
            };
            fallback().map_err(|e| e.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn with_snapshot<T>(
    db: &mut Connection,
    source: &Path,
    operation: impl FnOnce(&View<'_>) -> Result<T>,
) -> Result<T> {
    let file = File::open(source)?;
    let source_stamp = stamp(&file)?;
    let tx = db.transaction()?;
    if let Some(view) = snapshot(&tx, &source_stamp)? {
        return operation(&view);
    }
    drop(tx);
    // Re-open after obtaining the write lock: another writer or a simultaneous
    // first click may already have migrated/replaced the atomic source.
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut file = File::open(source)?;
    let source_stamp = stamp(&file)?;
    if snapshot(&tx, &source_stamp)?.is_none() {
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        project(&tx, &source_stamp, &content)?;
    }
    let view = snapshot(&tx, &source_stamp)?.ok_or("missing conversation projection")?;
    let result = operation(&view);
    tx.commit()?;
    result
}

fn snapshot<'a>(db: &'a Connection, stamp: &str) -> Result<Option<View<'a>>> {
    Ok(db
        .query_row(
            "SELECT revision,total,turns FROM snapshot WHERE singleton=1 AND source=?1",
            [stamp],
            |row| {
                Ok(View {
                    db,
                    source: stamp.to_owned(),
                    revision: row.get::<_, i64>(0)? as u64,
                    total: row.get::<_, i64>(1)? as usize,
                    turn_count: row.get::<_, i64>(2)? as usize,
                })
            },
        )
        .optional()?)
}

pub(crate) fn record_write(path: &Path, content: &str) -> Result<()> {
    // The conversation writer owns the repository lock through this operation.
    let source_stamp = stamp(&File::open(path)?)?;
    let mut db = open_database(&database_path(path))?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if snapshot(&tx, &source_stamp)?.is_none() {
        project(&tx, &source_stamp, content)?;
    }
    tx.commit()?;
    Ok(())
}

fn project(tx: &Transaction<'_>, source_stamp: &str, source: &str) -> Result<()> {
    // Borrow the original bytes. Only changed message bodies are deserialized,
    // normalized and written. Hidden replay never gets its own SQLite copy.
    let mut header: BTreeMap<String, &RawValue> = serde_json::from_str(source)?;
    let empty: &RawValue = serde_json::from_str("[]")?;
    let raw_messages = header
        .insert("messages".to_string(), empty)
        .ok_or("conversation has no messages")?;
    header.remove("builtin_history");
    let mut metadata: Conversation = serde_json::from_slice(&serde_json::to_vec(&header)?)?;
    if let Some(summary) = &mut metadata.context_state.summary {
        summary.replay = None;
    }
    let message_digest = Sha256::digest(raw_messages.get().as_bytes());
    let context = &metadata.context_state;
    let boundary_digest = Sha256::digest(serde_json::to_vec(&(
        &context.compaction_boundaries,
        &context.clear_boundaries,
        context
            .summary
            .as_ref()
            .map(|s| (&s.id, &s.source_until_message_id, &s.content, s.stale)),
    ))?);
    let previous = tx
        .query_row(
            "SELECT messages,boundaries FROM projection_digests JOIN snapshot USING(singleton) WHERE singleton=1",
            [],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    let rows_changed = previous
        .as_ref()
        .is_none_or(|(old, _)| old != &message_digest[..]);
    if rows_changed {
        update_rows(tx, raw_messages)?;
    }
    let structural = rows_changed
        || previous
            .as_ref()
            .is_none_or(|(_, old)| old != &boundary_digest[..]);
    if structural {
        update_boundaries(tx, &metadata)?;
        tx.execute(
            "INSERT OR REPLACE INTO projection_digests VALUES (1,?1,?2)",
            params![&message_digest[..], &boundary_digest[..]],
        )?;
    }
    let metadata_json = serde_json::to_string(&metadata)?;
    record_metadata_replay(tx, source_stamp, &metadata, &metadata_json, structural)?;
    if rows_changed {
        tx.execute("INSERT OR REPLACE INTO snapshot VALUES (1,?1,?2,(SELECT COUNT(*) FROM items),(SELECT COUNT(*) FROM items WHERE role='user'),?3)",
            params![source_stamp,metadata.revision as i64,metadata_json])?;
    } else {
        tx.execute(
            "UPDATE snapshot SET source=?1,revision=?2,metadata=?3 WHERE singleton=1",
            params![source_stamp, metadata.revision as i64, metadata_json],
        )?;
    }
    Ok(())
}

// A live metadata notice alone cannot certify a revision: the same write can also
// edit messages or navigation boundaries. Journal only complete, proven deltas.
fn record_metadata_replay(
    tx: &Transaction<'_>,
    source: &str,
    next: &Conversation,
    json: &str,
    structural: bool,
) -> Result<()> {
    if !structural {
        let (revision, after, old): (i64, String, String) = tx.query_row(
            "SELECT revision,source,metadata FROM snapshot WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if (revision as u64).checked_add(1) == Some(next.revision) {
            let before: BTreeMap<String, Value> = serde_json::from_str(&old)?;
            let next_fields: BTreeMap<String, Value> = serde_json::from_str(json)?;
            let mut events = Vec::new();
            let mut complete = true;
            for key in before
                .keys()
                .chain(next_fields.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                if before.get(key) == next_fields.get(key) {
                    continue;
                }
                let event = match key.as_str() {
                    "revision" | "updated_at" => continue,
                    "title" => ChatConversationEvent::TitleUpdated {
                        title: next.title.clone(),
                    },
                    "context_state" => ChatConversationEvent::ContextUpdated {
                        context_state: (&next.context_state).into(),
                    },
                    "agent_todo_state" => ChatConversationEvent::TodoUpdated {
                        todo_state: (&next.agent_todo_state).into(),
                    },
                    "agent_plan_state" => ChatConversationEvent::PlanUpdated {
                        plan_state: (&next.agent_plan_state).into(),
                    },
                    "goal_state" => ChatConversationEvent::GoalUpdated {
                        goal_state: next.goal_state.as_ref().map(Into::into),
                    },
                    _ => {
                        complete = false;
                        break;
                    }
                };
                events.push(event);
            }
            if complete {
                let patch = serde_json::to_string(&ChatRevisionPatch {
                    revision: next.revision,
                    updated_at: next.updated_at,
                    events,
                })?;
                if patch.len() <= MAX_REPLAY_BYTES {
                    tx.execute(
                        "INSERT OR REPLACE INTO metadata_replay VALUES (?1,?2,?3,?4)",
                        params![next.revision as i64, after, source, patch],
                    )?;
                    tx.execute("DELETE FROM metadata_replay WHERE revision IN (
                        SELECT revision FROM (SELECT revision, ROW_NUMBER() OVER (ORDER BY revision DESC) AS n,
                            SUM(LENGTH(CAST(patch AS BLOB))) OVER (ORDER BY revision DESC) AS bytes FROM metadata_replay)
                        WHERE n>?1 OR bytes>?2)", params![MAX_REPLAY_EVENTS as i64, MAX_REPLAY_BYTES as i64])?;
                    return Ok(());
                }
            }
        }
    }
    tx.execute("DELETE FROM metadata_replay", [])?;
    Ok(())
}

fn update_rows(tx: &Transaction<'_>, raw_messages: &RawValue) -> Result<()> {
    let messages: Vec<&RawValue> = serde_json::from_str(raw_messages.get())?;
    let old = tx
        .prepare("SELECT digest FROM items ORDER BY ordinal")?
        .query_map([], |row| row.get::<_, Vec<u8>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (ordinal, raw) in messages.iter().enumerate() {
        let digest = Sha256::digest(raw.get().as_bytes());
        if old
            .get(ordinal)
            .is_some_and(|old| old.as_slice() == &digest[..])
        {
            continue;
        }
        let mut message: ChatMessage = serde_json::from_str(raw.get())?;
        catalog::normalize_history_message(&mut message);
        write_message(tx, ordinal, &mut message, &digest)?;
    }
    if old.len() > messages.len() {
        for table in ["items", "messages", "results", "artifacts", "message_usage"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE ordinal>=?1"),
                [messages.len() as i64],
            )?;
        }
    }
    Ok(())
}

fn update_boundaries(tx: &Transaction<'_>, metadata: &Conversation) -> Result<()> {
    let context = &metadata.context_state;
    tx.execute("DELETE FROM boundaries", [])?;
    if !context.compaction_boundaries.is_empty()
        || !context.clear_boundaries.is_empty()
        || context.summary.is_some()
    {
        let indices = tx
            .prepare("SELECT id,ordinal FROM items ORDER BY ordinal")?
            .query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as usize)))?
            .collect::<rusqlite::Result<HashMap<String, usize>>>()?;
        for entry in catalog::history_boundary_directory(metadata, &indices) {
            tx.execute(
                "INSERT INTO boundaries VALUES (?1,?2)",
                params![
                    entry["message_index"].as_i64(),
                    serde_json::to_string(&entry)?
                ],
            )?;
        }
    }
    Ok(())
}

fn write_message(
    tx: &Transaction<'_>,
    ordinal: usize,
    message: &mut ChatMessage,
    digest: &[u8],
) -> Result<()> {
    let ordinal = ordinal as i64;
    tx.prepare_cached("DELETE FROM results WHERE ordinal=?1")?
        .execute([ordinal])?;
    tx.prepare_cached("DELETE FROM artifacts WHERE ordinal=?1")?
        .execute([ordinal])?;
    for artifact in message
        .artifacts
        .iter()
        .chain(message.tool_calls.iter().flat_map(|t| &t.artifacts))
    {
        if let Some(id) = &artifact.id {
            tx.prepare_cached("INSERT OR REPLACE INTO artifacts VALUES (?1,?2,?3)")?
                .execute(params![ordinal, id, serde_json::to_string(artifact)?])?;
        }
    }
    if let Some(usage) = &message.usage {
        tx.prepare_cached("INSERT OR REPLACE INTO message_usage VALUES (?1,?2,?3,?4,?5,?6)")?
            .execute(params![
                ordinal,
                message.provider_id,
                usage.input_tokens.map(|n| n as f64),
                usage.cached_input_tokens.map(|n| n as f64),
                usage.cache_creation_input_tokens.map(|n| n as f64),
                usage.output_tokens.map(|n| n as f64)
            ])?;
    } else {
        tx.prepare_cached("DELETE FROM message_usage WHERE ordinal=?1")?
            .execute([ordinal])?;
    }
    let extracted = extract_results(message);
    let payload = serde_json::to_string(message)?;
    let mut inline_bytes = payload.len();
    for (tool, hash, preview) in extracted {
        // result_preview is an explicit nullable field on ToolCallRecord.
        inline_bytes += serde_json::to_string(&preview)?.len().saturating_sub(4);
        tx.prepare_cached("INSERT INTO results VALUES (?1,?2,?3,?4)")?
            .execute(params![ordinal, tool, hash, preview])?;
    }
    let title =
        (message.role == "user").then(|| message.content.chars().take(120).collect::<String>());
    tx.prepare_cached("INSERT OR REPLACE INTO items VALUES (?1,?2,?3,?4,?5,?6,?7,?8)")?
        .execute(params![
            ordinal,
            message.id,
            message.role,
            message.group_id,
            title,
            payload.len() as i64,
            inline_bytes as i64,
            digest
        ])?;
    tx.prepare_cached("INSERT OR REPLACE INTO messages VALUES (?1,?2)")?
        .execute(params![ordinal, payload])?;
    Ok(())
}

struct Candidate {
    user: bool,
    assistant: bool,
    group: Option<String>,
    bytes: usize,
}

#[derive(serde::Serialize)]
pub(crate) struct ProviderTokenUsage {
    provider_id: Option<String>,
    input_less_cached: Option<f64>,
    input_plus_cache_write: Option<f64>,
    cached: Option<f64>,
    output: Option<f64>,
}

impl View<'_> {
    pub(crate) fn usage(&self) -> Result<Vec<ProviderTokenUsage>> {
        let mut statement = self.db.prepare(
            "SELECT provider_id, SUM(MAX(0,input-COALESCE(cache_read,0))),
                SUM(input+COALESCE(cache_write,0)), SUM(cache_read), SUM(output)
             FROM message_usage GROUP BY provider_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(ProviderTokenUsage {
                provider_id: row.get(0)?,
                input_less_cached: row.get(1)?,
                input_plus_cache_write: row.get(2)?,
                cached: row.get(3)?,
                output: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub(crate) fn replay(&self, after: &ChatHistoryCursor) -> Result<Option<ChatHistoryReplay>> {
        if after.revision > self.revision {
            return Ok(None);
        }
        let mut patches = Vec::new();
        let mut last = after.clone();
        if last.revision < self.revision {
            let mut statement = self.db.prepare_cached(
                "SELECT revision,previous_source,source,patch FROM metadata_replay WHERE revision>?1 ORDER BY revision")?;
            let mut rows = statement.query([after.revision as i64])?;
            while let Some(row) = rows.next()? {
                let revision = row.get::<_, i64>(0)? as u64;
                if last.revision.checked_add(1) != Some(revision)
                    || row.get::<_, String>(1)? != last.source
                {
                    return Ok(None);
                }
                patches.push(serde_json::from_str(&row.get::<_, String>(3)?)?);
                last = ChatHistoryCursor {
                    revision,
                    source: row.get(2)?,
                };
            }
        }
        Ok(
            (last.revision == self.revision && last.source == self.source).then(|| {
                ChatHistoryReplay {
                    after: after.clone(),
                    through: last,
                    patches,
                }
            }),
        )
    }

    fn header(&self) -> Result<Conversation> {
        let json: String =
            self.db
                .query_row("SELECT metadata FROM snapshot WHERE singleton=1", [], |r| {
                    r.get(0)
                })?;
        Ok(serde_json::from_str(&json)?)
    }

    pub(crate) fn metadata(&self, app: &AppHandle) -> Result<Conversation> {
        let mut conversation = self.header()?;
        conversation.runtime_locked =
            crate::external_agents::session::runtime_locked(app, &conversation);
        Ok(conversation)
    }

    pub(crate) fn window_ranges(
        &self,
        anchor: Option<&str>,
        defer: bool,
    ) -> Result<Vec<Range<usize>>> {
        let tail = self.select(0..self.total, false, INITIAL_TURNS, defer)?;
        if let Some(id) = anchor {
            let index: Option<usize> = self
                .db
                .query_row(
                    "SELECT ordinal FROM items WHERE id=?1 AND ordinal<?2 ORDER BY ordinal LIMIT 1",
                    params![id, tail.start as i64],
                    |r| Ok(r.get::<_, i64>(0)? as usize),
                )
                .optional()?;
            if let Some(index) = index {
                let left = self.select(0..index + 1, false, INITIAL_TURNS, defer)?;
                let right = self.select(index + 1..tail.start, true, INITIAL_TURNS, defer)?;
                if right.end == tail.start {
                    return Ok(vec![left.start..tail.end]);
                }
                return Ok(vec![left.start..right.end, tail]);
            }
        }
        Ok(vec![tail])
    }

    pub(crate) fn page_range(
        &self,
        before: usize,
        after: Option<usize>,
        defer: bool,
    ) -> Result<Range<usize>> {
        let end = before.min(self.total);
        self.select(
            after.unwrap_or(0).min(end)..end,
            after.is_some(),
            PAGE_TURNS,
            defer,
        )
    }

    fn select(
        &self,
        bounds: Range<usize>,
        forward: bool,
        turns: usize,
        defer: bool,
    ) -> Result<Range<usize>> {
        let order = if forward { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT role,group_id,CASE WHEN ?3 AND bytes!=inline_bytes AND NOT EXISTS
            (SELECT 1 FROM items AS other WHERE other.id=i.id AND other.ordinal!=i.ordinal LIMIT 1)
            THEN bytes ELSE inline_bytes END
            FROM items AS i WHERE ordinal>=?1 AND ordinal<?2 ORDER BY ordinal {order} LIMIT ?4"
        );
        let mut stmt = self.db.prepare_cached(&sql)?;
        let limit = if self.turn_count > 0 {
            MAX_TURN_ITEMS
        } else {
            MAX_ITEMS
        };
        let candidates = stmt
            .query_map(
                params![
                    bounds.start as i64,
                    bounds.end as i64,
                    defer,
                    (limit + SIBLINGS + 1) as i64
                ],
                |r| {
                    let role: String = r.get(0)?;
                    Ok(Candidate {
                        user: role == "user",
                        assistant: role == "assistant",
                        group: r.get(1)?,
                        bytes: r.get::<_, i64>(2)? as usize,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut count = 0;
        let mut bytes = 0usize;
        let mut users = 0;
        for row in candidates.iter().take(limit) {
            if count > 0
                && (bytes.saturating_add(row.bytes) > MAX_BYTES
                    || users == turns && (!forward || row.user))
            {
                break;
            }
            count += 1;
            bytes = bytes.saturating_add(row.bytes);
            users += usize::from(row.user);
        }
        if users == 0 {
            count = count.min(MAX_ITEMS);
        }
        // Keep the sibling arms at the cut together and their adjacent question/answer.
        if count > 0 {
            if forward
                && candidates[count - 1].user
                && candidates.get(count).is_some_and(|r| r.assistant)
            {
                count += 1;
            }
            if let Some(group) = &candidates[count - 1].group {
                let end = count + SIBLINGS;
                while count < end
                    && candidates
                        .get(count)
                        .is_some_and(|r| r.group.as_ref() == Some(group))
                {
                    count += 1;
                }
            }
            if !forward
                && candidates[count - 1].assistant
                && candidates.get(count).is_some_and(|r| r.user)
            {
                count += 1;
            }
        }
        Ok(if forward {
            bounds.start..bounds.start + count
        } else {
            bounds.end - count..bounds.end
        })
    }

    pub(crate) fn directory(&self, ranges: &[Range<usize>]) -> Result<Vec<Value>> {
        let mut turns = self.db.prepare_cached("SELECT ordinal,id,title FROM items WHERE role='user' AND ordinal>=?1 AND ordinal<?2 ORDER BY ordinal")?;
        let mut markers = self.db.prepare_cached(
            "SELECT payload FROM boundaries WHERE ordinal>=?1 AND ordinal<?2 ORDER BY ordinal",
        )?;
        let mut entries = Vec::new();
        for range in ranges {
            for entry in turns.query_map(params![range.start as i64,range.end as i64], |r| {
                let ordinal:i64 = r.get(0)?;
                let id:String = r.get(1)?;
                let title:String = r.get(2)?;
                Ok(serde_json::json!({"kind":"turn","id":format!("turn-{id}"),"message_id":id,"message_index":ordinal,"title":title}))
            })? { entries.push(entry?); }
            for payload in markers
                .query_map(params![range.start as i64, range.end as i64], |r| {
                    r.get::<_, String>(0)
                })?
            {
                entries.push(serde_json::from_str(&payload?)?);
            }
        }
        entries.sort_by_key(|e| e["message_index"].as_u64());
        Ok(entries)
    }

    pub(crate) fn messages(
        &self,
        app: &AppHandle,
        id: &str,
        ranges: &[Range<usize>],
        defer: bool,
    ) -> Result<Page> {
        let pending = super::sub_agent::progress::pending(app, id)?;
        let mut page = self.load_rows(ranges, defer)?;
        if !pending.is_empty() {
            let mut conversation = self.header()?;
            conversation.messages = page.messages;
            super::sub_agent::progress::apply(&mut conversation, &pending);
            page.messages = conversation.messages;
        }
        Ok(page)
    }

    fn load_rows(&self, ranges: &[Range<usize>], defer: bool) -> Result<Page> {
        let mut messages = Vec::new();
        let mut deferred = DeferredToolResults::new();
        let mut rows = self.db.prepare_cached("SELECT m.ordinal,m.payload,i.bytes!=i.inline_bytes FROM messages AS m
            JOIN items AS i ON i.ordinal=m.ordinal WHERE m.ordinal>=?1 AND m.ordinal<?2 ORDER BY m.ordinal")?;
        let mut details = self.db.prepare_cached("SELECT tool,digest,CASE WHEN ?2 THEN NULL ELSE preview END FROM results WHERE ordinal=?1")?;
        let mut ambiguous = self
            .db
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM items WHERE id=?1 LIMIT 1 OFFSET 1)")?;
        for range in ranges {
            for row in rows.query_map(params![range.start as i64, range.end as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })? {
                let (ordinal, payload, has_results) = row?;
                let mut message: ChatMessage = serde_json::from_str(&payload)?;
                if !has_results {
                    messages.push(message);
                    continue;
                }
                let lazy = defer && !ambiguous.query_row([&message.id], |r| r.get::<_, bool>(0))?;
                let mut tools = details.query(params![ordinal, lazy])?;
                while let Some(tool) = tools.next()? {
                    let id: String = tool.get(0)?;
                    if lazy {
                        deferred
                            .entry(message.id.clone())
                            .or_default()
                            .insert(id, tool.get(1)?);
                    } else if let Some(call) = message.tool_calls.iter_mut().find(|t| t.id == id) {
                        call.result_preview = tool.get(2)?;
                    }
                }
                messages.push(message);
            }
        }
        if messages.len() != ranges.iter().map(|r| r.len()).sum::<usize>() {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        let mut needed: Vec<_> = super::artifacts::missing_history_artifact_ids(&messages)
            .into_iter()
            .collect();
        needed.sort();
        let mut artifacts = Vec::new();
        let mut find = self.db.prepare_cached(
            "SELECT payload FROM artifacts WHERE id=?1 ORDER BY ordinal DESC LIMIT 1",
        )?;
        for id in needed {
            if let Some(json) = find.query_row([id], |r| r.get::<_, String>(0)).optional()? {
                artifacts.push(serde_json::from_str(&json)?);
            }
        }
        Ok(Page {
            messages,
            deferred,
            artifacts,
        })
    }

    pub(crate) fn tool_result(&self, message_id: &str, tool: &str, digest: &str) -> Result<String> {
        let result = self
            .db
            .query_row(
                "SELECT r.preview FROM items i JOIN results r ON r.ordinal=i.ordinal
            WHERE i.id=?1 AND r.tool=?2 AND r.digest=?3 AND NOT EXISTS
            (SELECT 1 FROM items WHERE id=?1 LIMIT 1 OFFSET 1)",
                params![message_id, tool, digest],
                |r| r.get(0),
            )
            .optional()?;
        result.ok_or_else(|| "工具结果已变更或不存在，请重新打开对话".into())
    }
}

pub(crate) struct Page {
    pub messages: Vec<ChatMessage>,
    pub artifacts: Vec<ModelArtifact>,
    pub deferred: DeferredToolResults,
}

pub(crate) fn get_metadata(app: &AppHandle, id: &str) -> std::result::Result<Conversation, String> {
    read(app, id, |view| view.metadata(app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(index: usize, role: &str) -> Value {
        json!({"id":format!("m{index}"),"role":role,"content":format!("text{index}"),"timestamp":1})
    }

    fn source(messages: Vec<Value>) -> Value {
        json!({"id":"conv_fixture","title":"conversation","provider_id":"p","model":"model","messages":messages,"created_at":1,"updated_at":1})
    }

    fn reopen<T>(path: &Path, query: impl FnOnce(&View<'_>) -> Result<T>) -> T {
        let mut db = open_database(&database_path(path)).unwrap();
        with_snapshot(&mut db, path, query).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn history_and_rollback_journal_keep_conversation_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = database_path(&dir.path().join("conv_fixture.json"));
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let db = open_database(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
        db.execute_batch("BEGIN IMMEDIATE; INSERT INTO messages VALUES (0,'test')")
            .unwrap();
        assert_eq!(mode(&path.with_extension("sqlite3-journal")), 0o600);
        db.execute_batch("ROLLBACK").unwrap();
        drop(db);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        open_database(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn metadata_replay_requires_an_unbroken_window_lineage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conv_fixture.json");
        let mut doc = source(vec![message(0, "user")]);
        let write = |doc: &Value| {
            let conversation: Conversation = serde_json::from_value(doc.clone()).unwrap();
            super::super::storage::write_conversation_file_at_path(&path, conversation).unwrap();
        };
        write(&doc);
        let start = reopen(&path, |view| {
            Ok(ChatHistoryCursor {
                revision: view.revision,
                source: view.source.clone(),
            })
        });
        doc["revision"] = json!(1);
        doc["title"] = json!("renamed");
        write(&doc);
        doc["revision"] = json!(2);
        doc["agent_plan_state"] = json!({"mode":"plan","status":"empty","updated_at":1});
        write(&doc);
        let through = reopen(&path, |view| {
            let replay = view.replay(&start)?.unwrap();
            assert_eq!(replay.patches.len(), 2);
            assert!(
                matches!(&replay.patches[0].events[..], [ChatConversationEvent::TitleUpdated {title}] if title == "renamed")
            );
            assert!(matches!(
                &replay.patches[1].events[..],
                [ChatConversationEvent::PlanUpdated { .. }]
            ));
            Ok(replay.through)
        });
        // A restored file can carry the same revision but different persisted rows.
        doc["messages"][0]["content"] = json!("rewritten");
        write(&doc);
        reopen(&path, |view| {
            assert!(view.replay(&start)?.is_none());
            assert!(view.replay(&through)?.is_none());
            Ok(())
        });
    }

    #[test]
    fn cold_window_pages_and_anchor_read_only_their_rows_after_legacy_backfill() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conv_fixture.json");
        let mut messages: Vec<_> = (0..220)
            .map(|i| message(i, if i % 2 == 0 { "user" } else { "assistant" }))
            .collect();
        messages[1]["artifacts"] = json!([{"id":"art_early","name":"early.png","mime_type":"image/png","data_url":"data:image/png;base64,QUJD","path":null}]);
        messages[219]["content"] = json!("![image](artifact:art_early) `artifact:art_unused`");
        messages[219]["api_messages"] = json!([{"role":"assistant","content":"hidden replay"}]);
        let original = source(messages).to_string();
        std::fs::write(&path, &original).unwrap();
        reopen(&path, |view| {
            assert_eq!(view.total, 220);
            assert_eq!(view.turn_count, 110);
            assert_eq!(view.directory(&[0..220])?.len(), 110);
            Ok(())
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        // An off-page payload is deliberately unreadable. Indexed window, anchor,
        // older/forward pages and artifact dependency lookup still work after reopening.
        let db = open_database(&database_path(&path)).unwrap();
        db.execute("UPDATE messages SET payload='invalid' WHERE ordinal=1", [])
            .unwrap();
        drop(db);
        reopen(&path, |view| {
            let ranges = view.window_ranges(None, true)?;
            assert_eq!(ranges, vec![200..220]);
            assert_eq!(view.directory(&ranges)?.len(), 10);
            let page = view.load_rows(&ranges, true)?;
            assert_eq!(page.messages.len(), 20);
            assert_eq!(page.messages[0].id, "m200");
            assert!(page.messages[19].api_messages.is_empty());
            assert_eq!(page.artifacts[0].id.as_deref(), Some("art_early"));
            // Legacy inline media belongs to the JSON; a transient read must
            // neither invent an unowned attachment path nor drop the image.
            assert_eq!(page.artifacts[0].data_url, "data:image/png;base64,QUJD");
            assert_eq!(view.page_range(200, None, true)?, 160..200);
            assert_eq!(view.page_range(200, Some(40), true)?, 40..80);
            assert_eq!(
                view.window_ranges(Some("m80"), true)?,
                vec![62..102, 200..220]
            );
            Ok(())
        });
    }

    #[test]
    fn usage_survives_v3_upgrade_and_follows_replaced_and_truncated_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conv_fixture.json");
        let mut messages = vec![
            message(0, "assistant"),
            message(1, "assistant"),
            message(2, "assistant"),
        ];
        messages[0]["usage"] =
            json!({"input_tokens":1,"cached_input_tokens":10,"cache_creation_input_tokens":4});
        messages[1]["usage"] = json!({"input_tokens":20,"output_tokens":0});
        messages[2]["usage"] = json!({"output_tokens":50});
        messages[2]["provider_id"] = json!("other");
        let mut doc = source(messages);
        std::fs::write(&path, doc.to_string()).unwrap();
        reopen(&path, |_| Ok(()));
        // Existing v3 sidecars already have normalized message rows and an up-to-date source stamp.
        open_database(&database_path(&path))
            .unwrap()
            .execute_batch("DROP TABLE message_usage; PRAGMA user_version=3;")
            .unwrap();
        reopen(&path, |view| {
            let rows = view.usage()?;
            assert_eq!(rows.len(), 2);
            assert_eq!(rows[0].input_less_cached, Some(20.0));
            assert_eq!(rows[0].input_plus_cache_write, Some(25.0));
            assert_eq!(rows[0].cached, Some(10.0));
            assert_eq!(rows[0].output, Some(0.0));
            assert_eq!(rows[1].provider_id.as_deref(), Some("other"));
            assert_eq!(rows[1].input_less_cached, None);
            assert_eq!(rows[1].output, Some(50.0));
            Ok(())
        });
        doc["messages"][0]["usage"] = Value::Null;
        doc["messages"].as_array_mut().unwrap().pop();
        std::fs::write(&path, doc.to_string()).unwrap();
        reopen(&path, |view| {
            let rows = view.usage()?;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].input_plus_cache_write, Some(20.0));
            assert_eq!(rows[0].cached, None);
            Ok(())
        });
    }

    #[test]
    fn byte_item_and_turn_limits_keep_oversized_rows_reachable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conv_fixture.json");
        let mut rows: Vec<_> = (0..640).map(|i| message(i, "assistant")).collect();
        rows[110] = message(110, "user");
        rows[635]["content"] = json!("x".repeat(MAX_BYTES + 200));
        rows[636]["group_id"] = json!("answers");
        rows[637]["group_id"] = json!("answers");
        std::fs::write(&path, source(rows).to_string()).unwrap();
        reopen(&path, |view| {
            assert_eq!(view.window_ranges(None, true)?, vec![636..640]);
            assert_eq!(view.page_range(636, None, true)?, 635..636);
            assert_eq!(view.page_range(635, None, true)?, 560..635);
            assert_eq!(view.page_range(580, None, true)?, 80..580);
            assert_eq!(view.page_range(580, Some(0), true)?, 0..500);
            Ok(())
        });
    }

    #[test]
    fn writes_repair_replacements_and_keep_deferred_results_bound_to_their_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conv_fixture.json");
        let output = "file data\n".repeat(1100);
        let mut answer = message(1, "assistant");
        answer["tool_calls"] = json!([{"id":"tool","name":"Read","source":"native","status":"success","arguments":r#"{"path":"data.csv"}"#,"result_preview":output}]);
        let mut doc = source(vec![message(0, "user"), answer.clone()]);
        let write = |doc: &Value| {
            let conversation: Conversation = serde_json::from_value(doc.clone()).unwrap();
            crate::chat::storage::write_conversation_file_at_path(&path, conversation).unwrap();
        };
        write(&doc);
        let hash = reopen(&path, |view| {
            let page = view.load_rows(&[0..2], true)?;
            assert!(page.messages[1].tool_calls[0].result_preview.is_none());
            let hash = page.deferred["m1"]["tool"].clone();
            assert_eq!(view.tool_result("m1", "tool", &hash)?, output);
            Ok(hash)
        });
        doc["messages"]
            .as_array_mut()
            .unwrap()
            .push(message(2, "user"));
        doc["revision"] = json!(2);
        write(&doc);
        reopen(&path, |view| {
            assert_eq!(view.revision, 2);
            assert_eq!(view.tool_result("m1", "tool", &hash)?, output);
            Ok(())
        });
        // A replaced file can retain its revision (restore/import) and bypass
        // record_write (a crash). No stale projection or ambiguous disclosure may leak.
        doc["messages"].as_array_mut().unwrap().push(answer);
        let replacement = path.with_extension("new");
        std::fs::write(&replacement, doc.to_string()).unwrap();
        std::fs::rename(replacement, &path).unwrap();
        reopen(&path, |view| {
            let page = view.load_rows(&[3..4], true)?;
            assert!(page.deferred.is_empty());
            assert_eq!(
                page.messages[0].tool_calls[0].result_preview.as_deref(),
                Some(output.as_str())
            );
            assert!(view.tool_result("m1", "tool", &hash).is_err());
            Ok(())
        });
        doc["messages"] = json!([message(0, "user"), message(1, "assistant")]);
        write(&doc);
        reopen(&path, |view| {
            assert_eq!(view.total, 2);
            assert!(view.tool_result("m1", "tool", &hash).is_err());
            Ok(())
        });
    }
}
