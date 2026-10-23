use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
/// Retry state is transient and run-scoped. An absolute deadline keeps countdowns
/// correct when a window is reopened or receives a replayed snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename_all = "camelCase")]
pub struct ChatRetryStatusPayload {
    pub attempt: u32,
    pub max_retries: u32,
    pub retry_at_ms: u64,
    pub reason: String,
}
