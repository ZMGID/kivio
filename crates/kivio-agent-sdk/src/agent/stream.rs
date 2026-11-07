use super::content::{AgentToolCall, AgentToolStatus, ContentBlock, ContentKind, ContentPhase};
use std::sync::{Arc, Mutex};

#[cfg(test)]
use serde_json::Value;

use crate::model::{
    GenerateOutput, ModelError, PendingToolCall, StreamPart, StreamSink, WebCitation,
};

use crate::tools::ToolDefinition;

use super::content::{ProviderSearch, ToolArgumentState};
use super::execute::match_tool_call;
use super::host::SpeculativeTaskScope;
use super::stop::{
    empty_assistant_response_error, pending_tool_calls_from_dsml, sanitize_assistant_text_response,
};
use super::types::AgentStreamPolicy;

#[derive(Default)]
struct ChatStreamAccumulator {
    content: String,
    reasoning: String,
}

struct ChatStreamSnapshot {
    content: String,
    reasoning: String,
}

#[derive(Clone)]
pub struct ToolCallDraftTracker {
    inner: Arc<Mutex<ToolCallDraftState>>,
}

struct ToolCallDraftState {
    tools: Vec<ToolDefinition>,
    round: u32,
    step_number: Option<u8>,
    next_order: u32,
    drafts: Vec<ToolCallDraft>,
    speculative: Option<Box<dyn SpeculativeTaskScope>>,
}

struct ToolCallDraft {
    model_name: String,
    arguments_raw: String,
    record: AgentToolCall,
    segment: ContentBlock,
    last_emitted_argument_chars: usize,
    done: bool,
    speculative: bool,
    argument_limit: Option<usize>,
    argument_limit_error: Option<String>,
}

impl ToolCallDraftTracker {
    pub fn new(
        tools: Vec<ToolDefinition>,
        round: u32,
        step_number: Option<u8>,
        first_order: u32,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ToolCallDraftState {
                tools,
                round,
                step_number,
                next_order: first_order,
                drafts: Vec::new(),
                speculative: None,
            })),
        }
    }

    pub(crate) fn enable_speculation(
        &self,
        executor: &dyn super::execute::ToolExecutor,
        context: &super::execute::ToolExecutionContext<'_>,
        effective_tools: &[crate::model::ModelTool],
    ) {
        let tools: Vec<_> = self
            .inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .tools
            .iter()
            .filter(|tool| {
                tool.speculative
                    && effective_tools
                        .iter()
                        .any(|effective| effective.name == tool.model.name)
            })
            .cloned()
            .collect();
        let scope = (!tools.is_empty())
            .then(|| executor.start_speculation(context, &tools))
            .flatten();
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .speculative = scope;
    }

    pub(crate) fn disable_speculation(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .speculative
            .take();
    }

    pub fn has_started(&self) -> bool {
        !self
            .inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .drafts
            .is_empty()
    }

    pub fn segments(&self) -> Vec<ContentBlock> {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .drafts
            .iter()
            .map(|draft| draft.segment.clone())
            .collect()
    }

    pub fn has_unfinished_drafts(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .drafts
            .iter()
            .any(|draft| !draft.done)
    }

    pub(crate) fn inflight(&self) -> Vec<(String, String)> {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .drafts
            .iter()
            .filter(|draft| !draft.done)
            .map(|draft| (draft.model_name.clone(), draft.arguments_raw.clone()))
            .collect()
    }

    pub(crate) fn reconcile_speculation(&self, calls: &[PendingToolCall], truncated: bool) {
        let state = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let Some(scope) = &state.speculative else {
            return;
        };
        let final_calls = if truncated {
            Vec::new()
        } else {
            calls
                .iter()
                .filter_map(|call| {
                    let draft = state
                        .drafts
                        .iter()
                        .find(|draft| draft.record.id == call.id)?;
                    (draft.speculative
                        && draft.model_name == call.function_name
                        && call.arguments_parse_error.is_none())
                    .then_some((call.id.as_str(), &call.arguments))
                })
                .collect()
        };
        scope.reconcile(&final_calls);
    }

    pub fn mark_error(&self, error: &str) -> Vec<AgentToolCall> {
        let now = chrono::Local::now().timestamp();
        let mut guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(scope) = &guard.speculative {
            scope.discard();
        }
        for draft in &mut guard.drafts {
            draft.record.status = AgentToolStatus::Error;
            draft.record.completed_at = Some(now);
            draft.record.duration_ms = draft
                .record
                .started_at
                .map(|started| (now.saturating_sub(started) as u64).saturating_mul(1000))
                .or(Some(0));
            draft.record.error = Some(error.to_string());
            draft.record.arguments = draft.arguments_raw.clone();
            draft.record.output = None;
        }
        guard
            .drafts
            .iter()
            .map(|draft| draft.record.clone())
            .collect()
    }
}

/// 模型**原生内置联网搜索**的实时卡追踪器（任务 07-23）。镜像 `ToolCallDraftTracker`
/// 的「sink 持 tracker 边流边累加 → 流后 caller 读回落盘」模式：`Arc<Mutex>` 让 sink 与
/// 调用方（planning/synthesis）共享同一状态。稳定 id + reasoning/正文之间预留的 order 槽
/// 使卡渲染在答案文本**之前**；去重累加 queries/citations；`started` 保证段只发一次。
#[derive(Clone)]
pub struct WebSearchCardTracker {
    inner: Arc<Mutex<WebSearchCardState>>,
}

