use super::*;
use rusqlite::{
    params, params_from_iter, types::Value, Connection, OptionalExtension, TransactionBehavior,
};
use std::io::{BufRead, BufReader};

fn unsigned<T: TryFrom<i64>>(value: i64) -> rusqlite::Result<T> {
    T::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, value))
}

fn db_error(error: impl std::fmt::Display) -> String {
    format!("usage database: {error}")
}

pub(super) fn open(dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(dir).map_err(db_error)?;
    let mut db = Connection::open(dir.join("usage.sqlite3")).map_err(db_error)?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(db_error)?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(db_error)?;
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(db_error)?;
    if version > 4 {
        return Err("usage database version is newer than this application".into());
    }
    if version == 0 {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let version: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if version == 0 {
            tx.execute_batch(SCHEMA).map_err(db_error)?;
            let mut skipped = 0usize;
            for entry in fs::read_dir(dir).map_err(db_error)? {
                let path = entry.map_err(db_error)?.path();
                if path.extension().and_then(|v| v.to_str()) != Some("jsonl") {
                    continue;
                }
                let file = fs::File::open(&path).map_err(db_error)?;
                for line in BufReader::new(file).lines() {
                    let line = line.map_err(db_error)?;
                    if line.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<UsageRecord>(&line) {
                        Ok(record) => {
                            insert(&tx, &record)?;
                        }
                        Err(_) => skipped += 1,
                    }
                }
            }
            tx.execute("INSERT INTO migration VALUES (?1)", [skipped as i64])
                .map_err(db_error)?;
            tx.execute_batch("PRAGMA user_version=1")
                .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
    }
    if version < 2 {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS cli_files (path TEXT PRIMARY KEY, stamp TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS cli_state (id INTEGER PRIMARY KEY CHECK(id=1), cleared_at INTEGER NOT NULL);
            INSERT OR IGNORE INTO cli_state VALUES (1,0); PRAGMA user_version=2;").map_err(db_error)?;
        tx.commit().map_err(db_error)?;
    }
    if version < 3 {
        db.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS cli_superseded (id TEXT PRIMARY KEY); PRAGMA user_version=3; COMMIT;").map_err(db_error)?;
    }
    if version < 4 {
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let current: i64 = tx
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if current < 4 {
            let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('records') WHERE name='service_tier')",[],|r|r.get(0)).map_err(db_error)?;
            if !present {
                tx.execute_batch("ALTER TABLE records ADD COLUMN service_tier TEXT")
                    .map_err(db_error)?;
            }
            tx.execute_batch("CREATE TABLE IF NOT EXISTS price_cache(id INTEGER PRIMARY KEY CHECK(id=1),fetched_at INTEGER NOT NULL,document TEXT NOT NULL); PRAGMA user_version=4;").map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
    }
    Ok(db)
}

const SCHEMA: &str = "CREATE TABLE records (
id TEXT PRIMARY KEY,
created_at INTEGER NOT NULL,
completed_at INTEGER NOT NULL,
duration_ms INTEGER NOT NULL,
first_token_ms INTEGER,
source TEXT NOT NULL,
operation TEXT NOT NULL,
provider_id TEXT NOT NULL,
provider_name TEXT NOT NULL,
model TEXT NOT NULL,
api_format TEXT NOT NULL,
status TEXT NOT NULL,
status_code INTEGER,
usage_source TEXT NOT NULL,
input_tokens INTEGER,
output_tokens INTEGER,
total_tokens INTEGER,
cached_input_tokens INTEGER,
cache_creation_input_tokens INTEGER,
cache_creation_input_tokens_long INTEGER,
reasoning_tokens INTEGER,
reasoning_effort TEXT,
cost_usd REAL,
cost_source TEXT NOT NULL,
conversation_id TEXT,
message_id TEXT,
error_kind TEXT, service_tier TEXT);
CREATE INDEX records_created ON records(created_at DESC, id DESC);
CREATE INDEX records_conversation ON records(conversation_id);
CREATE TABLE migration (skipped INTEGER NOT NULL);
CREATE VIEW accounted AS SELECT *,
 CASE WHEN api_format='anthropic_messages' THEN coalesce(input_tokens,0)+coalesce(cached_input_tokens,0)+coalesce(cache_creation_input_tokens,0) ELSE coalesce(input_tokens,0) END AS effective_input,
 CASE WHEN api_format='anthropic_messages' THEN coalesce(input_tokens,0)+coalesce(cached_input_tokens,0)+coalesce(cache_creation_input_tokens,0)+coalesce(output_tokens,0) ELSE coalesce(total_tokens,coalesce(input_tokens,0)+coalesce(output_tokens,0)) END AS effective_total
 FROM records;";

fn insert(db: &Connection, record: &UsageRecord) -> Result<(), String> {
    if record.cost_usd.is_some_and(|cost| !cost.is_finite()) {
        return Err("usage cost must be finite".into());
    }
    db.execute("INSERT INTO records (id,created_at,completed_at,duration_ms,first_token_ms,source,operation,provider_id,provider_name,model,api_format,status,status_code,usage_source,input_tokens,output_tokens,total_tokens,cached_input_tokens,cache_creation_input_tokens,cache_creation_input_tokens_long,reasoning_tokens,reasoning_effort,cost_usd,cost_source,conversation_id,message_id,error_kind) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27) ON CONFLICT(id) DO NOTHING", params![
        record.id,
        record.created_at,
        record.completed_at,
        i64::try_from(record.duration_ms).map_err(db_error)?,
        record.first_token_ms.map(i64::try_from).transpose().map_err(db_error)?,
        record.source,
        record.operation,
        record.provider_id,
        record.provider_name,
        record.model,
        record.api_format,
        record.status,
        record.status_code,
        record.usage_source,
        record.input_tokens.map(i64::try_from).transpose().map_err(db_error)?,
        record.output_tokens.map(i64::try_from).transpose().map_err(db_error)?,
        record.total_tokens.map(i64::try_from).transpose().map_err(db_error)?,
        record.cached_input_tokens.map(i64::try_from).transpose().map_err(db_error)?,
        record.cache_creation_input_tokens.map(i64::try_from).transpose().map_err(db_error)?,
        record.cache_creation_input_tokens_long.map(i64::try_from).transpose().map_err(db_error)?,
        record.reasoning_tokens.map(i64::try_from).transpose().map_err(db_error)?,
        record.reasoning_effort,
        record.cost_usd,
        record.cost_source,
        record.conversation_id,
        record.message_id,
        record.error_kind
    ]).map_err(db_error)?;
    if record.service_tier.is_some() {
        db.execute(
            "UPDATE records SET service_tier=?1 WHERE id=?2",
            params![record.service_tier, record.id],
        )
        .map_err(db_error)?;
    }
    Ok(())
}
pub(super) fn read_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecord> {
    Ok(UsageRecord {
        id: row.get("id")?,
        created_at: row.get("created_at")?,
        completed_at: row.get("completed_at")?,
        duration_ms: unsigned(row.get("duration_ms")?)?,
        first_token_ms: row
            .get::<_, Option<i64>>("first_token_ms")?
            .map(unsigned)
            .transpose()?,
        source: row.get("source")?,
        operation: row.get("operation")?,
        provider_id: row.get("provider_id")?,
        provider_name: row.get("provider_name")?,
        model: row.get("model")?,
        api_format: row.get("api_format")?,
        status: row.get("status")?,
        status_code: row.get("status_code")?,
        usage_source: row.get("usage_source")?,
        input_tokens: row
            .get::<_, Option<i64>>("input_tokens")?
            .map(unsigned)
            .transpose()?,
        output_tokens: row
            .get::<_, Option<i64>>("output_tokens")?
            .map(unsigned)
            .transpose()?,
        total_tokens: row
            .get::<_, Option<i64>>("total_tokens")?
            .map(unsigned)
            .transpose()?,
        cached_input_tokens: row
            .get::<_, Option<i64>>("cached_input_tokens")?
            .map(unsigned)
            .transpose()?,
        cache_creation_input_tokens: row
            .get::<_, Option<i64>>("cache_creation_input_tokens")?
            .map(unsigned)
            .transpose()?,
        cache_creation_input_tokens_long: row
            .get::<_, Option<i64>>("cache_creation_input_tokens_long")?
            .map(unsigned)
            .transpose()?,
        reasoning_tokens: row
            .get::<_, Option<i64>>("reasoning_tokens")?
            .map(unsigned)
            .transpose()?,
        reasoning_effort: row.get("reasoning_effort")?,
        service_tier: row.get("service_tier")?,
        cost_usd: row.get("cost_usd")?,
        cost_source: row.get("cost_source")?,
        conversation_id: row.get("conversation_id")?,
        message_id: row.get("message_id")?,
        error_kind: row.get("error_kind")?,
    })
}

const TOTALS: &str = "count(*), coalesce(sum(status='success'),0),
 coalesce(sum(usage_source='missing'),0), coalesce(sum(usage_source='provider_reported'),0),
 coalesce(sum(effective_total),0), coalesce(sum(effective_input),0),
 coalesce(sum(output_tokens),0), coalesce(sum(cached_input_tokens),0),
 coalesce(sum(cache_creation_input_tokens),0), coalesce(sum(reasoning_tokens),0),
 coalesce(sum(cost_usd),0.0), avg(CASE WHEN source != 'cli' THEN duration_ms END), count(DISTINCT conversation_id), coalesce(sum(cost_usd IS NULL),0)";

fn summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageSummary> {
    let total_requests: u64 = unsigned(row.get(0)?)?;
    let successful_requests: u64 = unsigned(row.get(1)?)?;
    Ok(UsageSummary {
        total_requests,
        successful_requests,
        failed_requests: total_requests - successful_requests,
        missing_usage_requests: unsigned(row.get(2)?)?,
        provider_reported_requests: unsigned(row.get(3)?)?,
        total_tokens: unsigned(row.get(4)?)?,
        input_tokens: unsigned(row.get(5)?)?,
        output_tokens: unsigned(row.get(6)?)?,
        cached_input_tokens: unsigned(row.get(7)?)?,
        cache_creation_input_tokens: unsigned(row.get(8)?)?,
        reasoning_tokens: unsigned(row.get(9)?)?,
        total_cost_usd: row.get(10)?,
        average_duration_ms: row.get(11)?,
        session_count: unsigned(row.get(12)?)?,
        unpriced_requests: unsigned(row.get(13)?)?,
    })
}

fn filters(query: &UsageStatsQuery) -> (String, Vec<Value>) {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    if let Some(start) = range_start(&query.range) {
        clauses.push("created_at >= ?");
        values.push(Value::Integer(start));
    }
    if let Some(source) = normalized_filter(query.source.as_deref()) {
        clauses.push("source = ?");
        values.push(Value::Text(source));
    }
    if let Some(status) = normalized_filter(query.status.as_deref()) {
        if status == "missing_usage" {
            clauses.push("usage_source = 'missing'");
        } else {
            clauses.push("status = ?");
            values.push(Value::Text(status));
        }
    }
    if let Some(search) = normalized_search(query.provider_search.as_deref()) {
        clauses.push("instr(lower(provider_id || ' ' || provider_name), ?) > 0");
        values.push(Value::Text(search));
    }
    if let Some(search) = normalized_search(query.model_search.as_deref()) {
        clauses.push("instr(lower(model), ?) > 0");
        values.push(Value::Text(search));
    }
    (
        if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        },
        values,
    )
}

