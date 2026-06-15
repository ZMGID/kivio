//! 框架级:模型调用失败的统一分类 + 恢复策略。
//!
//! 设计目标:一种失败 = 一条分类(`classify`)+ 一条策略(`decide`),所有模型调用阶段
//! 共用;并保证「产生过工具结果的轮次永不空手而归」这一不变式只在此处定义
//! (`DegradeToGathered` → `assemble_results_from_tool_records`)。
//!
//! 不重复造轮子:沿用 `api::extract_status_code` 从错误串里取 HTTP 状态码(failover 逻辑
//! 也是这么做的),内容审核 / 超长靠 body 关键词判定。错误既可能是流式 `ModelError`,
//! 也可能是非流式 `String`,统一按消息文本分类即可,无需改动适配器返回类型。

use crate::chat::types::{ToolCallRecord, ToolCallStatus};

/// 模型调用失败的归类(只列出我们会**区别处置**的类型)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureKind {
    /// 供应商内容审核拒绝(典型:400 + "content/risk/policy/safety/审核")。
    ContentModeration,
    /// 上下文超长(400/413 + "context/maximum/token length")。
    ContextOverflow,
    /// 模型调用成功但产出为空。
    Empty,
    /// 限流 / 鉴权 / 5xx / 网络等——底层 api.rs 已重试或换 key,升到这层即已耗尽。
    Exhausted,
    /// 其它(无法归类)。
    Other,
}

/// 对一次失败应采取的恢复动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryAction {
    /// 用"去敏 + 精简"的输入重做一次合成(可能产出真正的总结)。
    Remediate,
    /// 直接用已收集到的工具结果确定性兜底(不经模型 → 不被审核)。
    DegradeToGathered,
    /// 无可恢复(且没有工具结果)——交回上层用静态文案。
    Surface,
}

/// 把错误消息文本归类。`message` 为空视为 `Empty`(调用方在"成功但空响应"时传空串)。
pub(crate) fn classify(message: &str) -> FailureKind {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return FailureKind::Empty;
    }
    let status = crate::api::extract_status_code(trimmed);
    let lower = trimmed.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));

    // 内容审核:供应商措辞不一,关键词覆盖中英常见形态。
    if has(&[
        "content exists risk",
        "content policy",
        "content_policy",
        "content filter",
        "moderation",
        "safety",
        "sensitive",
        "审核",
        "违规",
        "敏感",
    ]) {
        return FailureKind::ContentModeration;
    }
    // 上下文超长。
    if has(&[
        "maximum context",
        "context length",
        "context_length_exceeded",
        "too many tokens",
        "reduce the length",
        "string too long",
    ]) {
        return FailureKind::ContextOverflow;
    }
    match status {
        // 审核常以 400 返回但措辞没命中上面的词:仍按 BadRequest→Other 处理,交给
        // Remediate 兜一手(去敏精简后重试),不会更糟。
        Some(429) | Some(401) | Some(402) | Some(403) => FailureKind::Exhausted,
        Some(code) if (500..600).contains(&code) => FailureKind::Exhausted,
        _ => FailureKind::Other,
    }
}

/// 策略:给定失败类型 + 上下文,决定动作。集中表达,取代各阶段散落判断。
///
/// `has_tool_results`:本轮是否已产生工具结果(决定能否兜底)。
/// `already_remediated`:是否已经做过一次 Remediate(避免无限重试)。
pub(crate) fn decide(
    kind: FailureKind,
    has_tool_results: bool,
    already_remediated: bool,
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
        // 请求因内容/长度被拒 → 用去敏精简的输入重做一次,通常能产出真正的总结。
        FailureKind::ContentModeration | FailureKind::ContextOverflow => RecoveryAction::Remediate,
        // 空响应 / 限流耗尽 / 其它:重做无意义,直接用已收集结果兜底,保证有结果。
        FailureKind::Empty | FailureKind::Exhausted | FailureKind::Other => {
            RecoveryAction::DegradeToGathered
        }
    }
}

/// 不变式实现:从已收集的工具结果确定性拼出可读答复(不经模型,不会被审核)。
/// 没有任何可用 preview 时返回空串,调用方据此退回静态文案。
pub(crate) fn assemble_results_from_tool_records(
    records: &[ToolCallRecord],
    language: &str,
) -> String {
    let zh = language.starts_with("zh");
    let mut blocks: Vec<String> = Vec::new();
    for record in records {
        if record.status != ToolCallStatus::Success {
            continue;
        }
        let preview = record
            .result_preview
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty());
        if let Some(preview) = preview {
            blocks.push(format!("【{}】\n{}", record.name, preview));
        }
    }
    if blocks.is_empty() {
        return String::new();
    }
    let header = if zh {
        "未能生成总结(模型供应商内容审核拦截或调用失败),以下是已检索到的内容:"
    } else {
        "Could not produce a summary (provider content moderation or call failure); here is what was gathered:"
    };
    format!("{header}\n\n{}", blocks.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(name: &str, status: ToolCallStatus, preview: Option<&str>) -> ToolCallRecord {
        ToolCallRecord {
            id: "t".into(),
            name: name.into(),
            source: "native".into(),
            server_id: None,
            arguments: String::new(),
            status,
            result_preview: preview.map(|p| p.to_string()),
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 0,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
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
            FailureKind::Exhausted
        );
    }

    #[test]
    fn decide_upholds_invariant() {
        // 无工具结果 → 交回上层
        assert_eq!(
            decide(FailureKind::ContentModeration, false, false),
            RecoveryAction::Surface
        );
        // 审核 + 有结果 + 未补救 → 先去敏重试
        assert_eq!(
            decide(FailureKind::ContentModeration, true, false),
            RecoveryAction::Remediate
        );
        // 补救后仍失败 → 确定性兜底
        assert_eq!(
            decide(FailureKind::ContentModeration, true, true),
            RecoveryAction::DegradeToGathered
        );
        // 已耗尽(限流/5xx)+ 有结果 → 直接兜底
        assert_eq!(
            decide(FailureKind::Exhausted, true, false),
            RecoveryAction::DegradeToGathered
        );
    }

    #[test]
    fn assemble_uses_successful_previews_only() {
        let records = vec![
            rec("web_search", ToolCallStatus::Success, Some("标题A\n标题B")),
            rec("web_search", ToolCallStatus::Error, Some("不该出现")),
            rec("noop", ToolCallStatus::Success, None),
        ];
        let out = assemble_results_from_tool_records(&records, "zh-CN");
        assert!(out.contains("标题A"));
        assert!(!out.contains("不该出现"));
        assert!(out.contains("web_search"));

        assert!(assemble_results_from_tool_records(&[], "zh-CN").is_empty());
    }
}
