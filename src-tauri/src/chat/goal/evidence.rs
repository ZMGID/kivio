//! Goal evidence is drawn from saved messages AND the exact live protocol run.
//! Tool results are folded into that run synchronously, including within a tool batch.
use crate::chat::protocol::{
    ChatRunSnapshot, ChatRunStatus, ChatToolArtifactPayload, ChatToolPayload,
};
use crate::chat::types::{Conversation, GoalCriterion, GoalStatus};

fn normalized_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = if let Some(rest) = path.strip_prefix("//?/UNC/") {
        format!("//{rest}")
    } else {
        path.strip_prefix("//?/").unwrap_or(&path).to_string()
    };
    if path.as_bytes().get(1) == Some(&b':') || path.starts_with("//") {
        path.to_lowercase()
    } else {
        path
    }
}

fn artifact_matches(artifact: &ChatToolArtifactPayload, reference: &str) -> bool {
    artifact.id.as_deref() == Some(reference)
        || artifact.name == reference
        || artifact
            .path
            .as_deref()
            .is_some_and(|path| normalized_path(path) == normalized_path(reference))
}

/// Shared by progress reporting and completion: never label an unresolved claim verified.
/// `current_run` is a model attestation bound to its real assistant message, not a test result.
pub(super) fn refresh(
    conversation: &mut Conversation,
    live: Option<&ChatRunSnapshot>,
) -> Vec<String> {
    let live = live.filter(|run| {
        run.status == ChatRunStatus::Running
            && run.conversation_id == conversation.id
            && conversation
                .goal_state
                .as_ref()
                .is_some_and(|goal| goal.active_run_id.as_deref() == Some(run.run_id.as_str()))
    });
    let messages = &conversation.messages[conversation.context_clear_start_index()..];
    let mut tools: Vec<ChatToolPayload> = messages
        .iter()
        .flat_map(|message| message.tool_calls.iter())
        .map(|record| ChatToolPayload::from_record(record, String::new()))
        .collect();
    if let Some(run) = live {
        for tool in &run.tools {
            if let Some(existing) = tools.iter_mut().find(|item| item.id == tool.id) {
                *existing = tool.clone();
            } else {
                tools.push(tool.clone());
            }
        }
    }
    let artifacts: Vec<ChatToolArtifactPayload> = messages
        .iter()
        .flat_map(|message| message.artifacts.iter())
        .map(Into::into)
        .chain(
            tools
                .iter()
                .filter(|tool| tool.status == "success")
                .flat_map(|tool| tool.artifacts.clone()),
        )
        .collect();
    let message_ids: Vec<_> = messages
        .iter()
        .filter(|message| message.role == "assistant")
        .map(|message| message.id.as_str())
        .collect();
    let Some(goal) = conversation.goal_state.as_mut() else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    for criterion in &mut goal.criteria {
        if criterion.evidence_kind.as_deref() == Some("model_self_check")
            && criterion.evidence_ref.as_deref() == Some("current_run")
        {
            if let Some(run) = live {
                criterion.evidence_ref = Some(run.message_id.clone());
            }
        }
        let error = validate(criterion, &tools, &artifacts, &message_ids, live);
        criterion.verified = error.is_none();
        if let Some(error) = error {
            errors.push(format!("{}: {error}", criterion.id));
        }
    }
    if super::is_running(goal.status) {
        goal.status = if !goal.criteria.is_empty() && errors.is_empty() {
            GoalStatus::Verifying
        } else {
            GoalStatus::Active
        };
    }
    errors
}

