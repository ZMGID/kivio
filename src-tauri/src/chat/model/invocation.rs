//! Shared credential resolution and provider invocation for every desktop host.
use super::types::*;

pub async fn generate_with_chat_provider(
    state: &dyn super::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    let resolved = crate::provider_oauth::resolve_for_model(provider)
        .await
        .map_err(ModelError::new)?;
    super::dispatch::generate_resolved(state, &resolved, retry_attempts, request).await
}
pub async fn stream_with_chat_provider(
    state: &dyn super::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
    sink: &mut (dyn StreamSink + Send),
) -> Result<GenerateOutput, ModelError> {
    crate::chat::video::validate_request(provider, &request)?;
    let resolved = crate::provider_oauth::resolve_for_model(provider)
        .await
        .map_err(ModelError::new)?;
    super::dispatch::stream_resolved(state, &resolved, retry_attempts, request, sink).await
}
