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

use std::path::PathBuf;
use std::sync::Arc;

use crate::kivio_code::tui::components::{
    ColorFn, Editor, EditorTheme, Loader, Markdown, MarkdownTheme, SelectItem, SelectList,
    SelectListLayoutOptions, SelectListTheme, Spacer, Text,
};
use crate::kivio_code::tui::render::Component;
use crate::kivio_code::tui::text_width::visible_width;

use super::agent_host::AgentUiEvent;
use super::slash::{dispatch_slash, SlashCommandSpec, SlashOutcome, INIT_PROMPT, SLASH_COMMANDS};
use super::tool_card::render_tool_card;
use crate::chat::types::{ToolCallRecord, ToolCallStatus};
use crate::kivio_code::tui::fuzzy::fuzzy_filter;

/// 一张待提交的图片附件。`label` 是它在编辑器文本里的占位符（`[Image #N]`，1-based，
/// 按当前未发送消息的添加顺序编号）。提交后清空、`/new` 时也清空。**永不**在终端渲染图片，
/// 只保留占位符文本。
#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingImage {
    path: PathBuf,
    label: String,
}

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
    /// 本轮 reasoning 的耗时（秒），在 finalize 时从 [`App::generating_elapsed`] 快照下来
    /// （此时 Generating 起点尚未被 `set_mode(Idle)` 清掉）。`Some(n)` ⇒ 折叠摘要显示
    /// `┄ thought for {n}s`；`None` ⇒ 显示 `┄ thought for a moment`。
    pub thought_secs: Option<u64>,
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

/// agent 的工作模式（与 [`AppMode`] 的 Idle/Generating 正交）：
/// - [`AgentMode::Build`]（默认）：全工具集（read/write/edit/bash/…）。
/// - [`AgentMode::Plan`]：只读研究 + 出方案（drop write/edit/bash，仅保留 read/ls/grep/find/web_fetch
///   + 只读 skill 工具）。对标 Claude Code 的 Plan Mode / opencode 的 plan agent。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AgentMode {
    /// 默认：完整工具集。
    #[default]
    Build,
    /// 只读研究 + 规划（无变更）。
    Plan,
}

/// `handle_key` / `submit` 的副作用，由事件循环消费。保持纯：状态变更在 App 内完成，仅把「需要外部
/// 做的事」作为枚举返回。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppEffect {
    /// 无副作用（已就地处理，事件循环只需重绘）。
    None,
    /// 退出交互模式。
    Quit,
    /// 开新会话（`/new` / `/clear`）：事件循环清屏 + 重置 [`TurnRuntime`] 上下文
    /// （runtime_messages / session / ctx gauge），而不仅清掉屏幕 transcript。
    NewConversation,
    /// 用户提交了一条消息，事件循环应交给 agent loop 跑。`text` 是占位符已替换的用户文字
    /// （`[Image #N]` 占位符 verbatim 保留）；`images` 是本轮附带的图片文件路径（按 `[Image #N]`
    /// 编号顺序），事件循环把它交给视觉 mixer 预分析。无图时 `images` 为空。
    Submitted { text: String, images: Vec<PathBuf> },
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
    /// `/mcp`：事件循环 block_on 探测已配置 MCP 服务器后，把状态摘要推进 transcript。
    ShowMcp,
    /// `/skill`：事件循环从活动 runtime 的 skill_registry 渲染技能列表推进 transcript。
    ShowSkills,
    /// `/settings`：请求打开设置覆盖层（事件循环把当前持久化的 toggle 值取出，调
    /// [`App::open_settings_selector`]）。
    OpenSettings,
    /// `/compact [focus]`：请求强制压缩当前对话历史（无视预算）。事件循环 block_on 走
    /// `force_compact`，成功后用压缩后的历史替换 runtime_messages 并刷新 footer ctx。
    /// `focus` 为聚焦指令（命令后剩余文字，None = 无聚焦）。
    Compact { focus: Option<String> },
}

/// 覆盖层（overlay）：当前打开的全屏选择器。打开时拦截输入、渲染在 editor 上方。
enum Overlay {
    /// 模型选择器：选定后发 [`AppEffect::ModelSelected`]。
    Model(SelectList),
    /// 会话选择器：选定后发 [`AppEffect::SessionSelected`]（item.value = 会话路径）。
    Session(SelectList),
    /// 设置开关列表：选定一项**就地翻转**对应 toggle（不发 Selected 效果），保存到
    /// kivio-code 配置，并关层 + 推一条通知。`item.value` 是设置键（如 `read_claude_dir`）。
    Settings(SelectList),
}

/// footer 数据模型（cwd / model / status；token 统计在一轮结束后由事件循环填入）。
struct Footer {
    cwd_display: String,
    /// 用于选择器定位 / 续会话解析的 id 形式值（`providerId:model`）。**不直接展示**。
    model: String,
    /// 人读展示串（`<Provider Name> · <model>`）。footer / welcome / 选择器定位用 `model`，
    /// 但渲染统一用本字段（FIX 2）。为空时退回 `model`（id 形式）。
    model_display: String,
    status: String,
    /// 当前模型的上下文窗口大小（tokens）+ 是否可靠（FIX 3）。`None` = 未知（仅显示原始
    /// token 数，不显示占比）。事件循环切模型 / 起轮时回填。
    context_window: Option<usize>,
    /// 最近一轮 agent 上报的 input_tokens，近似当前已占用的上下文（FIX 3）。
    context_tokens: Option<u64>,
}

/// 交互模式 App 状态机。
pub struct App {
    transcript: Vec<TranscriptItem>,
    editor: Editor,
    footer: Footer,
    mode: AppMode,
    /// build / plan 工作模式（Shift+Tab 切换；plan 为只读研究 + 规划）。默认 Build。
    agent_mode: AgentMode,
    kitty_active: bool,
    /// 最近一次 submit 留下的待处理回显（5a：让事件循环也能观察到“刚提交了什么”用于断言）。
    last_submitted: Option<String>,
    /// 当前打开的覆盖层（模型 / 会话选择器）；None = 无。打开时拦截输入。
    overlay: Option<Overlay>,
    /// generating 态下的 thinking spinner（事件循环按其 interval 调 [`App::tick_loader`]）。
    loader: Loader,
    /// spinner 的**相位标签**（与 reasoning 尾巴预览解耦）：随最近一次 agent 信号变化
    /// （planning→`thinking…`、工具运行→`reading …`/`running: …`、答案流式→`responding…`），
    /// 让 spinner 像 Claude Code / Codex 那样反映当前在做什么，而非固定 "thinking…"。
    /// 每轮起始（`set_mode(Generating)`）重置为默认 `thinking…`。render 时把 reasoning 尾巴
    /// 叠加在它之上。
    phase_label: String,
    /// 当 thinking/verbose 开启时，把 reasoning delta 显示在 spinner 旁（最近一行预览）。
    show_reasoning: bool,
    /// 是否在 transcript 上方渲染欢迎头（首屏品牌头；用户开始对话/清屏后仍保留作为页眉）。
    show_welcome: bool,
    /// slash 命令补全弹窗（编辑器内容以 `/` 开头且为整行时显示；BUG 4）。None = 不显示。
    slash_popup: Option<SelectList>,
    /// kivio-code 配置 `read_claude_dir` 的内存副本（`/settings` 翻转时同步更新 +
    /// 落盘）。下一轮 `build_system_prompt` 直接读磁盘上的同一值，故无需把它穿透进 TurnRuntime。
    read_claude_dir: bool,
    /// kivio-code 配置 `auto_plan` 的内存副本：build 模式遇复杂任务时是否给模型
    /// `enter_plan_mode` 工具 + 在系统提示里引导它先规划。`/autoplan on|off` 翻转并落盘。
    /// 默认从持久化配置 seed（默认 ON）。关掉后 `enter_plan_mode` 不进工具集、提示不加，
    /// 回到纯手动 Shift+Tab。
    auto_plan: bool,
    /// 「本轮 plan 是自动进入的」标记：build turn 调了 `enter_plan_mode` → 交互层自动切到
    /// plan 跑只读规划时置 true；该 plan turn 结束后据此切回 build 并暂停（"say proceed"），
    /// 然后清掉。用户中途 Esc 取消时也清掉，避免卡在半切换态。普通手动 plan（Shift+Tab /
    /// `/plan`）不置位，故结束后不会自动切回 build。
    auto_plan_pending: bool,
    /// 最近一次 resize / 初始化时的终端宽度（列）。`/skill`·`/mcp` 等摘要据此把每条截断到一行。
    terminal_cols: u16,
    /// 进入 Generating 态的时刻（`std::time::Instant`，单调时钟）。footer 据此显示
    /// `generating m:ss · Esc to cancel` 的实时计时；退出 Generating（回到 Idle）时清空。
    /// 用 `Instant` 而非 wall-clock，确保计时不受系统时间跳变影响。
    generating_started: Option<std::time::Instant>,
    /// 当前未发送消息附带的图片（Ctrl+V 粘贴 / 拖入路径）。每张对应编辑器里一个 `[Image #N]`
    /// 占位符；提交后清空，`/new` 时清空。**永不**渲染图片本体。
    pending_images: Vec<PendingImage>,
    /// 待处理的工具审批提示（agent loop 通过 [`AgentUiEvent::ApprovalRequest`] 请求）。存在时
    /// 拦截所有按键（y/n/Esc），并在 editor 位置渲染提示。答复后清空。
    pending_approval: Option<PendingApproval>,
    /// 本会话是否已授予文件/命令访问（session-consent）。首次同意后置真，后续 session-consent
    /// 请求直接放行、不再打扰（对齐 GUI「每会话确认一次」语义）。
    session_consent_granted: bool,
}

/// 一次待答复的审批请求：提示文案 + 是否 session-consent（可缓存）+ 回传决定的通道。
struct PendingApproval {
    prompt: String,
    session: bool,
    responder: std::sync::mpsc::Sender<bool>,
}

impl App {
    /// 构造一个新的交互 App。`cwd_display` 已做 home→`~` 折叠；`model` 形如 `provider:model`
    /// （id 形式，作为选择器定位 / 续会话解析的解析值）。展示串默认等于它，事件循环随后可用
    /// [`App::set_model_display`] 覆盖为人读串（`<Provider Name> · model`）。
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
            footer: Footer {
                cwd_display,
                model_display: model.clone(),
                model,
                status: "ready".to_string(),
                context_window: None,
                context_tokens: None,
            },
            mode: AppMode::Idle,
            agent_mode: AgentMode::Build,
            kitty_active: false,
            last_submitted: None,
            overlay: None,
            loader,
            phase_label: DEFAULT_PHASE_LABEL.to_string(),
            show_reasoning: false,
            show_welcome: true,
            slash_popup: None,
            // Seed from the persisted kivio-code config so `/settings` reflects the
            // saved value; the event loop may override via `set_read_claude_dir`.
            read_claude_dir: crate::kivio_code::config::load().read_claude_dir,
            auto_plan: crate::kivio_code::config::load().auto_plan,
            auto_plan_pending: false,
            terminal_cols: 80,
            generating_started: None,
            pending_images: Vec::new(),
            pending_approval: None,
            session_consent_granted: false,
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

