use super::content::{ContentPhase, ContentKind, ContentBlock, AgentToolCall};
use super::history::{export_history, RuntimeMessage};

use crate::chat::model::{GenerateOutput, ModelMessage, PendingToolCall};

use super::loop_::{LoopEnv, RunState};
use super::stop::{
    final_assistant_message, final_response_from_planning_message_with_images, merge_reasoning,
};
use super::stream::ToolCallDraftTracker;
use super::synthesis::SynthesisCompleted;
use super::types::{AgentPhase, AgentRunResult};

/// Converges the repeated "emit fallback + push api message + build AgentRunResult"
/// blocks in the agent loop. Emit order is fixed: stream delta (with optional
/// segment) -> stream done -> api message push (guarded or always) -> segment
/// append -> result construction.
pub(crate) struct RunResultBuilder<'a> {
    host: &'a super::context::AgentContext<'a>,
    content: String,
    api_reasoning: Option<String>,
    replay: Option<ModelMessage>,
    reasoning_tail: Option<String>,
    emit_segment: Option<ContentBlock>,
    append_template: Option<ContentBlock>,
    emit_done_reason: Option<&'static str>,
    push_api_always: bool,
    outcome: super::types::AgentRunStatus,
}

impl<'a> RunResultBuilder<'a> {
    pub(crate) fn new(host: &'a super::context::AgentContext<'a>, content: String) -> Self {
        Self {
            host,
            content,
            api_reasoning: None,
            replay: None,
            reasoning_tail: None,
            emit_segment: None,
            append_template: None,
            emit_done_reason: None,
            push_api_always: false,
            outcome: super::types::AgentRunStatus::Completed,
        }
    }

    /// Use `template` both as the segment attached to the emitted delta and as
    /// the template appended to the segment builder during `finish`.
    pub(crate) fn segment(mut self, template: &ContentBlock) -> Self {
        self.emit_segment = Some(template.clone());
        self.append_template = Some(template.clone());
        self
    }

    /// Attach an already-appended segment to the emitted delta without
    /// re-appending it during `finish`.
    pub(crate) fn emit_segment_opt(mut self, segment: Option<ContentBlock>) -> Self {
        self.emit_segment = segment;
        self
    }

    pub(crate) fn api_reasoning(mut self, reasoning: Option<String>) -> Self {
        self.api_reasoning = reasoning;
        self
    }

    pub(crate) fn replay(mut self, message: Option<ModelMessage>) -> Self {
        self.replay = message;
        self
    }

    pub(crate) fn reasoning_tail(mut self, reasoning: Option<String>) -> Self {
        self.reasoning_tail = reasoning;
        self
    }

    pub(crate) fn emit_done(mut self, reason: &'static str) -> Self {
        self.emit_done_reason = Some(reason);
        self
    }

    pub(crate) fn push_api_always(mut self) -> Self {
        self.push_api_always = true;
        self
    }

    pub(crate) fn outcome(mut self, outcome: super::types::AgentRunStatus) -> Self {
        self.outcome = outcome;
        self
    }

    /// 挂上降级兜底的结构化描述，让前端渲染成独立错误卡片。

    fn emit(&self) {
        if self.emit_done_reason.is_some() {
            self.host.content(&self.content, None, self.emit_segment.as_ref());
        }
    }

    fn push_api_message(&self, generated_api_messages: &mut Vec<RuntimeMessage>) {
        if self.outcome == super::types::AgentRunStatus::Cancelled
            && self.content.is_empty()
            && self.api_reasoning.as_deref().is_none_or(str::is_empty)
        {
            return;
        }
        if self.push_api_always || !generated_api_messages.is_empty() {
            generated_api_messages.push(RuntimeMessage::model(self.replay.clone().unwrap_or_else(
                || final_assistant_message(&self.content, self.api_reasoning.as_deref()),
            )));
        }
    }

    /// Emit + api push only, for fallbacks that flow into the common loop tail
    /// (segment append and result construction happen there). Returns the
    /// fallback content.
    pub(crate) fn emit_and_record(
        self,
        generated_api_messages: &mut Vec<RuntimeMessage>,
    ) -> String {
        self.emit();
        self.push_api_message(generated_api_messages);
        self.content
    }

