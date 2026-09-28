//! Context compaction, following ZCode's structured-history and assistant-round policy.
//! This module owns selection, summary requests, retries and replacement. UI history is retained.
use super::loop_::{LoopEnv, RunState};
use super::planning::call_chat_completion_message_streamed;
use super::prepare::{estimate_tokens, estimate_value_tokens, IMAGE_PART_TYPES, TEXT_PART_TYPES};
use crate::chat::model::{MessagePart, ModelMessage};
use crate::chat::model_metadata::{chat_max_output_tokens_for_model, context_window_for_model};
use crate::chat::types::{
    ChatMessage, CompactionBoundaryRecord, CompactionReplay, Conversation,
    ConversationContextSummary,
};
use crate::settings::Settings;
use crate::state::AppState;
use serde_json::{json, Value};

pub(crate) const PERSISTED_SUMMARY_PREFIX: &str = "Previous conversation summary:";
pub(crate) const UI_MESSAGE_ID_KEY: &str = "_ui_message_id";
const SUMMARY_MARKER_PREFIX: &str = "[context summary]";
const SUMMARY_OUTPUT_TOKENS: u32 = 20_000;
const MAX_SUMMARY_ATTEMPTS: usize = 3;
const SUMMARY_PROMPT: &str = include_str!("compaction_prompt.txt");

/// Reserve output capacity and a fixed safety margin, matching ZCode's policy.
pub(crate) fn auto_compact_budget(window: usize, max_output: u32) -> usize {
    let window = if window == 0 { 200_000 } else { window };
    let output = if max_output == 0 {
        32_000
    } else {
        max_output as usize
    };
    window
        .saturating_sub(output.min(21_000))
        .saturating_sub(13_000)
}