    /// 记录终端宽度（列）。`render` 每帧也会刷新它；事件循环在 resize 时可显式设置，
    /// 让 `/skill`·`/mcp` 之类的摘要在下一帧前已知一个合理宽度。
    pub fn set_terminal_cols(&mut self, cols: u16) {
        self.terminal_cols = cols;
    }

    /// 最近已知的终端宽度（列）。`/skill`·`/mcp` 摘要据此把每条截断到一行。
    pub fn terminal_cols(&self) -> u16 {
        self.terminal_cols
    }

    pub fn mode(&self) -> AppMode {
        self.mode
    }

    /// 当前 agent 工作模式（build / plan）。事件循环起轮时据此 gate 工具集 + 注入 plan 系统提示。
    pub fn agent_mode(&self) -> AgentMode {
        self.agent_mode
    }

    /// 设置 agent 工作模式（build / plan），并按切换方向推一条通知。空闲态由 Shift+Tab /
    /// `/plan` · `/build` 调用；切到同一模式则只刷新通知（幂等友好）。Plan→Build 的切换额外提示
    /// 用户「say 'proceed' to execute the plan」（显式审批，不自动提交）。
    pub fn set_agent_mode(&mut self, next: AgentMode) {
        let prev = self.agent_mode;
        self.agent_mode = next;
        match next {
            AgentMode::Plan => {
                self.push_notice("Plan mode: read-only research & planning");
            }
            AgentMode::Build => {
                if prev == AgentMode::Plan {
                    self.push_notice(
                        "Switched to build. Say 'proceed' to execute the plan, or give new instructions.",
                    );
                } else {
                    self.push_notice("Build mode: full tools");
                }
            }
        }
    }

    /// Shift+Tab 在空闲态切换 build↔plan（生成中 / 有 overlay 打开时忽略）。返回是否切换了
    /// （事件循环据此决定重绘——本方法已推通知 + 改状态，故返回值仅供测试 / 调用方参考）。
    fn toggle_agent_mode(&mut self) -> bool {
        if self.mode == AppMode::Generating || self.overlay.is_some() {
            return false;
        }
        let next = match self.agent_mode {
            AgentMode::Build => AgentMode::Plan,
            AgentMode::Plan => AgentMode::Build,
        };
        self.set_agent_mode(next);
        true
    }

    pub fn set_status(&mut self, status: impl Into<String>) {
        self.footer.status = status.into();
    }

    /// 当前活动模型的**解析值**（`provider:model`，id 形式）。选择器定位 / 续会话解析用，
    /// 不用于展示。事件循环切模型后回填。
    pub fn model(&self) -> &str {
        &self.footer.model
    }

    /// 设置活动模型的**解析值**（id 形式），由事件循环在 [`AppEffect::ModelSelected`] 后调用。
    /// 注意：这只更新选择器定位用的值；展示串由 [`App::set_model_display`] 单独设置。
    pub fn set_model(&mut self, model: impl Into<String>) {
        self.footer.model = model.into();
    }

    /// 设置活动模型的**展示串**（`<Provider Name> · model`），footer / welcome 渲染用。
    /// 与解析值解耦（FIX 2）：切模型时事件循环同时调 [`App::set_model`]（id 值）+ 本方法（展示串）。
    pub fn set_model_display(&mut self, display: impl Into<String>) {
        self.footer.model_display = display.into();
    }

    /// 是否有覆盖层打开（事件循环据此决定是否把 resize 转发给 overlay 等）。
    #[cfg(test)]
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

    /// 设置 `read_claude_dir` 的内存值（事件循环可在启动时同步实际持久化值）。
    pub fn set_read_claude_dir(&mut self, value: bool) {
        self.read_claude_dir = value;
    }

    /// 当前 `read_claude_dir` 内存值（测试用）。
    #[cfg(test)]
    pub fn read_claude_dir(&self) -> bool {
        self.read_claude_dir
    }

    /// 当前 `auto_plan` 内存值（build→plan 自动切换是否开启）。事件循环起 build turn 时据此
    /// 决定是否把 `enter_plan_mode` 工具加进工具集。
    pub fn auto_plan(&self) -> bool {
        self.auto_plan
    }

    /// 设置 `auto_plan` 内存值（事件循环可在启动时同步实际持久化值）。
    pub fn set_auto_plan(&mut self, value: bool) {
        self.auto_plan = value;
    }

    /// 「本轮 plan 是自动进入的」标记当前值（测试 / 事件循环判定用）。
    pub fn auto_plan_pending(&self) -> bool {
        self.auto_plan_pending
    }

    /// 设置 / 清除 auto-plan pending 标记。build turn 触发自动转 plan 时置 true；该 plan
    /// turn 结束切回 build、或用户 Esc 取消时清 false。
    pub fn set_auto_plan_pending(&mut self, value: bool) {
        self.auto_plan_pending = value;
    }

    /// 打开设置开关覆盖层（`/settings`）。当前仅一项：Read .claude / CLAUDE.md context，
    /// 展示其 on/off 状态。选定后就地翻转 + 保存（见 [`App::handle_overlay_key`]）。
    pub fn open_settings_selector(&mut self) {
        let on = self.read_claude_dir;
        let label = format!(
            "Read .claude / CLAUDE.md context  [{}]",
            if on { "on" } else { "off" }
        );
        let select_items = vec![SelectItem::new("read_claude_dir".to_string(), label, None)];
        let mut list = SelectList::new(
            select_items,
            10,
            default_select_theme(),
            SelectListLayoutOptions::default(),
        );
        list.set_kitty_active(self.kitty_active);
        self.overlay = Some(Overlay::Settings(list));
    }

