use std::path::PathBuf;

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::chat::agent::prepare as agent_prepare;
use crate::chat::model::openai_messages_from_model_messages;
use crate::chat::model_metadata::context_window_for_model;
use crate::chat::storage::load_conversation;
use crate::chat::{
    ChatMessage, CompactionBoundaryRecord, ContextClearBoundaryRecord, ContextUsageSegment,
    Conversation, ConversationContextState, ConversationContextSummary,
};
use crate::external_agents::detection::{
    EXTERNAL_AGENT_MODELS_CACHE_TTL, EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
};
use crate::settings::ModelProvider;
use crate::state::AppState;

use super::catalog::strip_transcripts_for_frontend;
use super::sanitization::{sanitize_api_message_for_model, sanitize_image_payloads_for_model};
use super::image_content_part;

#[tauri::command]
pub(crate) async fn chat_get_context_stats(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    let context_state = if conversation.agent_runtime.is_external() {
        let cwd = crate::external_agents::workspace::resolve_effective_cwd(
            &app,
            &conversation.id,
            conversation.project_id.as_deref(),
        )?;
        crate::external_agents::context::compute_external_context_state_with_probe(
            &conversation,
            true,
            None,
            None,
            Some(&cwd),
            Some(&cwd),
        )
        .await
    } else {
        compute_context_state(&app, &state, &conversation, None, &[]).await?
    };
    conversation = persist_context_state_best_effort(
        &app,
        &conversation_id,
        conversation,
        context_state.clone(),
    )
    .await?;
    let context_state = conversation.context_state.clone();
    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "contextState": crate::chat::protocol::ChatContextStatePayload::from(&context_state),
        "conversation": conversation,
    }))
}

/// Cache statistics only against the snapshot used to compute them. A conflict
/// returns the latest state so a stale refresh cannot overwrite a new summary.
pub(super) async fn persist_context_state_best_effort(
    app: &AppHandle,
    conversation_id: &str,
    conversation: crate::chat::Conversation,
    context_state: crate::chat::ConversationContextState,
) -> Result<crate::chat::Conversation, String> {
    let repository = crate::chat::repository::repository(app);
    match repository
        .update_context(
            app,
            conversation_id,
            conversation.revision,
            context_state.clone(),
        )
        .await
    {
        Ok(updated) => Ok(updated),
        Err(crate::chat::repository::ConversationRepositoryError::Conflict { .. }) => {
            let latest = repository
                .get(app, conversation_id)
                .await
                .map_err(crate::chat::repository::repository_error)?;
            Ok(latest)
        }
        Err(err) => Err(crate::chat::repository::repository_error(err)),
    }
}

#[tauri::command]
pub(crate) async fn chat_compress_context(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    if conversation.agent_runtime.is_external() {
        crate::external_agents::compact::request_external_compaction(
            &app,
            &state,
            &mut conversation,
        )
        .await?;
        let context_state_after_compact = conversation.context_state.clone();
        // 同 `chat_get_context_stats`：压缩**已经发生**了，落盘缓存抢不到版本不该报错。
        conversation = persist_context_state_best_effort(
            &app,
            &conversation_id,
            conversation,
            context_state_after_compact.clone(),
        )
        .await?;
        // 用**压缩后算出来的**那份，不能读回 `conversation.context_state`：落盘被让位时
        // 上面返回的是重新读到的会话，它身上还是压缩前的状态。
        let context_state = context_state_after_compact;
        emit_chat_context_state(
            &app,
            &conversation.id,
            conversation.revision,
            &context_state,
        );
        strip_transcripts_for_frontend(&mut conversation);
        return Ok(serde_json::json!({
            "success": true,
            "contextState": crate::chat::protocol::ChatContextStatePayload::from(&context_state),
            "conversation": conversation,
        }));
    }
    let _reservation =
        super::reply_runtime::ChatSendReservation::try_acquire(state.inner(), &conversation_id)
            .ok_or(super::reply_runtime::CHAT_REPLY_BUSY_ERROR)?;
    let generation = state.chat_runtime().begin_generation(&conversation_id);
    let run_id = format!("compact-{}", uuid::Uuid::new_v4());
    let _generation_guard = super::reply_runtime::ChatReplyGuard::try_new(
        state.inner(),
        &conversation_id,
        &run_id,
        generation,
    )
    .ok_or(super::reply_runtime::CHAT_REPLY_BUSY_ERROR)?;
    // Reload after admission, then CAS the summary and retained messages as one update.
    conversation = load_conversation(&app, &conversation_id)?;
    let compacted = tokio::select! {
        result = compress_conversation_context(&app, &state, &mut conversation) => result?,
        _ = super::interaction::wait_for_chat_cancel(state.inner(), &conversation_id, generation) => return Err("压缩已停止".into()),
    };
    if !compacted {
        return Err("近期上下文仍在保留预算内，没有可安全压缩的旧历史".into());
    }
    if !state
        .chat_runtime()
        .is_generation_active(&conversation_id, generation)
    {
        return Err("压缩已停止".into());
    }
    finalize_local_context_change(
        &app,
        &state,
        &conversation_id,
        conversation,
        Some(generation),
    )
    .await
}

#[tauri::command]
pub(crate) async fn chat_clear_context(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let mut conversation = load_conversation(&app, &conversation_id)?;
    apply_context_clear(&mut conversation)?;
    finalize_local_context_change(&app, &state, &conversation_id, conversation, None).await
}

