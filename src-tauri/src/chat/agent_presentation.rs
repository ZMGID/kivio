use crate::chat::agent::content::{
    AgentToolCall, AgentToolStatus, ContentBlock, ContentKind, ContentPhase, InputKind,
};
use crate::chat::agent::SteeringMessage;
use crate::chat::model::WebCitation;
use crate::chat::types::{
    ChatMessageSegment, ChatMessageSegmentKind, ChatMessageSegmentPhase, ToolCallRecord,
    ToolCallStatus,
};
use serde_json::Value;
pub(crate) const STEER_TOOL_NAME: &str = "user_steer";
pub(crate) const FOLLOW_UP_TOOL_NAME: &str = "user_follow_up";

pub(crate) fn project_stream_delta(
    delta: &str,
    reasoning_delta: Option<&str>,
    segment: Option<&ChatMessageSegment>,
) -> Vec<crate::chat::protocol::ChatRunEvent> {
    use crate::chat::protocol::{ChatRunEvent, ChatSegmentPayload};
    let segment = segment.map(ChatSegmentPayload::from);
    let reasoning = reasoning_delta.is_some_and(|value| !value.is_empty() || segment.is_some());
    let text = !delta.is_empty() || (segment.is_some() && !reasoning);
    let mut events = Vec::with_capacity(2);
    if reasoning {
        events.push(ChatRunEvent::ReasoningDelta {
            delta: reasoning_delta.unwrap_or_default().into(),
            segment: segment.clone(),
        });
    }
    if text {
        events.push(ChatRunEvent::TextDelta {
            delta: delta.into(),
            segment,
        });
    }
    events
}

pub(crate) fn project_tool_event(record: &ToolCallRecord) -> crate::chat::protocol::ChatRunEvent {
    crate::chat::protocol::ChatRunEvent::ToolUpdated {
        tool: crate::chat::protocol::ChatToolPayload::from_record(
            record,
            crate::chat::agent::execute::truncate_chars(&record.arguments, 800),
        ),
    }
}

pub(crate) fn project_event(
    event: &crate::chat::agent::events::AgentEvent<'_>,
) -> Vec<crate::chat::protocol::ChatRunEvent> {
    use crate::chat::{
        agent::events::AgentEventKind,
        protocol::{ChatRetryStatusPayload, ChatRunEvent},
    };
    match &event.kind {
        AgentEventKind::Content {
            delta,
            reasoning_delta,
            block,
        } => {
            let mut events =
                project_stream_delta(delta, *reasoning_delta, block.map(project_block).as_ref());
            if let Some(record) = block.and_then(project_embedded) {
                events.push(project_tool_event(&record));
            }
            events
        }
        AgentEventKind::ToolUpdated(call) => vec![project_tool_event(&project_tool(
            call,
            Some(event.identity.run_id),
        ))],
        AgentEventKind::SearchUpdated { block, announce } => {
            let mut events = if *announce {
                project_stream_delta("", None, Some(&project_block(block)))
            } else {
                Vec::new()
            };
            if let Some(record) = project_embedded(block) {
                events.push(project_tool_event(&record));
            }
            events
        }
        AgentEventKind::AttemptDiscarded {
            text_chars,
            reasoning_chars,
            block_ids,
            tool_ids,
        } => vec![ChatRunEvent::StreamAttemptDiscarded {
            text_chars: *text_chars,
            reasoning_chars: *reasoning_chars,
            segment_ids: block_ids.iter().map(|id| segment_id(*id)).collect(),
            tool_ids: tool_ids.clone(),
        }],
        AgentEventKind::Status { note } => vec![ChatRunEvent::StatusNoteUpdated {
            note: note.clone(),
            retry: None,
        }],
        AgentEventKind::Retry(retry) => vec![ChatRunEvent::StatusNoteUpdated {
            note: Some(format!(
                "{} · retry {}/{}",
                retry.reason, retry.attempt, retry.max_retries
            )),
            retry: Some(ChatRetryStatusPayload {
                reason: retry.reason.clone(),
                attempt: retry.attempt,
                max_retries: retry.max_retries,
                retry_at_ms: retry.retry_at_ms,
            }),
        }],
        _ => Vec::new(),
    }
}

