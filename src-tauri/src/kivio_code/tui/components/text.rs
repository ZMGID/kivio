//! Text / Spacer —— PI `components/{text,spacer}.ts` 端口。

use super::super::render::Component;
use super::super::text_width::{apply_background_to_line, visible_width, wrap_text_with_ansi};
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
    pub fn new(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        bg_fn: Option<ColorFn>,
    ) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            bg_fn,
            cached: None,
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
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
}
