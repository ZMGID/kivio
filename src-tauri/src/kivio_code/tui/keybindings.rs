//! Keybindings —— PI `keybindings.ts` 端口。
//!
//! 把动作 id（如 `"tui.editor.cursorUp"`、`"tui.input.submit"`）映射到一组按键标识，合并默认表
//! [`tui_keybindings`] 与用户覆盖，并检测用户绑定冲突。组件调用
//! `manager.matches(data, "tui.editor.deleteWordBackward", kitty_active)`。

use std::collections::HashMap;

use super::keys::matches_key;

/// 一条 keybinding 定义：默认按键 + 描述。
#[derive(Clone)]
pub struct KeybindingDefinition {
    pub default_keys: Vec<String>,
    pub description: String,
}

impl KeybindingDefinition {
    fn new(keys: &[&str], description: &str) -> Self {
        Self {
            default_keys: keys.iter().map(|s| s.to_string()).collect(),
            description: description.to_string(),
        }
    }
}

/// 默认 keybindings 表（编辑 / 输入 / 选择三类），对应 PI 的 `TUI_KEYBINDINGS`。
pub fn tui_keybindings() -> Vec<(&'static str, KeybindingDefinition)> {
    vec![
        ("tui.editor.cursorUp", KeybindingDefinition::new(&["up"], "Move cursor up")),
        ("tui.editor.cursorDown", KeybindingDefinition::new(&["down"], "Move cursor down")),
        ("tui.editor.cursorLeft", KeybindingDefinition::new(&["left", "ctrl+b"], "Move cursor left")),
        ("tui.editor.cursorRight", KeybindingDefinition::new(&["right", "ctrl+f"], "Move cursor right")),
        (
            "tui.editor.cursorWordLeft",
            KeybindingDefinition::new(&["alt+left", "ctrl+left", "alt+b"], "Move cursor word left"),
        ),
        (
            "tui.editor.cursorWordRight",
            KeybindingDefinition::new(&["alt+right", "ctrl+right", "alt+f"], "Move cursor word right"),
        ),
        ("tui.editor.cursorLineStart", KeybindingDefinition::new(&["home", "ctrl+a"], "Move to line start")),
        ("tui.editor.cursorLineEnd", KeybindingDefinition::new(&["end", "ctrl+e"], "Move to line end")),
        ("tui.editor.jumpForward", KeybindingDefinition::new(&["ctrl+]"], "Jump forward to character")),
        ("tui.editor.jumpBackward", KeybindingDefinition::new(&["ctrl+alt+]"], "Jump backward to character")),
        ("tui.editor.pageUp", KeybindingDefinition::new(&["pageUp"], "Page up")),
        ("tui.editor.pageDown", KeybindingDefinition::new(&["pageDown"], "Page down")),
        ("tui.editor.deleteCharBackward", KeybindingDefinition::new(&["backspace"], "Delete character backward")),
        (
            "tui.editor.deleteCharForward",
            KeybindingDefinition::new(&["delete", "ctrl+d"], "Delete character forward"),
        ),
        (
            "tui.editor.deleteWordBackward",
            KeybindingDefinition::new(&["ctrl+w", "alt+backspace"], "Delete word backward"),
        ),
        (
            "tui.editor.deleteWordForward",
            KeybindingDefinition::new(&["alt+d", "alt+delete"], "Delete word forward"),
        ),
        ("tui.editor.deleteToLineStart", KeybindingDefinition::new(&["ctrl+u"], "Delete to line start")),
        ("tui.editor.deleteToLineEnd", KeybindingDefinition::new(&["ctrl+k"], "Delete to line end")),
        ("tui.editor.yank", KeybindingDefinition::new(&["ctrl+y"], "Yank")),
        ("tui.editor.yankPop", KeybindingDefinition::new(&["alt+y"], "Yank pop")),
        ("tui.editor.undo", KeybindingDefinition::new(&["ctrl+-"], "Undo")),
        ("tui.input.newLine", KeybindingDefinition::new(&["shift+enter"], "Insert newline")),
        ("tui.input.submit", KeybindingDefinition::new(&["enter"], "Submit input")),
        ("tui.input.tab", KeybindingDefinition::new(&["tab"], "Tab / autocomplete")),
        ("tui.input.copy", KeybindingDefinition::new(&["ctrl+c"], "Copy selection")),
        ("tui.select.up", KeybindingDefinition::new(&["up"], "Move selection up")),
        ("tui.select.down", KeybindingDefinition::new(&["down"], "Move selection down")),
        ("tui.select.pageUp", KeybindingDefinition::new(&["pageUp"], "Selection page up")),
        ("tui.select.pageDown", KeybindingDefinition::new(&["pageDown"], "Selection page down")),
        ("tui.select.confirm", KeybindingDefinition::new(&["enter"], "Confirm selection")),
        ("tui.select.cancel", KeybindingDefinition::new(&["escape", "ctrl+c"], "Cancel selection")),
    ]
}

/// 一处用户绑定冲突：同一按键被多个动作绑定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingConflict {
    pub key: String,
    pub keybindings: Vec<String>,
}

fn normalize_keys(keys: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for k in keys {
        if seen.insert(k.clone()) {
            result.push(k.clone());
        }
    }
    result
}

