//! Autocomplete —— PI `autocomplete.ts` 的核心抽象端口（不含 `fd`/文件系统 walk，那属 CLI 集成层）。
//!
//! [`AutocompleteProvider`] 是 trait：`get_suggestions(lines, cursor_line, cursor_col)` 返回
//! `{items, prefix}`，`apply_completion(...)` 改写 lines+cursor。[`StaticAutocompleteProvider`] 用内存
//! slash 命令表 + fuzzy 过滤实现一个通用 provider（model/session/theme 选择器等可复用）。

use super::components::select_list::SelectItem;
use super::fuzzy::fuzzy_filter;

/// 一条补全项。
#[derive(Clone, Debug, PartialEq)]
pub struct AutocompleteItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

impl AutocompleteItem {
    pub fn new(
        value: impl Into<String>,
        label: impl Into<String>,
        description: Option<String>,
    ) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description,
        }
    }
}

impl From<AutocompleteItem> for SelectItem {
    fn from(a: AutocompleteItem) -> Self {
        SelectItem::new(a.value, a.label, a.description)
    }
}

/// 一次补全请求的结果。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutocompleteSuggestions {
    pub items: Vec<AutocompleteItem>,
    /// 触发补全的前缀（如 `/mod` 或 `@src/`）。`apply_completion` 据此替换。
    pub prefix: String,
}

/// 补全 provider。
pub trait AutocompleteProvider {
    /// 触发字符（如 `@`、`#`）。`/` slash 命令由 provider 内部处理。
    fn trigger_characters(&self) -> Vec<char> {
        Vec::new()
    }

    /// 取当前光标位置的补全建议。
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> AutocompleteSuggestions;

    /// 应用选中项，返回新的 (lines, cursor_line, cursor_col)。
    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        selected: &AutocompleteItem,
        prefix: &str,
    ) -> (Vec<String>, usize, usize);
}

/// 一条 slash 命令定义。
#[derive(Clone, Debug)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
}

impl SlashCommand {
    pub fn new(name: impl Into<String>, description: Option<String>) -> Self {
        Self {
            name: name.into(),
            description,
        }
    }
}

/// 通用内存 provider：slash 命令 fuzzy 过滤。
pub struct StaticAutocompleteProvider {
    commands: Vec<SlashCommand>,
}

impl StaticAutocompleteProvider {
    pub fn new(commands: Vec<SlashCommand>) -> Self {
        Self { commands }
    }

    /// 取光标前的文本（当前逻辑行的 `[..cursor_col]`）。
    fn text_before_cursor(lines: &[String], cursor_line: usize, cursor_col: usize) -> String {
        let line = lines.get(cursor_line).cloned().unwrap_or_default();
        line[..cursor_col.min(line.len())].to_string()
    }
}

impl AutocompleteProvider for StaticAutocompleteProvider {
    fn get_suggestions(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
    ) -> AutocompleteSuggestions {
        let before = Self::text_before_cursor(lines, cursor_line, cursor_col);
        // slash 命令仅在首行行首生效
        let trimmed = before.trim_start();
        if cursor_line != 0 || !trimmed.starts_with('/') {
            return AutocompleteSuggestions::default();
        }
        let query = trimmed.trim_start_matches('/');
        // 只取命令名部分（空格前），有参数则不再补全命令
        if query.contains(' ') {
            return AutocompleteSuggestions::default();
        }
        let filtered = fuzzy_filter(self.commands.clone(), query, |c| c.name.clone());
        let items: Vec<AutocompleteItem> = filtered
            .into_iter()
            .map(|c| {
                AutocompleteItem::new(
                    format!("/{}", c.name),
                    format!("/{}", c.name),
                    c.description,
                )
            })
            .collect();
        AutocompleteSuggestions {
            items,
            prefix: trimmed.to_string(),
        }
    }

    fn apply_completion(
        &self,
        lines: &[String],
        cursor_line: usize,
        cursor_col: usize,
        selected: &AutocompleteItem,
        prefix: &str,
    ) -> (Vec<String>, usize, usize) {
        let mut new_lines = lines.to_vec();
        let line = new_lines.get(cursor_line).cloned().unwrap_or_default();
        let before = &line[..cursor_col.min(line.len())];
        // 替换光标前最后 prefix.len() 字节为 selected.value
        let prefix_len = prefix.len().min(before.len());
        let replace_start = before.len() - prefix_len;
        let after = &line[cursor_col.min(line.len())..];
        let new_value = format!("{} ", selected.value); // 补全后追加空格
        let new_line = format!("{}{}{}", &before[..replace_start], new_value, after);
        let new_col = replace_start + new_value.len();
        new_lines[cursor_line] = new_line;
        (new_lines, cursor_line, new_col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> StaticAutocompleteProvider {
        StaticAutocompleteProvider::new(vec![
            SlashCommand::new("model", Some("pick model".into())),
            SlashCommand::new("compact", Some("compact context".into())),
            SlashCommand::new("commit", None),
            SlashCommand::new("new", None),
        ])
    }

    #[test]
    fn suggests_slash_commands() {
        let p = provider();
        let lines = vec!["/co".to_string()];
        let s = p.get_suggestions(&lines, 0, 3);
        assert!(!s.items.is_empty());
        // both "compact" and "commit" match "co"
        let names: Vec<&str> = s.items.iter().map(|i| i.value.as_str()).collect();
        assert!(names.contains(&"/compact"));
        assert!(names.contains(&"/commit"));
        assert!(!names.contains(&"/model"));
    }

    #[test]
    fn no_suggestions_off_first_line() {
        let p = provider();
        let lines = vec!["text".to_string(), "/mod".to_string()];
        let s = p.get_suggestions(&lines, 1, 4);
        assert!(s.items.is_empty());
    }

    #[test]
    fn no_suggestions_with_args() {
        let p = provider();
        let lines = vec!["/model gpt".to_string()];
        let s = p.get_suggestions(&lines, 0, 10);
        assert!(s.items.is_empty());
    }

    #[test]
    fn empty_query_lists_all() {
        let p = provider();
        let lines = vec!["/".to_string()];
        let s = p.get_suggestions(&lines, 0, 1);
        assert_eq!(s.items.len(), 4);
    }

    #[test]
    fn apply_completion_replaces_prefix() {
        let p = provider();
        let lines = vec!["/co".to_string()];
        let s = p.get_suggestions(&lines, 0, 3);
        let item = s
            .items
            .iter()
            .find(|i| i.value == "/compact")
            .unwrap()
            .clone();
        let (new_lines, _l, col) = p.apply_completion(&lines, 0, 3, &item, &s.prefix);
        assert_eq!(new_lines[0], "/compact ");
        assert_eq!(col, "/compact ".len());
    }

    #[test]
    fn suggestions_ordered_by_fuzzy() {
        let p = provider();
        let lines = vec!["/mod".to_string()];
        let s = p.get_suggestions(&lines, 0, 4);
        assert_eq!(s.items[0].value, "/model");
    }
}
