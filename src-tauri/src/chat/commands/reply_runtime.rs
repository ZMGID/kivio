use uuid::Uuid;

use crate::settings::Settings;
use crate::state::AppState;

use super::ChatMessage;

pub(super) const CHAT_REPLY_BUSY_ERROR: &str = "该对话正在生成中，请稍后再试";
/// 多模型一问多答的并排上限（决策 D4）。超过此数不允许发送。
pub(super) const MAX_REPLY_MODELS: usize = 4;

/// 由会话级 `reply_models` 解析出本次发送要 fan-out 的「臂」列表。
/// 返回去重后（按 provider_id+model）、保序的 `(provider_id, model)`。
/// - 0 或 1 个有效臂 → 返回长度 ≤1（调用方走单模型现状路径，行为不变）。
/// - ≥2 个 → 多模型 fan-out。
/// 校验：上限 `MAX_REPLY_MODELS`（超出 `Err`）；provider 必须存在（不存在的臂跳过）；
/// 空 model 跳过。
pub(super) fn resolve_reply_arms(
    settings: &Settings,
    reply_models: &[crate::chat::ModelRef],
) -> Result<Vec<(String, String)>, String> {
    if reply_models.len() > MAX_REPLY_MODELS {
        return Err(format!(
            "多模型并行回答最多同时选择 {MAX_REPLY_MODELS} 个模型（当前 {}）。",
            reply_models.len()
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut arms = Vec::new();
    for model_ref in reply_models {
        let provider_id = model_ref.provider_id.trim();
        let model = model_ref.model.trim();
        if provider_id.is_empty() || model.is_empty() {
            continue;
        }
        if settings.get_provider(provider_id).is_none() {
            continue;
        }
        let key = format!("{provider_id}\u{0}{model}");
        if seen.insert(key) {
            arms.push((provider_id.to_string(), model.to_string()));
        }
    }
    Ok(arms)
}

/// Reserves a conversation for a send; Stop invalidates it until this guard drops.
pub(crate) struct ChatSendReservation<'a> {
    state: &'a AppState,
    conversation_id: String,
    run_id: String,
    interruption_epoch: u64,
}

impl<'a> ChatSendReservation<'a> {
    /// 尝试预留某会话的发送哨兵。返回 None 表示该会话已有 run 在跑（busy）。
    pub(crate) fn try_acquire(state: &'a AppState, conversation_id: &str) -> Option<Self> {
        let run_id = format!("chat-send-reservation-{}", Uuid::new_v4());
        let interruption_epoch = Self::try_reserve(state, conversation_id, &run_id)?;
        Some(Self {
            state,
            conversation_id: conversation_id.to_string(),
            run_id,
            interruption_epoch,
        })
    }

    fn try_reserve(state: &AppState, conversation_id: &str, run_id: &str) -> Option<u64> {
        state.sub_agents.with_interruption_epoch(conversation_id, |epoch| {
            state.chat_runtime().try_reserve_send(conversation_id, run_id).then_some(epoch)
        })
    }

    pub(crate) fn interruption_epoch(&self) -> u64 {
        self.interruption_epoch
    }

    /// Queued variant: waits while the conversation is replying, then reserves.
    pub(super) async fn acquire_when_idle(state: &'a AppState, conversation_id: &str) -> Self {
        let run_id = format!("chat-send-reservation-{}", Uuid::new_v4());
        let interruption_epoch = state.chat_runtime()
            .reserve_send_when_idle(|| Self::try_reserve(state, conversation_id, &run_id))
            .await;
        Self {
            state,
            conversation_id: conversation_id.to_string(),
            run_id,
            interruption_epoch,
        }
    }
}

impl Drop for ChatSendReservation<'_> {
    fn drop(&mut self) {
        self.state
            .chat_runtime()
            .end_reply(&self.conversation_id, &self.run_id);
    }
}

/// RAII 守卫：占住某条 run 的回复槽位与活跃 generation，函数任意退出路径都释放。
/// 同一会话允许多条 run 并存（多模型一问多答），每条 run 各持一个守卫。
pub(super) struct ChatReplyGuard<'a> {
    state: &'a AppState,
    conversation_id: String,
    run_id: String,
    generation: u64,
}

impl<'a> ChatReplyGuard<'a> {
    /// 注册一条 run 的回复槽位。返回 None 表示同一 (conversation_id, run_id) 已在进行中。
    /// `generation` 一并登记，drop 时随槽位一起退役（不影响同会话其它在跑 run）。
    pub(super) fn try_new(
        state: &'a AppState,
        conversation_id: &str,
        run_id: &str,
        generation: u64,
    ) -> Option<Self> {
        if !state
            .chat_runtime()
            .try_begin_reply(conversation_id, run_id)
        {
            return None;
        }
        state.sub_agents.irc.begin_turn(conversation_id, crate::chat::sub_agent::MAIN_AGENT_ID);
        Some(Self {
            state,
            conversation_id: conversation_id.to_string(),
            run_id: run_id.to_string(),
            generation,
        })
    }
}

