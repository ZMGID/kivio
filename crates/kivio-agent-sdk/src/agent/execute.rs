use super::{
    content::{AgentToolCall, AgentToolStatus},
    control::AgentRunHandle,
    events::AgentEvents,
};
use crate::{
    model::{ModelArtifact, PendingToolCall},
    tools::{ToolDefinition, ToolOutput},
};
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    time::{Duration, Instant},
};
use tokio::time::timeout;

pub type AgentToolOutput = ToolOutput<Vec<ModelArtifact>>;
pub type ToolExecutorFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AgentToolOutput, String>> + Send + 'a>>;
pub type ToolFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct ToolExecutionContext<'a> {
    pub conversation_id: &'a str,
    pub run_id: &'a str,
    pub message_id: &'a str,
    pub generation: u64,
    pub round: u32,
    pub tool_call_id: &'a str,
}

pub struct ToolProgress<'a> {
    pub control: AgentRunHandle<'a>,
    events: &'a AgentEvents<'a>,
}
impl ToolProgress<'_> {
    pub fn updated(&self, record: &AgentToolCall) {
        self.events
            .emit(super::events::AgentEventKind::ToolUpdated(record));
    }
}

pub struct PreparedToolCall {
    pub record: AgentToolCall,
    pub arguments: Value,
    pub timeout_ms: Option<u64>,
    pub timeout_message: String,
    pub output_limit: Option<usize>,
    pub drain_on_cancel: bool,
    pub feedback: Vec<String>,
}

pub enum ToolPreparation {
    Ready(PreparedToolCall),
    Complete(AgentToolCall, String, Vec<Value>),
}

pub trait ToolExecutor: Send + Sync {
    /// Authorize and validate the call. Before returning `Complete`, publish its final record with `progress.updated`.
    fn prepare<'a>(
        &'a self,
        progress: &'a ToolProgress<'a>,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        call: PendingToolCall,
        record: AgentToolCall,
    ) -> ToolFuture<'a, ToolPreparation>;
    fn call<'a>(
        &'a self,
        ctx: &'a ToolExecutionContext<'a>,
        tool: &'a ToolDefinition,
        arguments: Value,
    ) -> ToolExecutorFuture<'a>;
    fn after<'a>(
        &'a self,
        _progress: &'a ToolProgress<'a>,
        _ctx: &'a ToolExecutionContext<'a>,
        _tool: &'a ToolDefinition,
        _arguments: Value,
        _content: &'a str,
        _feedback: Vec<String>,
    ) -> ToolFuture<'a, String> {
        Box::pin(async { String::new() })
    }
    fn start_speculation(
        &self,
        _ctx: &ToolExecutionContext<'_>,
        _tools: &[ToolDefinition],
    ) -> Option<Box<dyn super::host::SpeculativeTaskScope>> {
        None
    }
    fn unknown_feedback(&self, _name: &str) -> Option<String> {
        None
    }
    fn blocked_feedback(&self, tool: &ToolDefinition) -> String {
        format!("Tool `{}` is unavailable for this run.", tool.model.name)
    }
}

pub fn match_tool_call<'a>(
    tools: &'a [ToolDefinition],
    function_name: &str,
) -> Option<&'a ToolDefinition> {
    if let Some(tool) = tools
        .iter()
        .find(|tool| tool.model.name == function_name || tool.label.name == function_name)
    {
        return Some(tool);
    }
    if let Some(tool) = tools
        .iter()
        .find(|tool| tool.aliases.iter().any(|alias| alias == function_name))
    {
        return Some(tool);
    }
    let mut matches = tools.iter().filter(|tool| {
        tool.model.name.eq_ignore_ascii_case(function_name)
            || tool.label.name.eq_ignore_ascii_case(function_name)
    });
    let matched = matches.next()?;
    matches.next().is_none().then_some(matched)
}

pub fn unknown_tool_record(call: &PendingToolCall, round: u32, error: String) -> AgentToolCall {
    let now = chrono::Local::now().timestamp();
    AgentToolCall {
        id: call.id.clone(),
        name: call.function_name.clone(),
        source: "unknown".to_string(),
        server_id: None,
        arguments: call.arguments_raw.clone(),
        status: AgentToolStatus::Error,
        output: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round,
        sensitive: false,
        artifacts: Vec::new(),
        output_limit: None,
        draft: None,
        structured_content: None,
    }
}

pub fn invalid_tool_arguments_record(
    call: &PendingToolCall,
    tool: &ToolDefinition,
    round: u32,
    error: String,
) -> AgentToolCall {
    let now = chrono::Local::now().timestamp();
    AgentToolCall {
        id: call.id.clone(),
        name: tool.label.name.clone(),
        source: tool.label.source.clone(),
        server_id: tool.label.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: AgentToolStatus::Error,
        output: None,
        error: Some(error),
        duration_ms: Some(0),
        started_at: Some(now),
        completed_at: Some(now),
        round,
        sensitive: tool.label.sensitive,
        artifacts: Vec::new(),
        output_limit: None,
        draft: None,
        structured_content: None,
    }
}

