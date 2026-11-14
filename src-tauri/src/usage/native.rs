use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Read};

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub sources: Vec<SourceReport>,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReport {
    pub id: String,
    pub label: String,
    pub files: usize,
    pub records: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
}

const SOURCES: &[(&str, &str, &str, &str)] = &[
    ("codex", "Codex", "CODEX_HOME", ".codex"),
    ("claude", "Claude Code", "CLAUDE_CONFIG_DIR", ".claude"),
    ("grok", "Grok Build", "GROK_HOME", ".grok"),
    ("pi", "Pi", "PI_CODING_AGENT_DIR", ".pi/agent"),
    ("kimi", "Kimi Code", "KIMI_CODE_HOME", ".kimi-code"),
    (
        "opencode",
        "OpenCode",
        "OPENCODE_DATA_DIR",
        ".local/share/opencode",
    ),
];

fn expand(value: &str, home: &Path) -> PathBuf {
    value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
        .map(|p| home.join(p))
        .unwrap_or_else(|| PathBuf::from(value))
}
fn roots(id: &str, variable: &str, fallback: &str, home: &Path) -> Vec<PathBuf> {
    let env = crate::external_agents::overrides::env_for(id);
    let configured = |key: &str| {
        env.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var(key).ok())
    };
    let mut homes = vec![std::env::var(variable)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| expand(&s, home))
        .unwrap_or_else(|| home.join(fallback))];
    if let Some(value) = configured(variable).filter(|s| !s.trim().is_empty()) {
        homes.push(expand(&value, home));
    }
    let mut result = Vec::new();
    for base in homes {
        if !base.is_absolute() {
            continue;
        }
        match id {
            "claude" => result.push(base.join("projects")),
            "opencode" => result.push(base.join("opencode.db")),
            "codex" => {
                result.push(base.join("sessions"));
                result.push(base.join("archived_sessions"));
            }
            _ => result.push(base.join("sessions")),
        }
    }
    if id == "pi" {
        if let Some(path) = configured("PI_CODING_AGENT_SESSION_DIR") {
            let p = expand(&path, home);
            if p.is_absolute() {
                result.push(p);
            }
        }
    }
    if id == "opencode" {
        if let Some(path) = configured("XDG_DATA_HOME") {
            let p = expand(&path, home);
            if p.is_absolute() {
                result.push(p.join("opencode/opencode.db"));
            }
        }
    }
    result
}

pub(super) fn scan(dir: &Path) -> Result<ScanReport, String> {
    // Refreshes from multiple windows share one scan; model-call writes use separate connections.
    static SCAN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SCAN.lock().map_err(|_| "CLI usage scan lock poisoned")?;
    let home = directories::BaseDirs::new()
        .ok_or("Home directory unavailable")?
        .home_dir()
        .to_path_buf();
    let mut db = store::open(dir)?;
    let mut report = ScanReport::default();
    for &(id, label, variable, fallback) in SOURCES {
        let mut source = SourceReport {
            id: id.into(),
            label: label.into(),
            ..Default::default()
        };
        let mut visited = HashSet::new();
        for root in roots(id, variable, fallback, &home) {
            if let Err(error) = scan_path(&mut db, id, &root, &mut visited, &mut source) {
                source.errors.push(error);
            }
        }
        report.sources.push(source);
    }
    Ok(report)
}

fn scan_path(
    db: &mut rusqlite::Connection,
    id: &str,
    path: &Path,
    visited: &mut HashSet<PathBuf>,
    report: &mut SourceReport,
) -> Result<(), String> {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_symlink() {
                continue;
            }
            if let Err(error) = scan_path(db, id, &entry.path(), visited, report) {
                report.errors.push(error);
            }
        }
        return Ok(());
    }
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if id == "opencode" && name != "opencode.db" {
        return Ok(());
    }
    if id != "opencode"
        && !name.ends_with(".jsonl")
        && !(id == "codex" && name.ends_with(".jsonl.zst"))
    {
        return Ok(());
    }
    if id == "grok" && name != "updates.jsonl" || id == "kimi" && name != "wire.jsonl" {
        return Ok(());
    }
    report.files += 1;
    let stamp = format!(
        "v4:{}:{:?}",
        metadata.len(),
        metadata.modified().map_err(|e| e.to_string())?
    );
    let key = format!("{id}:{}", canonical.display());
    if id != "opencode" && store::cli_file_unchanged(db, &key, &stamp)? {
        return Ok(());
    }
    let (records, skipped, superseded) = if id == "opencode" {
        let (records, skipped) = read_opencode(path)?;
        (records, skipped, Vec::new())
    } else {
        read_transcript(id, path)?
    };
    report.skipped += skipped;
    report.records += records.len();
    store::save_cli_file(db, &key, &stamp, &records, &superseded)?;
    Ok(())
}

fn count(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}
fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn timestamp(v: &Value) -> Option<i64> {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .or_else(|| {
            v.as_i64().map(|n| {
                if n > 1_000_000_000_000 {
                    n
                } else {
                    n.checked_mul(1000).unwrap_or(0)
                }
            })
        })
}
fn identity(parts: &[&str]) -> String {
    format!("cli:{:x}", Sha256::digest(parts.join("\0").as_bytes()))
}

