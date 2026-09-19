use std::sync::Mutex;
use std::time::{Duration, Instant};

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

const LEASE_DURATION: Duration = Duration::from_secs(30);

struct Lease {
    owner_id: String,
    expires_at: Instant,
}

struct MailboxEntry {
    request: PendingChatExternalSend,
    lease: Option<Lease>,
}

pub(crate) struct ClaimBatch {
    pub(crate) requests: Vec<PendingChatExternalSend>,
    pub(crate) pending_leased: bool,
}

/// In-process handoff mailbox. Requests stay here until the renderer acknowledges
/// delivery. A renderer release or an expired lease makes unfinished work claimable.
#[derive(Default)]
pub(crate) struct ChatExternalSendMailbox {
    pending: Mutex<Vec<MailboxEntry>>,
}

impl ChatExternalSendMailbox {
    pub(crate) fn enqueue(&self, request: PendingChatExternalSend) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(MailboxEntry {
                request,
                lease: None,
            });
    }

    pub(crate) fn rollback(&self, request_id: &str) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let before = pending.len();
        pending.retain(|entry| entry.request.id != request_id);
        pending.len() != before
    }

    pub(crate) fn claim_all(&self, owner_id: &str) -> ClaimBatch {
        self.claim_all_at(owner_id, Instant::now())
    }

    fn claim_all_at(&self, owner_id: &str, now: Instant) -> ClaimBatch {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut requests = Vec::new();
        let mut pending_leased = false;
        for entry in pending.iter_mut() {
            if entry
                .lease
                .as_ref()
                .is_some_and(|lease| lease.expires_at <= now)
            {
                entry.lease = None;
            }
            if entry.lease.is_none() {
                entry.lease = Some(Lease {
                    owner_id: owner_id.to_owned(),
                    expires_at: now + LEASE_DURATION,
                });
                requests.push(entry.request.clone());
            } else if entry
                .lease
                .as_ref()
                .is_some_and(|lease| lease.owner_id == owner_id)
            {
                // An invoke reply can be lost after the claim commits. Returning the
                // same request to its owner lets it retry; the renderer deduplicates IDs.
                requests.push(entry.request.clone());
            } else if entry
                .lease
                .as_ref()
                .is_some_and(|lease| lease.owner_id != owner_id)
            {
                pending_leased = true;
            }
        }
        ClaimBatch {
            requests,
            pending_leased,
        }
    }

    pub(crate) fn ack(&self, owner_id: &str, request_id: &str) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let before = pending.len();
        pending.retain(|entry| {
            entry.request.id != request_id
                || entry
                    .lease
                    .as_ref()
                    .is_none_or(|lease| lease.owner_id != owner_id)
        });
        pending.len() != before
    }

    pub(crate) fn renew(&self, owner_id: &str) -> usize {
        self.renew_at(owner_id, Instant::now())
    }

    fn renew_at(&self, owner_id: &str, now: Instant) -> usize {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut renewed = 0;
        for entry in pending.iter_mut() {
            if let Some(lease) = entry
                .lease
                .as_mut()
                .filter(|lease| lease.owner_id == owner_id)
            {
                lease.expires_at = now + LEASE_DURATION;
                renewed += 1;
            }
        }
        renewed
    }

    pub(crate) fn release(&self, owner_id: &str) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for entry in pending.iter_mut() {
            if entry
                .lease
                .as_ref()
                .is_some_and(|lease| lease.owner_id == owner_id)
            {
                entry.lease = None;
            }
        }
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
    fn claims_are_exclusive_until_acknowledged() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("a"));
        mailbox.enqueue(request("b"));

        let taken = mailbox.claim_all("renderer").requests;
        assert_eq!(
            taken
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(mailbox.claim_all("other").requests.is_empty());
        assert!(mailbox.ack("renderer", "a"));
        assert!(mailbox.ack("renderer", "b"));
        assert!(mailbox.claim_all("other").requests.is_empty());
    }

    #[test]
    fn failed_window_open_rolls_back_only_its_request() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("keep"));
        mailbox.enqueue(request("failed"));

        assert!(mailbox.rollback("failed"));
        assert!(!mailbox.rollback("missing"));
        assert_eq!(mailbox.claim_all("renderer").requests[0].id, "keep");
    }

    #[test]
    fn unfinished_claim_survives_renderer_release_and_new_renderer_can_ack_it() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("handoff"));

        assert_eq!(mailbox.claim_all("old").requests[0].id, "handoff");
        assert!(mailbox.claim_all("new").requests.is_empty());
        mailbox.release("old");
        assert_eq!(mailbox.claim_all("new").requests[0].id, "handoff");
        assert!(!mailbox.ack("old", "handoff"));
        assert!(mailbox.ack("new", "handoff"));
        assert!(mailbox.claim_all("third").requests.is_empty());
    }

    #[test]
    fn late_release_from_old_renderer_does_not_clear_new_renderer_lease() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("handoff"));

        assert_eq!(mailbox.claim_all("old").requests.len(), 1);
        mailbox.release("old");
        assert_eq!(mailbox.claim_all("new").requests.len(), 1);
        mailbox.release("old");
        assert!(mailbox.claim_all("third").requests.is_empty());
        assert!(mailbox.ack("new", "handoff"));
    }

    #[test]
    fn abandoned_claim_recovers_when_release_cannot_be_delivered() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("handoff"));
        let start = Instant::now();

        assert_eq!(mailbox.claim_all_at("closed", start).requests.len(), 1);
        assert!(mailbox
            .claim_all_at(
                "reopened",
                start + LEASE_DURATION - Duration::from_millis(1)
            )
            .requests
            .is_empty());
        assert_eq!(
            mailbox
                .claim_all_at("reopened", start + LEASE_DURATION)
                .requests[0]
                .id,
            "handoff"
        );
    }

    #[test]
    fn active_renderer_renews_lease_during_a_long_send() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("long-run"));
        let start = Instant::now();

        assert_eq!(mailbox.claim_all_at("active", start).requests.len(), 1);
        assert_eq!(
            mailbox.renew_at("active", start + Duration::from_secs(20)),
            1
        );
        assert!(mailbox
            .claim_all_at("other", start + Duration::from_secs(31))
            .requests
            .is_empty());
        assert_eq!(
            mailbox
                .claim_all_at("other", start + Duration::from_secs(50))
                .requests[0]
                .id,
            "long-run"
        );
    }

    #[test]
    fn owner_can_reclaim_after_lost_invoke_reply_without_exposing_to_other_owner() {
        let mailbox = ChatExternalSendMailbox::default();
        mailbox.enqueue(request("lost-reply"));
        let start = Instant::now();

        assert_eq!(
            mailbox.claim_all_at("owner", start).requests[0].id,
            "lost-reply"
        );
        assert_eq!(
            mailbox
                .claim_all_at("owner", start + Duration::from_secs(1))
                .requests[0]
                .id,
            "lost-reply"
        );
        assert!(mailbox
            .claim_all_at("other", start + Duration::from_secs(1))
            .requests
            .is_empty());
        assert!(mailbox.ack("owner", "lost-reply"));
    }
}
