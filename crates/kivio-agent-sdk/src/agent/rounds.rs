use super::content::{AgentToolCall, AgentToolStatus};
use super::history::{import_history, RuntimeMessage};
use crate::model::FinishKind;
use serde_json::Value;

use crate::model::PendingToolCall;

use crate::tools::ToolDefinition;

use super::execute::{
    execute_tool_call, invalid_tool_arguments_record, match_tool_call, pending_tool_record,
    unknown_tool_record, ToolExecutionContext, ToolExecutor,
};
use super::finalize::cancelled_tool_round_run_result;
use super::loop_::{LoopEnv, RunState};
use super::planning::PlannedToolRound;
use super::types::AgentRunResult;

/// Maximum number of eligible tool calls executed concurrently in one batch.
pub(crate) const MAX_PARALLEL_TOOL_CALLS_PER_ROUND: usize = 12;

pub(crate) enum ToolRoundOutcome {
    /// The round completed; the skeleton continues to the next planning round.
    Continue,
    /// The round was cancelled mid-flight; done("cancelled") was emitted and a
    /// final result built. The skeleton returns it.
    Cancelled(AgentRunResult),
}

pub(crate) async fn run_tool_round(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    planned: PlannedToolRound,
) -> ToolRoundOutcome {
    let _draft_guard = planned.drafts;
    let config = env.config;
    let host = env.host;
    let assistant_message = RuntimeMessage::model(planned.message);
    state.runtime_messages.push(assistant_message.clone());
    state.runtime_len_at_last_call = state.runtime_messages.len();
    state.generated_api_messages.push(assistant_message);
    let mut task_records = planned.tool_calls.iter().filter_map(|call| {
        let tool = match_tool_call(&state.tools, &call.function_name)?;
        tool.checkpoint_before_dispatch
            .then(|| pending_tool_record(tool, call, round))
    });
    if let Some(first) = task_records.next() {
        let mut records = state.tool_records.clone();
        records.push(first);
        records.extend(task_records);
        // The current tool-call message has no results yet, so it is not replayable.
        host.persist_partial_assistant(
            &config.conversation_id,
            &config.message_id,
            &records,
            state.segment_builder.segments(),
            &state.generated_api_messages[..state.generated_api_messages.len() - 1],
        )
        .await;
    }
    let round_result = execute_tool_round(
        host,
        env.executor,
        ToolRoundContext {
            conversation_id: &config.conversation_id,
            run_id: &config.run_id,
            message_id: &config.message_id,
            generation: config.generation,
            round,
            finish_reason: planned.finish_kind,
        },
        &state.tools,
        &state.blocked_tool_calls,
        planned.tool_calls,
    )
    .await;
    let round_cancelled = round_result.cancelled;
    let responses = import_history(round_result.response_messages);
    state.runtime_messages.extend(responses.iter().cloned());
    state.generated_api_messages.extend(responses);
    state.tool_records.extend(round_result.tool_records);
    if round_cancelled {
        return ToolRoundOutcome::Cancelled(cancelled_tool_round_run_result(
            &state.planning_reasoning_parts,
            std::mem::take(&mut state.tool_records),
            std::mem::take(&mut state.segment_builder).all(),
            std::mem::take(&mut state.generated_api_messages),
        ));
    }
    ToolRoundOutcome::Continue
}

pub struct ToolRoundContext<'a> {
    pub conversation_id: &'a str,
    pub run_id: &'a str,
    pub message_id: &'a str,
    pub generation: u64,
    pub round: u32,
    pub finish_reason: Option<FinishKind>,
}

pub struct ToolRoundResult {
    pub response_messages: Vec<Value>,
    pub tool_records: Vec<AgentToolCall>,
    pub cancelled: bool,
}

struct ToolExecutionResult {
    response_message: Value,
    record: Option<AgentToolCall>,
    cancelled: bool,
    /// Extra user-role messages (OpenAI shape) appended after ALL results in the
    /// round — used by `read` to feed an image to a vision model.
    follow_up_messages: Vec<Value>,
}

