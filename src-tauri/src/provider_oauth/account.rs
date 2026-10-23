//! Tauri adapter for the shared OAuth account query.
pub use super::runtime::account::AccountIdentity;
use crate::{settings::ModelProvider, state::AppState};
#[tauri::command]
pub async fn provider_oauth_account(
    _state: tauri::State<'_, AppState>,
    provider: ModelProvider,
) -> Result<AccountIdentity, String> {
    super::runtime::account::provider_oauth_account(provider).await
}