fn record(
    id: &str,
    session: &str,
    key: &str,
    model: &str,
    at: i64,
    usage: ModelUsage,
    reported: Option<f64>,
) -> Option<UsageRecord> {
    if model.is_empty() || model == "<synthetic>" || !(1..=253_402_300_799_999).contains(&at) {
        return None;
    }
    let provider: ModelProvider = serde_json::from_value(serde_json::json!({"id":format!("cli:{id}"),"name":SOURCES.iter().find(|s|s.0==id)?.1,"baseUrl":"","apiFormat":"openai_chat"})).ok()?;
    let price = estimated_price(model, &usage);
    let reported = reported.filter(|v| v.is_finite() && *v >= 0.0);
    let (cost_usd, cost_source) = match reported {
        Some(v) => (Some(v), "cli_reported".into()),
        None => match price {
            Some(v) => (Some(v), "api_estimate".into()),
            None => (None, "unavailable".into()),
        },
    };
    if usage
        .input_tokens
        .unwrap_or(0)
        .saturating_add(usage.output_tokens.unwrap_or(0))
        == 0
        && cost_usd.unwrap_or(0.0) == 0.0
    {
        return None;
    }
    Some(UsageRecord {
        id: identity(&[id, key]),
        created_at: at / 1000,
        completed_at: at / 1000,
        duration_ms: 0,
        first_token_ms: None,
        source: "cli".into(),
        operation: id.into(),
        provider_id: provider.id,
        provider_name: provider.name,
        model: model.into(),
        api_format: "cli_inclusive".into(),
        status: "success".into(),
        status_code: None,
        usage_source: "provider_reported".into(),
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
        cache_creation_input_tokens_long: usage.cache_creation_input_tokens_long,
        reasoning_tokens: usage.reasoning_tokens,
        reasoning_effort: None,
        service_tier: None,
        cost_usd,
        cost_source,
        conversation_id: Some(format!("cli:{id}:{session}")),
        message_id: Some(key.into()),
        error_kind: None,
    })
}
fn estimated_price(model: &str, usage: &ModelUsage) -> Option<f64> {
    pricing::estimate(model, usage, "cli_inclusive", None)
}

pub(super) fn merge_records(previous: &mut UsageRecord, next: &UsageRecord) {
    if previous.operation == "grok" {
        *previous = next.clone();
        return;
    }
    previous.service_tier = next.service_tier.clone().or(previous.service_tier.clone());
    let ordinary = |r: &UsageRecord| {
        r.input_tokens
            .unwrap_or(0)
            .saturating_sub(r.cached_input_tokens.unwrap_or(0))
            .saturating_sub(r.cache_creation_input_tokens.unwrap_or(0))
    };
    let uncached = ordinary(previous).max(ordinary(next));
    previous.output_tokens = previous.output_tokens.max(next.output_tokens);
    previous.cached_input_tokens = previous.cached_input_tokens.max(next.cached_input_tokens);
    previous.cache_creation_input_tokens = previous
        .cache_creation_input_tokens
        .max(next.cache_creation_input_tokens);
    previous.cache_creation_input_tokens_long = previous
        .cache_creation_input_tokens_long
        .max(next.cache_creation_input_tokens_long);
    previous.reasoning_tokens = previous.reasoning_tokens.max(next.reasoning_tokens);
    previous.input_tokens = Some(
        uncached
            .saturating_add(previous.cached_input_tokens.unwrap_or(0))
            .saturating_add(previous.cache_creation_input_tokens.unwrap_or(0)),
    );
    previous.total_tokens = Some(
        previous
            .input_tokens
            .unwrap_or(0)
            .saturating_add(previous.output_tokens.unwrap_or(0)),
    );
    if next.cost_source == "cli_reported" || previous.cost_source == "cli_reported" {
        previous.cost_usd = match (previous.cost_source.as_str(), next.cost_source.as_str()) {
            ("cli_reported", "cli_reported") => match (previous.cost_usd, next.cost_usd) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            },
            (_, "cli_reported") => next.cost_usd,
            _ => previous.cost_usd,
        };
        previous.cost_source = "cli_reported".into();
    } else {
        previous.cost_usd = estimated_price(
            &previous.model,
            &ModelUsage {
                input_tokens: previous.input_tokens,
                output_tokens: previous.output_tokens,
                cached_input_tokens: previous.cached_input_tokens,
                cache_creation_input_tokens: previous.cache_creation_input_tokens,
                cache_creation_input_tokens_long: previous.cache_creation_input_tokens_long,
                ..Default::default()
            },
        );
        previous.cost_source = if previous.cost_usd.is_some() {
            "api_estimate"
        } else {
            "unavailable"
        }
        .into();
    }
}

fn tokens(input: u64, output: u64, read: u64, write: u64, inclusive: bool) -> ModelUsage {
    let input = if inclusive {
        input
    } else {
        input.saturating_add(read).saturating_add(write)
    };
    ModelUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: Some(input.saturating_add(output)),
        cached_input_tokens: Some(read.min(input)),
        cache_creation_input_tokens: Some(write.min(input.saturating_sub(read))),
        ..Default::default()
    }
}

fn codex_counts(v: &Value) -> Option<[u64; 6]> {
    let input = v["input_tokens"].as_u64()?;
    let output = v["output_tokens"].as_u64()?;
    let counts = [
        input,
        count(v, "cached_input_tokens"),
        count(v, "cache_write_input_tokens"),
        output,
        count(v, "reasoning_output_tokens"),
        v["total_tokens"]
            .as_u64()
            .unwrap_or(input.saturating_add(output)),
    ];
    valid_codex_counts(counts).then_some(counts)
}
fn valid_codex_counts(c: [u64; 6]) -> bool {
    c[1].saturating_add(c[2]) <= c[0] && c[4] <= c[3] && c[5] >= c[0].saturating_add(c[3])
}
fn codex_difference(a: [u64; 6], b: [u64; 6]) -> Option<[u64; 6]> {
    if (0..6).any(|i| a[i] < b[i]) {
        return None;
    }
    let difference = std::array::from_fn(|i| a[i] - b[i]);
    valid_codex_counts(difference).then_some(difference)
}

