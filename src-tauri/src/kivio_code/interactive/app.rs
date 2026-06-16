//! 交互模式的 App 状态机 —— PI `modes/interactive/interactive-mode.ts` 的循环形态在 Rust 端的
//! **纯状态机**抽象。
//!
//! 设计原则：把所有「输入 → 状态变更 → 期望副作用」的逻辑收进一个不依赖真实 TTY 的对象，便于用
//! `BufferTerminal` + 合成 [`Key`](super::Key) 单测（事件循环 `mod.rs` 只负责把真实输入喂进来 +
//! 把 `render()` 的行交给差分渲染器）。
//!
//! [`App`] 持有：
//! - 一个 transcript（[`TranscriptItem`] 列表：用户消息 / 助手消息（Markdown 渲染）/ 通知 / 工具卡片占位）；
//! - 输入用的 [`Editor`]（复用 Phase 4 组件，含历史 / kill-ring / autocomplete）；
//! - footer 模型（cwd / model / 状态）；
//! - 一个模式（[`AppMode::Idle`] / [`AppMode::Generating`]）。
//!
//! 对外暴露纯方法：[`App::handle_key`]（返回 [`AppEffect`]）、[`App::submit`]、[`App::render`]
//! （把 transcript + editor + footer 组合成行）。5a 阶段 submit 不真正调用 agent，而是把输入回显为
//! 一条助手通知（真正接 agent loop 留待 5b）。

use std::sync::Arc;

use crate::kivio_code::tui::components::{
    ColorFn, Editor, EditorTheme, Loader, Markdown, MarkdownTheme, SelectItem, SelectList,
    SelectListLayoutOptions, SelectListTheme, Spacer, Text,
};
use crate::kivio_code::tui::render::Component;
use crate::kivio_code::tui::text_width::visible_width;

use super::agent_host::AgentUiEvent;
use super::slash::{dispatch_slash, SlashOutcome, SlashCommandSpec, SLASH_COMMANDS};
use super::tool_card::render_tool_card;
use crate::chat::types::{ToolCallRecord, ToolCallStatus};
use crate::kivio_code::tui::fuzzy::fuzzy_filter;

/// transcript 里的一条目。每条目自带其渲染所需的 [`Component`]（懒构造、按需重渲染）。
pub enum TranscriptItem {
    /// 用户输入的一条消息。
    UserMessage(String),
    /// 助手输出（流式累积的 Markdown 文本）。
    AssistantMessage(AssistantMessage),
    /// 系统通知 / 提示（slash 命令输出、错误、提示等）。
    Notice(String),
    /// 工具卡片（按 tool-call id upsert 状态 / 结果 / diff）。
    ToolCard(ToolCard),
}

/// 一条助手消息：流式期间增量累积 `content`，完成后 `streaming=false`。`message_id`
/// 让流式 delta 能定位到正在写的这条（多条助手消息按 id 区分）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssistantMessage {
    pub message_id: String,
    pub content: String,
    pub reasoning: String,
    pub streaming: bool,
}

/// 工具卡片：从 [`ToolCallRecord`] 投影出渲染所需的字段，按 `id` upsert。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCard {
    /// provider 分配的 tool-call id（upsert key）。
    pub id: String,
    /// 工具名（read / write / edit / bash …）。
    pub tool_name: String,
    /// 当前状态。
    pub status: ToolCallStatus,
    /// 简短的参数摘要（如 `path=src/main.rs`），用于卡片标题行。
    pub summary: String,
    /// 结果预览（成功）或错误文本（失败）。5c 起按工具类型在 `tool_card.rs` 里成形渲染，
    /// 故这里保留较完整文本（不再预裁剪到 200 列），由渲染器按工具裁剪。
    pub detail: Option<String>,
    /// 文件改动的 unified diff（仅 write/edit；从 structured_content 提取）。
    pub diff: Option<String>,
    /// 原始 structured_content（read 的行数 / mutation 的完整 diff 等），供 `tool_card.rs`
    /// 做按工具的可读渲染。
    pub structured_content: Option<serde_json::Value>,
}

/// 旧的占位结构名保留为别名，方便既有 `push_tool_card` 调用点与测试编译。
pub type ToolCardPlaceholder = ToolCard;

/// App 当前模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppMode {
    /// 空闲，等待用户输入。
    Idle,
    /// 正在生成助手回复（5b：agent loop 运行中；5a 不进入此态）。
    Generating,
}

/// `handle_key` / `submit` 的副作用，由事件循环消费。保持纯：状态变更在 App 内完成，仅把「需要外部
/// 做的事」作为枚举返回。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEffect {
    /// 无副作用（已就地处理，事件循环只需重绘）。
    None,
    /// 退出交互模式。
    Quit,
    /// 用户提交了一条消息，事件循环应交给 agent loop 跑。
    Submitted(String),
    /// 用户请求中断当前生成（Esc / generating 中的 Ctrl+C）。事件循环翻 cancel flag。
    Cancel,
    /// 请求打开模型选择器（事件循环从 settings 取 enabled 模型列表，调
    /// [`App::open_model_selector`]）。
    OpenModelSelector,
    /// 请求打开会话选择器（事件循环从磁盘取 cwd 下的会话列表，调
    /// [`App::open_session_selector`]）。
    OpenSessionSelector,
    /// 用户在模型选择器里选定了一个 `provider:model`，事件循环据此切换后续轮的活动模型。
    ModelSelected(String),
    /// 用户在会话选择器里选定了一个会话（携带其 `.jsonl` 路径），事件循环加载并重建 transcript。
    SessionSelected(String),
}

/// 覆盖层（overlay）：当前打开的全屏选择器。打开时拦截输入、渲染在 editor 上方。
enum Overlay {
    /// 模型选择器：选定后发 [`AppEffect::ModelSelected`]。
    Model(SelectList),
    /// 会话选择器：选定后发 [`AppEffect::SessionSelected`]（item.value = 会话路径）。
    Session(SelectList),
}

/// footer 数据模型（cwd / model / status；token 统计在一轮结束后由事件循环填入）。
struct Footer {
    cwd_display: String,
    model: String,
    status: String,
    /// 上一轮的 token usage 摘要（如 `1.2k in · 340 out`），无则不显示。
    usage: Option<String>,
}

/// 交互模式 App 状态机。
pub struct App {
    transcript: Vec<TranscriptItem>,
    editor: Editor,
    footer: Footer,
    mode: AppMode,
    kitty_active: bool,
    /// 最近一次 submit 留下的待处理回显（5a：让事件循环也能观察到“刚提交了什么”用于断言）。
    last_submitted: Option<String>,
    /// 当前打开的覆盖层（模型 / 会话选择器）；None = 无。打开时拦截输入。
    overlay: Option<Overlay>,
    /// generating 态下的 thinking spinner（事件循环按其 interval 调 [`App::tick_loader`]）。
    loader: Loader,
    /// 当 thinking/verbose 开启时，把 reasoning delta 显示在 spinner 旁（最近一行预览）。
    show_reasoning: bool,
    /// 是否在 transcript 上方渲染欢迎头（首屏品牌头；用户开始对话/清屏后仍保留作为页眉）。
    show_welcome: bool,
    /// slash 命令补全弹窗（编辑器内容以 `/` 开头且为整行时显示；BUG 4）。None = 不显示。
    slash_popup: Option<SelectList>,
}

impl App {
    /// 构造一个新的交互 App。`cwd_display` 已做 home→`~` 折叠；`model` 形如 `provider:model`。
    pub fn new(cwd_display: String, model: String) -> Self {
        let mut editor = Editor::new(default_editor_theme());
        editor.focused = true;
        editor.set_padding_x(1);
        let dim: ColorFn = Arc::new(|s: &str| format!("\x1b[2m{s}\x1b[22m"));
        let cyan: ColorFn = Arc::new(|s: &str| format!("\x1b[36m{s}\x1b[39m"));
        let loader = Loader::new(cyan, dim, "thinking…", None);
        Self {
            transcript: Vec::new(),
            editor,
            footer: Footer { cwd_display, model, status: "ready".to_string(), usage: None },
            mode: AppMode::Idle,
            kitty_active: false,
            last_submitted: None,
            overlay: None,
            loader,
            show_reasoning: false,
            show_welcome: true,
            slash_popup: None,
        }
    }