pub(super) fn invalidate_context_measurement(state: &AppState, conversation: &mut Conversation) {
    let context = &mut conversation.context_state;
    state.chat_runtime().seed_context_measurement(&conversation.id, context.lifecycle_id, context.measurement_seq, context.request_measurement.as_ref());
    let live = state.chat_runtime().invalidate_context_display(&conversation.id);
    context.measurement_seq = live.seq;
    context.lifecycle_id = live.lifecycle_id;
    context.request_measurement = Some(live.stored());
    context.reported_context_tokens = None;
    context.token_count_source = None;
    context.session_input_tokens = None;
    context.session_output_tokens = None;
    context.usage_ratio = None;
    context.segments.clear();
    context.status = "unknown".into();
}

/// Local context mutations share one compute → persist → event → response path.
/// `generation` marks a manual compaction; its result survives revision bumps that
/// leave the summarized history intact (statistics refreshes, renames).
async fn finalize_local_context_change(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation_id: &str,
    mut conversation: Conversation,
    generation: Option<u64>,
) -> Result<serde_json::Value, String> {
    invalidate_context_measurement(state, &mut conversation);
    let context_state = compute_context_state(app, state, &conversation, None, &[]).await?;
    conversation.context_state = context_state.clone();
    if generation.is_some_and(|generation| {
        !state
            .chat_runtime()
            .is_generation_active(conversation_id, generation)
    }) {
        return Err("压缩已停止".into());
    }
    let repository = crate::chat::repository::repository(app);
    let mut expected_revision = conversation.revision;
    let mut attempts = 0;
    conversation = loop {
        match repository
            .update_context(
                app,
                conversation_id,
                expected_revision,
                context_state.clone(),
            )
            .await
        {
            Ok(updated) => break updated,
            Err(crate::chat::repository::ConversationRepositoryError::Conflict { .. })
                if generation.is_some() && attempts < 3 =>
            {
                attempts += 1;
                let latest = repository
                    .get(app, conversation_id)
                    .await
                    .map_err(crate::chat::repository::repository_error)?;
                if !super::messages::history_unchanged(&latest, &conversation, None) {
                    return Err("压缩期间会话已变化，原有历史保持不变，请重新压缩".into());
                }
                expected_revision = latest.revision;
            }
            Err(err) => return Err(crate::chat::repository::repository_error(err)),
        }
    };
    emit_chat_context_state(app, &conversation.id, conversation.revision, &context_state);
    strip_transcripts_for_frontend(&mut conversation);
    Ok(serde_json::json!({
        "success": true,
        "contextState": crate::chat::protocol::ChatContextStatePayload::from(&context_state),
        "conversation": conversation,
    }))
}

pub(super) fn apply_context_clear(conversation: &mut Conversation) -> Result<(), String> {
    if conversation.agent_runtime.is_external() {
        return Err("清空上下文仅支持 Kivio Agent 和 Kivio Chat".to_string());
    }
    let last_id = conversation
        .messages
        .last()
        .map(|message| message.id.clone())
        .ok_or_else(|| "没有可清空的上下文".to_string())?;
    if conversation.context_clear_until_index() == Some(conversation.messages.len() - 1) {
        return Err("当前已是空上下文".to_string());
    }
    let created_at = chrono::Local::now().timestamp();
    conversation
        .context_state
        .clear_boundaries
        .push(ContextClearBoundaryRecord {
            id: format!("ctxclr_{}", uuid::Uuid::new_v4()),
            source_until_message_id: last_id,
            created_at,
        });
    // Drop the live summary: it describes history that is no longer in the
    // replay window, and later compaction must not chain from it.
    conversation.context_state.summary = None;
    conversation.context_state.compressed_message_count = 0;
    conversation.context_state.warning = None;
    Ok(())
}


pub(crate) fn active_summary(conversation: &Conversation) -> Option<&ConversationContextSummary> {
    let summary = conversation
        .context_state
        .summary
        .as_ref()
        .filter(|summary| !summary.stale)
        .filter(|summary| !summary.content.trim().is_empty())?;
    let through = summary
        .replay
        .as_ref()
        .map(|r| r.through_message_id.as_str())
        .unwrap_or(&summary.source_until_message_id);
    let until_idx = conversation
        .messages
        .iter()
        .position(|message| message.id == through)?;
    if let Some(clear_idx) = conversation.context_clear_until_index() {
        if until_idx <= clear_idx {
            return None;
        }
    }
    Some(summary)
}

fn summary_boundary_index(conversation: &Conversation) -> Option<usize> {
    let summary = active_summary(conversation)?;
    conversation.messages.iter().position(|message| {
        message.id
            == summary
                .replay
                .as_ref()
                .map(|r| r.through_message_id.as_str())
                .unwrap_or(&summary.source_until_message_id)
    })
}

