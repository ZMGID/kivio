//! Shared durable effects of a completed agent run and retained-history replay.
use crate::chat::types::{CompactionBoundaryRecord, Conversation, ConversationContextSummary};
use serde_json::Value;

/// Apply compaction against the prior summary before replacing it, so accumulated
/// source IDs, file evidence and compression counts survive across turns.
pub fn apply_compaction(
    conversation: &mut Conversation,
    boundary: Option<CompactionBoundaryRecord>,
    summary: Option<ConversationContextSummary>,
) {
    if let Some(boundary) = boundary {
        conversation
            .context_state
            .compaction_boundaries
            .push(boundary);
    }
    // L2 压缩对齐落盘路径：run 结束时把 L2 产出的 summary 写回 context_state.summary +
    // compression_count（不再只 push boundary）。质量兜底已在 compaction 核心拦截，此处直接采用。
    if let Some(mut summary) = summary {
        // L2 产出的 summary.source_message_ids 为空（运行时侧拿不到完整 UI id 列表）——
        // 在此按 source_until_message_id 从 conversation 累积（含旧 summary 覆盖范围），
        // 与落盘路径 compact_conversation_inner 口径一致。必须在替换 summary **之前**读旧 S1。
        summary.source_message_ids = crate::chat::agent::compaction::accumulate_source_ids(
            conversation,
            &summary.source_until_message_id,
        );
        // Files-touched ledger: recompute cumulatively here (this L2 summary was
        // built on the runtime Value stream, which lacks the Conversation). Same
        // recompute as the disk path, so neither clobbers the other's ledger.
        let ledger = crate::chat::agent::file_ledger::build_for_boundary(
            conversation,
            &summary.source_until_message_id,
        );
        summary.file_ledger = (!ledger.is_empty()).then_some(ledger);
        conversation.context_state.last_compressed_at = Some(summary.created_at);
        conversation.context_state.compressed_message_count = summary.source_message_ids.len();
        conversation.context_state.compression_count = conversation
            .context_state
            .compression_count
            .saturating_add(1);
        conversation.context_state.summary = Some(summary);
        // R-4：多次链式压缩后提示准确性下降（与 compact_conversation 口径一致）。
        conversation.context_state.warning = crate::chat::agent::compaction::decay_warning_for(
            conversation.context_state.compression_count,
        );
    }
}

pub fn generated_image_artifacts(
    images: &[crate::chat::model::GeneratedImageData],
) -> Vec<crate::mcp::types::ChatToolArtifact> {
    images
        .iter()
        .enumerate()
        .map(|(idx, image)| {
            let extension = extension_for_mime(&image.mime_type);
            crate::mcp::types::ChatToolArtifact {
                id: None,
                name: format!("generated-image-{}.{}", idx + 1, extension),
                mime_type: image.mime_type.clone(),
                data_url: format!("data:{};base64,{}", image.mime_type, image.data),
                size_bytes: decoded_base64_len(&image.data),
                path: None,
            }
        })
        .collect()
}

pub(crate) fn decoded_base64_len(value: &str) -> Option<u64> {
    let compact_len = value.chars().filter(|ch| !ch.is_whitespace()).count();
    if compact_len == 0 {
        return Some(0);
    }
    let padding = value
        .chars()
        .rev()
        .take_while(|ch| ch.is_whitespace() || *ch == '=')
        .filter(|ch| *ch == '=')
        .count()
        .min(2);
    Some(((compact_len * 3) / 4).saturating_sub(padding) as u64)
}

pub(crate) fn extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        _ => "png",
    }
}

/// Return the first uncovered UI-message index and the exact provider prefix.
/// Stale/missing summaries and context-clear boundaries use the shared projection.
pub fn replay_prefix(conversation: &Conversation) -> (usize, Vec<Value>) {
    let start = super::context_projection::context_replay_start_index(conversation);
    let mut prefix = Vec::new();
    if let Some(summary) = super::context_projection::active_summary(conversation) {
        prefix.push(summary_message(summary));
        if let Some(replay) = &summary.replay {
            prefix.extend(
                replay
                    .messages
                    .iter()
                    .filter(|message| super::compaction::is_replayable(message))
                    .cloned(),
            );
        }
    }
    (start, prefix)
}

fn summary_message(summary: &ConversationContextSummary) -> Value {
    let mut content = format!(
        "{}\n{}",
        crate::chat::agent::compaction::PERSISTED_SUMMARY_PREFIX,
        summary.content.trim()
    );
    // Deterministic files-touched ledger, rendered under the LLM summary as a
    // factual floor (budget-isolated; never fed to the summarizer).
    if let Some(ledger) = &summary.file_ledger {
        let block = crate::chat::agent::file_ledger::render_block(ledger);
        if !block.is_empty() {
            content.push_str("\n\n");
            content.push_str(&block);
        }
    }
    // User role, as in the run that produced it (ZCode keeps the same shape): a
    // system message would be hoisted into the system prompt, leaving a retained
    // tail that starts with an assistant tool call as the first provider message.
    serde_json::json!({
        "role": "user",
        "content": content,
    })
}
