use crate::chat::agent::content::AgentToolStatus;
use crate::chat::agent::{
    history::RuntimeMessage,
    recovery::{
        FallbackReason, RecoveryDecision, RecoveryFailure, RecoveryInput, RecoverySnapshot,
    },
};
use crate::chat::model::ModelRole;
use crate::chat::model::{ModelError, ModelErrorKind};

enum PendingRecovery {
    Overflow,
    Reduced { kind: FailureKind, error: String },
}

pub(crate) struct RecoveryPolicy {
    language: String,
    api_format: String,
    context_window: usize,
    pending: Option<PendingRecovery>,
    degraded: Option<DegradedAnswer>,
}

impl RecoveryPolicy {
    pub(crate) fn new(
        provider: &crate::settings::ModelProvider,
        model: &str,
        language: &str,
    ) -> Self {
        Self {
            language: language.to_owned(),
            api_format: provider.api_format.clone(),
            context_window: crate::chat::model_metadata::context_window_for_model(
                Some(provider),
                model,
            )
            .0,
            pending: None,
            degraded: None,
        }
    }

    pub(crate) fn recover(
        &mut self,
        run: RecoverySnapshot<'_>,
        error: &ModelError,
    ) -> RecoveryDecision {
        let mut kind = failure_kind(error.kind);
        if kind == FailureKind::Empty
            && is_silent_overflow(
                run.last_step_usage.and_then(|usage| {
                    crate::chat::model::context_usage::prompt_tokens(usage, &self.api_format)
                }),
                self.context_window,
            )
        {
            kind = FailureKind::ContextOverflow;
        }
        self.pending = None;
        self.degraded = None;
        match decide(kind, !run.tool_records.is_empty(), false, false) {
            RecoveryAction::Surface => RecoveryDecision::Surface,
            RecoveryAction::DegradeToGathered => {
                RecoveryDecision::Complete(self.degrade(kind, Some(&error.message)))
            }
            RecoveryAction::CompactAndRetry => {
                self.pending = Some(PendingRecovery::Overflow);
                RecoveryDecision::Retry(RecoveryInput::CompactCurrent)
            }
            RecoveryAction::Remediate => {
                self.pending = Some(PendingRecovery::Reduced {
                    kind,
                    error: error.message.clone(),
                });
                RecoveryDecision::Retry(RecoveryInput::Reduced(build_neutral_reduced_messages(
                    run,
                    &self.language,
                )))
            }
        }
    }

    pub(crate) fn recovery_failed(&mut self, failure: RecoveryFailure<'_>) -> String {
        match self.pending.take() {
            Some(PendingRecovery::Overflow) => match failure {
                RecoveryFailure::Blocked | RecoveryFailure::Empty => {
                    self.degrade(FailureKind::ContextOverflow, None)
                }
                RecoveryFailure::Error(error) => {
                    self.degrade(FailureKind::ContextOverflow, Some(&error.message))
                }
            },
            Some(PendingRecovery::Reduced { kind, error }) => self.degrade(kind, Some(&error)),
            None => String::new(),
        }
    }

    fn degrade(&mut self, kind: FailureKind, error: Option<&str>) -> String {
        let card = DegradedAnswer::new(kind, &self.language, error);
        let text = card.text.clone();
        self.degraded = Some(card);
        text
    }

    pub(crate) fn context_blocked(&mut self) -> String {
        self.degrade(FailureKind::ContextOverflow, None)
    }

    pub(crate) fn model_failure_card(&self, error: &ModelError) -> DegradedAnswer {
        DegradedAnswer::new(
            failure_kind(error.kind),
            &self.language,
            Some(&error.message),
        )
    }

    pub(crate) fn fallback(&self, reason: FallbackReason) -> String {
        match reason {
            FallbackReason::EmptySynthesis => empty_synthesis_fallback_response(&self.language),
            FallbackReason::FailedSynthesis => synthesis_failed_fallback_response(&self.language),
            FallbackReason::FailedPlanning => {
                tool_planning_failed_fallback_response(&self.language)
            }
        }
    }

    pub(crate) fn take_degraded(&mut self) -> Option<DegradedAnswer> {
        self.degraded.take()
    }
}

