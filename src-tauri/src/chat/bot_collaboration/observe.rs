//! Turns a finished native reply into a collaboration outcome.
//! The text is the assistant message itself. A user message is not rewritten
//! into a fake assistant reply.

use std::borrow::Cow;

use crate::chat::types::{ChatMessage, Conversation};

use super::types::{Actor, Delivery, DeliveryKind, PeerMessage, RunFinish, NO_FABRICATED_REPLY};

pub fn classify_native_reply(
    role: &str,
    content: &str,
    stream_outcome: Option<&str>,
    coordination_tool_succeeded: bool,
) -> RunFinish {
    if role != "assistant" {
        return RunFinish::Failed {
            message: format!("运行结束但最后一条不是助手消息，{NO_FABRICATED_REPLY}"),
        };
    }
    let text = content.trim();
    match stream_outcome {
        // A coordination tool already spent this exchange's hop. Empty prose is
        // still a completed run; it must not be rewritten into a second reply.
        Some("completed" | "recovered") if !text.is_empty() || coordination_tool_succeeded => {
            RunFinish::Succeeded {
                native_output: text.to_string(),
                model_enqueued: coordination_tool_succeeded,
            }
        }
        Some("completed" | "recovered") => RunFinish::Failed {
            message: format!("助手没有产生可投递的正文，{NO_FABRICATED_REPLY}"),
        },
        Some("cancelled" | "interrupted") => RunFinish::Cancelled {
            message: "运行已取消，未转发".into(),
        },
        Some(other) => RunFinish::Failed {
            message: format!("运行失败（{other}），{NO_FABRICATED_REPLY}"),
        },
        None => RunFinish::Failed {
            message: format!("运行没有完成状态，{NO_FABRICATED_REPLY}"),
        },
    }
}

/// Single producer for the provider envelope. Current-turn overrides and
/// history replay both call this with the clean body they are actually sending.
pub fn peer_envelope(peer: &PeerMessage, body: &str) -> String {
    let from = match &peer.from {
        Actor::User => "用户".to_string(),
        Actor::Bot { id, name, .. } => format!("{name}（{id}）"),
    };
    let room = peer.room_id.as_deref().unwrap_or("直接");
    let kind = match peer.kind {
        DeliveryKind::Peer => "同伴",
        DeliveryKind::Mention => "点名",
        DeliveryKind::Reply => "回复",
        DeliveryKind::Handoff => "交接",
        DeliveryKind::Broadcast => "广播",
    };
    format!(
        "[kivio-peer {id}]\n发送方：{from}\n接收方：{to_name}（{to_bot}）\n房间：{room}\n类型：{kind}\n交流：{exchange}\n\n{body}\n\n用你自己的助手身份处理。不要代替对方再写一条回复。",
        id = peer.delivery_id,
        to_name = peer.to_name,
        to_bot = peer.to_bot_id,
        exchange = peer.exchange_id,
        body = body,
    )
}

pub fn peer_user_message(delivery: &Delivery) -> String {
    peer_envelope(&PeerMessage::from(delivery), &delivery.body)
}

/// Exact envelope produced for this recipient. A shared prefix is not enough,
/// and a delivery addressed to another conversation never matches.
pub fn trusted_legacy_delivery<'a>(
    conversation_id: &str,
    content: &str,
    deliveries: &'a [Delivery],
) -> Option<&'a Delivery> {
    if !content.starts_with("[kivio-peer ") {
        return None;
    }
    let mut found = None;
    for delivery in deliveries {
        if delivery.to_conversation_id != conversation_id {
            continue;
        }
        if peer_user_message(delivery) != content {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(delivery);
    }
    found
}

/// A colleague authored this user row. Provenance already on the message counts.
/// A legacy row counts only when one durable delivery for this conversation
/// reproduces the stored text exactly.
pub fn peer_authored_content(
    conversation_id: &str,
    message: &ChatMessage,
    deliveries: &[Delivery],
) -> bool {
    message.role == "user"
        && (message.peer.is_some()
            || trusted_legacy_delivery(conversation_id, &message.content, deliveries).is_some())
}

/// Read-time view of legacy rows whose stored text is still the original
/// envelope. Callers must not write the projected conversation back on refresh.
pub fn project_trusted_legacy_peers(conversation: &mut Conversation, deliveries: &[Delivery]) {
    for message in &mut conversation.messages {
        if message.role != "user" || message.peer.is_some() {
            continue;
        }
        let Some(delivery) =
            trusted_legacy_delivery(&conversation.id, &message.content, deliveries)
        else {
            continue;
        };
        message.peer = Some(PeerMessage::from(delivery));
        message.content = delivery.body.clone();
    }
}

/// Body to compose with attachments. Stored peer rows are already clean.
/// Legacy rows yield the delivery body only on an exact envelope match.
pub fn clean_peer_body<'a>(
    conversation_id: &str,
    message: &'a ChatMessage,
    deliveries: &[Delivery],
) -> Cow<'a, str> {
    if message.role != "user" || message.peer.is_some() {
        return Cow::Borrowed(message.content.as_str());
    }
    trusted_legacy_delivery(conversation_id, &message.content, deliveries)
        .map(|delivery| Cow::Owned(delivery.body.clone()))
        .unwrap_or(Cow::Borrowed(message.content.as_str()))
}

/// Provider text for one user message. The latest-user override, when present,
/// is the attachment-composed clean body and is wrapped once. It is not applied
/// again on later replay.
pub fn provider_user_text<'a>(
    conversation_id: &str,
    message: &'a ChatMessage,
    last_user_override: Option<&'a str>,
    deliveries: &'a [Delivery],
) -> Cow<'a, str> {
    if message.role != "user" {
        return Cow::Borrowed(last_user_override.unwrap_or(&message.content));
    }
    if let Some(peer) = &message.peer {
        return Cow::Owned(peer_envelope(
            peer,
            last_user_override.unwrap_or(&message.content),
        ));
    }
    if let Some(delivery) = trusted_legacy_delivery(conversation_id, &message.content, deliveries) {
        return Cow::Owned(peer_envelope(
            &PeerMessage::from(delivery),
            last_user_override.unwrap_or(&delivery.body),
        ));
    }
    Cow::Borrowed(last_user_override.unwrap_or(&message.content))
}
