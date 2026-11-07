//! 运行中用户插话（steering）：把用户在生成期间发来的消息注入正在跑的这一轮。
//!
//! 对齐 Codex CLI 的 steer —— 不打断、不丢已做的工具工作，只在**下一个轮次边界**（工具跑完、
//! 下次调模型之前）把用户那句话塞进模型历史，模型带着新指示继续。
//!
//! Consumed input is retained as an ordered content block and in both replay histories.

use super::content::{ContentBlock, ContentKind, ContentPhase};
use super::error::AgentFailure;
use serde::{Deserialize, Serialize};

use super::history::RuntimeMessage;
use super::loop_::{LoopEnv, RunState};
use crate::model::ModelRole;

/// Maximum Unicode character count retained by `SteeringMessage::new`.
pub const MAX_STEER_CHARS: usize = 4000;

/// Queued user input. Its host-supplied ID is preserved in `ContentKind::Input`
/// when the core consumes it; admission alone does not mean it has been consumed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SteeringMessage {
    pub id: String,
    pub text: String,
}

impl SteeringMessage {
    /// 规范化：trim + 截断。文本为空则 None（空插话没有意义，也不该占一张卡）。
    pub fn new(id: String, text: &str) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        let text = if trimmed.chars().count() > MAX_STEER_CHARS {
            trimmed.chars().take(MAX_STEER_CHARS).collect()
        } else {
            trimmed.to_string()
        };
        Some(Self { id, text })
    }
}

/// 轮首注入：**one-at-a-time**（对齐 pi `PendingMessageQueue` 的默认 `QueueMode`）——先把
/// 信箱里新到的插话全部拉进 RunState 本地队列，再只弹**一条**注入本轮。模型逐条应对，
/// 多条排队的插话不会一次灌进同一轮；剩余的由后续轮次边界与 FinalAnswer 边界的
/// [`steering_pending`] 检查保证送达（队列清空前 loop 不收束）。
pub(crate) async fn inject_steering_messages(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<(), AgentFailure> {
    state
        .pending_steering
        .extend(env.host.control.take_steering());
    let Some(message) = state.pending_steering.front().cloned() else {
        return Ok(());
    };
    apply_prompt_hooks(env, state, &message.text, session).await?;
    if !env.host.control.active() {
        return Ok(());
    }
    state.pending_steering.pop_front();
    append_injected_user_turn(
        env,
        state,
        round,
        &message,
        super::content::InputKind::Steer,
    );
    Ok(())
}

/// FinalAnswer 边界的外层检查（对齐 pi agent-loop 的外层 `while`：agent 本要停下时轮询
/// followUp 队列）：模型给出了终答，但信箱/本地队列里还有没送达的插话——run 不该在用户
/// 话没说完时收束。把信箱新到的拉进本地队列后判空；true ⇒ 调用方把终答落成中间消息并
/// 继续循环。
pub(crate) fn steering_pending(env: &LoopEnv<'_>, state: &mut RunState) -> bool {
    state
        .pending_steering
        .extend(env.host.control.take_steering());
    !state.pending_steering.is_empty()
}

/// 终答边界才取 follow-up 信箱。轮首不取，避免把「下一轮再问」做成「下一步就插进工具循环」。
pub(crate) fn follow_up_pending(env: &LoopEnv<'_>, state: &mut RunState) -> bool {
    state
        .pending_follow_up
        .extend(env.host.control.take_follow_up());
    !state.pending_follow_up.is_empty()
}

/// 终答已被吸收后注入**一条** follow-up（one-at-a-time，与 steer 同一纪律）。
pub(crate) async fn inject_follow_up_messages(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<(), AgentFailure> {
    let Some(message) = state.pending_follow_up.front().cloned() else {
        return Ok(());
    };
    apply_prompt_hooks(env, state, &message.text, session).await?;
    if !env.host.control.active() {
        return Ok(());
    }
    state.pending_follow_up.pop_front();
    append_injected_user_turn(
        env,
        state,
        round,
        &message,
        super::content::InputKind::FollowUp,
    );
    Ok(())
}

async fn apply_prompt_hooks(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    prompt: &str,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<(), AgentFailure> {
    let inputs = tokio::select! {
        biased;
        _ = env.host.control.wait_inactive() => return Ok(()),
        inputs = session.input(prompt, env.host.control.clone()) =>
            inputs.map_err(AgentFailure::hook("input"))?,
    };
    if !env.host.control.active() {
        return Ok(());
    }
    state.history.extend(inputs.iter().cloned());
    state.generated_api_messages.extend(inputs);
    Ok(())
}

fn append_injected_user_turn(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    message: &SteeringMessage,
    kind: super::content::InputKind,
) {
    let injected = RuntimeMessage::text(ModelRole::User, message.text.clone());
    state.history.push(injected.clone());
    state.generated_api_messages.push(injected);

    let block = ContentBlock {
        id: state.segment_builder.reserve_order(),
        kind: ContentKind::Input {
            kind,
            message: message.clone(),
        },
        phase: ContentPhase::ToolLoop,
        step_number: None,
        round: Some(round),
        text: None,
        tool_call_id: None,
    };
    env.host.content("", None, Some(&block));
    state.segment_builder.append_existing_segments(vec![block]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_rejects_blank() {
        assert!(SteeringMessage::new("a".into(), "   \n ").is_none());
        let message = SteeringMessage::new("a".into(), "  改用 rg  ").expect("non-blank");
        assert_eq!(message.text, "改用 rg");
        // 截断按字符数（不是字节），CJK 不会被切半个字。
        let long = "字".repeat(MAX_STEER_CHARS + 10);
        let clipped = SteeringMessage::new("b".into(), &long).expect("non-blank");
        assert_eq!(clipped.text.chars().count(), MAX_STEER_CHARS);
    }
}