#[derive(Clone)]
struct CodexPending {
    at: i64,
    model: String,
    speed: Option<String>,
    counts: [u64; 6],
}
fn pending_record(session: &str, pending: CodexPending) -> Option<UsageRecord> {
    let c = pending.counts;
    let mut usage = tokens(c[0], c[3], c[1], c[2], true);
    usage.reasoning_tokens = Some(c[4]);
    let mut r = record(
        "codex",
        session,
        &format!("{session}:{}:pending:{c:?}", pending.at),
        &pending.model,
        pending.at,
        usage,
        None,
    )?;
    r.service_tier = pending.speed;
    Some(r)
}

#[derive(Default)]
struct ParseState {
    session: String,
    file: String,
    model: String,
    service_tier: Option<String>,
    cumulative: Option<[u64; 6]>,
    last_usage: Option<[u64; 6]>,
    sequence_start: Option<(i64, [u64; 6])>,
    pending: Vec<CodexPending>,
    ledger: bool,
    snapshot_ids: Vec<String>,
    superseded: Vec<String>,
    seen_meta: bool,
    fork_at: Option<i64>,
    latest_snapshot_at: i64,
}
fn parse(id: &str, v: &Value, state: &mut ParseState, line: usize) -> Vec<UsageRecord> {
    let mut out = Vec::new();
    let at = timestamp(&v["timestamp"]).or_else(|| timestamp(&v["time"]));
    match id {
        "claude" if string(v, "type") == "assistant" => {
            let m = &v["message"];
            let u = &m["usage"];
            if !u.is_object() {
                return out;
            }
            let key = format!("{}:{}", string(m, "id"), string(v, "requestId"));
            if key == ":" {
                return out;
            }
            let mut usage = tokens(
                count(u, "input_tokens"),
                count(u, "output_tokens"),
                count(u, "cache_read_input_tokens"),
                count(u, "cache_creation_input_tokens"),
                false,
            );
            usage.cache_creation_input_tokens_long =
                u["cache_creation"]["ephemeral_1h_input_tokens"].as_u64();
            if let Some(r) = at.and_then(|at| {
                record(
                    id,
                    string(v, "sessionId"),
                    &key,
                    string(m, "model"),
                    at,
                    usage,
                    v["costUSD"].as_f64(),
                )
            }) {
                out.push(r);
            }
            for (index, advisor) in u["iterations"].as_array().into_iter().flatten().enumerate() {
                if string(advisor, "type") != "advisor_message" {
                    continue;
                }
                let mut usage = tokens(
                    count(advisor, "input_tokens"),
                    count(advisor, "output_tokens"),
                    count(advisor, "cache_read_input_tokens"),
                    count(advisor, "cache_creation_input_tokens"),
                    false,
                );
                usage.cache_creation_input_tokens_long =
                    advisor["cache_creation"]["ephemeral_1h_input_tokens"].as_u64();
                if let Some(r) = at.and_then(|at| {
                    record(
                        id,
                        string(v, "sessionId"),
                        &format!("{key}:advisor:{index}"),
                        string(advisor, "model"),
                        at,
                        usage,
                        None,
                    )
                }) {
                    out.push(r);
                }
            }
        }
        "pi" => {
            if string(v, "type") == "session" {
                state.session = string(v, "id").into();
                return out;
            }
            let m = if string(v, "type") == "message" {
                &v["message"]
            } else {
                v
            };
            if string(v, "type") == "message"
                && !matches!(string(m, "role"), "assistant" | "toolResult")
            {
                return out;
            }
            let u = &m["usage"];
            if !u.is_object() {
                return out;
            }
            let model = if string(m, "model").is_empty() {
                "Tools/summaries"
            } else {
                m.get("responseModel")
                    .and_then(Value::as_str)
                    .unwrap_or(string(m, "model"))
            };
            let key = format!("{}:{}:{model}", string(v, "id"), string(v, "timestamp"));
            let usage = tokens(
                count(u, "input"),
                count(u, "output"),
                count(u, "cacheRead"),
                count(u, "cacheWrite"),
                false,
            );
            if let Some(r) = at.and_then(|at| {
                record(
                    id,
                    &state.session,
                    &key,
                    model,
                    at,
                    usage,
                    u["cost"]["total"].as_f64().filter(|c| *c > 0.0),
                )
            }) {
                out.push(r);
            }
        }
        "codex" => {
            let p = &v["payload"];
            match string(v, "type") {
                "session_meta" if !state.seen_meta => {
                    state.seen_meta = true;
                    state.session = p["id"]
                        .as_str()
                        .or(p["session_id"].as_str())
                        .unwrap_or("")
                        .into();
                    if p.get("forked_from_id").is_some()
                        || p["source"]["subagent"]["thread_spawn"].is_object()
                    {
                        state.fork_at = at;
                    }
                    return out;
                }
                "turn_context" => {
                    if let Some(model) = p["model"].as_str() {
                        state.model = model.into();
                    }
                    state.ledger = false;
                    state.snapshot_ids.clear();
                    return out;
                }
                _ => {}
            }
            if string(p, "type") == "thread_settings_applied" {
                state.service_tier = p["thread_settings"]["service_tier"]
                    .as_str()
                    .map(str::to_owned);
                return out;
            }
            let Some(at) = at else {
                return out;
            };
            if string(v, "type") != "token_usage_record" && string(p, "type") != "token_count" {
                return out;
            }
            if let Some(anchor) = state.fork_at {
                if at - anchor < 1000 {
                    state.fork_at = Some(at);
                    state.cumulative = codex_counts(&p["info"]["total_token_usage"]);
                    return out;
                }
                state.fork_at = None;
            }
            if string(v, "type") == "token_usage_record" {
                let Some(counts) = codex_counts(&p["usage"]) else {
                    return out;
                };
                if state.model.is_empty() {
                    return out;
                }
                if string(p, "response_id").is_empty() {
                    state.ledger = false;
                    state.pending.push(CodexPending {
                        at,
                        model: state.model.clone(),
                        speed: state.service_tier.clone(),
                        counts,
                    });
                    return out;
                }
                for pending in std::mem::take(&mut state.pending) {
                    let base = state.cumulative.unwrap_or([0; 6]);
                    state.cumulative = Some(std::array::from_fn(|i| {
                        base[i].saturating_add(pending.counts[i])
                    }));
                    if let Some(r) = pending_record(&state.session, pending) {
                        out.push(r);
                    }
                }
                let base = state.cumulative.unwrap_or([0; 6]);
                state.cumulative = Some(std::array::from_fn(|i| base[i].saturating_add(counts[i])));
                state.latest_snapshot_at = state.latest_snapshot_at.max(at);
            }
            let (u, key) = if string(v, "type") == "token_usage_record"
                && !string(p, "response_id").is_empty()
            {
                (
                    &p["usage"],
                    format!("{}:{}", state.session, string(p, "response_id")),
                )
            } else if string(p, "type") == "token_count" {
                let last = codex_counts(&p["info"]["last_token_usage"]);
                let total = codex_counts(&p["info"]["total_token_usage"]);
                let previous = state.cumulative;
                let pending = state.pending.clone();
                let baseline_at = state.latest_snapshot_at;
                if let Some(total) = total {
                    state.pending.clear();
                    state.cumulative = Some(total);
                    state.latest_snapshot_at = baseline_at.max(at);
                }
                let Some(last) = last else {
                    return out;
                };
                if state.ledger || last[0] + last[3] == 0 || last[5] != last[0] + last[3] {
                    return out;
                }
                if let Some(total) = total {
                    if Some(total) == previous {
                        return out;
                    }
                    let restart = total == last;
                    if restart && state.sequence_start == Some((at, last)) {
                        state.cumulative = previous;
                        return out;
                    }
                    let lowered = previous.is_some_and(|p| (0..6).any(|i| total[i] < p[i]));
                    let rewound = lowered && at > baseline_at;
                    let valid = if previous.is_none() || rewound {
                        codex_difference(total, last).is_some()
                    } else {
                        codex_difference(total, previous.unwrap()) == Some(last)
                    };
                    if !restart && !valid {
                        let bridged =
                            previous.is_some_and(|base| {
                                !pending.is_empty()
                                    && (0..6).all(|i| {
                                        total[i].checked_sub(base[i])
                                            == Some(pending.iter().fold(0u64, |sum, p| {
                                                sum.saturating_add(p.counts[i])
                                            }))
                                    })
                            });
                        let remainder = pending
                            .last()
                            .and_then(|p| codex_difference(p.counts, last));
                        if bridged && remainder.is_some() {
                            let count = pending.len();
                            for (index, mut request) in pending.into_iter().enumerate() {
                                if index + 1 == count {
                                    request.counts = remainder.unwrap();
                                }
                                if let Some(r) = pending_record(&state.session, request) {
                                    out.push(r);
                                }
                            }
                        } else {
                            if at < baseline_at
                                || (at == baseline_at && lowered && total[0] + total[3] > 0)
                            {
                                state.cumulative = previous;
                                state.pending = pending;
                            }
                            return out;
                        }
                    }
                    if restart {
                        state.sequence_start = Some((at, last));
                    }
                } else {
                    if !p["info"]["total_token_usage"].is_null() || state.last_usage == Some(last) {
                        return out;
                    }
                    let base = previous.unwrap_or([0; 6]);
                    state.cumulative =
                        Some(std::array::from_fn(|i| base[i].saturating_add(last[i])));
                    state.latest_snapshot_at = baseline_at.max(at);
                }
                state.last_usage = Some(last);
                (
                    &p["info"]["last_token_usage"],
                    format!("{}:{at}:{}", state.session, p["info"]),
                )
            } else {
                return out;
            };
            let (Some(input), Some(output)) =
                (u["input_tokens"].as_u64(), u["output_tokens"].as_u64())
            else {
                return out;
            };
            if count(u, "cached_input_tokens").saturating_add(count(u, "cache_write_input_tokens"))
                > input
                || count(u, "reasoning_output_tokens") > output
            {
                return out;
            }
            let mut usage = tokens(
                input,
                output,
                count(u, "cached_input_tokens"),
                count(u, "cache_write_input_tokens"),
                true,
            );
            usage.reasoning_tokens = u["reasoning_output_tokens"].as_u64();
            if let Some(mut r) = record(id, &state.session, &key, &state.model, at, usage, None) {
                r.service_tier = state.service_tier.clone();
                if string(v, "type") == "token_usage_record" {
                    state.ledger = true;
                    state.superseded.append(&mut state.snapshot_ids);
                } else {
                    state.snapshot_ids.push(r.id.clone());
                }
                out.push(r);
            }
        }
        "kimi" if string(v, "type") == "usage.record" && string(v, "usageScope") == "turn" => {
            let u = &v["usage"];
            let agent = Path::new(&state.file)
                .parent()
                .and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .unwrap_or("");
            let key = format!("{}:{agent}:{}", state.session, identity(&[&v.to_string()]));
            state.superseded.push(identity(&[
                id,
                &format!("{}:{}:{line}", state.file, v["time"]),
            ]));
            let usage = tokens(
                count(u, "inputOther"),
                count(u, "output"),
                count(u, "inputCacheRead"),
                count(u, "inputCacheCreation"),
                false,
            );
            if let Some(r) = at.and_then(|at| {
                record(
                    id,
                    &state.session,
                    &key,
                    string(v, "model"),
                    at,
                    usage,
                    None,
                )
            }) {
                out.push(r);
            }
        }
        "grok" => {
            let p = &v["params"];
            let update = &p["update"];
            if string(update, "sessionUpdate") != "turn_completed" {
                return out;
            }
            let Some(at) = p["_meta"]["agentTimestampMs"].as_i64().or(at) else {
                return out;
            };
            let u = &update["usage"];
            let items: Vec<(&str, &Value)> = match u["modelUsage"].as_object() {
                Some(m) if !m.is_empty() => m.iter().map(|(k, v)| (k.as_str(), v)).collect(),
                _ => vec![("grok", u)],
            };
            let known_cost: f64 = items
                .iter()
                .filter_map(|(_, v)| v["costUsdTicks"].as_f64())
                .filter(|n| *n >= 0.0)
                .sum::<f64>()
                / 10_000_000_000.0;
            let remaining = u["costUsdTicks"]
                .as_f64()
                .filter(|n| *n >= 0.0)
                .map(|n| (n / 10_000_000_000.0 - known_cost).max(0.0));
            let unpriced_tokens: f64 = items
                .iter()
                .filter(|(_, v)| v["costUsdTicks"].as_f64().filter(|n| *n >= 0.0).is_none())
                .map(|(_, v)| count(v, "inputTokens") as f64 + count(v, "outputTokens") as f64)
                .sum();
            for (model, u) in items {
                let key = format!(
                    "{}:{}:{model}",
                    string(p, "sessionId"),
                    update
                        .get("prompt_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("{at}:{line}"))
                );
                let usage = tokens(
                    count(u, "inputTokens"),
                    count(u, "outputTokens"),
                    count(u, "cachedReadTokens"),
                    count(u, "cacheCreationTokens"),
                    true,
                );
                if let Some(r) = record(
                    id,
                    string(p, "sessionId"),
                    &key,
                    model,
                    at,
                    usage,
                    u["costUsdTicks"]
                        .as_f64()
                        .filter(|n| *n >= 0.0)
                        .map(|n| n / 10_000_000_000.0)
                        .or_else(|| {
                            remaining.filter(|_| unpriced_tokens > 0.0).map(|cost| {
                                cost * (count(u, "inputTokens") as f64
                                    + count(u, "outputTokens") as f64)
                                    / unpriced_tokens
                            })
                        }),
                ) {
                    out.push(r);
                }
            }
        }
        _ => {}
    }
    out
}

fn read_transcript(
    id: &str,
    path: &Path,
) -> Result<(Vec<UsageRecord>, usize, Vec<String>), String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let input: Box<dyn Read> = if path.extension().and_then(|s| s.to_str()) == Some("zst") {
        Box::new(zstd::stream::read::Decoder::new(file).map_err(|e| e.to_string())?)
    } else {
        Box::new(file)
    };
    let mut reader = BufReader::new(input);
    let mut state = ParseState {
        file: path.to_string_lossy().into_owned(),
        session: if id == "kimi" {
            path.ancestors()
                .find(|p| p.file_name().and_then(|n| n.to_str()) == Some("agents"))
                .and_then(Path::parent)
                .unwrap_or(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            path.to_string_lossy().into_owned()
        },
        ..Default::default()
    };
    let mut records: BTreeMap<String, UsageRecord> = BTreeMap::new();
    let mut skipped = 0;
    let mut line = 0;
    loop {
        let mut bytes = Vec::new();
        let n = reader
            .by_ref()
            .take(8 * 1024 * 1024)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        line += 1;
        if bytes.last() != Some(&b'\n') {
            if n == 8 * 1024 * 1024 {
                reader.skip_until(b'\n').map_err(|e| e.to_string())?;
                skipped += 1;
                continue;
            }
        }
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            if bytes.last() != Some(&b'\n') {
                break;
            }
            skipped += 1;
            continue;
        };
        let parsed = parse(id, &value, &mut state, line);
        if id == "codex" && string(&value["payload"], "type") == "token_count" && parsed.is_empty()
        {
            if let Some(at) = timestamp(&value["timestamp"]) {
                let old = identity(&[
                    id,
                    &format!("{}:{at}:{}", state.session, value["payload"]["info"]),
                ]);
                if !records.contains_key(&old) {
                    state.superseded.push(old);
                }
            }
        }
        for record in parsed {
            records
                .entry(record.id.clone())
                .and_modify(|previous| {
                    if id != "grok" {
                        merge_records(previous, &record);
                    }
                })
                .or_insert(record);
        }
    }
    for id in &state.superseded {
        records.remove(id);
    }
    Ok((records.into_values().collect(), skipped, state.superseded))
}

