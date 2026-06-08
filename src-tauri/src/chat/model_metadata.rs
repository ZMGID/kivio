use std::sync::OnceLock;

use serde_json::Value;

use crate::settings::{ModelInfo, ModelPricing, ModelProvider};

const FALLBACK_CONTEXT_WINDOW_TOKENS: usize = 200_000;

fn context_window_from_model_info(info: Option<&ModelInfo>) -> Option<usize> {
    info.and_then(|info| info.context_window)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn max_output_from_model_info(info: Option<&ModelInfo>) -> Option<u32> {
    info.and_then(|info| info.max_output)
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn model_vision_from_model_info(info: Option<&ModelInfo>) -> Option<bool> {
    info.and_then(|info| info.capabilities.as_ref())
        .and_then(|capabilities| capabilities.vision)
}

fn model_image_generation_from_model_info(info: Option<&ModelInfo>) -> Option<bool> {
    info.and_then(|info| info.capabilities.as_ref())
        .and_then(|capabilities| capabilities.image_generation)
}

fn model_database_entries() -> Option<&'static serde_json::Map<String, Value>> {
    static MODEL_DATABASE: OnceLock<Value> = OnceLock::new();
    MODEL_DATABASE
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../../src/data/modelDatabase.json"))
                .unwrap_or(Value::Null)
        })
        .as_object()
}

fn model_database_entry(model: &str) -> Option<&'static Value> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }

    let entries = model_database_entries()?;
    let name = model.to_ascii_lowercase();
    let stripped = name.rsplit('/').next().unwrap_or(&name);

    if let Some(entry) = entries.get(name.as_str()) {
        return Some(entry);
    }
    if let Some(entry) = entries.get(stripped) {
        return Some(entry);
    }

    let candidates = if name == stripped {
        vec![stripped]
    } else {
        vec![name.as_str(), stripped]
    };

    entries
        .iter()
        .filter_map(|(key, entry)| {
            if key == "_meta"
                || !candidates
                    .iter()
                    .any(|candidate| candidate.starts_with(key) && key.len() < candidate.len())
            {
                return None;
            }
            Some((key.len(), entry))
        })
        .max_by_key(|(key_len, _)| *key_len)
        .map(|(_, entry)| entry)
        .or_else(|| {
            entries
                .iter()
                .filter_map(|(key, entry)| {
                    if key == "_meta"
                        || !candidates
                            .iter()
                            .any(|candidate| key != candidate && candidate.contains(key))
                    {
                        return None;
                    }
                    Some((key.len(), entry))
                })
                .max_by_key(|(key_len, _)| *key_len)
                .map(|(_, entry)| entry)
        })
}

fn model_database_context_window(model: &str) -> Option<usize> {
    context_window_from_database_entry(model_database_entry(model))
}

fn model_database_max_output(model: &str) -> Option<u32> {
    max_output_from_database_entry(model_database_entry(model))
}

fn context_window_from_database_entry(entry: Option<&Value>) -> Option<usize> {
    entry?
        .get("contextWindow")
        .and_then(Value::as_u64)
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn max_output_from_database_entry(entry: Option<&Value>) -> Option<u32> {
    entry?
        .get("maxOutput")
        .and_then(Value::as_u64)
        .and_then(|tokens| u32::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
}

fn model_database_vision(model: &str) -> Option<bool> {
    model_database_entry(model)?
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("vision"))
        .and_then(Value::as_bool)
}

fn model_database_image_generation(model: &str) -> Option<bool> {
    model_database_entry(model)?
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("imageGeneration"))
        .and_then(Value::as_bool)
}

fn model_pricing_from_model_info(info: Option<&ModelInfo>) -> Option<ModelPricing> {
    info.and_then(|info| info.pricing.clone())
}

fn model_database_pricing(model: &str) -> Option<ModelPricing> {
    let pricing = model_database_entry(model)?.get("pricing")?;
    Some(ModelPricing {
        input: pricing.get("input").and_then(Value::as_f64),
        output: pricing.get("output").and_then(Value::as_f64),
        cached_input: pricing.get("cachedInput").and_then(Value::as_f64),
    })
}

pub(crate) fn model_supports_vision(provider: Option<&ModelProvider>, model: &str) -> Option<bool> {
    let provider = provider?;
    model_vision_from_model_info(provider.model_overrides.get(model))
        .or_else(|| model_database_vision(model))
}

pub(crate) fn model_supports_image_generation(
    provider: Option<&ModelProvider>,
    model: &str,
) -> Option<bool> {
    let provider = provider?;
    model_image_generation_from_model_info(provider.model_overrides.get(model))
        .or_else(|| model_database_image_generation(model))
        .or_else(|| image_generation_model_name_heuristic(provider, model))
}

pub(crate) fn model_can_generate_images_directly(provider: &ModelProvider, model: &str) -> bool {
    model_supports_image_generation(Some(provider), model) == Some(true)
        && crate::chat::image_generation::has_known_direct_image_generation_route(provider, model)
}

