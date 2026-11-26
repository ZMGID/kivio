use crate::chat::ask_user::{normalize_prompt, AskUserPromptPayload};
use crate::external_agents::types::UnifiedAgentEvent;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

// Gemini excludes its built-in ask_user in ACP mode, including a tool with that exact name.
pub(crate) const TOOL: &str = "kivio_ask_user";

// The MCP request does not carry the ACP tool call ID. Its answer is echoed in a later
// session/update, which can arrive out of order when Gemini calls several tools.
#[derive(Default)]
pub(crate) struct QuestionReceipts {
    native_calls: HashSet<String>,
    answers: HashMap<String, Value>,
}

impl QuestionReceipts {
    pub fn deliver(&mut self, answer: Answer) {
        self.answers.insert(answer.id.clone(), answer.card);
        let _ = answer.reply.send(
            json!({
                "phase": answer.response.phase, "answers": answer.response.answers,
                "kivioQuestionId": answer.id,
            })
            .to_string(),
        );
    }

    pub fn observe(&mut self, event: &mut UnifiedAgentEvent) -> Option<(String, Value)> {
        match event {
            UnifiedAgentEvent::ToolUse { id, name, .. } if name.split('(').next() == Some(TOOL) => {
                self.native_calls.insert(id.clone());
                if id.starts_with("mcp_Kivio-server_kivio_ask_user__") {
                    *name = "ask_user".into();
                }
                None
            }
            UnifiedAgentEvent::ToolResult {
                tool_use_id,
                content,
                is_error: false,
            } if self.native_calls.remove(tool_use_id) => {
                let echo: Value = serde_json::from_str(content).ok()?;
                let card = self
                    .answers
                    .remove(echo.get("kivioQuestionId")?.as_str()?)?;
                content.clear();
                Some((tool_use_id.clone(), card))
            }
            _ => None,
        }
    }

    pub fn unclaimed(self) -> impl Iterator<Item = (String, Value)> {
        self.answers.into_iter()
    }
}

pub(crate) struct Answer {
    pub id: String,
    pub card: Value,
    pub response: crate::chat::ask_user::AskUserResponseResult,
    pub reply: oneshot::Sender<String>,
}

pub(crate) struct Question {
    pub prompt: AskUserPromptPayload,
    pub reply: oneshot::Sender<String>,
}

#[derive(Clone, Default)]
pub(crate) struct GeminiQuestions(Arc<Mutex<Option<mpsc::Sender<Question>>>>);

impl GeminiQuestions {
    pub(crate) fn begin_turn(&self) -> mpsc::Receiver<Question> {
        let (tx, rx) = mpsc::channel(8);
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        rx
    }

    pub(super) async fn ask(&self, arguments: Value) -> Result<String, String> {
        let prompt = normalize_prompt(arguments)?;
        let (reply, result) = oneshot::channel();
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .ok_or("Gemini is not awaiting tool results")?
            .try_send(Question { prompt, reply })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Closed(_) => "The Gemini turn ended",
                mpsc::error::TrySendError::Full(_) => "Too many pending Gemini questions",
            })?;
        result.await.map_err(|_| "The Gemini turn ended".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args() -> Value {
        json!({"questions":[{"id":"fruit","prompt":"Choose","options":[
            {"id":"a","label":"Apple"},{"id":"b","label":"Banana"}]}]})
    }

    #[test]
    fn echoed_receipts_join_parallel_questions_to_their_own_native_calls() {
        let mut receipts = QuestionReceipts::default();
        let mut echoes = Vec::new();
        for id in ["first", "second", "interrupted"] {
            let mut start = UnifiedAgentEvent::ToolUse {
                id: format!("native-{id}"),
                name: "kivio_ask_user(questions: [...])".into(),
                input: Value::Null,
            };
            assert!(receipts.observe(&mut start).is_none());
            let (reply, mut rx) = oneshot::channel();
            receipts.deliver(Answer {
                id: id.into(),
                card: json!({"askUser":{"title":id}}),
                response: crate::chat::ask_user::skipped_response(),
                reply,
            });
            echoes.push(rx.try_recv().unwrap());
        }
        let mut unrelated = UnifiedAgentEvent::ToolResult {
            tool_use_id: "some-other-tool".into(),
            content: echoes[1].clone(),
            is_error: false,
        };
        assert!(receipts.observe(&mut unrelated).is_none());
        for (id, echo) in [("second", &echoes[1]), ("first", &echoes[0])] {
            let mut end = UnifiedAgentEvent::ToolResult {
                tool_use_id: format!("native-{id}"),
                content: echo.clone(),
                is_error: false,
            };
            assert_eq!(
                receipts.observe(&mut end),
                Some((format!("native-{id}"), json!({"askUser":{"title":id}})))
            );
            assert!(receipts.observe(&mut end).is_none());
        }
        assert_eq!(
            receipts.unclaimed().map(|(id, _)| id).collect::<Vec<_>>(),
            ["interrupted"]
        );
    }

    #[tokio::test]
    async fn only_the_owning_turn_can_answer_a_gemini_mcp_question() {
        let session = GeminiQuestions::default();
        assert!(session.ask(args()).await.is_err());
        let mut first = session.begin_turn();
        let stale = tokio::spawn({
            let session = session.clone();
            async move { session.ask(args()).await }
        });
        let pending = first.recv().await.unwrap();
        drop(first);
        drop(pending);
        assert!(stale.await.unwrap().is_err());
        assert!(session.ask(args()).await.is_err());
        let mut next = session.begin_turn();
        let current = tokio::spawn({
            let session = session.clone();
            async move { session.ask(args()).await }
        });
        let question = next.recv().await.unwrap();
        assert_eq!(question.prompt.questions[0].options[1].id, "b");
        question.reply.send("banana chosen".into()).unwrap();
        assert_eq!(current.await.unwrap().unwrap(), "banana chosen");
    }
}
