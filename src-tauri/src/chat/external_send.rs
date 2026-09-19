use std::sync::Mutex;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChatExternalAttachment {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChatExternalMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChatExternalSend {
    pub id: String,
    pub content: String,
    pub attachments: Vec<PendingChatExternalAttachment>,
    #[serde(default)]
    pub messages: Vec<PendingChatExternalMessage>,
}

/// One-shot handoff mailbox from external entry points to the Chat renderer.
/// Enqueue happens before opening the window; a failed open rolls back only the
/// matching request. Taking atomically drains the mailbox exactly once.
#[derive(Default)]
pub(crate) struct ChatExternalSendMailbox {
    pending: Mutex<Vec<PendingChatExternalSend>>,
}

impl ChatExternalSendMailbox {
    pub(crate) fn enqueue(&self, request: PendingChatExternalSend) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(request);
    }

    pub(crate) fn rollback(&self, request_id: &str) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let before = pending.len();
        pending.retain(|request| request.id != request_id);
        pending.len() != before
    }

    pub(crate) fn take_all(&self) -> Vec<PendingChatExternalSend> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut *pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(id: &str) -> PendingChatExternalSend {
        PendingChatExternalSend {
            id: id.to_string(),
            content: id.to_string(),
            attachments: Vec::new(),
            messages: Vec::new(),
        }
    }

    #[test]
    fn take_is_atomic_and_one_shot() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("a"));
        mailbox.enqueue(request("b"));

        let taken = mailbox.take_all();
        assert_eq!(
            taken
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(mailbox.take_all().is_empty());
    }

    #[test]
    fn failed_window_open_rolls_back_only_its_request() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("keep"));
        mailbox.enqueue(request("failed"));

        assert!(mailbox.rollback("failed"));
        assert!(!mailbox.rollback("missing"));
        assert_eq!(mailbox.take_all()[0].id, "keep");
    }
}
