use std::{
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

use serde_json::Value;
use tokio::time::timeout;

use crate::chat::ask_user;
use crate::chat::model::PendingToolCall;
use crate::chat::types::{ToolCallRecord, ToolCallStatus};
use crate::mcp::types::McpToolCallResult;
use crate::mcp::ChatToolDefinition;
use crate::settings::Settings;
use crate::skills;

use super::host::AgentHost;
use super::prepare::{builtin_tool_bypasses_approval, disabled_builtin_tool_feedback};

pub type ToolExecutorFuture<'a> =
    Pin<Box<dyn Future<Output = Result<McpToolCallResult, String>> + Send + 'a>>;

pub trait ToolExecutor: Send + Sync {
    fn call<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ChatToolDefinition,
        arguments: Value,
        skill_cache: Option<&'a mut skills::SkillRunCache>,
    ) -> ToolExecutorFuture<'a>;
}

pub struct ToolExecutionContext<'a> {
    pub conversation_id: &'a str,
    pub run_id: &'a str,
    pub message_id: &'a str,
    pub generation: u64,
    pub round: u32,
    /// Sub-agent nesting depth: 0 for a top-level chat run, +1 per spawned
    /// sub-agent. Used to gate `agent` spawns (`MAX_SUB_AGENT_DEPTH`) and to
    /// reject sensitive tools inside sub-agents.
    pub depth: u8,
    /// Conversation that conversation-scoped tools (todo, native file
    /// workspace) operate on. Equals `conversation_id` for a top-level run; for
    /// a sub-agent it is the PARENT conversation so the sub-agent can claim the
    /// parent's todos and resolve the parent's project workspace, while
    /// `conversation_id` stays a synthetic id used only for generation/streaming
    /// isolation.
    pub tool_conversation_id: &'a str,
    /// The model-issued tool_call id of the call currently executing. Sub-agent
    /// progress is reported back onto the parent tool card addressed by this id.
    pub tool_call_id: &'a str,
}

pub fn match_tool_call<'a>(
    tools: &'a [ChatToolDefinition],
    function_name: &str,
) -> Option<&'a ChatToolDefinition> {
    tools
        .iter()
        .find(|tool| tool.openai_tool_name() == function_name || tool.name == function_name)
}

pub fn unknown_tool_record(call: &PendingToolCall, round: u32, error: String) -> ToolCallRecord {
    let now = chrono::Local::now().timestamp();
    ToolCallRecord {
        id: call.id.clone(),
        name: call.function_name.clone(),
        source: "unknown".to_string(),
        server_id: None,
        arguments: call.arguments_raw.clone(),
        status: ToolCallStatus::Error,
        result_preview: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round,
        sensitive: false,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: None,
    }
}

pub fn invalid_tool_arguments_record(
    call: &PendingToolCall,
    tool: &ChatToolDefinition,
    round: u32,
    error: String,
) -> ToolCallRecord {
    let now = chrono::Local::now().timestamp();
    ToolCallRecord {
        id: call.id.clone(),
        name: tool.name.clone(),
        source: tool.source.clone(),
        server_id: tool.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: ToolCallStatus::Error,
        result_preview: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round,
        sensitive: tool.sensitive,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: None,
    }
}

