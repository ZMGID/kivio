//! Feed the imported Zeron question wizard, retaining one waiter per request.
use super::*;
use tokio::sync::oneshot;
use zeron_proto::{UserInputAnswer, UserInputQuestion};
pub(super) struct Pending {
    pub chat_id: String,
    pub questions: Vec<UserInputQuestion>,
    pub prompt: Option<kivio_model_runtime::ask_user::AskUserPromptPayload>,
    strict_options: bool,
    pub answer: oneshot::Sender<Vec<UserInputAnswer>>,
}
impl KivioService {
    pub(super) fn ask(
        &self,
        chat_id: &str,
        questions: Vec<UserInputQuestion>,
    ) -> oneshot::Receiver<Vec<UserInputAnswer>> {
        self.ask_validated(chat_id, questions, None, false)
    }
    pub(super) fn ask_validated(
        &self,
        chat_id: &str,
        questions: Vec<UserInputQuestion>,
        prompt: Option<kivio_model_runtime::ask_user::AskUserPromptPayload>,
        strict_options: bool,
    ) -> oneshot::Receiver<Vec<UserInputAnswer>> {
        let (answer, receiver) = oneshot::channel();
        let id = uuid::Uuid::new_v4().to_string();
        self.inputs.lock().unwrap().insert(
            id,
            Pending {
                chat_id: chat_id.into(),
                questions,
                prompt,
                strict_options,
                answer,
            },
        );
        self.changes.send_modify(|n| *n += 1);
        receiver
    }
    pub(super) fn input_parts(&self, chat_id: &str) -> Vec<Value> {
        self.inputs.lock().unwrap().iter().filter(|(_, p)| p.chat_id == chat_id)
            .map(|(id, p)| json!({"kind":"input","id":id,"requestId":id,"questions":p.questions,"resolved":false})).collect()
    }
    pub(super) fn respond_input(&self, params: Value) -> Result<RpcReply, RpcError> {
        let chat_id = params["chatId"]
            .as_str()
            .ok_or_else(|| failed("chatId missing"))?;
        let command = &params["command"];
        if command["kind"] != "respondInput" {
            return Err(failed("此会话命令未接入"));
        }
        let request_id = command["requestId"]
            .as_str()
            .ok_or_else(|| failed("requestId missing"))?;
        let answers: Vec<UserInputAnswer> =
            serde_json::from_value(command["answers"].clone()).map_err(failed)?;
        let mut inputs = self.inputs.lock().map_err(failed)?;
        let pending = inputs
            .get(request_id)
            .filter(|p| p.chat_id == chat_id)
            .ok_or_else(|| failed("问题已结束或不属于此对话"))?;
        if answers
            .iter()
            .any(|a| !pending.questions.iter().any(|q| q.id == a.question_id))
        {
            return Err(failed("回答包含未知问题"));
        }
        let mut seen = std::collections::HashSet::new();
        if answers
            .iter()
            .any(|answer| !seen.insert(&answer.question_id))
        {
            return Err(failed("同一问题只能提交一次回答"));
        }
        if pending.strict_options && !answers.is_empty() {
            if answers.len() != pending.questions.len()
                || answers.iter().any(|answer| {
                    let question = pending
                        .questions
                        .iter()
                        .find(|question| question.id == answer.question_id)
                        .unwrap();
                    answer.labels.is_empty()
                        || (!question.multi_select && answer.labels.len() != 1)
                        || answer
                            .labels
                            .iter()
                            .any(|label| !question.options.contains(label))
                })
            {
                return Err(failed("请选择有效的授权选项"));
            }
        }
        if let Some(prompt) = &pending.prompt {
            super::agent::question_response(prompt, &answers).map_err(failed)?;
        }
        let pending = inputs.remove(request_id).unwrap();
        let delivered = pending.answer.send(answers);
        drop(inputs);
        self.changes.send_modify(|n| *n += 1);
        delivered.map_err(|_| failed("此任务已不再等待回答"))?;
        RpcReply::value(&json!({}))
    }
}

impl KivioService {
    pub(super) async fn run_codex(
        &self,
        conversation: &Conversation,
        launch: kivio_model_runtime::external_agents::native::Launch,
        events: tokio::sync::mpsc::Sender<
            kivio_model_runtime::external_agents::types::UnifiedAgentEvent,
        >,
        cancel: oneshot::Receiver<()>,
    ) -> Result<(), String> {
        use kivio_model_runtime::external_agents::session::live::{
            ApprovalBridge, ApprovalDecision,
        };
        let (asks, mut requests) = tokio::sync::mpsc::channel(32);
        let (decisions, answers) = tokio::sync::mpsc::channel(32);
        let call = kivio_model_runtime::external_agents::native::run(
            &self.directory,
            conversation,
            launch,
            events,
            cancel,
            Some(ApprovalBridge {
                requests: asks,
                decisions: answers,
            }),
        );
        tokio::pin!(call);
        let mut pending: futures::stream::FuturesUnordered<
            std::pin::Pin<Box<dyn std::future::Future<Output = ApprovalDecision> + Send>>,
        > = Default::default();
        loop {
            tokio::select! {
                result = &mut call => return result,
                Some(ask) = requests.recv() => {
                    let questions = if ask.requires_user_interaction {
                        ask.input["questions"].as_array().map(|items| items.iter().map(|q| UserInputQuestion {
                            id:q["id"].as_str().unwrap_or("question").into(),
                            header:q["header"].as_str().unwrap_or("问题").into(),
                            question:q["question"].as_str().unwrap_or("").into(),
                            options:q["options"].as_array().map(|options| options.iter().filter_map(|o| o.as_str().or(o["label"].as_str()).map(str::to_owned)).collect()).unwrap_or_default(),
                            multi_select:q["multiSelect"].as_bool().unwrap_or(false),prefill:None,multiline:false,
                        }).collect()).unwrap_or_default()
                    } else { vec![UserInputQuestion { id:"permission".into(),header:ask.tool_name.clone(),
                        question:format!("允许本次操作？\n{}",serde_json::to_string_pretty(&ask.input).unwrap_or_default()),
                        options:vec!["拒绝".into(),"允许本次".into()],multi_select:false,prefill:None,multiline:false }] };
                    if questions.is_empty() {
                        let _ = decisions.send(ApprovalDecision { request_id:ask.request_id,approved:false,updated_input:None,set_permission_mode:None,updated_permissions:None }).await;
                        continue;
                    }
                    let receiver = self.ask_validated(&conversation.id, questions, None, !ask.requires_user_interaction);
                    pending.push(Box::pin(async move {
                        let answers = receiver.await.unwrap_or_default();
                        let approved = if ask.requires_user_interaction { !answers.is_empty() } else { answers.iter().any(|a| a.question_id == "permission" && a.labels == ["允许本次"]) };
                        let updated_input = ask.requires_user_interaction.then(|| json!({"answers":answers.iter().map(|a| (a.question_id.clone(),json!({"answers":a.labels}))).collect::<serde_json::Map<String,Value>>()}));
                        ApprovalDecision { request_id:ask.request_id,approved,updated_input,set_permission_mode:None,updated_permissions:None }
                    }));
                }
                Some(answer) = pending.next(), if !pending.is_empty() => { let _ = decisions.send(answer).await; }
            }
        }
    }
}