fn validate(
    criterion: &GoalCriterion,
    tools: &[ChatToolPayload],
    artifacts: &[ChatToolArtifactPayload],
    message_ids: &[&str],
    live: Option<&ChatRunSnapshot>,
) -> Option<&'static str> {
    if criterion
        .evidence
        .as_deref()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Some("report concrete evidence for this criterion");
    }
    let reference = criterion.evidence_ref.as_deref().unwrap_or("").trim();
    if reference.is_empty() {
        return Some("evidence reference is empty");
    }
    match criterion.evidence_kind.as_deref() {
        Some("tool_result") => {
            let Some(index) = tools.iter().position(|tool| tool.id == reference) else {
                return Some("tool call not found; use the actual successful tool-call ID");
            };
            if tools[index].status != "success" || tools[index].error.is_some() {
                return Some("referenced tool did not succeed; run the check successfully first");
            }
            // Sequence order handles writes and checks completed within the same second.
            if tools[index + 1..].iter().any(|tool| tool.status == "success" && super::is_mutating_tool(&tool.name)) {
                return Some("a write followed this check; re-run the check and cite its new ID");
            }
            None
        }
        Some("artifact") => {
            (!artifacts.iter().any(|artifact| artifact_matches(artifact, reference)))
                .then_some("artifact not found in successful tool results or saved deliverables; use its returned artifact ID or full path")
        }
        Some("source") => {
            (!(reference.starts_with("https://") || reference.starts_with("http://")))
                .then_some("source evidence requires an http(s) URL")
        }
        Some("model_self_check") => {
            (!(message_ids.contains(&reference) || live.is_some_and(|run| run.message_id == reference)))
                .then_some("model self-check requires current_run or an existing assistant message ID")
        }
        _ => Some("unsupported evidence kind"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conversation() -> Conversation {
        let mut conversation: Conversation = serde_json::from_value(json!({
            "id":"conv", "title":"Goal test", "provider_id":"test", "model":"test",
            "messages":[], "created_at":0, "updated_at":0
        }))
        .unwrap();
        super::super::start(&mut conversation, "Produce a document and check it").unwrap();
        let goal = conversation.goal_state.as_mut().unwrap();
        goal.active_run_id = Some("run".into());
        goal.criteria = vec![
            criterion("c1", "artifact", "art_doc"),
            criterion("c2", "model_self_check", "current_run"),
        ];
        conversation
    }

    fn criterion(id: &str, kind: &str, reference: &str) -> GoalCriterion {
        GoalCriterion {
            id: id.into(),
            text: "Requirement".into(),
            verified: false,
            evidence: Some("The delivered plan contains four focus sessions and breaks".into()),
            evidence_kind: Some(kind.into()),
            evidence_ref: Some(reference.into()),
        }
    }

    fn tool(id: &str, name: &str, status: &str) -> ChatToolPayload {
        serde_json::from_value(json!({
            "id":id,"name":name,"source":"native","status":status,"argumentsPreview":"{}",
            "round":1,"sensitive":false,"artifacts":[],"completedAt":1
        }))
        .unwrap()
    }

    fn live() -> ChatRunSnapshot {
        let mut write = tool("call_write", "write", "success");
        write.artifacts.push(
            serde_json::from_value(json!({
                "id":"art_doc", "name":"plan.md", "mimeType":"text/markdown", "dataUrl":"",
                "path":r"\\?\C:\Users\tester\plan.md"
            }))
            .unwrap(),
        );
        serde_json::from_value(json!({
            "protocolVersion":1,"conversationId":"conv","runId":"run","messageId":"assistant",
            "lastSeq":3,"baseRevision":0,"status":"running","content":"Delivered plan", "reasoning":"",
            "segments":[],"tools":[write],"subagents":[],"pendingInteractions":[],"warnings":[]
        })).unwrap()
    }

    #[test]
    fn live_artifact_and_self_check_validate_before_final_message_is_saved() {
        let mut conversation = conversation();
        assert!(conversation.messages.is_empty());
        assert!(refresh(&mut conversation, Some(&live())).is_empty());
        let goal = conversation.goal_state.unwrap();
        assert_eq!(goal.status, GoalStatus::Verifying);
        assert!(goal.criteria.iter().all(|item| item.verified));
        assert_eq!(goal.criteria[1].evidence_ref.as_deref(), Some("assistant"));
    }

    #[test]
    fn windows_artifact_id_name_and_path_spellings_resolve_the_same_record() {
        for reference in [
            "art_doc",
            "plan.md",
            "C:/Users/tester/plan.md",
            r"C:\Users\tester\plan.md",
            r"\\?\C:\Users\tester\plan.md",
        ] {
            let mut conversation = conversation();
            conversation.goal_state.as_mut().unwrap().criteria[0].evidence_ref =
                Some(reference.into());
            assert!(
                refresh(&mut conversation, Some(&live())).is_empty(),
                "{reference}"
            );
        }
    }

    #[test]
    fn missing_failed_or_foreign_run_evidence_is_not_verified() {
        let mut conversation = conversation();
        let mut run = live();
        run.tools[0].status = "error".into();
        assert!(!refresh(&mut conversation, Some(&run)).is_empty());
        assert!(!conversation.goal_state.as_ref().unwrap().criteria[0].verified);
        run = live();
        run.run_id = "old-run".into();
        assert_eq!(refresh(&mut conversation, Some(&run)).len(), 2);
        run = live();
        run.conversation_id = "other".into();
        assert_eq!(refresh(&mut conversation, Some(&run)).len(), 2);
    }

    #[test]
    fn check_followed_by_a_write_in_the_same_second_requires_rechecking() {
        let mut conversation = conversation();
        conversation.goal_state.as_mut().unwrap().criteria =
            vec![criterion("c1", "tool_result", "check")];
        let mut run = live();
        run.tools = vec![tool("check", "bash", "success")];
        assert!(refresh(&mut conversation, Some(&run)).is_empty());
        run.tools.push(tool("write", "write", "success"));
        assert!(refresh(&mut conversation, Some(&run))[0].contains("re-run"));
        assert!(!conversation.goal_state.as_ref().unwrap().criteria[0].verified);
        run.tools.push(tool("recheck", "bash", "success"));
        conversation.goal_state.as_mut().unwrap().criteria[0].evidence_ref = Some("recheck".into());
        assert!(refresh(&mut conversation, Some(&run)).is_empty());
    }

    #[test]
    fn saved_tool_artifacts_work_without_message_level_artifacts() {
        let mut conversation = conversation();
        conversation.messages.push(serde_json::from_value(json!({
            "id":"assistant","role":"assistant","content":"Plan","timestamp":0,
            "artifacts":[], "tool_calls":[{
                "id":"write", "name":"write", "source":"native", "arguments":"{}", "status":"success",
                "round":1,"sensitive":false,"artifacts":[{
                    "id":"art_doc","name":"plan.md","mime_type":"text/markdown","data_url":""
                }]
            }]
        })).unwrap());
        conversation.goal_state.as_mut().unwrap().criteria[1].evidence_ref =
            Some("assistant".into());
        assert!(refresh(&mut conversation, None).is_empty());
    }
}