pub async fn execute_tool_call(
    host: &dyn AgentHost,
    executor: &dyn ToolExecutor,
    settings: &Settings,
    ctx: &ToolExecutionContext<'_>,
    tool: &ChatToolDefinition,
    call: PendingToolCall,
    skill_cache: Option<&mut skills::SkillRunCache>,
) -> (ToolCallRecord, String) {
    let now = chrono::Local::now().timestamp();
    let mut record = ToolCallRecord {
        id: call.id.clone(),
        name: tool.name.clone(),
        source: tool.source.clone(),
        server_id: tool.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: ToolCallStatus::Pending,
        result_preview: None,
        error: None,
        duration_ms: None,
        started_at: Some(now),
        completed_at: None,
        round: ctx.round,
        sensitive: tool.sensitive,
        artifacts: Vec::new(),
        trace_id: Some(ctx.run_id.to_string()),
        span_id: Some(tool_span_id(ctx.round, &call.id)),
        structured_content: None,
    };
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);

    if let Err(err) = validate_tool_arguments(tool, &call.arguments) {
        record.status = ToolCallStatus::Error;
        record.duration_ms = Some(0);
        record.completed_at = Some(chrono::Local::now().timestamp());
        record.error = Some(err.clone());
        host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
        return (
            record,
            format!("Tool arguments failed schema validation: {err}. Retry this tool call with arguments that match the declared JSON schema."),
        );
    }

    if tool.source == "native" && ask_user::is_ask_user_tool_name(&tool.name) {
        return execute_ask_user_call(host, settings, ctx, record, call.arguments.clone()).await;
    }

    // Gate. The file/shell tools (read/write/edit/bash/grep/find/ls) are
    // governed by a single per-conversation **session consent**: prompt once,
    // then run freely with full-disk access for the rest of the conversation.
    // `approval_policy`: "auto" implicitly consents (no prompt); "always_confirm"
    // confirms each call (the per-call prompt IS the gate — no separate session
    // consent); the default prompts once per conversation. Everything else (MCP
    // tools, etc.) keeps the existing per-call approval.
    let skip = |record: &mut ToolCallRecord, reason: &str| {
        record.status = ToolCallStatus::Skipped;
        record.completed_at = Some(chrono::Local::now().timestamp());
        record.error = Some(reason.to_string());
    };

    if super::prepare::tool_requires_session_consent(tool) {
        let policy = settings.chat_tools.approval_policy.as_str();
        if policy == "always_confirm" {
            if !host.request_tool_approval(ctx, &record).await {
                skip(&mut record, "Tool call was not approved");
                host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
                let content = record.error.clone().unwrap_or_default();
                return (record, content);
            }
        } else if policy != "auto" && !host.request_session_consent(ctx).await {
            skip(&mut record, "用户未授权本会话使用文件 / 命令工具");
            host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
            let content = record.error.clone().unwrap_or_default();
            return (record, content);
        }
    } else if tool_requires_approval(settings, tool) {
        let approved = host.request_tool_approval(ctx, &record).await;
        if !approved {
            skip(&mut record, "Tool call was not approved");
            host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
            let content = record.error.clone().unwrap_or_default();
            return (record, content);
        }
    }

    record.status = ToolCallStatus::Running;
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
    let started = Instant::now();
    let timeout_ms = effective_tool_timeout_ms(settings, tool, &call.arguments);
    let result = tokio::select! {
        result = timeout(
            Duration::from_millis(timeout_ms),
            executor.call(ctx, tool, call.arguments.clone(), skill_cache),
        ) => result,
        _ = host.wait_for_generation_inactive(ctx.conversation_id, ctx.generation) => {
            record.status = ToolCallStatus::Cancelled;
            record.duration_ms = Some(started.elapsed().as_millis() as u64);
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some("Tool call cancelled".to_string());
            host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
            let content = record.error.clone().unwrap_or_default();
            return (record, content);
        }
    };
    record.duration_ms = Some(started.elapsed().as_millis() as u64);
    record.completed_at = Some(chrono::Local::now().timestamp());
    let max_tool_output_chars = settings.chat_tools.max_tool_output_chars;
    let tool_content = match result {
        Ok(Ok(output)) if !output.is_error => {
            record.status = ToolCallStatus::Success;
            record.artifacts = output.artifacts.clone();
            record.structured_content = output.structured_content.clone();
            record.result_preview = Some(limit_tool_text_for_model(
                &format_tool_result_preview(&tool_content_with_structured_output(
                    &output,
                    &tool.source,
                )),
                max_tool_output_chars,
            ));
            limit_tool_text_for_model(
                &tool_content_with_structured_output(&output, &tool.source),
                max_tool_output_chars,
            )
        }
        Ok(Ok(output)) => {
            record.status = ToolCallStatus::Error;
            record.structured_content = output.structured_content.clone();
            record.error = Some(truncate_chars(&output.content, 1000));
            limit_tool_text_for_model(
                &tool_content_with_structured_output(&output, &tool.source),
                max_tool_output_chars,
            )
        }
        Ok(Err(err)) => {
            record.status = ToolCallStatus::Error;
            record.error = Some(err.clone());
            limit_tool_text_for_model(&err, max_tool_output_chars)
        }
        Err(_) => {
            record.status = ToolCallStatus::Error;
            let err =
                format!("工具调用超时（{timeout_ms}ms）。请缩小任务，或在设置中调高工具超时时间。");
            record.error = Some(err.clone());
            err
        }
    };
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
    (record, tool_content)
}