/// First UI-message index that still belongs in the live model context.
/// Messages before this are either discarded by a context clear or covered by
/// an active compaction summary.
pub(crate) fn context_replay_start_index(conversation: &Conversation) -> usize {
    let clear_start = conversation.context_clear_start_index();
    let summary_start = summary_boundary_index(conversation)
        .map(|idx| idx + 1)
        .unwrap_or(0);
    clear_start.max(summary_start)
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

fn prune_clear_boundaries_if_needed(conversation: &mut Conversation) {
    conversation
        .context_state
        .clear_boundaries
        .retain(|boundary| {
            conversation
                .messages
                .iter()
                .any(|message| message.id == boundary.source_until_message_id)
                || conversation
                    .messages
                    .iter()
                    .any(|message| message.timestamp > boundary.created_at)
        });
}

pub(super) fn mark_summary_stale_if_needed(conversation: &mut Conversation, changed_index: usize) {
    prune_clear_boundaries_if_needed(conversation);
    conversation.invalidate_summary_from(changed_index);
}

pub(super) fn count_tokens_in_value(value: &Value) -> usize {
    // 口径统一：委托压缩侧共用的 estimate_value_tokens（图片部件记 0，文本按文本，
    // 对象递归）。曾经两处各写一份，压缩侧漏了图片归零导致 base64 打爆估算。
    agent_prepare::estimate_value_tokens(value)
}

fn ceil_div_u32(value: u32, divisor: u32) -> usize {
    value.div_ceil(divisor) as usize
}

fn estimate_openai_tile_image_tokens(
    width: u32,
    height: u32,
    base_tokens: usize,
    tile_tokens: usize,
) -> usize {
    let mut scaled_width = width.max(1) as f64;
    let mut scaled_height = height.max(1) as f64;
    let longest = scaled_width.max(scaled_height);
    if longest > 2048.0 {
        let scale = 2048.0 / longest;
        scaled_width *= scale;
        scaled_height *= scale;
    }
    let shortest = scaled_width.min(scaled_height);
    if shortest > 768.0 {
        let scale = 768.0 / shortest;
        scaled_width *= scale;
        scaled_height *= scale;
    }
    let tiles = (scaled_width / 512.0).ceil().max(1.0) as usize
        * (scaled_height / 512.0).ceil().max(1.0) as usize;
    base_tokens + tiles * tile_tokens
}

fn estimate_openai_patch_image_tokens(
    width: u32,
    height: u32,
    patch_budget: Option<usize>,
    multiplier: f64,
    max_dimension: u32,
) -> usize {
    // Match OMP's order: fit pixel dimensions first, then the patch budget.
    let mut width = width.max(1) as f64;
    let mut height = height.max(1) as f64;
    let longest = width.max(height);
    if longest > max_dimension as f64 {
        let scale = max_dimension as f64 / longest;
        width = (width * scale).round().max(1.0);
        height = (height * scale).round().max(1.0);
    }
    let mut patches = (width / 32.0).ceil() as usize * (height / 32.0).ceil() as usize;
    if let Some(budget) = patch_budget.filter(|budget| patches > *budget) {
        let shrink = (budget as f64 * 32.0 * 32.0 / (width * height)).sqrt();
        let scaled_width = width * shrink / 32.0;
        let scaled_height = height * shrink / 32.0;
        let adjusted = shrink
            * (scaled_width.floor() / scaled_width).min(scaled_height.floor() / scaled_height);
        let resized_width = (width * adjusted).floor() as u32;
        let resized_height = (height * adjusted).floor() as u32;
        // Extreme aspect ratios can round one side to zero; use the budget,
        // not a zero-token image or an iterative one-percent shrink loop.
        patches = if resized_width == 0 || resized_height == 0 {
            budget
        } else {
            (ceil_div_u32(resized_width, 32) * ceil_div_u32(resized_height, 32)).min(budget)
        };
    }
    (patches as f64 * multiplier).ceil() as usize
}

fn estimate_anthropic_image_tokens(model: &str, width: u32, height: u32) -> usize {
    let lower = model.to_ascii_lowercase();
    let high_resolution_opus = lower.contains("opus")
        && (lower.contains("4.7")
            || lower.contains("4-7")
            || lower.contains("4.8")
            || lower.contains("4-8"));
    let cap = if high_resolution_opus { 4_784 } else { 1_600 };
    ((width.max(1) as f64 * height.max(1) as f64) / 750.0)
        .ceil()
        .min(cap as f64) as usize
}

fn estimate_gemini_image_tokens(width: u32, height: u32) -> usize {
    if width <= 384 && height <= 384 {
        return 258;
    }
    let tiles = ceil_div_u32(width.max(1), 768) * ceil_div_u32(height.max(1), 768);
    tiles.max(1) * 258
}

/// Display measurement of the exact request, including model-specific image costs.
pub(super) fn measure_request_segments(
    messages: &[Value],
    tools: &[crate::mcp::ChatToolDefinition],
    provider: Option<&ModelProvider>,
    model: &str,
) -> Vec<ContextUsageSegment> {
    let mut segments = crate::chat::agent::context_measure::measure_prepared_request(messages, tools);
    let image_tokens = messages.iter().map(|message| request_image_tokens(message, provider, model)).sum::<usize>();
    if image_tokens > 0 {
        if let Some(conversation) = segments.iter_mut().find(|segment| segment.id == "conversation") {
            conversation.estimated_tokens += image_tokens;
        } else {
            segments.push(ContextUsageSegment {
                id: "conversation".into(), label: "Conversation".into(),
                estimated_tokens: image_tokens, chars: 0, color: None,
            });
        }
    }
    segments
}

/// Decode only the image header, following OMP's bounded metadata probe.
fn image_header_dimensions(data: &str) -> Option<(u32, u32)> {
    use base64::Engine as _;
    const HEADER_BASE64_CHARS: usize = 65_536_usize.div_ceil(3) * 4;
    let count = data.len().min(HEADER_BASE64_CHARS) / 4 * 4;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data.as_bytes()[..count]).ok()?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format().ok()?.into_dimensions().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

fn image_url_dimensions(url: &str) -> Option<(u32, u32)> {
    let (metadata, data) = url.strip_prefix("data:")?.split_once(',')?;
    metadata.ends_with(";base64").then(|| image_header_dimensions(data)).flatten()
}

/// Estimate request image parts, never their encoded length or remote contents.
fn request_image_tokens(value: &Value, provider: Option<&ModelProvider>, model: &str) -> usize {
    match value {
        Value::Array(values) => values.iter().map(|value| request_image_tokens(value, provider, model)).sum(),
        Value::Object(object) if object.get("type").and_then(Value::as_str)
            .is_some_and(|kind| agent_prepare::IMAGE_PART_TYPES.contains(&kind)) => {
            let image = object.get("image_url");
            let source = object.get("source");
            let detail = image.and_then(|image| image.get("detail"))
                .or_else(|| object.get("detail")).and_then(Value::as_str);
            let url = image.and_then(|image| image.as_str().or_else(|| image.get("url")?.as_str()))
                .or_else(|| object.get("dataUrl")?.as_str())
                .or_else(|| source?.get("url")?.as_str());
            let dimensions = if let Some(url) = url {
                image_url_dimensions(url)
            } else {
                source.and_then(|source| source.get("data"))
                    .or_else(|| object.get("data")).and_then(Value::as_str)
                    .and_then(image_header_dimensions)
            };
            estimate_image_tokens(provider, model, dimensions, detail)
        }
        Value::Object(object) => object.values().map(|value| request_image_tokens(value, provider, model)).sum(),
        _ => 0,
    }
}

/// Model/detail rules apply equally to decoded images and unknown-size budgets.
/// OMP supplies the sizing approach; model parameters follow OpenAI's vision guide.
pub(super) fn estimate_image_tokens(
    provider: Option<&ModelProvider>,
    model: &str,
    dimensions: Option<(u32, u32)>,
    detail: Option<&str>,
) -> usize {
    // Avoid allocating a lowercased provider/model descriptor for every image.
    let identifies = |name: &str| {
        let contains = |value: &str| value.as_bytes().windows(name.len())
            .any(|part| part.eq_ignore_ascii_case(name.as_bytes()));
        contains(model) || provider.is_some_and(|provider| {
            contains(&provider.name) || contains(&provider.base_url) || contains(&provider.api_format)
        })
    };
    if identifies("anthropic") || identifies("claude") {
        return dimensions.map_or_else(
            || estimate_anthropic_image_tokens(model, u32::MAX, u32::MAX),
            |(width, height)| estimate_anthropic_image_tokens(model, width, height),
        );
    }
    if identifies("gemini") || identifies("google") {
        return dimensions.map_or(1_600, |(width, height)| estimate_gemini_image_tokens(width, height));
    }

    let patch = |budget: Option<usize>, multiplier: f64, max_dimension: u32| {
        dimensions.map_or_else(
            // Original detail on newer models has a rejection limit, not a
            // resizing budget. Unknown inputs use the largest accepted count.
            || (budget.unwrap_or(30_000) as f64 * multiplier).ceil() as usize,
            |(width, height)| estimate_openai_patch_image_tokens(
                width, height, budget, multiplier, max_dimension,
            ),
        )
    };
    let astra = identifies("gpt-6-astra");
    let extended_original = astra || identifies("gpt-5.6") || identifies("gpt-5-6");
    let v55 = identifies("gpt-5.5") || identifies("gpt-5-5");
    let v54 = identifies("gpt-5.4") || identifies("gpt-5-4");
    if extended_original || v55 || v54 {
        return match detail.unwrap_or("auto") {
            "low" if v54 => patch(Some(6_144), 1.2, 2_048),
            "low" => patch(Some(256), 1.2, 512),
            "high" => patch(Some(2_500), 1.2, if astra { 65_535 } else { 2_048 }),
            "auto" if v54 => patch(Some(2_500), 1.2, 2_048),
            _ if extended_original => patch(None, 1.2, 65_535),
            _ => patch(Some(10_000), 1.2, 6_000),
        };
    }
    if identifies("gpt-5.2") || identifies("gpt-5-2") {
        return patch(Some(6_144), 1.2, 2_048);
    }
    if identifies("gpt-4.1-mini") || identifies("gpt-4-1-mini") {
        return patch(Some(6_144), 1.62, 2_048);
    }
    if identifies("gpt-4.1-nano") || identifies("gpt-4-1-nano") {
        return patch(Some(1_536), 2.46, 2_048);
    }
    if identifies("gpt-5-mini") {
        return patch(Some(1_536), 1.2, 2_048);
    }
    if identifies("gpt-5-nano") {
        return patch(Some(1_536), 1.5, 2_048);
    }
    if identifies("o4-mini") {
        return patch(Some(1_536), 1.72, 2_048);
    }
    let (base, tile) = if identifies("gpt-4o-mini") {
        (2_833, 5_667)
    } else if identifies("gpt-5") {
        (70, 140)
    } else if identifies("o1") || identifies("o3") {
        (75, 150)
    } else if identifies("computer-use") {
        (65, 129)
    } else {
        (85, 170)
    };
    if detail == Some("low") {
        return base;
    }
    // A 2048px long side and at most 768px short side cover at most eight tiles.
    dimensions.map_or(base + 8 * tile, |(width, height)| {
        estimate_openai_tile_image_tokens(width, height, base, tile)
    })
}

/// 解析会话的真实用量锚点：从尾部找最近一条带 `anchor_usage` 且 provider 与当前一致的 assistant。
/// 返回 `(anchor_total_tokens, trailing_estimate)`：
/// - `anchor_total_tokens` = 该 assistant 上次调用「整个 prompt + 该次响应」的真实 token 总数
///   （含 output，按 provider 家族消歧，见 `context_estimate::anchor_total_tokens`）；
/// - `trailing_estimate` = 该 assistant **之后**（不含它本身，其 output 已计入锚点）到末尾所有消息的估算。
///
/// provider 与 `conversation.provider_id` 不一致（切换过供应商，计数口径不可比）、锚点消息之后发生过
/// 压缩（消息序列已变，旧计数失真，R4）或无 usage → `(None, 0)`，调用方回落纯字符估算。
/// 对齐 `context_estimate::effective_context_tokens` 的锚点口径。
pub(super) fn resolve_usage_anchor(
    conversation: &Conversation,
    provider: Option<&ModelProvider>,
) -> (Option<u64>, usize) {
    let Some(provider) = provider else {
        return (None, 0);
    };
    let api_format = provider.api_format.as_str();
    // 压缩边界失效（R4）：锚点消息生成后若发生过压缩（自动/手动），其记录的 token 数反映的是压缩前的
    // 完整历史，与压缩后实际发送的 prompt 不再可比——锚点作废。取最晚一次压缩时刻，任何时间戳 ≤ 该时刻的
    // assistant 锚点都视为失真（run 内自动压缩后仍会生成更晚的 assistant，其 anchor_usage 是压缩后调用
    // 值、时间戳晚于边界，不受影响）。
    let latest_compaction_at = conversation
        .context_state
        .compaction_boundaries
        .iter()
        .map(|b| b.created_at)
        .max();
    let anchor = conversation
        .messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, message)| {
            if message.role != "assistant" || group_answer_excluded_from_context(conversation, message) {
                return None;
            }
            if conversation
                .context_clear_until_index()
                .is_some_and(|clear_idx| idx <= clear_idx)
            {
                return None;
            }
            if active_summary(conversation)
                .and_then(|s| s.replay.as_ref())
                .and_then(|r| {
                    conversation
                        .messages
                        .iter()
                        .position(|m| m.id == r.through_message_id)
                })
                .is_some_and(|end| idx < end || (idx == end
                    && !conversation.context_state.compaction_boundaries.last()
                        .is_some_and(|boundary| matches!(boundary.trigger.as_str(), "agent_loop" | "reactive"))))
            {
                return None;
            }
            let usage = message.anchor_usage.as_ref()?;
            // Missing identity in legacy records cannot prove that a model switch
            // preserved the tokenizer/context. Wait for a fresh reported request.
            if message.provider_id.as_deref() != Some(provider.id.as_str())
                || message.model.as_deref() != Some(conversation.model.as_str())
            {
                return None;
            }
            // 压缩后锚点失真（R4）：边界晚于锚点消息 → 作废（回落纯估算）。
            if let Some(compacted_at) = latest_compaction_at {
                if compacted_at > message.timestamp {
                    return None;
                }
            }
            let tokens = crate::chat::agent::context_estimate::anchor_total_tokens(usage, api_format);
            tokens.map(|total| (idx, total))
        });
    match anchor {
        Some((idx, total)) => {
            // trailing = 锚点消息**之后**的消息（锚点消息本身的 output 已计入 total，故 idx+1..）。
            let trailing = conversation.messages[idx + 1..]
                .iter()
                .map(crate::chat::agent::compaction::estimate_chat_message_tokens)
                .sum();
            (Some(total), trailing)
        }
        None => (None, 0),
    }
}

