use super::*;
use serde_json::Value;
use std::sync::{OnceLock, RwLock};

const URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const API_URL: &str = "https://api.github.com/repos/BerriAI/litellm/contents/model_prices_and_context_window.json?ref=main";
type Rates = BTreeMap<String, Value>;
fn rates() -> &'static RwLock<Rates> {
    static RATES: OnceLock<RwLock<Rates>> = OnceLock::new();
    RATES.get_or_init(Default::default)
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
                if value
                    .as_ref()
                    .is_some_and(|old| token_rates(old, "", None) != token_rates(v, "", None))
                {
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
    let key = model.trim().split('[').next()?.to_lowercase();
    let v = table.get(&key)?;
    let base = token_rates(v, "", None)?;
    let input = usage.input_tokens.unwrap_or(0);
    let read = usage.cached_input_tokens.unwrap_or(0);
    let write = usage.cache_creation_input_tokens.unwrap_or(0);
    let ordinary = if format == "anthropic_messages" {
        input
    } else {
        input.saturating_sub(read).saturating_sub(write)
    };
    let prompt = ordinary.saturating_add(read).saturating_add(write);
    let threshold = v
        .as_object()?
        .keys()
        .filter_map(|k| {
            k.strip_prefix("input_cost_per_token_above_")?
                .split("k_tokens")
                .next()?
                .parse::<u64>()
                .ok()
        })
        .filter(|n| prompt > n.saturating_mul(1000))
        .max();
    let suffix = threshold
        .map(|n| format!("_above_{n}k_tokens"))
        .unwrap_or_default();
    let standard = token_rates(v, &suffix, Some(base)).unwrap_or(base);
    let speed_suffix = match speed {
        Some("fast" | "priority") => "_priority",
        Some("ultrafast") => "_ultrafast",
        _ => "",
    };
    let selected = if speed_suffix.is_empty() {
        standard
    } else if speed_suffix == "_priority" && number(&v["provider_specific_entry"], "fast").is_some()
    {
        let multiple = number(&v["provider_specific_entry"], "fast")?;
        standard.map(|n| n * multiple)
    } else {
        token_rates(v, &format!("{suffix}{speed_suffix}"), Some(standard))
            .or_else(|| token_rates(v, speed_suffix, Some(base)))
            .unwrap_or(standard)
    };
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
pub(super) fn estimate(
    model: &str,
    usage: &ModelUsage,
    format: &str,
    speed: Option<&str>,
) -> Option<f64> {
    estimate_with(&*rates().read().ok()?, model, usage, format, speed)
}
pub(super) fn ensure_loaded(dir: &Path) {
    if rates().read().is_ok_and(|table| table.is_empty()) {
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
        *rates().write().map_err(|_| "Price lock poisoned")? = table;
        Ok(at)
    } else {
        Ok(0)
    }
}
use rusqlite::OptionalExtension;
pub(super) async fn refresh(dir: &Path, force: bool) -> Result<(), String> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = LOCK.lock().await;
    let fetched = load(dir)?;
    if Local::now().timestamp() - fetched < if force { 60 } else { 86400 } {
        return Ok(());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut error = String::new();
    for url in [URL, API_URL] {
        match download(&client, url).await {
            Ok(document) => {
                let table = parse(&document)?;
                let db = store::open(dir)?;
                db.execute("INSERT INTO price_cache VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET fetched_at=excluded.fetched_at,document=excluded.document",rusqlite::params![Local::now().timestamp(),document.to_string()]).map_err(|e|e.to_string())?;
                *rates().write().map_err(|_| "Price lock poisoned")? = table;
                return Ok(());
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

pub(super) fn reprice(db: &mut rusqlite::Connection) -> Result<(), String> {
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let rows = tx
        .prepare("SELECT * FROM records WHERE cost_source NOT IN ('user_override','cli_reported')")
        .map_err(|e| e.to_string())?
        .query_map([], store::read_record)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for r in rows {
        let cost = estimate(
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
        );
        tx.execute(
            "UPDATE records SET cost_usd=?1,cost_source=?2 WHERE id=?3",
            rusqlite::params![
                cost,
                if cost.is_some() {
                    "litellm"
                } else {
                    "unavailable"
                },
                r.id
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_prices_reprice_history_without_overwriting_reported_or_custom_costs() {
        let temp = tempfile::tempdir().unwrap();
        let mut db = store::open(temp.path()).unwrap();
        let row:UsageRecord=serde_json::from_value(serde_json::json!({"id":"auto","createdAt":1,"completedAt":1,"durationMs":0,"source":"cli","operation":"test","providerId":"p","providerName":"p","model":"reprice-test","apiFormat":"cli_inclusive","status":"success","usageSource":"provider_reported","inputTokens":1000,"outputTokens":100,"costUsd":99,"costSource":"api_estimate"})).unwrap();
        UsageLedger::default().append(temp.path(), &row).unwrap();
        for (id, source) in [("reported", "cli_reported"), ("custom", "user_override")] {
            let mut r = row.clone();
            r.id = id.into();
            r.cost_source = source.into();
            UsageLedger::default().append(temp.path(), &r).unwrap();
        }
        for price in [0.00001, 0.00002] {
            let document = serde_json::json!({"reprice-test":{"input_cost_per_token":price,"output_cost_per_token":price}});
            db.execute(
                "INSERT OR REPLACE INTO price_cache VALUES(1,123,?1)",
                [document.to_string()],
            )
            .unwrap();
            assert_eq!(load(temp.path()).unwrap(), 123);
            reprice(&mut db).unwrap();
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