async fn execute_ask_user_call(
    host: &dyn AgentHost,
    settings: &Settings,
    ctx: &ToolExecutionContext<'_>,
    mut record: ToolCallRecord,
    arguments: Value,
) -> (ToolCallRecord, String) {
    let prompt = match ask_user::normalize_prompt(arguments) {
        Ok(prompt) => prompt,
        Err(err) => {
            record.status = ToolCallStatus::Error;
            record.duration_ms = Some(0);
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some(err.clone());
            host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
            return (
                record,
                format!("Invalid ask_user prompt: {err}. Retry with a valid questions payload."),
            );
        }
    };

    record.status = ToolCallStatus::Running;
    record.structured_content = Some(ask_user::structured_content(
        &prompt,
        ask_user::ASK_USER_PHASE_AWAITING,
        &Default::default(),
    ));
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);

    let started = Instant::now();
    let response = host
        .request_user_response(ctx, &record, prompt.clone())
        .await;
    record.duration_ms = Some(started.elapsed().as_millis() as u64);
    record.completed_at = Some(chrono::Local::now().timestamp());
    record.structured_content = Some(ask_user::structured_content(
        &prompt,
        &response.phase,
        &response.answers,
    ));
    let content = ask_user::tool_result_content(&response);
    match response.phase.as_str() {
        ask_user::ASK_USER_PHASE_ANSWERED => {
            record.status = ToolCallStatus::Success;
            record.result_preview = Some("User answered clarification questions".to_string());
        }
        ask_user::ASK_USER_PHASE_SKIPPED => {
            record.status = ToolCallStatus::Skipped;
            record.result_preview = Some("User skipped clarification".to_string());
        }
        ask_user::ASK_USER_PHASE_TIMEOUT => {
            record.status = ToolCallStatus::Skipped;
            record.result_preview = Some("Clarification timed out".to_string());
        }
        ask_user::ASK_USER_PHASE_CANCELLED => {
            record.status = ToolCallStatus::Cancelled;
            record.error = Some("Clarification cancelled".to_string());
        }
        _ => {
            record.status = ToolCallStatus::Error;
            record.error = Some(format!(
                "Invalid ask_user response phase: {}",
                response.phase
            ));
        }
    }
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);
    (
        record,
        limit_tool_text_for_model(&content, settings.chat_tools.max_tool_output_chars),
    )
}

pub fn disabled_tool_content(call: &PendingToolCall) -> Option<String> {
    disabled_builtin_tool_feedback(&call.function_name)
}

pub fn tool_requires_approval(settings: &Settings, tool: &ChatToolDefinition) -> bool {
    if builtin_tool_bypasses_approval(tool) {
        return false;
    }
    match settings.chat_tools.approval_policy.as_str() {
        "auto" => false,
        "always_confirm" => true,
        _ if tool.source == "mcp" => {
            if tool.destructive_hint() == Some(true)
                || tool.open_world_hint() == Some(true)
                || tool.read_only_hint() == Some(false)
            {
                return true;
            }
            if tool.read_only_hint() == Some(true) {
                return false;
            }
            tool.sensitive
        }
        _ => tool.sensitive,
    }
}

fn tool_span_id(round: u32, tool_call_id: &str) -> String {
    format!("tool_round_{round}_{tool_call_id}")
}

pub fn validate_tool_arguments(tool: &ChatToolDefinition, arguments: &Value) -> Result<(), String> {
    validate_schema_value(&tool.input_schema, arguments, "arguments")
}

