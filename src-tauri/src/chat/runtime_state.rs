use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use super::agent::SteeringMessage;

/// Process-local ownership for Chat run coordination.
///
/// The composition root deliberately exposes behavior, not these indexes.  In
/// particular, a conversation can own several generations/reply slots at once;
/// ending one run must never retire its siblings.
#[derive(Default)]
pub(crate) struct ChatRuntimeState {
    next_generation: AtomicU64,
    runs: Mutex<ChatRunIndexes>,
    popout_create_lock: tokio::sync::Mutex<()>,
    conversation_create_lock: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct ChatRunIndexes {
    active_generations: HashMap<String, HashSet<u64>>,
    active_replies: HashMap<String, HashSet<String>>,
    pending_steering: HashMap<String, Vec<SteeringMessage>>,
    pending_follow_up: HashMap<String, Vec<SteeringMessage>>,
    pending_goal_user_queue: HashSet<String>,
}

impl ChatRuntimeState {
    pub(crate) fn begin_generation(&self, conversation_id: &str) -> u64 {
        let generation = self.next_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.indexes()
            .active_generations
            .entry(conversation_id.to_string())
            .or_default()
            .insert(generation);
        generation
    }

    pub(crate) fn cancel_conversation(&self, conversation_id: &str) {
        let mut indexes = self.indexes();
        indexes.active_generations.remove(conversation_id);
        indexes.pending_steering.remove(conversation_id);
        indexes.pending_follow_up.remove(conversation_id);
    }

    pub(crate) fn end_generation(&self, conversation_id: &str, generation: u64) {
        let mut indexes = self.indexes();
        retire_generation(&mut indexes, conversation_id, generation);
    }

    /// Reply slot and generation are retired as one operation. Releasing the
    /// reply slot first would let a new send begin between two independent
    /// locks, leaving old pending input attached to the new run.
    pub(crate) fn finish_reply_generation(
        &self,
        conversation_id: &str,
        run_id: &str,
        generation: u64,
    ) {
        let mut indexes = self.indexes();
        retire_generation(&mut indexes, conversation_id, generation);
        retire_reply(&mut indexes, conversation_id, run_id);
    }

    pub(crate) fn is_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
        self.indexes()
            .active_generations
            .get(conversation_id)
            .is_some_and(|active| active.contains(&generation))
    }

    pub(crate) fn has_active_generation(&self, conversation_id: &str) -> bool {
        self.indexes()
            .active_generations
            .get(conversation_id)
            .is_some_and(|active| !active.is_empty())
    }

    pub(crate) fn push_steering(&self, conversation_id: &str, message: SteeringMessage) -> bool {
        let mut indexes = self.indexes();
        if !has_active_generation(&indexes, conversation_id) {
            return false;
        }
        indexes
            .pending_steering
            .entry(conversation_id.to_string())
            .or_default()
            .push(message);
        true
    }

    pub(crate) fn take_steering(&self, conversation_id: &str) -> Vec<SteeringMessage> {
        self.indexes()
            .pending_steering
            .remove(conversation_id)
            .unwrap_or_default()
    }

    pub(crate) fn push_follow_up(&self, conversation_id: &str, message: SteeringMessage) -> bool {
        let mut indexes = self.indexes();
        if !has_active_generation(&indexes, conversation_id) {
            return false;
        }
        indexes
            .pending_follow_up
            .entry(conversation_id.to_string())
            .or_default()
            .push(message);
        true
    }

    pub(crate) fn take_follow_up(&self, conversation_id: &str) -> Vec<SteeringMessage> {
        self.indexes()
            .pending_follow_up
            .remove(conversation_id)
            .unwrap_or_default()
    }

    pub(crate) fn has_pending_input(&self, conversation_id: &str) -> bool {
        let indexes = self.indexes();
        indexes
            .pending_steering
            .get(conversation_id)
            .is_some_and(|messages| !messages.is_empty())
            || indexes
                .pending_follow_up
                .get(conversation_id)
                .is_some_and(|messages| !messages.is_empty())
    }

    pub(crate) fn set_goal_user_queue_pending(&self, conversation_id: &str, pending: bool) {
        let mut indexes = self.indexes();
        if pending {
            indexes
                .pending_goal_user_queue
                .insert(conversation_id.to_string());
        } else {
            indexes.pending_goal_user_queue.remove(conversation_id);
        }
    }

    pub(crate) fn has_goal_user_queue_pending(&self, conversation_id: &str) -> bool {
        self.indexes()
            .pending_goal_user_queue
            .contains(conversation_id)
    }

    pub(crate) fn forget_conversation(&self, conversation_id: &str) {
        let mut indexes = self.indexes();
        indexes.active_generations.remove(conversation_id);
        indexes.active_replies.remove(conversation_id);
        indexes.pending_steering.remove(conversation_id);
        indexes.pending_follow_up.remove(conversation_id);
        indexes.pending_goal_user_queue.remove(conversation_id);
    }

    pub(crate) fn try_begin_reply(&self, conversation_id: &str, run_id: &str) -> bool {
        let mut indexes = self.indexes();
        let runs = indexes
            .active_replies
            .entry(conversation_id.to_string())
            .or_default();
        runs.insert(run_id.to_string())
    }

    pub(crate) fn try_reserve_send(&self, conversation_id: &str, run_id: &str) -> bool {
        let mut indexes = self.indexes();
        let runs = indexes
            .active_replies
            .entry(conversation_id.to_string())
            .or_default();
        if !runs.is_empty() {
            return false;
        }
        runs.insert(run_id.to_string());
        true
    }

