use crate::chat::model::invocation::stream_with_chat_provider;
// Model requests for the existing Agent loop, independent of the desktop host.
use super::host::AgentHost;
use super::stream::{
    validate_stream_output, AgentStreamSink, ChatStreamOutput, ToolCallDraftTracker,
    WebSearchCardTracker,
};
use super::types::AgentStreamPolicy;
use crate::chat::model::{
    generate_request_from_openai_messages, GenerateOptions, GenerateOutput, GenerateRequest,
    GenerateRequestContext, ModelError, StreamPart, StreamSink,
};
use crate::chat::types::ChatMessageSegment;
use crate::mcp::ChatToolDefinition;
use serde_json::Value;

/// 模型调用并同时返回 provider 报告的 usage，
/// 供循环把每次模型调用的 token 消耗累计进 AgentRunResult。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn call_chat_completion_message_with_usage(
    state: &dyn crate::chat::model::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    thinking_enabled: bool,
    thinking_level: Option<String>,
    builtin_web_search: bool,
    max_output_tokens: u32,
    conversation_id: &str,
    message_id: &str,
    label: &str,
) -> Result<(Value, Option<crate::chat::model::ModelUsage>), String> {
    let (message, usage, _web_search, _images) = call_chat_completion_output_with_usage(
        state,
        provider,
        model,
        messages,
        tools,
        retry_attempts,
        thinking_enabled,
        thinking_level,
        builtin_web_search,
        max_output_tokens,
        conversation_id,
        message_id,
        label,
    )
    .await?;
    Ok((message, usage))
}

/// 同 `call_chat_completion_message_with_usage`，但额外回传适配器解析出的内置搜索引用
/// （`GenerateOutput.web_search`）。非流式答案路径用它把内置搜索可视化成工具卡（任务 07-23）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn call_chat_completion_output_with_usage(
    state: &dyn crate::chat::model::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    thinking_enabled: bool,
    thinking_level: Option<String>,
    builtin_web_search: bool,
    max_output_tokens: u32,
    conversation_id: &str,
    message_id: &str,
    label: &str,
) -> Result<
    (
        Value,
        Option<crate::chat::model::ModelUsage>,
        Option<crate::chat::model::BuiltinWebSearch>,
        Vec<crate::chat::model::GeneratedImageData>,
    ),
    String,
> {
    let request = generate_request_from_openai_messages(
        model,
        messages,
        tools,
        GenerateOptions {
            thinking_enabled,
            thinking_level,
            builtin_web_search,
            max_tokens: max_output_tokens,
            ..GenerateOptions::default()
        },
        label,
        GenerateRequestContext::new(Some(conversation_id), Some(message_id)),
    );
    // **一律走流式**（`generate_via_stream_collect`），哪怕调用方只要一个完整结果。
    // 部分 openai_responses 代理只可靠地服务流式请求，非流式 `generate` 直接报
    // "Unknown Responses API error" —— 压缩、标题总结都各自被这个坑绊过一次。
    // 这里是所有「要完整结果」的模型调用的唯一出口（planning / synthesis / 恢复重试 /
    // 原生工具都汇到这儿），改在这一处就等于全线走流式；`GenerateOutput` 两条路径同型，
    // usage / web_search / images 照旧回传（正常答案路径本来就只走流式，是被走熟的那条）。
    let output = generate_via_stream_collect(state, provider, retry_attempts, request)
        .await
        .map_err(|err| err.to_string())?;
    let usage = output.usage.clone();
    let web_search = output.web_search.clone();
    let images = output.images.clone();
    Ok((
        output.to_openai_compatible_message(),
        usage,
        web_search,
        images,
    ))
}

/// 与 `call_chat_completion_message_with_usage` 同形（返回 `to_openai_compatible_message()` Value），
/// 但走**流式**路径而非非流式 `generate`：内部用 `generate_via_stream_collect` 触发流式后
/// 取回 provider 组装好的 `GenerateOutput`。
///
/// 动机：压缩的摘要调用是 agent 里**唯一**的非流式模型调用；部分 provider（如 `openai_responses`
/// 代理）只可靠地服务流式请求，非流式摘要调用会失败（"Unknown Responses API error"），导致压缩在
/// 这类 provider 上永远摘不动。整个 agent 的 planning/synthesis 已经证明流式在该 provider 上可用，
/// 故把摘要调用也改走流式。流式被所有 provider 普遍支持，对支持非流式的 GUI provider 也无退化。
///
/// 这是**无头**收集（不涉及 `AgentHost`、不发任何 host 事件），与手动压缩/非 UI 路径一致。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn call_chat_completion_message_streamed(
    state: &dyn crate::chat::model::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    thinking_enabled: bool,
    max_output_tokens: u32,
    conversation_id: &str,
    message_id: &str,
    label: &str,
) -> Result<Value, String> {
    let request = generate_request_from_openai_messages(
        model,
        messages,
        tools,
        GenerateOptions {
            thinking_enabled,
            max_tokens: max_output_tokens,
            ..GenerateOptions::default()
        },
        label,
        GenerateRequestContext::new(Some(conversation_id), Some(message_id)),
    );
    let output = generate_via_stream_collect(state, provider, retry_attempts, request)
        .await
        .map_err(|err| err.to_string())?;
    let mut message = output.to_openai_compatible_message();
    if let Some(reason) = output.finish_reason {
        message["finish_reason"] = serde_json::json!(reason);
    }
    Ok(message)
}

/// 无头流式收集 sink：丢弃所有增量。摘要调用只消费返回的 `GenerateOutput.text`
/// （所有 provider 适配器都在流式路径上从累积增量填好 `text`），故无需在此累积；
/// 仅把 `StreamPart::Error` 上抛为 `ModelError`。不向任何 `AgentHost` 发事件。
struct DiscardStreamSink;