struct WebSearchCardState {
    block: ContentBlock,
    started: bool,
}
impl WebSearchCardTracker {
    pub(crate) fn new(
        order: u32,
        round: Option<u32>,
        phase: ContentPhase,
        provider_label: String,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WebSearchCardState {
                block: ContentBlock {
                    id: order,
                    kind: ContentKind::Search(ProviderSearch {
                        id: format!("websearch_{}", uuid::Uuid::new_v4()),
                        provider: provider_label,
                        queries: Vec::new(),
                        citations: Vec::new(),
                        complete: false,
                    }),
                    phase,
                    step_number: None,
                    round,
                    text: None,
                    tool_call_id: None,
                },
                started: false,
            })),
        }
    }
    pub(crate) fn note(
        &self,
        queries: &[String],
        citations: &[WebCitation],
    ) -> (ContentBlock, bool) {
        let mut guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let announce = !guard.started;
        guard.started = true;
        let ContentKind::Search(search) = &mut guard.block.kind else {
            unreachable!()
        };
        for query in queries {
            let query = query.trim();
            if !query.is_empty() && !search.queries.iter().any(|existing| existing == query) {
                search.queries.push(query.to_string());
            }
        }
        for citation in citations {
            let url = citation.url.trim();
            if url.is_empty() {
                continue;
            }
            if let Some(existing) = search
                .citations
                .iter_mut()
                .find(|existing| existing.url == url)
            {
                if existing.snippet.is_none() {
                    existing.snippet = citation.snippet.clone();
                }
                if existing.published_date.is_none() {
                    existing.published_date = citation.published_date.clone();
                }
            } else {
                search.citations.push(citation.clone());
            }
        }
        (guard.block.clone(), announce)
    }
    pub(crate) fn finalize_success(&self) -> Option<ContentBlock> {
        let mut guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        if !guard.started {
            return None;
        }
        let ContentKind::Search(search) = &mut guard.block.kind else {
            unreachable!()
        };
        search.complete = true;
        Some(guard.block.clone())
    }
    pub(crate) fn take_card(&self) -> Option<ContentBlock> {
        let guard = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        guard.started.then(|| guard.block.clone())
    }
}

fn chat_stream_snapshot(accumulator: &Arc<Mutex<ChatStreamAccumulator>>) -> ChatStreamSnapshot {
    let guard = accumulator.lock().unwrap_or_else(|err| err.into_inner());
    ChatStreamSnapshot {
        content: guard.content.clone(),
        reasoning: guard.reasoning.clone(),
    }
}

pub struct AgentStreamSink<'a> {
    host: &'a super::context::AgentContext<'a>,
    accumulator: Arc<Mutex<ChatStreamAccumulator>>,
    buffer_tool_planning_text: bool,
    text_segment: Option<ContentBlock>,
    reasoning_segment: Option<ContentBlock>,
    tool_draft_tracker: Option<ToolCallDraftTracker>,
    web_search_tracker: Option<WebSearchCardTracker>,
    text_buffer: String,
    text_suppressed: bool,
    emitted_text_chars: u32,
    request_started: bool,
    speculation: Option<(
        &'a dyn super::execute::ToolExecutor,
        &'a super::execute::ToolExecutionContext<'a>,
    )>,
}

impl<'a> AgentStreamSink<'a> {
    pub fn new(
        host: &'a super::context::AgentContext<'a>,
        buffer_tool_planning_text: bool,
        text_segment: Option<ContentBlock>,
        reasoning_segment: Option<ContentBlock>,
        tool_draft_tracker: Option<ToolCallDraftTracker>,
        web_search_tracker: Option<WebSearchCardTracker>,
    ) -> Self {
        Self {
            host,
            accumulator: Arc::new(Mutex::new(ChatStreamAccumulator::default())),
            buffer_tool_planning_text,
            text_segment,
            reasoning_segment,
            tool_draft_tracker,
            web_search_tracker,
            text_buffer: String::new(),
            text_suppressed: false,
            emitted_text_chars: 0,
            request_started: false,
            speculation: None,
        }
    }

    pub fn snapshot(&self) -> (String, String) {
        let snapshot = chat_stream_snapshot(&self.accumulator);
        (snapshot.content, snapshot.reasoning)
    }

    pub(crate) fn with_speculation(
        mut self,
        speculation: Option<(
            &'a dyn super::execute::ToolExecutor,
            &'a super::execute::ToolExecutionContext<'a>,
        )>,
    ) -> Self {
        self.speculation = speculation;
        self
    }

    pub(crate) fn request_was_started(&self) -> bool {
        self.request_started
    }

    fn emit_text_delta(&mut self, delta: &str) {
        self.emitted_text_chars += delta.chars().count() as u32;
        self.host.content(delta, None, self.text_segment.as_ref());
    }

    pub fn discard_attempt(&mut self) {
        let mut segment_ids: Vec<u32> = self
            .text_segment
            .iter()
            .chain(self.reasoning_segment.iter())
            .map(|s| s.id.clone())
            .collect();
        let mut tool_ids = Vec::new();
        if let Some(tracker) = &self.tool_draft_tracker {
            let mut state = tracker.inner.lock().unwrap_or_else(|err| err.into_inner());
            if let Some(scope) = &state.speculative {
                scope.discard();
            }
            if let Some(order) = state.drafts.iter().map(|d| d.segment.id).min() {
                state.next_order = order;
            }
            for draft in state.drafts.drain(..) {
                segment_ids.push(draft.segment.id);
                tool_ids.push(draft.record.id);
            }
        }
        if let Some(tracker) = &self.web_search_tracker {
            let mut state = tracker.inner.lock().unwrap_or_else(|err| err.into_inner());
            if state.started {
                segment_ids.push(state.block.id);
                if let ContentKind::Search(search) = &mut state.block.kind {
                    tool_ids.push(search.id.clone());
                    search.queries.clear();
                    search.citations.clear();
                    search.complete = false;
                }
            }
            state.started = false;
        }
        let (_, reasoning) = self.snapshot();
        self.host.discard_attempt(
            self.emitted_text_chars,
            reasoning.chars().count() as u32,
            segment_ids,
            tool_ids,
        );
    }

