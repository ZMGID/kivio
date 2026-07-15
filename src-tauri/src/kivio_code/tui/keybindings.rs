//! Keybindings —— PI `keybindings.ts` 端口。
//!
//! 把动作 id（如 `"tui.editor.cursorUp"`、`"tui.input.submit"`）映射到一组按键标识（默认表
//! [`tui_keybindings`]）。组件调用
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
        (
            "tui.editor.cursorUp",
            KeybindingDefinition::new(&["up"], "Move cursor up"),
        ),
        (
            "tui.editor.cursorDown",
            KeybindingDefinition::new(&["down"], "Move cursor down"),
        ),
        (
            "tui.editor.cursorLeft",
            KeybindingDefinition::new(&["left", "ctrl+b"], "Move cursor left"),
        ),
        (
            "tui.editor.cursorRight",
            KeybindingDefinition::new(&["right", "ctrl+f"], "Move cursor right"),
        ),
        (
            "tui.editor.cursorWordLeft",
            KeybindingDefinition::new(&["alt+left", "ctrl+left", "alt+b"], "Move cursor word left"),
        ),
        (
            "tui.editor.cursorWordRight",
            KeybindingDefinition::new(
                &["alt+right", "ctrl+right", "alt+f"],
                "Move cursor word right",
            ),
        ),
        (
            "tui.editor.cursorLineStart",
            KeybindingDefinition::new(&["home", "ctrl+a"], "Move to line start"),
        ),
        (
            "tui.editor.cursorLineEnd",
            KeybindingDefinition::new(&["end", "ctrl+e"], "Move to line end"),
        ),
        (
            "tui.editor.jumpForward",
            KeybindingDefinition::new(&["ctrl+]"], "Jump forward to character"),
        ),
        (
            "tui.editor.jumpBackward",
            KeybindingDefinition::new(&["ctrl+alt+]"], "Jump backward to character"),
        ),
        (
            "tui.editor.pageUp",
            KeybindingDefinition::new(&["pageUp"], "Page up"),
        ),
        (
            "tui.editor.pageDown",
            KeybindingDefinition::new(&["pageDown"], "Page down"),
        ),
        (
            "tui.editor.deleteCharBackward",
            KeybindingDefinition::new(&["backspace"], "Delete character backward"),
        ),
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
        (
            "tui.editor.deleteToLineStart",
            KeybindingDefinition::new(&["ctrl+u"], "Delete to line start"),
        ),
        (
            "tui.editor.deleteToLineEnd",
            KeybindingDefinition::new(&["ctrl+k"], "Delete to line end"),
        ),
        (
            "tui.editor.yank",
            KeybindingDefinition::new(&["ctrl+y"], "Yank"),
        ),
        (
            "tui.editor.yankPop",
            KeybindingDefinition::new(&["alt+y"], "Yank pop"),
        ),
        (
            "tui.editor.undo",
            KeybindingDefinition::new(&["ctrl+-"], "Undo"),
        ),
        (
            "tui.input.newLine",
            KeybindingDefinition::new(&["shift+enter"], "Insert newline"),
        ),
        (
            "tui.input.submit",
            KeybindingDefinition::new(&["enter"], "Submit input"),
        ),
        (
            "tui.input.tab",
            KeybindingDefinition::new(&["tab"], "Tab / autocomplete"),
        ),
        (
            "tui.input.copy",
            KeybindingDefinition::new(&["ctrl+c"], "Copy selection"),
        ),
        (
            "tui.select.up",
            KeybindingDefinition::new(&["up"], "Move selection up"),
        ),
        (
            "tui.select.down",
            KeybindingDefinition::new(&["down"], "Move selection down"),
        ),
        (
            "tui.select.pageUp",
            KeybindingDefinition::new(&["pageUp"], "Selection page up"),
        ),
        (
            "tui.select.pageDown",
            KeybindingDefinition::new(&["pageDown"], "Selection page down"),
        ),
        (
            "tui.select.confirm",
            KeybindingDefinition::new(&["enter"], "Confirm selection"),
        ),
        (
            "tui.select.cancel",
            KeybindingDefinition::new(&["escape", "ctrl+c"], "Cancel selection"),
        ),
    ]
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

/// keybinding 管理器：从默认表解析按键，提供 [`matches`](KeybindingsManager::matches)。
pub struct KeybindingsManager {
    keys_by_id: HashMap<String, Vec<String>>,
}

impl KeybindingsManager {
    /// 用默认表构造。
    pub fn with_defaults() -> Self {
        let mut keys_by_id = HashMap::new();
        for (id, def) in tui_keybindings() {
            keys_by_id.insert(id.to_string(), normalize_keys(&def.default_keys));
        }
        Self { keys_by_id }
    }

    /// 输入 `data` 是否匹配动作 `keybinding`。
    pub fn matches(&self, data: &str, keybinding: &str, kitty_active: bool) -> bool {
        if let Some(keys) = self.keys_by_id.get(keybinding) {
            keys.iter().any(|k| matches_key(data, k, kitty_active))
        } else {
            false
        }
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
}
