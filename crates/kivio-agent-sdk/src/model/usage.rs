//! Provider usage normalization, independent of billing and persistence.
use super::ModelUsage;
use serde_json::Value;

pub fn model_usage_from_openai_value(value: &Value) -> Option<ModelUsage> {
    let usage = value.get("usage")?.as_object()?;
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
        cached_input_tokens: cache_tokens([
            prompt_details.and_then(|details| details.get("cached_tokens")),
            usage.get("cache_read_input_tokens"),
            usage.get("prompt_cache_hit_tokens"),
            usage.get("cached_tokens"),
        ]),
        cache_creation_input_tokens: cache_tokens([
            prompt_details.and_then(|details| details.get("cache_creation_tokens")),
            prompt_details.and_then(|details| details.get("cache_write_tokens")),
            usage.get("cache_creation_input_tokens"),
        ]),
        reasoning_tokens: completion_details
            .and_then(|details| details.get("reasoning_tokens"))
            .and_then(Value::as_u64),
        context_window_tokens: None,
        context_tokens: None,
    })
}

// Some gateways emit a zero placeholder beside a populated provider-specific field.
fn cache_tokens<const N: usize>(fields: [Option<&Value>; N]) -> Option<u64> {
    let values = fields.map(|field| field.and_then(Value::as_u64));
    values
        .iter()
        .flatten()
        .copied()
        .find(|value| *value > 0)
        .or_else(|| values.into_iter().flatten().next())
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compatible_cache_fields_preserve_reads_and_writes() {
        for fields in [
            json!({"prompt_tokens_details":{"cached_tokens":80,"cache_write_tokens":10}}),
            json!({"cache_read_input_tokens":80,"cache_creation_input_tokens":10}),
            json!({"cached_tokens":80,"prompt_tokens_details":{"cache_creation_tokens":10}}),
            json!({"prompt_cache_hit_tokens":80,"cache_creation_input_tokens":10}),
            json!({"prompt_tokens_details":{"cached_tokens":0},"cache_read_input_tokens":80,
                "cache_creation_input_tokens":10}),
        ] {
            let mut value =
                json!({"usage":{"prompt_tokens":100,"completion_tokens":5,"total_tokens":105}});
            value["usage"]
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let usage = model_usage_from_openai_value(&value).unwrap();
            assert_eq!(usage.cached_input_tokens, Some(80), "{value}");
            assert_eq!(usage.cache_creation_input_tokens, Some(10), "{value}");
            assert_eq!(usage.input_tokens, Some(100));
        }
    }

    #[test]
    fn missing_cache_telemetry_is_distinct_from_zero() {
        assert!(model_usage_from_openai_value(&json!({"usage":null})).is_none());
        let missing =
            model_usage_from_openai_value(&json!({"usage":{"prompt_tokens":100}})).unwrap();
        assert_eq!(missing.cached_input_tokens, None);
        let zero = model_usage_from_openai_value(
            &json!({"usage":{"prompt_tokens_details":{"cached_tokens":0}}}),
        )
        .unwrap();
        assert_eq!(zero.cached_input_tokens, Some(0));
    }

    #[test]
    fn standard_cache_count_takes_precedence_over_other_positive_aliases() {
        let usage = model_usage_from_openai_value(&json!({"usage":{
            "prompt_tokens_details":{"cached_tokens":80},"cached_tokens":90
        }}))
        .unwrap();
        assert_eq!(usage.cached_input_tokens, Some(80));
    }
}