pub(crate) fn segment_id(id: u32) -> String {
    format!("seg_{id}")
}

pub(crate) fn project_compaction(
    fact: &crate::chat::agent::events::CompactionFact<'_>,
) -> crate::chat::types::CompactionBoundaryRecord {
    crate::chat::types::CompactionBoundaryRecord {
        id: fact.id.into(),
        source_until_message_id: fact.covered_through.into(),
        display_after_message_id: fact.history_tail.map(str::to_owned),
        token_estimate_before: fact.tokens_before,
        token_estimate_after: fact.tokens_after,
        summary_content: fact.summary.into(),
        trigger: fact.trigger.into(),
        created_at: fact.created_at,
    }
}

pub(crate) fn project_block(block: &ContentBlock) -> ChatMessageSegment {
    let (kind, tool_call_id) = match &block.kind {
        ContentKind::Text => (ChatMessageSegmentKind::Text, None),
        ContentKind::Reasoning => (ChatMessageSegmentKind::Reasoning, None),
        ContentKind::Tool => (ChatMessageSegmentKind::Tool, block.tool_call_id.clone()),
        ContentKind::Search(search) => (ChatMessageSegmentKind::Tool, Some(search.id.clone())),
        ContentKind::Input { kind, message } => (
            ChatMessageSegmentKind::Tool,
            Some(input_record(*kind, message, block.round.unwrap_or_default()).id),
        ),
    };
    ChatMessageSegment {
        id: segment_id(block.id),
        kind,
        phase: match block.phase {
            ContentPhase::Plain => ChatMessageSegmentPhase::Plain,
            ContentPhase::ToolLoop => ChatMessageSegmentPhase::ToolLoop,
            ContentPhase::Synthesis => ChatMessageSegmentPhase::Synthesis,
        },
        order: block.id,
        step_number: block.step_number,
        round: block.round,
        text: block.text.clone(),
        tool_call_id,
    }
}

pub(crate) fn project_tool(call: &AgentToolCall, run_id: Option<&str>) -> ToolCallRecord {
    let status = match call.status {
        AgentToolStatus::Pending => ToolCallStatus::Pending,
        AgentToolStatus::Running => ToolCallStatus::Running,
        AgentToolStatus::Success => ToolCallStatus::Success,
        AgentToolStatus::Error => ToolCallStatus::Error,
        AgentToolStatus::Cancelled => ToolCallStatus::Cancelled,
        AgentToolStatus::Skipped => ToolCallStatus::Skipped,
    };
    let mut record = ToolCallRecord {
        id: call.id.clone(),
        name: call.name.clone(),
        source: call.source.clone(),
        server_id: call.server_id.clone(),
        arguments: call.arguments.clone(),
        status,
        result_preview: call.output.as_deref().map(|output| {
            crate::chat::agent::execute::limit_tool_text_for_model(
                &crate::chat::tools::summarize_tool_text(output),
                call.output_limit,
            )
        }),
        error: call.error.clone(),
        duration_ms: call.duration_ms,
        started_at: call.started_at,
        completed_at: call.completed_at,
        round: call.round,
        sensitive: call.sensitive,
        artifacts: call.artifacts.clone(),
        trace_id: run_id.map(str::to_owned),
        span_id: run_id.map(|_| format!("tool_round_{}_{}", call.round, call.id)),
        structured_content: call.structured_content.clone(),
    };
    if let Some(draft) = &call.draft {
        let phase = if call.status == AgentToolStatus::Error {
            "error"
        } else if draft.complete {
            "arguments_ready"
        } else {
            "generating_arguments"
        };
        let chars = call.arguments.chars().count();
        if !draft.complete {
            record.arguments = tool_draft_arguments(&draft.model_name, phase, chars);
        }
        record.result_preview =
            (phase != "error").then(|| tool_draft_preview(&draft.model_name, phase, chars));
        record.structured_content = Some(tool_draft_structured_content(
            &draft.model_name,
            phase,
            chars,
        ));
    }
    record
}

fn input_record(kind: InputKind, message: &SteeringMessage, round: u32) -> ToolCallRecord {
    match kind {
        InputKind::Steer => build_steer_record(message, round),
        InputKind::FollowUp => build_follow_up_record(message, round),
    }
}

