//! Shared incremental splitting for screenshot translation streams.
use crate::prompts::COMBINED_TRANSLATE_SEPARATOR;
/// 合并翻译流的一段拆分产物：分隔符前为译文、后为原文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombinedSplitPiece {
    Translated(String),
    Original(String),
}

/// `<<<ORIGINAL>>>` 拆分状态机（从 `stream_translate_combined` 的内联逻辑提炼，
/// 行为逐条对齐；本体的迁移在后续步骤完成）。
///
/// 关键点（与原实现一致）：
/// - 分隔符可能跨 delta 边界 → tail 缓冲住末尾 (SEPARATOR.len()-1) 字节，防止把分隔符
///   前缀当译文 emit 出去
/// - tail 切片必须落在 UTF-8 char boundary，否则 `String::drain` 会 panic
///   （用户截图常含 CJK，每字 3 字节）
/// - 分隔符命中时：前段 trim 尾部换行、后段 trim 头部换行（仅限同一 delta 内的相邻换行）
#[derive(Debug, Default)]
pub struct CombinedTranslateSplitter {
    tail: String,
    sep_seen: bool,
    translated: String,
    original: String,
}

impl CombinedTranslateSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段文本增量，返回本次可安全发出的拆分片段（可能为空）。
    pub fn push(&mut self, delta: &str) -> Vec<CombinedSplitPiece> {
        let mut out = Vec::new();
        if delta.is_empty() {
            return out;
        }
        if self.sep_seen {
            self.original.push_str(delta);
            out.push(CombinedSplitPiece::Original(delta.to_string()));
            return out;
        }

        let sep = COMBINED_TRANSLATE_SEPARATOR;
        let sep_len = sep.len();
        self.tail.push_str(delta);
        if let Some(idx) = self.tail.find(sep) {
            // 分隔符命中：拆 before / after，trim 掉分隔符相邻的换行，分别发出
            let before: String = self.tail.drain(..idx).collect();
            // 移除分隔符本身
            self.tail.drain(..sep_len);
            let after: String = std::mem::take(&mut self.tail);

            let before_emit = before.trim_end_matches('\n').to_string();
            if !before_emit.is_empty() {
                self.translated.push_str(&before_emit);
                out.push(CombinedSplitPiece::Translated(before_emit));
            }
            self.sep_seen = true;
            let after_emit = after.trim_start_matches('\n').to_string();
            if !after_emit.is_empty() {
                self.original.push_str(&after_emit);
                out.push(CombinedSplitPiece::Original(after_emit));
            }
        } else {
            // 没命中：emit 安全前缀（保留末尾 sep_len-1 字节防止跨 delta 分隔符被切碎）
            let max_emit = self.tail.len().saturating_sub(sep_len.saturating_sub(1));
            if max_emit == 0 {
                return out;
            }
            // 找一个合法 char boundary（CJK 字符多字节，不能切到字符中间）
            let mut safe = max_emit;
            while safe > 0 && !self.tail.is_char_boundary(safe) {
                safe -= 1;
            }
            if safe == 0 {
                return out;
            }
            let to_emit: String = self.tail.drain(..safe).collect();
            self.translated.push_str(&to_emit);
            out.push(CombinedSplitPiece::Translated(to_emit));
        }
        out
    }

    /// 流结束/取消时把残留 tail 当译文 flush（避免末尾几个字符丢失）。
    /// 与原实现一致：仅在分隔符尚未出现且 tail 非空时有内容。
    pub fn flush_tail(&mut self) -> Option<String> {
        if self.sep_seen || self.tail.is_empty() {
            return None;
        }
        let tail = std::mem::take(&mut self.tail);
        self.translated.push_str(&tail);
        Some(tail)
    }

    /// 已累积的完整译文（含已 flush 的 tail）。
    pub fn translated(&self) -> &str {
        &self.translated
    }

    /// 已累积的完整原文。
    pub fn original(&self) -> &str {
        &self.original
    }
}
