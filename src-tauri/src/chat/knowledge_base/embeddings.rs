//! Embedding provider adapter. Anthropic has no embeddings endpoint, so this
//! is deliberately separate from `chat/model/` and only speaks the
//! OpenAI-compatible `POST {base_url}/embeddings` shape (OpenAI / Jina / Voyage
//! / DashScope / SiliconFlow / local LM Studio all match it). Multi-key
//! failover + retry is reused from `crate::api`.

use serde_json::Value;

use crate::api::{send_with_failover, with_standard_request_timeout};
use crate::settings::ModelProvider;
use crate::state::AppState;

/// Embed a batch of inputs in one request. Returns one vector per input, in
/// input order.
pub async fn embed_batch(
    state: &AppState,
    provider: &ModelProvider,
    model: &str,
    inputs: &[String],
    attempts: usize,
) -> Result<Vec<Vec<f32>>, String> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    if model.trim().is_empty() {
        return Err("Embedding model is not set".to_string());
    }
    let keys: Vec<String> = provider
        .api_keys
        .iter()
        .filter(|k| !k.trim().is_empty())
        .cloned()
        .collect();
    if keys.is_empty() {
        return Err(format!("Provider '{}' has no API key", provider.name));
    }
    let url = format!("{}/embeddings", provider.base_url.trim_end_matches('/'));
    let body = serde_json::json!({ "model": model, "input": inputs });

    let response = send_with_failover(
        state,
        "Embeddings API",
        attempts,
        &provider.id,
        &keys,
        |key| {
            with_standard_request_timeout(state.http.post(url.clone()).bearer_auth(key).json(&body))
                .send()
        },
    )
    .await?;

    let value: Value = response
        .json()
        .await
        .map_err(|e| format!("embeddings response not JSON: {e}"))?;

    let vectors = parse_embeddings_response(&value)?;
    if vectors.len() != inputs.len() {
        return Err(format!(
            "embeddings count mismatch: got {}, expected {}",
            vectors.len(),
            inputs.len()
        ));
    }
    Ok(vectors)
}

/// Parse the OpenAI-compatible `/embeddings` response body into row-ordered
/// vectors. Pure (no I/O) so the wire-format contract is unit-testable.
pub fn parse_embeddings_response(value: &Value) -> Result<Vec<Vec<f32>>, String> {
    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| {
            // Surface the provider's error message if present.
            let msg = value
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("missing `data` array");
            format!("embeddings API error: {msg}")
        })?;

    let mut indexed: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
    for item in data {
        let idx = item.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        let emb = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or("embeddings: item missing `embedding`")?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect::<Vec<f32>>();
        indexed.push((idx, emb));
    }
    indexed.sort_by_key(|(i, _)| *i);
    Ok(indexed.into_iter().map(|(_, e)| e).collect())
}

/// Embed a single query string.
pub async fn embed_query(
    state: &AppState,
    provider: &ModelProvider,
    model: &str,
    query: &str,
    attempts: usize,
) -> Result<Vec<f32>, String> {
    let mut v = embed_batch(state, provider, model, &[query.to_string()], attempts).await?;
    v.pop()
        .ok_or_else(|| "embeddings: empty result for query".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_reorders_by_index() {
        // Provider returns rows out of order; we must sort by `index`.
        let body = serde_json::json!({
            "data": [
                { "index": 1, "embedding": [0.0, 1.0] },
                { "index": 0, "embedding": [1.0, 0.0] },
            ],
            "usage": { "prompt_tokens": 4 }
        });
        let vectors = parse_embeddings_response(&body).unwrap();
        assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    }

    #[test]
    fn surfaces_provider_error_message() {
        let body = serde_json::json!({ "error": { "message": "invalid api key" } });
        let err = parse_embeddings_response(&body).unwrap_err();
        assert!(err.contains("invalid api key"), "got: {err}");
    }

    #[test]
    fn rejects_item_without_embedding() {
        let body = serde_json::json!({ "data": [{ "index": 0 }] });
        assert!(parse_embeddings_response(&body).is_err());
    }
}
