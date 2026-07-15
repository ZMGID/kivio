//! Markdown —— PI `components/markdown.ts` 端口。
//!
//! 把 Markdown 解析（[`pulldown_cmark`]）并渲染成 ANSI 行数组，对接 [`Component`] 差分渲染器。
//! 在交互模式里用于渲染助手消息。覆盖：标题（按级别样式）、**粗体** / *斜体* / `行内代码` /
//! ~~删除线~~、有序 / 无序列表（嵌套缩进，条目间无空行）、引用块、围栏代码块（syntect 语法
//! 高亮）、水平分割线、链接（文本 + dim URL）、表格。
//!
//! 与 PI 的对齐方式：PI 用 `marked` 的 token 树递归（block + inline）渲染；pulldown-cmark 是
//! 事件流（`Start`/`End`/`Text`/...），所以这里用一个**栈式状态机**把事件折叠回与 PI 等价的
//! 行数组。所有样式都走 [`MarkdownTheme`] 的 [`ColorFn`]，不硬编码裸转义（除空字符串占位）。
//!
//! 代码块语法高亮：用 [`syntect`] 的默认 syntax set + 一个终端主题，把 span 映射成 24-bit
//! ANSI。syntect 的 syntax/theme set 加载有成本，放在 [`OnceLock`] 里惰性初始化。未知语言 →
//! 回退为不高亮的纯代码（仍走 `code_block` ColorFn）。

use std::sync::OnceLock;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use super::super::render::Component;
use super::super::text_width::{apply_background_to_line, visible_width, wrap_text_with_ansi};
use super::ColorFn;

/// Markdown 各元素的着色函数。每个接收文本返回带 ANSI 的文本。对应 PI 的 `MarkdownTheme`。
pub struct MarkdownTheme {
    pub heading: ColorFn,
    pub link: ColorFn,
    pub link_url: ColorFn,
    pub code: ColorFn,
    pub code_block: ColorFn,
    pub code_block_border: ColorFn,
    pub quote: ColorFn,
    pub quote_border: ColorFn,
    pub hr: ColorFn,
    pub list_bullet: ColorFn,
    pub bold: ColorFn,
    pub italic: ColorFn,
    pub strikethrough: ColorFn,
    pub underline: ColorFn,
    /// 代码块每行前缀（默认 "  "）。
    pub code_block_indent: String,
    /// 是否对代码块做 syntect 语法高亮（默认 true）。
    pub highlight_code: bool,
}

impl MarkdownTheme {
    /// 一个 ANSI dim/plain 的默认主题，便于无主题场景与测试使用。
    pub fn plain() -> Self {
        fn dim(s: &str) -> String {
            format!("\x1b[2m{s}\x1b[22m")
        }
        Self {
            heading: arc(|s| format!("\x1b[1m{s}\x1b[22m")),
            link: arc(|s| format!("\x1b[34m{s}\x1b[39m")),
            link_url: arc(dim),
            code: arc(|s| format!("\x1b[36m{s}\x1b[39m")),
            code_block: arc(|s| s.to_string()),
            code_block_border: arc(dim),
            quote: arc(dim),
            quote_border: arc(dim),
            hr: arc(dim),
            list_bullet: arc(|s| format!("\x1b[33m{s}\x1b[39m")),
            bold: arc(|s| format!("\x1b[1m{s}\x1b[22m")),
            italic: arc(|s| format!("\x1b[3m{s}\x1b[23m")),
            strikethrough: arc(|s| format!("\x1b[9m{s}\x1b[29m")),
            underline: arc(|s| format!("\x1b[4m{s}\x1b[24m")),
            code_block_indent: "  ".to_string(),
            highlight_code: true,
        }
    }
}

fn arc(f: impl Fn(&str) -> String + Send + Sync + 'static) -> ColorFn {
    std::sync::Arc::new(f)
}

// =============================================================================
// syntect 惰性初始化
// =============================================================================

struct HighlightAssets {
    syntaxes: SyntaxSet,
    theme: Theme,
}

static HIGHLIGHT_ASSETS: OnceLock<HighlightAssets> = OnceLock::new();