fn last_user_text(messages: &[RuntimeMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|message| message.role() == Some(ModelRole::User))
        .filter(|message| !crate::chat::todo::is_reminder_message(message))
        .map(RuntimeMessage::text_content)
        .find(|text| !text.trim().is_empty())
}

fn build_neutral_reduced_messages(
    run: RecoverySnapshot<'_>,
    language: &str,
) -> Vec<RuntimeMessage> {
    let question = last_user_text(run.history).unwrap_or_default();
    let previews = run
        .tool_records
        .iter()
        .filter(|record| record.status == AgentToolStatus::Success)
        .filter_map(|record| {
            record
                .output
                .as_deref()
                .map(|output| {
                    crate::chat::agent::execute::limit_tool_text_for_model(
                        &crate::chat::tools::summarize_tool_text(output),
                        record.output_limit,
                    )
                })
                .filter(|preview| !preview.trim().is_empty())
                .map(|preview| format!("【{}】\n{}", record.name, preview))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    vec![
        RuntimeMessage::system(format!("Answer the user's question objectively and neutrally, strictly based on the search snippets below. Only organize and state information already present in the snippets; add no commentary, stance, or outside content. Respond in the language of the user's question unless they request another language. If unclear, use the configured language: {language}.")),
        RuntimeMessage::text(ModelRole::User, format!("User question: {question}\n\nSearch snippets:\n{previews}")),
    ]
}

/// 降级回答的结构化描述，落到 `ChatMessage.degraded` 供前端渲染成独立卡片。
///
/// 为什么不只发拼好的文本：降级信息此前是一段纯文本塞进 assistant 正文，看起来和
/// 模型的真实回答一模一样，会被复制、会进对话历史再发回模型。结构化后正文归正文、
/// 故障归卡片。`text` 保留同样的纯文本，供旧前端与外部 CLI 回落。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DegradedAnswer {
    /// 失败归类的稳定标识（`rate_limited` / `context_overflow` / `moderation` /
    /// `empty_response` / `unknown`），前端据此选图标与措辞，不解析文案。
    pub kind: String,
    /// 一行人读的失败原因（已按 kind 精确化，不再笼统说"可能上下文过长"）。
    pub reason: String,
    /// 供应商返回的原始报错（已剥壳裁剪）。`reason` 只是分类话术，真正能排查的是这个。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Plain-text fallback, also used to identify the notice in older transcripts.
    pub text: String,
}

impl DegradedAnswer {
    fn new(kind: FailureKind, language: &str, error: Option<&str>) -> Self {
        let reason = failure_reason_line(kind, language.starts_with("zh"));
        Self {
            kind: kind.wire_kind().into(),
            reason: reason.into(),
            detail: error.and_then(extract_error_detail),
            text: reason.into(),
        }
    }
}

impl FailureKind {
    /// 稳定的 wire 标识。前端按它分支，不依赖本地化文案。
    pub(crate) fn wire_kind(self) -> &'static str {
        match self {
            FailureKind::ContentModeration => "moderation",
            FailureKind::ContextOverflow => "context_overflow",
            FailureKind::Empty => "empty_response",
            FailureKind::Exhausted => "rate_limited",
            FailureKind::ProviderUnavailable => "provider_unavailable",
            FailureKind::Timeout => "timeout",
            FailureKind::InvalidImage => "invalid_image",
            FailureKind::InvalidRequest => "invalid_request",
            FailureKind::Other => "unknown",
        }
    }
}

/// 模型调用失败的归类(只列出我们会**区别处置**的类型)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    /// 供应商内容审核拒绝(典型:400 + "content/risk/policy/safety/审核")。
    ContentModeration,
    /// 上下文超长(400/413 + "context/maximum/token length")。
    ContextOverflow,
    /// 模型调用成功但产出为空。
    Empty,
    /// Rate limits reported after the provider request returns.
    Exhausted,
    ProviderUnavailable,
    /// 传输层超时 / 连接中断 / 流断包——**没有** HTTP 状态码,供应商压根没答完。
    /// 典型:大上下文经第三方中转,SSE 跑到一半被截断,重试再超时。
    Timeout,
    /// Invalid media cannot be repaired by sending the same request again.
    InvalidImage,
    /// Deterministic request, credentials or account errors require intervention.
    InvalidRequest,
    /// 其它(无法归类)。
    Other,
}