pub(crate) fn pending_tool_record(
    tool: &ToolDefinition,
    call: &PendingToolCall,
    round: u32,
) -> AgentToolCall {
    AgentToolCall {
        id: call.id.clone(),
        name: tool.label.name.clone(),
        source: tool.label.source.clone(),
        server_id: tool.label.server_id.clone(),
        arguments: call.arguments_raw.clone(),
        status: AgentToolStatus::Pending,
        output: None,
        error: None,
        duration_ms: None,
        started_at: Some(chrono::Local::now().timestamp()),
        completed_at: None,
        round,
        sensitive: tool.label.sensitive,
        artifacts: Vec::new(),
        output_limit: None,
        draft: None,
        structured_content: None,
    }
}

pub async fn execute_tool_call(
    host: &super::context::AgentContext<'_>,
    executor: &dyn ToolExecutor,
    ctx: &ToolExecutionContext<'_>,
    tool: &ToolDefinition,
    call: PendingToolCall,
) -> (AgentToolCall, String, Vec<Value>) {
    let record = pending_tool_record(tool, &call, ctx.round);
    host.tool_updated(&record);
    let progress = ToolProgress {
        control: host.control.clone(),
        events: &host.events,
    };
    let prepared = executor.prepare(&progress, ctx, tool, call, record).await;
    let mut prepared = match prepared {
        ToolPreparation::Complete(record, content, follow_ups) => {
            return (record, content, follow_ups)
        }
        ToolPreparation::Ready(prepared) => prepared,
    };
    let mut record = prepared.record;
    record.status = AgentToolStatus::Running;
    host.tool_updated(&record);
    let started = Instant::now();
    let call_fut = executor.call(ctx, tool, prepared.arguments.clone());
    let result = if prepared.drain_on_cancel {
        Ok(call_fut.await)
    } else {
        tokio::select! {
            result = async {
                if let Some(timeout_ms) = prepared.timeout_ms {
                    timeout(Duration::from_millis(timeout_ms), call_fut).await
                } else { Ok(call_fut.await) }
            } => result,
            _ = host.control.wait_inactive() => {
                record.status = AgentToolStatus::Cancelled;
                record.duration_ms = Some(started.elapsed().as_millis() as u64);
                record.completed_at = Some(chrono::Local::now().timestamp());
                record.error = Some("Tool call cancelled".into());
                host.tool_updated(&record);
                return (record, "Tool call cancelled".into(), Vec::new());
            }
        }
    };
    if !prepared.drain_on_cancel && !host.control.active() {
        record.status = AgentToolStatus::Cancelled;
        record.duration_ms = Some(started.elapsed().as_millis() as u64);
        record.completed_at = Some(chrono::Local::now().timestamp());
        record.error = Some("Tool call cancelled".into());
        host.tool_updated(&record);
        return (record, "Tool call cancelled".into(), Vec::new());
    }
    record.duration_ms = Some(started.elapsed().as_millis() as u64);
    record.completed_at = Some(chrono::Local::now().timestamp());
    let mut follow_ups = Vec::new();
    let mut content = match result {
        Ok(Ok(output)) if !output.is_error => {
            record.status = AgentToolStatus::Success;
            record.artifacts = output.host_data;
            record.structured_content = output.structured_content;
            follow_ups =
                crate::model::openai_messages_from_model_messages(&output.follow_up_messages);
            record.output = Some(output.content.clone());
            record.output_limit = prepared.output_limit;
            limit_tool_text_for_model(&output.content, prepared.output_limit)
        }
        Ok(Ok(output)) => {
            record.status = AgentToolStatus::Error;
            record.structured_content = output.structured_content;
            record.error = Some(truncate_chars(&output.content, 1000));
            limit_tool_text_for_model(&output.content, prepared.output_limit)
        }
        Ok(Err(error)) => {
            record.status = AgentToolStatus::Error;
            record.error = Some(error.clone());
            limit_tool_text_for_model(&error, prepared.output_limit)
        }
        Err(_) => {
            record.status = AgentToolStatus::Error;
            record.error = Some(prepared.timeout_message.clone());
            prepared.timeout_message
        }
    };
    let feedback = executor
        .after(
            &progress,
            ctx,
            tool,
            prepared.arguments,
            &content,
            std::mem::take(&mut prepared.feedback),
        )
        .await;
    content.push_str(&feedback);
    host.tool_updated(&record);
    (record, content, follow_ups)
}

pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

pub fn limit_tool_text_for_model(value: &str, max_chars: Option<usize>) -> String {
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
