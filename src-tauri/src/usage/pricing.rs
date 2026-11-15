use super::*;
use serde_json::Value;
use std::sync::{OnceLock, RwLock};

const URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const API_URL: &str = "https://api.github.com/repos/BerriAI/litellm/contents/model_prices_and_context_window.json?ref=main";
type Rates = BTreeMap<String, Value>;

#[derive(Default)]
struct CachedRates {
    dir: PathBuf,
    fetched_at: i64,
    table: Rates,
}

pub(super) fn model_id<'a>(provider: &'a ModelProvider, model: &'a str) -> &'a str {
    provider
        .model_overrides
        .get(model)
        .and_then(|info| info.pricing_model.as_deref())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(model)
}
fn rates() -> &'static RwLock<CachedRates> {
    static RATES: OnceLock<RwLock<CachedRates>> = OnceLock::new();
    RATES.get_or_init(Default::default)
}

fn install_rates(dir: &Path, fetched_at: i64, table: Rates) -> Result<(), String> {
    let mut cached = rates().write().map_err(|_| "Price lock poisoned")?;
    // A delayed disk read must not replace a newer download for the same ledger.
    if cached.dir != dir || fetched_at >= cached.fetched_at {
        *cached = CachedRates {
            dir: dir.to_path_buf(),
            fetched_at,
            table,
        };
    }
    Ok(())
}
fn number(v: &Value, key: &str) -> Option<f64> {
    v[key].as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}
fn parse(document: &Value) -> Result<Rates, String> {
    let mut table: Rates = document
        .as_object()
        .ok_or("Invalid LiteLLM price table")?
        .iter()
        .filter(|(_, v)| {
            number(v, "input_cost_per_token").is_some()
                && number(v, "output_cost_per_token").is_some()
        })
        .map(|(k, v)| (k.trim().to_lowercase(), v.clone()))
        .collect();
    if table.is_empty() {
        return Err("Empty LiteLLM price table".into());
    }
    let mut aliases: BTreeMap<String, (bool, Option<Value>)> = BTreeMap::new();
    for (key, v) in &table {
        let (prefix, bare) = key
            .split_once('/')
            .map(|(p, _)| (p, key.rsplit('/').next().unwrap()))
            .unwrap_or(("", key.strip_prefix("anthropic.").unwrap_or(key)));
        if bare == key || table.contains_key(bare) {
            continue;
        }
        let first = matches!(
            prefix,
            "openai" | "anthropic" | "gemini" | "deepseek" | "meta" | "mistral" | "xai"
        );
        match aliases.get_mut(bare) {
            None => {
                aliases.insert(bare.into(), (first, Some(v.clone())));
            }
            Some((held, value)) if first && !*held => {
                *held = true;
                *value = Some(v.clone());
            }
            Some((held, value)) if first == *held => {
                if value.as_ref().is_some_and(|old| !same_pricing(old, v)) {
                    *value = None;
                }
            }
            _ => {}
        }
    }
    for (name, (_, v)) in aliases {
        if let Some(v) = v {
            table.insert(name, v);
        }
    }
    Ok(table)
}

