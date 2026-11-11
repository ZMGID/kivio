use kivio::settings::{sanitize_settings, OpenAIConfig, Settings};

#[test]
fn unconfigured_settings_have_no_model() {
    for settings in [
        Settings::default(),
        serde_json::from_str("{}").unwrap(),
        serde_json::from_str(r#"{"screenshotTranslation":{}}"#).unwrap(),
    ] {
        assert!(settings.translator_provider_id.is_empty());
        assert!(settings.translator_model.is_empty());
        assert!(settings.screenshot_translation.provider_id.is_empty());
        assert!(settings.screenshot_translation.model.is_empty());
        let settings = sanitize_settings(settings);
        assert!(settings.providers.is_empty());
        assert_eq!(
            settings.effective_chat_model(),
            (String::new(), String::new())
        );
        assert!(settings.lens.model.is_empty());
        let saved = serde_json::to_value(settings).unwrap();
        assert_eq!(saved["translatorModel"], "");
        assert_eq!(saved["screenshotTranslation"]["model"], "");
        assert_eq!(saved["chatModel"], "");
    }
}

#[test]
fn unconfigured_settings_clear_orphaned_model_names() {
    let settings = sanitize_settings(
        serde_json::from_value(serde_json::json!({
            "translatorProviderId": "default-translator", "translatorModel": "gpt-4o",
            "screenshotTranslation": {"providerId": "default-ocr", "model": "gpt-4o"},
            "lens": {"model": "gpt-4o"},
            "chatModel": "gpt-4o"
        }))
        .unwrap(),
    );
    assert!(settings.providers.is_empty());
    assert!(settings.translator_model.is_empty());
    assert!(settings.screenshot_translation.model.is_empty());
    assert!(settings.lens.model.is_empty());
    assert_eq!(
        settings.effective_chat_model(),
        (String::new(), String::new())
    );
}

#[test]
fn legacy_model_defaults_are_empty_but_explicit_choices_survive() {
    assert!(OpenAIConfig::default().model.is_empty());
    for model in [None, Some("gpt-4o"), Some("configured-model")] {
        let mut legacy = serde_json::json!({"apiKey": "test-key"});
        if let Some(model) = model {
            legacy["model"] = model.into();
        }
        let settings = sanitize_settings(
            serde_json::from_value(serde_json::json!({
                "openai": legacy.clone(), "screenshotTranslation": {"openai": legacy}
            }))
            .unwrap(),
        );
        assert_eq!(settings.providers.len(), 2);
        assert_eq!(settings.translator_model, model.unwrap_or_default());
        assert_eq!(
            settings.screenshot_translation.model,
            model.unwrap_or_default()
        );
        for provider in settings.providers {
            assert_eq!(
                provider.enabled_models,
                model.into_iter().collect::<Vec<_>>()
            );
        }
    }
}