fn highlight_assets() -> &'static HighlightAssets {
    HIGHLIGHT_ASSETS.get_or_init(|| {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let theme_set = ThemeSet::load_defaults();
        // 终端友好的暗色主题。base16-ocean.dark 在 default-themes 里始终存在。
        let theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.values().next())
            .cloned()
            .unwrap_or_default();
        HighlightAssets { syntaxes, theme }
    })
}

/// 把一个代码块按语言高亮成多行（每行含 24-bit ANSI），未知语言返回 `None`（调用方回退纯文本）。
fn highlight_code_block(code: &str, lang: &str) -> Option<Vec<String>> {
    let assets = highlight_assets();
    let syntax = if lang.is_empty() {
        return None;
    } else {
        assets
            .syntaxes
            .find_syntax_by_token(lang)
            .or_else(|| assets.syntaxes.find_syntax_by_extension(lang))?
    };

    let mut highlighter = HighlightLines::new(syntax, &assets.theme);
    let mut out: Vec<String> = Vec::new();
    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, &assets.syntaxes).ok()?;
        out.push(spans_to_ansi(&ranges));
    }
    // 去掉尾随的空行（代码块文本通常以 \n 结束）。
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.pop();
    }
    Some(out)
}

/// 把 syntect span 映射成 24-bit 前景色 ANSI（行尾 reset）。去掉换行符。
fn spans_to_ansi(ranges: &[(SynStyle, &str)]) -> String {
    let mut out = String::new();
    for (style, text) in ranges {
        let text = text.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            continue;
        }
        let c = style.foreground;
        out.push_str(&format!("\x1b[38;2;{};{};{}m{text}\x1b[39m", c.r, c.g, c.b));
    }
    out
}

// =============================================================================
// 组件
// =============================================================================

/// 把 Markdown 渲染成 ANSI 行的组件，word-wrap + 可选 padding + 可选背景色，按 (text,width) 缓存。
pub struct Markdown {
    text: String,
    padding_x: usize,
    padding_y: usize,
    theme: MarkdownTheme,
    bg_fn: Option<ColorFn>,
    cached: Option<(String, u16, Vec<String>)>,
}

impl Markdown {
    pub fn new(
        text: impl Into<String>,
        padding_x: usize,
        padding_y: usize,
        theme: MarkdownTheme,
        bg_fn: Option<ColorFn>,
    ) -> Self {
        Self {
            text: text.into(),
            padding_x,
            padding_y,
            theme,
            bg_fn,
            cached: None,
        }
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cached = None;
    }

    /// 解析 + 渲染成「未 padding、未加背景」的行（每行可能含内嵌换行待 wrap）。
    fn render_blocks(&self, content_width: usize) -> Vec<String> {
        let normalized = self.text.replace('\t', "   ");
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(&normalized, opts);
        let mut renderer = BlockRenderer::new(&self.theme, content_width);
        for event in parser {
            renderer.handle(event);
        }
        renderer.finish()
    }
}

impl Component for Markdown {
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
        let content_width = w.saturating_sub(self.padding_x * 2).max(1);

        let rendered = self.render_blocks(content_width);

        // wrap（无 padding、无背景）
        let mut wrapped: Vec<String> = Vec::new();
        for line in &rendered {
            for wl in wrap_text_with_ansi(line, content_width) {
                wrapped.push(wl);
            }
        }

