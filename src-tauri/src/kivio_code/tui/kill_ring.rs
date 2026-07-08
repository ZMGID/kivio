//! Emacs 风格 kill-ring —— PI `kill-ring.ts` 端口。
//!
//! 记录被删除（kill）的文本条目。连续的 kill 可累积进同一条目（accumulate）；
//! 支持 yank（取最近）与 yank-pop（rotate 循环旧条目）。

/// kill/yank 环形缓冲。
#[derive(Default)]
pub struct KillRing {
    ring: Vec<String>,
}

impl KillRing {
    pub fn new() -> Self {
        Self::default()
    }

    /// 把 `text` 加入 kill-ring。
    ///
    /// - `prepend`：accumulate 时是前插（向后删除）还是后接（向前删除）。
    /// - `accumulate`：与最近条目合并而非新建。
    pub fn push(&mut self, text: &str, prepend: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate && !self.ring.is_empty() {
            let last = self.ring.pop().unwrap();
            let merged = if prepend {
                format!("{text}{last}")
            } else {
                format!("{last}{text}")
            };
            self.ring.push(merged);
        } else {
            self.ring.push(text.to_string());
        }
    }

    /// 取最近条目（不修改环）。
    pub fn peek(&self) -> Option<&str> {
        self.ring.last().map(|s| s.as_str())
    }

    /// 把末尾条目移到最前（yank-pop 循环）。
    pub fn rotate(&mut self) {
        if self.ring.len() > 1 {
            let last = self.ring.pop().unwrap();
            self.ring.insert(0, last);
        }
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_peek() {
        let mut kr = KillRing::new();
        assert!(kr.is_empty());
        kr.push("hello", false, false);
        assert_eq!(kr.peek(), Some("hello"));
        assert_eq!(kr.len(), 1);
    }

    #[test]
    fn empty_text_ignored() {
        let mut kr = KillRing::new();
        kr.push("", false, false);
        assert_eq!(kr.len(), 0);
    }

    #[test]
    fn accumulate_append_forward() {
        let mut kr = KillRing::new();
        kr.push("foo", false, false);
        // forward deletion appends
        kr.push("bar", false, true);
        assert_eq!(kr.peek(), Some("foobar"));
        assert_eq!(kr.len(), 1);
    }

    #[test]
    fn accumulate_prepend_backward() {
        let mut kr = KillRing::new();
        kr.push("bar", true, false);
        // backward deletion prepends
        kr.push("foo", true, true);
        assert_eq!(kr.peek(), Some("foobar"));
        assert_eq!(kr.len(), 1);
    }

    #[test]
    fn no_accumulate_creates_new_entry() {
        let mut kr = KillRing::new();
        kr.push("a", false, false);
        kr.push("b", false, false);
        assert_eq!(kr.len(), 2);
        assert_eq!(kr.peek(), Some("b"));
    }

    #[test]
    fn rotate_cycles() {
        let mut kr = KillRing::new();
        kr.push("a", false, false);
        kr.push("b", false, false);
        kr.push("c", false, false);
        // ring = [a, b, c]; peek = c
        assert_eq!(kr.peek(), Some("c"));
        kr.rotate(); // -> [c, a, b]; peek = b
        assert_eq!(kr.peek(), Some("b"));
        kr.rotate(); // -> [b, c, a]; peek = a
        assert_eq!(kr.peek(), Some("a"));
    }

    #[test]
    fn rotate_noop_when_single() {
        let mut kr = KillRing::new();
        kr.push("only", false, false);
        kr.rotate();
        assert_eq!(kr.peek(), Some("only"));
    }
}