    fn handle_text_delta(&mut self, delta: String) {
        self.accumulator
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .content
            .push_str(&delta);

        if self.text_suppressed {
            return;
        }
        if !self.buffer_tool_planning_text {
            self.emit_text_delta(&delta);
            return;
        }

        self.text_buffer.push_str(&delta);
        if crate::dsml_tools::contains_dsml_tool_markup(&self.text_buffer) {
            self.text_buffer.clear();
            self.text_suppressed = true;
            return;
        }
        if should_flush_tool_planning_text_buffer(&self.text_buffer) {
            self.flush_pending_text();
        }
    }

    pub fn flush_pending_text(&mut self) {
        if self.text_suppressed || self.text_buffer.is_empty() {
            return;
        }
        let delta = std::mem::take(&mut self.text_buffer);
        self.emit_text_delta(&delta);
    }

    fn emit_tool_call_start(&mut self, id: String, name: String) {
        let Some(tracker) = self.tool_draft_tracker.as_ref() else {
            return;
        };
        let mut guard = tracker.inner.lock().unwrap_or_else(|err| err.into_inner());
        if guard.drafts.iter().any(|draft| draft.record.id == id) {
            return;
        }
        if match_tool_call(&guard.tools, &name).is_none()
            && self
                .speculation
                .and_then(|(executor, _)| executor.unknown_feedback(&name))
                .is_some()
        {
            return;
        }
        let (record_name, source, server_id, sensitive) =
            if let Some(tool) = match_tool_call(&guard.tools, &name) {
                (
                    tool.label.name.clone(),
                    tool.label.source.clone(),
                    tool.label.server_id.clone(),
                    tool.label.sensitive,
                )
            } else {
                (name.clone(), "unknown".to_string(), None, false)
            };
        let definition = match_tool_call(&guard.tools, &name);
        let speculative = definition.is_some_and(|tool| tool.speculative);
        let argument_limit = definition.and_then(|tool| tool.argument_limit);
        let argument_limit_error = definition.and_then(|tool| tool.argument_limit_error.clone());
        let order = guard.next_order;
        guard.next_order = guard.next_order.saturating_add(1);
        let now = chrono::Local::now().timestamp();
        let segment = ContentBlock {
            id: order,
            kind: ContentKind::Tool,
            phase: ContentPhase::ToolLoop,
            step_number: guard.step_number,
            round: Some(guard.round),
            text: None,
            tool_call_id: Some(id.clone()),
        };
        let record = AgentToolCall {
            id: id.clone(),
            name: record_name,
            source,
            server_id,
            arguments: String::new(),
            status: AgentToolStatus::Pending,
            output: None,
            error: None,
            duration_ms: None,
            started_at: Some(now),
            completed_at: None,
            round: guard.round,
            sensitive,
            artifacts: Vec::new(),
            output_limit: None,
            draft: Some(ToolArgumentState {
                model_name: name.clone(),
                complete: false,
            }),
            structured_content: None,
        };
        guard.drafts.push(ToolCallDraft {
            model_name: name,
            arguments_raw: String::new(),
            record: record.clone(),
            segment: segment.clone(),
            last_emitted_argument_chars: 0,
            done: false,
            speculative,
            argument_limit,
            argument_limit_error,
        });
        drop(guard);
        self.host.content("", None, Some(&segment));
        self.host.tool_updated(&record);
    }

    fn emit_tool_call_delta(&mut self, id: String, delta: String) -> Result<(), ModelError> {
        let Some(tracker) = self.tool_draft_tracker.as_ref() else {
            return Ok(());
        };
        let record_to_emit = {
            let mut guard = tracker.inner.lock().unwrap_or_else(|err| err.into_inner());
            let ToolCallDraftState {
                drafts,
                speculative,
                ..
            } = &mut *guard;
            let Some(draft) = drafts.iter_mut().find(|draft| draft.record.id == id) else {
                return Ok(());
            };
            if let Some(limit) = draft.argument_limit {
                let next = draft
                    .arguments_raw
                    .chars()
                    .count()
                    .saturating_add(delta.chars().count());
                if next > limit {
                    return Err(ModelError::new(
                        draft.argument_limit_error.clone().unwrap_or_else(|| {
                            format!("{} arguments are too large", draft.model_name)
                        }),
                    ));
                }
            }
            draft.arguments_raw.push_str(&delta);
            if draft.speculative {
                if let Some(scope) = speculative {
                    scope.update(&id, &draft.arguments_raw);
                }
            }
            if draft.argument_limit.is_some() {
                return Ok(());
            }
            let chars = draft.arguments_raw.chars().count();
            if chars == 0 || chars.saturating_sub(draft.last_emitted_argument_chars) < 2048 {
                return Ok(());
            }
            draft.last_emitted_argument_chars = chars;
            draft.record.arguments = draft.arguments_raw.clone();
            Some(draft.record.clone())
        };
        if let Some(record) = record_to_emit {
            self.host.tool_updated(&record);
        }
        Ok(())
    }

    fn emit_tool_call_done(&mut self, call: &PendingToolCall) -> Result<(), ModelError> {
        let Some(tracker) = self.tool_draft_tracker.as_ref() else {
            return Ok(());
        };
        let record_to_emit = {
            let mut guard = tracker.inner.lock().unwrap_or_else(|err| err.into_inner());
            let ToolCallDraftState {
                drafts,
                speculative,
                ..
            } = &mut *guard;
            let Some(draft) = drafts.iter_mut().find(|draft| draft.record.id == call.id) else {
                return Ok(());
            };
            if draft
                .argument_limit
                .is_some_and(|limit| call.arguments_raw.chars().count() > limit)
            {
                return Err(ModelError::new(
                    draft
                        .argument_limit_error
                        .clone()
                        .unwrap_or_else(|| format!("{} arguments are too large", draft.model_name)),
                ));
            }
            if draft.speculative {
                if let Some(scope) = speculative {
                    let args = (draft.model_name == call.function_name
                        && call.arguments_parse_error.is_none())
                    .then_some(&call.arguments);
                    scope.reconcile_call(&call.id, args);
                }
            }
            draft.done = true;
            draft.arguments_raw = call.arguments_raw.clone();
            let chars = draft.arguments_raw.chars().count();
            draft.last_emitted_argument_chars = chars;
            draft.record.arguments = call.arguments_raw.clone();
            draft.record.draft.as_mut().expect("draft state").complete = true;
            Some(draft.record.clone())
        };
        if let Some(record) = record_to_emit {
            self.host.tool_updated(&record);
        }
        Ok(())
    }