    /// 是否把 reasoning delta 显示在 spinner 旁（`--verbose` 或 thinking 开启时）。
    pub fn set_show_reasoning(&mut self, show: bool) {
        self.show_reasoning = show;
    }

    /// 是否渲染品牌欢迎头（续跑会话时关掉，让 transcript 成为焦点）。
    pub fn set_show_welcome(&mut self, show: bool) {
        self.show_welcome = show;
    }

    /// 推进 thinking spinner 一帧（仅 generating 态有意义）；返回是否变化（用于决定重绘）。
    pub fn tick_loader(&mut self) -> bool {
        if self.mode == AppMode::Generating {
            self.loader.tick()
        } else {
            false
        }
    }

    /// loader 帧间隔（事件循环用它决定 tick 节奏）。
    pub fn loader_interval(&self) -> std::time::Duration {
        self.loader.interval()
    }

    pub fn set_kitty_active(&mut self, active: bool) {
        self.kitty_active = active;
        self.editor.set_kitty_active(active);
        if let Some(overlay) = self.overlay_select_mut() {
            overlay.set_kitty_active(active);
        }
    }

    /// 终端尺寸变化时调用（editor 据此决定可见行数 / 翻页）。
    pub fn set_terminal_rows(&mut self, rows: u16) {
        self.editor.set_terminal_rows(rows as usize);
    }

    pub fn mode(&self) -> AppMode {
        self.mode
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.footer.status = status.into();
    }

    /// 当前活动模型（`provider:model`）。事件循环切模型后回填 footer。
    pub fn model(&self) -> &str {
        &self.footer.model
    }

    /// 设置活动模型（footer 展示），由事件循环在 [`AppEffect::ModelSelected`] 后调用。
    pub fn set_model(&mut self, model: impl Into<String>) {
        self.footer.model = model.into();
    }

    /// 是否有覆盖层打开（事件循环据此决定是否把 resize 转发给 overlay 等）。
    pub fn overlay_open(&self) -> bool {
        self.overlay.is_some()
    }

    /// 打开模型选择器：`items` 是 `(provider:model, label, description)` 列表，
    /// `current` 是当前活动模型（用于把选中项定位到它）。
    pub fn open_model_selector(&mut self, items: Vec<(String, String, Option<String>)>) {
        if items.is_empty() {
            self.push_notice("No enabled models found. Enable models in the Kivio app.");
            return;
        }
        let current = self.footer.model.clone();
        let select_items: Vec<SelectItem> = items
            .into_iter()
            .map(|(value, label, desc)| SelectItem::new(value, label, desc))
            .collect();
        let initial = select_items
            .iter()
            .position(|i| i.value == current)
            .unwrap_or(0);
        let mut list = SelectList::new(
            select_items,
            10,
            default_select_theme(),
            SelectListLayoutOptions::default(),
        );
        list.set_kitty_active(self.kitty_active);
        list.set_selected_index(initial);
        self.overlay = Some(Overlay::Model(list));
    }

    /// 打开会话选择器：`items` 是 `(session_path, label, description)` 列表。
    pub fn open_session_selector(&mut self, items: Vec<(String, String, Option<String>)>) {
        if items.is_empty() {
            self.push_notice("No saved sessions for this directory yet.");
            return;
        }
        let select_items: Vec<SelectItem> = items
            .into_iter()
            .map(|(value, label, desc)| SelectItem::new(value, label, desc))
            .collect();
        let mut list = SelectList::new(
            select_items,
            10,
            default_select_theme(),
            SelectListLayoutOptions::default(),
        );
        list.set_kitty_active(self.kitty_active);
        self.overlay = Some(Overlay::Session(list));
    }

    /// 关闭任何打开的覆盖层。
    pub fn close_overlay(&mut self) {
        self.overlay = None;
    }

    /// 取覆盖层内 SelectList 可变引用（无论种类）。
    fn overlay_select_mut(&mut self) -> Option<&mut SelectList> {
        match &mut self.overlay {
            Some(Overlay::Model(list)) | Some(Overlay::Session(list)) => Some(list),
            None => None,
        }
    }

    /// 用一个已加载的 [`Session`] 重建 transcript（resume）：清空后按记录逐条还原为
    /// user / assistant 消息 + 工具卡片。仅 UI 重建；`runtime_messages` 由事件循环单独 seed。
    pub fn rebuild_from_session(&mut self, session: &crate::kivio_code::session::Session) {
        use crate::kivio_code::session::SessionRecord;
        self.transcript.clear();
        for record in session.branch_records() {
            match record {
                SessionRecord::Message { role, content, .. } => match role.as_str() {
                    "user" => self.transcript.push(TranscriptItem::UserMessage(content.clone())),
                    "assistant" => self.push_assistant(content.clone()),
                    // system messages aren't shown in the transcript.
                    _ => {}
                },
                SessionRecord::ToolCall { call_id, name, arguments, .. } => {
                    let summary = summarize_arguments(name, &arguments.to_string());
                    self.transcript.push(TranscriptItem::ToolCard(ToolCard {
                        id: call_id.clone(),
                        tool_name: name.clone(),
                        status: ToolCallStatus::Success,
                        summary,
                        detail: None,
                        diff: None,
                        structured_content: None,
                    }));
                }
                SessionRecord::ToolResult { call_id, is_error, content, .. } => {
                    // Attach the result to the matching card (upsert by call_id).
                    if let Some(TranscriptItem::ToolCard(card)) = self
                        .transcript
                        .iter_mut()
                        .rev()
                        .find(|it| matches!(it, TranscriptItem::ToolCard(c) if &c.id == call_id))
                    {
                        card.status = if *is_error {
                            ToolCallStatus::Error
                        } else {
                            ToolCallStatus::Success
                        };
                        card.detail = Some(content.clone());
                    }
                }
                SessionRecord::Compaction { summary, .. } => {
                    self.push_notice(format!("(compacted) {summary}"));
                }
                SessionRecord::ModelChange { .. } | SessionRecord::Header { .. } => {}
            }
        }
        self.push_notice("Resumed session.");
    }

    /// transcript 条目数（测试用）。
    pub fn transcript_len(&self) -> usize {
        self.transcript.len()
    }

    /// transcript 条目的紧凑形状（测试用）：每条 `(kind, text)`，kind ∈ user/assistant/notice/tool，
    /// text 是助手段落文本 / 卡片工具名等，便于断言**时间序**。
    #[cfg(test)]
    pub fn transcript_shape(&self) -> Vec<(&'static str, String)> {
        self.transcript
            .iter()
            .map(|item| match item {
                TranscriptItem::UserMessage(t) => ("user", t.clone()),
                TranscriptItem::AssistantMessage(m) => ("assistant", m.content.clone()),
                TranscriptItem::Notice(t) => ("notice", t.clone()),
                TranscriptItem::ToolCard(c) => ("tool", c.tool_name.clone()),
            })
            .collect()
    }

    /// 最近一次提交的原文（测试用）。
    pub fn last_submitted(&self) -> Option<&str> {
        self.last_submitted.as_deref()
    }

    /// 当前编辑器内容（测试用）。
    pub fn editor_text(&self) -> String {
        self.editor.get_text()
    }

    /// 追加一条已完成的助手消息（通知 / 测试用；流式由 [`apply_agent_event`] 驱动）。
    pub fn push_assistant(&mut self, text: impl Into<String>) {
        self.transcript.push(TranscriptItem::AssistantMessage(AssistantMessage {
            message_id: String::new(),
            content: text.into(),
            reasoning: String::new(),
            streaming: false,
        }));
    }

    /// 追加一条通知。
    pub fn push_notice(&mut self, text: impl Into<String>) {
        self.transcript.push(TranscriptItem::Notice(text.into()));
    }

    /// 追加一个工具卡片（测试 / 直接构造用；运行时由 [`apply_agent_event`] upsert）。
    pub fn push_tool_card(&mut self, card: ToolCard) {
        self.transcript.push(TranscriptItem::ToolCard(card));
    }

    /// 进入 / 退出 generating 态（事件循环在 spawn agent / 收尾时调用）。
    pub fn set_mode(&mut self, mode: AppMode) {
        self.mode = mode;
    }