/// 对一次失败应采取的恢复动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    /// 上下文超长:先压缩一次历史(maybe_compact_send_view),再用压缩后的消息重发一次。
    /// 对标 PI 的 `_overflowRecoveryAttempted` 单次守门——只压缩-重试一次,避免「压完仍超」死循环。
    CompactAndRetry,
    /// 用"去敏 + 精简"的输入重做一次合成(可能产出真正的总结)。
    Remediate,
    /// 直接用已收集到的工具结果确定性兜底(不经模型 → 不被审核)。
    DegradeToGathered,
    /// 无可恢复(且没有工具结果)——交回上层用静态文案。
    Surface,
}

/// 静默超窗的判定线：provider 实报的 prompt token 占窗口的比例。
/// 取 0.99 对齐 pi `isContextOverflow` 的 case 3；case 2 的「严格大于窗口」是它的子集,
/// 一条线同时盖住两种形态。
const SILENT_OVERFLOW_RATIO: f64 = 0.99;

/// pi `isContextOverflow` 的 case 2/3 —— **不报错的超窗**。
///
/// [`classify`] 只认错误文案，但有一类供应商/网关压根不报错：超窗请求照收，然后回
/// 一个 200 + 空正文（pi 点名 z.ai），或者把输入截断到刚好填满窗口、`finish_reason`
/// 给 `length` 而 output 是 0（pi 点名 Xiaomi MiMo）。两种形态都没有任何文案可匹配,
/// 只看文本就永远发现不了，表现为「模型返回了空响应」。
///
/// 唯一的证据是 provider 实报的用量：prompt 已经顶到窗口，那这就是撑爆了，不管对方
/// 说没说。命中后按 `ContextOverflow` 处置（压缩一次再重发），而不是当成 `Empty`
/// 直接降级 —— 后者对同一份输入重来一次只会再失败。
///
/// **命中不了的场景要心里有数**：这条判据的分母是**模型**的窗口。经第三方中转时,
/// 中转自己的上限往往远小于模型窗口（实测 60 万 token 打 1M 窗口即被吞），那种情况
/// 用量离窗口还远，这里判不出来 —— 真正的防线是工具输出上限，别让请求长到那个份上。
pub(crate) fn is_silent_overflow(prompt_tokens: Option<u64>, window: usize) -> bool {
    let Some(prompt) = prompt_tokens else {
        return false;
    };
    if window == 0 {
        return false;
    }
    prompt as f64 >= window as f64 * SILENT_OVERFLOW_RATIO
}

#[cfg(test)]
pub(crate) fn classify(message: &str) -> FailureKind {
    failure_kind(ModelError::new(message).kind)
}

pub(crate) fn failure_kind(kind: ModelErrorKind) -> FailureKind {
    match kind {
        ModelErrorKind::InvalidImage => FailureKind::InvalidImage,
        ModelErrorKind::ContentModeration => FailureKind::ContentModeration,
        ModelErrorKind::ContextOverflow => FailureKind::ContextOverflow,
        ModelErrorKind::InvalidRequest
        | ModelErrorKind::Auth
        | ModelErrorKind::ToolsUnsupported => FailureKind::InvalidRequest,
        ModelErrorKind::RateLimited => FailureKind::Exhausted,
        ModelErrorKind::ProviderUnavailable => FailureKind::ProviderUnavailable,
        ModelErrorKind::Timeout | ModelErrorKind::Transport => FailureKind::Timeout,
        ModelErrorKind::EmptyResponse => FailureKind::Empty,
        _ => FailureKind::Other,
    }
}