        // 加 margin + 背景
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

// =============================================================================
// 块级渲染状态机
// =============================================================================

/// 一个待渲染的内联节点（用于把内联事件折叠成带样式字符串）。
enum InlineNode {
    Text(String),
    Code(String),
    Styled(ColorFn, Vec<InlineNode>),
    /// 链接：内层节点 + href。
    Link(Vec<InlineNode>, String),
}

impl InlineNode {
    fn flatten(&self, theme: &MarkdownTheme) -> String {
        match self {
            InlineNode::Text(t) => t.clone(),
            InlineNode::Code(t) => (theme.code)(t),
            InlineNode::Styled(f, kids) => {
                let inner: String = kids.iter().map(|k| k.flatten(theme)).collect();
                f(&inner)
            }
            InlineNode::Link(kids, href) => {
                let text: String = kids.iter().map(|k| k.flatten(theme)).collect();
                let raw_text: String = kids.iter().map(|k| k.raw_text()).collect();
                let styled = (theme.link)(&(theme.underline)(&text));
                if &raw_text == href {
                    styled
                } else {
                    format!("{styled}{}", (theme.link_url)(&format!(" ({href})")))
                }
            }
        }
    }
    /// 无样式的纯文本（用于链接 text==href 判断）。
    fn raw_text(&self) -> String {
        match self {
            InlineNode::Text(t) => t.clone(),
            InlineNode::Code(t) => t.clone(),
            InlineNode::Styled(_, kids) | InlineNode::Link(kids, _) => {
                kids.iter().map(|k| k.raw_text()).collect()
            }
        }
    }
}

/// 列表上下文：是否有序、下一项序号、嵌套深度。
struct ListCtx {
    ordered: bool,
    next_index: u64,
    depth: usize,
}

struct BlockRenderer<'a> {
    theme: &'a MarkdownTheme,
    width: usize,
    lines: Vec<String>,
    // 内联折叠栈：栈底是当前块的根节点序列。
    inline_stack: Vec<Vec<InlineNode>>,
    // 链接 href 栈（与 Start(Link)/End 对齐）。
    link_hrefs: Vec<String>,
    list_stack: Vec<ListCtx>,
    // 引用块嵌套深度。
    quote_depth: usize,
    // 当前块类型，决定 End 时如何输出。
    block: BlockKind,
    heading_level: usize,
    // 代码块：语言 + 累积文本。
    code_lang: String,
    code_buf: String,
    // 每个 list item 是否已渲染过首行（用于续行缩进）。
    item_first_line_done: Vec<bool>,
    // 表格状态。
    table_head: bool,
    table_header: Vec<String>,
    table_rows: Vec<Vec<String>>,
    table_current_row: Vec<String>,
}

#[derive(PartialEq, Clone, Copy)]
enum BlockKind {
    None,
    Paragraph,
    Heading,
    CodeBlock,
}

impl<'a> BlockRenderer<'a> {
    fn new(theme: &'a MarkdownTheme, width: usize) -> Self {
        Self {
            theme,
            width,
            lines: Vec::new(),
            inline_stack: vec![Vec::new()],
            link_hrefs: Vec::new(),
            list_stack: Vec::new(),
            quote_depth: 0,
            block: BlockKind::None,
            heading_level: 0,
            code_lang: String::new(),
            code_buf: String::new(),
            item_first_line_done: Vec::new(),
            table_head: false,
            table_header: Vec::new(),
            table_rows: Vec::new(),
            table_current_row: Vec::new(),
        }
    }

    fn push_inline(&mut self, node: InlineNode) {
        self.inline_stack
            .last_mut()
            .expect("inline stack non-empty")
            .push(node);
    }

    /// 输出一条已成型的块级行（处理列表前缀 + 引用前缀）。
    fn emit_block_line(&mut self, content: String) {
        let mut line = content;
        // 列表前缀
        if let Some(ctx) = self.list_stack.last() {
            let depth = ctx.depth;
            let indent = "    ".repeat(depth);
            let marker = if ctx.ordered {
                format!("{}. ", ctx.next_index)
            } else {
                "- ".to_string()
            };
            let first_done = *self.item_first_line_done.last().unwrap_or(&true);
            let prefix = if first_done {
                format!("{indent}{}", " ".repeat(visible_width(&marker)))
            } else {
                format!("{indent}{}", (self.theme.list_bullet)(&marker))
            };
            line = format!("{prefix}{line}");
            if let Some(flag) = self.item_first_line_done.last_mut() {
                *flag = true;
            }
        }
        // 引用前缀
        for _ in 0..self.quote_depth {
            line = format!("{}{line}", (self.theme.quote_border)("│ "));
        }
        self.lines.push(line);
    }

