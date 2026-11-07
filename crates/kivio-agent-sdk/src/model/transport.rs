use super::{stream_read_error, ModelError, ModelErrorKind};

pub(crate) async fn send(
    request: reqwest::RequestBuilder,
    label: &str,
) -> Result<reqwest::Response, ModelError> {
    let response = request.send().await.map_err(|error| {
        ModelError::with_kind(
            format!("{label} request failed: {error}"),
            if error.is_timeout() {
                ModelErrorKind::Timeout
            } else if error.is_builder() {
                ModelErrorKind::InvalidRequest
            } else {
                ModelErrorKind::Transport
            },
        )
    })?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .parse::<u64>()
                .ok()
                .map(std::time::Duration::from_secs)
                .or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(value)
                        .ok()
                        .map(|date| {
                            (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                                .to_std()
                                .unwrap_or_default()
                        })
                })
        });
    let body = response
        .text()
        .await
        .map_err(|error| stream_read_error(&format!("{label} error body"), &error))?;
    let mut error = ModelError::from_provider_http(
        status.as_u16(),
        &body,
        format!(
            "{label} API {}: {}",
            status.as_u16(),
            body.chars().take(500).collect::<String>()
        ),
    );
    error.retry_after = retry_after;
    Err(error)
}