/// 策略:给定失败类型 + 上下文,决定动作。集中表达,取代各阶段散落判断。
///
/// `has_tool_results`:本轮是否已产生工具结果(决定能否兜底)。
/// `already_remediated`:是否已经做过一次 Remediate(避免无限重试)。
/// `overflow_recovery_attempted`:是否已经做过一次 CompactAndRetry(单次守门,避免
/// 「压缩后仍超 → 再压缩」死循环,对标 PI `_overflowRecoveryAttempted`)。
pub(crate) fn decide(
    kind: FailureKind,
    has_tool_results: bool,
    already_remediated: bool,
    overflow_recovery_attempted: bool,
) -> RecoveryAction {
    if !has_tool_results {
        // 没有可兜底的素材:只能交回上层(静态文案 / 向上传播错误)。
        return RecoveryAction::Surface;
    }
    if already_remediated {
        // 去敏重试都失败了,确定性兜底,保证有结果。
        return RecoveryAction::DegradeToGathered;
    }
    match kind {
        // 上下文超长:走「压缩一次 → 重发一次」专用通道(而非去敏重试)。压缩+重试已经
        // 试过一次仍失败 → 降级到确定性兜底,避免死循环(单次守门)。
        FailureKind::ContextOverflow => {
            if overflow_recovery_attempted {
                RecoveryAction::DegradeToGathered
            } else {
                RecoveryAction::CompactAndRetry
            }
        }
        // Only explicit content moderation permits the existing one-shot summary recovery.
        // Unknown errors and deterministic input failures must not trigger another call.
        FailureKind::ContentModeration => RecoveryAction::Remediate,
        FailureKind::ProviderUnavailable => RecoveryAction::Surface,
        // 空响应 / 限流耗尽:重做无意义(同样的输入只会再失败),直接用已收集结果兜底。
        // 超时同理,而且更甚:去敏重做还要再排队等一次 3 分钟超时,纯粹浪费用户时间。
        FailureKind::Empty
        | FailureKind::Exhausted
        | FailureKind::Timeout
        | FailureKind::InvalidImage
        | FailureKind::InvalidRequest
        | FailureKind::Other => RecoveryAction::DegradeToGathered,
    }
}

/// 把错误串压成**一句能看**的原始报错，给卡片当证据用。
///
/// 输入形如 `Chat tools planning Error: 400 Bad Request - {"error":{"message":"…"}} (attempt 3/3)`，
/// 三步瘦身：去掉内部阶段名前缀 → 把 JSON body 换成里面的 message → 裁剪。
/// 状态码和 `(attempt N/M)` 都留着，它们正是排查要看的东西。
///
/// ponytail: 只认 `" Error: "` 一个前缀分隔符、只解一次 JSON。解不动就原样出，
/// 显示得丑一点也好过把真报错吞掉。
pub(crate) fn extract_error_detail(raw: &str) -> Option<String> {
    const MAX_DETAIL_CHARS: usize = 240;
    let mut detail = raw.trim();
    if detail.is_empty() {
        return None;
    }
    // "Chat tools planning Error: xxx" → "xxx"（阶段名对用户无意义）
    if let Some(at) = detail.find(" Error: ") {
        detail = detail[at + " Error: ".len()..].trim_start();
    }
    let mut detail = detail
        .strip_prefix("ChatGPT Responses error: ")
        .unwrap_or(detail)
        .to_string();
    // {"error":{"message":"真话"}} → 真话
    if let (Some(start), Some(end)) = (detail.find('{'), detail.rfind('}')) {
        if start < end {
            if let Some(msg) = serde_json::from_str::<serde_json::Value>(&detail[start..=end])
                .ok()
                .and_then(|v| {
                    let err = v.get("error");
                    err.and_then(|e| e.get("message"))
                        .or_else(|| v.get("message"))
                        .or(err)
                        .and_then(|m| m.as_str().map(str::to_string))
                })
                .filter(|s| !s.trim().is_empty())
            {
                detail.replace_range(start..=end, msg.trim());
            }
        }
    }
    let detail = detail.trim();
    Some(if detail.chars().count() > MAX_DETAIL_CHARS {
        format!(
            "{}…",
            detail.chars().take(MAX_DETAIL_CHARS).collect::<String>()
        )
    } else {
        detail.to_string()
    })
}