    fn flush_paragraph(&mut self) {
        let nodes = self
            .inline_stack
            .last_mut()
            .expect("inline stack")
            .drain(..)
            .collect::<Vec<_>>();
        if nodes.is_empty() {
            return;
        }
        let text: String = nodes.iter().map(|n| n.flatten(self.theme)).collect();
        // 段内可能含 soft/hard break → 已被表示为换行；逐行 emit（emit_block_line 处理列表/引用前缀）。
        for seg in text.split('\n') {
            self.emit_block_line(seg.to_string());
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.on_text(&t),
            Event::Code(t) => self.push_inline(InlineNode::Code(t.to_string())),
            Event::SoftBreak => self.on_text(" "),
            Event::HardBreak => self.push_inline(InlineNode::Text("\n".to_string())),
            Event::Rule => {
                let rule = (self.theme.hr)(&"─".repeat(self.width.min(80)));
                self.emit_block_line(rule);
                self.lines.push(String::new());
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                // 当作纯文本（终端无法渲染 HTML）。
                self.push_inline(InlineNode::Text(h.to_string()));
            }
            _ => {}
        }
    }

    fn on_text(&mut self, t: &str) {
        if self.block == BlockKind::CodeBlock {
            self.code_buf.push_str(t);
        } else {
            self.push_inline(InlineNode::Text(t.to_string()));
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.block = BlockKind::Paragraph;
            }
            Tag::Heading { level, .. } => {
                self.block = BlockKind::Heading;
                self.heading_level = heading_depth(level);
            }
            Tag::BlockQuote(_) => {
                self.quote_depth += 1;
            }
            Tag::CodeBlock(kind) => {
                self.block = BlockKind::CodeBlock;
                self.code_lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code_buf.clear();
            }
            Tag::List(start) => {
                // 紧凑列表里，父条目文本在嵌套子列表前没有 paragraph 包裹 —— 先 flush，避免
                // 子列表行排到父条目文本之前。
                if !self.list_stack.is_empty() {
                    self.flush_paragraph();
                }
                let depth = self.list_stack.len();
                self.list_stack.push(ListCtx {
                    ordered: start.is_some(),
                    next_index: start.unwrap_or(1),
                    depth,
                });
            }
            Tag::Item => {
                self.item_first_line_done.push(false);
            }
            Tag::Emphasis => self.inline_stack.push(Vec::new()),
            Tag::Strong => self.inline_stack.push(Vec::new()),
            Tag::Strikethrough => self.inline_stack.push(Vec::new()),
            Tag::Link { dest_url, .. } => {
                self.inline_stack.push(Vec::new());
                self.link_hrefs.push(dest_url.to_string());
            }
            Tag::Table(_) => {
                self.table_reset();
            }
            Tag::TableHead => {
                self.table_head = true;
                self.table_current_row.clear();
            }
            Tag::TableRow => {
                self.table_current_row.clear();
            }
            Tag::TableCell => {
                self.inline_stack.push(Vec::new());
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush_paragraph();
                self.block = BlockKind::None;
                // 段后空行（不在列表 / 引用里时；列表条目间不空行）。
                if self.list_stack.is_empty() {
                    self.lines.push(String::new());
                }
            }
            TagEnd::Heading(_) => {
                let nodes = self
                    .inline_stack
                    .last_mut()
                    .expect("inline")
                    .drain(..)
                    .collect::<Vec<_>>();
                let inner: String = nodes.iter().map(|n| n.flatten(self.theme)).collect();
                let prefix = format!("{} ", "#".repeat(self.heading_level));
                let styled = if self.heading_level == 1 {
                    (self.theme.heading)(&(self.theme.bold)(&(self.theme.underline)(&format!(
                        "{prefix}{inner}"
                    ))))
                } else {
                    (self.theme.heading)(&(self.theme.bold)(&format!("{prefix}{inner}")))
                };
                self.emit_block_line(styled);
                self.lines.push(String::new());
                self.block = BlockKind::None;
            }
            TagEnd::BlockQuote(_) => {
                self.quote_depth = self.quote_depth.saturating_sub(1);
                if self.quote_depth == 0 && self.list_stack.is_empty() {
                    self.lines.push(String::new());
                }
            }
            TagEnd::CodeBlock => {
                self.flush_code_block();
                self.block = BlockKind::None;
                if self.list_stack.is_empty() {
                    self.lines.push(String::new());
                }
            }
            TagEnd::List(_) => {
                self.list_stack.pop();
                if self.list_stack.is_empty() {
                    self.lines.push(String::new());
                }
            }
            TagEnd::Item => {
                // 紧凑列表（tight）的条目文本不被 paragraph 包裹 —— 在此 flush 待定内联。
                // 松散列表（loose）已在 TagEnd::Paragraph flush 过，此处为空 flush，无副作用。
                self.flush_paragraph();
                self.item_first_line_done.pop();
                // 条目内的下一个序号递增（须在 flush 之后，marker 已用过当前序号）。
                if let Some(ctx) = self.list_stack.last_mut() {
                    if ctx.ordered {
                        ctx.next_index += 1;
                    }
                }
            }
            TagEnd::Emphasis => {
                let kids = self.inline_stack.pop().unwrap_or_default();
                self.push_inline(InlineNode::Styled(self.theme.italic.clone(), kids));
            }
            TagEnd::Strong => {
                let kids = self.inline_stack.pop().unwrap_or_default();
                self.push_inline(InlineNode::Styled(self.theme.bold.clone(), kids));
            }
            TagEnd::Strikethrough => {
                let kids = self.inline_stack.pop().unwrap_or_default();
                self.push_inline(InlineNode::Styled(self.theme.strikethrough.clone(), kids));
            }
            TagEnd::Link => {
                let kids = self.inline_stack.pop().unwrap_or_default();
                let href = self.link_hrefs.pop().unwrap_or_default();
                self.push_inline(InlineNode::Link(kids, href));
            }
            TagEnd::Table => {
                self.flush_table();
                if self.list_stack.is_empty() {
                    self.lines.push(String::new());
                }
            }
            TagEnd::TableHead => {
                self.table_head = false;
                let row = std::mem::take(&mut self.table_current_row);
                self.table_header = row;
            }
            TagEnd::TableRow => {
                if !self.table_head {
                    let row = std::mem::take(&mut self.table_current_row);
                    self.table_rows.push(row);
                }
            }
            TagEnd::TableCell => {
                let kids = self.inline_stack.pop().unwrap_or_default();
                let text: String = kids.iter().map(|n| n.flatten(self.theme)).collect();
                self.table_current_row.push(text);
            }
            _ => {}
        }
    }

    fn flush_code_block(&mut self) {
        let indent = self.theme.code_block_indent.clone();
        let lang = self.code_lang.clone();
        let code = std::mem::take(&mut self.code_buf);

        self.emit_block_line((self.theme.code_block_border)(&format!("```{lang}")));

        let highlighted = if self.theme.highlight_code {
            highlight_code_block(&code, &lang)
        } else {
            None
        };
        match highlighted {
            Some(hl_lines) => {
                for hl in hl_lines {
                    self.emit_block_line(format!("{indent}{hl}"));
                }
            }
            None => {
                // 回退：纯文本（去尾换行）。
                let trimmed = code.strip_suffix('\n').unwrap_or(&code);
                for code_line in trimmed.split('\n') {
                    self.emit_block_line(format!("{indent}{}", (self.theme.code_block)(code_line)));
                }
            }
        }
        self.emit_block_line((self.theme.code_block_border)("```"));
    }

    fn finish(mut self) -> Vec<String> {
        // 去掉尾随空行。
        while self.lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            self.lines.pop();
        }
        self.lines
    }

    // ---- 表格状态（放在 impl 末尾以聚拢） ----
    fn table_reset(&mut self) {
        self.table_header.clear();
        self.table_rows.clear();
        self.table_current_row.clear();
    }
}