fn skipped(db: &Connection) -> Result<usize, String> {
    db.query_row("SELECT skipped FROM migration", [], |r| unsigned(r.get(0)?))
        .map_err(db_error)
}

fn summary(db: &Connection, filter: &str, values: &[Value]) -> Result<UsageSummary, String> {
    db.query_row(
        &format!("SELECT {TOTALS} FROM accounted{filter}"),
        params_from_iter(values),
        summary_row,
    )
    .map_err(db_error)
}

fn groups(
    db: &Connection,
    filter: &str,
    values: &[Value],
    models: bool,
) -> Result<Vec<UsageGroupStats>, String> {
    let group = if models {
        "provider_id, model"
    } else {
        "provider_id"
    };
    // The bare name/model belong to the newest row selected by SQLite's single max().
    let mut stmt = db.prepare(&format!("SELECT {TOTALS}, provider_id, provider_name, model, max(created_at) FROM accounted{filter} GROUP BY {group} ORDER BY 5 DESC, provider_id, model")).map_err(db_error)?;
    let rows = stmt
        .query_map(params_from_iter(values), |r| {
            let totals = summary_row(r)?;
            let provider_id: String = r.get(14)?;
            let provider_name: String = r.get(15)?;
            let model: String = r.get(16)?;
            Ok(UsageGroupStats {
                id: if models {
                    format!("{provider_id}::{model}")
                } else {
                    provider_id.clone()
                },
                label: if models {
                    model.clone()
                } else if provider_name.trim().is_empty() {
                    provider_id.clone()
                } else {
                    provider_name.clone()
                },
                provider_id: Some(provider_id),
                provider_name: Some(provider_name),
                model: models.then_some(model),
                session_count: totals.session_count,
                unpriced_requests: totals.unpriced_requests,
                request_count: totals.total_requests,
                success_count: totals.successful_requests,
                total_tokens: totals.total_tokens,
                input_tokens: totals.input_tokens,
                output_tokens: totals.output_tokens,
                cached_input_tokens: totals.cached_input_tokens,
                cache_creation_input_tokens: totals.cache_creation_input_tokens,
                cost_usd: totals.total_cost_usd,
                average_duration_ms: totals.average_duration_ms,
                last_used_at: r.get(17)?,
            })
        })
        .map_err(db_error)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
}

