use tauri::State;

use crate::chat::model_metadata::{
    thinking_capabilities_for_model, ThinkingCapabilities, ThinkingOffMode,
};
use crate::settings::ModelProvider;
use crate::state::AppState;

/// Resolve the persisted choice using the same capability snapshot as the picker.
/// Off is rejected for always-on models; stale unsupported levels are reported,
/// rather than silently sending a different effort or falling back to provider defaults.
/// With no explicit choice, prefer High, then the highest available level.
pub(crate) fn resolve_thinking(
    conv_level: Option<&str>,
    _global_enabled: bool,
    provider: Option<&ModelProvider>,
    model: &str,
) -> Result<(bool, Option<String>), String> {
    let capabilities = thinking_capabilities_for_model(provider, model);
    if capabilities.off_mode == ThinkingOffMode::NotApplicable {
        // No reasoning API parameter is applicable to this model.
        return Ok((true, None));
    }
    if conv_level == Some("off") {
        if capabilities.off_mode == ThinkingOffMode::Unsupported {
            return Err(format!(
                "{model} 不支持关闭思考，请选择该模型支持的思考强度。"
            ));
        }
        return Ok((false, None));
    }
    if capabilities.levels.is_empty() {
        return Ok((true, None));
    }
    let level = match conv_level {
        Some(level @ ("low" | "medium" | "high" | "xhigh" | "max")) => level,
        _ => {
            if capabilities.levels.iter().any(|level| level == "high") {
                "high"
            } else {
                capabilities.levels.last().unwrap()
            }
        }
    };
    if !capabilities.levels.iter().any(|allowed| allowed == level) {
        return Err(format!(
            "{model} 不支持思考强度 {level}，支持：{}",
            capabilities.levels.join(", ")
        ));
    }
    Ok((true, Some(level.to_string())))
}

/// Return both the effort levels and Off semantics, including toggle-only models.
/// 传 `provider_id` 而不是 api_format：override 挂在 provider 上，api_format 也能就地取到。
#[tauri::command]
pub(crate) fn chat_thinking_capabilities_for_model(
    state: State<'_, AppState>,
    model: String,
    provider_id: Option<String>,
) -> ThinkingCapabilities {
    let settings = state.settings_read();
    let provider = provider_id
        .as_deref()
        .and_then(|id| settings.get_provider(id));
    thinking_capabilities_for_model(provider, &model)
}