    /// 设置上一轮 token usage 摘要（footer 展示）。
    pub fn set_usage(&mut self, usage: Option<String>) {
        self.footer.usage = usage;
    }

    /// 当前是否有一条正在流式写入的助手消息（测试 / 收尾判断用）。
    pub fn assistant_streaming(&self) -> bool {
        self.transcript.iter().any(|item| {
            matches!(item, TranscriptItem::AssistantMessage(m) if m.streaming)
        })
    }

    /// 取最后一条助手消息的累积文本（`/copy` 与测试用）。纯方法，不触剪贴板。
    pub fn last_assistant_text(&self) -> Option<&str> {
        self.transcript.iter().rev().find_map(|item| match item {
            TranscriptItem::AssistantMessage(m) => Some(m.content.as_str()),
            _ => None,
        })
    }

    /// `/copy`：把最近一条助手消息复制到系统剪贴板，并按结果推一条通知。
    ///
    /// 把「找最近助手文本」交给纯方法 [`App::last_assistant_text`]（已单测），实际写剪贴板交给薄封装
    /// [`copy_to_clipboard`]（测试不触发真实剪贴板）。无助手消息时通知「Nothing to copy」；剪贴板后端
    /// 报错（部分 headless 环境可能发生）时把错误作为通知，不 panic。
    pub fn copy_last_assistant_to_clipboard(&mut self) {
        let text = match self.last_assistant_text() {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => {
                self.push_notice("Nothing to copy");
                return;
            }
        };
        let char_count = text.chars().count();
        match copy_to_clipboard(&text) {
            Ok(()) => self.push_notice(format!("Copied {char_count} chars to clipboard")),
            Err(err) => self.push_notice(format!("Could not copy to clipboard: {err}")),
        }
    }

    /// 取某 id 的工具卡片（测试用）。
    pub fn tool_card(&self, id: &str) -> Option<&ToolCard> {
        self.transcript.iter().rev().find_map(|item| match item {
            TranscriptItem::ToolCard(card) if card.id == id => Some(card),
            _ => None,
        })
    }

    /// 工具卡片数量（测试用）。
    pub fn tool_card_count(&self) -> usize {
        self.transcript
            .iter()
            .filter(|item| matches!(item, TranscriptItem::ToolCard(_)))
            .count()
    }

    /// 把一条 agent 事件折叠进 transcript。事件循环在 drain mpsc 时逐条调用；本方法纯（不触
    /// TTY / 不调模型），便于喂合成事件做单测。
    ///
    /// - [`AgentUiEvent::StreamDelta`]：定位到 `message_id` 的助手消息（不存在则新建一条流式中的），
    ///   追加 delta / reasoning，并将其后出现的工具卡片视为「在这条消息之后」（保持时间序）。
    /// - [`AgentUiEvent::ToolRecord`]：按 `record.id` upsert 一张工具卡片（Pending→Running→Success/Error）。
    /// - [`AgentUiEvent::Done`]：把对应助手消息 finalize（停止流式）。
    pub fn apply_agent_event(&mut self, event: AgentUiEvent) {
        match event {
            AgentUiEvent::StreamDelta { message_id, delta, reasoning } => {
                self.stream_assistant_delta(&message_id, &delta, &reasoning);
            }
            AgentUiEvent::ToolRecord(record) => {
                self.upsert_tool_card(&record);
            }
            AgentUiEvent::Done { message_id, reason } => {
                self.finalize_assistant(&message_id, &reason);
            }
        }
    }

    /// 把 delta 追加进 transcript，**保持时间序**。
    ///
    /// 关键约束（修 BUG 1）：只有当 transcript 的最后一条仍是这次轮次正在写的助手段落时，才把 delta
    /// 续写进它；若在它之后又 push 了别的条目（典型是工具卡片），下一条 delta 必须**在末尾另起一条新的
    /// 助手段落**（与前一段共享同一 `message_id`，但是时间序上不同的可视块）。这样「文字 → 工具卡片 →
    /// 文字」就会按发生顺序交错呈现，而不是所有文字挤在最上面、所有卡片堆在最下面。
    fn stream_assistant_delta(&mut self, message_id: &str, delta: &str, reasoning: &str) {
        // 末尾若是「同 message_id 且仍在流式」的助手段落，续写它；否则另起一段。
        if let Some(TranscriptItem::AssistantMessage(msg)) = self.transcript.last_mut() {
            if msg.streaming && (message_id.is_empty() || msg.message_id == message_id) {
                msg.content.push_str(delta);
                msg.reasoning.push_str(reasoning);
                return;
            }
        }
        let mut msg = AssistantMessage {
            message_id: message_id.to_string(),
            content: String::new(),
            reasoning: String::new(),
            streaming: true,
        };
        msg.content.push_str(delta);
        msg.reasoning.push_str(reasoning);
        self.transcript.push(TranscriptItem::AssistantMessage(msg));
    }

    /// 标记 `message_id` 这一轮的所有助手段落流式结束（一轮可能因工具卡片穿插而拆成多段，全部 seal）。
    /// `cancelled` / `error` 时把状态说明追加到**最后一个**该轮段落（无则推一条通知）。
    fn finalize_assistant(&mut self, message_id: &str, reason: &str) {
        let note = match reason {
            "cancelled" => Some("(cancelled)"),
            "error" => Some("(error)"),
            _ => None,
        };
        let mut last_idx: Option<usize> = None;
        for (idx, item) in self.transcript.iter_mut().enumerate() {
            if let TranscriptItem::AssistantMessage(m) = item {
                if message_id.is_empty() || m.message_id == message_id {
                    m.streaming = false;
                    last_idx = Some(idx);
                }
            }
        }
        match (note, last_idx) {
            (Some(note), Some(idx)) => {
                if let TranscriptItem::AssistantMessage(m) = &mut self.transcript[idx] {
                    if !m.content.is_empty() {
                        m.content.push_str("\n\n");
                    }
                    m.content.push_str(note);
                }
            }
            (Some(note), None) => {
                // No streamed content arrived (e.g. cancelled before any token):
                // still surface the outcome as a notice so the user sees it.
                self.push_notice(note);
            }
            _ => {}
        }
    }

    /// upsert 一张工具卡片：已存在（同 id）则就地更新状态 / 结果 / diff；否则新建并 push。
    fn upsert_tool_card(&mut self, record: &ToolCallRecord) {
        let card = ToolCard::from_record(record);
        for item in self.transcript.iter_mut() {
            if let TranscriptItem::ToolCard(existing) = item {
                if existing.id == card.id {
                    *existing = card;
                    return;
                }
            }
        }
        self.transcript.push(TranscriptItem::ToolCard(card));
    }

    /// 清空 transcript（`/new`）。
    pub fn clear_transcript(&mut self) {
        self.transcript.clear();
    }