pub(super) fn conversation_cache_usage(
    conversation: &Conversation,
    _settings: &crate::settings::Settings,
) -> Option<(u64, u64)> {
    let clear_until = conversation.context_clear_until_index();
    let (mut input, mut read) = (0u64, 0u64);
    for (idx, message) in conversation.messages.iter().enumerate() {
        if message.role != "assistant"
            || message.id.starts_with("subagent-result-")
            || clear_until.is_some_and(|end| idx <= end)
            || group_answer_excluded_from_context(conversation, message)
        {
            continue;
        }
        let Some((next_input, next_read)) = message.cache_pair_input.zip(message.cache_pair_read)
            .filter(|(input, read)| *input > 0 && read <= input) else { continue };
        input = input.saturating_add(next_input);
        read = read.saturating_add(next_read);
    }
    (input > 0).then_some((input, read))
}

pub(super) async fn compute_context_state(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &Conversation,
    _last_user_api_content: Option<&str>,
    _last_user_image_paths: &[PathBuf],
) -> Result<ConversationContextState, String> {
    if conversation.agent_runtime.is_external() {
        // 缓存 key 必须与写入方 chat_detect_external_agent_models 一致：探测 cwd
        // （resolve_detection_cwd，非项目会话 = __global__），否则该读取恒 miss。
        let model_cache_key =
            crate::external_agents::workspace::resolve_detection_cwd(app, Some(&conversation.id))
                .ok()
                .and_then(|cwd| {
                    conversation
                        .agent_runtime
                        .external_agent_id
                        .as_deref()
                        .map(|agent_id| {
                            crate::external_agents::slash::cache_key(
                                agent_id,
                                cwd.to_string_lossy().as_ref(),
                            )
                        })
                });
        let cached_models = model_cache_key.as_deref().and_then(|cache_key| {
            state.external_discovery().get_cached_external_agent_models(
                cache_key,
                EXTERNAL_AGENT_MODELS_CACHE_TTL,
                EXTERNAL_AGENT_MODELS_FALLBACK_TTL,
            )
        });
        // 执行 cwd（resolve_effective_cwd，每会话独立 workspace）与上面的探测 cwd 是两回事：
        // 它只用于按 workDir 关联 kimi 落盘的 wire.jsonl（见 kimi_usage 模块）。拿不到不影响其余。
        let work_dir = crate::external_agents::workspace::resolve_effective_cwd(
            app,
            &conversation.id,
            conversation.project_id.as_deref(),
        )
        .ok();
        return Ok(
            crate::external_agents::context::compute_external_context_state_with_probe(
                conversation,
                false,
                None,
                cached_models.as_ref().map(|c| c.models.as_slice()),
                None,
                work_dir.as_deref(),
            )
            .await,
        );
    }

    let settings = state.settings_read().clone();
    let provider = settings.get_provider(&conversation.provider_id).cloned();
    let live = state.chat_runtime().context_measurement(&conversation.id);
    let measurement = crate::chat::agent::context_measure::resolve_display(conversation, live.as_ref());
    let reported_context_tokens = measurement.reported_tokens;
    let segments = measurement.segments;
    let estimated_input_tokens = segments.iter().map(|segment| segment.estimated_tokens).sum();
    let mut cache = conversation_cache_usage(conversation, &settings);
    if let Some(live) = live.as_ref().filter(|live| {
        !conversation.messages.iter().any(|message| message.id == live.message_id
            && message.cache_pair_input.is_some())
    }) {
        crate::chat::agent::context_measure::add_cache_pairs(&mut cache, live.run_cache);
    }
    let cache_hit_rate = cache.map(|(input, read)| read as f64 / input as f64);
    let (context_window_tokens, context_window_estimated) =
        context_window_for_model(provider.as_ref(), &conversation.model);
    let usage_ratio = reported_context_tokens.and_then(|tokens| {
        (context_window_tokens > 0).then(|| tokens as f32 / context_window_tokens as f32)
    });
    let summary = conversation.context_state.summary.clone();
    let status = context_status(usage_ratio, summary.as_ref());
    let last_compressed_at = summary
        .as_ref()
        .filter(|summary| !summary.stale)
        .map(|summary| summary.created_at)
        .or(conversation.context_state.last_compressed_at);
    let compressed_message_count = summary
        .as_ref()
        .filter(|summary| !summary.stale)
        .map(|summary| summary.source_message_ids.len())
        .unwrap_or_default();
    let mut compression_count = conversation.context_state.compression_count;
    if compression_count == 0 && active_summary(conversation).is_some() {
        compression_count = 1;
    }

    Ok(ConversationContextState {
        estimated_input_tokens,
        context_window_tokens: Some(context_window_tokens),
        context_window_estimated,
        auto_compact_threshold_tokens: Some(crate::chat::agent::compaction::auto_compact_budget(
            context_window_tokens,
        )),
        usage_ratio,
        status,
        segments,
        reported_context_tokens,
        cache_hit_rate,
        last_measured_at: chrono::Local::now().timestamp(),
        last_compressed_at,
        compressed_message_count,
        compression_count,
        summary,
        compaction_boundaries: conversation.context_state.compaction_boundaries.clone(),
        clear_boundaries: conversation.context_state.clear_boundaries.clone(),
        warning: conversation.context_state.warning.clone(),
        context_source: Some(crate::external_agents::context::CONTEXT_SOURCE_BUILTIN.to_string()),
        token_count_source: reported_context_tokens.map(|_| "provider_context_reported".to_string()),
        session_input_tokens: None,
        session_output_tokens: None,
        external_agent_id: None,
        external_model: None,
        measurement_seq: measurement.seq,
        lifecycle_id: measurement.lifecycle_id,
        request_measurement: measurement.persist,
    })
}

