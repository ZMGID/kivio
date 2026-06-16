//! Text / BoxView / Spacer / TruncatedText —— PI `components/{text,box,spacer,truncated-text}.ts` 端口。
//!
//! 注意：PI 的 `Box` **不画边框**，它是一个「给所有子组件加 padding + 背景色」的容器；本端口
//! 的 [`BoxView`] 与之一致（任务简述里写的 "Box (border)" 与 PI 实际语义不符，这里以对齐 PI 为准）。

use super::super::render::Component;
use super::super::text_width::{apply_background_to_line, truncate_to_width, visible_width, wrap_text_with_ansi};
use super::ColorFn;

/// 多行文本组件，word-wrap + 可选水平/垂直 padding + 可选背景色，按 (text,width) 缓存。
pub struct Text {
    text: String,
    padding_x: usize,
    padding_y: usize,
    bg_fn: Option<ColorFn>,
    cached: Option<(String, u16, Vec<String>)>,
}

impl Text {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize, bg_fn: Option<ColorFn>) -> Self {
        Self { text: text.into(), padding_x, padding_y, bg_fn, cached: None }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cached = None;
    }

    pub fn set_bg_fn(&mut self, bg_fn: Option<ColorFn>) {
        self.bg_fn = bg_fn;
        self.cached = None;
    }
}

impl Component for Text {
    fn render(&mut self, width: u16) -> Vec<String> {
        if let Some((ct, cw, cl)) = &self.cached {
            if ct == &self.text && *cw == width {
                return cl.clone();
            }
        }
        if self.text.trim().is_empty() {
            let result: Vec<String> = Vec::new();
            self.cached = Some((self.text.clone(), width, result.clone()));
            return result;
        }
        let w = width as usize;
        let normalized = self.text.replace('\t', "   ");
        let content_width = w.saturating_sub(self.padding_x * 2).max(1);
        let wrapped = wrap_text_with_ansi(&normalized, content_width);

        let margin = " ".repeat(self.padding_x);
        let mut content_lines: Vec<String> = Vec::new();
        for line in &wrapped {
            let with_margins = format!("{margin}{line}{margin}");
            if let Some(bg) = &self.bg_fn {
                content_lines.push(apply_background_to_line(&with_margins, w, &**bg));
            } else {
                let vis = visible_width(&with_margins);
                let pad = w.saturating_sub(vis);
                content_lines.push(format!("{with_margins}{}", " ".repeat(pad)));
            }
        }

        let empty_line = " ".repeat(w);
        let make_empty = || -> String {
            if let Some(bg) = &self.bg_fn {
                apply_background_to_line(&empty_line, w, &**bg)
            } else {
                empty_line.clone()
            }
        };

        let mut result: Vec<String> = Vec::new();
        for _ in 0..self.padding_y {
            result.push(make_empty());
        }
        result.extend(content_lines);
        for _ in 0..self.padding_y {
            result.push(make_empty());
        }
        if result.is_empty() {
            result.push(String::new());
        }
        self.cached = Some((self.text.clone(), width, result.clone()));
        result
    }

    fn invalidate(&mut self) {
        self.cached = None;
    }
}

/// 渲染 N 行空行的占位组件。
pub struct Spacer {
    lines: usize,
}

impl Spacer {
    pub fn new(lines: usize) -> Self {
        Self { lines }
    }
    pub fn set_lines(&mut self, lines: usize) {
        self.lines = lines;
    }
}

impl Component for Spacer {
    fn render(&mut self, _width: u16) -> Vec<String> {
        vec![String::new(); self.lines]
    }
}

/// 单行截断文本（取首行、`truncate_to_width`、padding 到宽度）。
pub struct TruncatedText {
    text: String,
    padding_x: usize,
    padding_y: usize,
}

impl TruncatedText {
    pub fn new(text: impl Into<String>, padding_x: usize, padding_y: usize) -> Self {
        Self { text: text.into(), padding_x, padding_y }
    }
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }
}

impl Component for TruncatedText {
    fn render(&mut self, width: u16) -> Vec<String> {
        let w = width as usize;
        let empty_line = " ".repeat(w);
        let mut result: Vec<String> = Vec::new();
        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }
        let available = w.saturating_sub(self.padding_x * 2).max(1);
        let single = match self.text.find('\n') {
            Some(idx) => &self.text[..idx],
            None => &self.text,
        };
        let display = truncate_to_width(single, available, "...", false);
        let pad = " ".repeat(self.padding_x);
        let with_padding = format!("{pad}{display}{pad}");
        let vis = visible_width(&with_padding);
        let pad_needed = w.saturating_sub(vis);
        result.push(format!("{with_padding}{}", " ".repeat(pad_needed)));
        for _ in 0..self.padding_y {
            result.push(empty_line.clone());
        }
        result
    }
}

