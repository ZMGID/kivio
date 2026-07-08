//! 泛型 undo 栈 —— PI `undo-stack.ts` 端口。
//!
//! 存放 state 快照（克隆入栈）。pop 直接返回快照（已脱离）。Rust 端用 `Clone` 替代 JS 的
//! `structuredClone`。

/// 克隆入栈语义的 undo 栈。
pub struct UndoStack<S: Clone> {
    stack: Vec<S>,
}

impl<S: Clone> Default for UndoStack<S> {
    fn default() -> Self {
        Self { stack: Vec::new() }
    }
}

impl<S: Clone> UndoStack<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 把 `state` 的克隆压栈。
    pub fn push(&mut self, state: &S) {
        self.stack.push(state.clone());
    }

    /// 弹出最近快照（已脱离，可直接所有权返回）。
    pub fn pop(&mut self) -> Option<S> {
        self.stack.pop()
    }

    /// 清空全部快照。
    pub fn clear(&mut self) {
        self.stack.clear();
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct State {
        value: String,
        cursor: usize,
    }

    #[test]
    fn push_pop_roundtrip() {
        let mut s = UndoStack::new();
        let a = State {
            value: "a".into(),
            cursor: 1,
        };
        let b = State {
            value: "ab".into(),
            cursor: 2,
        };
        s.push(&a);
        s.push(&b);
        assert_eq!(s.len(), 2);
        assert_eq!(s.pop(), Some(b));
        assert_eq!(s.pop(), Some(a));
        assert_eq!(s.pop(), None);
    }

    #[test]
    fn push_clones_independent_of_caller() {
        let mut s = UndoStack::new();
        let mut state = State {
            value: "x".into(),
            cursor: 0,
        };
        s.push(&state);
        // mutate caller's copy after push
        state.value.push_str("yz");
        state.cursor = 9;
        // snapshot must be unchanged
        assert_eq!(
            s.pop(),
            Some(State {
                value: "x".into(),
                cursor: 0
            })
        );
    }

    #[test]
    fn clear_empties() {
        let mut s = UndoStack::new();
        s.push(&State {
            value: "a".into(),
            cursor: 0,
        });
        s.push(&State {
            value: "b".into(),
            cursor: 0,
        });
        s.clear();
        assert!(s.is_empty());
        assert_eq!(s.pop(), None);
    }
}