pub(super) async fn compress_conversation_context(
    app: &AppHandle,
    state: &State<'_, AppState>,
    conversation: &mut Conversation,
) -> Result<bool, String> {
    let settings = state.settings_read().clone();
    crate::chat::agent::compaction::compact_conversation(
        app,
        state.inner(),
        &settings,
        conversation,
    )
    .await
}

pub(super) fn emit_chat_context_state(
    app: &AppHandle,
    conversation_id: &str,
    revision: u64,
    context_state: &ConversationContextState,
) {
    crate::chat::protocol::emit_conversation_event(
        app,
        conversation_id,
        revision,
        crate::chat::protocol::ChatConversationEvent::ContextUpdated {
            context_state: context_state.into(),
        },
    );
}

/// **生成过程中**的上下文占用（分子 + 分母）。
///
/// Main-request API reports update occupancy. Categories are attached once per request.
/// Live events are not written to disk. Compression sends a source-less invalidation,
/// never an estimated occupancy. External CLI keeps its own contract.
///
/// `context_window_tokens` 为 `None` = 本次上报没带窗口，**前端必须保留已知的旧值**
/// （分母粘滞，见 `applyLiveContextUsage`）。
pub(crate) fn emit_chat_context_usage_live(
    app: &AppHandle,
    _conversation_id: &str,
    run_id: &str,
    used_tokens: u64,
    token_count_source: Option<&str>,
    context_window_tokens: Option<u64>,
    cache_usage: Option<(u64, u64)>,
    measurement_seq: u64,
    lifecycle_id: u64,
    segments: Option<&[ContextUsageSegment]>,
) {
    crate::chat::protocol::emit_run_event(
        app,
        run_id,
        crate::chat::protocol::ChatRunEvent::ContextUsageUpdated {
            usage: crate::chat::protocol::ChatContextUsagePayload {
                used_tokens,
                token_count_source: token_count_source.map(str::to_string),
                context_window_tokens,
                cache_input_tokens: cache_usage.map(|(input, _)| input),
                cache_read_tokens: cache_usage.map(|(_, read)| read),
                measurement_seq,
                lifecycle_id,
                segments: segments.map(|segments| {
                    segments
                        .iter()
                        .map(|segment| crate::chat::protocol::ChatContextUsageSegmentPayload {
                            id: segment.id.clone(),
                            label: segment.label.clone(),
                            estimated_tokens: segment.estimated_tokens as u64,
                            chars: segment.chars as u64,
                            color: segment.color.clone(),
                        })
                        .collect()
                }),
            },
        },
    );
}