    /// 处理一段已解码的输入序列（一个按键 / 转义序列的原始字节串，由事件循环从 StdinBuffer 喂入）。
    ///
    /// 返回 [`AppEffect`]。app 级按键（提交 / Ctrl+C / Ctrl+D）优先于 editor；其余转发给 editor。
    pub fn handle_key(&mut self, data: &str) -> AppEffect {
        use crate::kivio_code::tui::keys::matches_key;

        // 覆盖层（模型 / 会话选择器）优先吃掉所有输入。
        if self.overlay.is_some() {
            return self.handle_overlay_key(data);
        }

        // Ctrl+L：打开模型选择器（任何时候；事件循环填充列表）。
        if matches_key(data, "ctrl+l", self.kitty_active) {
            return AppEffect::OpenModelSelector;
        }

        // Esc：generating 中请求中断当前 agent 轮次（空闲时透传给 editor，由其处理补全关闭等）。
        if matches_key(data, "escape", self.kitty_active) && self.mode == AppMode::Generating {
            return AppEffect::Cancel;
        }

        // Ctrl+D：退出（仅在 editor 为空时；非空则当作 forward-delete 交给 editor，对齐常见 shell 习惯）。
        if matches_key(data, "ctrl+d", self.kitty_active) {
            if self.editor.get_text().is_empty() {
                return AppEffect::Quit;
            }
            // 非空：交给 editor 当 forward-delete。
            self.editor.handle_input(data);
            return AppEffect::None;
        }

        // Ctrl+C：generating 中 → 中断当前轮次；否则清空编辑器（已空则给退出提示）。
        if matches_key(data, "ctrl+c", self.kitty_active) {
            if self.mode == AppMode::Generating {
                return AppEffect::Cancel;
            }
            if self.editor.get_text().is_empty() {
                self.push_notice("(To exit, press Ctrl+D or type /quit)");
            } else {
                self.editor.set_text("");
            }
            return AppEffect::None;
        }

        // slash 命令补全弹窗打开时优先处理导航 / 补全 / 关闭（BUG 4）。
        if self.slash_popup.is_some() {
            // Esc：关闭弹窗（不取消生成——此分支只在 idle 到达）。
            if matches_key(data, "escape", self.kitty_active) {
                self.slash_popup = None;
                return AppEffect::None;
            }
            // Up/Down：在弹窗内导航。
            if matches_key(data, "up", self.kitty_active) || matches_key(data, "down", self.kitty_active)
            {
                if let Some(popup) = self.slash_popup.as_mut() {
                    popup.handle_input(data);
                }
                return AppEffect::None;
            }
            // Tab：把选中命令补全进编辑器（保持在编辑态，让用户可继续输入参数）。
            if matches_key(data, "tab", self.kitty_active) {
                self.complete_slash_selection();
                return AppEffect::None;
            }
            // Enter：补全选中命令并立即执行（complete-then-run）。
            if matches_key(data, "enter", self.kitty_active) {
                self.complete_slash_selection();
                self.slash_popup = None;
                return self.submit();
            }
        }

        // 提交：Enter（editor 在内部也会响应 submit，但我们要拦截以分流 slash / echo）。
        if matches_key(data, "enter", self.kitty_active) {
            return self.submit();
        }

        // 其余一律交给 editor（含历史 / 编辑 / autocomplete / 换行 alt+enter 等）。
        self.editor.handle_input(data);
        self.refresh_slash_popup();
        AppEffect::None
    }

    /// 按当前编辑器内容刷新 slash 命令弹窗：整行（首行、无换行）以 `/` 开头且尚无空格时，
    /// 用 [`slash_candidates`] fuzzy 过滤命令；否则关闭弹窗。
    fn refresh_slash_popup(&mut self) {
        let candidates = slash_candidates(&self.editor.get_text());
        match candidates {
            Some(items) if !items.is_empty() => {
                let select_items: Vec<SelectItem> = items
                    .into_iter()
                    .map(|(value, label, desc)| SelectItem::new(value, label, desc))
                    .collect();
                if let Some(popup) = self.slash_popup.as_mut() {
                    popup.set_filtered_items(select_items);
                } else {
                    let mut list = SelectList::new(
                        select_items,
                        8,
                        default_select_theme(),
                        SelectListLayoutOptions::default(),
                    );
                    list.set_kitty_active(self.kitty_active);
                    self.slash_popup = Some(list);
                }
            }
            _ => self.slash_popup = None,
        }
    }

    /// 把 slash 弹窗里选中的命令写回编辑器（替换整行为 `/<name> `），并关弹窗。
    fn complete_slash_selection(&mut self) {
        if let Some(value) = self
            .slash_popup
            .as_ref()
            .and_then(|p| p.get_selected_item())
            .map(|i| i.value)
        {
            self.editor.set_text(&format!("{value} "));
        }
    }

    /// 提交当前编辑器内容。slash 命令就地分发；否则记入 transcript 并返回 [`AppEffect::Submitted`]，
    /// 由事件循环交给 agent loop 跑。
    pub fn submit(&mut self) -> AppEffect {
        let raw = self.editor.get_expanded_text();
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            return AppEffect::None;
        }

        // 记入历史并清空编辑器。
        self.editor.add_to_history(&trimmed);
        self.editor.set_text("");
        self.slash_popup = None;

        // slash 命令分发。
        if trimmed.starts_with('/') {
            return self.dispatch_slash_command(&trimmed);
        }

        // generating 中拒绝新提交（一次只跑一轮；事件循环也会 gate，这里双保险）。
        if self.mode == AppMode::Generating {
            self.push_notice("(busy — wait for the current turn to finish or press Esc)");
            return AppEffect::None;
        }

