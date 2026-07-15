//! SelectList —— PI `components/select-list.ts` 端口。
//!
//! 可滚动、可过滤的选择列表：滚动窗口（居中选中项）、`→ ` 选中前缀、两列 label/description
//! 布局（计算 primary 列宽）、`(i/n)` 滚动信息、首尾环绕。主题是 5 个 [`ColorFn`]。

use super::super::keybindings::KeybindingsManager;
use super::super::text_width::{truncate_to_width, visible_width};
use super::ColorFn;

const DEFAULT_PRIMARY_COLUMN_WIDTH: usize = 32;
const PRIMARY_COLUMN_GAP: usize = 2;
const MIN_DESCRIPTION_WIDTH: usize = 10;

fn normalize_to_single_line(text: &str) -> String {
    let collapsed = text
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect::<String>();
    // collapse runs of spaces that came from newlines, then trim
    let mut out = String::with_capacity(collapsed.len());
    let mut prev_space = false;
    for c in collapsed.chars() {
        if c == ' ' {
            if !prev_space {
                out.push(c);
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn clamp(v: usize, lo: usize, hi: usize) -> usize {
    v.max(lo).min(hi)
}

/// 列表项。
#[derive(Clone, Debug, PartialEq)]
pub struct SelectItem {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

impl SelectItem {
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

/// 5 个着色函数。
pub struct SelectListTheme {
    pub selected_prefix: ColorFn,
    pub selected_text: ColorFn,
    pub description: ColorFn,
    pub scroll_info: ColorFn,
    pub no_match: ColorFn,
}

/// primary 列宽布局选项。
#[derive(Default, Clone)]
pub struct SelectListLayoutOptions {
    pub min_primary_column_width: Option<usize>,
    pub max_primary_column_width: Option<usize>,
}

/// 可滚动可过滤的选择列表。
pub struct SelectList {
    items: Vec<SelectItem>,
    filtered_items: Vec<SelectItem>,
    selected_index: usize,
    max_visible: usize,
    theme: SelectListTheme,
    layout: SelectListLayoutOptions,
    kb: KeybindingsManager,
    kitty_active: bool,

    pub on_select: Option<Box<dyn FnMut(SelectItem) + Send>>,
    pub on_cancel: Option<Box<dyn FnMut() + Send>>,
    pub on_selection_change: Option<Box<dyn FnMut(SelectItem) + Send>>,
}

impl SelectList {
    pub fn new(
        items: Vec<SelectItem>,
        max_visible: usize,
        theme: SelectListTheme,
        layout: SelectListLayoutOptions,
    ) -> Self {
        Self {
            filtered_items: items.clone(),
            items,
            selected_index: 0,
            max_visible: max_visible.max(1),
            theme,
            layout,
            kb: KeybindingsManager::with_defaults(),
            kitty_active: false,
            on_select: None,
            on_cancel: None,
            on_selection_change: None,
        }
    }

    pub fn set_kitty_active(&mut self, active: bool) {
        self.kitty_active = active;
    }

    /// 用 value 前缀过滤（PI 语义：`value.startsWith(filter)`）。
    pub fn set_filter(&mut self, filter: &str) {
        let f = filter.to_lowercase();
        self.filtered_items = self
            .items
            .iter()
            .filter(|i| i.value.to_lowercase().starts_with(&f))
            .cloned()
            .collect();
        self.selected_index = 0;
    }

    /// 直接替换过滤后的列表（供外部 fuzzy 过滤注入）。
    pub fn set_filtered_items(&mut self, items: Vec<SelectItem>) {
        self.filtered_items = items;
        self.selected_index = 0;
    }

    pub fn set_selected_index(&mut self, index: usize) {
        if self.filtered_items.is_empty() {
            self.selected_index = 0;
        } else {
            self.selected_index = index.min(self.filtered_items.len() - 1);
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected_index
    }

    pub fn get_selected_item(&self) -> Option<SelectItem> {
        self.filtered_items.get(self.selected_index).cloned()
    }

    #[cfg(test)]
    pub fn filtered_len(&self) -> usize {
        self.filtered_items.len()
    }

    fn notify_selection_change(&mut self) {
        if let Some(item) = self.filtered_items.get(self.selected_index).cloned() {
            if let Some(cb) = self.on_selection_change.as_mut() {
                cb(item);
            }
        }
    }

    fn display_value(item: &SelectItem) -> &str {
        if item.label.is_empty() {
            &item.value
        } else {
            &item.label
        }
    }

    fn primary_column_bounds(&self) -> (usize, usize) {
        let raw_min = self
            .layout
            .min_primary_column_width
            .or(self.layout.max_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        let raw_max = self
            .layout
            .max_primary_column_width
            .or(self.layout.min_primary_column_width)
            .unwrap_or(DEFAULT_PRIMARY_COLUMN_WIDTH);
        (1.max(raw_min.min(raw_max)), 1.max(raw_min.max(raw_max)))
    }

    fn primary_column_width(&self) -> usize {
        let (min, max) = self.primary_column_bounds();
        let widest = self
            .filtered_items
            .iter()
            .map(|i| visible_width(Self::display_value(i)) + PRIMARY_COLUMN_GAP)
            .max()
            .unwrap_or(0);
        clamp(widest, min, max)
    }

    fn render_item(
        &self,
        item: &SelectItem,
        is_selected: bool,
        width: usize,
        desc: Option<&str>,
        primary_w: usize,
    ) -> String {
        let prefix = if is_selected { "→ " } else { "  " };
        let prefix_w = visible_width(prefix);

        if let Some(desc) = desc {
            if width > 40 {
                let eff_primary = 1.max(primary_w.min(width.saturating_sub(prefix_w + 4)));
                let max_primary = 1.max(eff_primary.saturating_sub(PRIMARY_COLUMN_GAP));
                let truncated_value =
                    truncate_to_width(Self::display_value(item), max_primary, "", false);
                let tv_w = visible_width(&truncated_value);
                let spacing = " ".repeat(1.max(eff_primary.saturating_sub(tv_w)));
                let desc_start = prefix_w + tv_w + spacing.len();
                let remaining = width.saturating_sub(desc_start + 2);
                if remaining > MIN_DESCRIPTION_WIDTH {
                    let truncated_desc = truncate_to_width(desc, remaining, "", false);
                    if is_selected {
                        return (self.theme.selected_text)(&format!(
                            "{prefix}{truncated_value}{spacing}{truncated_desc}"
                        ));
                    }
                    let desc_text = (self.theme.description)(&format!("{spacing}{truncated_desc}"));
                    return format!("{prefix}{truncated_value}{desc_text}");
                }
            }
        }

        let max_w = width.saturating_sub(prefix_w + 2);
        let truncated_value = truncate_to_width(Self::display_value(item), max_w, "", false);
        if is_selected {
            (self.theme.selected_text)(&format!("{prefix}{truncated_value}"))
        } else {
            format!("{prefix}{truncated_value}")
        }
    }
}

impl Component for SelectList {
    fn handle_input(&mut self, data: &str) {
        let kitty = self.kitty_active;
        if self.filtered_items.is_empty() {
            if self.kb.matches(data, "tui.select.cancel", kitty) {
                if let Some(cb) = self.on_cancel.as_mut() {
                    cb();
                }
            }
            return;
        }
        let n = self.filtered_items.len();
        if self.kb.matches(data, "tui.select.up", kitty) {
            self.selected_index = if self.selected_index == 0 {
                n - 1
            } else {
                self.selected_index - 1
            };
            self.notify_selection_change();
        } else if self.kb.matches(data, "tui.select.down", kitty) {
            self.selected_index = if self.selected_index == n - 1 {
                0
            } else {
                self.selected_index + 1
            };
            self.notify_selection_change();
        } else if self.kb.matches(data, "tui.select.confirm", kitty) {
            if let Some(item) = self.filtered_items.get(self.selected_index).cloned() {
                if let Some(cb) = self.on_select.as_mut() {
                    cb(item);
                }
            }
        } else if self.kb.matches(data, "tui.select.cancel", kitty) {
            if let Some(cb) = self.on_cancel.as_mut() {
                cb();
            }
        }
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let mut lines: Vec<String> = Vec::new();

        if self.filtered_items.is_empty() {
            lines.push((self.theme.no_match)("  No matching commands"));
            return lines;
        }

        let primary_w = self.primary_column_width();
        let n = self.filtered_items.len();
        let start = self
            .selected_index
            .saturating_sub(self.max_visible / 2)
            .min(n.saturating_sub(self.max_visible));
        let end = (start + self.max_visible).min(n);

        for i in start..end {
            let item = self.filtered_items[i].clone();
            let is_selected = i == self.selected_index;
            let desc = item
                .description
                .as_ref()
                .map(|d| normalize_to_single_line(d));
            lines.push(self.render_item(&item, is_selected, width, desc.as_deref(), primary_w));
        }

        if start > 0 || end < n {
            let scroll_text = format!("  ({}/{})", self.selected_index + 1, n);
            lines.push((self.theme.scroll_info)(&truncate_to_width(
                &scroll_text,
                width.saturating_sub(2),
                "",
                false,
            )));
        }

        lines
    }
}

use super::super::render::Component;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn identity_theme() -> SelectListTheme {
        let id: ColorFn = Arc::new(|s: &str| s.to_string());
        SelectListTheme {
            selected_prefix: id.clone(),
            selected_text: id.clone(),
            description: id.clone(),
            scroll_info: id.clone(),
            no_match: id,
        }
    }

    fn items(n: usize) -> Vec<SelectItem> {
        (0..n)
            .map(|i| SelectItem::new(format!("v{i}"), format!("label{i}"), None))
            .collect()
    }

    fn list(n: usize, max_visible: usize) -> SelectList {
        SelectList::new(
            items(n),
            max_visible,
            identity_theme(),
            SelectListLayoutOptions::default(),
        )
    }

    #[test]
    fn navigation_down_up_wraps() {
        let mut l = list(3, 5);
        assert_eq!(l.selected_index(), 0);
        l.handle_input("\x1b[B"); // down
        assert_eq!(l.selected_index(), 1);
        l.handle_input("\x1b[A"); // up
        assert_eq!(l.selected_index(), 0);
        l.handle_input("\x1b[A"); // up wraps to bottom
        assert_eq!(l.selected_index(), 2);
        l.handle_input("\x1b[B"); // down wraps to top
        assert_eq!(l.selected_index(), 0);
    }

    #[test]
    fn filter_by_prefix() {
        let mut l = SelectList::new(
            vec![
                SelectItem::new("apple", "Apple", None),
                SelectItem::new("apricot", "Apricot", None),
                SelectItem::new("banana", "Banana", None),
            ],
            5,
            identity_theme(),
            SelectListLayoutOptions::default(),
        );
        l.set_filter("ap");
        assert_eq!(l.filtered_len(), 2);
    }

    #[test]
    fn no_match_message() {
        let mut l = list(3, 5);
        l.set_filter("zzz");
        let lines = l.render(60);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("No matching"));
    }

    #[test]
    fn scroll_info_shown_when_overflow() {
        let mut l = list(10, 3);
        let lines = l.render(60);
        // 3 visible + 1 scroll-info line
        assert_eq!(lines.len(), 4);
        assert!(lines[3].contains("(1/10)"));
    }

    #[test]
    fn selected_prefix_arrow() {
        let mut l = list(3, 5);
        let lines = l.render(60);
        assert!(lines[0].starts_with("→ "));
        assert!(lines[1].starts_with("  "));
    }

    #[test]
    fn confirm_fires_on_select() {
        use std::sync::Mutex;
        let captured = Arc::new(Mutex::new(String::new()));
        let c2 = captured.clone();
        let mut l = list(3, 5);
        l.on_select = Some(Box::new(move |item| *c2.lock().unwrap() = item.value));
        l.handle_input("\r");
        assert_eq!(*captured.lock().unwrap(), "v0");
    }

    #[test]
    fn two_column_description_layout() {
        let mut l = SelectList::new(
            vec![SelectItem::new("cmd", "cmd", Some("does a thing".into()))],
            5,
            identity_theme(),
            SelectListLayoutOptions::default(),
        );
        let lines = l.render(80);
        assert!(lines[0].contains("cmd"));
        assert!(lines[0].contains("does a thing"));
    }
}