pub(crate) fn project_embedded(block: &ContentBlock) -> Option<ToolCallRecord> {
    match &block.kind {
        ContentKind::Search(search) => Some(build_web_search_record(
            &search.id,
            &search.provider,
            &search.queries,
            &search.citations,
            if search.complete {
                ToolCallStatus::Success
            } else {
                ToolCallStatus::Running
            },
            block.round,
        )),
        ContentKind::Input { kind, message } => Some(input_record(
            *kind,
            message,
            block.round.unwrap_or_default(),
        )),
        _ => None,
    }
}

pub(crate) fn project_snapshot(
    calls: &[AgentToolCall],
    blocks: &[ContentBlock],
    run_id: Option<&str>,
) -> (Vec<ToolCallRecord>, Vec<ChatMessageSegment>) {
    let mut tools: Vec<_> = calls
        .iter()
        .map(|call| project_tool(call, run_id))
        .collect();
    tools.extend(blocks.iter().filter_map(project_embedded));
    (tools, blocks.iter().map(project_block).collect())
}

pub(crate) fn build_web_search_record(
    id: &str,
    provider_label: &str,
    queries: &[String],
    citations: &[WebCitation],
    status: ToolCallStatus,
    round: Option<u32>,
) -> ToolCallRecord {
    // structured_content 对齐前端内置搜索来源卡读取的形状（type 作路由标记）。
    let structured = serde_json::json!({
        "type": "builtin_web_search",
        "provider": provider_label,
        "queries": queries,
        "citations": citations,
    });
    // 折叠行「目标」显示查询词：ToolCallBlock 从 arguments.query 取。
    let arguments = serde_json::json!({ "query": queries.join("; ") }).to_string();
    // 纯文本兜底：前端旧版本不识别 structured 时仍能看到来源列表。
    let mut preview = String::new();
    for citation in citations {
        preview.push_str(&format!("- {} — {}\n", citation.title, citation.url));
    }
    ToolCallRecord {
        id: id.to_string(),
        name: "web_search".to_string(),
        source: "native".to_string(),
        server_id: None,
        arguments,
        status,
        result_preview: if preview.trim().is_empty() {
            None
        } else {
            Some(preview.trim().to_string())
        },
        error: None,
        duration_ms: None,
        started_at: None,
        completed_at: None,
        round: round.unwrap_or(0),
        sensitive: false,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: Some(structured),
    }
}

pub fn build_steer_record(message: &SteeringMessage, round: u32) -> ToolCallRecord {
    ToolCallRecord {
        id: format!("steer_{}", message.id),
        name: STEER_TOOL_NAME.to_string(),
        source: "native".to_string(),
        server_id: None,
        arguments: serde_json::json!({ "text": message.text }).to_string(),
        status: ToolCallStatus::Success,
        // 纯文本兜底：不识别 structured 的旧前端仍能看到这句话，而不是一张空卡。
        result_preview: Some(message.text.clone()),
        error: None,
        duration_ms: None,
        started_at: None,
        completed_at: None,
        round,
        sensitive: false,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: Some(serde_json::json!({
            "type": "user_steer",
            "steer_id": message.id,
            "text": message.text,
        })),
    }
}

/// 原生 follow-up 的显示记录。它与 steer 同样渲染成时间线里的用户小气泡，
/// 但保留独立类型，避免把“当前轮次引导”和“轮末追加”混成一种协议语义。
pub fn build_follow_up_record(message: &SteeringMessage, round: u32) -> ToolCallRecord {
    ToolCallRecord {
        id: format!("follow_up_{}", message.id),
        name: FOLLOW_UP_TOOL_NAME.to_string(),
        source: "native".to_string(),
        server_id: None,
        arguments: serde_json::json!({ "text": message.text }).to_string(),
        status: ToolCallStatus::Success,
        result_preview: Some(message.text.clone()),
        error: None,
        duration_ms: None,
        started_at: None,
        completed_at: None,
        round,
        sensitive: false,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: Some(serde_json::json!({
            "type": "user_follow_up",
            "follow_up_id": message.id,
            "text": message.text,
        })),
    }
}