fn validate_schema_value(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if schema.is_null() || schema.as_object().is_some_and(|object| object.is_empty()) {
        return Ok(());
    }
    if let Some(options) = schema.get("anyOf").and_then(Value::as_array) {
        if options
            .iter()
            .any(|option| validate_schema_value(option, value, path).is_ok())
        {
            return Ok(());
        }
        return Err(format!("{path} does not match any allowed schema"));
    }
    if let Some(options) = schema.get("oneOf").and_then(Value::as_array) {
        let matches = options
            .iter()
            .filter(|option| validate_schema_value(option, value, path).is_ok())
            .count();
        if matches == 1 {
            return Ok(());
        }
        return Err(format!("{path} must match exactly one allowed schema"));
    }
    if let Some(types) = schema.get("type") {
        validate_schema_type(types, value, path)?;
    }
    if let Some(enum_values) = schema.get("enum").and_then(Value::as_array) {
        if !enum_values.iter().any(|candidate| candidate == value) {
            return Err(format!("{path} must be one of the declared enum values"));
        }
    }
    if let Some(object) = value.as_object() {
        validate_object_schema(schema, object, path)?;
    }
    if let Some(items) = schema.get("items") {
        if let Some(array) = value.as_array() {
            for (idx, item) in array.iter().enumerate() {
                validate_schema_value(items, item, &format!("{path}[{idx}]"))?;
            }
        }
    }
    if let Some(max_items) = schema.get("maxItems").and_then(Value::as_u64) {
        if value
            .as_array()
            .is_some_and(|array| array.len() as u64 > max_items)
        {
            return Err(format!("{path} must contain at most {max_items} items"));
        }
    }
    validate_numeric_range(schema, value, path)?;
    Ok(())
}

fn validate_schema_type(types: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(type_name) = types.as_str() {
        if value_matches_schema_type(type_name, value) {
            return Ok(());
        }
        return Err(format!("{path} must be {type_name}"));
    }
    if let Some(type_names) = types.as_array() {
        if type_names
            .iter()
            .filter_map(Value::as_str)
            .any(|type_name| value_matches_schema_type(type_name, value))
        {
            return Ok(());
        }
        return Err(format!("{path} has an invalid type"));
    }
    Ok(())
}

fn value_matches_schema_type(type_name: &str, value: &Value) -> bool {
    match type_name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        _ => true,
    }
}

fn validate_object_schema(
    schema: &Value,
    object: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<(), String> {
    let properties = schema.get("properties").and_then(Value::as_object);
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                return Err(format!("{path}.{key} is required"));
            }
        }
    }
    if let Some(properties) = properties {
        for (key, child_schema) in properties {
            if let Some(child_value) = object.get(key) {
                validate_schema_value(child_schema, child_value, &format!("{path}.{key}"))?;
            }
        }
    }
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        if let Some(extra_key) = object.keys().find(|key| match properties {
            Some(properties) => !properties.contains_key(*key),
            None => true,
        }) {
            return Err(format!("{path}.{extra_key} is not allowed"));
        }
    }
    Ok(())
}

fn validate_numeric_range(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    let Some(number) = value.as_f64() else {
        return Ok(());
    };
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
        if number < minimum {
            return Err(format!("{path} must be at least {minimum}"));
        }
    }
    if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64) {
        if number > maximum {
            return Err(format!("{path} must be at most {maximum}"));
        }
    }
    Ok(())
}

/// MCP 契约要求 structuredContent 进入模型可见文本（除非 text 已含同样 JSON）。
/// native 工具不受此约束：它们的 content 都是为模型精心格式化的文本（read_file 的
/// cat -n、文件变更的 summary+diff、todo/memory 的排版文本），再拼整包 JSON 只会
/// 让 token 翻倍（read_file 行号化后 content 不再等于 JSON，旧逻辑会整包重复追加）。
fn tool_content_with_structured_output(output: &McpToolCallResult, source: &str) -> String {
    if source == "native" {
        return output.content.clone();
    }
    let Some(structured_content) = output.structured_content.as_ref() else {
        return output.content.clone();
    };
    let structured_json = serde_json::to_string(structured_content).unwrap_or_default();
    if structured_json.is_empty() || output.content.contains(&structured_json) {
        return output.content.clone();
    }
    if output.content.trim().is_empty() {
        return structured_json;
    }
    format!(
        "{}\n\nstructuredContent: {}",
        output.content, structured_json
    )
}