struct ExecutableToolCall<'a> {
    call: PendingToolCall,
    tool: &'a ToolDefinition,
}

pub fn visible_tool_segment_calls(
    executor: &dyn ToolExecutor,
    tools: &[ToolDefinition],
    blocked_tool_calls: &[ToolDefinition],
    tool_calls: &[PendingToolCall],
) -> Vec<PendingToolCall> {
    tool_calls
        .iter()
        .filter(|call| {
            match_tool_call(tools, &call.function_name).is_some()
                || match_tool_call(blocked_tool_calls, &call.function_name).is_some()
                || executor.unknown_feedback(&call.function_name).is_none()
        })
        .cloned()
        .collect()
}

pub async fn execute_tool_round(
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    ctx: ToolRoundContext<'_>,
    tools: &[ToolDefinition],
    blocked_tool_calls: &[ToolDefinition],
    tool_calls: Vec<PendingToolCall>,
) -> ToolRoundResult {
    // 对齐 pi `failToolCallsFromTruncatedMessage`：finish_reason=="length" 说明整条响应
    // 在 max_output_tokens 处被拦腰截断——流式工具参数经 salvage 收尾后完全可能「JSON 合法
    // 但语义残缺」（写文件只写了半个 content、编辑少了半条 edit），逐个看解析是否成功不可靠。
    // 整批作废、一个都不执行，让模型带着完整参数重发。
    if ctx.finish_reason == Some(FinishKind::Length) && !tool_calls.is_empty() {
        let mut response_messages = Vec::new();
        let mut tool_records = Vec::new();
        for tool_call in tool_calls {
            let result = truncated_tool_call_result(host, &ctx, tools, tool_call);
            push_tool_execution_result(result, &mut response_messages, &mut tool_records);
        }
        return ToolRoundResult {
            response_messages,
            tool_records,
            cancelled: false,
        };
    }

    let mut response_messages = Vec::new();
    let mut tool_records = Vec::new();
    let mut parallel_batch: Vec<ExecutableToolCall<'_>> = Vec::new();
    let mut cancelled = false;

    let mut tool_calls = tool_calls.into_iter();
    while let Some(tool_call) = tool_calls.next() {
        let Some(tool) = match_tool_call(tools, &tool_call.function_name) else {
            if flush_parallel_batch(
                &mut parallel_batch,
                &mut response_messages,
                &mut tool_records,
                host,
                executor,
                &ctx,
            )
            .await
            {
                cancelled = true;
                push_cancelled_tool_call(
                    host,
                    &ctx,
                    tools,
                    tool_call,
                    &mut response_messages,
                    &mut tool_records,
                );
                push_cancelled_tool_calls(
                    host,
                    &ctx,
                    tools,
                    tool_calls,
                    &mut response_messages,
                    &mut tool_records,
                );
                break;
            }
            let result = unknown_or_disabled_tool_result(
                host,
                executor,
                &ctx,
                tools,
                blocked_tool_calls,
                tool_call,
            );
            push_tool_execution_result(result, &mut response_messages, &mut tool_records);
            continue;
        };

        if let Some(error) = tool_call.arguments_parse_error.clone() {
            if flush_parallel_batch(
                &mut parallel_batch,
                &mut response_messages,
                &mut tool_records,
                host,
                executor,
                &ctx,
            )
            .await
            {
                cancelled = true;
                push_cancelled_tool_call(
                    host,
                    &ctx,
                    tools,
                    tool_call,
                    &mut response_messages,
                    &mut tool_records,
                );
                push_cancelled_tool_calls(
                    host,
                    &ctx,
                    tools,
                    tool_calls,
                    &mut response_messages,
                    &mut tool_records,
                );
                break;
            }
            let result = invalid_tool_arguments_result(host, &ctx, &tool_call, tool, error);
            push_tool_execution_result(result, &mut response_messages, &mut tool_records);
            continue;
        }

        if tool.parallel_safe {
            parallel_batch.push(ExecutableToolCall {
                call: tool_call,
                tool,
            });
            if parallel_batch.len() >= MAX_PARALLEL_TOOL_CALLS_PER_ROUND {
                if flush_parallel_batch(
                    &mut parallel_batch,
                    &mut response_messages,
                    &mut tool_records,
                    host,
                    executor,
                    &ctx,
                )
                .await
                {
                    cancelled = true;
                    push_cancelled_tool_calls(
                        host,
                        &ctx,
                        tools,
                        tool_calls,
                        &mut response_messages,
                        &mut tool_records,
                    );
                    break;
                }
            }
            continue;
        }

        if flush_parallel_batch(
            &mut parallel_batch,
            &mut response_messages,
            &mut tool_records,
            host,
            executor,
            &ctx,
        )
        .await
        {
            cancelled = true;
            push_cancelled_tool_call(
                host,
                &ctx,
                tools,
                tool_call,
                &mut response_messages,
                &mut tool_records,
            );
            push_cancelled_tool_calls(
                host,
                &ctx,
                tools,
                tool_calls,
                &mut response_messages,
                &mut tool_records,
            );
            break;
        }
        let result = execute_serial_tool_call(host, executor, &ctx, tool, tool_call).await;
        if push_tool_execution_result(result, &mut response_messages, &mut tool_records) {
            cancelled = true;
            push_cancelled_tool_calls(
                host,
                &ctx,
                tools,
                tool_calls,
                &mut response_messages,
                &mut tool_records,
            );
            break;
        }
    }

    if !cancelled {
        cancelled = flush_parallel_batch(
            &mut parallel_batch,
            &mut response_messages,
            &mut tool_records,
            host,
            executor,
            &ctx,
        )
        .await;
    }

    ToolRoundResult {
        response_messages,
        tool_records,
        cancelled,
    }
}