        // 普通消息：记入 transcript，交给 agent loop。
        self.transcript.push(TranscriptItem::UserMessage(trimmed.clone()));
        self.last_submitted = Some(trimmed.clone());
        AppEffect::Submitted(trimmed)
    }

    fn dispatch_slash_command(&mut self, input: &str) -> AppEffect {
        match dispatch_slash(input) {
            SlashOutcome::Quit => AppEffect::Quit,
            SlashOutcome::ClearTranscript => {
                self.clear_transcript();
                AppEffect::None
            }
            SlashOutcome::CopyLastAssistant => {
                self.copy_last_assistant_to_clipboard();
                AppEffect::None
            }
            SlashOutcome::OpenModelSelector => AppEffect::OpenModelSelector,
            SlashOutcome::OpenSessionSelector => AppEffect::OpenSessionSelector,
            SlashOutcome::Notice(text) => {
                self.push_notice(text);
                AppEffect::None
            }
            SlashOutcome::Unknown(name) => {
                self.push_notice(format!("Unknown command: /{name}. Type /help for the list."));
                AppEffect::None
            }
        }
    }

    /// 覆盖层打开时的按键处理：Enter 确认（发对应 Selected 效果并关层）、Esc 取消（关层）、
    /// 其余（Up/Down 等导航）转发给 SelectList。
    fn handle_overlay_key(&mut self, data: &str) -> AppEffect {
        use crate::kivio_code::tui::keys::matches_key;

        if matches_key(data, "escape", self.kitty_active)
            || matches_key(data, "ctrl+c", self.kitty_active)
        {
            self.close_overlay();
            return AppEffect::None;
        }

        if matches_key(data, "enter", self.kitty_active) {
            let selected = self
                .overlay_select_mut()
                .and_then(|l| l.get_selected_item())
                .map(|i| i.value);
            let kind_is_model = matches!(self.overlay, Some(Overlay::Model(_)));
            self.close_overlay();
            return match selected {
                Some(value) if kind_is_model => AppEffect::ModelSelected(value),
                Some(value) => AppEffect::SessionSelected(value),
                None => AppEffect::None,
            };
        }

        // Navigation (up/down/wrap) goes to the list.
        if let Some(list) = self.overlay_select_mut() {
            list.handle_input(data);
        }
        AppEffect::None
    }

    /// 渲染品牌欢迎头（BUG 2）：克制的 macOS 原生气质的 bordered 头，含标题+版本、cwd、活动模型、
    /// 紧凑提示行。用既有 [`ColorFn`] 配色，不用喧闹的 ASCII art。
    fn render_welcome(&self, width: u16) -> Vec<String> {
        let dim: ColorFn = Arc::new(|s: &str| format!("\x1b[2m{s}\x1b[22m"));
        let bold: ColorFn = Arc::new(|s: &str| format!("\x1b[1m{s}\x1b[22m"));
        let cyan: ColorFn = Arc::new(|s: &str| format!("\x1b[36m{s}\x1b[39m"));

        let version = env!("CARGO_PKG_VERSION");
        let title = format!("{} {}", bold("Kivio Code"), dim(&format!("v{version}")));
        let cwd_line = format!("{}  {}", dim("cwd"), self.footer.cwd_display);
        let model_line = format!("{}  {}", dim("model"), cyan(&self.footer.model));
        let tips = dim("/help · /model · Ctrl+D exit · Esc cancel");

        // 盒宽：在终端宽内、留两列外边距，给一个上限避免在超宽终端拉得过长。
        let w = width as usize;
        let inner = w.saturating_sub(4).clamp(1, 56);
        let line_visible = |s: &str| {
            let pad = inner.saturating_sub(visible_width(s));
            format!("│ {s}{} │", " ".repeat(pad))
        };
        let top = format!("╭{}╮", "─".repeat(inner + 2));
        let bottom = format!("╰{}╯", "─".repeat(inner + 2));

        let mut out: Vec<String> = Vec::new();
        out.push(dim(&top));
        out.push(line_visible(&title));
        out.push(line_visible(&dim("Terminal coding agent")));
        out.push(line_visible(""));
        out.push(line_visible(&cwd_line));
        out.push(line_visible(&model_line));
        out.push(line_visible(""));
        out.push(line_visible(&tips));
        out.push(dim(&bottom));
        out
    }

    /// 渲染整棵 UI（transcript → 间隔 → editor → footer）成行数组（每行 ≤ width 可见列）。
    ///
    /// 每次调用重建组件树：transcript 体量在 5a 可控，重建简单可靠；5b 大 transcript 可改增量缓存。
    pub fn render(&mut self, width: u16) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();

        // 品牌欢迎头（首屏页眉）。
        if self.show_welcome {
            lines.extend(self.render_welcome(width));
            lines.push(String::new());
        }

        // transcript。
        for item in &self.transcript {
            match item {
                TranscriptItem::UserMessage(text) => {
                    let mut t = Text::new(format!("> {text}"), 1, 0, None);
                    lines.extend(t.render(width));
                    lines.push(String::new());
                }
                TranscriptItem::AssistantMessage(msg) => {
                    // reasoning（thinking）作为次要的 DIM 块呈现在答案之上（BUG 3）：
                    // 流式中逐行显示完整推理；完成后折叠为一行 dim 摘要，保持其从属于答案的视觉层级。
                    if !msg.reasoning.trim().is_empty() {
                        lines.extend(render_reasoning(&msg.reasoning, msg.streaming, width));
                    }
                    // 流式中追加一个光标提示，让用户看到「还在写」。
                    let body = if msg.streaming {
                        format!("{}▌", msg.content)
                    } else {
                        msg.content.clone()
                    };
                    if !body.trim().is_empty() {
                        let mut md = Markdown::new(body, 1, 0, MarkdownTheme::plain(), None);
                        lines.extend(md.render(width));
                    }
                    lines.push(String::new());
                }
                TranscriptItem::Notice(text) => {
                    let mut t = Text::new(format!("· {text}"), 1, 0, None);
                    lines.extend(t.render(width));
                    lines.push(String::new());
                }
                TranscriptItem::ToolCard(card) => {
                    lines.extend(render_tool_card(card, width));
                    lines.push(String::new());
                }
            }
        }

        // thinking spinner（generating 态）。在 transcript 与 editor/overlay 之间。
        if self.mode == AppMode::Generating {
            if self.show_reasoning {
                if let Some(reasoning) = self.latest_reasoning_tail() {
                    self.loader.set_message(format!("thinking… {reasoning}"));
                } else {
                    self.loader.set_message("thinking…");
                }
            }
            lines.extend(self.loader.render(width));
        }

        // overlay（模型 / 会话选择器）打开时替代 editor；否则渲染 editor。
        if let Some(overlay) = &mut self.overlay {
            lines.push(String::new());
            let (heading, list) = match overlay {
                Overlay::Model(list) => ("Select a model (Enter to choose · Esc to cancel)", list),
                Overlay::Session(list) => {
                    ("Resume a session (Enter to choose · Esc to cancel)", list)
                }
            };
            let mut h = Text::new(heading.to_string(), 1, 0, None);
            lines.extend(h.render(width));
            lines.extend(list.render(width));
        } else {
            // editor。
            lines.extend(self.editor.render(width));
            // slash 命令补全弹窗（编辑器之下；BUG 4）。
            if let Some(popup) = &mut self.slash_popup {
                lines.extend(popup.render(width));
            }
        }

        // footer：一行空隔 + 状态行。
        let mut spacer = Spacer::new(0);
        lines.extend(spacer.render(width));
        lines.extend(self.render_footer(width));

        lines
    }

    /// 取最近一条助手消息 reasoning 的尾行（spinner 旁显示用），裁剪到 60 列。
    fn latest_reasoning_tail(&self) -> Option<String> {
        self.transcript.iter().rev().find_map(|item| match item {
            TranscriptItem::AssistantMessage(m) if !m.reasoning.is_empty() => {
                let last = m.reasoning.lines().last().unwrap_or("").trim();
                if last.is_empty() {
                    None
                } else {
                    Some(clip(last, 60))
                }
            }
            _ => None,
        })
    }

    fn render_footer(&mut self, width: u16) -> Vec<String> {
        let status = match self.mode {
            AppMode::Idle => self.footer.status.clone(),
            AppMode::Generating => "generating… (Esc to cancel)".to_string(),
        };
        let mut text = format!("{}  ·  {}  ·  {}", self.footer.cwd_display, self.footer.model, status);
        if let Some(usage) = &self.footer.usage {
            text.push_str(&format!("  ·  {usage}"));
        }
        let mut footer = Text::new(text, 1, 0, None);
        footer.render(width)
    }
}

impl ToolCard {
    /// 从一条 [`ToolCallRecord`] 投影出卡片。参数摘要从 `arguments` JSON 取常见键
    /// （path/command/pattern/…）；diff 从 `structured_content`（file mutation）里取。
    pub fn from_record(record: &ToolCallRecord) -> Self {
        let summary = summarize_arguments(&record.name, &record.arguments);
        // Keep a fuller detail; tool_card.rs clips per tool family.
        let detail = record
            .error
            .clone()
            .or_else(|| record.result_preview.clone());
        let diff = extract_diff(record);
        Self {
            id: record.id.clone(),
            tool_name: record.name.clone(),
            status: record.status.clone(),
            summary,
            detail,
            diff,
            structured_content: record.structured_content.clone(),
        }
    }
}

/// 从工具参数 JSON 里取一个简短的人读摘要。失败 / 无常见键时返回空串。
fn summarize_arguments(tool_name: &str, arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return String::new();
    };
    let obj = match value.as_object() {
        Some(obj) => obj,
        None => return String::new(),
    };
    // 按工具/优先级取一个最具代表性的键。
    for key in ["path", "command", "pattern", "query", "url", "old_string"] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            let label = if key == "old_string" { "match" } else { key };
            // bash 的 command 通常多行/很长，裁剪到首行 + 80 列。
            let v = v.lines().next().unwrap_or(v);
            return format!("{label}={}", clip(v, 80));
        }
    }
    let _ = tool_name;
    String::new()
}

/// 从 file-mutation 工具的 `structured_content` 里取 unified diff（完整，不裁剪——裁剪交给
/// `tool_card.rs`）。非文件改动工具返回 `None`。保留为 `card.diff` 兜底字段（当
/// structured_content 缺 diff 时仍可用）。
fn extract_diff(record: &ToolCallRecord) -> Option<String> {
    if !matches!(record.name.as_str(), "write" | "edit" | "write_file" | "edit_file") {
        return None;
    }
    let structured = record.structured_content.as_ref()?;
    let diff = structured.get("diff").and_then(|d| d.as_str())?;
    if diff.trim().is_empty() {
        return None;
    }
    Some(diff.to_string())
}

/// `/copy` 的薄剪贴板封装：用 `arboard`（Tauri app 已依赖）把文本写入系统剪贴板。
/// 单测不调用本函数（避免在 headless / CI 上触真实剪贴板）；交互路径里其错误被
/// [`App::copy_last_assistant_to_clipboard`] 转成一条通知而非 panic。
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text.to_string()).map_err(|e| e.to_string())
}

/// 字符安全地裁剪到 `max` 列，超出加 `…`。
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let head: String = text.chars().take(max).collect();
        format!("{head}…")
    }
}

/// slash 命令补全候选（BUG 4 的纯逻辑，便于单测）。
///
/// 仅当 `input` 是单行、以 `/` 开头、且命令名后还没有空格（即仍在敲命令名）时返回 `Some(candidates)`；
/// 候选用 [`fuzzy_filter`] 按命令名匹配，每条展开为 `(/<name>, /<name>, description)`。别名不单独列出
/// （只补全规范名）。空查询（仅 `/`）列出全部命令。不满足触发条件时返回 `None`（调用方据此关闭弹窗）。
fn slash_candidates(input: &str) -> Option<Vec<(String, String, Option<String>)>> {
    // 整行触发：不允许换行（多行输入不是 slash 命令）。
    if input.contains('\n') {
        return None;
    }
    let trimmed = input.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }
    let query = trimmed.trim_start_matches('/');
    // 命令名后出现空格 ⇒ 已在输入参数，不再补全命令名。
    if query.contains(char::is_whitespace) {
        return None;
    }
    let specs: Vec<&'static SlashCommandSpec> = SLASH_COMMANDS.iter().collect();
    let filtered = fuzzy_filter(specs, query, |s| s.name.to_string());
    Some(
        filtered
            .into_iter()
            .map(|s| {
                (
                    format!("/{}", s.name),
                    format!("/{}", s.name),
                    Some(s.description.to_string()),
                )
            })
            .collect(),
    )
}