pub(super) fn emit_chat_compaction_state(
    app: &AppHandle,
    _conversation_id: &str,
    run_id: &str,
    phase: &str,
    trigger: Option<&str>,
    boundary: Option<&CompactionBoundaryRecord>,
) {
    crate::chat::protocol::emit_run_event(
        app,
        run_id,
        crate::chat::protocol::ChatRunEvent::CompactionUpdated {
            phase: phase.to_string(),
            trigger: trigger.map(str::to_string),
            boundary: boundary.map(Into::into),
        },
    );
}

fn context_status(
    usage_ratio: Option<f32>,
    summary: Option<&ConversationContextSummary>,
) -> String {
    if summary.is_some_and(|item| item.stale) {
        return "stale".to_string();
    }
    if summary.is_some() {
        return "compressed".to_string();
    }
    let Some(ratio) = usage_ratio else {
        return "unknown".to_string();
    };
    if ratio >= 0.95 {
        "critical".to_string()
    } else if ratio >= 0.70 {
        "warning".to_string()
    } else {
        "normal".to_string()
    }
}

// Display categories are measured from the prepared request, not rebuilt here.
// This builder is still what the model receives, so group filtering stays here.

/// 多答组（任务 06-30）历史过滤：判断某条带 `group_id` 的 assistant 消息是否应排除出上下文。
/// 规则（决策 D5）：同一 `group_id` 只保留「选中条」——
/// - `conversation.group_selections[group_id]` 指定的 message_id；
/// - 无记录则取该组在 `messages` 中**顺序第一条** assistant。
/// 其余答案仅保留展示、排除出发给模型的历史（R6）。非多答消息（无 group_id）一律保留。
/// `pub(crate)`：落盘压缩（compaction.rs）复用同一谓词，保证摘要输入与 replay 视图口径一致。
pub(crate) fn group_answer_excluded_from_context(
    conversation: &Conversation,
    message: &ChatMessage,
) -> bool {
    let Some(group_id) = message.group_id.as_deref() else {
        return false;
    };
    if message.role != "assistant" {
        return false;
    }
    let selected = conversation
        .group_selections
        .get(group_id)
        .map(String::as_str)
        .or_else(|| {
            // 无显式选择时，优先保留该组第一条「非错误」assistant 进上下文，跳过
            // stream_outcome == "error" 的失败臂——否则失败臂的错误文案会作为上一轮
            // 答案回灌给模型。全组皆 error（罕见）时才退回顺序第一条。
            let in_group =
                |m: &&ChatMessage| m.role == "assistant" && m.group_id.as_deref() == Some(group_id);
            conversation
                .messages
                .iter()
                .find(|m| in_group(m) && m.stream_outcome.as_deref() != Some("error"))
                .or_else(|| conversation.messages.iter().find(in_group))
                .map(|m| m.id.as_str())
        });
    selected != Some(message.id.as_str())
}