    pub(crate) fn has_active_reply(&self, conversation_id: &str) -> bool {
        self.indexes()
            .active_replies
            .get(conversation_id)
            .is_some_and(|runs| !runs.is_empty())
    }

    pub(crate) fn end_reply(&self, conversation_id: &str, run_id: &str) {
        let mut indexes = self.indexes();
        retire_reply(&mut indexes, conversation_id, run_id);
    }

    pub(crate) async fn lock_popout_creation(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.popout_create_lock.lock().await
    }

    pub(crate) async fn lock_conversation_creation(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.conversation_create_lock.lock().await
    }

    fn indexes(&self) -> std::sync::MutexGuard<'_, ChatRunIndexes> {
        self.runs.lock().unwrap_or_else(|error| error.into_inner())
    }
}

fn has_active_generation(indexes: &ChatRunIndexes, conversation_id: &str) -> bool {
    indexes
        .active_generations
        .get(conversation_id)
        .is_some_and(|active| !active.is_empty())
}

fn retire_generation(indexes: &mut ChatRunIndexes, conversation_id: &str, generation: u64) {
    if let Some(active) = indexes.active_generations.get_mut(conversation_id) {
        active.remove(&generation);
        if active.is_empty() {
            indexes.active_generations.remove(conversation_id);
            indexes.pending_steering.remove(conversation_id);
            indexes.pending_follow_up.remove(conversation_id);
        }
    }
}

fn retire_reply(indexes: &mut ChatRunIndexes, conversation_id: &str, run_id: &str) {
    if let Some(runs) = indexes.active_replies.get_mut(conversation_id) {
        runs.remove(run_id);
        if runs.is_empty() {
            indexes.active_replies.remove(conversation_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_runs_end_independently_and_cancel_is_conversation_scoped() {
        let runtime = ChatRuntimeState::default();
        let first = runtime.begin_generation("a");
        let sibling = runtime.begin_generation("a");
        let other = runtime.begin_generation("b");

        runtime.end_generation("a", first);
        assert!(!runtime.is_generation_active("a", first));
        assert!(runtime.is_generation_active("a", sibling));
        assert!(runtime.is_generation_active("b", other));

        runtime.cancel_conversation("a");
        assert!(!runtime.is_generation_active("a", sibling));
        assert!(runtime.is_generation_active("b", other));
    }

    #[test]
    fn reply_slots_are_per_run_but_send_reservation_is_per_conversation() {
        let runtime = ChatRuntimeState::default();
        assert!(runtime.try_begin_reply("a", "run-1"));
        assert!(runtime.try_begin_reply("a", "run-2"));
        assert!(!runtime.try_begin_reply("a", "run-1"));
        assert!(!runtime.try_reserve_send("a", "send"));

        runtime.end_reply("a", "run-1");
        assert!(runtime.has_active_reply("a"));
        runtime.end_reply("a", "run-2");
        assert!(runtime.try_reserve_send("a", "send"));
    }

    #[test]
    fn forget_clears_only_one_conversation_runtime() {
        let runtime = ChatRuntimeState::default();
        let a = runtime.begin_generation("a");
        let b = runtime.begin_generation("b");
        assert!(runtime.try_begin_reply("a", "run-a"));
        runtime.set_goal_user_queue_pending("a", true);

        runtime.forget_conversation("a");

        assert!(!runtime.is_generation_active("a", a));
        assert!(!runtime.has_active_reply("a"));
        assert!(!runtime.has_goal_user_queue_pending("a"));
        assert!(runtime.is_generation_active("b", b));
    }

    #[test]
    fn cancelled_mailbox_input_cannot_leak_into_next_run() {
        let runtime = ChatRuntimeState::default();
        let old = runtime.begin_generation("conversation");
        assert!(runtime.push_steering(
            "conversation",
            SteeringMessage {
                id: "old".into(),
                text: "old".into(),
            },
        ));
        runtime.cancel_conversation("conversation");
        assert!(!runtime.is_generation_active("conversation", old));
        assert!(runtime.take_steering("conversation").is_empty());
        assert!(!runtime.push_steering(
            "conversation",
            SteeringMessage {
                id: "late".into(),
                text: "late".into(),
            },
        ));
        runtime.begin_generation("conversation");
        assert!(runtime.take_steering("conversation").is_empty());
    }

    #[test]
    fn last_run_finish_clears_pending_but_sibling_finish_preserves_it() {
        let runtime = ChatRuntimeState::default();
        let first = runtime.begin_generation("conversation");
        let sibling = runtime.begin_generation("conversation");
        assert!(runtime.push_follow_up(
            "conversation",
            SteeringMessage {
                id: "pending".into(),
                text: "continue".into(),
            },
        ));
        runtime.end_generation("conversation", first);
        assert!(runtime.has_pending_input("conversation"));
        runtime.end_generation("conversation", sibling);
        assert!(!runtime.has_pending_input("conversation"));
    }

    #[test]
    fn finishing_reply_retires_generation_and_slot_atomically() {
        let runtime = ChatRuntimeState::default();
        let generation = runtime.begin_generation("conversation");
        assert!(runtime.try_begin_reply("conversation", "run"));
        assert!(runtime.push_steering(
            "conversation",
            SteeringMessage {
                id: "old".into(),
                text: "old".into(),
            },
        ));

        runtime.finish_reply_generation("conversation", "run", generation);

        assert!(!runtime.is_generation_active("conversation", generation));
        assert!(!runtime.has_active_reply("conversation"));
        assert!(!runtime.has_pending_input("conversation"));
        assert!(runtime.try_reserve_send("conversation", "next"));
    }
}