fn push_tool_execution_result(
    result: ToolExecutionResult,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<AgentToolCall>,
) -> bool {
    let cancelled = result.cancelled;
    if let Some(record) = result.record {
        tool_records.push(record);
    }
    // A round may span parallel batches, serial calls, errors and cancellation.
    // Answer every call before any image/user turn, including follow-ups already
    // collected from an earlier batch. Preserve order within both groups.
    let result_end = response_messages
        .iter()
        .position(|message| message["role"] != "tool")
        .unwrap_or(response_messages.len());
    response_messages.insert(result_end, result.response_message);
    response_messages.extend(result.follow_up_messages);
    cancelled
}

async fn flush_parallel_batch(
    batch: &mut Vec<ExecutableToolCall<'_>>,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<AgentToolCall>,
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    ctx: &ToolRoundContext<'_>,
) -> bool {
    if batch.is_empty() {
        return false;
    }

    let mut cancelled = false;
    while !batch.is_empty() {
        let limit = batch.len().min(MAX_PARALLEL_TOOL_CALLS_PER_ROUND);
        let mut chunk = batch.drain(..limit).collect::<Vec<_>>();
        let results = execute_parallel_chunk(host, executor, ctx, &mut chunk).await;
        for result in results {
            cancelled |= push_tool_execution_result(result, response_messages, tool_records);
        }
        if cancelled {
            for item in batch.drain(..) {
                let result = cancelled_tool_result(host, ctx, &item.call, Some(item.tool));
                push_tool_execution_result(result, response_messages, tool_records);
            }
            break;
        }
    }
    cancelled
}