/// 一行人读的失败原因(按 `kind`)。这是修复「降级消息误导」的核心:429 就说限流,
/// 上下文超长就说上下文,绝不再把限流说成"可能上下文过长"。
fn failure_reason_line(kind: FailureKind, zh: bool) -> &'static str {
    match kind {
        FailureKind::InvalidImage => {
            if zh {
                "图片数据、格式或尺寸不被接受，已停止重试。请重新读取图片，或转换为 PNG/JPEG 后重新发送。"
            } else {
                "Image data, format or dimensions were rejected. Retries stopped. Read the image again or convert it to PNG/JPEG before resending."
            }
        }
        FailureKind::InvalidRequest => {
            if zh {
                "请求被拒绝，重复发送无法解决。请按下方错误检查输入、模型参数、访问权限或账户额度。已完成的工具结果保留。"
            } else {
                "The request was rejected and will not be repeated. Check the input, model parameters, access or account quota using the error below. Completed tool results are preserved."
            }
        }
        FailureKind::ProviderUnavailable => {
            if zh {
                "模型服务暂时不可用，请稍后重试。"
            } else {
                "The model service is temporarily unavailable. Try again later."
            }
        }
        FailureKind::Exhausted => {
            if zh {
                "模型服务限流或暂时故障，重试后仍未完成。请稍后重试或更换供应商。"
            } else {
                "The model service was rate-limited or temporarily unavailable and retries did not complete. Retry later or switch providers."
            }
        }
        FailureKind::ContextOverflow => {
            if zh {
                "上下文超出模型窗口,压缩后重试仍失败。请改用更大上下文的模型,或精简对话后重试。"
            } else {
                "The context exceeded the model's window and still failed after compaction. Switch to a larger-context model or trim the conversation."
            }
        }
        FailureKind::ContentModeration => {
            if zh {
                "请求被供应商内容审核拦截。换个措辞,或更换供应商后重试。"
            } else {
                "The request was blocked by the provider's content moderation. Rephrase or switch providers."
            }
        }
        FailureKind::Empty => {
            if zh {
                "模型返回了空响应。请重试,或更换模型。"
            } else {
                "The model returned an empty response. Retry or switch models."
            }
        }
        FailureKind::Timeout => {
            if zh {
                "请求超时 / 流式连接中断,多次重试后仍失败 —— 供应商没把这次回答传完。常见于上下文过大叠加第三方中转:精简对话,或换一个更稳的接口地址。"
            } else {
                "The request timed out or the stream broke, and retries still failed — the provider never finished this response. Usually an oversized context through a relay endpoint: trim the conversation or switch to a more reliable endpoint."
            }
        }
        FailureKind::Other => {
            if zh {
                "模型调用失败。请重试,或更换供应商。"
            } else {
                "The model call failed. Retry or switch providers."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::agent::content::AgentToolCall;

    fn policy() -> RecoveryPolicy {
        RecoveryPolicy {
            language: "en".into(),
            api_format: "openai".into(),
            context_window: 100,
            pending: None,
            degraded: None,
        }
    }

    #[test]
    fn reduced_recovery_keeps_question_and_original_failure() {
        let history = vec![
            RuntimeMessage::text(ModelRole::User, "actual question"),
            RuntimeMessage::text(
                ModelRole::User,
                "<todo-reminder>\npending work\n</todo-reminder>",
            ),
        ];
        let records = vec![rec("read", AgentToolStatus::Success, Some("useful result"))];
        let run = RecoverySnapshot {
            history: &history,
            tool_records: &records,
            last_step_usage: None,
        };
        let mut policy = policy();
        let failure =
            ModelError::with_kind("original moderation", ModelErrorKind::ContentModeration);
        let RecoveryDecision::Retry(RecoveryInput::Reduced(messages)) =
            policy.recover(run, &failure)
        else {
            panic!("moderation with gathered results should produce one reduced request")
        };
        assert!(messages[1].text_content().contains("actual question"));
        assert!(!messages[1].text_content().contains("pending work"));
        let retry_error = ModelError::with_kind("later timeout", ModelErrorKind::Timeout);
        policy.recovery_failed(RecoveryFailure::Error(&retry_error));
        assert_eq!(
            policy.take_degraded().unwrap().detail.as_deref(),
            Some("original moderation")
        );
    }

    #[test]
    fn overflow_recovery_provides_a_card_for_blocked_context_and_retry_failure() {
        let records = vec![rec("read", AgentToolStatus::Success, Some("useful result"))];
        let run = RecoverySnapshot {
            history: &[],
            tool_records: &records,
            last_step_usage: None,
        };
        let mut policy = policy();
        let failure = ModelError::with_kind("overflow", ModelErrorKind::ContextOverflow);
        assert!(matches!(
            policy.recover(run, &failure),
            RecoveryDecision::Retry(RecoveryInput::CompactCurrent)
        ));
        assert!(!policy.recovery_failed(RecoveryFailure::Blocked).is_empty());
        assert_eq!(policy.take_degraded().unwrap().kind, "context_overflow");
        policy.recover(run, &failure);
        let retry_error = ModelError::with_kind("retry rejected", ModelErrorKind::InvalidRequest);
        policy.recovery_failed(RecoveryFailure::Error(&retry_error));
        assert_eq!(
            policy.take_degraded().unwrap().detail.as_deref(),
            Some("retry rejected")
        );
    }

    #[test]
    fn last_user_text_skips_todo_reminders() {
        let messages = vec![
            RuntimeMessage::text(ModelRole::User, "修一下登录页"),
            RuntimeMessage::text(
                ModelRole::User,
                "<todo-reminder>\n1. [pending] x\n</todo-reminder>",
            ),
        ];
        assert_eq!(last_user_text(&messages).as_deref(), Some("修一下登录页"));
    }

    #[test]
    fn silent_overflow_needs_usage_evidence_at_the_window() {
        // 顶到窗口（含刚好过 0.99 线）→ 判超窗。
        assert!(is_silent_overflow(Some(200_000), 200_000));
        assert!(is_silent_overflow(Some(210_000), 200_000)); // pi case 2：严格超窗
        assert!(is_silent_overflow(Some(198_000), 200_000)); // pi case 3：0.99 线
                                                             // 差一点点就不算——0.99 线以下不许猜。
        assert!(!is_silent_overflow(Some(197_000), 200_000));
        // 实测那一例：60 万 token 打 1M 窗口，卡住的是中转不是模型窗口，
        // 这条判据本来就该判不出来（真正的防线是工具输出上限）。
        assert!(!is_silent_overflow(Some(602_824), 1_000_000));
        // 没有用量 / 窗口未知 → 不猜。
        assert!(!is_silent_overflow(None, 200_000));
        assert!(!is_silent_overflow(Some(999_999), 0));
    }

    fn rec(name: &str, status: AgentToolStatus, preview: Option<&str>) -> AgentToolCall {
        AgentToolCall {
            id: "t".into(),
            name: name.into(),
            source: "native".into(),
            server_id: None,
            arguments: String::new(),
            status,
            output: preview.map(|p| p.to_string()),
            output_limit: None,
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 0,
            sensitive: false,
            artifacts: Vec::new(),
            draft: None,
            structured_content: None,
        }
    }

    #[test]
    fn classify_detects_moderation_overflow_empty() {
        assert_eq!(
            classify("Chat stream Error: 400 Bad Request - {\"error\":{\"message\":\"Content Exists Risk\"}}"),
            FailureKind::ContentModeration
        );
        assert_eq!(
            classify("Error: 400 - This model's maximum context length is 8192 tokens"),
            FailureKind::ContextOverflow
        );
        assert_eq!(classify(""), FailureKind::Empty);
        assert_eq!(
            classify("Chat stream Error: 429 Too Many Requests"),
            FailureKind::Exhausted
        );
        assert_eq!(
            classify("Chat API Error: 500 Internal Server Error"),
            FailureKind::ProviderUnavailable
        );
    }

    #[test]
    fn classify_detects_broad_overflow_provider_wording() {
        // 各供应商的超长文案都应归到 ContextOverflow(多以 400 返回)。
        for msg in [
            "Error: 400 - prompt is too long: 250000 tokens > 200000 maximum",
            "Chat API Error: 413 - request too large",
            "Error: 400 context_length_exceeded",
            "Error: 400 - input is too long for requested model",
            "Error: 400 - This request exceeds the maximum token limit",
            "Error: 400 reduce the length of the messages",
            "错误:400 - 上下文长度超出最大限制",
            "错误:400 - 输入过长",
            "Error: 400 - The input token count (1196265) exceeds the maximum number of tokens allowed (1048575)",
            "Error: 400 - Prompt has 300000 tokens, but the configured context size is 200000 tokens",
            "Error: 400 - prompt exceeds max length",
            "Error: 400 - Range of input length should be [1, 262144]",
        ] {
            assert_eq!(
                classify(msg),
                FailureKind::ContextOverflow,
                "should classify as overflow: {msg}"
            );
        }
    }

    #[test]
    fn classify_excludes_rate_limit_from_overflow() {
        // 限流/配额文案虽含 limit/token 等词,绝不能被当成 overflow。核心断言:不是 ContextOverflow。
        for msg in [
            "Chat stream Error: 429 Too Many Requests - rate limit exceeded, too many tokens per minute",
            "Chat API Error: 429 - quota exceeded for this token limit",
            "Chat API Error: 402 - insufficient available balance",
        ] {
            let kind = classify(msg);
            assert_ne!(kind, FailureKind::ContextOverflow, "must not be overflow: {msg}");
            assert!(matches!(kind, FailureKind::Exhausted | FailureKind::InvalidRequest));
        }
    }

    #[test]
    fn decide_upholds_invariant() {
        // 无工具结果 → 交回上层
        assert_eq!(
            decide(FailureKind::ContentModeration, false, false, false),
            RecoveryAction::Surface
        );
        // 审核 + 有结果 + 未补救 → 先去敏重试
        assert_eq!(
            decide(FailureKind::ContentModeration, true, false, false),
            RecoveryAction::Remediate
        );
        // 补救后仍失败 → 确定性兜底
        assert_eq!(
            decide(FailureKind::ContentModeration, true, true, false),
            RecoveryAction::DegradeToGathered
        );
        // 已耗尽(限流/5xx)+ 有结果 → 直接兜底
        assert_eq!(
            decide(FailureKind::Exhausted, true, false, false),
            RecoveryAction::DegradeToGathered
        );
        // Unknown failures retain completed results without speculative retries.
        assert_eq!(
            decide(FailureKind::Other, true, false, false),
            RecoveryAction::DegradeToGathered
        );
        // 空响应重做无意义 → 直接兜底
        assert_eq!(
            decide(FailureKind::Empty, true, false, false),
            RecoveryAction::DegradeToGathered
        );
    }

    #[test]
    fn decide_overflow_compacts_once_then_degrades() {
        // overflow + 有结果 + 未尝试压缩重试 → 走压缩重试通道
        assert_eq!(
            decide(FailureKind::ContextOverflow, true, false, false),
            RecoveryAction::CompactAndRetry
        );
        // overflow + 已尝试过一次压缩重试 → 单次守门:降级兜底,不再压缩(防死循环)
        assert_eq!(
            decide(FailureKind::ContextOverflow, true, false, true),
            RecoveryAction::DegradeToGathered
        );
        // overflow + 无工具结果 → 仍交回上层(没素材可兜底)
        assert_eq!(
            decide(FailureKind::ContextOverflow, false, false, false),
            RecoveryAction::Surface
        );
    }

    #[test]
    fn failure_card_preserves_diagnostic_and_reads_older_fallback_text() {
        let card = DegradedAnswer::new(
            FailureKind::ProviderUnavailable,
            "zh-CN",
            Some(
                r#"ChatGPT Responses error: {"code":"server_is_overloaded","message":"Our servers are currently overloaded."}"#,
            ),
        );
        assert_eq!(card.kind, "provider_unavailable");
        assert_eq!(card.text, card.reason);
        assert!(card
            .detail
            .as_deref()
            .unwrap()
            .contains("Our servers are currently overloaded."));
        let old: DegradedAnswer = serde_json::from_value(serde_json::json!({
            "kind":"unknown", "reason":"failed", "text":"old tool summary",
            "toolSummaries":[{"name":"bash","preview":"exit_code: 0"}], "totalToolCalls":9
        }))
        .unwrap();
        assert_eq!(old.text, "old tool summary");
    }

    #[test]
    fn extract_error_detail_strips_stage_prefix_and_unwraps_json() {
        // 400 + JSON body：阶段名去掉、body 换成 message、状态码和重试次数留着
        assert_eq!(
            extract_error_detail(
                r#"Chat stream Error: 400 Bad Request - {"error":{"message":"Content Exists Risk"}} (attempt 1/1)"#
            )
            .as_deref(),
            Some("400 Bad Request - Content Exists Risk (attempt 1/1)")
        );
        // 真实日志样本：401（顶层 error.message）
        assert_eq!(
            extract_error_detail(
                r#"Chat tools planning Error: 401 Unauthorized - {"error":{"message":"Authentication Fails, Your api key: ****f094 is invalid","type":"authentication_error"}} (attempt 1/3)"#
            )
            .as_deref(),
            Some("401 Unauthorized - Authentication Fails, Your api key: ****f094 is invalid (attempt 1/3)")
        );
        // 真实日志样本：超时（无 JSON，原样保留 url 与重试次数）
        assert_eq!(
            extract_error_detail(
                "Chat tools planning Error: error sending request for url (https://relay.example/v1/responses) → operation timed out [timeout,request] (attempt 3/3)"
            )
            .as_deref(),
            Some("error sending request for url (https://relay.example/v1/responses) → operation timed out [timeout,request] (attempt 3/3)")
        );
        assert_eq!(extract_error_detail("   "), None);
        // 超长裁剪，不把整份 body 糊进卡片
        let long = extract_error_detail(&"x".repeat(2000)).expect("some");
        assert!(long.chars().count() <= 241, "got {}", long.chars().count());
    }

    #[test]
    fn classify_detects_transport_failures_without_status() {
        // 真实日志里的两条：超时 + SSE 断包。没有状态码 ⇒ Timeout，而不是笼统的 Other。
        for msg in [
            "Chat tools planning Error: error sending request for url (https://relay.example/v1/responses) → operation timed out [timeout,request] (attempt 3/3)",
            "Chat tools planning 流式响应读取中断：the provider sent an incomplete or invalid encoded stream chunk。",
            "Chat stream Error: connection reset by peer",
            "stream ended without terminal event or completed response",
        ] {
            assert_eq!(classify(msg), FailureKind::Timeout, "should be timeout: {msg}");
        }
        // A gateway HTTP failure is reported by the provider.
        assert_eq!(
            classify("Chat API Error: 504 Gateway Timeout"),
            FailureKind::ProviderUnavailable
        );
        // 超时不做去敏重做——那只会再排队等一次超时
        assert_eq!(
            decide(FailureKind::Timeout, true, false, false),
            RecoveryAction::DegradeToGathered
        );
        assert_eq!(FailureKind::Timeout.wire_kind(), "timeout");
    }

    #[test]
    fn wire_kind_is_stable_per_failure_kind() {
        // 前端按这些字符串分支，改动即破坏契约。
        assert_eq!(FailureKind::Exhausted.wire_kind(), "rate_limited");
        assert_eq!(FailureKind::ContextOverflow.wire_kind(), "context_overflow");
        assert_eq!(FailureKind::ContentModeration.wire_kind(), "moderation");
        assert_eq!(FailureKind::Empty.wire_kind(), "empty_response");
        assert_eq!(FailureKind::Other.wire_kind(), "unknown");
    }
}

pub(crate) fn empty_synthesis_fallback_response(language: &str) -> String {
    if language.starts_with("zh") {
        "工具调用已经完成，但模型没有返回最终总结。上方工具结果已保存在本轮回复中，你可以继续追问，或让我重新生成总结。".to_string()
    } else {
        "The tool calls completed, but the model did not return a final summary. The tool results above were saved with this reply; you can continue from them or regenerate the summary.".to_string()
    }
}

pub(crate) fn synthesis_failed_fallback_response(language: &str) -> String {
    if language.starts_with("zh") {
        "最终总结生成失败(可能是模型供应商内容审核拦截)。上方工具结果已保存在本轮回复中,你可以继续追问、让我重新生成,或更换聊天模型再试。".to_string()
    } else {
        "Final summary generation failed (possibly provider content moderation). The tool results above were saved with this reply; you can continue from them, regenerate, or switch the chat model and retry.".to_string()
    }
}

pub(crate) fn tool_planning_failed_fallback_response(language: &str) -> String {
    if language.starts_with("zh") {
        "工具调用参数生成失败，这一步还没有真正执行写入。主对话已保留，你可以让我缩小范围、改用补丁，或重新生成。".to_string()
    } else {
        "Tool-call argument generation failed before the write actually ran. This conversation was preserved; you can ask me to narrow the scope, use a patch, or regenerate.".to_string()
    }
}