/// 给所有子组件加 padding + 背景色的容器（**不画边框**，对齐 PI 的 `Box`）。
pub struct BoxView {
    children: Vec<Box<dyn Component>>,
    padding_x: usize,
    padding_y: usize,
    bg_fn: Option<ColorFn>,
}

impl BoxView {
    pub fn new(padding_x: usize, padding_y: usize, bg_fn: Option<ColorFn>) -> Self {
        Self { children: Vec::new(), padding_x, padding_y, bg_fn }
    }
    pub fn add_child(&mut self, c: Box<dyn Component>) {
        self.children.push(c);
    }
    pub fn clear(&mut self) {
        self.children.clear();
    }

    fn apply_bg(&self, line: &str, width: usize) -> String {
        let vis = visible_width(line);
        let pad = width.saturating_sub(vis);
        let padded = format!("{line}{}", " ".repeat(pad));
        if let Some(bg) = &self.bg_fn {
            apply_background_to_line(&padded, width, &**bg)
        } else {
            padded
        }
    }
}

impl Component for BoxView {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.children.is_empty() {
            return Vec::new();
        }
        let w = width as usize;
        let content_width = w.saturating_sub(self.padding_x * 2).max(1);
        let left_pad = " ".repeat(self.padding_x);

        let mut child_lines: Vec<String> = Vec::new();
        for child in &mut self.children {
            for line in child.render(content_width as u16) {
                child_lines.push(format!("{left_pad}{line}"));
            }
        }
        if child_lines.is_empty() {
            return Vec::new();
        }

        let mut result: Vec<String> = Vec::new();
        for _ in 0..self.padding_y {
            result.push(self.apply_bg("", w));
        }
        for line in &child_lines {
            result.push(self.apply_bg(line, w));
        }
        for _ in 0..self.padding_y {
            result.push(self.apply_bg("", w));
        }
        result
    }

    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn text_wraps_and_pads() {
        let mut t = Text::new("hello world", 1, 0, None);
        let lines = t.render(9);
        // content width = 9 - 2 = 7; "hello" fits, "world" wraps
        for l in &lines {
            assert_eq!(visible_width(l), 9, "line not padded to width: {l:?}");
        }
        assert!(lines[0].contains("hello"));
    }

    #[test]
    fn text_vertical_padding() {
        let mut t = Text::new("hi", 0, 2, None);
        let lines = t.render(10);
        // 2 empty + 1 content + 2 empty = 5
        assert_eq!(lines.len(), 5);
        assert_eq!(visible_width(&lines[0]), 10);
    }

    #[test]
    fn text_empty_renders_nothing() {
        let mut t = Text::new("   ", 1, 1, None);
        assert!(t.render(20).is_empty());
    }

    #[test]
    fn text_caches() {
        let mut t = Text::new("abc", 0, 0, None);
        let a = t.render(10);
        let b = t.render(10);
        assert_eq!(a, b);
    }

    #[test]
    fn text_with_bg_fn() {
        let bg: ColorFn = Arc::new(|s: &str| format!("<{s}>"));
        let mut t = Text::new("x", 0, 0, Some(bg));
        let lines = t.render(5);
        assert_eq!(lines[0], "<x    >");
    }

    #[test]
    fn spacer_emits_empty_lines() {
        let mut s = Spacer::new(3);
        assert_eq!(s.render(80), vec!["", "", ""]);
    }

    #[test]
    fn truncated_takes_first_line_and_pads() {
        let mut t = TruncatedText::new("line one\nline two", 0, 0);
        let lines = t.render(20);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("line one"));
        assert!(!lines[0].contains("line two"));
        assert_eq!(visible_width(&lines[0]), 20);
    }

    #[test]
    fn truncated_adds_ellipsis_when_too_long() {
        let mut t = TruncatedText::new("this is a very long line", 0, 0);
        let lines = t.render(10);
        assert_eq!(visible_width(&lines[0]), 10);
        assert!(lines[0].contains("..."));
    }

    #[test]
    fn boxview_pads_children() {
        let mut b = BoxView::new(2, 1, None);
        b.add_child(Box::new(Text::new("hi", 0, 0, None)));
        let lines = b.render(10);
        // 1 top pad + content + 1 bottom pad
        assert!(lines.len() >= 3);
        for l in &lines {
            assert_eq!(visible_width(l), 10);
        }
        // content line should have 2-space left pad
        let content = lines.iter().find(|l| l.contains("hi")).unwrap();
        assert!(content.starts_with("  "));
    }

    #[test]
    fn boxview_empty_when_no_children() {
        let mut b = BoxView::new(1, 1, None);
        assert!(b.render(10).is_empty());
    }
}
