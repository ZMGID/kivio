//! Provider usage normalization, independent of billing and persistence.
use super::ModelUsage;
use serde_json::Value;

pub fn model_usage_from_openai_value(value: &Value) -> Option<ModelUsage> {
    let usage = value.get("usage")?;
    let prompt_details = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"));
    let completion_details = usage
        .get("completion_tokens_details")
        .or_else(|| usage.get("output_tokens_details"));
    Some(ModelUsage {
        input_tokens: usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("input_tokens").and_then(Value::as_u64)),
        output_tokens: usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .or_else(|| usage.get("output_tokens").and_then(Value::as_u64)),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
        cached_input_tokens: prompt_details
            .and_then(|details| details.get("cached_tokens"))
            .and_then(Value::as_u64),
        cache_creation_input_tokens: prompt_details
            .and_then(|details| details.get("cache_creation_tokens"))
            .and_then(Value::as_u64),
        reasoning_tokens: completion_details
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_u64),
        context_window_tokens: None,
        context_tokens: None,
    })
}

pub fn model_usage_from_anthropic_value(value: &Value) -> Option<ModelUsage> {
    let usage = value.get("usage")?;
    let input = usage.get("input_tokens").and_then(Value::as_u64);
    let output = usage.get("output_tokens").and_then(Value::as_u64);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64);
    let cache_read = usage.get("cache_read_input_tokens").and_then(Value::as_u64);
    Some(ModelUsage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input.zip(output).map(|(a, b)| a.saturating_add(b)),
        cached_input_tokens: cache_read,
        cache_creation_input_tokens: cache_creation,
        reasoning_tokens: None,
        context_window_tokens: None,
        context_tokens: None,
    })
}

pub fn model_usage_from_stream_value(value: &Value) -> Option<ModelUsage> {
    model_usage_from_openai_value(value).or_else(|| model_usage_from_anthropic_value(value))
}
