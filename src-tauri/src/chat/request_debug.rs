//! In-memory provider request capture for the developer "Request Debug" panel.
//!
//! When `settings.chat_tools.request_debug_enabled` is on, each model adapter
//! (`openai` / `anthropic` / `responses`) records the full request (url + sanitized
//! headers + body) and the aggregated response (text / tool_calls / finish_reason /
//! usage / error) into a bounded ring buffer. The existing disk mirror remains
//! best-effort; memory is process-local and clear removes both representations.
//!
//! **Zero overhead when disabled**: adapters must check `state.request_debug_enabled()`
//! BEFORE constructing any body/headers/record — this module never runs otherwise.
//!
//! **No secret leak**: [`sanitize_headers`] masks `Authorization` / `x-api-key` style
//! headers to a short preview (`Bearer sk-x…`); the request body never carries the key.

#[cfg(test)]
use std::collections::BTreeMap;

#[cfg(test)]
use crate::chat::model::{GenerateOutput, GenerateRequest, ModelUsage, PendingToolCall};
#[cfg(test)]
use crate::settings::ModelProvider;
#[cfg(test)]
use uuid::Uuid;

use crate::state::AppState;

#[path = "request_debug_data.rs"]
mod data;
pub use data::*;

include!("request_debug_owner.rs");

/// Push a record into the ring buffer, evicting the oldest entries past capacity.
/// Also mirrors the whole buffer to disk (newest-first JSONL) so the last
/// [`REQUEST_DEBUG_CAPACITY`] requests survive a restart and can be inspected
/// out-of-band. ponytail: full-buffer rewrite per call (≤50 capped records) —
/// switch to append+rotate only if this ever shows up in a profile.
pub fn record(state: &AppState, record: RequestDebugRecord) {
    state.request_debug().record(record);
}

/// Path of the on-disk mirror: `<app_data>/request_debug/records.jsonl`.
#[cfg(test)]
fn debug_log_path(state: &AppState) -> PathBuf {
    state.request_debug().path.clone()
}

/// Snapshot the buffer, newest first.
pub fn snapshot(state: &AppState) -> Vec<RequestDebugRecord> {
    state.request_debug().snapshot()
}