/// 给一条 runtime 消息标注来源 UI 消息 id（`_ui_message_id`）。
/// 该字段只存在于运行期视图：发给 provider 前会经 `model_message_from_openai_message`
/// 只抽取已知字段，未知字段天然被剥离，不会进任何 wire 请求。压缩落盘时
/// `compaction::source_until_message_id_for_split` 据此把 runtime 旧段精确映射回 UI 消息。
fn tag_ui_message_id(mut message: Value, ui_message_id: &str) -> Value {
    if let Some(obj) = message.as_object_mut() {
        obj.insert(
            "_ui_message_id".to_string(),
            Value::String(ui_message_id.to_string()),
        );
    }
    message
}

/// `app` 为 `None` 时跳过图片 rehydrate（纯估算调用方不需要真实 base64——token 口径
/// 本来就不计图片字节）。真正要发给模型的路径必须传 `Some`。
pub(crate) fn build_chat_api_messages(
    app: Option<&AppHandle>,
    system_prompt: &str,
    conversation: &Conversation,
    last_user_idx: Option<usize>,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
) -> Result<Vec<Value>, String> {
    build_chat_api_messages_with_video(
        app,
        system_prompt,
        conversation,
        last_user_idx,
        last_user_api_content,
        last_user_image_paths,
        true,
    )
}