    /// 消费一帧内置搜索增量（任务 07-23）：无 tracker（非内置模式）直接忽略——保证
    /// `web_search_tracker=None` 时行为与现状逐字节一致。首帧 emit 段（把卡插入答案之前的
    /// 预留槽），每帧 emit Running 记录（同 id 幂等 → 前端原地更新）。
    fn handle_web_search(&mut self, queries: Vec<String>, citations: Vec<WebCitation>) {
        if let Some(tracker) = &self.web_search_tracker {
            let (block, announce) = tracker.note(&queries, &citations);
            self.host.search_updated(&block, announce);
        }
    }
    pub fn finish_web_search_card(&self) -> Option<ContentBlock> {
        self.web_search_tracker
            .as_ref()
            .and_then(WebSearchCardTracker::finalize_success)
    }
}

impl super::provider_runtime::ModelOutputSink for AgentStreamSink<'_> {
    fn request_started(&mut self, call: &super::provider_runtime::ModelCall) {
        if !self.host.control.active() {
            return;
        }
        self.request_started = true;
        self.host
            .context_prepared(&call.provider_id, &call.model, &call.history, &call.tools);
        if let Some(tracker) = &self.tool_draft_tracker {
            if call.options.tool_choice == crate::model::ToolChoice::None || call.tools.is_empty() {
                tracker.disable_speculation();
            } else if let Some((executor, context)) = self.speculation {
                tracker.enable_speculation(executor, context, &call.tools);
            }
        }
    }

    fn retry(&mut self, status: super::events::RetryStatus) {
        self.host.retry(status);
    }

    fn reset_attempt(&mut self) {
        self.discard_attempt();
        *self
            .accumulator
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = ChatStreamAccumulator::default();
        self.text_buffer.clear();
        self.text_suppressed = false;
        self.emitted_text_chars = 0;
    }

    fn draft_snapshot(&self) -> Vec<(String, String)> {
        self.tool_draft_tracker
            .as_ref()
            .map(ToolCallDraftTracker::inflight)
            .unwrap_or_default()
    }
}
impl StreamSink for AgentStreamSink<'_> {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        if !self.host.control.active() {
            return Ok(());
        }
        match part {
            StreamPart::ContextUsage {
                input_tokens,
                output_tokens,
            } => {
                self.host.context_usage(
                    input_tokens.saturating_add(output_tokens),
                    Some("provider_context_reported"),
                    None,
                    None,
                );
            }
            StreamPart::TextDelta { delta } => {
                self.handle_text_delta(delta);
            }
            StreamPart::ReasoningDelta { delta } => {
                self.accumulator
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .reasoning
                    .push_str(&delta);
                self.host
                    .content("", Some(&delta), self.reasoning_segment.as_ref());
            }
            StreamPart::ToolCallStart { id, name } => self.emit_tool_call_start(id, name),
            StreamPart::ToolCallDelta { id, delta } => self.emit_tool_call_delta(id, delta)?,
            StreamPart::ToolCallDone { call } => self.emit_tool_call_done(&call)?,
            StreamPart::WebSearch { queries, citations } => {
                self.handle_web_search(queries, citations)
            }
            // 出图数据帧：Step 3 才落地为 artifact，此处先占位不处理。
            StreamPart::ImageData { .. } => {}
            StreamPart::Finish { .. } => {}
        }
        Ok(())
    }
}

pub fn should_flush_tool_planning_text_buffer(buffer: &str) -> bool {
    let trimmed = buffer.trim_start();
    if trimmed.starts_with('<') && trimmed.len() < 64 {
        return false;
    }
    buffer.chars().count() >= 12 || buffer.contains('\n')
}

pub(crate) fn normalize_stream_output(
    mut output: GenerateOutput,
    snapshot_content: String,
    snapshot_reasoning: String,
    policy: AgentStreamPolicy,
) -> GenerateOutput {
    if matches!(policy, AgentStreamPolicy::PlanningNoDoneUntilNoTools) {
        if let Some(message) = &output.canonical_message {
            output.text = message.text_content();
        }
    }
    if output.text.trim().is_empty() {
        output.text = snapshot_content;
    }
    if output.reasoning.is_none() && !snapshot_reasoning.trim().is_empty() {
        output.reasoning = Some(snapshot_reasoning);
    }
    let dsml_calls = if matches!(policy, AgentStreamPolicy::PlanningNoDoneUntilNoTools)
        && output.tool_calls.is_empty()
    {
        pending_tool_calls_from_dsml(&output.text)
    } else {
        Vec::new()
    };
    let cleaned = sanitize_assistant_text_response(output.text.trim());
    if !dsml_calls.is_empty() {
        let mut replay = output.replay_message();
        replay.content.retain(|part| {
            !matches!(
                part,
                crate::model::MessagePart::Text { .. } | crate::model::MessagePart::ToolCall { .. }
            ) && !matches!(part, crate::model::MessagePart::AnthropicNativeBlock { block, .. }
            if block["type"] == "text")
        });
        if !cleaned.is_empty() {
            replay.content.push(crate::model::MessagePart::Text {
                text: cleaned.clone(),
            });
        }
        replay
            .content
            .extend(dsml_calls.iter().map(PendingToolCall::replay_part));
        output.canonical_message = Some(replay);
        output.tool_calls = dsml_calls;
    }
    output.text = cleaned;
    output
}