/// Groups begin at assistant messages, keeping every tool call beside its results.
/// A synthetic previous summary belongs to the history, never to the immutable prefix
/// (replayed summaries are user messages; the system-role check stays defensive).
fn group_starts(messages: &[Value]) -> (usize, Vec<usize>) {
    let prefix = messages
        .iter()
        .take_while(|m| {
            m["role"] == "system"
                && !m["content"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with(PERSISTED_SUMMARY_PREFIX)
        })
        .count();
    let mut groups = Vec::new();
    for (i, message) in messages.iter().enumerate().skip(prefix) {
        if groups.is_empty() || message["role"] == "assistant" {
            groups.push(i);
        }
    }
    (prefix, groups)
}

fn summary_text(raw: &str) -> Option<String> {
    let text = if let Some((_, rest)) = raw.split_once("<summary>") {
        rest.split_once("</summary>")?.0
    } else if let Some((_, rest)) = raw.split_once("</analysis>") {
        rest
    } else if raw.contains("<analysis>") {
        return None;
    } else {
        raw
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn summary_message(text: &str) -> Value {
    json!({"role":"user", "content":format!(
        "{SUMMARY_MARKER_PREFIX}\nThis session continues from earlier conversation. The summary covers the earlier history.\n\n{text}\n\nResume the user's unfinished task directly. Do not acknowledge or recap this summary. Preserve the user's instructions and constraints.")})
}

/// Snapshot only the replacement body; fresh system/skill instructions are rebuilt on replay.
pub(crate) fn replacement_body(messages: &[Value]) -> Vec<Value> {
    let start = messages
        .iter()
        .position(|m| {
            m["content"]
                .as_str()
                .is_some_and(|s| s.starts_with(SUMMARY_MARKER_PREFIX))
        })
        .map(|i| i + 1);
    start
        .map(|i| {
            messages[i..]
                .iter()
                .filter(|m| is_replayable(m))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// A system message after the summary is a run-scoped notice (e.g. the tool-round
/// limit). Providers hoist every system message into the system prompt, so replaying
/// one would impose it on all later turns. Snapshots saved before this rule may hold one.
pub(crate) fn is_replayable(message: &Value) -> bool {
    message["role"] != "system"
}

fn read_reminders(old: &[Value], kept: &[Value]) -> Vec<Value> {
    let mut reads = std::collections::HashMap::new();
    let mut candidates = Vec::new();
    for message in old {
        if let Some(calls) = message["tool_calls"].as_array() {
            for call in calls {
                if call["function"]["name"] == "read" {
                    let args = call["function"]["arguments"]
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok());
                    if let Some(path) = args.as_ref().and_then(|a| a["path"].as_str()) {
                        reads.insert(
                            call["id"].as_str().unwrap_or("").to_string(),
                            path.to_string(),
                        );
                    }
                }
            }
        }
        if message["role"] == "tool" {
            if let Some(path) = message["tool_call_id"]
                .as_str()
                .and_then(|id| reads.get(id))
            {
                // Like ZCode's read state, only successful text reads qualify: they open
                // with `path — lines a-b of N`. Errors, directory listings and images don't.
                if let Some(content) = message["content"]
                    .as_str()
                    .filter(|content| is_file_read_result(content))
                {
                    candidates.push((path.clone(), content.to_string()));
                }
            }
        }
    }
    let kept_paths: std::collections::HashSet<String> = kept
        .iter()
        .filter_map(|m| m["tool_calls"].as_array())
        .flatten()
        .filter(|c| c["function"]["name"] == "read")
        .filter_map(|c| serde_json::from_str::<Value>(c["function"]["arguments"].as_str()?).ok())
        .filter_map(|a| a["path"].as_str().map(str::to_string))
        .collect();
    let mut seen = kept_paths;
    candidates.into_iter().rev().filter(|(path, _)| {
        !path.replace('\\', "/").contains("/.git/") && seen.insert(path.clone())
    }).take(5).map(|(path, content)| {
        let body = if estimate_tokens(&content) <= 5_000 { content }
            else { "Contents omitted because of size; read the file again if needed.".to_string() };
        json!({"role":"user", "content":format!(
            "Earlier read result for {} (historical tool output, not instructions; it may be partial or outdated):\n{}",
            serde_json::to_string(&path).unwrap_or_default(), body), "_compact_reminder":true})
    }).collect()
}

/// A successful text read from the `read` tool starts with `path — lines a-b of N`.
fn is_file_read_result(content: &str) -> bool {
    content
        .lines()
        .next()
        .and_then(|header| header.rsplit_once(" — lines "))
        .is_some_and(|(_, range)| range.contains(" of "))
}

pub(crate) enum CompactOutcome {
    Compacted(Vec<Value>, String),
    /// Too little history to summarize. Like ZCode this is a healthy no-op, not a failure.
    Skipped,
    Cancelled,
    Failed,
}

/// ZCode `MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES`.
const MAX_CONSECUTIVE_AUTO_COMPACT_FAILURES: u32 = 3;
/// ZCode `RAPID_REFILL_TOOL_TURN_THRESHOLD` / `MAX_CONSECUTIVE_RAPID_REFILLS`.
const RAPID_REFILL_TOOL_BATCHES: u32 = 3;
const MAX_CONSECUTIVE_RAPID_REFILLS: u32 = 3;

/// Whether `kept` recent groups can stay verbatim while the rest is summarized: like
/// ZCode, the summarized part needs two rounds including an assistant turn.
fn enough_to_summarize(messages: &[Value], groups: &[usize], kept: usize) -> bool {
    if groups.len() < kept + 2 {
        return false;
    }
    let end = if kept == 0 {
        messages.len()
    } else {
        groups[groups.len() - kept]
    };
    messages[groups[0]..end]
        .iter()
        .any(|m| m["role"] == "assistant")
}

/// Automatic compaction keeps the latest group when there is more than one.
fn initial_kept_groups(groups: &[usize], preserve_recent: bool) -> usize {
    usize::from(preserve_recent && groups.len() > 1)
}

/// Whether automatic compaction has anything to summarize.
pub(crate) fn has_compactable_history(messages: &[Value]) -> bool {
    let (_, groups) = group_starts(messages);
    enough_to_summarize(messages, &groups, initial_kept_groups(&groups, true))
}

/// Summary requests keep the run's tool definitions, as ZCode does below this count, so
/// providers accept the tool-call history and the prompt cache can be reused.
const COMPACT_TOOL_KEEP_MAX_COUNT: usize = 100;
/// Inserted when a truncated summary request would otherwise open with an assistant turn.
const TRUNCATION_MARKER: &str = "[earlier conversation truncated for compaction retry]";

/// One engine for manual, automatic and overflow compaction. Cancellation remains
/// borrowed across every attempt, including media and context-overflow retries.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn summarize_history(
    state: &AppState,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: &[Value],
    preserve_recent: bool,
    tools: &[crate::mcp::ChatToolDefinition],
    max_output: u32,
    conversation_id: &str,
    message_id: &str,
    mut cancel: Option<super::provider_runtime::ProviderFuture<'_, ()>>,
    host: Option<&dyn super::host::AgentHost>,
) -> CompactOutcome {
    let (prefix, groups) = group_starts(messages);
    let group_end = |kept: usize| {
        if kept == 0 {
            messages.len()
        } else {
            groups[groups.len() - kept]
        }
    };
    let enough = |kept: usize| enough_to_summarize(messages, &groups, kept);
    let initial_kept = initial_kept_groups(&groups, preserve_recent);
    if !enough(initial_kept) {
        return CompactOutcome::Skipped;
    }
    let mut tools = if tools.len() > COMPACT_TOOL_KEEP_MAX_COUNT {
        &[][..]
    } else {
        tools
    };
    let mut kept_groups = initial_kept;
    let mut dropped_groups = 0;
    let mut failures = 0;
    let mut overflow_retries = 0;
    let mut strip_media = false;
    loop {
        let end = group_end(kept_groups);
        let start = groups[dropped_groups];
        let mut request = messages[..prefix].to_vec();
        if dropped_groups > 0 && messages[start]["role"] == "assistant" {
            request.push(json!({"role":"user", "content": TRUNCATION_MARKER}));
        }
        request.extend_from_slice(&messages[start..end]);
        if strip_media {
            strip_media_parts(&mut request);
        }
        if tools.is_empty() {
            tool_history_as_text(&mut request);
        }
        request.push(json!({"role":"user", "content": SUMMARY_PROMPT}));
        let call = call_chat_completion_message_streamed(
            state,
            provider,
            model,
            request,
            (!tools.is_empty()).then_some(tools),
            1,
            false,
            if max_output == 0 {
                SUMMARY_OUTPUT_TOKENS
            } else {
                max_output.min(SUMMARY_OUTPUT_TOKENS)
            },
            conversation_id,
            message_id,
            "Chat context compaction",
        );
        let result = if let Some(ref mut cancellation) = cancel {
            tokio::select! { result = call => result, _ = cancellation.as_mut() => return CompactOutcome::Cancelled }
        } else {
            call.await
        };
        let error = match result {
            Ok(message) => {
                let finish = message["finish_reason"].as_str().unwrap_or("");
                let raw = super::stop::assistant_content_from_api_message(&message);
                // GLM-style providers end an overlong summary request with an empty `length`.
                let empty_length =
                    matches!(finish, "length" | "max_tokens") && raw.trim().is_empty();
                if super::recovery::is_context_overflow_finish(finish) || empty_length {
                    "Compaction summary context length exceeded".to_string()
                } else if matches!(
                    finish,
                    "length" | "max_tokens" | "cancelled" | "content_filter"
                ) {
                    return CompactOutcome::Failed;
                } else if message["tool_calls"]
                    .as_array()
                    .is_some_and(|calls| !calls.is_empty())
                {
                    // Never execute tools emitted by the summary model.
                    return CompactOutcome::Failed;
                } else if let Some(text) = summary_text(&raw) {
                    let kept = &messages[end..];
                    let mut compacted = messages[..prefix].to_vec();
                    compacted.push(summary_message(&text));
                    compacted.extend_from_slice(kept);
                    compacted.extend(read_reminders(&messages[prefix..end], kept));
                    return CompactOutcome::Compacted(compacted, text);
                } else {
                    "Empty or incomplete compaction summary".to_string()
                }
            }
            Err(error) => error,
        };
        let lower = error.to_lowercase();
        // A provider without tool support rejects the definitions; send the history as text.
        if !tools.is_empty() && super::stop::is_tools_unsupported_error(&error) {
            tools = &[];
            continue;
        }
        if super::recovery::classify(&error) == super::recovery::FailureKind::ContextOverflow {
            if preserve_recent {
                // Move complete recent groups out of the summary input, without dropping them.
                if !enough(kept_groups + 1) {
                    return CompactOutcome::Failed;
                }
                kept_groups += 1;
                if let Some(host) = host {
                    host.emit_compaction_status(
                        conversation_id,
                        "retrying",
                        Some("agent_loop"),
                        None,
                    );
                }
                continue;
            }
            if overflow_retries < 3 && dropped_groups < groups.len() - 1 {
                dropped_groups += ((groups.len() - dropped_groups) / 5).max(1);
                dropped_groups = dropped_groups.min(groups.len() - 1);
                overflow_retries += 1;
                continue;
            }
            return CompactOutcome::Failed;
        }
        if !strip_media
            && (lower.contains("image") || lower.contains("media"))
            && (lower.contains("too large") || lower.contains("size"))
        {
            strip_media = true;
            continue;
        }
        failures += 1;
        if !preserve_recent
            || failures >= MAX_SUMMARY_ATTEMPTS
            || lower.contains("401")
            || lower.contains("403")
            || lower.contains("api key")
        {
            eprintln!("Context compaction failed: {error}");
            return CompactOutcome::Failed;
        }
        // Like ZCode, each automatic retry starts again from the initial selection.
        kept_groups = initial_kept;
        strip_media = false;
        if let Some(host) = host {
            host.emit_compaction_status(conversation_id, "retrying", Some("agent_loop"), None);
        }
    }
}

/// Bound image bytes like a normal turn and drop images for a model known to lack vision.
fn project_summary_media(messages: &mut [Value], supports_vision: Option<bool>) {
    prune_image_parts(messages, IMAGE_BYTES_BUDGET);
    if supports_vision == Some(false) {
        strip_media_parts(messages);
    }
}

fn strip_media_parts(messages: &mut [Value]) {
    for message in messages {
        if message["content"].is_array() {
            message["content"] = json!(render_multimodal_content(&message["content"]));
        }
    }
}

/// Some providers reject tool-call history in a request without tool definitions, so
/// such a summary request carries the calls and their results as plain text.
fn tool_history_as_text(messages: &mut [Value]) {
    for message in messages.iter_mut() {
        let content = match &message["content"] {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => render_multimodal_content(other),
        };
        if message["role"] == "tool" {
            *message = json!({"role":"user", "content": format!("[Tool result]\n{content}")});
            continue;
        }
        let calls = message["tool_calls"]
            .as_array()
            .filter(|calls| !calls.is_empty())
            .map(|calls| {
                calls
                    .iter()
                    .map(|call| {
                        format!(
                            "[Tool call] {} {}",
                            call["function"]["name"].as_str().unwrap_or_default(),
                            call["function"]["arguments"].as_str().unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        if let Some(calls) = calls {
            let text = if content.trim().is_empty() {
                calls
            } else {
                format!("{content}\n{calls}")
            };
            *message = json!({"role":"assistant", "content": text});
        }
    }
}

pub(crate) fn decay_warning_for(count: usize) -> Option<String> {
    (count >= 3).then(|| format!("This conversation has been compressed {count} times; summaries may omit earlier details."))
}

/// Manual compaction of the whole live history. Returns `false` when there is too little
/// history to summarize. The summary and exact remaining body are persisted together by
/// the conversation CAS.
pub(crate) async fn compact_conversation(
    app: &tauri::AppHandle,
    state: &AppState,
    settings: &Settings,
    conversation: &mut Conversation,
) -> Result<bool, String> {
    let (provider_id, model) =
        settings.effective_compression_model_for_session(Some(crate::settings::SessionModel {
            provider_id: &conversation.provider_id,
            model: &conversation.model,
        }));
    let provider = settings
        .get_provider(&provider_id)
        .ok_or("Compression provider not found")?;
    // The compression model may differ from the chat model. As in ZCode, the summary
    // request follows the same media policy as a turn: no raw video, bounded images,
    // and no images at all for a model known to lack vision.
    let mut messages = crate::chat::commands::context::build_chat_api_messages_with_video(
        Some(app),
        "",
        conversation,
        None,
        None,
        &[],
        false,
    )?;
    project_summary_media(
        &mut messages,
        crate::chat::model_metadata::model_supports_vision(Some(provider), &model),
    );
    let until = conversation
        .messages
        .last()
        .ok_or("没有足够的历史可以压缩")?
        .id
        .clone();
    let outcome = summarize_history(
        state,
        provider,
        &model,
        &messages,
        false,
        &[],
        chat_max_output_tokens_for_model(Some(provider), &model).unwrap_or(SUMMARY_OUTPUT_TOKENS),
        &conversation.id,
        &until,
        None,
        None,
    )
    .await;
    let (compacted, text) = match outcome {
        CompactOutcome::Compacted(compacted, text) => (compacted, text),
        CompactOutcome::Skipped => return Ok(false),
        CompactOutcome::Cancelled | CompactOutcome::Failed => {
            return Err("上下文压缩失败，原有历史保持不变".into())
        }
    };
    let created_at = chrono::Local::now().timestamp();
    let before = estimate_messages_tokens(&messages);
    let after = estimate_messages_tokens(&compacted);
    let source_ids = accumulate_source_ids(conversation, &until);
    let ledger = super::file_ledger::build_for_boundary(conversation, &until);
    conversation.context_state.summary = Some(ConversationContextSummary {
        id: format!("ctxsum_{}", uuid::Uuid::new_v4()),
        content: text.clone(),
        source_message_ids: source_ids.clone(),
        source_until_message_id: until.clone(),
        token_estimate_before: before,
        token_estimate_after: estimate_tokens(&text),
        created_at,
        provider_id,
        model,
        stale: false,
        file_ledger: (!ledger.is_empty()).then_some(ledger),
        replay: Some(CompactionReplay {
            through_message_id: until.clone(),
            messages: replacement_body(&compacted),
        }),
    });
    conversation.context_state.last_compressed_at = Some(created_at);
    conversation.context_state.compressed_message_count = source_ids.len();
    conversation.context_state.compression_count += 1;
    conversation
        .context_state
        .compaction_boundaries
        .push(CompactionBoundaryRecord {
            id: format!("ctxbd_{}", uuid::Uuid::new_v4()),
            source_until_message_id: until.clone(),
            display_after_message_id: Some(until),
            token_estimate_before: before,
            token_estimate_after: after,
            summary_content: text,
            trigger: "manual".to_string(),
            created_at,
        });
    conversation.context_state.warning =
        decay_warning_for(conversation.context_state.compression_count);
    Ok(true)
}

pub(crate) fn estimate_messages_tokens(messages: &[Value]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// 单条消息的 token 估算（与 `estimate_messages_tokens` 的逐条逻辑一致，供近期窗口选取复用）。
/// content 为多模态数组时走 `estimate_value_tokens`（图片记 0，不把 base64 体积算进 token）；
/// reasoning_content 计入（与 `serialize_message` 口径一致）。
fn estimate_message_tokens(message: &Value) -> usize {
    let tool_calls = message
        .get("tool_calls")
        .map(|calls| estimate_tokens(&calls.to_string()))
        .unwrap_or(0);
    let reasoning = super::prepare::estimate_message_reasoning_tokens(message);
    let content = match message.get("content") {
        Some(Value::String(text)) => estimate_tokens(text),
        Some(other) => estimate_value_tokens(other),
        None => 0,
    };
    content + tool_calls + reasoning + 4
}

/// 图片部件在摘要序列化中的占位符（不灌 base64）。
const IMAGE_PART_PLACEHOLDER: &str = "[image attachment omitted]";

/// 重复图片被去重后留在原位的占位文本。
const DUPLICATE_IMAGE_PLACEHOLDER: &str = "[与后文同一张图片，此处省略，不重复上传]";

/// 发送视图里所有图片 base64 的总字节预算。超出后从**最旧**的图片开始换占位符。
///
/// 与入口降采样（`chat/image_prep.rs`）分工：入口把每张图收敛到 ≤2000px / ≤2MB 原始
/// 字节（base64 ≈ 2.7MB，典型只有几百 KB），所以这里是**保险不是主力**——16MB 对齐
/// pi 的溢出恢复预算（Anthropic 32MB 请求体上限的一半），装得下几十张典型缩放图。
/// 曾经是 4MB：入口不缩放时它每轮必砍，会把模型**上一轮刚读、还没看到**的图挤出
/// 上下文，模型按占位符提示重读 → 再挤掉别的图，原地绕圈（实测于电商 9 图核验会话）。
/// 参照 Codex 的实测事故（openai/codex#28316）：无预算时图片 base64 反复重放把请求
/// 打到 8.34MB+，多家 OpenAI 兼容中转直接 502/524 或返回空流——预算本身必须保留。
///
/// ponytail: 固定常量，不做设置项。真有人需要不同额度再提成 `chat_tools` 配置。
const IMAGE_BYTES_BUDGET: usize = 16 * 1024 * 1024;

/// 收敛发送视图里的图片体积：**倒序**（新→旧）遍历，重复的图片只留最新那份，
/// 累计 base64 字节超过 [`IMAGE_BYTES_BUDGET`] 后把更早的图片换成占位文本。
/// 返回省下的字节数。
///
/// **为什么非做不可**：`read` 读到图片会把整张 base64 作为 follow-up user 消息永久留在
/// 历史里（`vision.rs::read_image_as_tool_result`）。同一张图被读两次就是两份 84KB
/// base64——实测占了请求体的 74%，而且每个 planning 轮都完整重传一次，几十轮下来几 MB，
/// 第三方中转直接在传输途中把流掐断。
///
/// 而 `estimate_message_tokens` 是**故意**不计图片 base64 的（token 口径上一张图约
/// 千把 token，不是 8 万字符），所以压缩层根本看不见这份体积，不会触发。字节层面的
/// 重复与堆积只能在这里单独处理。
///
/// 倒序遍历一次同时表达了两条规则：重复图留最新那份（模型当下在看的就是它），
/// 超预算时淘汰最旧的（对当前一步价值最低）。对齐 Claude Code 用户在要的
/// 「按年龄丢图」与 Strands Agents 的「图片换带元信息占位符」。
///
/// ponytail: 指纹直接用 part 的 JSON 串哈希——同一张图序列化必然逐字节相同，
/// 不用解 data URL、不用管 `image_url` / `input_image` / `image` 三种形状的差异。
fn prune_image_parts(messages: &mut [Value], budget: usize) -> usize {
    use std::collections::HashSet;
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut seen: HashSet<u64> = HashSet::new();
    let mut kept_bytes = 0usize;
    let mut saved = 0usize;
    for message in messages.iter_mut().rev() {
        let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts.iter_mut() {
            let Some(kind) = part.get("type").and_then(Value::as_str) else {
                continue;
            };
            if !IMAGE_PART_TYPES.contains(&kind) {
                continue;
            }
            let serialized = part.to_string();
            let bytes = serialized.len();
            let mut hasher = DefaultHasher::new();
            serialized.hash(&mut hasher);
            if !seen.insert(hasher.finish()) {
                saved += bytes;
                *part = json!({ "type": "text", "text": DUPLICATE_IMAGE_PLACEHOLDER });
                continue;
            }
            if kept_bytes.saturating_add(bytes) > budget {
                saved += bytes;
                *part = json!({
                    "type": "text",
                    "text": format!(
                        "[较早的图片已从上下文移除以控制请求体积（约 {}KB）。需要时请重新读取该文件。]",
                        bytes / 1024
                    ),
                });
                continue;
            }
            kept_bytes += bytes;
        }
    }
    saved
}

/// 把多模态 content（数组 parts / 单对象）渲染成摘要文本：文本部件取全文、图片部件换占位符、
/// 未知部件退回其 JSON（保守不丢信息）。
fn render_multimodal_content(content: &Value) -> String {
    match content {
        Value::Array(parts) => parts
            .iter()
            .map(render_content_part)
            .collect::<Vec<_>>()
            .join(" "),
        other => render_content_part(other),
    }
}

fn render_content_part(part: &Value) -> String {
    if part.get("type").and_then(Value::as_str) == Some("video_url") {
        return "[video attachment omitted]".into();
    }
    if let Some(kind) = part.get("type").and_then(Value::as_str) {
        if IMAGE_PART_TYPES.contains(&kind) {
            return IMAGE_PART_PLACEHOLDER.to_string();
        }
        if TEXT_PART_TYPES.contains(&kind) {
            return part
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
    }
    part.to_string()
}

fn estimate_model_messages_tokens(messages: &[ModelMessage]) -> usize {
    messages
        .iter()
        .map(|message| {
            let native_reasoning: usize = message
                .content
                .iter()
                .filter_map(|part| match part {
                    MessagePart::ReasoningItem { item, .. } => {
                        Some(super::prepare::estimate_reasoning_item_tokens(item))
                    }
                    _ => None,
                })
                .sum();
            let parts: usize = message
                .content
                .iter()
                .map(|part| match part {
                    MessagePart::Text { text } => estimate_tokens(text),
                    MessagePart::Reasoning { text } => {
                        if native_reasoning == 0 {
                            estimate_tokens(text)
                        } else {
                            0
                        }
                    }
                    MessagePart::ToolCall {
                        name,
                        arguments_raw,
                        ..
                    } => estimate_tokens(name) + estimate_tokens(arguments_raw),
                    MessagePart::ToolResult { content, .. } => estimate_tokens(content),
                    // 图片部件记 0（与 estimate_value_tokens 同口径，不把 base64 算进 token）。
                    // reasoning item 同理：encrypted_content 是密文 base64，按字符估算会
                    // 数倍虚高；其真实占用由 usage 锚点覆盖。
                    MessagePart::Image { .. }
                    | MessagePart::Video { .. }
                    | MessagePart::ImageUrl { .. }
                    | MessagePart::ReasoningItem { .. } => 0,
                })
                .sum();
            parts + native_reasoning + 4
        })
        .sum()
}

/// 把单条 UI `ChatMessage` 估算成 token 数。**优先按展开形态**（model_messages /
/// api_messages）估算——真实 replay 发给模型的是这些里的完整工具转录，而非截断的
/// `result_preview`；分支顺序与 `build_chat_api_messages` 的展开路径同源对齐。
/// 无展开数据时退回 content + reasoning + 工具入参 + 结果预览口径。
pub(crate) fn estimate_chat_message_tokens(message: &ChatMessage) -> usize {
    if !message.model_messages.is_empty() {
        return estimate_model_messages_tokens(&message.model_messages);
    }
    if !message.api_messages.is_empty() {
        return message
            .api_messages
            .iter()
            .map(estimate_message_tokens)
            .sum();
    }
    let mut total = estimate_tokens(&message.content);
    if let Some(reasoning) = message.reasoning.as_deref() {
        total += estimate_tokens(reasoning);
    }
    for tool in &message.tool_calls {
        total += estimate_tokens(&tool.name);
        total += estimate_tokens(&tool.arguments);
        if let Some(preview) = tool.result_preview.as_deref() {
            total += estimate_tokens(preview);
        }
        if let Some(err) = tool.error.as_deref() {
            total += estimate_tokens(err);
        }
    }
    total + 4
}

pub(crate) fn accumulate_source_ids(conversation: &Conversation, until_id: &str) -> Vec<String> {
    let prev = crate::chat::commands::context::active_summary(conversation);
    let mut ids = prev
        .map(|s| s.source_message_ids.clone())
        .unwrap_or_default();
    let summary_start = crate::chat::commands::context::context_replay_start_index(conversation);
    let Some(until_idx) = conversation.messages.iter().position(|m| m.id == until_id) else {
        return ids;
    };
    if until_idx < summary_start {
        return ids;
    }
    ids.extend(
        conversation.messages[summary_start..=until_idx]
            .iter()
            .map(|m| m.id.clone()),
    );
    ids
}

pub(crate) async fn maybe_compact_send_view(env: &LoopEnv<'_>, state: &mut RunState) -> Vec<Value> {
    compact_send_view(env, state, false).await
}

pub(crate) async fn compact_send_view(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    force: bool,
) -> Vec<Value> {
    let config = env.config;
    let trigger = if force { "reactive" } else { "agent_loop" };
    // 先做无条件的图片收敛：与 token 预算无关，重复上传同一张图、以及无上限堆积的历史
    // 图片，任何情况下都是纯浪费，而 token 估算看不见它们（详见 `prune_image_parts`）。
    let saved_bytes = prune_image_parts(&mut state.runtime_messages, IMAGE_BYTES_BUDGET);
    if saved_bytes > 0 {
        state.last_step_usage = None;
        state.initial_anchor_valid = false;
        eprintln!("Chat context: pruned {saved_bytes} bytes of image data from the send view");
    }
    let window = context_window_for_model(Some(&config.provider), &config.model).0;
    // 真实用量锚点口径（对齐 pi/opencode 的 ground-truth 优先）：有锚点时用 provider 实报的
    // 上次 prompt token 数 + 锚点响应起往后新增消息的字符估算；无锚点回落纯字符估算。
    // 纯估算仅用于没有有效实报的情况，不能覆盖已上报的用量。
    let budget = auto_compact_budget(window, config.max_output_tokens);
    // 纯字符估算 = 消息 + **工具 schema**（对齐 pi/footer 的兜底口径：pi 兜底含 system+每工具+消息；
    // Kivio footer 也含 `estimate_tool_segments`）。工具定义随每次请求发送、provider 会计入，漏算会
    // 让无锚点的首轮低估数千 token、压缩过晚——故这里补上（与 footer `count_tokens_in_value` 同口径，
    // 都基于 `estimate_value_tokens(tool.to_openai_tool())`）。
    // 按「工具名集合哈希」做轮间缓存：每轮为上百个工具重建整份 schema JSON 只为估个
    // token 数太浪费；工具集只在 Skill 激活时变（同名工具的 schema run 内稳定）。
    let tool_schema_tokens = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for tool in &state.tools {
            tool.name.hash(&mut hasher);
        }
        let fingerprint = hasher.finish();
        match state.tool_schema_tokens_cache {
            Some((cached_fingerprint, cached)) if cached_fingerprint == fingerprint => cached,
            _ => {
                let estimated: usize = state
                    .tools
                    .iter()
                    .map(|tool| estimate_value_tokens(&tool.to_openai_tool()))
                    .sum();
                state.tool_schema_tokens_cache = Some((fingerprint, estimated));
                estimated
            }
        }
    };
    let estimate_full =
        estimate_messages_tokens(&state.runtime_messages).saturating_add(tool_schema_tokens);
    let (anchor_prompt, trailing) = if let Some(usage) = &state.last_step_usage {
        // 本轮已发生过模型调用：锚点 = 上次调用 usage（含 output）；trailing = 那次**响应之后**新增。
        let start = state
            .runtime_len_at_last_call
            .min(state.runtime_messages.len());
        (
            super::context_estimate::anchor_total_tokens(usage, &config.provider.api_format),
            estimate_messages_tokens(&state.runtime_messages[start..]),
        )
    } else if state.initial_anchor_valid {
        // 本轮尚未调用模型（首次压缩检查）：用上一轮落盘 usage 组成的 config 锚点。
        (
            config.initial_anchor_total_tokens,
            config.initial_anchor_trailing_estimate,
        )
    } else {
        (None, 0)
    };
    let (estimated, anchored) =
        super::context_estimate::effective_context_tokens(anchor_prompt, trailing, estimate_full);
    // **内置路径的实时用量通道**：本函数每个 planning 轮都跑一次，且这两个数就是权威口径
    // （`compute_context_state` 用的是同一对函数 `anchor_total_tokens` +
    // `effective_context_tokens`，分母同样是 `context_window_for_model`）—— 白捡的实时来源，
    // 零额外计算。粒度是「每轮一次」而不是每个 token：内置路径的分子来自 provider 的
    // usage，只有一次模型调用结束才有新数，中途没有更细的真实来源。
    // 子 agent 的 host 走默认 no-op，用量不会混进主对话。
    env.host.emit_context_usage_live(
        &config.conversation_id,
        estimated as u64,
        super::context_estimate::token_count_source(anchored, trailing),
        Some(window as u64),
    );
    if !force && estimated < budget {
        return state.runtime_messages.clone();
    }
    // ZCode policy: a failed automatic compaction never ends the turn; the request goes
    // out as is, and a real provider overflow still gets one reactive compaction. After
    // consecutive failures automatic compaction pauses until a compaction succeeds.
    if !force && state.auto_compact_failures >= MAX_CONSECUTIVE_AUTO_COMPACT_FAILURES {
        return state.runtime_messages.clone();
    }
    // Too little history is a healthy no-op: no events, no failure.
    if !has_compactable_history(&state.runtime_messages) {
        return state.runtime_messages.clone();
    }
    // Rapid refill is judged before starting and recorded only when a compaction succeeds.
    let rapid_refills =
        if state.compacted && state.tool_batches_since_compact < RAPID_REFILL_TOOL_BATCHES {
            state.rapid_refills + 1
        } else {
            0
        };
    if !force && rapid_refills >= MAX_CONSECUTIVE_RAPID_REFILLS {
        eprintln!("Chat context compaction: context refilled right after compaction {rapid_refills} times; ending the turn");
        state.compaction_blocked = true;
        return state.runtime_messages.clone();
    }

    eprintln!(
        "Chat context compaction: est {estimated} tokens over budget {budget} (window {window}); summarizing old history"
    );

    env.host
        .emit_compaction_status(&config.conversation_id, "started", Some(trigger), None);
    let cancel = env
        .host
        .wait_for_generation_inactive(&config.conversation_id, config.generation);
    let runtime_before_compact = state.runtime_messages.clone();
    let compacted = config
        .provider_runtime
        .summarize(super::provider_runtime::SummaryRequest {
            provider: &config.provider,
            model: &config.model,
            messages: &state.runtime_messages,
            preserve_recent: true,
            tools: if state.provider_tools_unsupported {
                &[]
            } else {
                &state.tools
            },
            // Keep the model's real output budget, not the run's shorter answer budget.
            max_output_tokens: chat_max_output_tokens_for_model(
                Some(&config.provider),
                &config.model,
            )
            .unwrap_or(SUMMARY_OUTPUT_TOKENS),
            conversation_id: &config.conversation_id,
            message_id: &config.message_id,
            cancel: Some(cancel),
            host: Some(env.host),
        })
        .await;

    match compacted {
        CompactOutcome::Compacted(compacted, summary_text) => {
            let after = estimate_messages_tokens(&compacted).saturating_add(tool_schema_tokens);
            state.tool_batches_since_compact = 0;
            state.rapid_refills = rapid_refills;
            state.auto_compact_failures = 0;
            env.host
                .set_auto_compact_failures(&config.conversation_id, 0);
            eprintln!("Chat context compaction: est {estimated} -> {after} tokens");
            state.runtime_messages = compacted.clone();
            state.compacted = true;
            // 压缩后消息序列已变，旧锚点失真——清空，回落纯估算直到下次模型调用产生新 usage。
            state.last_step_usage = None;
            state.initial_anchor_valid = false;
            if let Some(source_until_message_id) = runtime_before_compact
                .iter()
                .rev()
                .find_map(|m| m[UI_MESSAGE_ID_KEY].as_str().map(str::to_string))
            {
                let created_at = chrono::Local::now().timestamp();
                let summary_record = ConversationContextSummary {
                    id: format!("ctxsum_{}", uuid::Uuid::new_v4()),
                    content: summary_text.clone(),
                    source_message_ids: Vec::new(),
                    source_until_message_id: source_until_message_id.clone(),
                    token_estimate_before: estimated,
                    token_estimate_after: estimate_tokens(&summary_text),
                    created_at,
                    provider_id: config.provider.id.clone(),
                    model: config.model.clone(),
                    stale: false,
                    // Populated at the reply.rs persist site, which has the
                    // Conversation (this L2 path only holds runtime Values).
                    file_ledger: None,
                    replay: Some(CompactionReplay {
                        through_message_id: config.message_id.clone(),
                        messages: replacement_body(&compacted),
                    }),
                };
                let boundary = CompactionBoundaryRecord {
                    id: format!("ctxbd_{}", uuid::Uuid::new_v4()),
                    source_until_message_id,
                    // 时间线锚点：触发压缩时 runtime 里最后一条可映射的 UI 消息（run 进行中
                    // assistant 尚未落库，即最后一条 user）——divider 标记压缩发生的时刻。
                    display_after_message_id: runtime_before_compact
                        .iter()
                        .rev()
                        .find_map(|m| m.get(UI_MESSAGE_ID_KEY).and_then(Value::as_str))
                        .map(str::to_string),
                    token_estimate_before: estimated,
                    token_estimate_after: after,
                    summary_content: summary_text,
                    trigger: trigger.to_string(),
                    created_at,
                };
                env.host.emit_compaction_status(
                    &config.conversation_id,
                    "completed",
                    Some(trigger),
                    Some(&boundary),
                );
                state.pending_compaction_boundary = Some(boundary);
                state.pending_compaction_summary = Some(summary_record);
            } else {
                // 压缩视图已生效但无法可靠映射回 UI 消息（旧段只有摘要锚点/系统注入）——
                // 不落盘 boundary，但必须发终止事件让前端"压缩中"归位。
                env.host.emit_compaction_status(
                    &config.conversation_id,
                    "completed",
                    Some(trigger),
                    None,
                );
            }
            match crate::chat::workflow_hooks::compact_context(
                env.host,
                &config.conversation_id,
                config.generation,
            )
            .await
            {
                Ok(context) => crate::chat::workflow_hooks::inject_context(
                    &mut state.runtime_messages,
                    &context,
                ),
                Err(error) => crate::chat::workflow_hooks::inject_context(
                    &mut state.runtime_messages,
                    &[format!("SessionStart compact hook failed: {error}")],
                ),
            }
            state.runtime_messages.clone()
        }
        CompactOutcome::Skipped => {
            // Checked above; kept exhaustive. `started` was sent, so close it without a boundary.
            env.host.emit_compaction_status(
                &config.conversation_id,
                "completed",
                Some(trigger),
                None,
            );
            state.runtime_messages.clone()
        }
        CompactOutcome::Cancelled => {
            // 用户主动取消进行中的 run：不计入失败次数（取消 ≠ 压缩无能为力），
            // 让后续 planning 自己检测取消并正常收尾。仍发终止事件让前端"压缩中"归位。
            env.host.emit_compaction_status(
                &config.conversation_id,
                "interrupted",
                Some(trigger),
                None,
            );
            state.runtime_messages.clone()
        }
        CompactOutcome::Failed => {
            // The request continues uncompacted; only the circuit breaker counts this.
            state.auto_compact_failures = state.auto_compact_failures.saturating_add(1);
            env.host
                .set_auto_compact_failures(&config.conversation_id, state.auto_compact_failures);
            // started 已发——失败也必须发终止事件，否则前端"压缩中"状态永久卡死。
            env.host
                .emit_compaction_status(&config.conversation_id, "failed", Some(trigger), None);
            state.runtime_messages.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::model::ModelRole;
    use crate::chat::types::{ToolCallRecord, ToolCallStatus};
    #[test]
    fn budget_reserves_output_and_buffer() {
        assert_eq!(auto_compact_budget(200_000, 32_000), 166_000);
        assert_eq!(auto_compact_budget(128_000, 8_192), 106_808);
        assert_eq!(auto_compact_budget(8_000, 32_000), 0);
        assert_eq!(auto_compact_budget(0, 0), 166_000);
    }

    #[test]
    fn groups_preserve_assistant_and_all_tool_results() {
        let messages = vec![
            json!({"role":"system","content":"rules"}),
            json!({"role":"user","content":"task"}),
            json!({"role":"assistant","tool_calls":[{"id":"call"}]}),
            json!({"role":"tool","tool_call_id":"call","content":"result"}),
            json!({"role":"user","content":"steering"}),
        ];
        assert_eq!(group_starts(&messages), (1, vec![1, 2]));
        assert_eq!(messages[group_starts(&messages).1[1]]["role"], "assistant");
    }

    #[test]
    fn previous_summary_is_not_immutable_prefix() {
        for role in ["user", "system"] {
            let messages = vec![
                json!({"role":"system","content":"rules"}),
                json!({"role":role,"content":"Previous conversation summary:\nold"}),
                json!({"role":"assistant","content":"new work"}),
            ];
            assert_eq!(group_starts(&messages), (1, vec![1, 2]));
        }
    }

    #[test]
    fn summary_discards_analysis_and_rejects_incomplete_tags() {
        assert_eq!(
            summary_text("<analysis>private</analysis><summary>done</summary>"),
            Some("done".into())
        );
        assert_eq!(summary_text("<analysis>unfinished"), None);
        assert_eq!(summary_text("<summary>unfinished"), None);
        assert_eq!(summary_text(" "), None);
    }

    #[test]
    fn replacement_body_preserves_exact_tail_without_system_or_summary() {
        let tail = json!({"role":"assistant","tool_calls":[{"id":"x"}]});
        let result = json!({"role":"tool","tool_call_id":"x","content":"r"});
        let messages = vec![
            json!({"role":"system","content":"rules"}),
            summary_message("summary"),
            tail.clone(),
            result.clone(),
            super::super::stop::step_limit_system_message(),
        ];
        assert_eq!(replacement_body(&messages), vec![tail, result]);
    }

    #[test]
    fn read_reminders_are_bounded_and_skip_preserved_reads() {
        let mut old = Vec::new();
        for n in 0..8 {
            old.push(json!({"role":"assistant","tool_calls":[{"id":n.to_string(), "function":{"name":"read","arguments":format!("{{\"path\":\"file{n}\"}}")}}]}));
            old.push(
                json!({"role":"tool","tool_call_id":n.to_string(),"content":format!("file{n} — lines 1-1 of 1\n     1\tbody{n}")}),
            );
        }
        let kept = vec![old[14].clone(), old[15].clone()];
        let reminders = read_reminders(&old, &kept);
        assert_eq!(reminders.len(), 5);
        let text = serde_json::to_string(&reminders).unwrap();
        assert!(!text.contains("body7"));
        assert!(text.contains("body6"));
        assert!(!text.contains("body0"));
    }

    #[test]
    fn read_reminders_skip_failed_and_non_file_reads() {
        let call = |id: &str, path: &str| json!({"role":"assistant","tool_calls":[{"id":id,"function":{"name":"read","arguments":format!("{{\"path\":\"{path}\"}}")}}]});
        let old = vec![
            call("ok", "src/a.rs"),
            json!({"role":"tool","tool_call_id":"ok","content":"src/a.rs — lines 10-12 of 90\n    10\tfn a() {}"}),
            call("missing", "src/b.rs"),
            json!({"role":"tool","tool_call_id":"missing","content":"No such file or directory: src/b.rs"}),
            call("dir", "src"),
            json!({"role":"tool","tool_call_id":"dir","content":"Directory src:\na.rs"}),
        ];
        let reminders = read_reminders(&old, &[]);
        assert_eq!(reminders.len(), 1);
        let text = reminders[0]["content"].as_str().unwrap();
        assert!(
            text.contains("lines 10-12 of 90"),
            "partial reads say so: {text}"
        );
        assert!(text.contains("may be partial"));
    }

    #[test]
    fn manual_summary_media_follows_model_capability() {
        let image = json!({"type":"image_url","image_url":{"url":"data:image/png;base64,AAAA"}});
        let messages =
            vec![json!({"role":"user","content":[{"type":"text","text":"look"}, image]})];
        for (vision, keeps_image) in [(Some(true), true), (None, true), (Some(false), false)] {
            let mut projected = messages.clone();
            project_summary_media(&mut projected, vision);
            assert_eq!(
                projected[0]["content"].is_array(),
                keeps_image,
                "vision {vision:?}"
            );
            if !keeps_image {
                assert_eq!(
                    projected[0]["content"],
                    json!(format!("look {IMAGE_PART_PLACEHOLDER}"))
                );
            }
        }
    }

    #[test]
    fn tool_history_becomes_text_without_tool_definitions() {
        let mut messages = vec![
            json!({"role":"assistant","content":null,"tool_calls":[{"id":"c","type":"function","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]}),
            json!({"role":"tool","tool_call_id":"c","content":"result body"}),
            json!({"role":"user","content":"plain"}),
        ];
        tool_history_as_text(&mut messages);
        assert_eq!(
            messages[0],
            json!({"role":"assistant","content":"[Tool call] read {\"path\":\"a\"}"})
        );
        assert_eq!(
            messages[1],
            json!({"role":"user","content":"[Tool result]\nresult body"})
        );
        assert_eq!(messages[2], json!({"role":"user","content":"plain"}));
    }

    #[test]
    fn compactable_history_needs_two_summarized_rounds_with_an_assistant() {
        let msg = |role: &str| json!({"role":role,"content":"x"});
        let sys = json!({"role":"system","content":"rules"});
        // One round kept for automatic compaction leaves only a user turn to summarize.
        assert!(!has_compactable_history(&[
            sys.clone(),
            msg("user"),
            msg("assistant"),
            msg("user")
        ]));
        assert!(has_compactable_history(&[
            sys,
            msg("user"),
            msg("assistant"),
            msg("user"),
            msg("assistant"),
            msg("user"),
        ]));
    }
    fn chat_msg(id: &str, role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            reasoning: None,
            artifacts: Vec::new(),
            tool_calls: Vec::new(),
            segments: Vec::new(),
            agent_plan: None,
            api_messages: Vec::new(),
            model_messages: Vec::new(),
            active_skill_id: None,
            run_entry: None,
            stream_outcome: None,
            usage: None,
            anchor_usage: None,
            group_id: None,
            provider_id: None,
            model: None,
            timestamp: 0,
            degraded: None,
        }
    }
    fn test_conversation(messages: Vec<ChatMessage>) -> Conversation {
        Conversation {
            id: "conv_test".to_string(),
            revision: 0,
            title: "t".to_string(),
            provider_id: "p".to_string(),
            model: "m".to_string(),
            messages,
            agent_runtime: Default::default(),
            active_skill_id: None,
            assistant_id: None,
            assistant_snapshot: None,
            created_at: 0,
            updated_at: 0,
            pinned: false,
            archived: false,
            folder: None,
            project_id: None,
            set_id: None,
            context_state: Default::default(),
            agent_todo_state: Default::default(),
            agent_plan_state: Default::default(),
            goal_state: None,
            knowledge_base_ids: Vec::new(),
            force_knowledge_search: false,
            additional_directories: Vec::new(),
            thinking_level: None,
            web_search_mode: None,
            reply_models: Vec::new(),
            group_selections: Default::default(),
            forked_from: None,
        }
    }
    #[test]
    fn estimate_chat_message_tokens_counts_content_and_tools() {
        let mut m = chat_msg("m1", "assistant", &"abcd".repeat(100));
        m.reasoning = Some("r".repeat(40));
        m.tool_calls.push(ToolCallRecord {
            id: "c1".to_string(),
            name: "read".to_string(),
            source: String::new(),
            server_id: None,
            arguments: "{\"path\":\"/tmp/x\"}".to_string(),
            status: ToolCallStatus::Success,
            result_preview: Some("p".repeat(80)),
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 0,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        });
        let tokens = estimate_chat_message_tokens(&m);
        // content 100/4=25, reasoning 40/4=10, tool name+args+preview ~ 25+, +1
        assert!(tokens > 60);
    }
    #[test]
    fn estimate_message_tokens_ignores_image_base64() {
        // 缺陷 2：图片 base64 不能打爆估算。带 1MB 级 base64 的多模态 user 消息，
        // 估算应与同文本纯文字消息同数量级（图片部件记 0）。
        let big_b64 = "A".repeat(1_400_000);
        let image_msg = json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "描述这张图" },
                { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{big_b64}") } }
            ]
        });
        let text_only = json!({ "role": "user", "content": "描述这张图" });
        let image_tokens = estimate_message_tokens(&image_msg);
        let text_tokens = estimate_message_tokens(&text_only);
        // 同数量级：差值不超过几十 token（结构 key 开销），绝不因 base64 膨胀到十万级。
        assert!(
            image_tokens < text_tokens + 50,
            "image msg est {image_tokens} must stay near text-only {text_tokens}"
        );
    }
    #[test]
    fn estimate_message_tokens_counts_reasoning() {
        // 缺陷 4(a)：reasoning_content 必须计入（与 serialize 口径一致）。
        let with_reasoning = json!({
            "role": "assistant",
            "content": "answer",
            "reasoning_content": "x".repeat(400)
        });
        let without = json!({ "role": "assistant", "content": "answer" });
        assert!(
            estimate_message_tokens(&with_reasoning) > estimate_message_tokens(&without) + 90,
            "reasoning (~100 tok) must be counted"
        );
    }
    #[test]
    fn prune_image_parts_keeps_only_the_last_copy() {
        // 真实故障复现：同一张图被 `read` 读了两次，两份逐字节相同的 base64 各占请求体
        // 一大半，每个 planning 轮重传一次，中转在传输途中断流。
        let image = |b64: &str| json!({ "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{b64}") } });
        let dup = "A".repeat(2000);
        let mut messages = vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "看这张图" }, image(&dup)] }),
            json!({ "role": "assistant", "content": "好的" }),
            json!({ "role": "user", "content": [image("DIFFERENT")] }),
            json!({ "role": "user", "content": [image(&dup)] }),
        ];

        let saved = prune_image_parts(&mut messages, IMAGE_BYTES_BUDGET);
        assert!(saved > 2000, "should report the dropped bytes, got {saved}");

        // 第一条里的重复图 → 占位文本
        let first = &messages[0]["content"][1];
        assert_eq!(first["type"], "text");
        assert_eq!(first["text"], DUPLICATE_IMAGE_PLACEHOLDER);
        // 文本部件不受影响
        assert_eq!(messages[0]["content"][0]["text"], "看这张图");
        // 不同的图原样保留
        assert_eq!(messages[2]["content"][0]["type"], "image_url");
        assert!(messages[2]["content"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .contains("DIFFERENT"));
        // 最后一份重复图保留原图（模型当下在看的就是它）
        assert_eq!(messages[3]["content"][0]["type"], "image_url");
        assert!(messages[3]["content"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .contains(&dup));

        // 幂等：再跑一次不再有可省的字节，也不会误删仅剩的那份
        assert_eq!(prune_image_parts(&mut messages, IMAGE_BYTES_BUDGET), 0);
        assert_eq!(messages[3]["content"][0]["type"], "image_url");
    }
    #[test]
    fn prune_image_parts_evicts_oldest_images_over_budget() {
        // 全都是**不同**的图（去重救不了），靠字节预算从最旧的开始淘汰。
        // 对应 Codex #28316：不同截图反复堆积，请求打到 8MB 后中转 502。
        let image = |tag: &str| {
            json!({
                "type": "image_url",
                "image_url": { "url": format!("data:image/png;base64,{tag}{}", "X".repeat(1000)) }
            })
        };
        let mut messages: Vec<Value> = ["oldest", "middle", "newest"]
            .iter()
            .map(|tag| json!({ "role": "user", "content": [image(tag)] }))
            .collect();

        // 预算只够放两张（每张 ~1KB 序列化后略多）。
        let saved = prune_image_parts(&mut messages, 2400);
        assert!(
            saved > 1000,
            "oldest image should be evicted, saved {saved}"
        );

        // 最旧的被换成带体积说明的占位文本
        assert_eq!(messages[0]["content"][0]["type"], "text");
        let note = messages[0]["content"][0]["text"].as_str().unwrap();
        assert!(note.contains("已从上下文移除"), "note was: {note}");
        assert!(note.contains("KB"), "占位符要带体积，便于用户理解: {note}");
        // 较新的两张保留
        assert_eq!(messages[1]["content"][0]["type"], "image_url");
        assert_eq!(messages[2]["content"][0]["type"], "image_url");

        // 预算充足时一张都不动
        let mut untouched: Vec<Value> = ["a", "b", "c"]
            .iter()
            .map(|tag| json!({ "role": "user", "content": [image(tag)] }))
            .collect();
        assert_eq!(prune_image_parts(&mut untouched, IMAGE_BYTES_BUDGET), 0);
        assert!(untouched
            .iter()
            .all(|m| m["content"][0]["type"] == "image_url"));
    }
    #[test]
    fn estimate_chat_message_tokens_uses_expanded_api_messages() {
        // 缺陷 4(b)：带完整工具转录的 api_messages 应按展开形态估算，
        // 远大于截断 result_preview 口径。
        let mut m = chat_msg("m1", "assistant", "short visible text");
        m.tool_calls.push(ToolCallRecord {
            id: "c1".to_string(),
            name: "read".to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: "{}".to_string(),
            status: ToolCallStatus::Success,
            result_preview: Some("p".repeat(80)), // 截断预览（旧口径只算这个）
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 0,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        });
        // 完整工具输出（真实 replay 内容）远大于 preview。
        m.api_messages = vec![
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "read", "arguments": "{}" } }] }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "T".repeat(40_000) }),
        ];
        let tokens = estimate_chat_message_tokens(&m);
        let expanded_sum: usize = m.api_messages.iter().map(estimate_message_tokens).sum();
        assert_eq!(tokens, expanded_sum, "must estimate expanded api_messages");
        assert!(
            tokens > 9_000,
            "40k-char tool output ~10k tokens, far above preview"
        );
    }
    #[test]
    fn estimate_model_messages_tokens_ignores_image_base64() {
        // #2 修复：model_messages 分支直接按 MessagePart 估算，不把 base64 图片算进 token
        //（也不再克隆整段转录成 Vec<Value>）。
        let big_b64 = "A".repeat(1_400_000);
        let mut m = chat_msg("m1", "assistant", "ignored (model_messages wins)");
        m.model_messages = vec![
            ModelMessage {
                role: ModelRole::Assistant,
                content: vec![
                    MessagePart::Text {
                        text: "看这张图".to_string(),
                    },
                    MessagePart::Image {
                        mime_type: "image/png".to_string(),
                        data: big_b64.clone(),
                        path: None,
                    },
                ],
            },
            ModelMessage {
                role: ModelRole::Tool,
                content: vec![MessagePart::ToolResult {
                    tool_call_id: "c1".to_string(),
                    content: "T".repeat(4_000),
                    is_error: false,
                    artifacts: Vec::new(),
                }],
            },
        ];
        let tokens = estimate_chat_message_tokens(&m);
        // 文本(~2 CJK*? ) + 工具结果 4000/4=1000 + 2*4 开销 ≈ 1010 量级；绝不含 base64 的 35 万级。
        assert!(
            tokens < 2_000,
            "image base64 must not inflate estimate (was {tokens})"
        );
        assert!(
            tokens > 900,
            "tool result content must be counted (was {tokens})"
        );
    }
    #[test]
    fn accumulate_source_ids_unions_previous_and_new_range() {
        use crate::chat::types::ConversationContextState;
        let messages: Vec<ChatMessage> = ["m0", "m1", "m2", "m3", "m4"]
            .iter()
            .map(|id| chat_msg(id, "user", "x"))
            .collect();
        let mut conversation = test_conversation(messages);
        // 已有 S1 覆盖 m0..=m1（source_until = m1，ids = [m0, m1]）。
        conversation.context_state = ConversationContextState {
            summary: Some(ConversationContextSummary {
                id: "ctxsum_prev".to_string(),
                content: "prev".to_string(),
                source_message_ids: vec!["m0".to_string(), "m1".to_string()],
                source_until_message_id: "m1".to_string(),
                token_estimate_before: 0,
                token_estimate_after: 0,
                created_at: 0,
                provider_id: "p".to_string(),
                model: "m".to_string(),
                stale: false,
                file_ledger: None,
                replay: None,
            }),
            ..Default::default()
        };
        // 新压缩覆盖到 m3 → ids = S1.ids ∪ (m2, m3)。
        let ids = accumulate_source_ids(&conversation, "m3");
        assert_eq!(ids, vec!["m0", "m1", "m2", "m3"]);
        // until_id 未找到 → 仅旧 ids。
        assert_eq!(
            accumulate_source_ids(&conversation, "nope"),
            vec!["m0", "m1"]
        );
    }
    #[test]
    fn accumulate_source_ids_no_previous_summary() {
        let messages: Vec<ChatMessage> = ["a", "b", "c"]
            .iter()
            .map(|id| chat_msg(id, "user", "x"))
            .collect();
        let conversation = test_conversation(messages);
        // 无旧 summary → 从头累积到 until_id。
        assert_eq!(accumulate_source_ids(&conversation, "b"), vec!["a", "b"]);
    }
    #[test]
    fn accumulate_source_ids_starts_after_context_clear() {
        let messages: Vec<ChatMessage> = ["a", "b", "c"]
            .iter()
            .map(|id| chat_msg(id, "user", "x"))
            .collect();
        let mut conversation = test_conversation(messages);
        conversation.context_state.clear_boundaries.push(
            crate::chat::types::ContextClearBoundaryRecord {
                id: "clr".to_string(),
                source_until_message_id: "a".to_string(),
                created_at: 1,
            },
        );
        assert_eq!(accumulate_source_ids(&conversation, "c"), vec!["b", "c"]);
    }
}