impl<'a> BlockRenderer<'a> {
    fn flush_table(&mut self) {
        let header = std::mem::take(&mut self.table_header);
        let rows = std::mem::take(&mut self.table_rows);
        let num_cols = header.len();
        if num_cols == 0 {
            return;
        }

        // 计算列宽（自然宽度，受可用宽度约束）。
        let border_overhead = 3 * num_cols + 1;
        let mut widths: Vec<usize> = header.iter().map(|c| visible_width(c)).collect();
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                if i < num_cols {
                    widths[i] = widths[i].max(visible_width(cell));
                }
            }
        }
        // 收缩到可用宽度。
        let avail_for_cells = self.width.saturating_sub(border_overhead);
        let total: usize = widths.iter().sum();
        if total > avail_for_cells && total > 0 {
            for w in widths.iter_mut() {
                *w = (*w * avail_for_cells / total).max(1);
            }
        }

        let pad_cell = |text: &str, w: usize| -> String {
            let vis = visible_width(text);
            if vis >= w {
                // 截断到 w 列（简化：按 wrap 取首行）。
                let wrapped = wrap_text_with_ansi(text, w);
                let first = wrapped.first().cloned().unwrap_or_default();
                let fw = visible_width(&first);
                format!("{first}{}", " ".repeat(w.saturating_sub(fw)))
            } else {
                format!("{text}{}", " ".repeat(w - vis))
            }
        };

        let line = |left: &str, mid: &str, right: &str| -> String {
            let cells: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            format!("{left}─{}─{right}", cells.join(&format!("─{mid}─")))
        };

        self.emit_block_line((self.theme.code_block_border)(&line("┌", "┬", "┐")));
        // 竖线分隔符与横线统一走 code_block_border（dim），保证整框线宽一致；单元格内容样式不变。
        let bar = (self.theme.code_block_border)("│");
        // header
        let hcells: Vec<String> = header
            .iter()
            .enumerate()
            .map(|(i, c)| (self.theme.bold)(&pad_cell(c, widths[i])))
            .collect();
        self.emit_block_line(format!("{bar} {} {bar}", hcells.join(&format!(" {bar} "))));
        self.emit_block_line((self.theme.code_block_border)(&line("├", "┼", "┤")));
        for row in &rows {
            let cells: Vec<String> = widths
                .iter()
                .enumerate()
                .map(|(i, w)| pad_cell(row.get(i).map(|s| s.as_str()).unwrap_or(""), *w))
                .collect();
            self.emit_block_line(format!("{bar} {} {bar}", cells.join(&format!(" {bar} "))));
        }
        self.emit_block_line((self.theme.code_block_border)(&line("└", "┴", "┘")));
    }
}