/// 把 reasoning（thinking）渲染成 DIM 次要块（BUG 3）。
///
/// 流式中（`streaming=true`）逐行 dim 显示完整推理，让用户看到模型「在想什么」；完成后折叠为一行
/// `💭 thought for a moment` dim 摘要，保持其从属于答案的视觉层级。
fn render_reasoning(reasoning: &str, streaming: bool, width: u16) -> Vec<String> {
    let dim: ColorFn = Arc::new(|s: &str| format!("\x1b[2;3m{s}\x1b[0m"));
    if streaming {
        let mut lines: Vec<String> = Vec::new();
        for raw in reasoning.lines() {
            let mut t = Text::new(format!("· {}", raw.trim_end()), 1, 0, None);
            for line in t.render(width) {
                lines.push(dim(&line));
            }
        }
        lines
    } else {
        let mut t = Text::new("💭 thought for a moment".to_string(), 1, 0, None);
        t.render(width).into_iter().map(|l| dim(&l)).collect()
    }
}

/// 一个 ANSI dim 风格的默认 editor 主题（边框灰、补全下拉素色），不依赖完整主题系统（Phase 4f 留待后续）。
fn default_editor_theme() -> EditorTheme {
    let dim: ColorFn = Arc::new(|s: &str| format!("\x1b[2m{s}\x1b[22m"));
    let cyan: ColorFn = Arc::new(|s: &str| format!("\x1b[36m{s}\x1b[39m"));
    EditorTheme {
        border_color: dim.clone(),
        select_list: default_select_theme_inner(cyan, dim),
    }
}

/// overlay 选择器主题（与 editor 内 autocomplete 同风格）。
fn default_select_theme() -> SelectListTheme {
    let dim: ColorFn = Arc::new(|s: &str| format!("\x1b[2m{s}\x1b[22m"));
    let cyan: ColorFn = Arc::new(|s: &str| format!("\x1b[36m{s}\x1b[39m"));
    default_select_theme_inner(cyan, dim)
}