    /// 取覆盖层内 SelectList 可变引用（无论种类）。
    fn overlay_select_mut(&mut self) -> Option<&mut SelectList> {
        match &mut self.overlay {
            Some(Overlay::Model(list))
            | Some(Overlay::Session(list))
            | Some(Overlay::Settings(list)) => Some(list),
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
                    "user" => self
                        .transcript
                        .push(TranscriptItem::UserMessage(content.clone())),
                    "assistant" => self.push_assistant(content.clone()),
                    // system messages aren't shown in the transcript.
                    _ => {}
                },
                SessionRecord::ToolCall {
                    call_id,
                    name,
                    arguments,
                    ..
                } => {
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
                SessionRecord::ToolResult {
                    call_id,
                    is_error,
                    content,
                    ..
                } => {
                    // Attach the result to the matching card (upsert by call_id).
                    if let Some(TranscriptItem::ToolCard(card)) =
                        self.transcript.iter_mut().rev().find(
                            |it| matches!(it, TranscriptItem::ToolCard(c) if &c.id == call_id),
                        )
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
        self.transcript
            .push(TranscriptItem::AssistantMessage(AssistantMessage {
                message_id: String::new(),
                content: text.into(),
                reasoning: String::new(),
                streaming: false,
                thought_secs: None,
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
    /// 进入 generating 时把 spinner 相位标签重置为默认 `thinking…`，让新一轮不残留上一轮的
    /// "running: …" 之类的状态。
    pub fn set_mode(&mut self, mode: AppMode) {
        if mode == AppMode::Generating && self.mode != AppMode::Generating {
            self.set_phase_label(DEFAULT_PHASE_LABEL.to_string());
            // 起轮：快照单调时钟起点，footer 据此显示实时计时。
            self.generating_started = Some(std::time::Instant::now());
        }
        if mode == AppMode::Idle {
            // 收尾：清掉计时起点，footer 回到静态 ready 状态。
            self.generating_started = None;
        }
        self.mode = mode;
    }

    /// 当前 Generating 轮次的已用时长（单调时钟）。非 Generating 态或未起轮时为 `None`。
    /// footer 计时与 reasoning 折叠摘要（`thought for {N}s`）共用此值。
    pub fn generating_elapsed(&self) -> Option<std::time::Duration> {
        self.generating_started.map(|start| start.elapsed())
    }

    /// 测试钩子：把 Generating 起点向过去回拨 `elapsed`，使 [`generating_elapsed`] 返回一个
    /// 确定值（避免依赖真实时间流逝），从而对 footer 计时做可重复断言。
    #[cfg(test)]
    pub fn set_generating_elapsed_for_test(&mut self, elapsed: std::time::Duration) {
        self.generating_started = std::time::Instant::now().checked_sub(elapsed);
    }

    /// 设置 spinner 的相位标签（基础串），并同步到 loader。reasoning 尾巴预览在 render 时叠加。
    fn set_phase_label(&mut self, label: String) {
        self.phase_label = label;
        self.loader.set_message(self.phase_label.clone());
    }

    /// 当前 spinner 相位标签（测试断言用）。
    #[cfg(test)]
    pub fn phase_label(&self) -> &str {
        &self.phase_label
    }

    /// 设置当前模型的上下文窗口大小（tokens；`None` = 未知，则 footer 只显示原始 token 数）。
    /// 由事件循环在起轮 / 切模型时回填（FIX 3）。
    pub fn set_context_window(&mut self, window: Option<usize>) {
        self.footer.context_window = window;
    }

    /// 设置当前对话的上下文占用估算（tokens），footer 据此算占比。
    /// 来源是 agent loop 压缩所用的同一估算器（`compaction::estimate_messages_tokens`），
    /// 与 0.85 压缩触发点对齐，**不是**单轮花费。
    pub fn set_context_tokens(&mut self, tokens: Option<u64>) {
        self.footer.context_tokens = tokens;
    }

    /// 当前 footer 记录的上下文占用估算（测试断言用）。
    #[cfg(test)]
    pub fn context_tokens(&self) -> Option<u64> {
        self.footer.context_tokens
    }

    /// 当前是否有一条正在流式写入的助手消息（测试 / 收尾判断用）。
    pub fn assistant_streaming(&self) -> bool {
        self.transcript
            .iter()
            .any(|item| matches!(item, TranscriptItem::AssistantMessage(m) if m.streaming))
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

    /// 把一个已落盘的图片路径登记为待提交附件，返回分配的 `[Image #N]` 占位符（N = 当前
    /// `pending_images` 长度 + 1，1-based，提交 / `/new` 后从 1 重新计数）。不插入编辑器、不渲染图片
    /// 本体——纯状态更新，便于单测。
    fn attach_pending_image(&mut self, path: PathBuf) -> String {
        let number = self.pending_images.len() + 1;
        let label = format!("[Image #{number}]");
        self.pending_images.push(PendingImage {
            path,
            label: label.clone(),
        });
        label
    }

    /// 待提交图片数量（测试用）。
    #[cfg(test)]
    pub fn pending_image_count(&self) -> usize {
        self.pending_images.len()
    }

    /// 处理 Ctrl+V：优先尝试把系统剪贴板里的**图片**作为附件（编码 PNG → 落盘 →
    /// 插入 `[Image #N]` 占位符 + 推通知）。剪贴板无图片 / 后端报错时**静默 no-op**
    /// （文本粘贴由 bracketed-paste 单独走 [`super::InputEvent::Paste`]，不在此分支）。
    /// 永不 panic、永不渲染图片本体。
    fn handle_clipboard_image_paste(&mut self) {
        let image = match arboard::Clipboard::new().and_then(|mut c| c.get_image()) {
            Ok(image) => image,
            // 无图片（或 headless / 无剪贴板后端）：静默放过，让文本粘贴路径处理。
            Err(_) => return,
        };

        let width = image.width;
        let height = image.height;
        let rgba =
            match image::RgbaImage::from_raw(width as u32, height as u32, image.bytes.into_owned())
            {
                Some(buf) => buf,
                None => {
                    self.push_notice("Could not read clipboard image (bad dimensions)");
                    return;
                }
            };

        let mut png_bytes: Vec<u8> = Vec::new();
        if let Err(err) = rgba.write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        ) {
            self.push_notice(format!("Could not encode clipboard image: {err}"));
            return;
        }

        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes);
        match crate::chat::attachments::save_pasted_image("pasted", "image/png", &b64) {
            Ok(crate::chat::attachments::PastedImageSave::Saved { path, .. }) => {
                let label = self.attach_pending_image(path);
                self.editor.insert_text(&label);
                self.refresh_slash_popup();
                self.push_notice(format!("Attached {label} from clipboard"));
            }
            Ok(crate::chat::attachments::PastedImageSave::Failed { error }) => {
                self.push_notice(format!("Could not attach clipboard image: {error}"));
            }
            Err(err) => {
                self.push_notice(format!("Could not attach clipboard image: {err}"));
            }
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
    #[cfg(test)]
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
            AgentUiEvent::StreamDelta {
                message_id,
                delta,
                reasoning,
            } => {
                self.stream_assistant_delta(&message_id, &delta, &reasoning);
            }
            AgentUiEvent::ToolRecord(record) => {
                self.upsert_tool_card(&record);
            }
            AgentUiEvent::Done { message_id, reason } => {
                self.finalize_assistant(&message_id, &reason);
            }
            AgentUiEvent::ApprovalRequest {
                prompt,
                session,
                responder,
            } => {
                // Cached session consent: a prior "yes" this session auto-approves
                // further session-consent requests without re-prompting.
                if session && self.session_consent_granted {
                    let _ = responder.send(true);
                    return;
                }
                self.pending_approval = Some(PendingApproval {
                    prompt,
                    session,
                    responder,
                });
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
        // 相位标签：有可见答案文本 → `responding…`；只有 reasoning（无可见 delta）→ `thinking…`。
        // （工具运行中收到答案 delta 也意味着模型已开始作答，故覆盖工具相位。）
        if !delta.is_empty() {
            self.set_phase_label("responding…".to_string());
        } else if !reasoning.is_empty() {
            self.set_phase_label(DEFAULT_PHASE_LABEL.to_string());
        }
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
            thought_secs: None,
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
        // 折叠摘要的耗时：在 Generating 起点被清掉之前快照下来（事件循环随后会 set_mode(Idle)）。
        let thought_secs = self.generating_elapsed().map(|d| d.as_secs());
        let mut last_idx: Option<usize> = None;
        for (idx, item) in self.transcript.iter_mut().enumerate() {
            if let TranscriptItem::AssistantMessage(m) = item {
                if message_id.is_empty() || m.message_id == message_id {
                    m.streaming = false;
                    if !m.reasoning.trim().is_empty() {
                        m.thought_secs = thought_secs;
                    }
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
        // 相位标签随工具状态变化：运行中（Pending/Running）→ 现在进行时短语（如 `reading lib.rs`）；
        // 完成（Success/Error）→ 退回默认 `thinking…`（模型接着会思考/作答）。
        match record.status {
            ToolCallStatus::Pending | ToolCallStatus::Running => {
                self.set_phase_label(tool_phase_label(&record.name, &record.arguments));
            }
            // 完成 / 取消 / 跳过：退回默认 `thinking…`（模型接着会思考/作答）。
            ToolCallStatus::Success
            | ToolCallStatus::Error
            | ToolCallStatus::Cancelled
            | ToolCallStatus::Skipped => {
                self.set_phase_label(DEFAULT_PHASE_LABEL.to_string());
            }
        }
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

    /// 清空 transcript（`/new`）。同时丢弃未提交的图片附件（占位符编号从 `[Image #1]` 重新开始）。
    pub fn clear_transcript(&mut self) {
        self.transcript.clear();
        self.pending_images.clear();
    }

    /// 翻转 `read_claude_dir` 并持久化到 kivio-code 配置（`/settings` 选定时调用）。更新内存值，
    /// 保存失败时把错误作为通知（不 panic），成功时推一条 `Read .claude: on/off` 通知。
    /// 下一轮 `build_system_prompt` 读磁盘上的同一值，故无需重启即生效。
    fn toggle_read_claude_dir(&mut self) {
        let next = !self.read_claude_dir;
        self.read_claude_dir = next;
        // Load-modify-save so flipping this toggle preserves the other persisted fields
        // (default model, approval policy) set from the GUI settings tab.
        let mut cfg = crate::kivio_code::config::load();
        cfg.read_claude_dir = next;
        match crate::kivio_code::config::save(&cfg) {
            Ok(()) => {
                self.push_notice(format!("Read .claude: {}", if next { "on" } else { "off" }))
            }
            Err(err) => self.push_notice(format!("Could not save settings: {err}")),
        }
    }

    /// 处理 `/autoplan [on|off]`。`Some(on)` 设置并持久化（load-modify-save 保留其他字段），
    /// `None`（无参数）只显示当前状态。更新内存值，让下一轮 build turn 据 `auto_plan` 决定是否
    /// 给 `enter_plan_mode` 工具；下一轮 `build_system_prompt` 读磁盘同值决定是否加引导段。
    fn handle_autoplan_command(&mut self, arg: Option<bool>) {
        match arg {
            None => {
                self.push_notice(format!(
                    "Auto plan is {} (use /autoplan on|off to change).",
                    if self.auto_plan { "on" } else { "off" }
                ));
            }
            Some(next) => {
                self.auto_plan = next;
                let mut cfg = crate::kivio_code::config::load();
                cfg.auto_plan = next;
                match crate::kivio_code::config::save(&cfg) {
                    Ok(()) => {
                        self.push_notice(format!("Auto plan: {}", if next { "on" } else { "off" }))
                    }
                    Err(err) => self.push_notice(format!("Could not save settings: {err}")),
                }
            }
        }
    }

    /// 处理一段已解码的输入序列（一个按键 / 转义序列的原始字节串，由事件循环从 StdinBuffer 喂入）。
    ///
    /// 返回 [`AppEffect`]。app 级按键（提交 / Ctrl+C / Ctrl+D）优先于 editor；其余转发给 editor。
    pub fn handle_key(&mut self, data: &str) -> AppEffect {
        use crate::kivio_code::tui::keys::matches_key;

        // 待审批提示优先吃掉所有输入：y=批准；n/Esc=拒绝并取消本轮。
        if self.pending_approval.is_some() {
            return self.handle_approval_key(data);
        }

        // 覆盖层（模型 / 会话选择器）优先吃掉所有输入。
        if self.overlay.is_some() {
            return self.handle_overlay_key(data);
        }

        // Ctrl+L：打开模型选择器（任何时候；事件循环填充列表）。
        if matches_key(data, "ctrl+l", self.kitty_active) {
            return AppEffect::OpenModelSelector;
        }

        // Shift+Tab：在空闲态切换 build↔plan 工作模式（生成中忽略；overlay 已在上面早返回）。
        // 切换在 App 内完成（推通知 + 改状态 + 刷 footer），无需事件循环介入。
        if matches_key(data, "shift+tab", self.kitty_active) {
            self.toggle_agent_mode();
            return AppEffect::None;
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

        // Ctrl+V：尝试把剪贴板里的图片作为附件（image-first，文本回退由 bracketed-paste 处理）。
        // 仅在空闲态处理；生成中忽略。
        if matches_key(data, "ctrl+v", self.kitty_active) {
            if self.mode != AppMode::Generating {
                self.handle_clipboard_image_paste();
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
            if matches_key(data, "up", self.kitty_active)
                || matches_key(data, "down", self.kitty_active)
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

    /// 待审批态的按键处理：`y`/`Y` 批准（session 请求则记住本会话）；`n`/`N`/`Esc` 拒绝并取消本轮
    /// （拒绝文件/命令访问通常意味着「停下」，故连带取消，避免带着被拒工具继续跑）。其它键忽略。
    fn handle_approval_key(&mut self, data: &str) -> AppEffect {
        use crate::kivio_code::tui::keys::matches_key;
        let approve = data.eq_ignore_ascii_case("y");
        let deny = data.eq_ignore_ascii_case("n") || matches_key(data, "escape", self.kitty_active);
        if !approve && !deny {
            return AppEffect::None;
        }
        if let Some(pending) = self.pending_approval.take() {
            let _ = pending.responder.send(approve);
            if approve {
                if pending.session {
                    self.session_consent_granted = true;
                    self.push_notice("Approved — file/command access granted for this session.");
                } else {
                    self.push_notice("Approved.");
                }
                return AppEffect::None;
            }
            self.push_notice("Denied — cancelling this turn.");
            return AppEffect::Cancel;
        }
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

        // 拖入 / 输入的本地图片路径 → 附件：扫描文本里指向存在图片文件的 token，登记为附件并把
        // 该 token 替换为 `[Image #N]`（编号续接已有的 clipboard 附件）。非图片 / 不存在的 token 原样保留。
        let start_number = self.pending_images.len() + 1;
        let (rewritten, drag_paths) = extract_image_paths_from_text(&trimmed, start_number);
        for path in drag_paths {
            self.attach_pending_image(path);
        }

        // 普通消息：记入 transcript，交给 agent loop。
        self.submit_message(rewritten)
    }

    /// 把一条消息记入 transcript + last_submitted，返回 [`AppEffect::Submitted`]，交给 agent loop。
    /// 携带本轮附带的图片路径（来自 clipboard 粘贴 + 拖入路径，按 `[Image #N]` 编号顺序），并清空
    /// `pending_images`（下一条消息从 `[Image #1]` 重新计数）。普通用户输入与 `/init` 共用此路径；
    /// 调用方负责 generating-gate。
    fn submit_message(&mut self, text: String) -> AppEffect {
        self.transcript
            .push(TranscriptItem::UserMessage(text.clone()));
        self.last_submitted = Some(text.clone());
        let images: Vec<PathBuf> = std::mem::take(&mut self.pending_images)
            .into_iter()
            .map(|img| img.path)
            .collect();
        AppEffect::Submitted { text, images }
    }

    fn dispatch_slash_command(&mut self, input: &str) -> AppEffect {
        match dispatch_slash(input) {
            SlashOutcome::Quit => AppEffect::Quit,
            SlashOutcome::NewConversation => AppEffect::NewConversation,
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
            SlashOutcome::RunInit => {
                // /init runs a normal agent turn seeded with the canned INIT prompt;
                // reject while a turn is already generating, exactly like a normal submit.
                if self.mode == AppMode::Generating {
                    self.push_notice("(busy — wait for the current turn to finish or press Esc)");
                    return AppEffect::None;
                }
                self.submit_message(INIT_PROMPT.to_string())
            }
            SlashOutcome::ShowMcp => AppEffect::ShowMcp,
            SlashOutcome::ShowSkills => AppEffect::ShowSkills,
            SlashOutcome::OpenSettings => AppEffect::OpenSettings,
            SlashOutcome::EnterPlan => {
                // Same path as Shift+Tab; mode lives on App so no TurnRuntime needed.
                self.set_agent_mode(AgentMode::Plan);
                AppEffect::None
            }
            SlashOutcome::EnterBuild => {
                self.set_agent_mode(AgentMode::Build);
                AppEffect::None
            }
            SlashOutcome::SetAutoPlan(arg) => {
                self.handle_autoplan_command(arg);
                AppEffect::None
            }
            SlashOutcome::Compact { focus } => {
                // Compaction rewrites the conversation history; reject while a turn is
                // generating (the runtime is mid-mutation), exactly like a normal submit.
                if self.mode == AppMode::Generating {
                    self.push_notice("(busy — wait for the current turn to finish or press Esc)");
                    return AppEffect::None;
                }
                AppEffect::Compact { focus }
            }
            SlashOutcome::Unknown(name) => {
                self.push_notice(format!(
                    "Unknown command: /{name}. Type /help for the list."
                ));
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
            // Settings overlay: flip the toggle in place + persist, then close +
            // notice (does NOT emit a Selected effect like model/session do).
            if matches!(self.overlay, Some(Overlay::Settings(_))) {
                self.close_overlay();
                self.toggle_read_claude_dir();
                return AppEffect::None;
            }
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
        let model_line = format!("{}  {}", dim("model"), cyan(&self.footer.model_display));
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
    /// 单区视图（测试 / 兼容用）：把 [`Self::render_frame`] 的 static + dynamic 拼回单个数组。
    pub fn render(&mut self, width: u16) -> Vec<String> {
        let frame = self.render_frame(width);
        let mut lines = frame.static_lines;
        lines.extend(frame.dynamic_lines);
        lines
    }

    /// 渲染一帧并切分为 static（已定稿历史，提交一次进 scrollback 不再 diff）/ dynamic（仍在变的尾部
    /// + spinner + editor/overlay + footer，就地差分，永远在底部）双区。
    ///
    /// graduation 规则：static = transcript 中「已定稿」item 组成的**最长前缀**（见
    /// [`Self::item_finalized`]）；dynamic = 第一个未定稿 item 起到帧尾的全部。welcome 头在出现
    /// 第一个已定稿 item 后随该前缀一并进 static（只打印一次）；在此之前与一切同属 dynamic。
    /// spinner 永远在 dynamic 区，物理上不可能漏进 scrollback —— 这正是修「多行冻结 spinner 堆叠」的根因。
    pub fn render_frame(&mut self, width: u16) -> crate::kivio_code::tui::render::Frame {
        use crate::kivio_code::tui::render::Frame;
        self.terminal_cols = width;

        // 最长已定稿前缀长度。
        let mut graduated = 0usize;
        for item in &self.transcript {
            if self.item_finalized(item) {
                graduated += 1;
            } else {
                break;
            }
        }

        let mut static_lines: Vec<String> = Vec::new();
        // welcome 仅在已有定稿前缀（对话真正开始、welcome 成为冻结页眉）时进 static；否则留 dynamic。
        if self.show_welcome && graduated > 0 {
            static_lines.extend(self.render_welcome(width));
            static_lines.push(String::new());
        }
        for item in self.transcript.iter().take(graduated) {
            static_lines.extend(Self::render_transcript_item(item, width));
        }

        let mut dynamic_lines: Vec<String> = Vec::new();
        // welcome 未进 static 时（无定稿前缀），仍作为 dynamic 区顶部页眉显示。
        if self.show_welcome && graduated == 0 {
            dynamic_lines.extend(self.render_welcome(width));
            dynamic_lines.push(String::new());
        }
        for item in self.transcript.iter().skip(graduated) {
            dynamic_lines.extend(Self::render_transcript_item(item, width));
        }

        // thinking spinner（generating 态）。在 transcript 与 editor/overlay 之间 —— 永远 dynamic。
        if self.mode == AppMode::Generating {
            if self.show_reasoning {
                if let Some(reasoning) = self.latest_reasoning_tail() {
                    self.loader
                        .set_message(format!("{} {reasoning}", self.phase_label));
                } else {
                    self.loader.set_message(self.phase_label.clone());
                }
            }
            dynamic_lines.extend(self.loader.render(width));
        }

        // overlay（模型 / 会话选择器）打开时替代 editor；否则渲染 editor。
        if let Some(pending) = &self.pending_approval {
            // 待审批：在 editor 位置渲染提示 + 按键说明，独占输入。
            dynamic_lines.push(String::new());
            let mut heading = Text::new(pending.prompt.clone(), 1, 0, None);
            dynamic_lines.extend(heading.render(width));
            let mut hint = Text::new(
                "[y] approve   [n] deny (cancels this turn)   [Esc] deny".to_string(),
                1,
                0,
                None,
            );
            dynamic_lines.extend(hint.render(width));
        } else if let Some(overlay) = &mut self.overlay {
            dynamic_lines.push(String::new());
            let (heading, list) = match overlay {
                Overlay::Model(list) => ("Select a model (Enter to choose · Esc to cancel)", list),
                Overlay::Session(list) => {
                    ("Resume a session (Enter to choose · Esc to cancel)", list)
                }
                Overlay::Settings(list) => ("Settings (Enter to toggle · Esc to close)", list),
            };
            let mut h = Text::new(heading.to_string(), 1, 0, None);
            dynamic_lines.extend(h.render(width));
            dynamic_lines.extend(list.render(width));
        } else {
            dynamic_lines.extend(self.editor.render(width));
            if let Some(popup) = &mut self.slash_popup {
                dynamic_lines.extend(popup.render(width));
            }
        }

        // footer：一行空隔 + 状态行 —— 永远 dynamic。
        let mut spacer = Spacer::new(0);
        dynamic_lines.extend(spacer.render(width));
        dynamic_lines.extend(self.render_footer(width));

        Frame {
            static_lines,
            dynamic_lines,
        }
    }

    /// 一条 transcript item 是否「已定稿」（可提交进 scrollback 不再 diff）：
    /// - `UserMessage` / `Notice`：加入即定稿。
    /// - `AssistantMessage`：`streaming == false` 才定稿（流式中 content 在增长且带 `▌`）。
    /// - `ToolCard`：status 为终态（Success/Error/Skipped/Cancelled）才定稿；`Pending`/`Running` 属 dynamic。
    fn item_finalized(&self, item: &TranscriptItem) -> bool {
        match item {
            TranscriptItem::UserMessage(_) | TranscriptItem::Notice(_) => true,
            TranscriptItem::AssistantMessage(m) => !m.streaming,
            TranscriptItem::ToolCard(c) => matches!(
                c.status,
                ToolCallStatus::Success
                    | ToolCallStatus::Error
                    | ToolCallStatus::Cancelled
                    | ToolCallStatus::Skipped
            ),
        }
    }

    /// 渲染单条 transcript item 为行数组（含其尾随空行间隔）。static / dynamic 两区共用，保证视觉一致。
    fn render_transcript_item(item: &TranscriptItem, width: u16) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        match item {
            TranscriptItem::UserMessage(text) => {
                let mut t = Text::new(format!("> {text}"), 1, 0, None);
                lines.extend(t.render(width));
                lines.push(String::new());
            }
            TranscriptItem::AssistantMessage(msg) => {
                // reasoning（thinking）作为次要的 DIM 块呈现在答案之上（BUG 3）：
                // 流式中显示最近几行推理（dim+italic，从属于答案）；完成后折叠为一行 dim 摘要。
                if !msg.reasoning.trim().is_empty() {
                    lines.extend(render_reasoning(
                        &msg.reasoning,
                        msg.streaming,
                        msg.thought_secs,
                        width,
                    ));
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
            AppMode::Generating => {
                let elapsed = self.generating_elapsed().unwrap_or_default();
                format_generating_status(elapsed)
            }
        };
        // build / plan 工作模式 chip：放在 status 之前。plan 用 warning 黄醒目（read-only 提醒），
        // build 用 dim 低调。Shift+Tab 切换。
        let mode_chip = match self.agent_mode {
            AgentMode::Build => {
                let dim: ColorFn = Arc::new(|s: &str| format!("\x1b[2m{s}\x1b[22m"));
                dim(" build ")
            }
            AgentMode::Plan => {
                let warn: ColorFn = Arc::new(|s: &str| format!("\x1b[33m{s}\x1b[39m"));
                warn(" plan ")
            }
        };
        // 不在 footer 行展示模型/provider（已在欢迎头里给出），也不展示单轮 token usage
        // （`N in · N out`，用户不需要）；footer 仅 cwd · mode · 状态 · ctx。
        let mut text = format!(
            "{}  ·  {}  ·  {}",
            self.footer.cwd_display, mode_chip, status
        );
        if let Some(ctx) =
            format_context_usage(self.footer.context_tokens, self.footer.context_window)
        {
            text.push_str(&format!("  ·  {ctx}"));
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

/// spinner 的默认相位标签（planning / 思考中 / 工具完成后回退）。
const DEFAULT_PHASE_LABEL: &str = "thinking…";

/// 把一条运行中的工具调用映射成现在进行时的相位短语（spinner 标签用），如
/// `reading lib.rs`、`running: cargo test`、`searching TODO`。
///
/// 工具名 → 动词；再从 `arguments` JSON 取一个简短目标（路径 basename / 命令首段 / 模式 / host），
/// 整体裁剪到 ~40 列以保证与 spinner 共享一行不溢出。本地从 `record.arguments` 做轻量提取
/// （不依赖 `tool_card.rs`），避免跨模块耦合。
fn tool_phase_label(tool_name: &str, arguments: &str) -> String {
    let args: Option<serde_json::Value> = serde_json::from_str(arguments).ok();
    let obj = args.as_ref().and_then(|v| v.as_object());
    let str_arg = |key: &str| -> Option<String> {
        obj.and_then(|o| o.get(key))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let basename = |key: &str| -> Option<String> {
        str_arg(key).map(|p| {
            p.trim_end_matches('/')
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&p)
                .to_string()
        })
    };
    let with_target = |verb: &str, target: Option<String>| -> String {
        match target.filter(|t| !t.is_empty()) {
            Some(t) => format!("{verb} {t}"),
            None => verb.to_string(),
        }
    };

    let label = match tool_name {
        "read" | "read_file" => with_target("reading", basename("path")),
        "write" | "write_file" => with_target("writing", basename("path")),
        "edit" | "edit_file" => with_target("editing", basename("path")),
        "ls" | "list_dir" => {
            let target = basename("path").or_else(|| basename("dir"));
            with_target(
                "listing",
                target
                    .filter(|t| !t.is_empty())
                    .or_else(|| Some(".".to_string())),
            )
        }
        "find" | "glob_files" | "glob" => "finding files".to_string(),
        "grep" | "search_files" => {
            with_target("searching", str_arg("pattern").or_else(|| str_arg("query")))
        }
        "bash" | "run_command" => {
            let cmd = str_arg("command")
                .map(|c| c.lines().next().unwrap_or("").trim().to_string())
                .filter(|c| !c.is_empty());
            match cmd {
                Some(c) => format!("running: {c}"),
                None => "running command".to_string(),
            }
        }
        "web_fetch" => {
            let host = str_arg("url").and_then(|u| host_of(&u));
            with_target("fetching", host)
        }
        "skill" | "skill_activate" => with_target(
            "activating skill",
            str_arg("name").or_else(|| str_arg("skill")),
        ),
        other => format!("running {other}"),
    };
    clip(&label, 40)
}

/// 从一个 URL 里取 host（无 scheme 也能容错地取首段）。失败返回 `None`。
fn host_of(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split('@').next_back().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
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
    // skill 工具用 name/skill 作为代表(直接显示技能名,不加 key= 前缀)。
    if tool_name == "skill" || tool_name == "skill_activate" {
        if let Some(name) = obj
            .get("name")
            .or_else(|| obj.get("skill"))
            .and_then(|v| v.as_str())
        {
            return clip(name, 80);
        }
    }
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
    if !matches!(
        record.name.as_str(),
        "write" | "edit" | "write_file" | "edit_file"
    ) {
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
    clipboard
        .set_text(text.to_string())
        .map_err(|e| e.to_string())
}

/// 受支持的图片扩展名（拖入路径检测用）。
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// 把一个**可能是终端拖入/输入**的路径 token 还原为真实路径串。
///
/// 处理两类转义（macOS Terminal.app / iTerm2 拖入文件时的行为）：
/// - 去掉首尾成对的单引号或双引号（`'/a b.png'` → `/a b.png`）。
/// - 反斜杠转义：`\<char>` → `<char>`（`/a\ b.png` → `/a b.png`，`\(` → `(` 等）。
///
/// 不存在的引号 / 转义则原样返回。纯函数，便于单测。
fn dequote_unescape_path(token: &str) -> String {
    let trimmed = token.trim();
    // 去成对引号。
    let inner = if (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
        || (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    // 反斜杠转义还原（见 backslash_escapes：Windows 上 `\` 是路径分隔符，仅转义空白）。
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some(next) if backslash_escapes(next) => {
                    out.push(next);
                    chars.next();
                }
                // Windows 路径分隔符：`\` 后跟普通字符 → 字面保留（`C:\dir\file`）。
                Some(_) => out.push('\\'),
                // 末尾孤立的反斜杠：丢弃。
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// 反斜杠是否转义其后字符。非 Windows 沿用 macOS/iTerm 拖入语义（`\<任意>` 皆转义，
/// 覆盖 `\ ` / `\(` 等）；Windows 上 `\` 主要是路径分隔符，仅当其后是 shell 特殊字符
/// （空白、括号、引号等，即"非路径安全字符"）时才视为转义，保证 `C:\dir\file.png`
/// 里 `\<字母/数字>` 保留为字面分隔符不被拆坏。
#[cfg(windows)]
fn backslash_escapes(next: char) -> bool {
    // 路径安全字符：字母数字（含 CJK 等 unicode）、以及路径里常见的 `_ . - / \ :`。
    // 其后跟这些字符时 `\` 视为字面分隔符；否则（空格、`()` 等）才当转义。
    !(next.is_alphanumeric() || matches!(next, '_' | '.' | '-' | '/' | '\\' | ':'))
}
#[cfg(not(windows))]
fn backslash_escapes(_next: char) -> bool {
    true
}

/// 判断一个路径串是否指向一个**存在的本地图片文件**（扩展名 ∈ [`IMAGE_EXTENSIONS`] 且 `is_file()`）。
fn is_existing_image_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    let p = std::path::Path::new(path);
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            IMAGE_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false);
    ext_ok && p.is_file()
}

/// 扫描提交文本，把其中指向**存在的本地图片文件**的空白分隔 token 识别为附件。
///
/// 规则（对齐 Claude Code 拖入 / `@path` 行为）：
/// - 按空白切分；每个 token 先去引号 + 反斜杠转义还原（[`dequote_unescape_path`]）。
/// - 额外接受前导 `@`（`@img.png` 形式）：剥掉 `@` 后再判定。
/// - 仅当还原后的路径**存在且是图片文件**时识别为附件；否则 token 原样保留。
///
/// 返回 `(替换后的文本, 新识别出的图片路径列表)`。识别到的 token 在文本里被替换为 `[Image #N]`
/// 占位符，N 从 `start_number` 起按出现顺序递增。其余文本（含换行 / 连续空白）原样保留。纯函数，便于单测。
fn extract_image_paths_from_text(text: &str, start_number: usize) -> (String, Vec<PathBuf>) {
    let mut images = Vec::new();
    let mut next_number = start_number;
    let mut out = String::with_capacity(text.len());
    // 手动按「空白段 / 非空白段（token）」交替扫描，保留原始空白（含换行、连续空格）。
    let mut chars = text.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            // 收集空白段原样输出。
            while let Some(&(_, w)) = chars.peek() {
                if w.is_whitespace() {
                    out.push(w);
                    chars.next();
                } else {
                    break;
                }
            }
            continue;
        }
        // 收集一个 token。若以引号开头，吞到配对引号为止（支持含空格的引号路径，
        // 如 `'/a b.png'`）；否则吞到下一个空白（反斜杠转义的空格 `\ ` 在 dequote 阶段还原）。
        let mut end = start;
        let quote = if c == '\'' || c == '"' { Some(c) } else { None };
        if let Some(q) = quote {
            // 吞掉开引号本身。
            chars.next();
            end = start + c.len_utf8();
            let mut closed = false;
            while let Some(&(idx, t)) = chars.peek() {
                end = idx + t.len_utf8();
                chars.next();
                if t == q {
                    closed = true;
                    break;
                }
            }
            // 未闭合引号：回退到普通 token 语义不值得，直接用已吞到的内容。
            let _ = closed;
        } else {
            // 反斜杠转义的空白（`\ `）属于 token 一部分，不在此断开（drag 路径常见形态）。
            // Windows 上 `\` 是路径分隔符（`C:\dir\file`），不吞掉其后普通字符（见 backslash_escapes）。
            while let Some(&(idx, t)) = chars.peek() {
                if t == '\\' {
                    end = idx + t.len_utf8();
                    chars.next();
                    if let Some(&(nidx, n)) = chars.peek() {
                        if backslash_escapes(n) {
                            // `\<空白>` 等转义：连同被转义字符并入 token，不在此断开。
                            end = nidx + n.len_utf8();
                            chars.next();
                        }
                        // 否则 `\` 是字面分隔符（Windows 路径）：下一轮正常处理 n。
                    }
                    continue;
                }
                if t.is_whitespace() {
                    break;
                }
                end = idx + t.len_utf8();
                chars.next();
            }
        }
        let token = &text[start..end];
        let candidate = token.strip_prefix('@').unwrap_or(token);
        let resolved = dequote_unescape_path(candidate);
        if is_existing_image_path(&resolved) {
            out.push_str(&format!("[Image #{next_number}]"));
            images.push(PathBuf::from(resolved));
            next_number += 1;
        } else {
            out.push_str(token);
        }
    }

    (out, images)
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

/// footer 在 Generating 态的状态串：`generating m:ss · Esc to cancel`。
///
/// 用已用时长（[`App::generating_elapsed`]，单调时钟）格式化为 `m:ss`，让用户在一轮里看到时间
/// 累加。纯函数，便于用注入的 `elapsed` 做确定性单测（不依赖真实时间流逝）。`Esc to cancel`
/// 始终保留。
fn format_generating_status(elapsed: std::time::Duration) -> String {
    format!("generating {} · Esc to cancel", format_mmss(elapsed))
}

/// 把时长格式化为 `m:ss`（分:秒，秒补零），如 `0:07` / `1:23` / `12:05`。≥60 分钟时分位自然增大
/// （`75:00`），不折成小时——一轮生成几乎不会到那个量级。
fn format_mmss(elapsed: std::time::Duration) -> String {
    let total = elapsed.as_secs();
    let minutes = total / 60;
    let seconds = total % 60;
    format!("{minutes}:{seconds:02}")
}

/// footer 的上下文窗口占用指示（FIX 3）。
///
/// - 已知窗口（`window = Some(limit)`）且有最近 input_tokens：`ctx 61.2k/128k (48%)`。
/// - 窗口未知（`None`）但有 token 数：仅 `ctx 61.2k`（优雅降级，不显示 %/上限）。
/// - 无 token 数：`None`（footer 不加这一段）。
///
/// 纯函数，便于单测；`window` 为 `Some(0)` 视为未知（避免除零）。
fn format_context_usage(tokens: Option<u64>, window: Option<usize>) -> Option<String> {
    let tokens = tokens?;
    match window {
        Some(limit) if limit > 0 => {
            let pct = ((tokens as f64 / limit as f64) * 100.0).round() as u64;
            Some(format!(
                "ctx {}/{} ({pct}%)",
                human_tokens(tokens),
                human_tokens_round(limit as u64)
            ))
        }
        _ => Some(format!("ctx {}", human_tokens(tokens))),
    }
}

/// 把 token 数折成 `1.2k` / `1.0M` 风格的短串（与事件循环侧 `human_tokens` 同义；这里独立一份
/// 避免跨模块暴露内部 helper）。≥1,000,000 用 megatokens（`1048576 -> 1.0M`），≥1000 用 `k`，
/// 更小的原样显示。
fn human_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// 上下文窗口上限的短串：megatokens（`1048576 -> 1.0M`）；整 k 值省略小数（`128k`，不是
/// `128.0k`），非整 k 保留一位小数。
fn human_tokens_round(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        if n % 1000 == 0 {
            format!("{}k", n / 1000)
        } else {
            format!("{:.1}k", n as f64 / 1000.0)
        }
    } else {
        n.to_string()
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
/// - 流式中（`streaming=true`）：`┄ thinking` 头 + 最近 ~3 行推理，整体 dim+italic、按宽折行，
///   让用户看到模型「在想什么」但视觉上从属于答案（次要）。
/// - 完成后（`streaming=false`）：折叠为一行 `┄ thought for {N}s`（用 finalize 时快照的耗时），
///   无耗时则 `┄ thought for a moment`（无 emoji）。
///
/// 调用方已 gate「无 reasoning 不渲染」，故此处不再判空。
fn render_reasoning(
    reasoning: &str,
    streaming: bool,
    thought_secs: Option<u64>,
    width: u16,
) -> Vec<String> {
    let dim: ColorFn = Arc::new(|s: &str| format!("\x1b[2;3m{s}\x1b[0m"));
    if streaming {
        let mut lines: Vec<String> = Vec::new();
        // `┄ thinking` 头，标明这是推理（而非答案）。
        let mut header = Text::new("┄ thinking".to_string(), 1, 0, None);
        for line in header.render(width) {
            lines.push(dim(&line));
        }
        // 仅显示最近 ~3 行非空推理（保持次要、不喧宾夺主）。
        let recent: Vec<&str> = reasoning
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .collect();
        let start = recent.len().saturating_sub(REASONING_TAIL_LINES);
        for raw in &recent[start..] {
            let mut t = Text::new(format!("  {raw}"), 1, 0, None);
            for line in t.render(width) {
                lines.push(dim(&line));
            }
        }
        lines
    } else {
        let summary = match thought_secs {
            Some(secs) => format!("┄ thought for {secs}s"),
            None => "┄ thought for a moment".to_string(),
        };
        let mut t = Text::new(summary, 1, 0, None);
        t.render(width).into_iter().map(|l| dim(&l)).collect()
    }
}

/// 流式 reasoning 块显示的最近行数（保持次要、不喧宾夺主）。
const REASONING_TAIL_LINES: usize = 3;

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
        assert_eq!(
            effect,
            AppEffect::Submitted {
                text: "do a thing".to_string(),
                images: Vec::new(),
            }
        );
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
    fn slash_new_yields_new_conversation_effect() {
        // `/new` must NOT clear the transcript in-place anymore — it returns a
        // NewConversation effect so the event loop can also reset the runtime
        // (runtime_messages / session / ctx gauge), not just the on-screen text.
        let mut a = app();
        type_str(&mut a, "hi");
        a.handle_key("\r");
        assert!(a.transcript_len() > 0);
        type_str(&mut a, "/new");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::NewConversation);
    }

    #[test]
    fn slash_clear_yields_new_conversation_effect() {
        let mut a = app();
        type_str(&mut a, "hi");
        a.handle_key("\r");
        type_str(&mut a, "/clear");
        assert_eq!(a.handle_key("\r"), AppEffect::NewConversation);
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
    fn slash_init_submits_canned_prompt() {
        let mut a = app();
        type_str(&mut a, "/init");
        let effect = a.handle_key("\r");
        assert_eq!(
            effect,
            AppEffect::Submitted {
                text: INIT_PROMPT.to_string(),
                images: Vec::new(),
            }
        );
        // The INIT prompt is recorded as the turn's user message.
        assert_eq!(a.last_submitted(), Some(INIT_PROMPT));
    }

    // ---- image input: drag/path parsing + placeholder numbering ----

    /// 在临时目录建一个真实的 `.png` 文件，返回其绝对路径串。
    fn temp_png(tag: &str) -> String {
        let mut dir = std::env::temp_dir();
        dir.push(format!("kivio-code-imgtest-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir temp");
        let path = dir.join(format!("{tag}.png"));
        // 1x1 PNG 不必有效——检测只看扩展名 + 文件存在。
        std::fs::write(&path, b"not-a-real-png-but-exists").expect("write png");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn dequote_unescape_strips_quotes_and_backslashes() {
        assert_eq!(dequote_unescape_path("/a/b.png"), "/a/b.png");
        assert_eq!(dequote_unescape_path("'/a b/c.png'"), "/a b/c.png");
        assert_eq!(dequote_unescape_path("\"/a b/c.png\""), "/a b/c.png");
        assert_eq!(dequote_unescape_path("/a\\ b/c.png"), "/a b/c.png");
        assert_eq!(dequote_unescape_path("/x\\(1\\).png"), "/x(1).png");
    }

    #[test]
    fn is_existing_image_path_checks_extension_and_existence() {
        let png = temp_png("exists");
        assert!(is_existing_image_path(&png));
        // Non-existent path with image extension.
        assert!(!is_existing_image_path("/nope/missing.png"));
        // Existing file but non-image extension is rejected.
        let txt = {
            let mut p = std::path::PathBuf::from(&png);
            p.set_extension("txt");
            std::fs::write(&p, b"x").unwrap();
            p.to_string_lossy().into_owned()
        };
        assert!(!is_existing_image_path(&txt));
    }

    #[test]
    fn extract_converts_real_image_path_to_placeholder() {
        let png = temp_png("convert");
        let text = format!("describe {png} please");
        let (rewritten, paths) = extract_image_paths_from_text(&text, 1);
        assert_eq!(rewritten, "describe [Image #1] please");
        assert_eq!(paths, vec![PathBuf::from(&png)]);
    }

    #[test]
    fn extract_leaves_nonexistent_and_nonimage_tokens_untouched() {
        let text = "see /nope/x.png and notes.txt and just-words";
        let (rewritten, paths) = extract_image_paths_from_text(text, 1);
        assert_eq!(rewritten, text);
        assert!(paths.is_empty());
    }

    #[test]
    fn extract_handles_quoted_and_space_escaped_paths() {
        // Build an image file whose path contains a space.
        let mut dir = std::env::temp_dir();
        dir.push(format!("kivio-code-imgspace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("my shot.png");
        std::fs::write(&path, b"x").unwrap();
        let path_str = path.to_string_lossy().into_owned();

        // Single-quoted form (terminal drag style).
        let quoted = format!("look at '{path_str}'");
        let (rewritten, paths) = extract_image_paths_from_text(&quoted, 1);
        assert_eq!(rewritten, "look at [Image #1]");
        assert_eq!(paths, vec![path.clone()]);

        // Backslash-escaped space form.
        let escaped = format!("look at {}", path_str.replace(' ', "\\ "));
        let (rewritten2, paths2) = extract_image_paths_from_text(&escaped, 1);
        assert_eq!(rewritten2, "look at [Image #1]");
        assert_eq!(paths2, vec![path]);
    }

    #[test]
    fn extract_at_prefixed_path_is_attached() {
        let png = temp_png("atpath");
        let text = format!("@{png}");
        let (rewritten, paths) = extract_image_paths_from_text(&text, 2);
        assert_eq!(rewritten, "[Image #2]");
        assert_eq!(paths, vec![PathBuf::from(&png)]);
    }

    #[test]
    fn extract_numbers_multiple_images_from_start() {
        let a = temp_png("multi-a");
        let b = temp_png("multi-b");
        let text = format!("{a} then {b}");
        let (rewritten, paths) = extract_image_paths_from_text(&text, 1);
        assert_eq!(rewritten, "[Image #1] then [Image #2]");
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn attach_pending_image_increments_label() {
        let mut a = app();
        assert_eq!(
            a.attach_pending_image(PathBuf::from("/a.png")),
            "[Image #1]"
        );
        assert_eq!(
            a.attach_pending_image(PathBuf::from("/b.png")),
            "[Image #2]"
        );
        assert_eq!(a.pending_image_count(), 2);
    }

    #[test]
    fn submit_yields_placeholder_text_and_image_list() {
        let png = temp_png("submit");
        let mut a = app();
        type_str(&mut a, &format!("look at {png}"));
        let effect = a.handle_key("\r");
        match effect {
            AppEffect::Submitted { text, images } => {
                assert_eq!(text, "look at [Image #1]");
                assert_eq!(images, vec![PathBuf::from(&png)]);
            }
            other => panic!("expected Submitted, got {other:?}"),
        }
        // Pending images reset after submit so the next message starts at [Image #1].
        assert_eq!(a.pending_image_count(), 0);
    }

    #[test]
    fn submit_carries_clipboard_attachments_then_drag_paths_in_order() {
        let drag = temp_png("order-drag");
        let mut a = app();
        // Simulate a clipboard-pasted image already attached + its placeholder typed.
        let label = a.attach_pending_image(PathBuf::from("/clip/pasted.png"));
        assert_eq!(label, "[Image #1]");
        type_str(&mut a, &format!("{label} and {drag}"));
        let effect = a.handle_key("\r");
        match effect {
            AppEffect::Submitted { text, images } => {
                // Drag path numbering continues after the clipboard attachment.
                assert_eq!(text, "[Image #1] and [Image #2]");
                assert_eq!(
                    images,
                    vec![PathBuf::from("/clip/pasted.png"), PathBuf::from(&drag)]
                );
            }
            other => panic!("expected Submitted, got {other:?}"),
        }
        assert_eq!(a.pending_image_count(), 0);
    }

    #[test]
    fn new_conversation_clears_pending_images() {
        let mut a = app();
        a.attach_pending_image(PathBuf::from("/a.png"));
        a.attach_pending_image(PathBuf::from("/b.png"));
        assert_eq!(a.pending_image_count(), 2);
        a.clear_transcript();
        assert_eq!(a.pending_image_count(), 0);
        // Numbering restarts at 1 after a /new.
        assert_eq!(
            a.attach_pending_image(PathBuf::from("/c.png")),
            "[Image #1]"
        );
    }

    #[test]
    fn slash_init_rejected_while_generating() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        type_str(&mut a, "/init");
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.last_submitted(), None);
    }

    #[test]
    fn slash_mcp_yields_show_mcp_effect() {
        let mut a = app();
        type_str(&mut a, "/mcp");
        assert_eq!(a.handle_key("\r"), AppEffect::ShowMcp);
    }

    #[test]
    fn slash_skill_yields_show_skills_effect() {
        let mut a = app();
        type_str(&mut a, "/skill");
        assert_eq!(a.handle_key("\r"), AppEffect::ShowSkills);
    }

    #[test]
    fn slash_settings_yields_open_settings_effect() {
        let mut a = app();
        type_str(&mut a, "/settings");
        assert_eq!(a.handle_key("\r"), AppEffect::OpenSettings);
    }

    // ---- plan / build agent mode ----

    #[test]
    fn shift_tab_toggles_build_and_plan_when_idle() {
        let mut a = app();
        // default is build.
        assert_eq!(a.agent_mode(), AgentMode::Build);
        // Shift+Tab → plan, and a notice is pushed.
        let effect = a.handle_key("\x1b[Z");
        assert_eq!(effect, AppEffect::None);
        assert_eq!(a.agent_mode(), AgentMode::Plan);
        assert!(a.render(80).join("\n").contains("Plan mode"));
        // Shift+Tab again → back to build.
        a.handle_key("\x1b[Z");
        assert_eq!(a.agent_mode(), AgentMode::Build);
        assert!(a.render(80).join("\n").contains("Switched to build"));
    }

    #[test]
    fn shift_tab_ignored_while_generating() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.handle_key("\x1b[Z");
        assert_eq!(
            a.agent_mode(),
            AgentMode::Build,
            "no toggle while generating"
        );
    }

    #[test]
    fn shift_tab_ignored_while_overlay_open() {
        let mut a = app();
        a.open_model_selector(vec![(
            "openai:gpt-4o".into(),
            "gpt-4o".into(),
            Some("OpenAI".into()),
        )]);
        assert!(a.overlay_open());
        a.handle_key("\x1b[Z");
        assert_eq!(
            a.agent_mode(),
            AgentMode::Build,
            "no toggle while overlay open"
        );
    }

    #[test]
    fn slash_plan_and_build_set_mode() {
        let mut a = app();
        type_str(&mut a, "/plan");
        assert_eq!(a.handle_key("\r"), AppEffect::None);
        assert_eq!(a.agent_mode(), AgentMode::Plan);

        type_str(&mut a, "/build");
        assert_eq!(a.handle_key("\r"), AppEffect::None);
        assert_eq!(a.agent_mode(), AgentMode::Build);
    }

    #[test]
    fn footer_renders_mode_chip() {
        let mut a = app();
        // build chip by default.
        let footer_build = a
            .render(120)
            .into_iter()
            .rfind(|l| l.contains("~/proj"))
            .expect("footer line");
        assert!(
            footer_build.contains("build"),
            "footer shows build chip: {footer_build}"
        );
        // switch to plan → plan chip.
        a.set_agent_mode(AgentMode::Plan);
        let footer_plan = a
            .render(120)
            .into_iter()
            .rfind(|l| l.contains("~/proj"))
            .expect("footer line");
        assert!(
            footer_plan.contains("plan"),
            "footer shows plan chip: {footer_plan}"
        );
    }

    #[test]
    fn open_settings_selector_renders_toggle_state() {
        let mut a = app();
        a.set_read_claude_dir(true);
        a.open_settings_selector();
        assert!(a.overlay_open());
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Settings"), "settings heading present");
        assert!(joined.contains("Read .claude"), "toggle item present");
        assert!(joined.contains("[on]"), "reflects current on state");
        // Esc closes without flipping.
        let effect = a.handle_key("\x1b");
        assert_eq!(effect, AppEffect::None);
        assert!(!a.overlay_open());
        assert!(a.read_claude_dir(), "Esc must not flip the toggle");
    }

    #[test]
    fn settings_toggle_flips_in_memory_value_and_persists() {
        // The toggle writes kivio-code's real config; snapshot + restore it so the
        // test never clobbers the developer's machine config.
        let before = crate::kivio_code::config::load();

        let mut a = app();
        a.set_read_claude_dir(true);
        a.open_settings_selector();
        // Enter on the (single) toggle item flips it off, closes the overlay, and
        // pushes a "Read .claude: off" notice.
        let effect = a.handle_key("\r");
        assert_eq!(effect, AppEffect::None);
        assert!(!a.overlay_open(), "overlay closed after toggle");
        assert!(!a.read_claude_dir(), "in-memory value flipped to off");
        let joined = a.render(80).join("\n");
        assert!(
            joined.contains("Read .claude: off"),
            "notice reflects new state"
        );
        // The new value is persisted so the next turn's system prompt honors it.
        assert!(!crate::kivio_code::config::load().read_claude_dir);

        // Restore the developer's original config.
        let _ = crate::kivio_code::config::save(&before);
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
        // footer line shows cwd + status, but NOT the model (model lives in the welcome header).
        let footer_line = lines
            .iter()
            .rfind(|l| l.contains("~/proj"))
            .expect("footer line present");
        assert!(footer_line.contains("~/proj"), "footer cwd present");
        assert!(footer_line.contains("ready"), "footer status present");
        assert!(
            !footer_line.contains("gpt-4o"),
            "footer must not show the model"
        );
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
    fn approval_request_then_y_approves_and_replies() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.apply_agent_event(AgentUiEvent::ApprovalRequest {
            prompt: "Run bash?".to_string(),
            session: false,
            responder: tx,
        });
        assert!(a.pending_approval.is_some(), "prompt is pending");
        assert_eq!(a.handle_key("y"), AppEffect::None);
        assert!(
            a.pending_approval.is_none(),
            "prompt cleared after decision"
        );
        assert_eq!(rx.recv().unwrap(), true, "approve → true");
    }

    #[test]
    fn approval_request_then_deny_cancels_turn() {
        let mut a = app();
        for key in ["n", "\x1b"] {
            let (tx, rx) = std::sync::mpsc::channel();
            a.apply_agent_event(AgentUiEvent::ApprovalRequest {
                prompt: "Run bash?".to_string(),
                session: false,
                responder: tx,
            });
            assert_eq!(a.handle_key(key), AppEffect::Cancel, "deny cancels turn");
            assert_eq!(rx.recv().unwrap(), false, "deny → false");
        }
    }

    #[test]
    fn session_consent_is_cached_after_first_yes() {
        let mut a = app();
        // First session-consent request: prompts, user approves.
        let (tx1, rx1) = std::sync::mpsc::channel();
        a.apply_agent_event(AgentUiEvent::ApprovalRequest {
            prompt: "Allow FS/shell?".to_string(),
            session: true,
            responder: tx1,
        });
        assert!(a.pending_approval.is_some());
        a.handle_key("y");
        assert_eq!(rx1.recv().unwrap(), true);

        // Second session-consent request: auto-approved, no prompt shown.
        let (tx2, rx2) = std::sync::mpsc::channel();
        a.apply_agent_event(AgentUiEvent::ApprovalRequest {
            prompt: "Allow FS/shell?".to_string(),
            session: true,
            responder: tx2,
        });
        assert!(
            a.pending_approval.is_none(),
            "cached session consent skips the prompt"
        );
        assert_eq!(rx2.recv().unwrap(), true, "auto-approved from cache");
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
        assert_eq!(
            a.tool_card("call_1").unwrap().status,
            ToolCallStatus::Pending
        );

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
        assert_eq!(
            a.tool_card("call_1").unwrap().status,
            ToolCallStatus::Success
        );
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
        assert!(
            joined.contains("new line"),
            "diff rendered in card: {joined}"
        );
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
    fn footer_omits_per_turn_token_usage() {
        // The footer must NOT show the per-turn `N in · N out` usage — the user
        // does not want it. It still carries cwd · status (and ctx when known).
        let mut a = app();
        a.set_context_window(Some(128_000));
        a.set_context_tokens(Some(61_200));
        let lines = a.render(120);
        let joined = lines.join("\n");
        let footer_line = lines
            .iter()
            .rfind(|l| l.contains("~/proj"))
            .expect("footer line present");
        // no per-turn token usage text anywhere in the footer.
        assert!(
            !footer_line.contains(" in "),
            "footer must not show `… in …` usage"
        );
        assert!(
            !footer_line.contains(" out"),
            "footer must not show `… out` usage"
        );
        // cwd + status + ctx are still present.
        assert!(footer_line.contains("~/proj"), "footer cwd present");
        assert!(footer_line.contains("ready"), "footer status present");
        assert!(
            joined.contains("ctx 61.2k/128k (48%)"),
            "footer ctx present"
        );
    }

    // ---- FIX 3: context-window occupancy in footer ----

    #[test]
    fn format_context_usage_known_window_shows_percent() {
        // 61.2k of a 128k window ≈ 48%.
        let s = format_context_usage(Some(61_200), Some(128_000)).unwrap();
        assert_eq!(s, "ctx 61.2k/128k (48%)");
    }

    #[test]
    fn format_context_usage_unknown_window_degrades_to_raw_tokens() {
        let s = format_context_usage(Some(61_200), None).unwrap();
        assert_eq!(s, "ctx 61.2k");
        // a Some(0) window is treated as unknown (no divide-by-zero / 0% noise).
        let s0 = format_context_usage(Some(500), Some(0)).unwrap();
        assert_eq!(s0, "ctx 500");
    }

    #[test]
    fn format_context_usage_no_tokens_is_none() {
        assert!(format_context_usage(None, Some(128_000)).is_none());
        assert!(format_context_usage(None, None).is_none());
    }

    #[test]
    fn human_tokens_formats_megatokens() {
        // ≥ 1,000,000 → megatokens with one decimal.
        assert_eq!(human_tokens(1_048_576), "1.0M");
        assert_eq!(human_tokens(2_000_000), "2.0M");
        // thousands keep `k`.
        assert_eq!(human_tokens(2_200), "2.2k");
        // small raw numbers as-is.
        assert_eq!(human_tokens(512), "512");
    }

    #[test]
    fn human_tokens_round_formats_megatokens() {
        assert_eq!(human_tokens_round(1_048_576), "1.0M");
        assert_eq!(human_tokens_round(2_000_000), "2.0M");
        // integral k stays sans decimal.
        assert_eq!(human_tokens_round(128_000), "128k");
    }

    #[test]
    fn footer_context_window_million_reads_as_megatokens() {
        // A 1,048,576-token window must render as 1.0M, not 1048.6k.
        let s = format_context_usage(Some(2_200), Some(1_048_576)).unwrap();
        assert_eq!(s, "ctx 2.2k/1.0M (0%)");
    }

    #[test]
    fn footer_renders_context_occupancy_when_window_known() {
        let mut a = app();
        a.set_context_window(Some(128_000));
        a.set_context_tokens(Some(61_200));
        let joined = a.render(120).join("\n");
        assert!(
            joined.contains("ctx 61.2k/128k (48%)"),
            "footer shows ctx occupancy: {joined}"
        );
    }

    #[test]
    fn footer_degrades_when_context_window_unknown() {
        let mut a = app();
        a.set_context_window(None);
        a.set_context_tokens(Some(61_200));
        let joined = a.render(120).join("\n");
        assert!(
            joined.contains("ctx 61.2k"),
            "raw tokens only when window unknown"
        );
        assert!(!joined.contains('%'), "no percent without a known window");
    }

    // ---- Phase 5c: model selector / overlay ----

    fn model_items() -> Vec<(String, String, Option<String>)> {
        vec![
            (
                "openai:gpt-4o".into(),
                "gpt-4o".into(),
                Some("OpenAI".into()),
            ),
            (
                "anthropic:claude".into(),
                "claude".into(),
                Some("Anthropic".into()),
            ),
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
        assert_eq!(
            effect,
            AppEffect::ModelSelected("anthropic:claude".to_string())
        );
        assert!(!a.overlay_open(), "overlay closed after confirm");
    }

    #[test]
    fn model_selector_starts_at_current_model() {
        let mut a = App::new("~".into(), "anthropic:claude".into());
        a.set_terminal_rows(24);
        a.open_model_selector(model_items());
        // current is the second item → selecting immediately returns it.
        let effect = a.handle_key("\r");
        assert_eq!(
            effect,
            AppEffect::ModelSelected("anthropic:claude".to_string())
        );
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
    fn set_model_updates_resolution_value_and_display_independently() {
        let mut a = app();
        // The resolution value (used by the model selector / resume) is id-based…
        a.set_model("anthropic:claude-3");
        assert_eq!(a.model(), "anthropic:claude-3");
        // …while the footer renders the separate human-readable display label (FIX 2).
        a.set_model_display("Anthropic · claude-3");
        let joined = a.render(80).join("\n");
        assert!(
            joined.contains("Anthropic · claude-3"),
            "footer shows display label"
        );
        // resolution value still resolves the provider id, unaffected by display.
        assert_eq!(a.model(), "anthropic:claude-3");
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
        assert_eq!(
            effect,
            AppEffect::SessionSelected("/path/to/a.jsonl".to_string())
        );
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

    // ---- phase-aware spinner label ----

    fn record_with_args(
        name: &str,
        status: ToolCallStatus,
        args: serde_json::Value,
    ) -> ToolCallRecord {
        let mut r = tool_record("call_x", name, status);
        r.arguments = args.to_string();
        r
    }

    #[test]
    fn phase_label_defaults_to_thinking() {
        let a = app();
        assert_eq!(a.phase_label(), "thinking…");
    }

    #[test]
    fn running_list_dir_sets_listing_phase() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "list_dir",
            ToolCallStatus::Running,
            serde_json::json!({ "path": "src/kivio_code" }),
        ))));
        assert_eq!(a.phase_label(), "listing kivio_code");
    }

    #[test]
    fn running_ls_without_path_lists_cwd() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "ls",
            ToolCallStatus::Running,
            serde_json::json!({}),
        ))));
        assert_eq!(a.phase_label(), "listing .");
    }

    #[test]
    fn running_read_file_sets_reading_basename() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "read_file",
            ToolCallStatus::Pending,
            serde_json::json!({ "path": "/abs/path/to/main.rs" }),
        ))));
        assert_eq!(a.phase_label(), "reading main.rs");
    }

    #[test]
    fn running_run_command_sets_running_cmd_head() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "run_command",
            ToolCallStatus::Running,
            serde_json::json!({ "command": "cargo test\nsecond line" }),
        ))));
        assert_eq!(a.phase_label(), "running: cargo test");
    }

    #[test]
    fn running_grep_sets_searching_pattern() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "search_files",
            ToolCallStatus::Running,
            serde_json::json!({ "pattern": "TODO" }),
        ))));
        assert_eq!(a.phase_label(), "searching TODO");
    }

    #[test]
    fn running_web_fetch_sets_fetching_host() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "web_fetch",
            ToolCallStatus::Running,
            serde_json::json!({ "url": "https://example.com/path?q=1" }),
        ))));
        assert_eq!(a.phase_label(), "fetching example.com");
    }

    #[test]
    fn unknown_tool_uses_running_tool_name() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "mcp_custom_tool",
            ToolCallStatus::Running,
            serde_json::json!({}),
        ))));
        assert_eq!(a.phase_label(), "running mcp_custom_tool");
    }

    #[test]
    fn answer_delta_sets_responding() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "Here is".to_string(),
            reasoning: String::new(),
        });
        assert_eq!(a.phase_label(), "responding…");
    }

    #[test]
    fn reasoning_only_delta_stays_thinking() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: String::new(),
            reasoning: "considering".to_string(),
        });
        assert_eq!(a.phase_label(), "thinking…");
    }

    #[test]
    fn tool_success_reverts_phase_to_default() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "read_file",
            ToolCallStatus::Running,
            serde_json::json!({ "path": "main.rs" }),
        ))));
        assert_eq!(a.phase_label(), "reading main.rs");
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "read_file",
            ToolCallStatus::Success,
            serde_json::json!({ "path": "main.rs" }),
        ))));
        assert_eq!(a.phase_label(), "thinking…");
    }

    #[test]
    fn new_turn_resets_phase_label() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "run_command",
            ToolCallStatus::Running,
            serde_json::json!({ "command": "ls" }),
        ))));
        assert_eq!(a.phase_label(), "running: ls");
        // End the turn, then start a fresh one — the stale "running: ls" must clear.
        a.set_mode(AppMode::Idle);
        a.set_mode(AppMode::Generating);
        assert_eq!(a.phase_label(), "thinking…");
    }

    #[test]
    fn phase_label_long_target_is_width_trimmed() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "run_command",
            ToolCallStatus::Running,
            serde_json::json!({ "command": "a".repeat(200) }),
        ))));
        assert!(
            a.phase_label().chars().count() <= 41,
            "trimmed to ~40 + ellipsis"
        );
        assert!(a.phase_label().ends_with('…'));
    }

    #[test]
    fn spinner_line_shows_phase_label() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::ToolRecord(Box::new(record_with_args(
            "read_file",
            ToolCallStatus::Running,
            serde_json::json!({ "path": "lib.rs" }),
        ))));
        let joined = a.render(80).join("\n");
        assert!(
            joined.contains("reading lib.rs"),
            "spinner shows current phase: {joined}"
        );
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
        assert!(
            joined.contains("It is empty."),
            "assistant message restored"
        );
        // tool card present and carries its result via summary/path.
        assert!(joined.contains("main.rs"), "tool card restored");
        assert_eq!(
            a.tool_card("call_1").map(|c| c.status.clone()),
            Some(ToolCallStatus::Success)
        );

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
        assert_eq!(
            a.transcript_shape(),
            vec![("assistant", "Hello".to_string())]
        );
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
        assert!(
            !a.assistant_streaming(),
            "all turn segments sealed after Done"
        );
    }

    // ---- BUG 2: branded welcome header ----

    #[test]
    fn initial_render_shows_welcome_header() {
        let mut a = app();
        let joined = a.render(80).join("\n");
        assert!(joined.contains("Kivio Code"), "title present");
        assert!(
            joined.contains(env!("CARGO_PKG_VERSION")),
            "version present"
        );
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
        assert!(
            joined.contains("considering options"),
            "reasoning visible while streaming"
        );
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
        assert!(
            joined.contains("thought for a moment"),
            "collapsed thought summary shown"
        );
        assert!(
            !joined.contains("long chain of thought"),
            "full reasoning hidden once done"
        );
        // No emoji in the collapsed reasoning line (the 💭 was removed).
        assert!(!joined.contains('💭'), "reasoning line is emoji-free");
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
        assert!(
            slash_candidates("/model gpt").is_none(),
            "args → no command completion"
        );
        assert!(
            slash_candidates("line\n/model").is_none(),
            "multiline → no completion"
        );
    }

    #[test]
    fn typing_slash_opens_popup_and_renders() {
        let mut a = app();
        type_str(&mut a, "/mo");
        let joined = a.render(80).join("\n");
        assert!(joined.contains("/model"), "popup lists matching command");
        assert!(
            joined.contains("Switch the active model"),
            "popup shows description"
        );
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
        assert!(
            !joined.contains("Switch the active model"),
            "popup dismissed"
        );
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

    // ---- B3: live elapsed timer in the footer while generating ----

    #[test]
    fn format_mmss_pads_seconds_and_grows_minutes() {
        use std::time::Duration;
        assert_eq!(format_mmss(Duration::from_secs(0)), "0:00");
        assert_eq!(format_mmss(Duration::from_secs(7)), "0:07");
        assert_eq!(format_mmss(Duration::from_secs(72)), "1:12");
        assert_eq!(format_mmss(Duration::from_secs(725)), "12:05");
    }

    #[test]
    fn format_generating_status_shows_elapsed_and_cancel_hint() {
        use std::time::Duration;
        let s = format_generating_status(Duration::from_secs(12));
        assert!(s.contains("0:12"), "elapsed mm:ss present: {s}");
        assert!(s.contains("Esc to cancel"), "cancel hint retained: {s}");
        assert!(s.starts_with("generating"), "labelled generating: {s}");
    }

    #[test]
    fn footer_counts_up_while_generating_and_reverts_when_idle() {
        use std::time::Duration;
        let mut a = app();
        a.set_mode(AppMode::Generating);
        // Deterministically pin the elapsed via the test seam (no real waiting).
        a.set_generating_elapsed_for_test(Duration::from_secs(12));
        let footer = a
            .render(120)
            .into_iter()
            .rfind(|l| l.contains("~/proj"))
            .expect("footer line");
        assert!(
            footer.contains("0:12"),
            "footer shows elapsed mm:ss: {footer}"
        );
        assert!(
            footer.contains("Esc to cancel"),
            "footer keeps cancel hint: {footer}"
        );

        // Back to idle → timer cleared, status reverts to the static ready text.
        a.set_mode(AppMode::Idle);
        assert!(a.generating_elapsed().is_none(), "timer cleared on idle");
        let footer_idle = a
            .render(120)
            .into_iter()
            .rfind(|l| l.contains("~/proj"))
            .expect("footer line");
        assert!(
            footer_idle.contains("ready"),
            "idle footer reverts: {footer_idle}"
        );
        assert!(
            !footer_idle.contains("Esc to cancel"),
            "no cancel hint when idle: {footer_idle}"
        );
    }

    // ---- B4: streaming reasoning block + collapsed thought line ----

    #[test]
    fn streaming_reasoning_shows_thinking_block_with_recent_lines() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "the answer".to_string(),
            reasoning: "step one\nstep two\nstep three\nstep four".to_string(),
        });
        let joined = a.render(80).join("\n");
        // `┄ thinking` header + dim/italic + only the most recent ~3 lines.
        assert!(
            joined.contains("┄ thinking"),
            "thinking header shown: {joined}"
        );
        assert!(joined.contains("\x1b[2;3m"), "reasoning dim/italic");
        assert!(joined.contains("step four"), "most recent line shown");
        assert!(joined.contains("step two"), "recent tail shown");
        // The oldest line is dropped (only the last 3 are shown).
        assert!(
            !joined.contains("step one"),
            "oldest reasoning line trimmed: {joined}"
        );
        assert!(
            joined.contains("the answer"),
            "answer still rendered alongside"
        );
    }

    #[test]
    fn finalized_reasoning_collapses_with_elapsed_seconds() {
        let mut a = app();
        a.set_mode(AppMode::Generating);
        a.set_generating_elapsed_for_test(std::time::Duration::from_secs(9));
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "answer".to_string(),
            reasoning: "deliberating".to_string(),
        });
        // Finalize captures the elapsed BEFORE the event loop flips back to idle.
        a.apply_agent_event(AgentUiEvent::Done {
            message_id: "m1".to_string(),
            reason: "completed".to_string(),
        });
        a.set_mode(AppMode::Idle);
        let joined = a.render(80).join("\n");
        assert!(
            joined.contains("thought for 9s"),
            "collapsed with elapsed: {joined}"
        );
        assert!(
            !joined.contains("deliberating"),
            "full reasoning hidden once done"
        );
    }

    #[test]
    fn finalized_reasoning_without_timer_falls_back_to_moment() {
        // No generation timer (set_mode(Generating) never called) → graceful fallback.
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "answer".to_string(),
            reasoning: "thinking it over".to_string(),
        });
        a.apply_agent_event(AgentUiEvent::Done {
            message_id: "m1".to_string(),
            reason: "completed".to_string(),
        });
        let joined = a.render(80).join("\n");
        assert!(
            joined.contains("thought for a moment"),
            "fallback summary: {joined}"
        );
    }

    #[test]
    fn no_reasoning_block_when_no_reasoning_produced() {
        let mut a = app();
        a.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m1".to_string(),
            delta: "just an answer".to_string(),
            reasoning: String::new(),
        });
        a.apply_agent_event(AgentUiEvent::Done {
            message_id: "m1".to_string(),
            reason: "completed".to_string(),
        });
        let joined = a.render(80).join("\n");
        assert!(joined.contains("just an answer"), "answer rendered");
        assert!(!joined.contains("thinking"), "no thinking block: {joined}");
        assert!(
            !joined.contains("thought for"),
            "no collapsed thought line: {joined}"
        );
    }
}
