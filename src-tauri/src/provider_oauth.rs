//! Tauri command facade for the shared OAuth credential and login owner.
use crate::{settings::ModelProvider, state::AppState};
use serde_json::Value;
#[path = "provider_oauth_runtime.rs"]
mod runtime;
pub use runtime::resolve_for_model;
pub use runtime::{
    CODEX_BASE, KIMI_BASE, Login, OAuthConfig, PollResult, antigravity, apply_headers, codex_body,
    header_pairs, inference_client, is_codex, validate_provider,
};
pub mod account;
pub mod usage;

#[tauri::command]
pub async fn provider_oauth_start(
    provider: String,
    use_system_proxy: bool,
) -> Result<Login, String> {
    runtime::provider_oauth_start(provider, use_system_proxy).await
}
#[tauri::command]
pub async fn provider_oauth_poll(login_id: String) -> Result<PollResult, String> {
    runtime::provider_oauth_poll(login_id).await
}
#[tauri::command]
pub async fn provider_oauth_cancel(login_id: String) {
    runtime::provider_oauth_cancel(login_id).await
}
#[tauri::command]
pub async fn provider_oauth_disconnect(credential_id: String) -> Result<(), String> {
    runtime::provider_oauth_disconnect(credential_id).await
}
pub async fn resolve_provider(
    _state: &AppState,
    provider: &ModelProvider,
) -> Result<ModelProvider, String> {
    runtime::resolve_provider(provider).await
}
pub async fn models(_state: &AppState, provider: &ModelProvider) -> Result<Vec<String>, String> {
    runtime::models(provider).await
}
pub(crate) fn export_credentials(settings: &crate::settings::Settings) -> Result<Value, String> {
    runtime::export_credentials(&settings.providers)
}
pub(crate) fn import_credentials(
    settings: &mut crate::settings::Settings,
    value: &Value,
) -> Result<Vec<String>, String> {
    runtime::import_credentials(&mut settings.providers, value)
}
pub(crate) use runtime::discard_imported_credentials;

pub async fn test_connection(
    state: &AppState,
    provider: &ModelProvider,
    model: Option<&str>,
) -> Result<(), String> {
    use crate::chat::model::*;
    let Some(model) = model else {
        models(state, provider).await?;
        return Ok(());
    };
    let request = GenerateRequest {
        model: model.to_string(),
        system: String::new(),
        messages: vec![ModelMessage::text(
            ModelRole::User,
            "Reply with exactly: kivio-ok",
        )],
        tools: vec![],
        options: GenerateOptions {
            max_tokens: 64,
            ..Default::default()
        },
        metadata: RequestMetadata {
            label: "OAuth connection test".into(),
            ..Default::default()
        },
    };
    generate_with_chat_provider(state, provider, 1, request)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