/// Empty the buffer and remove the on-disk mirror.
pub fn clear(state: &AppState) {
    state.request_debug().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(id_seed: &str) -> RequestDebugRecord {
        RequestDebugRecord {
            id: format!("dbg_{id_seed}"),
            created_at: 0,
            duration_ms: 0,
            provider_id: "p".into(),
            provider_name: "P".into(),
            model: "m".into(),
            api_format: "openai_chat".into(),
            operation: "chat".into(),
            source: "chat".into(),
            conversation_id: None,
            message_id: None,
            status: "success".into(),
            request: RequestDebugRequest {
                url: "https://x/v1/chat/completions".into(),
                headers: BTreeMap::new(),
                body: serde_json::json!({}),
                stream: false,
            },
            response: RequestDebugResponse::default(),
        }
    }

    #[test]
    fn ring_buffer_evicts_oldest_past_capacity() {
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        // Push one past capacity; the first record must have been evicted.
        for i in 0..(REQUEST_DEBUG_CAPACITY + 1) {
            record(&state, sample_record(&i.to_string()));
        }
        let snap = snapshot(&state);
        assert_eq!(snap.len(), REQUEST_DEBUG_CAPACITY);
        // Newest first: the last pushed (id = capacity) is at the front.
        assert_eq!(
            snap.first().unwrap().id,
            format!("dbg_{REQUEST_DEBUG_CAPACITY}")
        );
        // The very first record (id = 0) was evicted; oldest kept is id = 1.
        assert_eq!(snap.last().unwrap().id, "dbg_1");
        assert!(snap.iter().all(|r| r.id != "dbg_0"));
    }

    #[test]
    fn concurrent_records_keep_disk_order_in_sync_with_memory() {
        let base = std::env::temp_dir().join(format!("kivio-dbg-owner-{}", Uuid::new_v4()));
        let owner = std::sync::Arc::new(RequestDebugState::new(&base.join("usage")));
        let workers: Vec<_> = (0..8)
            .map(|i| {
                let owner = owner.clone();
                std::thread::spawn(move || owner.record(sample_record(&i.to_string())))
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let disk: Vec<String> = fs::read_to_string(&owner.path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<RequestDebugRecord>(line).unwrap().id)
            .collect();
        let memory: Vec<_> = owner.snapshot().into_iter().map(|r| r.id).collect();
        assert_eq!(disk, memory);
        assert_eq!(memory.len(), 8);
        owner.clear();
        assert!(owner.snapshot().is_empty());
        assert!(!owner.path.exists());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn clear_empties_the_buffer() {
        let state =
            AppState::new_headless(crate::settings::Settings::default(), std::env::temp_dir());
        record(&state, sample_record("a"));
        assert_eq!(snapshot(&state).len(), 1);
        clear(&state);
        assert!(snapshot(&state).is_empty());
    }

    #[test]
    fn record_mirrors_to_disk_and_clear_removes_it() {
        // Isolated usage_dir so the on-disk mirror lands in a unique place.
        let base = std::env::temp_dir().join(format!("kivio-dbg-test-{}", Uuid::new_v4()));
        let usage_dir = base.join("usage");
        std::fs::create_dir_all(&usage_dir).unwrap();
        let state = AppState::new_headless(crate::settings::Settings::default(), usage_dir);
        let path = debug_log_path(&state);

        record(&state, sample_record("a"));
        record(&state, sample_record("b"));

        let contents = std::fs::read_to_string(&path).expect("mirror file written");
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2, "one JSON line per record");
        // Newest-first: "b" precedes "a".
        assert!(lines[0].contains("dbg_b"));
        assert!(lines[1].contains("dbg_a"));
        // Each line is a standalone parseable record.
        let first: RequestDebugRecord = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first.id, "dbg_b");

        clear(&state);
        assert!(!path.exists(), "clear removes the mirror file");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sanitize_masks_credentials_and_keeps_the_rest() {
        let mut headers = BTreeMap::new();
        headers.insert("Authorization".into(), "Bearer sk-abcdef123456".into());
        headers.insert("x-api-key".into(), "sk-ant-secret-key".into());
        headers.insert("Accept-Encoding".into(), "identity".into());
        headers.insert("x-session-id".into(), "conv_123".into());

        let sanitized = sanitize_headers(headers);

        assert_eq!(sanitized["Authorization"], "Bearer sk-a…");
        assert_eq!(sanitized["x-api-key"], "sk-a…");
        // Non-sensitive headers are untouched.
        assert_eq!(sanitized["Accept-Encoding"], "identity");
        assert_eq!(sanitized["x-session-id"], "conv_123");
        // No full secret survives anywhere in the sanitized map.
        assert!(sanitized
            .values()
            .all(|v| !v.contains("abcdef123456") && !v.contains("ant-secret-key")));
    }

    #[test]
    fn build_debug_record_produces_sanitized_complete_record() {
        let provider = ModelProvider {
            id: "test".into(),
            name: "Test".into(),
            api_keys: vec!["sk-abcdef123456".into()],
            api_key_legacy: None,
            base_url: "https://api.openai.com/v1".into(),
            available_models: vec!["gpt-5".into()],
            enabled_models: vec!["gpt-5".into()],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        };
        let request = GenerateRequest {
            model: "gpt-5".into(),
            system: "sys".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            options: Default::default(),
            metadata: crate::chat::model::RequestMetadata {
                label: "Chat API".into(),
                conversation_id: Some("conv_1".into()),
                ..Default::default()
            },
        };
        let output = GenerateOutput {
            canonical_message: None,
            billing_usage: None,
            text: "hello".into(),
            reasoning: None,
            tool_calls: vec![PendingToolCall {
                id: "call_1".into(),
                function_name: "web_search".into(),
                arguments: serde_json::json!({ "q": "rust" }),
                arguments_raw: "{\"q\":\"rust\"}".into(),
                arguments_parse_error: None,
                signature: None,
            }],
            usage: Some(ModelUsage {
                total_tokens: Some(42),
                ..Default::default()
            }),
            finish_reason: Some("tool_calls".into()),
            provider_messages: Vec::new(),
            cancelled: false,
            web_search: None,
            images: Vec::new(),
            reasoning_items: Vec::new(),
        };

        let mut headers = BTreeMap::new();
        headers.insert(
            "Authorization".into(),
            format!("Bearer {}", provider.api_keys[0]),
        );
        let headers = sanitize_headers(headers);

        let record = build_debug_record(DebugRecordArgs {
            provider: &provider,
            request: &request,
            label: "Chat API",
            started_at: 100,
            duration_ms: 250,
            status: "success",
            url: "https://api.openai.com/v1/chat/completions".into(),
            headers,
            body: serde_json::json!({ "model": "gpt-5" }),
            stream: false,
            response: RequestDebugResponse::from_output(&output, Some(200)),
        });

        // Metadata derivation.
        assert!(record.id.starts_with("dbg_"));
        assert_eq!(record.provider_id, "test");
        assert_eq!(record.model, "gpt-5");
        assert_eq!(record.source, "chat");
        assert_eq!(record.operation, "chat_api");
        assert_eq!(record.conversation_id.as_deref(), Some("conv_1"));
        assert_eq!(record.status, "success");
        // Header is sanitized — no full key.
        assert_eq!(record.request.headers["Authorization"], "Bearer sk-a…");
        assert!(!serde_json::to_string(&record)
            .unwrap()
            .contains("abcdef123456"));
        // Response summary carries tool calls + usage + finish reason.
        assert_eq!(record.response.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(record.response.text.as_deref(), Some("hello"));
        let calls = record
            .response
            .tool_calls
            .as_array()
            .expect("tool calls array");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["name"], "web_search");
        assert_eq!(record.response.usage.and_then(|u| u.total_tokens), Some(42));
    }
}
