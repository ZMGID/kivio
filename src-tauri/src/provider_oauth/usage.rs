//! Tauri adapter for the shared OAuth usage query.
pub use super::runtime::usage::{AccountUsage, UsageWindow};
use crate::{settings::ModelProvider, state::AppState};
#[tauri::command]
pub async fn provider_oauth_usage(
    _state: tauri::State<'_, AppState>,
    provider: ModelProvider,
) -> Result<AccountUsage, String> {
    super::runtime::usage::provider_oauth_usage(provider).await
}