fn read_opencode(path: &Path) -> Result<(Vec<UsageRecord>, usize), String> {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
    let modern: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='session_message')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let table = if modern { "session_message" } else { "message" };
    let predicate = if modern {
        "type='assistant'"
    } else {
        "json_extract(data,'$.role')='assistant'"
    };
    let mut stmt = db.prepare(&format!("SELECT id,session_id,json_object('tokens',json_extract(data,'$.tokens'),'cost',json_extract(data,'$.cost'),'time',json_extract(data,'$.time'),'model',json_extract(data,'$.model'),'modelID',json_extract(data,'$.modelID'),'providerID',json_extract(data,'$.providerID')) FROM {table} WHERE json_valid(data) AND {predicate} ORDER BY rowid")).map_err(|e|format!("Unsupported OpenCode history: {e}"))?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    let mut skipped = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let key: String = row.get(0).map_err(|e| e.to_string())?;
        let session: String = row.get(1).map_err(|e| e.to_string())?;
        let raw: String = row.get(2).map_err(|e| e.to_string())?;
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            skipped += 1;
            continue;
        };
        let u = &v["tokens"];
        let usage = tokens(
            count(u, "input"),
            count(u, "output").saturating_add(count(u, "reasoning")),
            count(&u["cache"], "read"),
            count(&u["cache"], "write"),
            false,
        );
        let model = v["model"]["id"]
            .as_str()
            .or(v["model"]["modelID"].as_str())
            .unwrap_or(string(&v, "modelID"));
        let provider = v["model"]["providerID"]
            .as_str()
            .unwrap_or(string(&v, "providerID"));
        let free = matches!(provider, "opencode" | "opencode-go") && model.ends_with("-free");
        if let Some(r) = v["time"]["created"].as_i64().and_then(|at| {
            record(
                "opencode",
                &session,
                &key,
                model,
                at,
                usage,
                v["cost"]
                    .as_f64()
                    .filter(|n| *n > 0.0 || (free && *n == 0.0)),
            )
        }) {
            records.push(r);
        }
    }
    Ok((records, skipped))
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn write_lines(path: &Path, values: &[Value]) {
        fs::write(
            path,
            values.iter().map(|v| format!("{v}\n")).collect::<String>(),
        )
        .unwrap();
    }

    #[tokio::test]
    #[ignore = "reads local CLI accounting metadata into a temporary database"]
    async fn audit_real_cli_history() {
        let temp = tempfile::tempdir().unwrap();
        pricing::refresh(temp.path(), true).await.unwrap();
        let report = scan(temp.path()).unwrap();
        for source in report.sources {
            println!(
                "SOURCE {} files={} records={} skipped={} errors={:?}",
                source.id, source.files, source.records, source.skipped, source.errors
            );
        }
        pricing::reprice(&mut store::open(temp.path()).unwrap()).unwrap();
        let stats = UsageLedger::default()
            .stats(
                temp.path(),
                UsageStatsQuery {
                    range: "30d".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        for group in &stats.model_stats {
            println!(
                "MODEL {} tokens={} cost={}",
                group.label, group.total_tokens, group.cost_usd
            );
        }
        if let Ok(path) = std::env::var("KIVIO_USAGE_AUDIT_OUTPUT") {
            let db = store::open(temp.path()).unwrap();
            let records=db.prepare("SELECT * FROM records WHERE source='cli' AND operation='codex' ORDER BY created_at,id").unwrap().query_map([],store::read_record).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
            fs::write(path, serde_json::to_vec(&records).unwrap()).unwrap();
        }
        for group in stats.provider_stats {
            println!(
                "TOTAL {} sessions={} tokens={} cost={} unpriced={}",
                group.label,
                group.session_count,
                group.total_tokens,
                group.cost_usd,
                group.unpriced_requests
            );
        }
    }

    #[test]
    fn codex_reconciles_anonymous_requests_instead_of_losing_the_gap() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("history.jsonl");
        let snapshot = |time: &str, last: Value, total: Value| json!({"type":"event_msg","timestamp":time,"payload":{"type":"token_count","info":{"last_token_usage":last,"total_token_usage":total}}});
        let initial = json!({"input_tokens":10,"output_tokens":1});
        write_lines(
            &file,
            &[
                json!({"type":"session_meta","payload":{"id":"s"}}),
                json!({"type":"turn_context","payload":{"model":"custom"}}),
                snapshot("2026-11-14T00:00:00Z", initial.clone(), initial),
                json!({"type":"token_usage_record","timestamp":"2026-11-14T00:00:01Z","payload":{"usage":{"input_tokens":20,"output_tokens":2}}}),
                snapshot(
                    "2026-11-14T00:00:02Z",
                    json!({"input_tokens":5,"output_tokens":1}),
                    json!({"input_tokens":30,"output_tokens":3}),
                ),
            ],
        );
        let (rows, _, _) = read_transcript("codex", &file).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter_map(|r| r.total_tokens).sum::<u64>(), 33);
    }
    #[test]
    fn codex_rejects_snapshot_not_explained_by_cumulative_counts() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("history.jsonl");
        let initial = json!({"input_tokens":100,"output_tokens":0});
        let first = json!({"type":"event_msg","timestamp":"2026-11-14T00:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":initial,"total_token_usage":initial}}});
        let mut invalid = first.clone();
        invalid["timestamp"] = json!("2026-11-14T00:00:01Z");
        invalid["payload"]["info"]["total_token_usage"]["input_tokens"] = json!(250);
        write_lines(
            &file,
            &[
                json!({"type":"session_meta","payload":{"id":"s"}}),
                json!({"type":"turn_context","payload":{"model":"custom"}}),
                first,
                invalid,
            ],
        );
        let (rows, _, obsolete) = read_transcript("codex", &file).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, Some(100));
        assert_eq!(obsolete.len(), 1);
    }

    #[test]
    fn compressed_codex_and_plain_history_match() {
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("history.jsonl");
        let compressed = temp.path().join("history.jsonl.zst");
        write_lines(
            &plain,
            &[
                json!({"type":"session_meta","payload":{"id":"s"}}),
                json!({"type":"turn_context","payload":{"model":"custom"}}),
                json!({"type":"event_msg","timestamp":"2026-11-14T00:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":5}}}}),
            ],
        );
        fs::write(
            &compressed,
            zstd::stream::encode_all(fs::File::open(&plain).unwrap(), 1).unwrap(),
        )
        .unwrap();
        let (a, _, _) = read_transcript("codex", &plain).unwrap();
        let (b, _, _) = read_transcript("codex", &compressed).unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, b[0].id);
        assert_eq!(a[0].total_tokens, b[0].total_tokens);
    }
    #[test]
    fn grok_keeps_first_completion_even_when_later_one_is_larger() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("updates.jsonl");
        let first = json!({"params":{"sessionId":"s","_meta":{"agentTimestampMs":1794614400000i64},"update":{"sessionUpdate":"turn_completed","prompt_id":"p","usage":{"inputTokens":100,"outputTokens":5,"costUsdTicks":2500000000i64}}}});
        let mut next = first.clone();
        next["params"]["update"]["usage"]["inputTokens"] = json!(1000);
        next["params"]["update"]["usage"]["costUsdTicks"] = json!(5000000000i64);
        write_lines(&file, &[first, next]);
        let (rows, _, _) = read_transcript("grok", &file).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, Some(105));
        assert_eq!(rows[0].cost_usd, Some(0.25));
    }
    #[test]
    fn opencode_v2_counts_reasoning_and_free_gateway() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("opencode.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,data TEXT)",
        )
        .unwrap();
        let v = json!({"model":{"id":"test-free","providerID":"opencode"},"time":{"created":1794614400000i64},"tokens":{"input":10,"output":5,"reasoning":3,"cache":{"read":80,"write":10}},"cost":0});
        db.execute(
            "INSERT INTO session_message VALUES('m','s','assistant',?1)",
            [v.to_string()],
        )
        .unwrap();
        let (rows, _) = read_opencode(&path).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, Some(108));
        assert_eq!(rows[0].cost_usd, Some(0.0));
    }

    #[test]
    fn late_codex_ledger_replaces_persisted_snapshot_and_blocks_stale_copies() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        let meta = json!({"type":"session_meta","payload":{"id":"s"}});
        let turn = json!({"type":"turn_context","payload":{"model":"custom"}});
        let u = json!({"input_tokens":100,"output_tokens":5});
        let snapshot = json!({"type":"event_msg","timestamp":"2026-11-14T00:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":u,"total_token_usage":u}}});
        write_lines(&file, &[meta.clone(), turn.clone(), snapshot.clone()]);
        let (old, _, obsolete) = read_transcript("codex", &file).unwrap();
        let mut db = store::open(temp.path()).unwrap();
        store::save_cli_file(&mut db, "session", "1", &old, &obsolete).unwrap();
        let ledger = json!({"type":"token_usage_record","timestamp":"2026-11-14T00:00:01Z","payload":{"response_id":"r","usage":u}});
        write_lines(&file, &[meta, turn, snapshot, ledger]);
        let (rows, _, obsolete) = read_transcript("codex", &file).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(obsolete, vec![old[0].id.clone()]);
        store::save_cli_file(&mut db, "session", "2", &rows, &obsolete).unwrap();
        store::save_cli_file(&mut db, "stale-copy", "1", &old, &[]).unwrap();
        let stats = UsageLedger::default()
            .stats(
                temp.path(),
                UsageStatsQuery {
                    range: "all".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(stats.total_logs, 1);
        assert_eq!(stats.summary.total_tokens, 105);
    }

    #[test]
    fn kimi_copies_and_line_shifts_preserve_identity() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp
            .path()
            .join("home-a/sessions/session-id/agents/main/wire.jsonl");
        let b = temp
            .path()
            .join("home-b/sessions/session-id/agents/main/wire.jsonl");
        fs::create_dir_all(a.parent().unwrap()).unwrap();
        fs::create_dir_all(b.parent().unwrap()).unwrap();
        let event = json!({"type":"usage.record","usageScope":"turn","time":"2026-11-14T00:00:00Z","model":"custom","usage":{"inputOther":10,"output":5}});
        write_lines(&a, &[event.clone()]);
        write_lines(&b, &[json!({"type":"other"}), event]);
        let (first, _, _) = read_transcript("kimi", &a).unwrap();
        let (second, _, _) = read_transcript("kimi", &b).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].id, second[0].id);
        assert_eq!(first[0].conversation_id, second[0].conversation_id);
    }

    #[test]
    fn claude_counts_repeated_blocks_once_and_preserves_cache() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        let mut value = json!({"type":"assistant","timestamp":"2026-11-14T00:00:00Z","sessionId":"s","requestId":"r","message":{"id":"m","model":"claude-test","usage":{"input_tokens":10,"output_tokens":1,"cache_read_input_tokens":80,"cache_creation_input_tokens":10}}});
        let first = value.clone();
        value["message"]["usage"]["output_tokens"] = json!(5);
        write_lines(&file, &[first.clone(), value, first]);
        let (rows, skipped, _) = read_transcript("claude", &file).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(skipped, 0);
        assert_eq!(rows[0].input_tokens, Some(100));
        assert_eq!(rows[0].output_tokens, Some(5));
        assert_eq!(rows[0].total_tokens, Some(105));
        assert!(rows[0].cost_usd.is_none());
    }

    #[test]
    fn codex_request_ledger_and_snapshots_do_not_double_bill() {
        let mut state = ParseState::default();
        parse(
            "codex",
            &json!({"type":"session_meta","payload":{"id":"s"}}),
            &mut state,
            1,
        );
        parse(
            "codex",
            &json!({"type":"turn_context","payload":{"model":"unknown-model"}}),
            &mut state,
            2,
        );
        let u = json!({"input_tokens":100,"cached_input_tokens":80,"output_tokens":5,"total_tokens":105});
        let request = json!({"type":"token_usage_record","timestamp":"2026-11-14T00:00:00Z","payload":{"response_id":"response","usage":u}});
        let rows = parse("codex", &request, &mut state, 3);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_tokens, Some(100));
        assert_eq!(rows[0].total_tokens, Some(105));
        let snapshot = json!({"type":"event_msg","timestamp":"2026-11-14T00:00:01Z","payload":{"type":"token_count","info":{"last_token_usage":u,"total_token_usage":u}}});
        assert!(parse("codex", &snapshot, &mut state, 4).is_empty());
        let mut legacy = ParseState {
            model: "unknown-model".into(),
            session: "s".into(),
            ..Default::default()
        };
        assert_eq!(parse("codex", &snapshot, &mut legacy, 1).len(), 1);
        assert!(parse("codex", &snapshot, &mut legacy, 2).is_empty());
    }

    #[test]
    fn pi_and_grok_reported_costs_keep_their_units() {
        let mut state = ParseState {
            session: "s".into(),
            ..Default::default()
        };
        let pi = json!({"type":"message","id":"m","timestamp":"2026-11-14T00:00:00Z","message":{"role":"assistant","model":"custom","usage":{"input":10,"output":5,"cacheRead":80,"cacheWrite":10,"cost":{"total":0.25}}}});
        let rows = parse("pi", &pi, &mut state, 1);
        assert_eq!(rows[0].total_tokens, Some(105));
        assert_eq!(rows[0].cost_usd, Some(0.25));
        let grok = json!({"params":{"sessionId":"s","_meta":{"agentTimestampMs":1794614400000i64},"update":{"sessionUpdate":"turn_completed","prompt_id":"p","usage":{"inputTokens":100,"outputTokens":5,"cachedReadTokens":80,"costUsdTicks":2500000000i64}}}});
        let rows = parse("grok", &grok, &mut state, 1);
        assert_eq!(rows[0].total_tokens, Some(105));
        assert_eq!(rows[0].cost_usd, Some(0.25));
    }

    #[test]
    fn synchronization_is_idempotent_updates_usage_and_respects_clear() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let mut db = store::open(dir).unwrap();
        let mut r = record(
            "pi",
            "s",
            "m",
            "custom",
            Local::now().timestamp_millis() - 10000,
            tokens(10, 5, 0, 0, true),
            Some(0.25),
        )
        .unwrap();
        store::save_cli_file(&mut db, "history", "1", &[r.clone()], &[]).unwrap();
        store::save_cli_file(&mut db, "copy", "1", &[r.clone()], &[]).unwrap();
        assert!(store::cli_file_unchanged(&db, "history", "1").unwrap());
        let query = UsageStatsQuery {
            range: "all".into(),
            ..Default::default()
        };
        let stats = UsageLedger::default().stats(dir, query.clone()).unwrap();
        assert_eq!(stats.total_logs, 1);
        assert_eq!(stats.summary.session_count, 1);
        assert_eq!(stats.summary.total_cost_usd, 0.25);
        assert_eq!(stats.summary.average_duration_ms, None);
        r.output_tokens = Some(10);
        r.total_tokens = Some(20);
        r.cost_usd = Some(0.5);
        store::save_cli_file(&mut db, "history", "2", &[r.clone()], &[]).unwrap();
        assert_eq!(
            UsageLedger::default()
                .stats(dir, query.clone())
                .unwrap()
                .summary
                .total_cost_usd,
            0.5
        );
        UsageLedger::default().clear(dir).unwrap();
        store::save_cli_file(&mut db, "history", "3", &[r], &[]).unwrap();
        assert_eq!(
            UsageLedger::default().stats(dir, query).unwrap().total_logs,
            0
        );
    }

    #[test]
    fn corrupt_and_unfinished_lines_do_not_block_valid_records() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("session.jsonl");
        fs::write(&file, b"invalid\n{\"type\":\"message\"").unwrap();
        let (rows, skipped, _) = read_transcript("pi", &file).unwrap();
        assert!(rows.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn opencode_reads_database_without_modifying_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("opencode.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE message (id TEXT,session_id TEXT,data TEXT)")
            .unwrap();
        let value = json!({"role":"assistant","modelID":"custom","time":{"created":1794614400000i64},"tokens":{"input":10,"output":5,"cache":{"read":80,"write":10}},"cost":0.25});
        db.execute(
            "INSERT INTO message VALUES ('m','s',?1)",
            [value.to_string()],
        )
        .unwrap();
        let (rows, skipped) = read_opencode(&path).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_tokens, Some(105));
        assert_eq!(rows[0].cost_usd, Some(0.25));
    }
    #[test]
    fn partial_usage_channels_merge_without_losing_uncached_input() {
        let mut previous = record(
            "claude",
            "s",
            "m",
            "custom",
            1794614400000,
            tokens(10, 0, 80, 0, false),
            None,
        )
        .unwrap();
        let next = record(
            "claude",
            "s",
            "m",
            "custom",
            1794614400000,
            tokens(0, 5, 0, 10, false),
            None,
        )
        .unwrap();
        merge_records(&mut previous, &next);
        assert_eq!(previous.input_tokens, Some(100));
        assert_eq!(previous.output_tokens, Some(5));
        assert_eq!(previous.total_tokens, Some(105));
    }
    #[test]
    fn advisor_usage_is_attributed_to_its_model_and_grok_keeps_total_cost() {
        let mut state = ParseState::default();
        let claude = json!({"type":"assistant","timestamp":"2026-11-14T00:00:00Z","sessionId":"s","requestId":"r","message":{"id":"m","model":"executor","usage":{"input_tokens":10,"output_tokens":1,"iterations":[{"type":"advisor_message","model":"advisor","input_tokens":20,"output_tokens":2}]}}});
        let rows = parse("claude", &claude, &mut state, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].model, "advisor");
        assert_eq!(rows[1].total_tokens, Some(22));
        let grok = json!({"params":{"sessionId":"s","_meta":{"agentTimestampMs":1794614400000i64},"update":{"sessionUpdate":"turn_completed","prompt_id":"p","usage":{"costUsdTicks":10000000000i64,"modelUsage":{"a":{"inputTokens":10,"outputTokens":0},"b":{"inputTokens":30,"outputTokens":0}}}}}});
        let rows = parse("grok", &grok, &mut state, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().filter_map(|r| r.cost_usd).sum::<f64>(), 1.0);
    }
}