fn effective_tool_timeout_ms(
    settings: &Settings,
    tool: &ChatToolDefinition,
    arguments: &Value,
) -> u64 {
    let default_timeout_ms = settings.chat_tools.tool_timeout_ms;
    if tool.source == "mixer" && tool.name == "mixer_generate_image" {
        return default_timeout_ms.max(crate::chat::image_generation::IMAGE_GENERATION_TIMEOUT_MS);
    }
    if tool.source == "skill" && tool.name == "skill_run_script" {
        return crate::mcp::registry::effective_skill_script_timeout_ms(
            default_timeout_ms,
            arguments.get("timeout_ms").and_then(|value| value.as_u64()),
        );
    }
    if tool.source == "native" && matches!(tool.name.as_str(), "bash" | "run_python") {
        return arguments
            .get("timeout_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(default_timeout_ms)
            .clamp(1_000, 300_000)
            .max(default_timeout_ms);
    }
    // The `agent` spawn tool runs a whole sub-agent loop whose own budget
    // (SUB_AGENT_SYNC_TIMEOUT_SECS × SUB_AGENT_MAX_ATTEMPTS) is far longer than the
    // default generic tool timeout (120s). Without a longer outer timeout, the
    // generic 120s would fire first and mis-kill a still-running sub-agent. Give
    // the outer call a large backstop and let the sub-agent's inner timeout +
    // cascade cancel govern its lifecycle. check_agent_result / list_agent_tasks
    // are instant queries, so they intentionally keep the default timeout.
    if tool.source == "native" && tool.name == crate::chat::sub_agent::AGENT_TOOL_NAME {
        return crate::chat::sub_agent::SUB_AGENT_TOOL_TIMEOUT_MS.max(default_timeout_ms);
    }
    default_timeout_ms
}

pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn limit_tool_text_for_model(value: &str, max_chars: Option<usize>) -> String {
    match max_chars {
        Some(max_chars) => truncate_tool_content_for_model(value, max_chars),
        None => value.to_string(),
    }
}

/// 头+尾保留式截断：前 1/2 预算 + 后 1/4 预算，中间替换为截断说明。
/// 尾部保留是关键——编译错误、测试失败摘要通常在长输出的末尾。
fn truncate_tool_content_for_model(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let head_chars = max_chars / 2;
    let tail_chars = max_chars / 4;
    let head: String = value.chars().take(head_chars).collect();
    let tail: String = value
        .chars()
        .skip(char_count.saturating_sub(tail_chars))
        .collect();
    let omitted = char_count - head_chars - tail_chars;
    format!(
        "{head}\n\n[Tool output truncated: original {char_count} chars, showing first {head_chars} and last {tail_chars}; {omitted} chars omitted.]\n\n{tail}"
    )
}

fn format_tool_result_preview(content: &str) -> String {
    let trimmed = content.trim();
    let json_str = trimmed
        .strip_prefix("stdout:")
        .map(str::trim)
        .unwrap_or(trimmed);
    let Ok(value) = serde_json::from_str::<Value>(json_str) else {
        return content.to_string();
    };

    if let Some(answer) = value
        .get("answer")
        .and_then(|answer| answer.as_str())
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
    {
        return format!("答: {answer}");
    }

    let Some(results) = value.get("results").and_then(|results| results.as_array()) else {
        return content.to_string();
    };

    if results.is_empty() {
        return "无搜索结果".to_string();
    }

    let query = value
        .get("query")
        .and_then(|query| query.as_str())
        .unwrap_or_default();
    let query_label = if query.is_empty() {
        String::new()
    } else {
        format!("「{query}」")
    };
    let first = &results[0];
    let title = first
        .get("title")
        .or_else(|| first.get("url"))
        .and_then(|title| title.as_str())
        .unwrap_or_default();
    let snippet = first
        .get("content")
        .or_else(|| first.get("raw_content"))
        .and_then(|content| content.as_str())
        .unwrap_or_default();
    let snippet: String = snippet.chars().take(80).collect();
    let head = format!("{} 条结果{query_label}", results.len());
    if title.is_empty() && snippet.is_empty() {
        return head;
    }
    if snippet.is_empty() {
        return format!("{head}: {title}");
    }
    format!("{head}: {title} - {snippet}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    use crate::chat::agent::host::AgentHostFuture;
    use crate::chat::types::{ChatMessageSegment, ToolCallStatus};
    use crate::mcp::types::native_skill_activate_tool;

    #[derive(Default)]
    struct ExecuteTestHost {
        approvals: AtomicUsize,
        consents: AtomicUsize,
        deny_consent: bool,
        records: Mutex<Vec<ToolCallRecord>>,
    }

    impl AgentHost for ExecuteTestHost {
        fn emit_stream_delta(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _delta: &str,
            _reasoning_delta: Option<&str>,
            _segment: Option<&ChatMessageSegment>,
        ) {
        }

        fn emit_stream_done(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            _reason: &str,
            _full: &str,
        ) {
        }

        fn emit_tool_record(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _message_id: &str,
            record: &ToolCallRecord,
        ) {
            self.records
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .push(record.clone());
        }

        fn request_tool_approval<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
        ) -> AgentHostFuture<'a, bool> {
            self.approvals.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { true })
        }

        fn request_session_consent<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
        ) -> AgentHostFuture<'a, bool> {
            self.consents.fetch_add(1, Ordering::SeqCst);
            let granted = !self.deny_consent;
            Box::pin(async move { granted })
        }

        fn request_user_response<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            _record: &'a ToolCallRecord,
            _prompt: crate::chat::ask_user::AskUserPromptPayload,
        ) -> AgentHostFuture<'a, crate::chat::ask_user::AskUserResponseResult> {
            Box::pin(async { crate::chat::ask_user::skipped_response() })
        }

        fn is_generation_active(&self, _conversation_id: &str, _generation: u64) -> bool {
            true
        }

        fn wait_for_generation_inactive<'a>(
            &'a self,
            _conversation_id: &'a str,
            _generation: u64,
        ) -> AgentHostFuture<'a, ()> {
            Box::pin(async { std::future::pending::<()>().await })
        }
    }

    #[derive(Default)]
    struct ExecuteTestExecutor {
        calls: AtomicUsize,
        structured_content: Option<Value>,
    }

    impl ToolExecutor for ExecuteTestExecutor {
        fn call<'a>(
            &'a self,
            _ctx: &'a ToolExecutionContext<'a>,
            tool: &'a ChatToolDefinition,
            _arguments: Value,
            _skill_cache: Option<&'a mut skills::SkillRunCache>,
        ) -> ToolExecutorFuture<'a> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let content = format!("result:{}", tool.name);
            let structured_content = self.structured_content.clone();
            Box::pin(async move {
                Ok(McpToolCallResult {
                    content,
                    is_error: false,
                    raw: Value::Null,
                    artifacts: Vec::new(),
                    structured_content,
                })
            })
        }
    }

    fn test_execution_context() -> ToolExecutionContext<'static> {
        ToolExecutionContext {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation: 1,
            round: 2,
            depth: 0,
            tool_conversation_id: "conversation",
            tool_call_id: "call",
        }
    }

    fn test_pending_call(id: &str, function_name: &str, arguments: Value) -> PendingToolCall {
        PendingToolCall {
            id: id.to_string(),
            function_name: function_name.to_string(),
            arguments_raw: serde_json::to_string(&arguments).expect("serialize test args"),
            arguments,
            arguments_parse_error: None,
        }
    }

    fn sensitive_test_tool() -> ChatToolDefinition {
        ChatToolDefinition {
            id: "native__write_file".to_string(),
            name: "write".to_string(),
            description: "Write file".to_string(),
            source: "native".to_string(),
            server_id: None,
            server_name: Some("Kivio".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            sensitive: true,
            annotations: None,
            output_schema: None,
        }
    }

    #[test]
    fn unknown_tool_record_is_error_metadata() {
        let call = PendingToolCall {
            id: "call_1".to_string(),
            function_name: "missing".to_string(),
            arguments: Value::Null,
            arguments_raw: "{}".to_string(),
            arguments_parse_error: None,
        };

        let record = unknown_tool_record(&call, 2, "Unknown tool requested: missing".to_string());

        assert!(matches!(record.status, ToolCallStatus::Error));
        assert_eq!(record.round, 2);
        assert_eq!(record.source, "unknown");
    }

    #[tokio::test]
    async fn schema_validation_fails_before_approval_or_execution() {
        let host = ExecuteTestHost::default();
        let executor = ExecuteTestExecutor::default();
        let settings = Settings::default();
        let tool = sensitive_test_tool();
        let call = test_pending_call("call_invalid", "write", serde_json::json!({}));

        let (record, content) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
            None,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Error));
        assert!(record
            .error
            .as_deref()
            .is_some_and(|error| error.contains("arguments.path is required")));
        assert!(content.contains("schema validation"));
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
        let records = host.records.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(records.len(), 2);
        assert!(matches!(records[0].status, ToolCallStatus::Pending));
        assert!(matches!(records[1].status, ToolCallStatus::Error));
    }

    #[test]
    fn schema_validation_rejects_additional_properties_without_declared_properties() {
        let mut tool = sensitive_test_tool();
        tool.input_schema = serde_json::json!({
            "type": "object",
            "additionalProperties": false
        });

        let err = validate_tool_arguments(&tool, &serde_json::json!({ "extra": true }))
            .expect_err("extra properties should be rejected");

        assert!(err.contains("arguments.extra is not allowed"));
    }

    #[tokio::test]
    async fn successful_tool_records_trace_and_structured_content() {
        let host = ExecuteTestHost::default();
        let executor = ExecuteTestExecutor {
            calls: AtomicUsize::new(0),
            structured_content: Some(serde_json::json!({ "answer": 42 })),
        };
        let settings = Settings::default();
        let mut tool = sensitive_test_tool();
        tool.sensitive = false;
        let call = test_pending_call(
            "call_ok",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hello" }),
        );

        let (record, content) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
            None,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Success));
        assert_eq!(record.trace_id.as_deref(), Some("run"));
        assert_eq!(record.span_id.as_deref(), Some("tool_round_2_call_ok"));
        assert_eq!(
            record.structured_content.as_ref(),
            Some(&serde_json::json!({ "answer": 42 }))
        );
        // native 工具的 content 是为模型格式化的文本，不再追加整包 structuredContent
        // JSON（record/前端仍持有完整 structured_content）。MCP 契约见下一个测试。
        assert!(!content.contains("structuredContent"));
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn file_tool_skipped_when_session_consent_denied() {
        let host = ExecuteTestHost {
            deny_consent: true,
            ..Default::default()
        };
        let executor = ExecuteTestExecutor::default();
        let settings = Settings::default();
        let tool = sensitive_test_tool(); // native "write" — consent-gated
        let call = test_pending_call(
            "call_write",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hi" }),
        );

        let (record, _content) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
            None,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Skipped));
        assert_eq!(host.consents.load(Ordering::SeqCst), 1);
        // No per-call approval under default policy, and the tool never ran.
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn file_tool_uses_session_consent_not_per_call_approval() {
        let host = ExecuteTestHost::default(); // grants consent
        let executor = ExecuteTestExecutor::default();
        let settings = Settings::default(); // default policy: consent, no per-call confirm
        let tool = sensitive_test_tool();
        let call = test_pending_call(
            "call_write",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hi" }),
        );

        let (record, _content) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
            None,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Success));
        assert_eq!(host.consents.load(Ordering::SeqCst), 1);
        assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    /// MCP structured-content 契约（agent-runtime spec）：MCP 结果的 structuredContent
    /// 必须进入模型可见文本（除非文本已含同样 JSON）。
    #[tokio::test]
    async fn mcp_tool_content_still_carries_structured_content() {
        let host = ExecuteTestHost::default();
        let executor = ExecuteTestExecutor {
            calls: AtomicUsize::new(0),
            structured_content: Some(serde_json::json!({ "answer": 42 })),
        };
        let settings = Settings::default();
        let mut tool = sensitive_test_tool();
        tool.sensitive = false;
        tool.source = "mcp".to_string();
        tool.server_id = Some("server-1".to_string());
        let call = test_pending_call(
            "call_mcp",
            "write",
            serde_json::json!({ "path": "/tmp/out.txt", "content": "hello" }),
        );

        let (record, content) = execute_tool_call(
            &host,
            &executor,
            &settings,
            &test_execution_context(),
            &tool,
            call,
            None,
        )
        .await;

        assert!(matches!(record.status, ToolCallStatus::Success));
        assert!(content.contains("structuredContent"));
        assert!(content.contains("\"answer\":42"));
    }

    #[test]
    fn native_skill_tools_match_by_openai_name() {
        let tool = native_skill_activate_tool();
        assert!(match_tool_call(&[tool], "skill_activate").is_some());
    }

    #[test]
    fn format_tool_result_preview_summarizes_tavily_search_json() {
        let raw = r#"stdout:
{
  "answer": null,
  "query": "吉林市 明天 天气",
  "results": [
    {
      "title": "吉林市天气预报",
      "content": "明天晴有时多云，最高33℃"
    }
  ]
}"#;
        let preview = format_tool_result_preview(raw);
        assert!(preview.contains("1 条结果"));
        assert!(preview.contains("吉林市天气预报"));
        assert!(preview.contains("33"));
        assert!(!preview.contains("\"answer\": null"));
    }

    #[test]
    fn truncate_tool_content_for_model_marks_truncated_output() {
        // 头+尾保留：12 字符预算 → 头 6 + 尾 3，中间替换为截断说明。
        let content = "HEADxxOMITTEDyyTAIL_abc";
        let truncated = truncate_tool_content_for_model(content, 12);

        assert!(truncated.starts_with("HEADxx"));
        assert!(truncated.ends_with("abc"));
        assert!(truncated.contains("Tool output truncated"));
        assert!(truncated.contains("original 23 chars"));
        assert!(truncated.contains("first 6 and last 3"));
        assert!(!truncated.contains("OMITTEDyy"));
    }

    #[test]
    fn truncate_tool_content_for_model_keeps_short_output_unchanged() {
        assert_eq!(truncate_tool_content_for_model("abc", 3), "abc");
    }

    #[test]
    fn skill_run_script_timeout_uses_minimum_even_when_model_requests_less() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::native_skill_run_script_tool();
        let arguments = serde_json::json!({ "timeout_ms": 60_000 });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            120_000
        );
    }

    #[test]
    fn skill_run_script_timeout_clamps_large_model_requests() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::native_skill_run_script_tool();
        let arguments = serde_json::json!({ "timeout_ms": 500_000 });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            300_000
        );
    }

    #[test]
    fn mixer_image_generation_uses_extended_timeout() {
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 60_000;
        let tool = crate::mcp::types::mixer_generate_image_tool();
        let arguments = serde_json::json!({ "prompt": "draw a quiet desktop assistant" });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            crate::chat::image_generation::IMAGE_GENERATION_TIMEOUT_MS
        );
    }

    #[test]
    fn agent_spawn_uses_sub_agent_backstop_timeout() {
        // The `agent` spawn tool must outlast the sub-agent's own run budget so the
        // outer 120s default does not mis-kill a long sub-agent run.
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = 120_000;
        let tool = crate::chat::sub_agent::agent_tool();
        let arguments = serde_json::json!({ "prompt": "do a focused sub-task" });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            crate::chat::sub_agent::SUB_AGENT_TOOL_TIMEOUT_MS
        );
        assert!(
            crate::chat::sub_agent::SUB_AGENT_TOOL_TIMEOUT_MS > 120_000,
            "agent timeout must exceed the default generic tool timeout"
        );
    }

    #[test]
    fn agent_spawn_respects_larger_user_default_timeout() {
        // If the user configured an even larger generic timeout, honor it.
        let mut settings = Settings::default();
        settings.chat_tools.tool_timeout_ms = crate::chat::sub_agent::SUB_AGENT_TOOL_TIMEOUT_MS + 1;
        let tool = crate::chat::sub_agent::agent_tool();
        let arguments = serde_json::json!({ "prompt": "do a focused sub-task" });

        assert_eq!(
            effective_tool_timeout_ms(&settings, &tool, &arguments),
            crate::chat::sub_agent::SUB_AGENT_TOOL_TIMEOUT_MS + 1
        );
    }
}
