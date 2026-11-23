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
    let retry_after = retry_after_header(response.headers());
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
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
    )
    .with_request_id(request_id.as_deref());
    error.retry_after = retry_after;
    Err(error)
}

pub(super) fn retry_after_header(
    headers: &reqwest::header::HeaderMap,
) -> Option<std::time::Duration> {
    parse_retry_after(
        headers.get("retry-after-ms").and_then(|v| v.to_str().ok()),
        headers
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
    )
}

pub(super) fn parse_retry_after(
    milliseconds: Option<&str>,
    seconds_or_date: Option<&str>,
) -> Option<std::time::Duration> {
    use std::time::Duration;
    if let Some(ms) = milliseconds.and_then(|value| value.parse().ok()) {
        return Some(Duration::from_millis(ms));
    }
    let value = seconds_or_date?;
    value.parse().ok().map(Duration::from_secs).or_else(|| {
        chrono::DateTime::parse_from_rfc2822(value)
            .ok()
            .map(|date| {
                (date.with_timezone(&chrono::Utc) - chrono::Utc::now())
                    .to_std()
                    .unwrap_or_default()
            })
    })
}