fn heading_depth(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个用可识别哨兵包裹各元素的主题，便于断言「样式被施加」而不依赖具体 ANSI 码。
    fn sentinel_theme() -> MarkdownTheme {
        macro_rules! wrap {
            ($tag:expr) => {
                arc(move |s: &str| format!("<{}>{}</{}>", $tag, s, $tag))
            };
        }
        MarkdownTheme {
            heading: wrap!("H"),
            link: wrap!("LINK"),
            link_url: wrap!("URL"),
            code: wrap!("CODE"),
            code_block: wrap!("CB"),
            code_block_border: wrap!("CBB"),
            quote: wrap!("Q"),
            quote_border: wrap!("QB"),
            hr: wrap!("HR"),
            list_bullet: wrap!("BULLET"),
            bold: wrap!("B"),
            italic: wrap!("I"),
            strikethrough: wrap!("S"),
            underline: wrap!("U"),
            code_block_indent: "  ".to_string(),
            highlight_code: true,
        }
    }

    /// 渲染并返回去 padding 的 raw 行（直接走 BlockRenderer，跳过 wrap/padding，方便断言）。
    fn blocks(md: &str, width: usize, theme: &MarkdownTheme) -> Vec<String> {
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        opts.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(md, opts);
        let mut r = BlockRenderer::new(theme, width);
        for ev in parser {
            r.handle(ev);
        }
        r.finish()
    }

    fn joined(md: &str, width: usize, theme: &MarkdownTheme) -> String {
        blocks(md, width, theme).join("\n")
    }

    #[test]
    fn heading_styled_with_hashes() {
        let theme = sentinel_theme();
        let out = joined("# Title", 80, &theme);
        assert!(out.contains("<H>"), "heading style applied: {out:?}");
        assert!(out.contains("<B>"), "heading is bold");
        assert!(out.contains("<U>"), "h1 is underlined");
        assert!(out.contains("# Title"), "hash prefix kept: {out:?}");
    }

    #[test]
    fn h2_no_underline() {
        let theme = sentinel_theme();
        let out = joined("## Sub", 80, &theme);
        assert!(out.contains("<H>") && out.contains("<B>"));
        assert!(!out.contains("<U>"), "h2 should not be underlined: {out:?}");
        assert!(out.contains("## Sub"));
    }

    #[test]
    fn bold_italic_inline_code() {
        let theme = sentinel_theme();
        let out = joined("a **b** *c* `d`", 80, &theme);
        assert!(out.contains("<B>b</B>"), "bold: {out:?}");
        assert!(out.contains("<I>c</I>"), "italic: {out:?}");
        assert!(out.contains("<CODE>d</CODE>"), "inline code: {out:?}");
    }

    #[test]
    fn strikethrough_rendered() {
        let theme = sentinel_theme();
        let out = joined("~~gone~~", 80, &theme);
        assert!(out.contains("<S>gone</S>"), "strikethrough: {out:?}");
    }

    #[test]
    fn unordered_list_bullets() {
        let theme = sentinel_theme();
        let lines = blocks("- one\n- two", 80, &theme);
        let item_lines: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("one") || l.contains("two"))
            .collect();
        assert_eq!(item_lines.len(), 2);
        assert!(
            item_lines[0].contains("<BULLET>- </BULLET>"),
            "bullet styled: {:?}",
            item_lines[0]
        );
        // 条目之间无空行
        assert!(
            !lines.iter().any(|l| l.trim().is_empty()),
            "no blank lines between items: {lines:?}"
        );
    }

    #[test]
    fn ordered_list_numbers() {
        let theme = sentinel_theme();
        let lines = blocks("1. first\n2. second", 80, &theme);
        let joined = lines.join("\n");
        assert!(joined.contains("1. "), "1. marker: {joined:?}");
        assert!(joined.contains("2. "), "2. marker: {joined:?}");
    }

    #[test]
    fn nested_list_indented() {
        let theme = sentinel_theme();
        let lines = blocks("- top\n  - nested", 80, &theme);
        let nested = lines
            .iter()
            .find(|l| l.contains("nested"))
            .expect("nested line");
        let top = lines.iter().find(|l| l.contains("top")).expect("top line");
        // 嵌套行应有 4 空格缩进（深度 1）领先于顶层行的 bullet 前缀。
        let nested_lead = nested.len() - nested.trim_start().len();
        let top_lead = top.len() - top.trim_start().len();
        assert!(
            nested_lead > top_lead,
            "nested deeper indent: top={top:?} nested={nested:?}"
        );
        assert!(
            nested.contains("    "),
            "4-space indent present: {nested:?}"
        );
    }

    #[test]
    fn blockquote_border() {
        let theme = sentinel_theme();
        let out = joined("> quoted", 80, &theme);
        assert!(out.contains("<QB>│ </QB>"), "quote border: {out:?}");
        assert!(out.contains("quoted"));
    }

    #[test]
    fn horizontal_rule() {
        let theme = sentinel_theme();
        let out = joined("a\n\n---\n\nb", 80, &theme);
        assert!(out.contains("<HR>"), "hr styled: {out:?}");
        assert!(out.contains("─"), "hr glyph: {out:?}");
    }

    #[test]
    fn link_shows_text_and_dim_url() {
        let theme = sentinel_theme();
        let out = joined("[click](https://example.com)", 80, &theme);
        assert!(out.contains("<LINK>"), "link styled: {out:?}");
        assert!(out.contains("click"), "link text: {out:?}");
        assert!(out.contains("<URL>"), "url dim styled: {out:?}");
        assert!(out.contains("https://example.com"), "url shown: {out:?}");
    }

    #[test]
    fn autolink_without_redundant_url() {
        let theme = sentinel_theme();
        // text == href → 不重复展示 URL
        let out = joined("<https://example.com>", 80, &theme);
        assert!(out.contains("https://example.com"));
        assert!(
            !out.contains("<URL>"),
            "no redundant url when text==href: {out:?}"
        );
    }

    #[test]
    fn fenced_code_block_highlighted() {
        let theme = sentinel_theme();
        let out = joined("```rust\nfn main() {}\n```", 80, &theme);
        assert!(out.contains("```rust"), "fence open: {out:?}");
        assert!(out.contains("```"), "fence close");
        // syntect 24-bit 前景色 ANSI 应出现
        assert!(
            out.contains("\x1b[38;2;"),
            "syntax highlight ANSI present: {out:?}"
        );
        assert!(out.contains("main"), "code content present");
    }

    #[test]
    fn unknown_language_falls_back_to_plain() {
        let theme = sentinel_theme();
        let out = joined("```nonexistlang\nsome code here\n```", 80, &theme);
        assert!(out.contains("```nonexistlang"));
        // 回退路径走 code_block ColorFn，不产生 24-bit 高亮。
        assert!(
            !out.contains("\x1b[38;2;"),
            "no syntect highlight for unknown lang: {out:?}"
        );
        assert!(
            out.contains("<CB>some code here</CB>"),
            "plain code styled via code_block: {out:?}"
        );
    }

    #[test]
    fn no_language_fence_plain() {
        let theme = sentinel_theme();
        let out = joined("```\nplain text\n```", 80, &theme);
        assert!(
            !out.contains("\x1b[38;2;"),
            "no highlight without lang: {out:?}"
        );
        assert!(out.contains("<CB>plain text</CB>"));
    }

    #[test]
    fn wraps_at_width() {
        let theme = MarkdownTheme::plain();
        let mut md = Markdown::new(
            "the quick brown fox jumps over the lazy dog",
            0,
            0,
            theme,
            None,
        );
        let lines = md.render(12);
        for l in &lines {
            assert!(
                visible_width(l) <= 12,
                "line within width: {l:?} ({}) ",
                visible_width(l)
            );
        }
        assert!(lines.len() > 1, "long paragraph wraps to multiple lines");
    }

    #[test]
    fn padding_applied_and_width_filled() {
        let theme = MarkdownTheme::plain();
        let mut md = Markdown::new("hi", 2, 1, theme, None);
        let lines = md.render(20);
        // 顶部/底部各 1 空行 padding
        assert!(lines.len() >= 3);
        assert_eq!(visible_width(&lines[0]), 20, "padded to width");
        let content = lines.iter().find(|l| l.contains("hi")).expect("content");
        assert!(content.starts_with("  "), "left padding: {content:?}");
    }

    #[test]
    fn empty_renders_nothing() {
        let theme = MarkdownTheme::plain();
        let mut md = Markdown::new("   ", 1, 1, theme, None);
        assert!(md.render(40).is_empty());
    }

    #[test]
    fn caches_render() {
        let theme = MarkdownTheme::plain();
        let mut md = Markdown::new("**bold** text", 0, 0, theme, None);
        let a = md.render(40);
        let b = md.render(40);
        assert_eq!(a, b);
    }

    #[test]
    fn table_renders_with_borders() {
        let theme = sentinel_theme();
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let out = joined(md, 80, &theme);
        assert!(
            out.contains("┌") && out.contains("┐"),
            "top border: {out:?}"
        );
        assert!(out.contains("│"), "cell border");
        assert!(out.contains("└") && out.contains("┘"), "bottom border");
        assert!(
            out.contains('1') && out.contains('2'),
            "row content: {out:?}"
        );
        assert!(out.contains("<B>"), "header bold: {out:?}");
        // Uniform frame weight: the vertical separators carry the SAME dim border
        // color as the horizontal rules (both wrapped by code_block_border).
        assert!(
            out.contains("<CBB>│</CBB>"),
            "vertical separators dimmed like horizontals: {out:?}"
        );
        assert!(
            !out.contains("│ <B>"),
            "no bare (undimmed) vertical separator before a cell: {out:?}"
        );
    }

    #[test]
    fn highlight_assets_unknown_returns_none() {
        assert!(highlight_code_block("x = 1", "definitely-not-a-language").is_none());
        assert!(highlight_code_block("x = 1", "").is_none());
    }
}