// Per-token rates: ordinary input, output, cache read, short write, long write.
fn token_rates(v: &Value, suffix: &str, base: Option<[f64; 5]>) -> Option<[f64; 5]> {
    let input = number(v, &format!("input_cost_per_token{suffix}"))?;
    let output = number(v, &format!("output_cost_per_token{suffix}")).or(base.map(|b| b[1]))?;
    let cache = |key: &str, index: usize, fallback: f64| {
        number(v, &format!("{key}{suffix}")).unwrap_or_else(|| {
            base.filter(|b| b[0] > 0.0)
                .map(|b| b[index] / b[0] * input)
                .unwrap_or(fallback)
        })
    };
    let write = cache("cache_creation_input_token_cost", 3, input);
    Some([
        input,
        output,
        cache("cache_read_input_token_cost", 2, input),
        write,
        cache("cache_creation_input_token_cost_above_1hr", 4, write),
    ])
}
fn estimate_with(
    table: &Rates,
    model: &str,
    usage: &ModelUsage,
    format: &str,
    speed: Option<&str>,
) -> Option<f64> {
    if [
        usage.input_tokens,
        usage.output_tokens,
        usage.cached_input_tokens,
        usage.cache_creation_input_tokens,
    ]
    .iter()
    .all(Option::is_none)
    {
        return None;
    }
    let key = model.trim().split('[').next()?.to_lowercase();
    let v = table.get(&key).or_else(|| {
        static ALIASES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
        let aliases = ALIASES.get_or_init(|| {
            serde_json::from_str(include_str!("model_aliases.json"))
                .expect("Bundled pricing aliases must be valid JSON")
        });
        table.get(aliases.get(&key)?)
    })?;
    let input = usage.input_tokens.unwrap_or(0);
    let read = usage.cached_input_tokens.unwrap_or(0);
    let write = usage.cache_creation_input_tokens.unwrap_or(0);
    let ordinary = if format == "anthropic_messages" {
        input
    } else {
        input.saturating_sub(read).saturating_sub(write)
    };
    let prompt = ordinary.saturating_add(read).saturating_add(write);
    let selected = rates_for(v, prompt, speed)?;
    let long = usage
        .cache_creation_input_tokens_long
        .unwrap_or(0)
        .min(write);
    let result = ordinary as f64 * selected[0]
        + usage.output_tokens.unwrap_or(0) as f64 * selected[1]
        + read as f64 * selected[2]
        + (write - long) as f64 * selected[3]
        + long as f64 * selected[4];
    result.is_finite().then_some(result)
}

fn context_thresholds(v: &Value) -> impl Iterator<Item = u64> + '_ {
    v.as_object()
        .into_iter()
        .flat_map(|object| object.keys())
        .filter_map(|key| {
            let (threshold, suffix) = key
                .strip_prefix("input_cost_per_token_above_")?
                .split_once("k_tokens")?;
            if !matches!(suffix, "" | "_priority" | "_ultrafast") {
                return None;
            }
            threshold.parse::<u64>().ok()?.checked_mul(1000)
        })
}

fn same_pricing(a: &Value, b: &Value) -> bool {
    std::iter::once(0)
        .chain(
            context_thresholds(a)
                .chain(context_thresholds(b))
                .map(|n| n.saturating_add(1)),
        )
        .all(|prompt| {
            [None, Some("fast"), Some("ultrafast")]
                .into_iter()
                .all(|speed| rates_for(a, prompt, speed) == rates_for(b, prompt, speed))
        })
}

fn rates_for(v: &Value, prompt: u64, speed: Option<&str>) -> Option<[f64; 5]> {
    let base = token_rates(v, "", None)?;
    let threshold = context_thresholds(v).filter(|n| prompt > *n).max();
    let suffix = threshold
        .map(|n| format!("_above_{}k_tokens", n / 1000))
        .unwrap_or_default();
    let standard = token_rates(v, &suffix, Some(base)).unwrap_or(base);
    let speed_suffix = match speed {
        Some("fast" | "priority") => "_priority",
        Some("ultrafast") => "_ultrafast",
        _ => "",
    };
    Some(if speed_suffix.is_empty() {
        standard
    } else if speed_suffix == "_priority" && number(&v["provider_specific_entry"], "fast").is_some()
    {
        let multiple = number(&v["provider_specific_entry"], "fast")?;
        standard.map(|n| n * multiple)
    } else {
        token_rates(v, &format!("{suffix}{speed_suffix}"), Some(standard))
            .or_else(|| token_rates(v, speed_suffix, Some(base)))
            .unwrap_or(standard)
    })
}
pub(super) fn estimate(
    model: &str,
    usage: &ModelUsage,
    format: &str,
    speed: Option<&str>,
) -> Option<f64> {
    estimate_with(&rates().read().ok()?.table, model, usage, format, speed)
}
pub(super) fn ensure_loaded(dir: &Path) {
    if rates()
        .read()
        .is_ok_and(|cached| cached.table.is_empty() || cached.dir != dir)
    {
        let _ = load(dir);
    }
}
pub(super) fn load(dir: &Path) -> Result<i64, String> {
    let db = store::open(dir)?;
    let saved: Option<(i64, String)> = db
        .query_row(
            "SELECT fetched_at,document FROM price_cache WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((at, raw)) = saved {
        let table = parse(&serde_json::from_str(&raw).map_err(|e| format!("LiteLLM cache: {e}"))?)?;
        install_rates(dir, at, table)?;
        Ok(at)
    } else {
        install_rates(dir, 0, Rates::new())?;
        Ok(0)
    }
}
use rusqlite::OptionalExtension;
pub(super) async fn refresh(dir: &Path, force: bool) -> Result<i64, String> {
    refresh_with_urls(dir, force, &[URL, API_URL]).await
}