async fn execute_parallel_chunk(
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    ctx: &ToolRoundContext<'_>,
    chunk: &mut [ExecutableToolCall<'_>],
) -> Vec<ToolExecutionResult> {
    // join_all preserves input order and drives the whole chunk concurrently,
    // so the batch width can be anything up to MAX_PARALLEL_TOOL_CALLS_PER_ROUND
    // without per-arity hand-wiring (and without silently dropping calls past 4).
    futures::future::join_all(
        chunk
            .iter()
            .map(|item| execute_parallel_tool_call(host, executor, ctx, item)),
    )
    .await
}

async fn execute_parallel_tool_call(
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    ctx: &ToolRoundContext<'_>,
    item: &ExecutableToolCall<'_>,
) -> ToolExecutionResult {
    execute_tool_call_result(host, executor, ctx, item.tool, item.call.clone()).await
}

async fn execute_serial_tool_call(
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    ctx: &ToolRoundContext<'_>,
    tool: &ToolDefinition,
    tool_call: PendingToolCall,
) -> ToolExecutionResult {
    execute_tool_call_result(host, executor, ctx, tool, tool_call).await
}

async fn execute_tool_call_result(
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    round_ctx: &ToolRoundContext<'_>,
    tool: &ToolDefinition,
    tool_call: PendingToolCall,
) -> ToolExecutionResult {
    let tool_call_id = tool_call.id.clone();
    let execution_ctx = ToolExecutionContext {
        conversation_id: round_ctx.conversation_id,
        run_id: round_ctx.run_id,
        message_id: round_ctx.message_id,
        generation: round_ctx.generation,
        round: round_ctx.round,
        tool_call_id: &tool_call_id,
    };
    // 单点：串行与并行两条路径都经过这里，所以 start/end 一定成对。并行 chunk 里多个
    // 工具的事件会交错——那是真实语义，脚本靠 toolCallId 自己配对，不做重排。
    let mut lifecycle = super::events::ToolEventGuard::new(
        &host.events,
        round_ctx.round,
        &tool_call_id,
        &tool.label.name,
    );
    let (record, tool_content, follow_up_messages) =
        execute_tool_call(host, executor, &execution_ctx, tool, tool_call).await;
    lifecycle.cancelled = matches!(record.status, AgentToolStatus::Cancelled);
    let cancelled = matches!(record.status, AgentToolStatus::Cancelled);
    ToolExecutionResult {
        response_message: tool_message(tool_call_id.clone(), tool_content),
        record: Some(record),
        cancelled,
        follow_up_messages,
    }
}

fn push_cancelled_tool_calls(
    host: &super::context::AgentContext<'_>,
    ctx: &ToolRoundContext<'_>,
    tools: &[ToolDefinition],
    tool_calls: impl IntoIterator<Item = PendingToolCall>,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<AgentToolCall>,
) {
    for tool_call in tool_calls {
        push_cancelled_tool_call(host, ctx, tools, tool_call, response_messages, tool_records);
    }
}

fn push_cancelled_tool_call(
    host: &super::context::AgentContext<'_>,
    ctx: &ToolRoundContext<'_>,
    tools: &[ToolDefinition],
    tool_call: PendingToolCall,
    response_messages: &mut Vec<Value>,
    tool_records: &mut Vec<AgentToolCall>,
) {
    let tool = match_tool_call(tools, &tool_call.function_name);
    let result = cancelled_tool_result(host, ctx, &tool_call, tool);
    push_tool_execution_result(result, response_messages, tool_records);
}

fn cancelled_tool_result(
    host: &super::context::AgentContext<'_>,
    ctx: &ToolRoundContext<'_>,
    tool_call: &PendingToolCall,
    tool: Option<&ToolDefinition>,
) -> ToolExecutionResult {
    let now = chrono::Local::now().timestamp();
    let record = AgentToolCall {
        id: tool_call.id.clone(),
        name: tool
            .map(|tool| tool.label.name.clone())
            .unwrap_or_else(|| tool_call.function_name.clone()),
        source: tool
            .map(|tool| tool.label.source.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        server_id: tool.and_then(|tool| tool.label.server_id.clone()),
        arguments: tool_call.arguments_raw.clone(),
        status: AgentToolStatus::Cancelled,
        output: None,
        output_limit: None,
        error: Some("Tool call cancelled".to_string()),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round: ctx.round,
        sensitive: tool.map(|tool| tool.label.sensitive).unwrap_or(false),
        artifacts: Vec::new(),
        draft: None,
        structured_content: None,
    };
    host.tool_updated(&record);
    ToolExecutionResult {
        response_message: tool_message(tool_call.id.clone(), "Tool call cancelled"),
        record: Some(record),
        cancelled: true,
        follow_up_messages: Vec::new(),
    }
}

fn unknown_or_disabled_tool_result(
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    ctx: &ToolRoundContext<'_>,
    tools: &[ToolDefinition],
    blocked_tool_calls: &[ToolDefinition],
    tool_call: PendingToolCall,
) -> ToolExecutionResult {
    if let Some(tool) = match_tool_call(blocked_tool_calls, &tool_call.function_name) {
        let error = executor.blocked_feedback(tool);
        let record = skipped_tool_record(ctx, &tool_call, tool, error.clone());
        host.tool_updated(&record);
        return ToolExecutionResult {
            response_message: tool_message(tool_call.id, error),
            record: Some(record),
            cancelled: false,
            follow_up_messages: Vec::new(),
        };
    }

    let disabled = executor.unknown_feedback(&tool_call.function_name);
    let record = if disabled.is_none() {
        let error = format!("Unknown tool requested: {}", tool_call.function_name);
        let record = unknown_tool_record(&tool_call, ctx.round, error);
        host.tool_updated(&record);
        Some(record)
    } else {
        None
    };
    // 喂回自愈（对齐 opencode）：错误作为 tool result 返回时附上已声明工具清单，
    // 让模型下一轮自我纠正（Cursor 系模型会间歇性按训练时的工具名出牌，如大写 Grep）。
    let mut available: Vec<String> = tools.iter().map(|tool| tool.model.name.clone()).collect();
    available.sort();
    available.dedup();
    let reason = disabled.unwrap_or_else(|| format!("Unknown tool: {}.", tool_call.function_name));
    let content = if available.is_empty() {
        format!("{reason} Available tools: none in this request. Explain the limitation using the available context.")
    } else {
        format!("{reason} Available tools: {}. Use a declared tool with its exact name if it can perform the requested work.", available.join(", "))
    };
    ToolExecutionResult {
        response_message: tool_message(tool_call.id, content),
        record,
        cancelled: false,
        follow_up_messages: Vec::new(),
    }
}

fn skipped_tool_record(
    ctx: &ToolRoundContext<'_>,
    call: &PendingToolCall,
    tool: &ToolDefinition,
    error: String,
) -> AgentToolCall {
    let now = chrono::Local::now().timestamp();
    AgentToolCall {
        id: call.id.clone(),
        name: tool.label.name.clone(),
        source: tool.label.source.clone(),
        server_id: tool.label.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: AgentToolStatus::Skipped,
        output: None,
        output_limit: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round: ctx.round,
        sensitive: tool.label.sensitive,
        artifacts: Vec::new(),
        draft: None,
        structured_content: None,
    }
}

fn invalid_tool_arguments_result(
    host: &super::context::AgentContext<'_>,
    ctx: &ToolRoundContext<'_>,
    tool_call: &PendingToolCall,
    tool: &ToolDefinition,
    error: String,
) -> ToolExecutionResult {
    // finish_reason == "length" means the model hit max_output_tokens and the
    // tool-call JSON was cut off mid-stream — not malformed JSON. Telling the
    // model to "retry compact" is misleading; name the real cause so it (and
    // the user) can react: split a large write, or raise max output tokens.
    let truncated = ctx.finish_reason == Some(FinishKind::Length);
    let (record_error, model_hint) = if truncated {
        (
            format!(
                "Output truncated: hit the max output token limit (finish_reason=length), \
                 so the tool-call arguments are incomplete. Original error: {error}"
            ),
            "Your previous response was cut off at the max output token limit \
             (finish_reason=length), so this tool call's arguments are incomplete. \
             Don't retry the same oversized call — produce smaller output (e.g. write \
             the file in several smaller chunks via multiple calls), or ask the user to \
             raise the chat \"max output tokens\" setting for this model."
                .to_string(),
        )
    } else {
        (
            error,
            "Tool arguments JSON is invalid or incomplete. Retry this tool call with a \
             compact, valid JSON object for arguments."
                .to_string(),
        )
    };
    let record = invalid_tool_arguments_record(tool_call, tool, ctx.round, record_error);
    host.tool_updated(&record);
    ToolExecutionResult {
        response_message: tool_message(tool_call.id.clone(), model_hint),
        record: Some(record),
        cancelled: false,
        follow_up_messages: Vec::new(),
    }
}

/// finish_reason=="length" 的整批作废结果：不执行，只给模型一条「参数可能被截断，
/// 请重发」的错误结果（每个 tool_call_id 都必须被应答，缺一条下一轮请求直接 400）。
fn truncated_tool_call_result(
    host: &super::context::AgentContext<'_>,
    ctx: &ToolRoundContext<'_>,
    tools: &[ToolDefinition],
    tool_call: PendingToolCall,
) -> ToolExecutionResult {
    let error = "Not executed: the response hit the max output token limit \
                 (finish_reason=length), so the tool-call arguments may be truncated."
        .to_string();
    let model_hint = format!(
        "Tool call \"{}\" was not executed: the response hit the output token limit, so its \
         arguments may be truncated. Re-issue the tool call with complete arguments — and if \
         the arguments themselves are large (e.g. a big file write), produce smaller output \
         by splitting the work across several calls.",
        tool_call.function_name
    );
    let record = match match_tool_call(tools, &tool_call.function_name) {
        Some(tool) => invalid_tool_arguments_record(&tool_call, tool, ctx.round, error),
        None => unknown_tool_record(&tool_call, ctx.round, error),
    };
    host.tool_updated(&record);
    ToolExecutionResult {
        response_message: tool_message(tool_call.id, model_hint),
        record: Some(record),
        cancelled: false,
        follow_up_messages: Vec::new(),
    }
}

pub(crate) fn tool_message(tool_call_id: String, content: impl Into<String>) -> Value {
    serde_json::json!({
        "role": "tool",
        "tool_call_id": tool_call_id,
        "content": content.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn follow_up_messages_are_appended_after_tool_result() {
        // Mirrors `read` feeding an image to a vision model: the tool result is
        // text, the actual image rides as a follow-up user message that must
        // land right after the tool message so the tool_call_id is answered.
        let image_msg = json!({
            "role": "user",
            "content": [{
                "type": "image_url",
                "image_url": { "url": "data:image/png;base64,AAAA" }
            }]
        });
        let result = ToolExecutionResult {
            response_message: tool_message("call_1".to_string(), "已读取图片 x.png，见下方。"),
            record: None,
            cancelled: false,
            follow_up_messages: vec![image_msg.clone()],
        };
        let mut response_messages = Vec::new();
        let mut tool_records = Vec::new();
        let cancelled =
            push_tool_execution_result(result, &mut response_messages, &mut tool_records);

        assert!(!cancelled);
        assert_eq!(response_messages.len(), 2);
        assert_eq!(response_messages[0]["role"], "tool");
        assert_eq!(response_messages[0]["tool_call_id"], "call_1");
        assert_eq!(response_messages[1], image_msg);
    }

    #[test]
    fn no_follow_up_leaves_single_tool_result() {
        let result = ToolExecutionResult {
            response_message: tool_message("call_2".to_string(), "plain text"),
            record: None,
            cancelled: false,
            follow_up_messages: Vec::new(),
        };
        let mut response_messages = Vec::new();
        let mut tool_records = Vec::new();
        push_tool_execution_result(result, &mut response_messages, &mut tool_records);

        assert_eq!(response_messages.len(), 1);
        assert_eq!(response_messages[0]["role"], "tool");
    }
}
