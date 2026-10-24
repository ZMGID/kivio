//! Shared operation budgets used by the tool loop and tool implementations.
use serde_json::Value;
pub const IMAGE_GENERATION_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_BASH_OUTPUT_WAIT_MS: u64 = 30_000;
pub(crate) const RUN_DEFAULT_TIMEOUT_SECS: u64 = 600;
pub(crate) const RUN_MAX_TIMEOUT_SECS: u64 = 1_800;
const RUN_TIMEOUT_BUFFER_MS: u64 = 5_000;

pub(crate) fn run_timeout_ms(arguments: &Value) -> u64 {
    parse_timeout_secs(arguments)
        .saturating_mul(1000)
        .saturating_add(RUN_TIMEOUT_BUFFER_MS)
}

pub(crate) fn parse_timeout_secs(arguments: &Value) -> u64 {
    arguments
        .get("timeout_seconds")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        })
        .unwrap_or(RUN_DEFAULT_TIMEOUT_SECS)
        .clamp(5, RUN_MAX_TIMEOUT_SECS)
}
