use std::{
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};

use serde_json::Value;
use tokio::time::timeout;

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
        sensitive: false,
        artifacts: Vec::new(),
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
    };
    host.emit_tool_record(ctx.conversation_id, ctx.run_id, ctx.message_id, &record);

    let requires_approval = tool_requires_approval(settings, tool);
    if requires_approval {
        let approved = host.request_tool_approval(ctx, &record).await;
        if !approved {
            record.status = ToolCallStatus::Skipped;
            record.completed_at = Some(chrono::Local::now().timestamp());
            record.error = Some("Tool call was not approved".to_string());
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
            record.result_preview = Some(limit_tool_text_for_model(
                &format_tool_result_preview(&output.content),
                max_tool_output_chars,
            ));
            limit_tool_text_for_model(&output.content, max_tool_output_chars)
        }
        Ok(Ok(output)) => {
            record.status = ToolCallStatus::Error;
            record.error = Some(truncate_chars(&output.content, 1000));
            limit_tool_text_for_model(&output.content, max_tool_output_chars)
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
        _ => tool.sensitive,
    }
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
    if tool.source == "native" && matches!(tool.name.as_str(), "run_command" | "run_python") {
        return arguments
            .get("timeout_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(default_timeout_ms)
            .clamp(1_000, 300_000)
            .max(default_timeout_ms);
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

fn truncate_tool_content_for_model(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str(&format!(
        "\n\n[Tool output truncated: original {char_count} chars, showing first {max_chars}.]"
    ));
    truncated
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
    use crate::chat::types::ToolCallStatus;
    use crate::mcp::types::native_skill_activate_tool;

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
        let content = "abcdef";
        let truncated = truncate_tool_content_for_model(content, 3);

        assert!(truncated.starts_with("abc"));
        assert!(truncated.contains("Tool output truncated"));
        assert!(truncated.contains("original 6 chars"));
        assert!(truncated.contains("first 3"));
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
}