impl Drop for ChatReplyGuard<'_> {
    fn drop(&mut self) {
        self.state.chat_runtime().finish_reply_generation(
            &self.conversation_id,
            &self.run_id,
            self.generation,
        );
        // ChatRuntimeState retires pending input atomically with the final
        // generation; a sibling run must not lose its pending input here.
    }
}

/// 多模型一问多答（任务 06-30）单条「臂」的覆盖配置。`complete_assistant_reply`
/// 收到 `Some(arm)` 时：用该臂自己的 provider/model（而非会话级），把 `group_id`/
/// provider/model 写进 assistant 消息，**自动批准工具**（避免 N 个并发 run 各弹一次审批），
/// 并且 **不直接落盘**——产出的 assistant `ChatMessage` 由协调者（`chat_send_message`）回收后
/// 统一 upsert + 一次性 save，避开 N 条并发 run 同写一个 `conversations/{id}.json` 的竞态。
/// 单模型路径传 `None`，行为与改造前完全一致。
pub(super) struct ReplyArm {
    pub(super) group_id: String,
    pub(super) group_size: usize,
    pub(super) arm_index: usize,
    pub(super) provider_id: String,
    pub(super) model: String,
}

/// 多模型臂运行后回收的结果。协调者据此把 assistant 消息合并进真正的会话并落盘。
/// 单模型路径（`arm = None`）`message` 为 None（已在函数内自行落盘）。
pub(super) struct ArmReplyOutcome {
    pub(super) message: Option<ChatMessage>,
    pub(super) run_id: Option<String>,
    pub(super) error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::interaction::{cancel_user_generation, UserStopScope};

    #[test]
    fn stop_invalidates_reserved_turn_before_or_after_generation_admission() {
        let state = crate::state::test_app_state();
        let first = ChatSendReservation::try_acquire(&state, "conversation").unwrap();
        let epoch = first.interruption_epoch();
        cancel_user_generation(&state, "conversation", UserStopScope::Generation, || ());
        assert!(!state.sub_agents.resume_user_turn("conversation", epoch));
        assert!(state.sub_agents.user_interrupted("conversation"));
        assert!(state.chat_runtime().begin_reply_generation("conversation").is_none());
        drop(first);

        let second = ChatSendReservation::try_acquire(&state, "conversation").unwrap();
        assert!(state.sub_agents.resume_user_turn("conversation", second.interruption_epoch()));
        let generation = state.chat_runtime().begin_reply_generation("conversation").unwrap();
        cancel_user_generation(&state, "conversation", UserStopScope::Generation, || ());
        assert!(!state.chat_runtime().is_generation_active("conversation", generation));
        assert!(state.chat_runtime().begin_reply_generation("conversation").is_none());
        assert!(state.sub_agents.user_interrupted("conversation"));
        drop(second);

        let third = ChatSendReservation::try_acquire(&state, "conversation").unwrap();
        assert!(state.sub_agents.resume_user_turn("conversation", third.interruption_epoch()));
        assert!(state.chat_runtime().begin_reply_generation("conversation").is_some());
    }

    #[test]
    fn stopping_background_wait_invalidates_parent_admission_and_keeps_the_child_result() {
        use crate::chat::sub_agent::{jobs::JobStatus, MAIN_AGENT_ID};

        let state = crate::state::test_app_state();
        let root = "conversation";
        let (job, _) = state.sub_agents.jobs.register(root, MAIN_AGENT_ID, "Scout", "look up the answer");
        let reservation = ChatSendReservation::try_acquire(&state, root).unwrap();

        cancel_user_generation(&state, root, UserStopScope::BackgroundWait, || ());
        assert!(state.sub_agents.user_interrupted(root));
        assert!(state.chat_runtime().begin_reply_generation(root).is_none());
        assert_eq!(state.sub_agents.jobs.status(&job), Some(JobStatus::Running));
        state.sub_agents.jobs.settle(&job, Ok("child result".into()));
        assert_eq!(state.sub_agents.jobs.undelivered(root, MAIN_AGENT_ID)[0].result.as_deref(), Some("child result"));

        drop(reservation);
        let next = ChatSendReservation::try_acquire(&state, root).unwrap();
        assert!(state.sub_agents.resume_user_turn(root, next.interruption_epoch()));
        assert!(state.chat_runtime().begin_reply_generation(root).is_some());
    }

    #[tokio::test]
    async fn queued_turn_captures_epoch_when_it_acquires_the_reservation() {
        let state = crate::state::test_app_state();
        let first = ChatSendReservation::try_acquire(&state, "conversation").unwrap();
        let waiting = ChatSendReservation::acquire_when_idle(&state, "conversation");
        tokio::pin!(waiting);
        assert!(futures::poll!(waiting.as_mut()).is_pending());
        cancel_user_generation(&state, "conversation", UserStopScope::Generation, || ());
        drop(first);
        let next = waiting.await;
        assert!(state.sub_agents.resume_user_turn("conversation", next.interruption_epoch()));
        assert!(state.chat_runtime().begin_reply_generation("conversation").is_some());
    }
}