fn trend(
    db: &Connection,
    filter: &str,
    values: &[Value],
    range: &str,
) -> Result<Vec<UsageTrendPoint>, String> {
    let hourly = matches!(range, "today" | "1d");
    let format = if hourly { "%Y-%m-%d %H" } else { "%Y-%m-%d" };
    let label = if hourly { "%H:00" } else { "%m/%d" };
    let mut points: BTreeMap<_, _> = empty_trend(range)
        .into_iter()
        .map(|p| (p.date.clone(), p))
        .collect();
    let mut stmt = db.prepare(&format!("SELECT {TOTALS}, strftime('{format}',created_at,'unixepoch','localtime') AS bucket, strftime('{label}',created_at,'unixepoch','localtime') FROM accounted{filter} GROUP BY bucket")).map_err(db_error)?;
    let rows = stmt
        .query_map(params_from_iter(values), |r| {
            let totals = summary_row(r)?;
            Ok(UsageTrendPoint {
                date: r.get(14)?,
                label: r.get(15)?,
                requests: totals.total_requests,
                total_tokens: totals.total_tokens,
                input_tokens: totals.input_tokens,
                output_tokens: totals.output_tokens,
                cached_input_tokens: totals.cached_input_tokens,
                cache_creation_input_tokens: totals.cache_creation_input_tokens,
                cost_usd: totals.total_cost_usd,
            })
        })
        .map_err(db_error)?;
    for point in rows {
        let point = point.map_err(db_error)?;
        points.insert(point.date.clone(), point);
    }
    Ok(points.into_values().collect())
}

