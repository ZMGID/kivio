pub mod anthropic;
pub mod context_usage;
pub mod error;
pub mod gemini;
pub mod openai;
pub mod responses;
pub mod sse;
mod transport;
pub mod types;
pub mod usage;

pub use error::{
    extract_status_code, is_missing_stream_terminal_error, is_permanent_provider_error,
    stream_read_error, ModelError, ModelErrorKind,
};
pub use types::*;

/// Routing and decoding use the selected model and delivery mode.
fn apply_body_options(
    body: &mut serde_json::Value,
    options: &serde_json::Value,
    allow_stream_options: bool,
) {
    if let Some(fields) = options.as_object() {
        for (key, value) in fields {
            if matches!(key.as_str(), "model" | "stream")
                || (key == "stream_options" && !allow_stream_options)
            {
                continue;
            }
            body[key] = value.clone();
        }
    }
}

#[cfg(test)]
pub(crate) use error::is_tools_unsupported_error;
