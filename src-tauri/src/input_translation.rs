//! Input translation selection and prompt policy, shared by desktop hosts.
use crate::settings::Settings;
pub struct InputTranslation {
    pub provider_id: String,
    pub model: String,
    pub prompt: String,
    pub attempts: usize,
}
pub fn prepare(settings: &Settings, text: &str) -> Result<Option<InputTranslation>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if settings.translator_model.trim().is_empty() {
        return Err("Please select a model first".into());
    }
    let target = crate::utils::resolve_target_lang(&settings.target_lang, text);
    Ok(Some(InputTranslation {
        provider_id: settings.translator_provider_id.clone(),
        model: settings.translator_model.clone(),
        prompt: crate::prompts::build_translation_prompt(
            text,
            crate::utils::language_name(&target),
            settings.translator_prompt.as_deref(),
        ),
        attempts: if settings.retry_enabled {
            settings.retry_attempts.clamp(1, 11) as usize
        } else {
            1
        },
    }))
}