    /// Emit + api push + segment append + AgentRunResult construction, for
    /// fallbacks that end the run immediately.
    pub(crate) fn finish(
        self,
        mut segment_builder: SegmentBuilder,
        planning_reasoning_parts: &[String],
        tool_records: Vec<AgentToolCall>,
        mut generated_api_messages: Vec<RuntimeMessage>,
    ) -> AgentRunResult {
        self.emit();
        self.push_api_message(&mut generated_api_messages);
        if let Some(template) = &self.append_template {
            segment_builder.append_text_from_template(template, self.content.clone());
        }
        AgentRunResult {
            final_message: self.replay,
            content: self.content,
            reasoning: merge_reasoning(planning_reasoning_parts, self.reasoning_tail),
            tool_records,
            segments: segment_builder.all(),
            api_messages: export_history(&generated_api_messages),
            status: self.outcome,
            usage: None,
            cache_pairs: None,
            last_step_usage: None,
            history: Vec::new(),
            compacted: false,
            images: Vec::new(),
        }
    }
}

pub(crate) struct SegmentBuilder {
    next_order: u32,
    segments: Vec<ContentBlock>,
}

impl Default for SegmentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SegmentBuilder {
    pub(crate) fn new() -> Self {
        Self {
            next_order: 1000,
            segments: Vec::new(),
        }
    }

    pub(crate) fn reserve(
        &mut self,
        kind: ContentKind,
        phase: ContentPhase,
        step_number: Option<u8>,
        round: Option<u32>,
        _suffix: &str,
    ) -> ContentBlock {
        let segment = ContentBlock {
            id: self.next_order,
            kind,
            phase,
            step_number,
            round,
            text: None,
            tool_call_id: None,
        };
        self.next_order = self.next_order.saturating_add(1);
        segment
    }

    pub(crate) fn append_text_from_template(
        &mut self,
        template: &ContentBlock,
        text: impl Into<String>,
    ) -> Option<ContentBlock> {
        let text = text.into();
        if text.trim().is_empty() {
            return None;
        }
        let mut segment = template.clone();
        segment.text = Some(text);
        self.segments.push(segment.clone());
        Some(segment)
    }

    pub(crate) fn append_tool_calls(
        &mut self,
        phase: ContentPhase,
        step_number: Option<u8>,
        round: u32,
        calls: &[PendingToolCall],
    ) -> Vec<ContentBlock> {
        let mut segments = Vec::new();
        for call in calls {
            let segment = ContentBlock {
                id: self.next_order,
                kind: ContentKind::Tool,
                phase: phase.clone(),
                step_number,
                round: Some(round),
                text: None,
                tool_call_id: Some(call.id.clone()),
            };
            self.next_order = self.next_order.saturating_add(1);
            self.segments.push(segment.clone());
            segments.push(segment);
        }
        segments
    }

    pub(crate) fn append_existing_segments(
        &mut self,
        mut segments: Vec<ContentBlock>,
    ) -> Vec<ContentBlock> {
        for segment in &segments {
            self.next_order = self.next_order.max(segment.id.saturating_add(1));
        }
        self.segments.extend(segments.iter().cloned());
        segments.sort_by_key(|segment| segment.id);
        segments
    }

    pub(crate) fn next_order(&self) -> u32 {
        self.next_order
    }

    /// 只吞一个 order 号、不造段——给流式实时卡（内置 web_search）在 reasoning 与正文之间
    /// 预留位置槽（任务 07-23）。未搜索时该 order 号留空洞无害：前端按 order sort、后端按
    /// `max()` 取下一个，均不要求 order 连续。
    pub(crate) fn reserve_order(&mut self) -> u32 {
        let order = self.next_order;
        self.next_order = self.next_order.saturating_add(1);
        order
    }

    /// Borrow the segments accumulated so far without consuming the builder.
    /// Used by the loop's per-round crash-safety checkpoint to snapshot the
    /// in-progress assistant message; `all()` (which moves) still produces the
    /// final segment list at finalize time.
    pub(crate) fn segments(&self) -> &[ContentBlock] {
        &self.segments
    }

    pub(crate) fn all(self) -> Vec<ContentBlock> {
        self.segments
    }
}

pub(crate) fn segment_phase_for_agent_phase(phase: AgentPhase) -> ContentPhase {
    match phase {
        AgentPhase::Plain => ContentPhase::Plain,
        AgentPhase::Synthesis => ContentPhase::Synthesis,
        AgentPhase::ToolLoop => ContentPhase::ToolLoop,
    }
}