/// Text-only replay must not reopen or encode historical videos just to discard them later.
pub(crate) fn build_chat_api_messages_with_video(
    app: Option<&AppHandle>,
    system_prompt: &str,
    conversation: &Conversation,
    last_user_idx: Option<usize>,
    last_user_api_content: Option<&str>,
    last_user_image_paths: &[PathBuf],
    include_video: bool,
) -> Result<Vec<Value>, String> {
    let mut messages = vec![serde_json::json!({
        "role": "system",
        "content": system_prompt,
    })];

    // Replay the summary plus its exact retained body, then append only UI messages
    // beyond the snapshot. Legacy summaries still use their source boundary.
    // A context clear invalidates snapshots at or before its boundary.
    let start_idx = context_replay_start_index(conversation);
    let mut snapshot_reports = std::collections::HashSet::new();
    if let Some(summary) = active_summary(conversation) {
        messages.push(summary_message(summary));
        if let Some(replay) = &summary.replay {
            let mut tail: Vec<Value> = replay
                .messages
                .iter()
                .filter(|m| crate::chat::agent::compaction::is_replayable(m))
                .cloned()
                .collect();
            snapshot_reports.extend(
                tail.iter()
                    .filter_map(|m| {
                        m[crate::chat::sub_agent::control::REPORT_MESSAGE_ID_KEY].as_str()
                    })
                    .map(str::to_string),
            );
            if let Some(app) = app {
                crate::chat::attachments::rehydrate_api_message_images(
                    app,
                    &conversation.id,
                    &mut tail,
                );
            }
            messages.extend(tail);
        }
    }

    let mut remaining_video_bytes = crate::chat::video::MAX_VIDEO_BYTES;
    for (idx, message) in conversation.messages.iter().enumerate() {
        if idx < start_idx {
            continue;
        }
        // 多答组：仅保留选中条，其余答案不进发给模型的上下文（R6 / AC4）。
        if group_answer_excluded_from_context(conversation, message) {
            continue;
        }
        // The snapshot already carries this child report at the point the run saw it.
        if snapshot_reports.contains(&message.id) {
            continue;
        }
        let content = if Some(idx) == last_user_idx {
            last_user_api_content.unwrap_or(message.content.as_str())
        } else {
            message.content.as_str()
        };
        let sanitized_content = sanitize_image_payloads_for_model(content);
        if message.role == "assistant" && message.id.starts_with("subagent-result-") {
            messages.push(tag_ui_message_id(
                crate::chat::sub_agent::control::report_input(&sanitized_content),
                &message.id,
            ));
            continue;
        }
        let mut parts = Vec::new();
        if message.role == "user" {
            for attachment in &message.attachments {
                // Extension fallback supports videos saved by older versions as ordinary files.
                if crate::chat::video::mime_for_name(&attachment.name).is_some() {
                    if !include_video {
                        parts.push(serde_json::json!({"type": "text", "text": format!(
                            "[Video attachment: {} [{}]. Raw video is not included. Match saved observations by attachment ID, not filename. If the current question requires unrecorded video details, call mixer_video_analysis when available. Do not ask the user to select an analysis mode or type a command. If the tool is unavailable, explain that video understanding is unavailable. Never invent contents.]",
                            attachment.name, attachment.id
                        )}));
                        continue;
                    }
                    let Some(app) = app else { continue };
                    let path = crate::chat::attachments::resolve_attachment_file_path(
                        app,
                        Some(&conversation.id),
                        &attachment.path,
                    )?;
                    parts.push(crate::chat::video::content_part(
                        &path,
                        &mut remaining_video_bytes,
                    )?);
                }
            }
        }
        if Some(idx) == last_user_idx && !last_user_image_paths.is_empty() {
            parts.extend(
                last_user_image_paths
                    .iter()
                    .map(image_content_part)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        if !parts.is_empty() {
            parts.push(serde_json::json!({ "type": "text", "text": sanitized_content }));
            messages.push(tag_ui_message_id(
                serde_json::json!({
                    "role": message.role,
                    "content": parts,
                }),
                &message.id,
            ));
        } else {
            messages.push(tag_ui_message_id(
                serde_json::json!({
                    "role": message.role,
                    "content": sanitized_content,
                }),
                &message.id,
            ));
        }
        if message.role == "assistant" && !message.model_messages.is_empty() {
            messages.pop();
            // 落盘的图片部件只有 path，没 base64（见 attachments::externalize_model_message_images）。
            // 展开成 wire 格式之前先按 path 读盘填回来，模型看到的内容与当初一字不差。
            let mut model_messages = message.model_messages.clone();
            if let Some(app) = app {
                crate::chat::attachments::rehydrate_model_message_images(
                    app,
                    &conversation.id,
                    &mut model_messages,
                );
            }
            messages.extend(
                openai_messages_from_model_messages(&model_messages)
                    .iter()
                    .map(sanitize_api_message_for_model)
                    .map(|expanded| tag_ui_message_id(expanded, &message.id)),
            );
        } else if message.role == "assistant" && !message.api_messages.is_empty() {
            messages.pop();
            // 与上面 model_messages 同理：中断草稿的 `api_messages` 里图片已被外置成
            // `kivio-attachment://` 哨兵，发给模型前必须还原成 data URL。
            let mut api_messages = message.api_messages.clone();
            if let Some(app) = app {
                crate::chat::attachments::rehydrate_api_message_images(
                    app,
                    &conversation.id,
                    &mut api_messages,
                );
            }
            messages.extend(
                api_messages
                    .iter()
                    .map(sanitize_api_message_for_model)
                    .map(|expanded| tag_ui_message_id(expanded, &message.id)),
            );
        }
    }

    Ok(messages)
}
