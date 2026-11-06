//! 运行中用户插话（steering）：把用户在生成期间发来的消息注入正在跑的这一轮。
//!
//! 对齐 Codex CLI 的 steer —— 不打断、不丢已做的工具工作，只在**下一个轮次边界**（工具跑完、
//! 下次调模型之前）把用户那句话塞进模型历史，模型带着新指示继续。
//!
//! Consumed input is retained as an ordered content block and in both replay histories.

use serde::{Deserialize, Serialize};
use super::content::{ContentBlock, ContentKind, ContentPhase};



use super::history::RuntimeMessage;
use super::loop_::{LoopEnv, RunState};
use crate::chat::model::{GenerateOutput, ModelRole};

/// 单条插话文本上限。与 `ask_user.rs` 那批 `MAX_*_CHARS` 同一取舍：越界截断而不是报错，
/// 用户已经打完的字不该因为长度被整条丢掉。
pub const MAX_STEER_CHARS: usize = 4000;

/// 一条待注入的用户插话。`id` 由前端生成并原样回到卡片的 `structured_content.steer_id`，
/// 前端据此把队列里那条标记为「已生效」并出队。
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
///
/// 两处 push 都是必须的：`runtime_messages` 让**本轮**模型看见，`generated_api_messages` 让它
/// 随 assistant 消息落盘（`model_messages_from_openai_messages` 认 `"user"` role），**下一轮回放
/// 不丢**。工具结果之后紧跟一条 user 文本对三种线格式都安全：Anthropic / Gemini 适配器各有
/// `merge_consecutive_*_roles` 把它合进同一个 turn，OpenAI 系天然接受连续 user。
pub(crate) async fn inject_steering_messages(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<(), String> {
    state
        .pending_steering
        .extend(env.host.control.take_steering());
    let Some(message) = state.pending_steering.pop_front() else {
        return Ok(());
    };
    apply_prompt_hooks(env, state, &message.text, session).await?;
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
    state.pending_follow_up.extend(
        env.host
            .control.take_follow_up(),
    );
    !state.pending_follow_up.is_empty()
}

/// 终答已被吸收后注入**一条** follow-up（one-at-a-time，与 steer 同一纪律）。
pub(crate) async fn inject_follow_up_messages(
    env: &LoopEnv<'_>,
    state: &mut RunState,
    round: u32,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<(), String> {
    let Some(message) = state.pending_follow_up.pop_front() else {
        return Ok(());
    };
    apply_prompt_hooks(env, state, &message.text, session).await?;
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
    env: &LoopEnv<'_>, state: &mut RunState, prompt: &str,
    session: &mut dyn super::session::SessionPolicy,
) -> Result<(), String> {
    let inputs = session.input(prompt, env.host.control.clone()).await?;
    state.runtime_messages.extend(inputs.iter().cloned());
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
    state.runtime_messages.push(injected.clone());
    state.generated_api_messages.push(injected);

    let block = ContentBlock {
        id: state.segment_builder.reserve_order(),
        kind: ContentKind::Input { kind, message: message.clone() },
        phase: ContentPhase::ToolLoop,
        step_number: None, round: Some(round), text: None, tool_call_id: None,
    };
    env.host.content("", None, Some(&block));
    state.segment_builder.append_existing_segments(vec![block]);
}

/// 把本该收束的终答落成一条**中间** assistant 消息（对齐 pi：终答照常成为历史的一部分，
/// 随后的插话开启新的一轮）。文本/推理段已由 planning 落进 `segment_builder`（时间线顺序
/// 正确），这里只补两本历史账；下一轮 planning 的请求即带着这条 assistant + 新注入的插话。
pub(crate) fn absorb_final_answer(state: &mut RunState, message: GenerateOutput) {
    let message = RuntimeMessage::model(message.replay_message());
    state.runtime_messages.push(message.clone());
    state.generated_api_messages.push(message);
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
