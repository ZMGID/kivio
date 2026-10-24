//! Lens handoff admission: one stable operation creates one native conversation.
use super::*;
impl KivioService {
    pub(super) async fn lens_handoff(&self, mut params: Value) -> Result<RpcReply, RpcError> {
        let _admission = self.send_gate.lock().await;
        let id = params["chatId"]
            .as_str()
            .ok_or_else(|| failed("Lens chat ID missing"))?
            .to_owned();
        let message_id = id
            .strip_prefix("conv_")
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .ok_or_else(|| failed("Invalid Lens chat ID"))?
            .to_owned();
        let text = params["text"].as_str().unwrap_or_default().to_owned();
        let history: Vec<kivio_model_runtime::settings::ExplainMessage> =
            serde_json::from_value(params["history"].clone()).map_err(failed)?;
        if history.len() > 512
            || history
                .iter()
                .map(|message| message.content.len())
                .sum::<usize>()
                > 16 * 1024 * 1024
        {
            return Err(failed("Lens history exceeds the handoff limit"));
        }
        if (!history.is_empty() && history[0].role != "user")
            || history
                .iter()
                .any(|message| !matches!(message.role.as_str(), "user" | "assistant"))
        {
            return Err(failed("Invalid Lens history roles"));
        }
        let existing = self.store.load_optional(&id).map_err(failed)?;
        if let Some(existing) = existing {
            // A changed question is a new UI operation. Reusing an operation ID with
            // different history/content is a conflict, never an acknowledgement.
            if existing.messages.len() < history.len()
                || history
                    .iter()
                    .zip(&existing.messages)
                    .any(|(expected, actual)| {
                        expected.role != actual.role || expected.content != actual.content
                    })
            {
                return Err(failed("Lens 历史已变更，无法恢复此前的交接"));
            }
            if text.trim().is_empty() && !history.is_empty() {
                return RpcReply::value(&json!({"chat": self.chat(&existing)}));
            }
            if let Some(index) = existing
                .messages
                .iter()
                .position(|message| message.id == message_id)
            {
                let saved = &existing.messages[index];
                if saved.role != "user"
                    || saved.content != text
                    || existing.messages[index + 1..]
                        .iter()
                        .any(|message| message.role == "user")
                {
                    return Err(failed("Lens 问题已变更或已有后续问题，未重复发送"));
                }
                if self.runs.lock().map_err(failed)?.contains_key(&id)
                    || existing.messages[index + 1..]
                        .iter()
                        .any(|message| message.role == "assistant")
                {
                    return RpcReply::value(&json!({"chat": self.chat(&existing)}));
                }
                return self
                    .send_user_admitted(params, true, UserSubmission::Resume(message_id))
                    .await;
            }
            if existing.messages.len() != history.len() {
                return Err(failed("Lens 对话已有其他内容，未覆盖"));
            }
            if !history.is_empty() {
                params["attachments"] = json!([]);
            }
            return self
                .send_user_admitted(params, true, UserSubmission::Append(Some(message_id)))
                .await;
        }
        if history.is_empty() {
            return self
                .send_user_admitted(params, false, UserSubmission::Append(Some(message_id)))
                .await;
        }
        let (provider, model) = crate::providers::load_selection(
            &self.directory.join("conversations/providers.json"),
            None,
        )
        .map_err(failed)?;
        let batch = attachments::prepare(
            self.directory
                .join("conversations")
                .join(format!("{id}_attachments")),
            &params,
        )?;
        let now = chrono::Utc::now().timestamp();
        let mut messages = Vec::new();
        for (index, message) in history.iter().enumerate() {
            messages.push(json!({"id":uuid::Uuid::new_v4().to_string(),"role":message.role,"content":message.content,
                "timestamp":now,"attachments":if index == 0 { serde_json::to_value(batch.attachments()).map_err(failed)? } else { json!([]) }}));
        }
        let conversation: Conversation = serde_json::from_value(json!({"id":id,"title":history[0].content.chars().take(40).collect::<String>(),
            "provider_id":provider.id,"model":model,"messages":messages,"created_at":now,"updated_at":now,"agent_runtime":{"kind":"builtin"}})).map_err(failed)?;
        let conversation = self.store.create(conversation).map_err(failed)?;
        batch.commit();
        self.changes.send_modify(|version| *version += 1);
        self.catalog_changes.send_modify(|version| *version += 1);
        if !text.trim().is_empty() {
            params["attachments"] = json!([]);
            self.send_user_admitted(params, true, UserSubmission::Append(Some(message_id)))
                .await
        } else {
            RpcReply::value(&json!({"chat": self.chat(&conversation)}))
        }
    }
}

/// Typed UI boundary; protocol and attachment encoding stay with the handoff owner.
pub(crate) async fn handoff(
    engine: &crate::state::EngineHandle,
    id: &str,
    question: &str,
    history: &[kivio_model_runtime::settings::ExplainMessage],
    image: &std::path::Path,
) -> Result<zeron_proto::Chat, String> {
    use base64::Engine as _;
    let attachment = crate::attachments::stage_file(image)?;
    let reply = engine.client().call("KivioLensHandoff", json!({
        "chatId":id,"text":question,"runtime":"builtin","history":history,
        "attachments":[{"name":"Lens Screenshot.png","data":base64::engine::general_purpose::STANDARD.encode(attachment.bytes())}]
    })).await.map_err(|error| error.to_string())?;
    serde_json::from_value(reply["chat"].clone())
        .map_err(|error| format!("聊天已接收截图，但返回数据无效：{error}"))
}