pub fn validate_stream_output(
    label: &str,
    policy: AgentStreamPolicy,
    output: &GenerateOutput,
) -> Result<(), String> {
    let tool_calls_from_stream = !output.tool_calls.is_empty();
    // hosted 出图（Responses `image_generation_call` / Gemini inlineData）时正文本就是
    // 空串——图即答案。此时空正文不是失败，否则整轮报「空助手响应」而图被丢掉。
    if output.text.trim().is_empty() && !output.images.is_empty() {
        return Ok(());
    }
    if output.text.trim().is_empty() {
        match policy {
            AgentStreamPolicy::SynthesisAlwaysDone => {
                return Err(empty_assistant_response_error(label));
            }
            AgentStreamPolicy::SynthesisDeferEmpty => return Ok(()),
            AgentStreamPolicy::PlanningNoDoneUntilNoTools if !tool_calls_from_stream => {
                return Err(empty_assistant_response_error(label));
            }
            AgentStreamPolicy::PlanningNoDoneUntilNoTools => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::model::{FinishReason, GeneratedImageData};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::agent::content::InputKind;
    use crate::agent::context::AgentContext;
    use crate::agent::control::{AgentRunControl, AgentRunHandle, InputAdmission, StopAdmission};
    use crate::agent::events::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentRunIdentity, EventDelivery,
    };
    use crate::agent::execute::ToolExecutionContext;
    use crate::agent::execute::{
        ToolExecutor, ToolExecutorFuture, ToolFuture, ToolPreparation, ToolProgress,
    };
    use crate::agent::host::{AgentHost, AgentHostFuture, SpeculativeTaskScope};
    use crate::agent::provider_runtime::{ModelCall, ModelDelivery, ModelOutputSink};
    use crate::agent::steering::SteeringMessage;
    use crate::tools::ToolDefinition;
    const ARGUMENT_LIMIT: usize = 64;
    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition::new(crate::model::ModelTool {
            name: name.into(),
            description: String::new(),
            input_schema: serde_json::json!({"type":"object"}),
        })
    }
    fn limited_tool() -> ToolDefinition {
        let mut tool = tool("present_artifacts");
        tool.argument_limit = Some(ARGUMENT_LIMIT);
        tool.argument_limit_error = Some("artifact_ids arguments are too large".into());
        tool
    }

    #[derive(Default)]
    struct TestHost {
        stopped: AtomicBool,
        scope_drops: Option<Arc<AtomicUsize>>,
        records: Mutex<Vec<AgentToolCall>>,
        segments: Mutex<Vec<ContentBlock>>,
        searches: Mutex<Vec<ContentBlock>>,
    }

    struct TestScope(Arc<AtomicUsize>);
    impl SpeculativeTaskScope for TestScope {
        fn update(&self, _call: &str, _raw: &str) {}
        fn reconcile_call(&self, _call: &str, _args: Option<&Value>) {}
        fn reconcile(&self, _final_calls: &[(&str, &Value)]) {}
        fn discard(&self) {}
    }
    impl Drop for TestScope {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl AgentHost for TestHost {}

    impl ToolExecutor for TestHost {
        fn before_tool_call<'a>(
            &'a self,
            _: &'a ToolProgress<'a>,
            _: &'a ToolExecutionContext<'a>,
            _: &'a ToolDefinition,
            _: PendingToolCall,
            _: AgentToolCall,
        ) -> ToolFuture<'a, Result<ToolPreparation, String>> {
            unreachable!("stream tests do not execute tools")
        }
        fn call<'a>(
            &'a self,
            _progress: &'a ToolProgress<'a>,
            _: &'a ToolExecutionContext<'a>,
            _: &'a ToolDefinition,
            _: Value,
        ) -> ToolExecutorFuture<'a> {
            unreachable!("stream tests do not execute tools")
        }
        fn start_speculation(
            &self,
            _: &ToolExecutionContext<'_>,
            tools: &[ToolDefinition],
        ) -> Option<Box<dyn SpeculativeTaskScope>> {
            if !tools.iter().any(|tool| tool.speculative) {
                return None;
            }
            self.scope_drops
                .as_ref()
                .map(|drops| Box::new(TestScope(drops.clone())) as Box<dyn SpeculativeTaskScope>)
        }
    }
    #[test]
    fn tool_planning_text_buffer_delays_possible_dsml_prefix() {
        assert!(!should_flush_tool_planning_text_buffer("<|DSML|"));
        assert!(!should_flush_tool_planning_text_buffer("   <invoke"));
        assert!(should_flush_tool_planning_text_buffer(
            "普通回答已经足够长，可以开始流式显示了"
        ));
        assert!(should_flush_tool_planning_text_buffer("first line\n"));
    }

    #[test]
    fn tool_call_stream_parts_emit_draft_records_and_segments() {
        let host = TestHost::default();
        let tracker = ToolCallDraftTracker::new(vec![tool("write")], 2, Some(3), 1002);
        let context = test_context(&host);
        let mut sink =
            AgentStreamSink::new(&context, true, None, None, Some(tracker.clone()), None);

        sink.emit(StreamPart::ToolCallStart {
            id: "call_write".to_string(),
            name: "write".to_string(),
        })
        .expect("start should emit");
        sink.emit(StreamPart::ToolCallDelta {
            id: "call_write".to_string(),
            delta: "{\"path\":\"demo.html\",\"content\":\"".to_string(),
        })
        .expect("delta should emit");
        assert_eq!(
            tracker.inflight(),
            vec![(
                "write".to_string(),
                "{\"path\":\"demo.html\",\"content\":\"".to_string()
            )]
        );
        let call = PendingToolCall {
            id: "call_write".to_string(),
            function_name: "write".to_string(),
            arguments: serde_json::json!({
                "path": "demo.html",
                "content": "<html></html>"
            }),
            arguments_raw: "{\"path\":\"demo.html\",\"content\":\"<html></html>\"}".to_string(),
            arguments_parse_error: None,
            signature: None,
        };
        sink.emit(StreamPart::ToolCallDone { call })
            .expect("done should emit");
        assert!(tracker.inflight().is_empty());

        let records = host.records.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id, "call_write");
        assert_eq!(records[0].name, "write");
        assert!(matches!(records[0].status, AgentToolStatus::Pending));
        assert!(!records[0].draft.as_ref().unwrap().complete);
        assert!(records[1].draft.as_ref().unwrap().complete);
        assert!(records[1].arguments.contains("demo.html"));

        let segments = host.segments.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, ContentKind::Tool);
        assert_eq!(segments[0].phase, ContentPhase::ToolLoop);
        assert_eq!(segments[0].tool_call_id.as_deref(), Some("call_write"));
        assert_eq!(segments[0].id, 1002);
        assert!(tracker.has_started());
        assert!(!tracker.has_unfinished_drafts());
    }

    #[test]
    fn tool_call_draft_error_preserves_backend_record_after_stream_failure() {
        let host = TestHost::default();
        let tracker = ToolCallDraftTracker::new(vec![tool("write")], 1, Some(1), 1000);
        let context = test_context(&host);
        let mut sink =
            AgentStreamSink::new(&context, true, None, None, Some(tracker.clone()), None);

        sink.emit(StreamPart::ToolCallStart {
            id: "call_write".to_string(),
            name: "write".to_string(),
        })
        .expect("start should emit");
        sink.emit(StreamPart::ToolCallDelta {
            id: "call_write".to_string(),
            delta: "{\"path\":\"large.html\",\"content\":\"".to_string(),
        })
        .expect("delta should emit");

        let failed = tracker.mark_error("Chat tools planning read body failed");

        assert_eq!(failed.len(), 1);
        let record = &failed[0];
        assert_eq!(record.id, "call_write");
        assert_eq!(record.name, "write");
        assert!(matches!(record.status, AgentToolStatus::Error));
        assert_eq!(
            record.error.as_deref(),
            Some("Chat tools planning read body failed")
        );
        assert_eq!(record.round, 1);
        assert_eq!(record.draft.as_ref().unwrap().model_name, "write");
        assert!(!record.draft.as_ref().unwrap().complete);
        assert_eq!(record.arguments, "{\"path\":\"large.html\",\"content\":\"");
        assert!(record.completed_at.is_some());

        let segments = host.segments.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].tool_call_id.as_deref(), Some("call_write"));
        assert!(tracker.has_unfinished_drafts());
    }

    #[test]
    fn present_artifacts_argument_stream_aborts_when_too_large() {
        let host = TestHost::default();
        let tracker = ToolCallDraftTracker::new(vec![limited_tool()], 1, Some(1), 1000);
        let context = test_context(&host);
        let mut sink =
            AgentStreamSink::new(&context, true, None, None, Some(tracker.clone()), None);

        sink.emit(StreamPart::ToolCallStart {
            id: "call_present".to_string(),
            name: "present_artifacts".to_string(),
        })
        .expect("start should emit");
        let err = sink
            .emit(StreamPart::ToolCallDelta {
                id: "call_present".to_string(),
                delta: "x".repeat(ARGUMENT_LIMIT + 1),
            })
            .expect_err("oversized present_artifacts args must abort the stream");
        assert!(err.to_string().contains("too large"), "{err}");
        assert!(err.to_string().contains("artifact_ids"), "{err}");

        let records = host.records.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, AgentToolStatus::Pending);
        assert!(records[0].arguments.is_empty());
        assert!(!records[0].draft.as_ref().unwrap().complete);
    }

    #[test]
    fn present_artifacts_argument_stream_accepts_id_list() {
        let host = TestHost::default();
        let tracker = ToolCallDraftTracker::new(vec![limited_tool()], 1, Some(1), 1000);
        let context = test_context(&host);
        let mut sink = AgentStreamSink::new(&context, true, None, None, Some(tracker), None);
        let args = r#"{"artifact_ids":["art_a","art_b"]}"#;

        sink.emit(StreamPart::ToolCallStart {
            id: "call_present".to_string(),
            name: "present_artifacts".to_string(),
        })
        .expect("start should emit");
        sink.emit(StreamPart::ToolCallDelta {
            id: "call_present".to_string(),
            delta: args.to_string(),
        })
        .expect("small present_artifacts args should stream");
        sink.emit(StreamPart::ToolCallDone {
            call: PendingToolCall {
                id: "call_present".to_string(),
                function_name: "present_artifacts".to_string(),
                arguments: serde_json::json!({ "artifact_ids": ["art_a", "art_b"] }),
                arguments_raw: args.to_string(),
                arguments_parse_error: None,
                signature: None,
            },
        })
        .expect("done should emit");

        let records = host.records.lock().unwrap_or_else(|err| err.into_inner());
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].arguments, args);
    }

    #[test]
    fn synthesis_defer_empty_allows_agent_fallback_without_done() {
        let output = GenerateOutput::text(String::new(), None);

        assert!(validate_stream_output(
            "Chat stream",
            AgentStreamPolicy::SynthesisDeferEmpty,
            &output
        )
        .is_ok());
        assert_eq!(
            validate_stream_output(
                "Chat stream",
                AgentStreamPolicy::SynthesisAlwaysDone,
                &output
            )
            .expect_err("strict synthesis should still reject empty output"),
            "Chat stream returned an empty assistant response"
        );
    }

    #[test]
    fn image_only_output_validates_despite_empty_text() {
        let mut output = GenerateOutput::text(String::new(), None);
        output.images = vec![GeneratedImageData {
            mime_type: "image/png".to_string(),
            data: "aGVsbG8=".to_string(),
        }];

        validate_stream_output(
            "Chat tools planning",
            AgentStreamPolicy::PlanningNoDoneUntilNoTools,
            &output,
        )
        .expect("image-only planning output must validate");

        output.images.clear();
        assert!(validate_stream_output(
            "Chat tools planning",
            AgentStreamPolicy::PlanningNoDoneUntilNoTools,
            &output
        )
        .is_err());
    }

    #[test]
    fn synthesis_defer_empty_emits_done_for_non_empty_output() {
        let output = GenerateOutput::text("final".to_string(), None);

        validate_stream_output(
            "Chat stream",
            AgentStreamPolicy::SynthesisDeferEmpty,
            &output,
        )
        .expect("non-empty synthesis should validate");
    }

    #[test]
    fn web_search_card_streams_running_before_text_and_merges_by_id() {
        let host = TestHost::default();

        let tracker = WebSearchCardTracker::new(
            1001,
            Some(2),
            ContentPhase::ToolLoop,
            "Test Provider".to_string(),
        );
        let text_segment = ContentBlock {
            id: 1002,
            kind: ContentKind::Text,
            phase: ContentPhase::ToolLoop,
            step_number: Some(1),
            round: Some(2),
            text: None,
            tool_call_id: None,
        };
        let context = test_context(&host);
        let mut sink = AgentStreamSink::new(
            &context,
            true,
            Some(text_segment.clone()),
            None,
            None,
            Some(tracker.clone()),
        );

        sink.emit(StreamPart::WebSearch {
            queries: Vec::new(),
            citations: Vec::new(),
        })
        .expect("open card");
        sink.emit(StreamPart::WebSearch {
            queries: vec!["kivio release".to_string()],
            citations: Vec::new(),
        })
        .expect("query frame");
        sink.emit(StreamPart::WebSearch {
            queries: vec!["kivio release".to_string()],
            citations: vec![
                WebCitation {
                    title: "A".to_string(),
                    url: "https://a.com".to_string(),
                    ..Default::default()
                },
                WebCitation {
                    title: "dup".to_string(),
                    url: "https://a.com".to_string(),
                    ..Default::default()
                },
            ],
        })
        .expect("citation frame");

        let segments = host.segments.lock().unwrap_or_else(|err| err.into_inner());

        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].id, 1001);
        assert!(segments[0].id < text_segment.id);
        let ContentKind::Search(opened) = &segments[0].kind else {
            panic!("search block")
        };
        assert!(!opened.complete);
        let card_id = opened.id.clone();
        drop(segments);
        let searches = host.searches.lock().unwrap();
        assert_eq!(searches.len(), 3);
        for block in searches.iter() {
            let ContentKind::Search(search) = &block.kind else {
                panic!("search block")
            };
            assert_eq!(search.id, card_id);
            assert!(!search.complete);
        }
        let ContentKind::Search(last) = &searches.last().unwrap().kind else {
            panic!("search block")
        };
        assert_eq!(last.provider, "Test Provider");
        assert_eq!(last.citations.len(), 1);
        assert_eq!(last.queries, vec!["kivio release"]);
        drop(searches);
        let success = sink
            .finish_web_search_card()
            .expect("started card finalizes");
        let ContentKind::Search(search) = &success.kind else {
            panic!("search block")
        };
        assert!(search.complete);
        assert_eq!(search.id, card_id);
        let block = tracker.take_card().expect("card present after start");
        let ContentKind::Search(search) = &block.kind else {
            panic!("search block")
        };
        assert!(search.complete);
        assert_eq!(search.id, card_id);
        assert_eq!(block.id, 1001);
    }

    #[test]
    fn web_search_card_absent_before_first_frame() {
        let tracker =
            WebSearchCardTracker::new(1001, None, ContentPhase::Synthesis, "P".to_string());
        assert!(tracker.take_card().is_none());
        assert!(tracker.finalize_success().is_none());
    }

    #[test]
    fn web_search_part_without_tracker_is_ignored() {
        let host = TestHost::default();
        let context = test_context(&host);
        let mut sink = AgentStreamSink::new(&context, true, None, None, None, None);
        sink.emit(StreamPart::WebSearch {
            queries: vec!["q".to_string()],
            citations: Vec::new(),
        })
        .expect("ignored");
        assert!(host
            .segments
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_empty());
        assert!(host.searches.lock().unwrap().is_empty());
        assert!(host
            .records
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_empty());
    }

    #[test]
    fn stream_output_carries_reasoning_items_into_openai_message() {
        let output = crate::model::GenerateOutput {
            text: String::new(),
            reasoning: None,
            tool_calls: vec![crate::model::PendingToolCall {
                id: "call_1".to_string(),
                function_name: "read".to_string(),
                arguments: serde_json::json!({ "path": "a.png" }),
                arguments_raw: "{\"path\":\"a.png\"}".to_string(),
                arguments_parse_error: None,
                signature: None,
            }],
            usage: None,
            finish_reason: Some(FinishReason::from_openai("tool_calls")),
            cancelled: false,
            web_search: None,
            images: Vec::new(),
            canonical_message: None,
            billing_usage: None,
            reasoning_items: vec![crate::model::ProviderReasoningItem {
                model: "gpt-5.6".to_string(),
                item: serde_json::json!({
                    "type": "reasoning", "id": "rs_1", "summary": [],
                    "encrypted_content": "gAAA"
                }),
            }],
        };
        let stream_output = normalize_stream_output(
            output,
            String::new(),
            String::new(),
            AgentStreamPolicy::PlanningNoDoneUntilNoTools,
        );
        let message =
            crate::model::openai_messages_from_model_messages(&[stream_output.replay_message()])
                .remove(0);
        assert_eq!(message["reasoning_items"][0]["model"], "gpt-5.6");
        assert_eq!(message["reasoning_items"][0]["item"]["id"], "rs_1");
        assert_eq!(message["tool_calls"][0]["id"], "call_1");
    }

    impl AgentRunControl for TestHost {
        fn active(&self) -> bool {
            !self.stopped.load(Ordering::SeqCst)
        }
        fn stop(&self) -> StopAdmission {
            self.stopped.store(true, Ordering::SeqCst);
            StopAdmission::Requested
        }
        fn submit(&self, _kind: InputKind, _message: SteeringMessage) -> InputAdmission {
            InputAdmission::Unsupported
        }
        fn take(&self, _kind: InputKind) -> Vec<SteeringMessage> {
            Vec::new()
        }
        fn wait_inactive(&self) -> AgentHostFuture<'_, ()> {
            Box::pin(std::future::pending())
        }
    }
    impl AgentEventSink for TestHost {
        fn on_event(&self, event: &AgentEvent<'_>) -> EventDelivery {
            match &event.kind {
                AgentEventKind::Content {
                    block: Some(block), ..
                } => self.segments.lock().unwrap().push((*block).clone()),
                AgentEventKind::SearchUpdated { block, announce } => {
                    if *announce {
                        self.segments.lock().unwrap().push((*block).clone());
                    }
                    self.searches.lock().unwrap().push((*block).clone());
                }
                AgentEventKind::ToolUpdated(call) => {
                    self.records.lock().unwrap().push((*call).clone())
                }
                _ => {}
            }
            EventDelivery::Accepted
        }
    }
    fn test_context(host: &TestHost) -> AgentContext<'_> {
        let identity = AgentRunIdentity {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation: 1,
        };
        let control = AgentRunHandle::new(identity, host);
        AgentContext::new(host, host, control)
    }

    #[test]
    fn stopped_stream_ignores_late_frames() {
        let drops = Arc::new(AtomicUsize::new(0));
        let host = TestHost {
            scope_drops: Some(drops.clone()),
            ..Default::default()
        };
        let context = test_context(&host);
        let tracker = ToolCallDraftTracker::new(vec![tool("write")], 1, None, 1000);
        let mut sink =
            AgentStreamSink::new(&context, true, None, None, Some(tracker.clone()), None);
        host.stop();
        sink.emit(StreamPart::ToolCallStart {
            id: "late".into(),
            name: "write".into(),
        })
        .unwrap();
        sink.emit(StreamPart::TextDelta {
            delta: "late text".into(),
        })
        .unwrap();
        assert_eq!(sink.snapshot(), (String::new(), String::new()));
        assert!(!tracker.has_started());
        assert!(host.records.lock().unwrap().is_empty());
        assert!(host.segments.lock().unwrap().is_empty());
    }
    #[test]
    fn disabling_or_dropping_tracker_releases_speculative_scope() {
        let drops = Arc::new(AtomicUsize::new(0));
        let host = TestHost {
            scope_drops: Some(drops.clone()),
            ..Default::default()
        };
        let agent = test_context(&host);
        let context = ToolExecutionContext {
            conversation_id: "conversation",
            run_id: "run",
            message_id: "message",
            generation: 1,
            round: 1,
            tool_call_id: "",
        };
        let mut tool = ToolDefinition::new(crate::model::ModelTool {
            name: "task".into(),
            description: String::new(),
            input_schema: serde_json::json!({"type":"object"}),
        });
        tool.speculative = true;
        let effective_tools = vec![tool.model.clone()];
        let tracker = ToolCallDraftTracker::new(vec![tool.clone()], 1, None, 0);
        tracker.enable_speculation(&host, &context, &effective_tools);
        let other = tracker.clone();
        tracker.disable_speculation();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        tracker.enable_speculation(&host, &context, &effective_tools);
        drop(tracker);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        drop(other);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        host.stop();
        let tracker = ToolCallDraftTracker::new(vec![tool], 1, None, 0);
        {
            let mut sink =
                AgentStreamSink::new(&agent, true, None, None, Some(tracker.clone()), None)
                    .with_speculation(Some((&host, &context)));
            sink.request_started(&ModelCall {
                capabilities: None,
                provider_id: String::new(),
                model: String::new(),
                history: Vec::new(),
                tools: effective_tools,
                options: Default::default(),
                metadata: Default::default(),
                delivery: ModelDelivery::Streamed,
            });
        }
        drop(tracker);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn effective_request_gates_speculation_before_any_stream_part() {
        use crate::model::ToolChoice;
        for (choice, expose_tools, speculative, expected) in [
            (ToolChoice::Auto, true, true, 1),
            (ToolChoice::None, true, true, 0),
            (ToolChoice::Auto, false, true, 0),
            (ToolChoice::Auto, true, false, 0),
        ] {
            let mut tool = tool("task");
            tool.speculative = speculative;
            let effective_tools = if expose_tools {
                vec![tool.model.clone()]
            } else {
                Vec::new()
            };
            let drops = Arc::new(AtomicUsize::new(0));
            let host = TestHost {
                scope_drops: Some(drops.clone()),
                ..Default::default()
            };
            let agent = test_context(&host);
            let context = ToolExecutionContext {
                conversation_id: "conversation",
                run_id: "run",
                message_id: "message",
                generation: 1,
                round: 1,
                tool_call_id: "",
            };
            let tracker = ToolCallDraftTracker::new(vec![tool], 1, None, 0);
            {
                let mut sink =
                    AgentStreamSink::new(&agent, true, None, None, Some(tracker.clone()), None)
                        .with_speculation(Some((&host, &context)));
                sink.request_started(&ModelCall {
                    capabilities: None,
                    provider_id: String::new(),
                    model: String::new(),
                    history: Vec::new(),
                    tools: effective_tools,
                    options: crate::model::GenerateOptions {
                        tool_choice: choice,
                        ..Default::default()
                    },
                    metadata: Default::default(),
                    delivery: ModelDelivery::Streamed,
                });
            }
            drop(tracker);
            assert_eq!(drops.load(Ordering::SeqCst), expected);
        }
    }

    #[test]
    fn restarting_stream_removes_discarded_text_and_reasoning() {
        let host = TestHost::default();
        let agent = test_context(&host);
        let mut sink = AgentStreamSink::new(&agent, true, None, None, None, None);
        sink.emit(StreamPart::TextDelta {
            delta: "failed 🌱".into(),
        })
        .unwrap();
        sink.emit(StreamPart::ReasoningDelta {
            delta: "discarded thought".into(),
        })
        .unwrap();
        sink.reset_attempt();
        sink.emit(StreamPart::TextDelta {
            delta: "accepted answer".into(),
        })
        .unwrap();
        assert_eq!(sink.snapshot(), ("accepted answer".into(), String::new()));
    }
}
