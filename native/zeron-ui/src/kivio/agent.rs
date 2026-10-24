//! Shared agent interactions use Zeron's existing question surface.
use super::*;
use crate::chat_runtime::agent_host::AgentEvent;
use kivio_model_runtime::{
    ask_user::{self, AskUserAnswer, AskUserPromptPayload, AskUserResponseResult},
    conversation_data::ChatMessage,
};
use zeron_proto::{UserInputAnswer, UserInputQuestion};

/// Zeron returns labels, while the shared ask_user owner requires stable option IDs.
/// Disambiguate duplicate labels before rendering and use the same mapping on reply.
fn question_option_labels(question: &ask_user::AskUserQuestion) -> Vec<String> {
    let mut used: std::collections::HashSet<_> = question
        .options
        .iter()
        .map(|option| option.label.clone())
        .collect();
    question
        .options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            if question
                .options
                .iter()
                .filter(|other| other.label == option.label)
                .count()
                == 1
            {
                return option.label.clone();
            }
            let mut suffix = index + 1;
            loop {
                let label = format!("{} · 选项 {}", option.label, suffix);
                if used.insert(label.clone()) {
                    return label;
                }
                suffix += 1;
            }
        })
        .collect()
}

pub(super) fn question_response(
    prompt: &AskUserPromptPayload,
    answers: &[UserInputAnswer],
) -> Result<AskUserResponseResult, String> {
    if answers.is_empty() {
        return Ok(ask_user::skipped_response());
    }
    let mut response = AskUserResponseResult {
        phase: ask_user::ASK_USER_PHASE_ANSWERED.into(),
        answers: Default::default(),
    };
    for answer in answers {
        let question = prompt
            .questions
            .iter()
            .find(|q| q.id == answer.question_id)
            .ok_or("回答包含未知问题")?;
        let mut selected = Vec::new();
        let mut custom = Vec::new();
        let labels = question_option_labels(question);
        for label in &answer.labels {
            match labels.iter().position(|candidate| candidate == label) {
                Some(index) => selected.push(question.options[index].id.clone()),
                None => custom.push(label.clone()),
            }
        }
        if response
            .answers
            .insert(
                question.id.clone(),
                AskUserAnswer {
                    selected_option_ids: selected,
                    custom_text: (!custom.is_empty()).then(|| custom.join("\n")),
                },
            )
            .is_some()
        {
            return Err("同一问题只能提交一次回答".into());
        }
    }
    ask_user::validate_response(prompt, response)
}
impl KivioService {
    pub(super) fn agent_event(&self, chat_id: &str, message: &mut ChatMessage, event: AgentEvent) {
        match event {
            AgentEvent::Segment(segment) => {
                if let Some(existing) = message.segments.iter_mut().find(|s| s.id == segment.id) {
                    *existing = segment;
                } else {
                    message.segments.push(segment);
                }
            }
            AgentEvent::Tool(record) => {
                if let Some(existing) = message.tool_calls.iter_mut().find(|r| r.id == record.id) {
                    *existing = record;
                } else {
                    message.tool_calls.push(record);
                }
            }
            AgentEvent::Rollback {
                text_chars,
                reasoning_chars,
                segment_ids,
                tool_ids,
            } => {
                fn truncate(value: &mut String, count: u32) {
                    *value = value
                        .chars()
                        .take(value.chars().count().saturating_sub(count as usize))
                        .collect();
                }
                truncate(&mut message.content, text_chars);
                if let Some(reasoning) = &mut message.reasoning {
                    truncate(reasoning, reasoning_chars);
                }
                message.segments.retain(|s| !segment_ids.contains(&s.id));
                message.tool_calls.retain(|r| !tool_ids.contains(&r.id));
            }
            AgentEvent::Approval {
                title,
                detail,
                session,
                response,
            } => {
                if session && self.consents.lock().unwrap().contains(chat_id) {
                    let _ = response.send(true);
                    return;
                }
                let receiver = self.ask_validated(
                    chat_id,
                    vec![UserInputQuestion {
                        id: "permission".into(),
                        header: title,
                        question: detail,
                        options: vec!["拒绝".into(), "允许".into()],
                        multi_select: false,
                        prefill: None,
                        multiline: false,
                    }],
                    None,
                    true,
                );
                let consents = self.consents.clone();
                let chat_id = chat_id.to_owned();
                tokio::spawn(async move {
                    let approved = receiver
                        .await
                        .unwrap_or_default()
                        .iter()
                        .any(|a| a.question_id == "permission" && a.labels == ["允许"]);
                    // Never carry a late response into a cancelled run or another run.
                    if response.send(approved).is_ok() && approved && session {
                        consents.lock().unwrap().insert(chat_id);
                    }
                });
            }
            AgentEvent::Question { prompt, response } => {
                let questions = prompt
                    .questions
                    .iter()
                    .map(|q| UserInputQuestion {
                        id: q.id.clone(),
                        header: prompt.title.clone().unwrap_or_else(|| "问题".into()),
                        question: q.prompt.clone(),
                        options: question_option_labels(q),
                        multi_select: q.allow_multiple,
                        prefill: None,
                        multiline: true,
                    })
                    .collect();
                let receiver = self.ask_validated(chat_id, questions, Some(prompt.clone()), false);
                tokio::spawn(async move {
                    let result = match receiver.await {
                        Ok(answers) => question_response(&prompt, &answers)
                            .unwrap_or_else(|_| ask_user::cancelled_response()),
                        Err(_) => ask_user::cancelled_response(),
                    };
                    let _ = response.send(result);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> ChatMessage {
        serde_json::from_value(json!({"id":"assistant","role":"assistant","content":"前文😀中","reasoning":"已存想法🤔新","timestamp":1})).unwrap()
    }
    fn command(request: &str, answers: Value) -> Value {
        json!({"chatId":"conv_bridge", "command":{"kind":"respondInput","requestId":request,"answers":answers}})
    }

    #[tokio::test]
    async fn invalid_approval_keeps_pending_and_cancel_rejects_late_reply() {
        let root = tempfile::tempdir().unwrap();
        let service = KivioService::new(root.path().into());
        let (response, mut answer) = tokio::sync::oneshot::channel();
        service.agent_event(
            "conv_bridge",
            &mut message(),
            AgentEvent::Approval {
                title: "Approve".into(),
                detail: "Tool".into(),
                session: true,
                response,
            },
        );
        let id = service.input_parts("conv_bridge")[0]["requestId"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            service
                .respond_input(command(
                    &id,
                    json!([{"questionId":"permission","labels":["允许","拒绝"]}])
                ))
                .is_err()
        );
        assert!(service.respond_input(command(&id, json!([{"questionId":"permission","labels":["允许"]},{"questionId":"permission","labels":["拒绝"]}]))).is_err());
        assert_eq!(service.input_parts("conv_bridge").len(), 1);
        assert!(matches!(
            answer.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        service
            .handle("KivioCancel", json!({"chatId":"conv_bridge"}))
            .await
            .unwrap();
        assert!(!answer.await.unwrap());
        assert!(service.input_parts("conv_bridge").is_empty());
        assert!(
            service
                .respond_input(command(
                    &id,
                    json!([{"questionId":"permission","labels":["允许"]}])
                ))
                .is_err()
        );
        assert!(!service.consents.lock().unwrap().contains("conv_bridge"));
    }

    #[tokio::test]
    async fn corrected_question_preserves_distinct_option_identity() {
        let root = tempfile::tempdir().unwrap();
        let service = KivioService::new(root.path().into());
        let prompt = ask_user::normalize_prompt(json!({"questions":[{"id":"choice","prompt":"Which?","options":[{"id":"first","label":"Same"},{"id":"second","label":"Same"}]}]})).unwrap();
        let (response, answer) = tokio::sync::oneshot::channel();
        service.agent_event(
            "conv_bridge",
            &mut message(),
            AgentEvent::Question { prompt, response },
        );
        let input = service.input_parts("conv_bridge").remove(0);
        let id = input["requestId"].as_str().unwrap();
        let options = input["questions"][0]["options"].as_array().unwrap();
        assert_ne!(options[0], options[1]);
        assert!(
            service
                .respond_input(command(
                    id,
                    json!([{"questionId":"choice","labels":options}])
                ))
                .is_err()
        );
        assert_eq!(service.input_parts("conv_bridge").len(), 1);
        service
            .respond_input(command(
                id,
                json!([{"questionId":"choice","labels":[options[1]]}]),
            ))
            .unwrap();
        let answer = answer.await.unwrap();
        assert_eq!(answer.answers["choice"].selected_option_ids, ["second"]);
        assert!(service.input_parts("conv_bridge").is_empty());
    }

    #[test]
    fn retry_rollback_uses_unicode_characters_and_preserves_prior_tool_records() {
        let root = tempfile::tempdir().unwrap();
        let service = KivioService::new(root.path().into());
        let mut message = message();
        for id in ["prior", "attempt"] {
            message.tool_calls.push(
                serde_json::from_value(json!({"id":id,"name":"tool","status":"success"})).unwrap(),
            );
        }
        service.agent_event(
            "conv_bridge",
            &mut message,
            AgentEvent::Rollback {
                text_chars: 2,
                reasoning_chars: 2,
                segment_ids: vec![],
                tool_ids: vec!["attempt".into()],
            },
        );
        assert_eq!(message.content, "前文");
        assert_eq!(message.reasoning.as_deref(), Some("已存想法"));
        assert_eq!(message.tool_calls.len(), 1);
        assert_eq!(message.tool_calls[0].id, "prior");
    }
}