/// Final result when planning ended with a natural answer (no tool calls left).
pub(crate) fn finalize_planning_final(
    state: &mut RunState,
    message: GenerateOutput,
) -> Result<AgentRunResult, String> {
    let (response, reasoning) = final_response_from_planning_message_with_images(
        &message,
        &state.planning_reasoning_parts,
        !state.generated_images.is_empty(),
    )?;
    let replay = message.replay_message();
    if !state.generated_api_messages.is_empty()
        || message.canonical_message.is_some()
        || !message.reasoning_items.is_empty()
    {
        state
            .generated_api_messages
            .push(RuntimeMessage::model(replay.clone()));
    }
    Ok(AgentRunResult {
        final_message: Some(replay),
        content: response,
        reasoning,
        tool_records: std::mem::take(&mut state.tool_records),
        segments: std::mem::take(&mut state.segment_builder).all(),
        api_messages: export_history(&std::mem::take(&mut state.generated_api_messages)),
        status: super::types::AgentRunStatus::Completed,
        usage: None,
        cache_pairs: None,
        last_step_usage: None,
        history: Vec::new(),
        compacted: false,
        images: Vec::new(),
    })
}

/// Common loop tail: append final segments, push the closing step and build the
/// completed run result.
pub(crate) fn finalize_completed(
    _env: &LoopEnv<'_>,
    state: &mut RunState,
    completed: SynthesisCompleted,
) -> AgentRunResult {
    let SynthesisCompleted {
        replay,
        response,
        reasoning,
        response_reasoning,
        response_segment,
        response_reasoning_segment,
    } = completed;
    if !response.trim().is_empty() {
        state
            .segment_builder
            .append_text_from_template(&response_segment, response.clone());
    }
    if let Some(reasoning_part) = response_reasoning
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        state
            .segment_builder
            .append_text_from_template(&response_reasoning_segment, reasoning_part);
    }

    AgentRunResult {
        final_message: replay,
        content: response,
        reasoning,
        tool_records: std::mem::take(&mut state.tool_records),
        segments: std::mem::take(&mut state.segment_builder).all(),
        api_messages: export_history(&std::mem::take(&mut state.generated_api_messages)),
        status: super::types::AgentRunStatus::Completed,
        usage: None,
        cache_pairs: None,
        last_step_usage: None,
        history: Vec::new(),
        compacted: false,
        images: Vec::new(),
    }
}

pub(crate) fn tool_planning_failed_run_result(
    host: &super::context::AgentContext<'_>,
    content: String,
    mut segment_builder: SegmentBuilder,
    planning_text_segment: ContentBlock,
    tool_draft_tracker: ToolCallDraftTracker,
    planning_reasoning_parts: &[String],
    generated_api_messages: Vec<RuntimeMessage>,
    error: String,
) -> AgentRunResult {
    let failed_records = tool_draft_tracker.mark_error(&error);
    for record in &failed_records {
        host.tool_updated(record);
    }

    segment_builder.append_existing_segments(tool_draft_tracker.segments());
    let mut final_segment = planning_text_segment;
    final_segment.phase = ContentPhase::Synthesis;
    final_segment.round = None;
    let appended_segment =
        segment_builder.append_text_from_template(&final_segment, content.clone());

    RunResultBuilder::new(
        host,
        content,
    )
    .emit_segment_opt(appended_segment)
    .emit_done("done")
    .push_api_always()
    .outcome(super::types::AgentRunStatus::Failed(error))
    .finish(
        segment_builder,
        planning_reasoning_parts,
        failed_records,
        generated_api_messages,
    )
}

pub(crate) fn cancelled_tool_round_run_result(
    planning_reasoning_parts: &[String],
    tool_records: Vec<AgentToolCall>,
    segments: Vec<ContentBlock>,
    generated_api_messages: Vec<RuntimeMessage>,
) -> AgentRunResult {
    AgentRunResult {
        final_message: None,
        content: String::new(),
        reasoning: merge_reasoning(planning_reasoning_parts, None),
        tool_records,
        segments,
        api_messages: export_history(&generated_api_messages),
        status: super::types::AgentRunStatus::Cancelled,
        usage: None,
        cache_pairs: None,
        last_step_usage: None,
        history: Vec::new(),
        compacted: false,
        images: Vec::new(),
    }
}

/// Build a cancelled `AgentRunResult` from the loop's accumulated `state`.
/// Used by the planning/loop-top cancellation paths (no partial streamed
/// answer to preserve) so they end with `Ok(cancelled_result)` carrying the
/// tool records / segments / api messages gathered up to the cancel point —
/// mirroring the tool-round cancellation path so the whole turn is persisted
/// instead of being dropped via a bare `Err("cancelled")`.
pub(crate) fn cancelled_run_result_from_state(state: &mut RunState) -> AgentRunResult {
    cancelled_tool_round_run_result(
        &state.planning_reasoning_parts,
        std::mem::take(&mut state.tool_records),
        std::mem::take(&mut state.segment_builder).all(),
        std::mem::take(&mut state.generated_api_messages),
    )
}
