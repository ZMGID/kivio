//! Budgeting, request projections and bounded summary compaction.
mod estimate;
mod image;
mod media;
mod policy;
mod summary;
use crate::agent::session::CompactionPlan;
use crate::model::{GenerateOptions, ModelCapabilities, RequestMetadata};
pub use estimate::*;
pub use image::*;
pub use media::{prune_image_parts, prune_runtime_images, IMAGE_BYTES_BUDGET};
pub use policy::*;
use serde_json::Value;
pub use summary::{
    auto_compact_budget, has_compactable_history, request_context_budget, summarize_history,
    PERSISTED_SUMMARY_PREFIX, SUMMARY_OUTPUT_TOKENS,
};
#[doc(hidden)]
pub use summary::{
    group_starts, initial_kept_groups, project_summary_media, skill_reload_reminders,
    summary_message, summary_text, tool_history_as_text,
};

#[derive(Clone)]
pub struct SummaryOptions {
    pub provider_id: String,
    pub model: String,
    pub window: usize,
    pub options: GenerateOptions,
    pub capabilities: Option<ModelCapabilities>,
    pub metadata: RequestMetadata,
    pub automatic: bool,
}
impl SummaryOptions {
    pub fn new(provider_id: impl Into<String>, model: impl Into<String>, window: usize) -> Self {
        Self {
            provider_id: provider_id.into(),
            model: model.into(),
            window,
            options: GenerateOptions {
                max_tokens: SUMMARY_OUTPUT_TOKENS,
                thinking_enabled: false,
                ..GenerateOptions::default()
            },
            capabilities: None,
            metadata: RequestMetadata {
                label: "Chat context compaction".into(),
                ..RequestMetadata::default()
            },
            automatic: true,
        }
    }
}

pub enum CompactOutcome {
    Compacted(CompactionPlan, String),
    Skipped,
    Cancelled,
    Failed,
}

/// Host metadata probes and domain context that cannot be inferred from model messages.
pub trait SummaryProjection: Send + Sync {
    fn estimate_request_tokens(&self, messages: &[Value], model: &str) -> usize {
        estimate_request_tokens(messages, model)
    }
    /// Additional historical context placed beside the summary. Retained messages stay untouched.
    fn supplemental(&self, _old: &[Value], _kept: &[Value], _budget: usize) -> Vec<Value> {
        Vec::new()
    }
}

pub fn estimate_request_tokens(messages: &[Value], model: &str) -> usize {
    estimate_messages_tokens(messages).saturating_add(
        messages
            .iter()
            .map(|m| image_tokens(m, model))
            .sum::<usize>(),
    )
}
fn image_tokens(value: &Value, model: &str) -> usize {
    match value {
        Value::Array(values) => values.iter().map(|v| image_tokens(v, model)).sum(),
        Value::Object(object)
            if object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| IMAGE_PART_TYPES.contains(&kind)) =>
        {
            let detail = object
                .get("image_url")
                .and_then(|image| image.get("detail"))
                .or_else(|| object.get("detail"))
                .and_then(Value::as_str);
            estimate_image_tokens(None, model, None, detail)
        }
        Value::Object(object) => object.values().map(|v| image_tokens(v, model)).sum(),
        _ => 0,
    }
}
