use crate::{
    RuntimeHost,
    host::ModelHost,
    provider_probe, provider_request, runtime,
    settings::{ModelProvider, ProviderApiFormat},
    transport,
};
use std::path::Path;

pub async fn models(provider: &ModelProvider, root: &Path) -> Result<Vec<String>, String> {
    if provider.request.oauth.is_some() {
        return crate::provider_oauth::models(provider).await;
    }
    let host = RuntimeHost::shared(provider, root);
    let format = provider.api_format_kind();
    let url = format!(
        "{}/models{}",
        provider.base_url.trim_end_matches('/'),
        if format == ProviderApiFormat::Gemini {
            "?pageSize=1000"
        } else {
            ""
        }
    );
    let keys = if provider.api_keys.is_empty() {
        vec![String::new()]
    } else {
        provider.api_keys.clone()
    };
    let response = runtime::send_with_failover(
        host.provider_runtime(),
        "Models API",
        1,
        &provider.id,
        &keys,
        |key| {
            let request = provider_request::apply(
                provider_request::apply_api_key_auth(
                    host.client_for(provider).get(&url),
                    format,
                    key,
                ),
                provider,
                None,
            );
            transport::with_standard_request_timeout(request).send()
        },
    )
    .await?;
    let value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse models response JSON: {e}"))?;
    let mut models = provider_probe::parse_model_list_ids(&value)?;
    if crate::opencode_free::is_endpoint(&provider.base_url)
        && keys.iter().all(|key| key.trim().is_empty())
    {
        models.retain(|model| crate::opencode_free::is_free_model(model));
    }
    Ok(models)
}
pub async fn connection(provider: &ModelProvider, root: &Path) -> Result<(), String> {
    let provider = crate::provider_oauth::resolve_provider(provider).await?;
    let model = provider
        .enabled_models
        .first()
        .map(String::as_str)
        .filter(|model| !model.trim().is_empty());
    let host = RuntimeHost::shared(&provider, root);
    if provider.request.oauth.is_some() {
        let Some(model) = model else {
            crate::provider_oauth::models(&provider).await?;
            return Ok(());
        };
        let request = crate::GenerateRequest {
            model: model.to_owned(),
            system: String::new(),
            messages: vec![crate::ModelMessage::text(
                crate::ModelRole::User,
                "Reply with exactly: kivio-ok",
            )],
            tools: vec![],
            options: crate::GenerateOptions {
                max_tokens: 64,
                ..Default::default()
            },
            metadata: crate::RequestMetadata {
                label: "OAuth connection test".into(),
                ..Default::default()
            },
        };
        return crate::dispatch::generate_resolved(&*host, &provider, 1, request)
            .await
            .map(|_| ())
            .map_err(|e| e.message);
    }
    let format = provider.api_format_kind();
    let key = provider.preferred_api_key().unwrap_or("");
    let client = host.client_for(&provider);
    let base = provider.base_url.trim_end_matches('/');
    let build = || {
        let request = if let Some(model) = model {
            let (url, body) = provider_probe::connection_test_url_and_body(format, base, model);
            client.post(url).json(&body)
        } else {
            client.get(format!("{base}/models"))
        };
        let request = provider_request::apply(
            provider_request::apply_api_key_auth(request, format, key),
            &provider,
            None,
        );
        transport::with_standard_request_timeout(request).send()
    };
    runtime::send_with_retry("Provider API", 1, build)
        .await
        .map(|_| ())
}
