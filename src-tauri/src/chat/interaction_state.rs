use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard},
};

use serde_json::Value;
use tokio::sync::{oneshot, Mutex as AsyncMutex, MutexGuard as AsyncMutexGuard};

use super::ask_user::{
    take_validated_response, AskUserPromptPayload, AskUserResponseResult, PendingAskUserPrompt,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ToolApprovalOutcome {
    pub approved: bool,
    pub permission_mode: Option<String>,
}

#[derive(Debug)]
struct PendingSessionConsent {
    run_id: String,
    sender: oneshot::Sender<bool>,
}

#[derive(Debug)]
struct PendingToolApproval {
    conversation_id: String,
    tool_name: String,
    sender: oneshot::Sender<ToolApprovalOutcome>,
}

/// Owns the transient user-interaction state for Chat. Maps stay private so callers cannot split
/// validate/take or insert/take transitions across unrelated locks.
#[derive(Default)]
pub(crate) struct ChatInteractionState {
    pending_tool_approvals: Mutex<HashMap<String, PendingToolApproval>>,
    tool_always_allow: Mutex<HashSet<(String, String)>>,
    session_consent: Mutex<HashSet<String>>,
    pending_session_consents: Mutex<HashMap<String, PendingSessionConsent>>,
    consent_prompt_lock: AsyncMutex<()>,
    pending_user_prompts: Mutex<HashMap<String, PendingAskUserPrompt>>,
    answered_ask_user_content: Mutex<HashMap<String, Value>>,
}

impl ChatInteractionState {
    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn has_session_consent(&self, conversation_id: &str) -> bool {
        Self::lock(&self.session_consent).contains(conversation_id)
    }

    pub(crate) fn grant_session_consent(&self, conversation_id: &str) {
        Self::lock(&self.session_consent).insert(conversation_id.to_string());
    }

    pub(crate) fn has_tool_always_allow(&self, conversation_id: &str, tool_name: &str) -> bool {
        Self::lock(&self.tool_always_allow)
            .contains(&(conversation_id.to_string(), tool_name.to_ascii_lowercase()))
    }

    pub(crate) fn grant_tool_always_allow(&self, conversation_id: &str, tool_name: &str) {
        Self::lock(&self.tool_always_allow)
            .insert((conversation_id.to_string(), tool_name.to_ascii_lowercase()));
    }

    pub(crate) async fn lock_consent_prompt(&self) -> AsyncMutexGuard<'_, ()> {
        self.consent_prompt_lock.lock().await
    }

    pub(crate) fn begin_session_consent(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> oneshot::Receiver<bool> {
        let (sender, receiver) = oneshot::channel();
        Self::lock(&self.pending_session_consents).insert(
            conversation_id.to_string(),
            PendingSessionConsent {
                run_id: run_id.to_string(),
                sender,
            },
        );
        receiver
    }

    pub(crate) fn respond_session_consent(
        &self,
        conversation_id: &str,
        granted: bool,
    ) -> Option<String> {
        let pending = Self::lock(&self.pending_session_consents).remove(conversation_id)?;
        let _ = pending.sender.send(granted);
        Some(pending.run_id)
    }

    pub(crate) fn cancel_session_consent(&self, conversation_id: &str) -> Option<String> {
        Self::lock(&self.pending_session_consents)
            .remove(conversation_id)
            .map(|pending| pending.run_id)
    }

    pub(crate) fn begin_tool_approval(
        &self,
        tool_call_id: &str,
        conversation_id: &str,
        tool_name: &str,
    ) -> oneshot::Receiver<ToolApprovalOutcome> {
        let (sender, receiver) = oneshot::channel();
        Self::lock(&self.pending_tool_approvals).insert(
            tool_call_id.to_string(),
            PendingToolApproval {
                conversation_id: conversation_id.to_string(),
                tool_name: tool_name.to_string(),
                sender,
            },
        );
        receiver
    }

    pub(crate) fn respond_tool_approval(
        &self,
        tool_call_id: &str,
        outcome: ToolApprovalOutcome,
        always: bool,
    ) -> bool {
        let Some(pending) = Self::lock(&self.pending_tool_approvals).remove(tool_call_id) else {
            return false;
        };
        if outcome.approved && always {
            self.grant_tool_always_allow(&pending.conversation_id, &pending.tool_name);
        }
        let _ = pending.sender.send(outcome);
        true
    }

    pub(crate) fn cancel_tool_approval(&self, tool_call_id: &str) -> bool {
        Self::lock(&self.pending_tool_approvals)
            .remove(tool_call_id)
            .is_some()
    }

    pub(crate) fn begin_user_prompt(
        &self,
        tool_call_id: &str,
        run_id: &str,
        prompt: AskUserPromptPayload,
    ) -> oneshot::Receiver<AskUserResponseResult> {
        let (sender, receiver) = oneshot::channel();
        Self::lock(&self.pending_user_prompts).insert(
            tool_call_id.to_string(),
            PendingAskUserPrompt {
                run_id: run_id.to_string(),
                prompt,
                sender,
            },
        );
        receiver
    }

    /// Validate and claim a prompt under the same lock. Validation failure leaves it pending;
    /// success sends exactly one response and removes it before any observer can claim it again.
    pub(crate) fn respond_user_prompt(
        &self,
        tool_call_id: &str,
        response: AskUserResponseResult,
    ) -> Result<String, String> {
        let (pending, response) = take_validated_response(
            &mut Self::lock(&self.pending_user_prompts),
            tool_call_id,
            response,
        )?;
        let run_id = pending.run_id;
        let _ = pending.sender.send(response);
        Ok(run_id)
    }

    pub(crate) fn cancel_user_prompt(&self, tool_call_id: &str) -> Option<String> {
        Self::lock(&self.pending_user_prompts)
            .remove(tool_call_id)
            .map(|pending| pending.run_id)
    }

    pub(crate) fn forget_tool_interactions(&self, tool_call_ids: &[String]) {
        {
            let mut approvals = Self::lock(&self.pending_tool_approvals);
            for id in tool_call_ids {
                approvals.remove(id);
            }
        }
        {
            let mut prompts = Self::lock(&self.pending_user_prompts);
            for id in tool_call_ids {
                prompts.remove(id);
            }
        }
    }

    pub(crate) fn remember_answered_ask_user(&self, tool_call_id: String, content: Value) {
        Self::lock(&self.answered_ask_user_content).insert(tool_call_id, content);
    }

    pub(crate) fn take_answered_ask_user(&self, tool_call_id: &str) -> Option<Value> {
        Self::lock(&self.answered_ask_user_content).remove(tool_call_id)
    }

    pub(crate) fn forget_conversation(&self, conversation_id: &str) {
        Self::lock(&self.session_consent).remove(conversation_id);
        Self::lock(&self.tool_always_allow)
            .retain(|(conversation, _)| conversation != conversation_id);
        Self::lock(&self.pending_session_consents).remove(conversation_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ask_user::{
        AskUserAnswer, AskUserOption, AskUserQuestion, ASK_USER_PHASE_ANSWERED,
    };

    fn prompt() -> AskUserPromptPayload {
        AskUserPromptPayload {
            title: None,
            questions: vec![AskUserQuestion {
                id: "choice".into(),
                prompt: "Choose".into(),
                options: vec![
                    AskUserOption {
                        id: "a".into(),
                        label: "A".into(),
                        description: None,
                    },
                    AskUserOption {
                        id: "b".into(),
                        label: "B".into(),
                        description: None,
                    },
                ],
                allow_multiple: false,
                allow_custom: false,
                required: true,
                value_schema: None,
            }],
        }
    }

    #[tokio::test]
    async fn invalid_user_response_retains_pending_then_success_claims_exactly_once() {
        let state = ChatInteractionState::default();
        let receiver = state.begin_user_prompt("tool-1", "run-1", prompt());
        let invalid = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.into(),
            answers: HashMap::new(),
        };
        assert!(state.respond_user_prompt("tool-1", invalid).is_err());

        let valid = AskUserResponseResult {
            phase: ASK_USER_PHASE_ANSWERED.into(),
            answers: HashMap::from([(
                "choice".into(),
                AskUserAnswer {
                    selected_option_ids: vec!["a".into()],
                    custom_text: None,
                },
            )]),
        };
        assert_eq!(
            state.respond_user_prompt("tool-1", valid.clone()).unwrap(),
            "run-1"
        );
        assert_eq!(receiver.await.unwrap(), valid);
        assert!(state.respond_user_prompt("tool-1", valid).is_err());
    }

    #[tokio::test]
    async fn approval_and_consent_are_taken_once() {
        let state = ChatInteractionState::default();
        let approval = state.begin_tool_approval("tool-1", "conv-1", "Write");
        let outcome = ToolApprovalOutcome {
            approved: true,
            permission_mode: Some("default".into()),
        };
        assert!(state.respond_tool_approval("tool-1", outcome.clone(), true));
        assert_eq!(approval.await.unwrap(), outcome);
        assert!(state.has_tool_always_allow("conv-1", "write"));
        assert!(!state.respond_tool_approval("tool-1", ToolApprovalOutcome::default(), false));

        let consent = state.begin_session_consent("conv-1", "run-1");
        assert_eq!(
            state.respond_session_consent("conv-1", true).as_deref(),
            Some("run-1")
        );
        assert!(consent.await.unwrap());
        assert!(state.respond_session_consent("conv-1", false).is_none());
    }
}