#[derive(Default)]
pub(crate) struct UsageLedger;

impl UsageLedger {
    pub(super) fn append(&self, dir: &Path, record: &UsageRecord) -> Result<(), String> {
        insert(&open(dir)?, record)
    }

    pub(super) fn stats(
        &self,
        dir: &Path,
        query: UsageStatsQuery,
    ) -> Result<UsageStatsResponse, String> {
        let mut db = open(dir)?;
        let tx = db.transaction().map_err(db_error)?;
        let (filter, mut values) = filters(&query);
        let summary = summary(&tx, &filter, &values)?;
        let trend = trend(&tx, &filter, &values, &query.range)?;
        let provider_stats = groups(&tx, &filter, &values, false)?;
        let model_stats = groups(&tx, &filter, &values, true)?;
        let skipped_records = skipped(&tx)?;
        values.push(Value::Integer(
            query.limit.unwrap_or(DEFAULT_LOG_LIMIT).min(MAX_LOG_LIMIT) as i64,
        ));
        values.push(Value::Integer(
            i64::try_from(query.offset.unwrap_or(0)).unwrap_or(i64::MAX),
        ));
        let mut stmt = tx
            .prepare(&format!(
                "SELECT * FROM records{filter} ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?"
            ))
            .map_err(db_error)?;
        let logs = stmt
            .query_map(params_from_iter(&values), read_record)
            .map_err(db_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        Ok(UsageStatsResponse {
            total_logs: summary.total_requests as usize,
            summary,
            trend,
            provider_stats,
            model_stats,
            logs,
            skipped_records,
        })
    }

    pub(super) fn conversation_cost(
        &self,
        dir: &Path,
        conversation_id: &str,
    ) -> Result<ConversationCost, String> {
        let mut db = open(dir)?;
        let tx = db.transaction().map_err(db_error)?;
        let prefix = format!("subagent-{conversation_id}-");
        // Binary range keeps prefix matching literal (IDs may contain SQL wildcards).
        // shortcut: a non-UUID root may prefix another root; store explicit ancestry if IDs change.
        let upper = format!("subagent-{conversation_id}.");
        let mut result = tx.query_row(
            "SELECT sum(CASE WHEN cost_usd >= 0 AND usage_source != 'missing' THEN cost_usd END),
             coalesce(sum(cost_usd IS NULL OR cost_usd < 0 OR usage_source='missing'),0)
             FROM records WHERE conversation_id=?1 OR (conversation_id>=?2 AND conversation_id<?3)",
            params![conversation_id, prefix, upper],
            |r| Ok(ConversationCost {
                cost_usd: r.get(0)?,
                unpriced_requests: unsigned(r.get(1)?)?,
                skipped_records: 0,
            }),
        ).map_err(db_error)?;
        result.skipped_records = skipped(&tx)?;
        Ok(result)
    }

    pub(super) fn clear(&self, dir: &Path) -> Result<(), String> {
        let mut db = open(dir)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        tx.execute("DELETE FROM records", []).map_err(db_error)?;
        tx.execute(
            "UPDATE cli_state SET cleared_at=?1 WHERE id=1",
            [Local::now().timestamp()],
        )
        .map_err(db_error)?;
        tx.execute("UPDATE migration SET skipped=0", [])
            .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
}

pub(super) fn today(dir: &Path) -> Result<TodayUsage, String> {
    let mut db = open(dir)?;
    let tx = db.transaction().map_err(db_error)?;
    let query = UsageStatsQuery {
        range: "today".into(),
        ..Default::default()
    };
    let (filter, values) = filters(&query);
    let missing_cost_requests = tx
        .query_row(
            &format!("SELECT count(*) FROM (SELECT cost_usd FROM records{filter}) WHERE cost_usd IS NULL"),
            params_from_iter(&values),
            |r| unsigned(r.get(0)?),
        )
        .map_err(db_error)?;
    Ok(TodayUsage {
        date: Local::now().date_naive(),
        summary: summary(&tx, &filter, &values)?,
        missing_cost_requests,
        skipped_records: skipped(&tx)?,
    })
}

pub(super) fn cli_file_unchanged(db: &Connection, path: &str, stamp: &str) -> Result<bool, String> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM cli_files WHERE path=?1 AND stamp=?2)",
        params![path, stamp],
        |r| r.get(0),
    )
    .map_err(db_error)
}
pub(super) fn save_cli_file(
    db: &mut Connection,
    path: &str,
    stamp: &str,
    records: &[UsageRecord],
    superseded: &[String],
) -> Result<(), String> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let cleared: i64 = tx
        .query_row("SELECT cleared_at FROM cli_state WHERE id=1", [], |r| {
            r.get(0)
        })
        .map_err(db_error)?;
    for id in superseded {
        tx.execute("INSERT OR IGNORE INTO cli_superseded VALUES (?1)", [id])
            .map_err(db_error)?;
        tx.execute("DELETE FROM records WHERE id=?1 AND source='cli'", [id])
            .map_err(db_error)?;
    }
    for record in records.iter().filter(|r| r.created_at > cleared) {
        let obsolete: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cli_superseded WHERE id=?1)",
                [&record.id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if obsolete {
            continue;
        }
        let mut merged = tx
            .query_row(
                "SELECT * FROM records WHERE id=?1 AND source='cli'",
                [&record.id],
                read_record,
            )
            .optional()
            .map_err(db_error)?;
        if let Some(previous) = merged.as_mut() {
            native::merge_records(previous, record);
        }
        tx.execute(
            "DELETE FROM records WHERE id=?1 AND source='cli'",
            [&record.id],
        )
        .map_err(db_error)?;
        insert(&tx, merged.as_ref().unwrap_or(record))?;
    }
    tx.execute(
        "INSERT INTO cli_files VALUES (?1,?2) ON CONFLICT(path) DO UPDATE SET stamp=excluded.stamp",
        params![path, stamp],
    )
    .map_err(db_error)?;
    tx.commit().map_err(db_error)
}