fn image_generation_model_name_heuristic(provider: &ModelProvider, model: &str) -> Option<bool> {
    let descriptor = format!(
        "{} {} {} {}",
        provider.name, provider.base_url, provider.api_format, model
    )
    .to_ascii_lowercase();
    let known_image_model = [
        "gpt-image",
        "dall-e",
        "grok-imagine-image",
        "gemini-3.1-flash-image",
        "gemini-3-pro-image",
        "gemini-2.5-flash-image",
        "flux",
        "recraft",
        "riverflow",
        "stable-diffusion",
        "sdxl",
        "ideogram",
        "imagen",
        "image-generation",
        "image_generation",
    ]
    .iter()
    .any(|needle| descriptor.contains(needle));
    if known_image_model {
        Some(true)
    } else {
        None
    }
}

pub(crate) fn context_window_for_model(
    provider: Option<&ModelProvider>,
    model: &str,
) -> (usize, bool) {
    if let Some(tokens) = context_window_from_model_info(
        provider.and_then(|provider| provider.model_overrides.get(model)),
    ) {
        return (tokens, false);
    }
    if let Some(tokens) = model_database_context_window(model) {
        return (tokens, false);
    }

    let lower = model.to_ascii_lowercase();
    let known = [
        ("1m", 1_000_000usize),
        ("200k", 200_000usize),
        ("128k", 128_000usize),
        ("100k", 100_000usize),
        ("64k", 64_000usize),
        ("32k", 32_000usize),
        ("16k", 16_000usize),
        ("8k", 8_000usize),
    ];
    for (needle, tokens) in known {
        if lower.contains(needle) {
            return (tokens, false);
        }
    }
    if lower.contains("claude") {
        return (200_000, false);
    }
    if lower.contains("gpt-4o")
        || lower.contains("gpt-4.1")
        || lower.contains("gpt-5")
        || lower.contains("deepseek")
        || lower.contains("qwen")
        || lower.contains("gemini")
    {
        return (128_000, true);
    }
    (FALLBACK_CONTEXT_WINDOW_TOKENS, true)
}

pub(crate) fn chat_max_output_tokens_for_model(
    provider: Option<&ModelProvider>,
    model: &str,
    fallback: u32,
) -> u32 {
    max_output_from_model_info(provider.and_then(|provider| provider.model_overrides.get(model)))
        .or_else(|| model_database_max_output(model))
        .unwrap_or(fallback)
}

pub(crate) fn pricing_for_model(
    provider: Option<&ModelProvider>,
    model: &str,
) -> Option<(ModelPricing, String)> {
    if let Some(pricing) = model_pricing_from_model_info(
        provider.and_then(|provider| provider.model_overrides.get(model)),
    ) {
        return Some((pricing, "user_override".to_string()));
    }
    model_database_pricing(model).map(|pricing| (pricing, "model_pricing".to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::settings::{ModelInfo, ModelProvider};

    use super::*;

    fn test_provider_with_overrides(model_overrides: HashMap<String, ModelInfo>) -> ModelProvider {
        ModelProvider {
            id: "provider".to_string(),
            name: "Provider".to_string(),
            api_keys: vec!["sk-test".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: Vec::new(),
            enabled_models: Vec::new(),
            supports_tools: true,
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides,
        }
    }

    #[test]
    fn context_window_uses_model_override_before_name_heuristic() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "deepseek-v4-flash".to_string(),
            ModelInfo {
                context_window: Some(1_048_576),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            context_window_for_model(Some(&provider), "deepseek-v4-flash"),
            (1_048_576, false)
        );
    }

    #[test]
    fn context_window_uses_builtin_model_database_defaults() {
        assert_eq!(
            context_window_for_model(None, "deepseek-v4-flash"),
            (1_048_576, false)
        );
    }

    #[test]
    fn chat_max_output_uses_builtin_model_database_defaults() {
        assert_eq!(
            chat_max_output_tokens_for_model(None, "deepseek-v4-flash", 32_768),
            131_072
        );
    }

    #[test]
    fn chat_max_output_uses_model_override_before_database() {
        let mut overrides = HashMap::new();
        overrides.insert(
            "deepseek-v4-flash".to_string(),
            ModelInfo {
                max_output: Some(65_536),
                ..ModelInfo::default()
            },
        );
        let provider = test_provider_with_overrides(overrides);

        assert_eq!(
            chat_max_output_tokens_for_model(Some(&provider), "deepseek-v4-flash", 32_768),
            65_536
        );
    }

    #[test]
    fn chat_max_output_falls_back_to_setting_when_metadata_missing() {
        assert_eq!(
            chat_max_output_tokens_for_model(None, "custom-model", 32_768),
            32_768
        );
    }

    #[test]
    fn context_window_keeps_name_heuristic_when_metadata_missing() {
        assert_eq!(
            context_window_for_model(None, "custom-200k"),
            (200_000, false)
        );
        assert_eq!(
            context_window_for_model(None, "custom-deepseek-model"),
            (128_000, true)
        );
    }
}