fn tool_draft_arguments(name: &str, phase: &str, argument_chars: usize) -> String {
    serde_json::json!({
        "_kivioToolDraft": true,
        "tool": name,
        "phase": phase,
        "argumentChars": argument_chars,
    })
    .to_string()
}

fn tool_draft_structured_content(name: &str, phase: &str, argument_chars: usize) -> Value {
    serde_json::json!({
        "toolDraft": {
            "toolName": name,
            "phase": phase,
            "argumentChars": argument_chars,
        }
    })
}

fn tool_draft_preview(name: &str, phase: &str, argument_chars: usize) -> String {
    if name == "present_artifacts" {
        return if phase == "arguments_ready" {
            "工具参数已生成，等待调用…".to_string()
        } else {
            "正在展示文件…".to_string()
        };
    }
    if phase == "arguments_ready" {
        return "工具参数已生成，等待调用…".to_string();
    }
    let prefix = match name {
        "write" => "正在生成文件内容",
        "edit" => "正在生成编辑参数",
        _ => "正在生成工具参数",
    };
    if argument_chars == 0 {
        format!("{prefix}…")
    } else {
        format!("{prefix}…已收到 {} 字符", format_count(argument_chars))
    }
}

fn format_count(value: usize) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (idx, ch) in text.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub(crate) fn run_status(status: &crate::chat::agent::types::AgentRunStatus) -> &'static str {
    use crate::chat::agent::types::AgentRunStatus;
    match status {
        AgentRunStatus::Completed => "completed",
        AgentRunStatus::Recovered => "recovered",
        AgentRunStatus::CompactionBlocked => "compaction_blocked",
        AgentRunStatus::Cancelled => "cancelled",
        AgentRunStatus::Failed(_) => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_draft_projection_preserves_progress_errors_and_completed_arguments() {
        for name in ["write", "present_artifacts"] {
            let mut call = AgentToolCall {
                id: "call".into(),
                name: name.into(),
                source: "native".into(),
                server_id: None,
                arguments: r#"{"path":"demo.html"}"#.into(),
                status: AgentToolStatus::Pending,
                output: None,
                output_limit: None,
                error: None,
                duration_ms: None,
                started_at: None,
                completed_at: None,
                round: 2,
                sensitive: false,
                artifacts: Vec::new(),
                structured_content: None,
                draft: Some(crate::chat::agent::content::ToolArgumentState {
                    model_name: name.into(),
                    complete: false,
                }),
            };
            let pending = project_tool(&call, Some("run"));
            assert_eq!(pending.status, ToolCallStatus::Pending);
            assert_eq!(pending.trace_id.as_deref(), Some("run"));
            assert_eq!(pending.span_id.as_deref(), Some("tool_round_2_call"));
            let preview = pending.result_preview.unwrap();
            if name == "write" {
                assert!(preview.contains("正在生成文件内容"));
            } else {
                assert!(preview.contains("正在展示文件"));
                assert!(!preview.contains("已收到"));
            }
            call.status = AgentToolStatus::Error;
            let failed = project_tool(&call, Some("run"));
            assert_eq!(failed.status, ToolCallStatus::Error);
            assert!(failed.result_preview.is_none());
            let details = failed.structured_content.unwrap();
            assert_eq!(details["toolDraft"]["phase"], "error");
            assert_eq!(
                details["toolDraft"]["argumentChars"],
                call.arguments.chars().count()
            );
            call.status = AgentToolStatus::Pending;
            call.draft.as_mut().unwrap().complete = true;
            let ready = project_tool(&call, Some("run"));
            assert!(ready.result_preview.unwrap().contains("工具参数已生成"));
            assert_eq!(ready.arguments, call.arguments);
            let block = ContentBlock {
                id: 1002,
                kind: ContentKind::Tool,
                phase: ContentPhase::ToolLoop,
                step_number: Some(3),
                round: Some(2),
                text: None,
                tool_call_id: Some("call".into()),
            };
            let (_, segments) = project_snapshot(&[call], &[block], Some("run"));
            assert_eq!(segments[0].kind, ChatMessageSegmentKind::Tool);
            assert_eq!(segments[0].phase, ChatMessageSegmentPhase::ToolLoop);
            assert_eq!(segments[0].tool_call_id.as_deref(), Some("call"));
            assert_eq!(segments[0].order, 1002);
        }
    }

    #[test]
    fn search_projection_keeps_running_and_completed_citations() {
        for complete in [false, true] {
            let block = ContentBlock {
                id: 1001,
                kind: ContentKind::Search(crate::chat::agent::content::ProviderSearch {
                    id: "search".into(),
                    provider: "provider".into(),
                    queries: vec!["query".into()],
                    citations: vec![serde_json::from_value(
                        serde_json::json!({"title":"source", "url":"https://example.test/source"}),
                    )
                    .unwrap()],
                    complete,
                }),
                phase: ContentPhase::ToolLoop,
                step_number: None,
                round: Some(1),
                text: None,
                tool_call_id: None,
            };
            let (records, segments) = project_snapshot(&[], &[block], Some("run"));
            assert_eq!(
                records[0].status,
                if complete {
                    ToolCallStatus::Success
                } else {
                    ToolCallStatus::Running
                }
            );
            assert_eq!(records[0].name, "web_search");
            assert_eq!(records[0].source, "native");
            let data = records[0].structured_content.as_ref().unwrap();
            assert_eq!(data["queries"][0], "query");
            assert_eq!(data["citations"][0]["url"], "https://example.test/source");
            assert_eq!(segments[0].kind, ChatMessageSegmentKind::Tool);
            assert_eq!(segments[0].tool_call_id.as_deref(), Some("search"));
        }
    }

    #[test]
    fn formats_raw_search_result_before_applying_display_limit() {
        let call = AgentToolCall {
            id: "search".into(),
            name: "search_web".into(),
            source: "native".into(),
            server_id: None,
            arguments: "{}".into(),
            status: AgentToolStatus::Success,
            output: Some(
                serde_json::json!({"answer":"short answer", "payload":"x".repeat(4096)})
                    .to_string(),
            ),
            output_limit: Some(64),
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 1,
            sensitive: false,
            artifacts: Vec::new(),
            structured_content: None,
            draft: None,
        };
        let record = project_tool(&call, Some("run"));
        assert_eq!(record.result_preview.as_deref(), Some("答: short answer"));
        assert_eq!(record.span_id.as_deref(), Some("tool_round_1_search"));
    }
    #[test]
    fn reserved_search_and_consumed_input_use_the_same_live_and_saved_projection() {
        let mut block = ContentBlock {
            id: 1001,
            kind: ContentKind::Search(crate::chat::agent::content::ProviderSearch {
                id: "search".into(),
                provider: "provider".into(),
                queries: vec!["query".into()],
                citations: Vec::new(),
                complete: false,
            }),
            phase: ContentPhase::ToolLoop,
            step_number: None,
            round: Some(1),
            text: None,
            tool_call_id: None,
        };
        for kind in [
            block.kind.clone(),
            ContentKind::Input {
                kind: InputKind::Steer,
                message: SteeringMessage::new("input".into(), "继续🙂").unwrap(),
            },
        ] {
            block.kind = kind;
            let identity = crate::chat::agent::events::AgentRunIdentity {
                conversation_id: "session",
                run_id: "run",
                message_id: "message",
                generation: 1,
            };
            let event = crate::chat::agent::events::AgentEvent {
                identity,
                invocation_id: 1,
                sequence: 1,
                kind: crate::chat::agent::events::AgentEventKind::Content {
                    delta: "",
                    reasoning_delta: None,
                    block: Some(&block),
                },
            };
            let live = project_event(&event);
            let (records, blocks) = project_snapshot(&[], &[block.clone()], Some("run"));
            assert_eq!(live.len(), 2);
            assert_eq!(blocks[0].id, "seg_1001");
            assert!(
                matches!(&live[0], crate::chat::protocol::ChatRunEvent::TextDelta { delta, segment: Some(segment) } if delta.is_empty() && segment.id == blocks[0].id)
            );
            assert!(
                matches!(&live[1], crate::chat::protocol::ChatRunEvent::ToolUpdated { tool } if tool.id == records[0].id && tool.structured_content == records[0].structured_content)
            );
        }
    }
}