async fn refresh_with_urls(dir: &Path, force: bool, urls: &[&str]) -> Result<i64, String> {
    static LOCK: tokio::sync::Mutex<BTreeMap<PathBuf, std::time::Instant>> =
        tokio::sync::Mutex::const_new(BTreeMap::new());
    let mut attempts = LOCK.lock().await;
    let fetched = load(dir)?;
    let now = Local::now().timestamp();
    if now - fetched < if force { 60 } else { 86400 }
        || attempts
            .get(dir)
            .is_some_and(|at| at.elapsed().as_secs() < 60)
    {
        return Ok(fetched);
    }
    attempts.insert(dir.to_path_buf(), std::time::Instant::now());
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut error = String::new();
    for url in urls {
        match download(&client, url).await.and_then(|document| {
            let table = parse(&document)?;
            Ok((document, table))
        }) {
            Ok((document, table)) => {
                let db = store::open(dir)?;
                db.execute("INSERT INTO price_cache VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET fetched_at=excluded.fetched_at,document=excluded.document",rusqlite::params![Local::now().timestamp(),document.to_string()]).map_err(|e|e.to_string())?;
                install_rates(dir, now, table)?;
                return Ok(now);
            }
            Err(e) => error = e,
        }
    }
    Err(format!(
        "LiteLLM prices could not refresh; keeping cached prices: {error}"
    ))
}
async fn download(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let mut response = client
        .get(url)
        .header("User-Agent", "Kivio")
        .header("Accept", "application/vnd.github.raw+json")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
            return Err("LiteLLM table exceeds 16 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub(super) fn reprice(
    db: &mut rusqlite::Connection,
    providers: &[ModelProvider],
    only_missing: bool,
) -> Result<(), String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let rows = tx
        .prepare("SELECT * FROM records WHERE cost_source NOT IN ('user_override','cli_reported') AND (?1=0 OR cost_usd IS NULL)")
        .map_err(|e| e.to_string())?
        .query_map([only_missing], store::read_record)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for mut r in rows {
        r.cost_usd = None;
        r.cost_source = "unavailable".into();
        if let Some(provider) = providers
            .iter()
            .find(|provider| provider.id == r.provider_id)
        {
            apply_cost(provider, &mut r);
        } else if let Some(cost) = estimate(
            &r.model,
            &ModelUsage {
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
                cached_input_tokens: r.cached_input_tokens,
                cache_creation_input_tokens: r.cache_creation_input_tokens,
                cache_creation_input_tokens_long: r.cache_creation_input_tokens_long,
                ..Default::default()
            },
            &r.api_format,
            r.service_tier.as_deref(),
        ) {
            r.cost_usd = Some(cost);
            r.cost_source = "litellm".into();
        }
        tx.execute(
            "UPDATE records SET cost_usd=?1,cost_source=?2 WHERE id=?3",
            rusqlite::params![r.cost_usd, r.cost_source, r.id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_disk_load_does_not_replace_newer_prices() {
        let temp = tempfile::tempdir().unwrap();
        let old = serde_json::json!({"stale-test":{"input_cost_per_token":0.000001,"output_cost_per_token":0.000002}});
        let new = serde_json::json!({"stale-test":{"input_cost_per_token":0.000004,"output_cost_per_token":0.000008}});
        store::open(temp.path())
            .unwrap()
            .execute(
                "INSERT INTO price_cache VALUES(1,123,?1)",
                [old.to_string()],
            )
            .unwrap();
        install_rates(temp.path(), 456, parse(&new).unwrap()).unwrap();
        load(temp.path()).unwrap();
        let cost = estimate(
            "stale-test",
            &ModelUsage {
                input_tokens: Some(1000),
                output_tokens: Some(100),
                ..Default::default()
            },
            "openai_chat",
            None,
        )
        .unwrap();
        assert!((cost - 0.0048).abs() < 1e-12);
        let empty = tempfile::tempdir().unwrap();
        load(empty.path()).unwrap();
        assert!(estimate(
            "stale-test",
            &ModelUsage {
                input_tokens: Some(1000),
                ..Default::default()
            },
            "openai_chat",
            None
        )
        .is_none());
    }

    fn unpriced_record(id: &str, model: &str) -> UsageRecord {
        serde_json::from_value(serde_json::json!({
            "id":id,"createdAt":1,"completedAt":1,"durationMs":0,
            "source":"chat","operation":"test","providerId":"p","providerName":"p",
            "model":model,"apiFormat":"openai_chat","status":"success",
            "usageSource":"provider_reported","inputTokens":1000,"outputTokens":100,
            "costSource":"unavailable"
        }))
        .unwrap()
    }

    #[test]
    fn reference_models_and_provider_prices_match_live_calls_and_history() {
        let temp = tempfile::tempdir().unwrap();
        let mut db = store::open(temp.path()).unwrap();
        let document = serde_json::json!({"reference-model":{
            "input_cost_per_token":0.000001,"output_cost_per_token":0.000002
        }});
        db.execute(
            "INSERT INTO price_cache VALUES(1,123,?1)",
            [document.to_string()],
        )
        .unwrap();
        load(temp.path()).unwrap();
        let provider: ModelProvider = serde_json::from_value(serde_json::json!({
            "id":"p","name":"p","baseUrl":"https://example.test","apiFormat":"openai_chat",
            "modelOverrides":{
                "gateway":{"pricingModel":"reference-model"},
                "custom":{"pricing":{"input":2.0,"output":8.0,"cachedInput":0.2}},
                "partial":{"pricing":{"input":2.0}},
                "invalid":{"pricing":{"input":-1.0,"output":8.0}},
                "free":{"pricing":{"input":0.0,"output":0.0}}
            }
        }))
        .unwrap();
        for model in ["gateway", "custom", "partial", "invalid", "free"] {
            UsageLedger::default()
                .append(temp.path(), &unpriced_record(model, model))
                .unwrap();
        }
        let mut anthropic = unpriced_record("anthropic", "custom");
        anthropic.api_format = "anthropic_messages".into();
        anthropic.cached_input_tokens = Some(900);
        UsageLedger::default()
            .append(temp.path(), &anthropic)
            .unwrap();
        let mut priced = unpriced_record("priced", "gateway");
        priced.cost_usd = Some(99.0);
        priced.cost_source = "litellm".into();
        UsageLedger::default().append(temp.path(), &priced).unwrap();
        reprice(&mut db, std::slice::from_ref(&provider), true).unwrap();
        let preserved: f64 = db
            .query_row(
                "SELECT cost_usd FROM records WHERE id='priced'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(preserved, 99.0);
        for (id, expected) in [
            ("gateway", 0.0012),
            ("custom", 0.0028),
            ("anthropic", 0.00298),
            ("free", 0.0),
        ] {
            let actual: f64 = db
                .query_row("SELECT cost_usd FROM records WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .unwrap();
            assert!((actual - expected).abs() < 1e-12, "{id}: {actual}");
        }
        for id in ["partial", "invalid"] {
            let actual: Option<f64> = db
                .query_row("SELECT cost_usd FROM records WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(actual, None);
        }
        reprice(&mut db, std::slice::from_ref(&provider), false).unwrap();
        let repriced: f64 = db
            .query_row(
                "SELECT cost_usd FROM records WHERE id='priced'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((repriced - 0.0012).abs() < 1e-12);
        let state =
            AppState::new_headless(crate::settings::Settings::default(), temp.path().into());
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        state.usage_ledger.price_refresh.set(sender).unwrap();
        for (model, message_id) in [("gateway", "new"), ("missing", "missing")] {
            record_model_call(
                &state,
                UsageRecordInput {
                    provider: &provider,
                    model,
                    source: "chat",
                    operation: "test",
                    status: "success",
                    status_code: Some(200),
                    usage: Some(ModelUsage {
                        input_tokens: Some(1000),
                        output_tokens: Some(100),
                        ..Default::default()
                    }),
                    usage_source: "provider_reported",
                    started_at: 1,
                    duration_ms: 0,
                    first_token_ms: None,
                    reasoning_effort: None,
                    conversation_id: None,
                    message_id: Some(message_id.into()),
                    error_kind: None,
                },
            );
        }
        let live: f64 = db
            .query_row(
                "SELECT cost_usd FROM records WHERE message_id='new'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((live - 0.0012).abs() < 1e-12);
        assert_eq!(receiver.try_recv().unwrap(), "missing");
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn refresh_falls_back_from_invalid_prices_and_keeps_cache_during_failure_backoff() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let document = serde_json::json!({"refresh-test":{"input_cost_per_token":0.000001,"output_cost_per_token":0.000002}});
        let body = document.to_string();
        let server = tokio::spawn(async move {
            for body in ["{}".to_owned(), body] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                assert!(stream.read(&mut buffer).await.unwrap() > 0);
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let temp = tempfile::tempdir().unwrap();
        let fetched = refresh_with_urls(temp.path(), true, &[&url, &url])
            .await
            .unwrap();
        server.await.unwrap();
        assert!(fetched > 0);
        assert_eq!(
            refresh_with_urls(temp.path(), true, &["http://127.0.0.1:0"])
                .await
                .unwrap(),
            fetched
        );
        let failed = tempfile::tempdir().unwrap();
        store::open(failed.path())
            .unwrap()
            .execute(
                "INSERT INTO price_cache VALUES(1,123,?1)",
                [document.to_string()],
            )
            .unwrap();
        assert!(
            refresh_with_urls(failed.path(), true, &["http://127.0.0.1:0"])
                .await
                .is_err()
        );
        assert_eq!(
            refresh_with_urls(failed.path(), true, &["http://127.0.0.1:0"])
                .await
                .unwrap(),
            123
        );
        assert!(estimate(
            "refresh-test",
            &ModelUsage {
                input_tokens: Some(100),
                ..Default::default()
            },
            "openai_chat",
            None
        )
        .is_some());
    }
    #[test]
    fn conflicting_speed_and_context_prices_do_not_create_bare_aliases() {
        for extra in [
            serde_json::json!({"input_cost_per_token_priority": 3.0}),
            serde_json::json!({"input_cost_per_token_above_1k_tokens": 3.0}),
            serde_json::json!({"provider_specific_entry": {"fast": 3.0}}),
        ] {
            let first = serde_json::json!({"input_cost_per_token":1.0,"output_cost_per_token":2.0});
            let mut second = first.clone();
            second
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let table = parse(&serde_json::json!({"vendor-a/model":first,"vendor-b/model":second}))
                .unwrap();
            assert!(!table.contains_key("model"), "{extra}");
            assert!(table.contains_key("vendor-a/model"));
            assert!(table.contains_key("vendor-b/model"));
        }
    }

    #[test]
    fn missing_token_usage_is_not_priced_as_free() {
        let table = parse(
            &serde_json::json!({"model":{"input_cost_per_token":1.0,"output_cost_per_token":2.0}}),
        )
        .unwrap();
        assert_eq!(
            estimate_with(&table, "model", &ModelUsage::default(), "openai_chat", None),
            None
        );
    }

    #[test]
    fn cached_prices_reprice_history_without_overwriting_reported_or_custom_costs() {
        let temp = tempfile::tempdir().unwrap();
        let mut db = store::open(temp.path()).unwrap();
        let row:UsageRecord=serde_json::from_value(serde_json::json!({"id":"auto","createdAt":1,"completedAt":1,"durationMs":0,"source":"chat","operation":"chat_tools_planning","providerId":"p","providerName":"p","model":"deepseek-v4.1-flash","apiFormat":"openai_chat","status":"success","usageSource":"provider_reported","inputTokens":1000,"outputTokens":100,"costSource":"unavailable"})).unwrap();
        UsageLedger::default().append(temp.path(), &row).unwrap();
        for (id, source) in [("reported", "cli_reported"), ("custom", "user_override")] {
            let mut r = row.clone();
            r.id = id.into();
            r.cost_source = source.into();
            r.cost_usd = Some(99.0);
            UsageLedger::default().append(temp.path(), &r).unwrap();
        }
        for price in [0.00001, 0.00002] {
            let document = serde_json::json!({"deepseek/deepseek-flash":{"input_cost_per_token":price,"output_cost_per_token":price}});
            db.execute(
                "INSERT OR REPLACE INTO price_cache VALUES(1,123,?1)",
                [document.to_string()],
            )
            .unwrap();
            assert_eq!(load(temp.path()).unwrap(), 123);
            reprice(&mut db, &[], false).unwrap();
            let auto: f64 = db
                .query_row("SELECT cost_usd FROM records WHERE id='auto'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert!((auto - 1100.0 * price).abs() < 1e-10);
            let count:i64=db.query_row("SELECT count(*) FROM records WHERE id IN ('reported','custom') AND cost_usd=99",[],|r|r.get(0)).unwrap();
            assert_eq!(count, 2);
        }
    }

    #[test]
    fn deepseek_flash_price_alias_preserves_cache_and_exact_routes() {
        let mut table = parse(&serde_json::json!({
            "deepseek/deepseek-flash": {
                "input_cost_per_token": 0.0000003,
                "output_cost_per_token": 0.0000012,
                "cache_read_input_token_cost": 0.000000006
            },
            "vendor-a/deepseek-v4.1-flash": {
                "input_cost_per_token": 0.00001, "output_cost_per_token": 0.00002
            },
            "vendor-b/deepseek-v4.1-flash": {
                "input_cost_per_token": 0.00002, "output_cost_per_token": 0.00004
            }
        }))
        .unwrap();
        let usage = ModelUsage {
            input_tokens: Some(1000),
            output_tokens: Some(100),
            cached_input_tokens: Some(900),
            ..Default::default()
        };
        for model in ["deepseek-v4.1-flash", "deepseek/deepseek-v4.1-flash"] {
            let cost = estimate_with(&table, model, &usage, "openai_chat", None).unwrap();
            assert!((cost - 0.0001554).abs() < 1e-12, "{model}: {cost}");
        }
        for model in [
            "qwen3.8-flash-next",
            "other/deepseek-v4.1-flash",
            "deepseek-v4.1-flash-unknown",
        ] {
            assert_eq!(
                estimate_with(&table, model, &usage, "openai_chat", None),
                None
            );
        }
        table.insert(
            "deepseek-v4.1-flash".into(),
            table["vendor-a/deepseek-v4.1-flash"].clone(),
        );
        let cost =
            estimate_with(&table, "deepseek-v4.1-flash", &usage, "openai_chat", None).unwrap();
        assert!((cost - 0.012).abs() < 1e-12);
    }

    #[test]
    fn prices_cache_tiers_speed_and_missing_models() {
        let table=parse(&serde_json::json!({"model":{"input_cost_per_token":0.00001,"output_cost_per_token":0.00002,"cache_read_input_token_cost":0.000001,"input_cost_per_token_above_1k_tokens":0.00002,"output_cost_per_token_above_1k_tokens":0.00003,"input_cost_per_token_priority":0.00003,"output_cost_per_token_priority":0.00006}})).unwrap();
        let u = ModelUsage {
            input_tokens: Some(1000),
            output_tokens: Some(100),
            cached_input_tokens: Some(900),
            ..Default::default()
        };
        assert!((estimate_with(&table, "model", &u, "cli", None).unwrap() - 0.0039).abs() < 1e-10);
        assert!(
            (estimate_with(&table, "model", &u, "cli", Some("fast")).unwrap() - 0.0117).abs()
                < 1e-10
        );
        let u = ModelUsage {
            input_tokens: Some(2000),
            ..u
        };
        assert!((estimate_with(&table, "model", &u, "cli", None).unwrap() - 0.0268).abs() < 1e-10);
        assert_eq!(estimate_with(&table, "unknown", &u, "cli", None), None);
        assert!(parse(&serde_json::json!({})).is_err());
    }
}