impl StreamSink for DiscardStreamSink {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        if let StreamPart::Error { message } = part {
            return Err(ModelError::new(message));
        }
        Ok(())
    }
}

/// 走 provider 的**流式** `stream(...)`（经 `send_with_failover` + SSE 累积，与 planning/synthesis
/// 同一路径），用一个丢弃增量的 sink 触发流式，返回 provider 组装好的 `GenerateOutput`。
///
/// 所有适配器（`openai` / `anthropic` / `responses`）都在流式路径上把累积结果填进返回的
/// `GenerateOutput.text`/`reasoning`，故无需 sink 侧再累积兜底。
pub(crate) async fn generate_via_stream_collect(
    state: &dyn crate::chat::model::host::ModelHost,
    provider: &crate::settings::ModelProvider,
    retry_attempts: usize,
    request: GenerateRequest,
) -> Result<GenerateOutput, ModelError> {
    let mut sink = DiscardStreamSink;
    stream_with_chat_provider(state, provider, retry_attempts, request, &mut sink).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_scoped_chat_completion_inner(
    state: &dyn crate::chat::model::host::ModelHost,
    host: &dyn AgentHost,
    provider: &crate::settings::ModelProvider,
    model: &str,
    messages: Vec<Value>,
    tools: Option<&[ChatToolDefinition]>,
    retry_attempts: usize,
    thinking_enabled: bool,
    thinking_level: Option<String>,
    builtin_web_search: bool,
    max_output_tokens: u32,
    conversation_id: &str,
    run_id: &str,
    message_id: &str,
    generation: u64,
    label: &str,
    policy: AgentStreamPolicy,
    text_segment: Option<ChatMessageSegment>,
    reasoning_segment: Option<ChatMessageSegment>,
    tool_draft_tracker: Option<ToolCallDraftTracker>,
    web_search_tracker: Option<WebSearchCardTracker>,
) -> Result<ChatStreamOutput, ModelError> {
    host.begin_context_request(
        conversation_id,
        message_id,
        &provider.id,
        model,
        &messages,
        tools.unwrap_or_default(),
    );
    let request = generate_request_from_openai_messages(
        model,
        messages,
        tools,
        GenerateOptions {
            thinking_enabled,
            thinking_level,
            builtin_web_search,
            max_tokens: max_output_tokens,
            ..GenerateOptions::default()
        },
        label,
        GenerateRequestContext::new(Some(conversation_id), Some(message_id)),
    );
    crate::api::with_retry_budget(retry_attempts, async {
        let budget = crate::api::RetryBudget::current_or_new(retry_attempts);
        loop {
            let mut sink = AgentStreamSink::new(
                host,
                conversation_id,
                run_id,
                message_id,
                matches!(policy, AgentStreamPolicy::PlanningNoDoneUntilNoTools),
                text_segment.clone(),
                reasoning_segment.clone(),
                tool_draft_tracker.clone(),
                web_search_tracker.clone(),
            );
            let result = {
                let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
                let stream_future = crate::api::RETRY_PROGRESS.scope(
                    progress_tx,
                    stream_with_chat_provider(
                        state,
                        provider,
                        retry_attempts,
                        request.clone(),
                        &mut sink,
                    ),
                );
                tokio::pin!(stream_future);
                loop {
                    tokio::select! {
                        result = &mut stream_future => break Some(result),
                        Some(retry) = progress_rx.recv() => host.emit_retry_status(run_id, retry),
                        _ = host.wait_for_generation_inactive(conversation_id, generation) => break None,
                    }
                }
            };
            let Some(result) = result else {
                let (content, reasoning) = sink.snapshot();
                return Ok(ChatStreamOutput::new(
                    content.trim().to_string(),
                    reasoning.trim().to_string(),
                    true,
                ));
            };
            let output = match result {
                Ok(output) => output,
                Err(err) if err.is_stream_read_interrupted() => {
                    let Some((retry, delay_ms)) = budget.schedule(None, "Stream interrupted", false) else {
                        return Err(err);
                    };
                    sink.discard_attempt();
                    host.emit_retry_status(run_id, retry);
                    let delay = std::time::Duration::from_millis(delay_ms);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {},
                        _ = host.wait_for_generation_inactive(conversation_id, generation) => {
                            return Ok(ChatStreamOutput::new(String::new(), String::new(), true));
                        }
                    }
                    continue;
                }
                Err(err) => return Err(err),
            };
            host.emit_status_note(run_id, None);
            sink.flush_pending_text();
            // 内置搜索实时卡定稿：流成功结束 ⇒ 翻 Success 并发终态记录（取消路径已在上面的
            // select 分支 return，不会到这里，故取消时实时卡不落 Success，与 draft 行为一致）。
            if let Some(record) = sink.finish_web_search_card() {
                host.emit_tool_record(conversation_id, run_id, message_id, &record);
            }
            let (snapshot_content, snapshot_reasoning) = sink.snapshot();
            let stream_output = ChatStreamOutput::from_generate_output_with_snapshot(
                output,
                snapshot_content,
                snapshot_reasoning,
            );
            // Roll back exactly what this sink emitted, including Unicode and
            // provider-specific text normalization, before semantic recovery.
            if super::recovery::is_recoverable_text_length(&stream_output, max_output_tokens) {
                sink.discard_attempt();
            }
            validate_stream_output(label, policy, &stream_output).map_err(ModelError::new)?;
            return Ok(stream_output);
        }
    }).await
}