fn default_select_theme_inner(cyan: ColorFn, dim: ColorFn) -> SelectListTheme {
    SelectListTheme {
        selected_prefix: cyan.clone(),
        selected_text: cyan,
        description: dim.clone(),
        scroll_info: dim.clone(),
        no_match: dim,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut a = App::new("~/proj".to_string(), "openai:gpt-4o".to_string());
        a.set_terminal_rows(24);
        a
    }

    /// 模拟敲入一串普通字符（每个字符一个 handle_key）。
    fn type_str(a: &mut App, s: &str) {
        for ch in s.chars() {
            a.handle_key(&ch.to_string());
        }
    }

    #[test]
    fn editor_receives_keystrokes() {
        let mut a = app();
        type_str(&mut a, "hello");
        assert_eq!(a.editor_text(), "hello");
    }

    #[test]
    fn submit_appends_user_message_and_returns_submitted() {
        let mut a = app();
        type_str(&mut a, "do a thing");
        let effect = a.handle_key("\r"); // enter
        assert_eq!(effect, AppEffect::Submitted("do a thing".to_string()));
        // only the user message is recorded; the assistant message arrives via streaming.
        assert_eq!(a.transcript_len(), 1);
        assert_eq!(a.last_submitted(), Some("do a thing"));
        assert!(a.editor_text().is_empty(), "editor cleared after submit");
        let lines = a.render(60);
        let joined = lines.join("\n");
        assert!(joined.contains("do a thing"));
    }

    #[test]
    fn empty_submit_is_noop() {
        let mut a = app();
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.transcript_len(), 0);
    }

    #[test]
    fn slash_quit_yields_quit() {
        let mut a = app();
        type_str(&mut a, "/quit");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::Quit);
    }

    #[test]
    fn slash_new_clears_transcript() {
        let mut a = app();
        type_str(&mut a, "hi");
        a.handle_key("\r");
        assert!(a.transcript_len() > 0);
        type_str(&mut a, "/new");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.transcript_len(), 0);
    }

    #[test]
    fn slash_clear_clears_transcript() {
        let mut a = app();
        type_str(&mut a, "hi");
        a.handle_key("\r");
        type_str(&mut a, "/clear");
        a.handle_key("\r");
        assert_eq!(a.transcript_len(), 0);
    }

    #[test]
    fn slash_help_shows_commands_notice() {
        let mut a = app();
        type_str(&mut a, "/help");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.transcript_len(), 1);
        let joined = a.render(80).join("\n");
        assert!(joined.contains("/help"));
        assert!(joined.contains("/quit"));
        assert!(joined.contains("/new"));
        assert!(joined.contains("/copy"));
    }

    #[test]
    fn last_assistant_text_none_when_empty() {
        let a = app();
        assert_eq!(a.last_assistant_text(), None);
    }

    #[test]
    fn last_assistant_text_returns_most_recent() {
        let mut a = app();
        a.push_assistant("first answer");
        a.push_assistant("second answer");
        assert_eq!(a.last_assistant_text(), Some("second answer"));
    }

    #[test]
    fn copy_with_no_assistant_message_notices_nothing_to_copy() {
        let mut a = app();
        a.copy_last_assistant_to_clipboard();
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Nothing to copy"));
    }

    #[test]
    fn slash_copy_routes_through_copy_handler() {
        // With no assistant message yet, /copy must push the "Nothing to copy"
        // notice (and never touch the real clipboard) — proving the slash command
        // is wired to the copy handler on the App state-machine path.
        let mut a = app();
        type_str(&mut a, "/copy");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Nothing to copy"));
    }

    #[test]
    fn unknown_slash_yields_notice() {
        let mut a = app();
        type_str(&mut a, "/bogus");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Unknown command"));
        assert!(joined.contains("bogus"));
    }

    #[test]
    fn ctrl_d_quits_when_empty() {
        let mut a = app();
        let effect = a.handle_key("\x04"); // ctrl+d
        assert_eq!(effect, AppEffect::Quit);
    }

    #[test]
    fn ctrl_d_does_not_quit_when_editor_nonempty() {
        let mut a = app();
        type_str(&mut a, "abc");
        // cursor at end; ctrl+d (forward-delete) deletes nothing but must NOT quit
        let effect = a.handle_key("\x04");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.editor_text(), "abc");
    }

    #[test]
    fn ctrl_c_clears_editor() {
        let mut a = app();
        type_str(&mut a, "draft text");
        let effect = a.handle_key("\x03"); // ctrl+c
        assert_eq!(effect, AppEffect::None);
        assert!(a.editor_text().is_empty());
    }

    #[test]
    fn ctrl_c_on_empty_shows_hint() {
        let mut a = app();
        let effect = a.handle_key("\x03");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.transcript_len(), 1);
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Ctrl+D") || joined.contains("/quit"));
    }

    #[test]
    fn render_composes_transcript_editor_footer() {
        let mut a = app();
        type_str(&mut a, "question one");
        a.handle_key("\r");
        let lines = a.render(70);
        let joined = lines.join("\n");
        // transcript user line
        assert!(joined.contains("question one"), "transcript present");
        // footer shows cwd + model + status
        assert!(joined.contains("~/proj"), "footer cwd present");
        assert!(joined.contains("openai:gpt-4o"), "footer model present");
        assert!(joined.contains("ready"), "footer status present");
        // every line within width
        for l in &lines {
            assert!(
                crate::kivio_code::tui::text_width::visible_width(l) <= 70,
                "line exceeds width: {:?}",
                l
            );
        }
    }

    #[test]
    fn push_tool_card_renders() {
        let mut a = app();
        a.push_tool_card(ToolCard {
            id: "c1".to_string(),
            tool_name: "read".to_string(),
            status: ToolCallStatus::Success,
            summary: "path=src/main.rs".to_string(),
            detail: None,
            diff: None,
            structured_content: None,
        });
        let joined = a.render(60).join("\n");
        assert!(joined.contains("read"));
        assert!(joined.contains("src/main.rs"));
    }

    fn tool_record(id: &str, name: &str, status: ToolCallStatus) -> ToolCallRecord {
        ToolCallRecord {
            id: id.to_string(),
            name: name.to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: serde_json::json!({ "path": "src/lib.rs" }).to_string(),
            status,
            result_preview: Some("ok".to_string()),
            error: None,
            duration_ms: None,
            started_at: None,
            completed_at: None,
            round: 1,
            sensitive: false,
            artifacts: Vec::new(),
            trace_id: None,
            span_id: None,
            structured_content: None,
        }
    }

    #[test]
    fn apply_stream_delta_accumulates_into_assistant_message() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "Hel".to_string(),
            reasoning: String::new(),
        });
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "lo".to_string(),
            reasoning: String::new(),
        });
        assert_eq!(a.last_assistant_text(), Some("Hello"));
        assert!(a.assistant_streaming(), "still streaming before Done");
    }

    #[test]
    fn apply_done_finalizes_streaming() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "Answer".to_string(),
            reasoning: String::new(),
        });
        a.apply_agent_event(AgentUiEvent::Done {
            message_id: "m1".to_string(),
            reason: "completed".to_string(),
        });
        assert!(!a.assistant_streaming(), "finalized after Done");
        assert_eq!(a.last_assistant_text(), Some("Answer"));
    }

    #[test]
    fn apply_done_cancelled_appends_note() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "partial".to_string(),
            reasoning: String::new(),
        });
        a.apply_agent_event(AgentUiEvent::Done {
            message_id: "m1".to_string(),
            reason: "cancelled".to_string(),
        });
        let text = a.last_assistant_text().unwrap();
        assert!(text.contains("partial"));
        assert!(text.contains("cancelled"));
        assert!(!a.assistant_streaming());
    }

    #[test]
    fn tool_card_upsert_transitions_status_by_id() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(tool_record(
            "call_1",
            "read",
            ToolCallStatus::Pending,
        ))));
        assert_eq!(a.tool_card_count(), 1);
        assert_eq!(a.tool_card("call_1").unwrap().status, ToolCallStatus::Pending);

        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(tool_record(
            "call_1",
            "read",
            ToolCallStatus::Running,
        ))));
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(tool_record(
            "call_1",
            "read",
            ToolCallStatus::Success,
        ))));
        // Same id → still one card, now Success.
        assert_eq!(a.tool_card_count(), 1);
        assert_eq!(a.tool_card("call_1").unwrap().status, ToolCallStatus::Success);
        assert_eq!(a.tool_card("call_1").unwrap().summary, "path=src/lib.rs");
    }

    #[test]
    fn distinct_tool_ids_make_distinct_cards() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(tool_record(
            "call_1",
            "read",
            ToolCallStatus::Success,
        ))));
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(tool_record(
            "call_2",
            "grep",
            ToolCallStatus::Success,
        ))));
        assert_eq!(a.tool_card_count(), 2);
    }

    #[test]
    fn edit_tool_card_extracts_diff() {
        let mut record = tool_record("call_1", "edit", ToolCallStatus::Success);
        record.structured_content = Some(serde_json::json!({
            "diff": "@@ -1 +1 @@\n-old line\n+new line"
        }));
        let card = ToolCard::from_record(&record);
        assert!(card.diff.as_deref().unwrap().contains("+new line"));

        let mut a = app();
        a.push_tool_card(card);
        let joined = a.render(80).join("\n");
        assert!(joined.contains("new line"), "diff rendered in card: {joined}");
    }

    #[test]
    fn submit_while_generating_is_rejected() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        type_str(&mut a, "another");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        // user message not recorded; a busy notice is.
        assert_eq!(a.last_submitted(), None);
    }

    #[test]
    fn esc_while_generating_requests_cancel() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        let effect = a.handle_key("\x1b"); // ESC
        assert_eq!(effect, AppEffect::Cancel);
    }

    #[test]
    fn ctrl_c_while_generating_requests_cancel() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        let effect = a.handle_key("\x03"); // ctrl+c
        assert_eq!(effect, AppEffect::Cancel);
    }

    #[test]
    fn footer_shows_usage_when_set() {
        let mut a = app();
        a.set_usage(Some("1.2k in · 340 out".to_string()));
        let joined = a.render(80).join("\n");
        assert!(joined.contains("1.2k in"));
    }

    // ---- Phase 5c: model selector / overlay ----

    fn model_items() -> Vec<(String, String, Option<String>)> {
        vec![
            ("openai:gpt-4o".into(), "gpt-4o".into(), Some("OpenAI".into())),
            ("anthropic:claude".into(), "claude".into(), Some("Anthropic".into())),
        ]
    }

    #[test]
    fn ctrl_l_requests_model_selector() {
        let mut a = app();
        assert_eq!(a.handle_key("\x0c"), AppEffect::OpenModelSelector);
    }

    #[test]
    fn slash_model_requests_model_selector() {
        let mut a = app();
        type_str(&mut a, "/model");
        assert_eq!(a.handle_key("\r"), AppEffect::OpenModelSelector);
    }

    #[test]
    fn slash_sessions_requests_session_selector() {
        let mut a = app();
        type_str(&mut a, "/sessions");
        assert_eq!(a.handle_key("\r"), AppEffect::OpenSessionSelector);
    }

    #[test]
    fn open_model_selector_renders_and_intercepts_input() {
        let mut a = app();
        a.open_model_selector(model_items());
        assert!(a.overlay_open());
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Select a model"));
        assert!(joined.contains("gpt-4o"));
        assert!(joined.contains("claude"));
        // Typing while overlay open does NOT reach the editor.
        type_str(&mut a, "hello");
        assert!(a.editor_text().is_empty());
    }

    #[test]
    fn model_selector_confirm_emits_selected_and_closes() {
        let mut a = app();
        a.open_model_selector(model_items());
        // selection starts at current model (openai:gpt-4o); move down to anthropic.
        a.handle_key("\x1b[B"); // down
        let effect = a.handle_key("\r"); // confirm
        assert_eq!(effect, AppEffect::ModelSelected("anthropic:claude".to_string()));
        assert!(!a.overlay_open(), "overlay closed after confirm");
    }

    #[test]
    fn model_selector_starts_at_current_model() {
        let mut a = App::new("~".into(), "anthropic:claude".into());
        a.set_terminal_rows(24);
        a.open_model_selector(model_items());
        // current is the second item → selecting immediately returns it.
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::ModelSelected("anthropic:claude".to_string()));
    }

    #[test]
    fn overlay_esc_cancels_without_selection() {
        let mut a = app();
        a.open_model_selector(model_items());
        let effect = a.handle_key("\x1b"); // esc
        assert_eq!(effect, AppEffect::None);
        assert!(!a.overlay_open());
    }

    #[test]
    fn empty_model_list_pushes_notice_not_overlay() {
        let mut a = app();
        a.open_model_selector(Vec::new());
        assert!(!a.overlay_open());
        let joined = a.render(80).join("\n");
        assert!(joined.contains("No enabled models"));
    }

    #[test]
    fn set_model_updates_footer() {
        let mut a = app();
        a.set_model("anthropic:claude-3");
        assert_eq!(a.model(), "anthropic:claude-3");
        let joined = a.render(80).join("\n");
        assert!(joined.contains("anthropic:claude-3"));
    }

    #[test]
    fn session_selector_confirm_emits_session_selected() {
        let mut a = app();
        a.open_session_selector(vec![(
            "/path/to/a.jsonl".into(),
            "2026-06-16".into(),
            Some("read main.rs".into()),
        )]);
        assert!(a.overlay_open());
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::SessionSelected("/path/to/a.jsonl".to_string()));
        assert!(!a.overlay_open());
    }

    // ---- Phase 5c: thinking loader ----

    #[test]
    fn loader_shown_only_while_generating() {
        let mut a = app();
        // idle: no spinner text
        assert!(!a.render(80).join("\n").contains("thinking"));
        a.set_mode(AppMode::Generating);
        assert!(a.render(80).join("\n").contains("thinking"));
    }

    #[test]
    fn tick_loader_only_advances_when_generating() {
        let mut a = app();
        assert!(!a.tick_loader(), "no tick while idle");
        a.set_mode(AppMode::Generating);
        assert!(a.tick_loader(), "ticks while generating");
    }

    // ---- Phase 5c: input history cycling (editor-backed) ----

    #[test]
    fn up_arrow_cycles_previous_inputs() {
        let mut a = app();
        // submit two messages so they enter history.
        type_str(&mut a, "first message");
        a.handle_key("\r");
        type_str(&mut a, "second message");
        a.handle_key("\r");
        // editor empty; Up loads the most recent submission.
        assert!(a.editor_text().is_empty());
        a.handle_key("\x1b[A"); // up
        assert_eq!(a.editor_text(), "second message");
        a.handle_key("\x1b[A"); // up again → older
        assert_eq!(a.editor_text(), "first message");
        a.handle_key("\x1b[B"); // down → back to newer
        assert_eq!(a.editor_text(), "second message");
    }

    // ---- Phase 5c: resume rebuild ----

    #[test]
    fn rebuild_from_session_restores_transcript() {
        use crate::kivio_code::session::{Session, SessionRecord};
        let cwd = std::env::temp_dir().join(format!("kivio-app-resume-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&cwd).unwrap();
        let mut session = Session::create(&cwd, "p:m").unwrap();
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: "t".into(),
                role: "user".into(),
                content: "read main.rs".into(),
            })
            .unwrap();
        session
            .append(SessionRecord::ToolCall {
                id: String::new(),
                parent_id: None,
                timestamp: "t".into(),
                call_id: "call_1".into(),
                name: "read".into(),
                arguments: serde_json::json!({ "path": "main.rs" }),
            })
            .unwrap();
        session
            .append(SessionRecord::ToolResult {
                id: String::new(),
                parent_id: None,
                timestamp: "t".into(),
                call_id: "call_1".into(),
                name: "read".into(),
                content: "fn main() {}".into(),
                is_error: false,
            })
            .unwrap();
        session
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: "t".into(),
                role: "assistant".into(),
                content: "It is empty.".into(),
            })
            .unwrap();

        let mut a = app();
        a.rebuild_from_session(&session);
        let joined = a.render(80).join("\n");
        assert!(joined.contains("read main.rs"), "user message restored");
        assert!(joined.contains("It is empty."), "assistant message restored");
        // tool card present and carries its result via summary/path.
        assert!(joined.contains("main.rs"), "tool card restored");
        assert_eq!(a.tool_card("call_1").map(|c| c.status.clone()), Some(ToolCallStatus::Success));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    // ---- BUG 1: transcript interleaves by emission order ----

    #[test]
    fn stream_then_tool_then_stream_interleaves_in_order() {
        let mut a = app();
        // Round 1 assistant text.
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "A".to_string(),
            reasoning: String::new(),
        });
        // A tool card lands between rounds.
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(tool_record(
            "call_1",
            "read",
            ToolCallStatus::Success,
        ))));
        // Round 2 assistant text — must start a NEW segment AFTER the card.
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "B".to_string(),
            reasoning: String::new(),
        });

        let shape = a.transcript_shape();
        assert_eq!(
            shape,
            vec![
                ("assistant", "A".to_string()),
                ("tool", "read".to_string()),
                ("assistant", "B".to_string()),
            ],
            "text/tool/text must interleave by emission order, not pile up"
        );
        // last_assistant_text returns the LAST assistant segment.
        assert_eq!(a.last_assistant_text(), Some("B"));
        assert!(a.assistant_streaming());
    }

    #[test]
    fn consecutive_deltas_without_tool_stay_one_segment() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "Hel".to_string(),
            reasoning: String::new(),
        });
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "lo".to_string(),
            reasoning: String::new(),
        });
        // No tool between → single coalesced segment.
        assert_eq!(a.transcript_shape(), vec![("assistant", "Hello".to_string())]);
    }

    #[test]
    fn finalize_seals_all_segments_of_the_turn() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "A".to_string(),
            reasoning: String::new(),
        });
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(tool_record(
            "call_1",
            "read",
            ToolCallStatus::Success,
        ))));
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "B".to_string(),
            reasoning: String::new(),
        });
        a.apply_agent_event(AgentUiEvent::Done {
            message_id: "m1".to_string(),
            reason: "completed".to_string(),
        });
        assert!(!a.assistant_streaming(), "all turn segments sealed after Done");
    }

    // ---- BUG 2: branded welcome header ----

    #[test]
    fn initial_render_shows_welcome_header() {
        let mut a = app();
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Kivio Code"), "title present");
        assert!(joined.contains(env!("CARGO_PKG_VERSION")), "version present");
        assert!(joined.contains("~/proj"), "cwd present");
        assert!(joined.contains("openai:gpt-4o"), "active model present");
        assert!(joined.contains("/help"), "tips line present");
        assert!(joined.contains("Ctrl+D"), "tips mention exit");
    }

    #[test]
    fn welcome_header_can_be_suppressed() {
        let mut a = app();
        a.set_show_welcome(false);
        let joined = a.render(80).join("\n");
        assert!(!joined.contains("Kivio Code"));
    }

    // ---- BUG 3: reasoning rendered as a dim block ----

    #[test]
    fn streaming_reasoning_rendered_dim() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "answer".to_string(),
            reasoning: "considering options".to_string(),
        });
        let joined = a.render(80).join("\n");
        assert!(joined.contains("considering options"), "reasoning visible while streaming");
        // dim+italic ANSI escape is applied (\x1b[2;3m).
        assert!(joined.contains("\x1b[2;3m"), "reasoning is dim/italic");
        assert!(joined.contains("answer"), "answer still rendered");
    }

    #[test]
    fn finalized_reasoning_collapses_to_thought_line() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "answer".to_string(),
            reasoning: "long chain of thought".to_string(),
        });
        a.apply_agent_event(AgentUiEvent::Done {
            message_id: "m1".to_string(),
            reason: "completed".to_string(),
        });
        let joined = a.render(80).join("\n");
        assert!(joined.contains("thought for a moment"), "collapsed thought summary shown");
        assert!(!joined.contains("long chain of thought"), "full reasoning hidden once done");
    }

    // ---- BUG 4: slash command autocomplete ----

    #[test]
    fn slash_candidates_lists_all_for_bare_slash() {
        let c = slash_candidates("/").expect("triggers on bare slash");
        assert_eq!(c.len(), SLASH_COMMANDS.len());
    }

    #[test]
    fn slash_candidates_fuzzy_filters_by_name() {
        let c = slash_candidates("/mod").expect("triggers");
        let values: Vec<&str> = c.iter().map(|(v, _, _)| v.as_str()).collect();
        assert!(values.contains(&"/model"));
        assert!(!values.contains(&"/quit"));
    }

    #[test]
    fn slash_candidates_none_when_not_slash_or_has_args() {
        assert!(slash_candidates("hello").is_none());
        assert!(slash_candidates("/model gpt").is_none(), "args → no command completion");
        assert!(slash_candidates("line\n/model").is_none(), "multiline → no completion");
    }

    #[test]
    fn typing_slash_opens_popup_and_renders() {
        let mut a = app();
        type_str(&mut a, "/mo");
        let joined = a.render(80).join("\n");
        assert!(joined.contains("/model"), "popup lists matching command");
        assert!(joined.contains("Switch the active model"), "popup shows description");
    }

    #[test]
    fn popup_enter_completes_and_runs_command() {
        // "/mod" fuzzy-matches /model; Enter completes then runs → OpenModelSelector.
        let mut a = app();
        type_str(&mut a, "/mod");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::OpenModelSelector);
    }

    #[test]
    fn popup_tab_completes_into_editor_without_running() {
        let mut a = app();
        type_str(&mut a, "/mod");
        let effect = a.handle_key("\t");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.editor_text(), "/model ");
    }

    #[test]
    fn popup_esc_dismisses_without_clearing_editor() {
        let mut a = app();
        type_str(&mut a, "/mo");
        let effect = a.handle_key("\x1b"); // esc
        assert_eq!(effect, AppEffect::None);
        // editor text retained; popup gone.
        assert_eq!(a.editor_text(), "/mo");
        let joined = a.render(80).join("\n");
        assert!(!joined.contains("Switch the active model"), "popup dismissed");
    }

    #[test]
    fn typing_slash_help_then_enter_still_runs() {
        // Even with the popup open, an exact /help + Enter must execute /help.
        let mut a = app();
        type_str(&mut a, "/help");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        assert!(a.render(80).join("\n").contains("/help"));
    }
}