/// keybinding 管理器：合并默认 + 用户覆盖，提供 [`matches`](KeybindingsManager::matches)。
pub struct KeybindingsManager {
    definitions: Vec<(String, KeybindingDefinition)>,
    user_bindings: HashMap<String, Vec<String>>,
    keys_by_id: HashMap<String, Vec<String>>,
    conflicts: Vec<KeybindingConflict>,
}

impl KeybindingsManager {
    /// 用给定定义表 + 用户覆盖构造。
    pub fn new(
        definitions: Vec<(&'static str, KeybindingDefinition)>,
        user_bindings: HashMap<String, Vec<String>>,
    ) -> Self {
        let mut mgr = Self {
            definitions: definitions.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            user_bindings,
            keys_by_id: HashMap::new(),
            conflicts: Vec::new(),
        };
        mgr.rebuild();
        mgr
    }

    /// 用默认表构造。
    pub fn with_defaults() -> Self {
        Self::new(tui_keybindings(), HashMap::new())
    }

    fn rebuild(&mut self) {
        self.keys_by_id.clear();
        self.conflicts.clear();

        let valid_ids: std::collections::HashSet<&str> =
            self.definitions.iter().map(|(id, _)| id.as_str()).collect();

        // 用户绑定冲突检测
        let mut user_claims: HashMap<String, std::collections::BTreeSet<String>> = HashMap::new();
        for (binding, keys) in &self.user_bindings {
            if !valid_ids.contains(binding.as_str()) {
                continue;
            }
            for key in normalize_keys(keys) {
                user_claims.entry(key).or_default().insert(binding.clone());
            }
        }
        for (key, bindings) in &user_claims {
            if bindings.len() > 1 {
                self.conflicts.push(KeybindingConflict {
                    key: key.clone(),
                    keybindings: bindings.iter().cloned().collect(),
                });
            }
        }

        for (id, def) in &self.definitions {
            let keys = match self.user_bindings.get(id) {
                Some(user_keys) => normalize_keys(user_keys),
                None => normalize_keys(&def.default_keys),
            };
            self.keys_by_id.insert(id.clone(), keys);
        }
    }

    /// 输入 `data` 是否匹配动作 `keybinding`。
    pub fn matches(&self, data: &str, keybinding: &str, kitty_active: bool) -> bool {
        if let Some(keys) = self.keys_by_id.get(keybinding) {
            keys.iter().any(|k| matches_key(data, k, kitty_active))
        } else {
            false
        }
    }

    /// 取某动作绑定的按键列表。
    pub fn get_keys(&self, keybinding: &str) -> Vec<String> {
        self.keys_by_id.get(keybinding).cloned().unwrap_or_default()
    }

    /// 当前用户绑定冲突列表。
    pub fn conflicts(&self) -> &[KeybindingConflict] {
        &self.conflicts
    }

    /// 替换用户绑定并重建。
    pub fn set_user_bindings(&mut self, user_bindings: HashMap<String, Vec<String>>) {
        self.user_bindings = user_bindings;
        self.rebuild();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match() {
        let m = KeybindingsManager::with_defaults();
        assert!(m.matches("\x03", "tui.input.copy", false)); // ctrl+c
        assert!(m.matches("\r", "tui.input.submit", false)); // enter
        assert!(m.matches("\x1b[A", "tui.select.up", false)); // up
        assert!(m.matches("\x1b", "tui.select.cancel", false)); // escape
        assert!(m.matches("\x03", "tui.select.cancel", false)); // ctrl+c also cancels
    }

    #[test]
    fn editor_word_delete() {
        let m = KeybindingsManager::with_defaults();
        assert!(m.matches("\x17", "tui.editor.deleteWordBackward", false)); // ctrl+w
        assert!(m.matches("\x1b\x7f", "tui.editor.deleteWordBackward", false)); // alt+backspace
    }

    #[test]
    fn multiple_keys_per_action() {
        let m = KeybindingsManager::with_defaults();
        // cursorLeft = left OR ctrl+b
        assert!(m.matches("\x1b[D", "tui.editor.cursorLeft", false));
        assert!(m.matches("\x02", "tui.editor.cursorLeft", false)); // ctrl+b
    }

    #[test]
    fn user_override_replaces_defaults() {
        let mut user = HashMap::new();
        user.insert("tui.input.submit".to_string(), vec!["ctrl+s".to_string()]);
        let m = KeybindingsManager::new(tui_keybindings(), user);
        assert!(m.matches("\x13", "tui.input.submit", false)); // ctrl+s
        assert!(!m.matches("\r", "tui.input.submit", false)); // enter no longer submits
    }

    #[test]
    fn conflict_detection() {
        let mut user = HashMap::new();
        user.insert("tui.input.submit".to_string(), vec!["ctrl+x".to_string()]);
        user.insert("tui.input.copy".to_string(), vec!["ctrl+x".to_string()]);
        let m = KeybindingsManager::new(tui_keybindings(), user);
        assert_eq!(m.conflicts().len(), 1);
        assert_eq!(m.conflicts()[0].key, "ctrl+x");
    }

    #[test]
    fn get_keys_returns_resolved() {
        let m = KeybindingsManager::with_defaults();
        assert_eq!(m.get_keys("tui.editor.cursorLeft"), vec!["left", "ctrl+b"]);
    }
}
