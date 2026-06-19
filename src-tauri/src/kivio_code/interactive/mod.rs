//! 交互模式 —— 事件循环 + 输入线程 + 差分渲染协调 + agent loop 接线（Phase 5b）。
//!
//! Phase 4 的 TUI 库（差分渲染 [`Tui`]、组件树、键解码）接到真实终端，跑一个事件循环；Phase 5b 在此
//! 之上把 [`run_agent_loop`](crate::chat::agent::run_agent_loop) 接进来：提交一条消息会在 tokio
//! runtime 上 **后台跑一整轮 agent**，流式 / 工具记录 / 完成事件通过 [`AgentUiEvent`] 通道回到本
//! 事件循环，折叠进 [`App`] 并差分重绘。
//!
//! ## 三路事件汇入一个循环
//! 1. **输入线程**：一条专用 OS 线程在 raw 模式下阻塞 `read` stdin 原始字节，喂给 [`StdinBuffer`]，
//!    把切出的完整序列 / 粘贴段通过 [`mpsc`](std::sync::mpsc) 发到主循环（[`InputEvent`]）。
//! 2. **agent-event 通道**：后台 agent 任务通过 [`InteractiveAgentHost`] 把 [`AgentUiEvent`] 发到
//!    第二条 mpsc；主循环在同一个 `recv_timeout` tick 里 drain 它。
//! 3. **resize**：`recv_timeout` 超时分支轮询 `crossterm::terminal::size()`，变化则全量重绘。
//!
//! ## 一轮 agent turn 的生命周期
//! `AppEffect::Submitted(text)` → 把 user 消息持久化进 session、累积进 `runtime_messages` →
//! 取一个新 generation 建 [`RunCancel`] → 在 tokio runtime 上 `spawn` 一个任务跑 `run_agent_loop`
//! （host = [`InteractiveAgentHost`]，executor = [`CliToolExecutor`]）→ 主循环进入 `Generating`，
//! drain `AgentUiEvent` 重绘 → 任务完成把 [`AgentRunResult`] 通过结果通道送回 → 主循环持久化
//! assistant 消息 + 工具调用、累积 `runtime_messages`、刷新 footer usage，回到 `Idle`。
//!
//! ## 取消 / 多轮
//! - **取消**：Esc / generating 中 Ctrl+C → `AppEffect::Cancel` → 翻 [`RunCancel`]；loop 的
//!   `is_generation_active` 转 false，在下一个检查点停并返回 `Err("cancelled")`。
//! - **多轮**：`runtime_messages` 在 [`TurnRuntime`] 里跨轮累积，每次新提交都带上完整上下文。

pub mod agent_host;
pub mod app;
pub mod slash;
pub mod tool_card;

pub use agent_host::{AgentUiEvent, Generations, InteractiveAgentHost, RunCancel};
pub use app::{App, AppEffect, AppMode, AgentMode, ToolCard, ToolCardPlaceholder};

use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::chat::agent::run_agent_loop;
use crate::chat::agent::types::AgentRunResult;
use crate::chat::types::{ToolCallRecord, ToolCallStatus};
use crate::kivio_code::errors;
use crate::kivio_code::executor::CliToolExecutor;
use crate::kivio_code::session::{Session, SessionRecord};
use crate::kivio_code::{build_app_state, load_settings_from_disk, TurnAssembly};
use crate::state::AppState;

use super::tui::render::{Component, Tui};
use super::tui::stdin_buffer::StdinBuffer;
use super::tui::terminal::{CrosstermTerminal, RawModeGuard, Terminal};
use super::tui::text_width::truncate_to_width;

/// 输入线程发给主循环的事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// 一段完整的输入序列（一个按键 / 转义序列的原始字节串）。
    Key(String),
    /// 一段 bracketed paste 的内容。
    Paste(String),
    /// 终端 resize（携带新尺寸）。
    Resize(u16, u16),
    /// stdin 已 EOF / 关闭，输入线程即将退出。
    Eof,
}

/// 渲染一帧：清掉旧子组件，挂一个一次性的 [`AppFrame`] 组件，调用差分渲染器。
struct AppFrame {
    lines: Vec<String>,
}

impl Component for AppFrame {
    fn render(&mut self, _width: u16) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }
}

/// 交互模式的运行选项。由 bin 从 CLI 参数填充。
pub struct InteractiveOptions {
    /// 已折叠 home→`~` 的 cwd 展示串。
    pub cwd_display: String,
    /// 形如 `provider:model` 的模型展示串（已 resolve；`<no model>` 表示未配置）。
    pub model: String,
    /// agent 实际操作的工作目录（`-C/--cwd` 已解析；workspace + session 根均用它）。
    pub cwd: PathBuf,
    /// `--provider` 覆盖（resolve turn assembly 用）。
    pub provider_override: Option<String>,
    /// `--model` 覆盖（resolve turn assembly 用）。
    pub model_override: Option<String>,
    /// `--no-approve`：禁用敏感工具（write/edit/bash）。
    pub no_approve: bool,
    /// `--verbose`：流式显示 reasoning。
    pub verbose: bool,
    /// 会话续跑请求（`-c/--continue` 或 `-r/--resume <id|path>`）；None = 新会话。
    pub resume: Option<ResumeRequest>,
}

/// 续跑请求：最近一条（`-c`）或指定 id / 路径（`-r`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResumeRequest {
    /// 续跑该 cwd 下最近的会话。
    Recent,
    /// 续跑指定会话：一个 `.jsonl` 路径，或一个（可能是部分前缀的）会话 id。
    Reference(String),
}

/// 后台 agent 任务完成后送回主循环的结果（连同它跑在哪个 generation，便于丢弃过期任务）。
struct TurnDone {
    generation: u64,
    result: Result<AgentRunResult, String>,
    /// 这一轮的 assistant 消息 id（finalize / 持久化用）。
    message_id: String,
}

/// 交互会话的 agent 运行时上下文：跨轮持有 tokio runtime handle、`AppState`、本轮配置装配
/// [`TurnAssembly`]、cwd、generation 计数、累积的 `runtime_messages` 与 JSONL session。
///
/// 把「提交 → spawn agent → 收结果 → 持久化 + 累积上下文」的逻辑收进这里，让 [`run_loop`] 只负责
/// 事件分发，且这套逻辑（除真实 spawn 外）可被单测覆盖（见 `tests`）。
struct TurnRuntime {
    handle: tokio::runtime::Handle,
    state: Arc<AppState>,
    assembly: Arc<TurnAssembly>,
    cwd: PathBuf,
    timeout_ms: u64,
    /// 单调 generation 源：每次提交取下一个，过期的后台任务因 generation 不匹配被忽略。
    generations: Generations,
    /// 当前在跑的取消令牌（None = 空闲）。
    current: Option<RunCancel>,
    /// 这一轮分配的 assistant 消息 id（流式事件用同一 id 定位）。
    current_message_id: Option<String>,
    /// 跨轮累积的 runtime messages（system + 历次 user/assistant/tool）。
    runtime_messages: Vec<Value>,
    /// 持久化用的 JSONL session（best-effort：写失败仅记一条通知，不中断）。
    session: Option<Session>,
    /// 已写进 session 的工具调用 id（避免一个 record 多状态多次落盘）。
    persisted_tool_calls: std::collections::HashSet<String>,
    /// agent 任务把 [`AgentUiEvent`] 发到这里的 Sender（每轮新建一对，clone 给 host）。
    turn_done_tx: Sender<TurnDone>,
}

impl TurnRuntime {
    /// 是否有一轮在跑。
    fn is_generating(&self) -> bool {
        self.current.is_some()
    }

    /// 当前 settings 是否配置了显式视觉模型（图片 mixer 预分析的前提）。
    fn has_vision_model(&self) -> bool {
        self.state.settings_read().has_explicit_vision_model()
    }

    /// 主编码模型自身是否支持视觉。支持时图片**直接**交给主模型（跳过 mixer）——mixer 只是
    /// 给纯文本主模型补视觉的兜底。与 GUI `auxiliary_vision_model_for_images` 的判断对齐。
    fn main_model_supports_vision(&self) -> bool {
        crate::chat::model_metadata::model_supports_vision(
            Some(&self.assembly.provider),
            &self.assembly.model,
        ) == Some(true)
    }

    /// 起一轮 agent turn：把 user 消息持久化 + 累积进 runtime_messages，新建 generation/cancel，
    /// 在 tokio runtime 上 spawn 跑 `run_agent_loop`，事件经 `agent_tx` 回到主循环。
    ///
    /// `plan_mode`（来自 App 的当前 [`AgentMode`]）gate 本轮工具集为只读，并把一条**临时** plan-mode
    /// system note 追加到本轮 `runtime_messages` 的 **克隆**（不污染存储的 `self.runtime_messages`），
    /// 让模型只研究/搜索 + 出方案。
    fn begin_turn(
        &mut self,
        text: String,
        image_paths: Vec<PathBuf>,
        agent_tx: &Sender<AgentUiEvent>,
        plan_mode: bool,
    ) {
        // 持久化 user 消息（best-effort）。
        self.append_session(SessionRecord::Message {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            role: "user".to_string(),
            content: text.clone(),
        });

        // 累积进上下文。
        self.runtime_messages
            .push(json!({ "role": "user", "content": text }));

        let generation = self.generations.next();
        let cancel = RunCancel::new(generation);
        self.current = Some(cancel.clone());
        let message_id = format!("kivio-code-msg-{generation}");
        self.current_message_id = Some(message_id.clone());

        let host = InteractiveAgentHost::new(agent_tx.clone(), cancel);
        let state = self.state.clone();
        let assembly = self.assembly.clone();
        let skill_registry = assembly.skill_registry.clone();
        let chat_tools = assembly.effective_chat_tools.clone();
        // Clone the accumulated context for this turn; in plan mode append a TRANSIENT
        // plan-mode system note to the CLONE only — `self.runtime_messages` is left
        // unchanged so switching back to build doesn't carry a stale plan instruction.
        let mut messages = self.runtime_messages.clone();
        if plan_mode {
            messages.push(json!({
                "role": "system",
                "content": crate::kivio_code::PLAN_SYSTEM_NOTE,
            }));
        }
        let cwd = self.cwd.clone();
        let http = self.state.http.clone();
        let timeout_ms = self.timeout_ms;
        let done_tx = self.turn_done_tx.clone();
        let run_message_id = message_id.clone();
        // Vision mixer pre-analysis context (only when images are attached). Snapshot the
        // settings + the placeholder-substituted user text; the async block runs the aux
        // vision model and injects the textual observations into the per-turn `messages`
        // CLONE (NOT into `self.runtime_messages`), so the text-only coding model "sees"
        // the screenshots without the CLI's main request carrying image parts.
        let vision_settings = self.state.settings_read().clone();
        let vision_text = text;
        let vision_tx = agent_tx.clone();
        // 主模型自身支持视觉时，图片直接 inline 进主请求（跳过 mixer）。在 spawn 前同步算好，
        // 因为它依赖 `self.assembly`（任务里只持有 clone 出来的 owned 数据）。
        let main_supports_vision = self.main_model_supports_vision();

        self.handle.spawn(async move {
            // Step 0: vision handling (if images attached). Runs inside the spawned task so any
            // network call never blocks the UI thread; the per-turn `messages` clone is augmented
            // before the loop builds its config.
            if !image_paths.is_empty() {
                if main_supports_vision {
                    // 主模型自身支持视觉：把图片直接 inline 进最后一条 user 消息，交给主模型，
                    // 不走 mixer（与 GUI 主模型支持视觉时直发图片一致）。无 mixer 卡片。
                    let (parts, errors) =
                        crate::kivio_code::vision::inline_image_parts(&image_paths);
                    if !parts.is_empty() {
                        messages =
                            crate::kivio_code::vision::inject_inline_images(messages, parts);
                    }
                    if !errors.is_empty() {
                        // best-effort：读图失败（极罕见，路径来自刚校验的 pending image）不致命，
                        // 成功的图已 inline；用一张 Error 卡片提示，不静默丢弃。
                        let card_id = format!("kivio-code-vision-{generation}");
                        emit_vision_card(
                            &vision_tx,
                            &card_id,
                            ToolCallStatus::Error,
                            errors.len(),
                            None,
                            Some(errors.join("; ")),
                        );
                    }
                } else {
                    // 主模型不支持视觉：走 mixer 兜底——用显式视觉模型把图片转成文字观察注入。
                    let labels: Vec<String> = (1..=image_paths.len())
                        .map(|n| format!("[Image #{n}]"))
                        .collect();
                    let card_id = format!("kivio-code-vision-{generation}");
                    emit_vision_card(
                        &vision_tx,
                        &card_id,
                        ToolCallStatus::Running,
                        image_paths.len(),
                        None,
                        None,
                    );
                    let outcome = crate::kivio_code::vision::run_vision_mixer(
                        &state,
                        &vision_settings,
                        &labels,
                        &image_paths,
                        &vision_text,
                    )
                    .await;
                    match outcome {
                        crate::kivio_code::vision::VisionMixerOutcome::Analyzed {
                            provider_name,
                            model,
                            observations,
                        } => {
                            messages = crate::kivio_code::vision::inject_vision_observations(
                                messages,
                                &observations,
                            );
                            emit_vision_card(
                                &vision_tx,
                                &card_id,
                                ToolCallStatus::Success,
                                image_paths.len(),
                                Some(format!("{provider_name} · {model}")),
                                None,
                            );
                        }
                        crate::kivio_code::vision::VisionMixerOutcome::NoVisionModel => {
                            // The synchronous gate in `apply_effect` already pushed a Notice and
                            // cleared the images, so this branch should be unreachable; mark the
                            // card skipped defensively rather than leaving it spinning.
                            emit_vision_card(
                                &vision_tx,
                                &card_id,
                                ToolCallStatus::Skipped,
                                image_paths.len(),
                                None,
                                Some("no vision model configured".to_string()),
                            );
                        }
                    }
                }
            }

            let executor = CliToolExecutor::new(
                &cwd,
                http,
                timeout_ms,
                state.clone(),
                skill_registry,
                chat_tools,
            );
            // Build the borrowing config inside the task body so the borrows of the
            // owned `state`/`assembly` Arcs live exactly as long as the loop call.
            let config = assembly.into_config(
                &state,
                "kivio-code".to_string(),
                format!("kivio-code-run-{generation}"),
                run_message_id.clone(),
                generation,
                messages,
                plan_mode,
            );
            let result = run_agent_loop(config, &host, &executor).await;
            let _ = done_tx.send(TurnDone {
                generation,
                result,
                message_id: run_message_id,
            });
        });
    }

    /// 请求取消当前轮（翻 cancel flag；loop 在下个检查点停）。
    fn request_cancel(&self) {
        if let Some(cancel) = &self.current {
            cancel.cancel();
        }
    }

    /// 当前活动模型展示串（`provider:model`，id 形式）——选择器定位 / 续会话解析用。
    fn model_label(&self) -> String {
        self.assembly.model_label()
    }

    /// 当前活动模型的人读展示串（`<Provider Name> · model`）——footer / welcome / 通知用（FIX 2）。
    fn model_display_label(&self) -> String {
        self.assembly.model_label_display()
    }

    /// 当前活动模型的上下文窗口大小（tokens）；`context_window_for_model` 返回 `(tokens, is_fallback)`，
    /// 仅当**非** fallback（即可靠已知）时返回 `Some`，否则 `None` 让 footer 优雅降级（FIX 3）。
    fn context_window(&self) -> Option<usize> {
        let (tokens, is_fallback) = crate::chat::model_metadata::context_window_for_model(
            Some(&self.assembly.provider),
            &self.assembly.model,
        );
        if is_fallback {
            None
        } else {
            Some(tokens)
        }
    }

    /// 切换活动模型（`/model` / Ctrl+L 选定后）。`value` 形如 `provider:model`。重新 resolve
    /// 一个 [`TurnAssembly`]（沿用同 cwd / approve 策略），写一条 ModelChange 到 session，并把新的
    /// `system_prompt` 同步到 runtime_messages 的首条 system（如有）。成功返回 Ok(label)。
    fn switch_model(&mut self, value: &str) -> Result<String, String> {
        let (provider_override, model_override) = split_model_label(value);
        let settings = self.state.settings_read().clone();
        let approve_sensitive = self.assembly.effective_chat_tools.approval_policy == "auto";
        let new_assembly = TurnAssembly::resolve(
            &settings,
            provider_override.as_deref(),
            model_override.as_deref(),
            &self.cwd,
            approve_sensitive,
        )?;
        let label = new_assembly.model_label();
        // 更新首条 system（系统提示不随模型变，但稳妥起见同步）。
        if let Some(first) = self.runtime_messages.first_mut() {
            if first["role"] == "system" {
                first["content"] = json!(new_assembly.system_prompt.clone());
            }
        }
        self.timeout_ms = new_assembly.effective_chat_tools.tool_timeout_ms;
        self.assembly = Arc::new(new_assembly);
        self.append_session(SessionRecord::ModelChange {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            model: label.clone(),
        });
        Ok(label)
    }

    /// settings 里所有 enabled 模型作为选择器条目：`(provider:model, label, description)`。
    fn enabled_model_items(&self) -> Vec<(String, String, Option<String>)> {
        let settings = self.state.settings_read();
        let mut items = Vec::new();
        for provider in &settings.providers {
            if !provider.enabled {
                continue;
            }
            for model in &provider.enabled_models {
                let value = format!("{}:{}", provider.id, model);
                items.push((value, model.clone(), Some(provider.name.clone())));
            }
        }
        items
    }

    /// 该 cwd 下最近的会话作为选择器条目：`(jsonl_path, label, preview)`。
    fn session_items(&self) -> Vec<(String, String, Option<String>)> {
        session_items_for_cwd(&self.cwd)
    }

    /// `/mcp`：探测已配置 MCP 服务器，格式化为一段**每服务器一行**的紧凑摘要（推进 transcript Notice）。
    ///
    /// 在交互 UI 线程上 `block_on` 探测（与启动期 `collect_mcp_tools` 同源、同 ~20s 每服务器
    /// 上限，故不会无限阻塞）。无服务器配置 / chat tools 关闭时给出对应说明。`width` 为终端列宽，
    /// 每行据此截到一行（ANSI-aware）。
    fn mcp_status_summary(&self, width: usize) -> String {
        let settings = self.state.settings_read().clone();
        if !settings.chat_tools.enabled {
            return "MCP is off (enable chat tools in the Kivio app).".to_string();
        }
        let statuses = self
            .handle
            .block_on(crate::kivio_code::mcp_setup::collect_mcp_status(
                &self.state,
                &settings,
            ));
        format_mcp_summary(&statuses, width)
    }

    /// `/skill`：从活动 assembly 的 skill_registry 渲染一段**每技能一行**的紧凑技能列表（推进 transcript
    /// Notice）。无技能时提示用户去 `<app_data>/skills/…` 放 SKILL.md。`width` 为终端列宽，描述据此截到一行。
    fn skill_list_summary(&self, width: usize) -> String {
        let summaries =
            crate::kivio_code::skill_setup::skill_summaries(&self.assembly.skill_registry);
        format_skill_summary(&summaries, width)
    }

    /// `/sessions` 选定后：加载该会话，替换 session + 重建 runtime_messages，并刷新 UI transcript。
    /// best-effort：加载失败仅通知。返回是否成功。
    fn resume_session_path(&mut self, path: &str, app: &mut App) -> bool {
        let path = PathBuf::from(path);
        let session = match Session::load(&path) {
            Ok(s) => s,
            Err(err) => {
                app.push_notice(format!("Failed to load session: {err}"));
                return false;
            }
        };
        app.rebuild_from_session(&session);
        let mut messages = session.to_runtime_messages();
        if !messages.iter().any(|m| m["role"] == "system") {
            messages.insert(
                0,
                json!({ "role": "system", "content": self.assembly.system_prompt.clone() }),
            );
        }
        self.runtime_messages = messages;
        self.persisted_tool_calls = std::collections::HashSet::new();
        self.session = Some(session);
        // Reflect the resumed conversation's size in the footer ctx gauge immediately,
        // using the SAME estimator the agent loop's compaction uses (so the ctx % lines
        // up with the 0.85 compaction trigger).
        app.set_context_tokens(Some(
            crate::chat::agent::compaction::estimate_messages_tokens(&self.runtime_messages) as u64,
        ));
        true
    }

    /// `/new` · `/clear`：开一段全新对话。把上下文重置到系统提示基线（丢弃历史 user/assistant/tool
    /// 消息），新建一个 JSONL session 文件，清掉已落盘工具调用记录，并把 footer ctx gauge 拉回到
    /// 系统提示的小基线（用与压缩同源的估算器）。任何在跑的轮先取消，并把 app 切回 Idle。
    fn reset_conversation(&mut self, app: &mut App) {
        // 取消任何在跑的轮，丢弃其 in-flight 状态。
        self.request_cancel();
        self.current = None;
        self.current_message_id = None;

        // 重置上下文到系统提示基线。
        self.runtime_messages = vec![json!({
            "role": "system",
            "content": self.assembly.system_prompt.clone()
        })];
        // 起一个全新的 session 文件（失败像启动路径一样静默忽略）。
        self.session = Session::create(&self.cwd, &self.assembly.model_label()).ok();
        self.persisted_tool_calls.clear();

        // ctx gauge 回到系统提示的小基线（与 0.85 压缩触发点同源的估算器）。
        app.set_context_tokens(Some(
            crate::chat::agent::compaction::estimate_messages_tokens(&self.runtime_messages) as u64,
        ));
        app.set_mode(AppMode::Idle);
    }

    /// `/compact [focus]`：强制压缩当前对话历史（无视预算），走与 agent loop 自动路径相同的
    /// serialize→summary→replace 核心（[`crate::chat::agent::compaction::force_compact`]）。
    /// 在交互 UI 线程上 `block_on` 打一次摘要模型调用；成功则用压缩后的历史**替换**
    /// `runtime_messages` 并把 footer ctx gauge 刷到压缩后的小值，推一条 transcript 通知。
    /// 无可摘要旧段 / 空摘要 / 失败时不改 runtime_messages，仅推一条说明。
    fn compact_now(&mut self, focus: Option<&str>, app: &mut App) {
        let before = crate::chat::agent::compaction::estimate_messages_tokens(&self.runtime_messages);
        let compacted = self.handle.block_on(crate::chat::agent::compaction::force_compact(
            &self.state,
            &self.assembly.provider,
            &self.assembly.model,
            &self.runtime_messages,
            self.assembly.max_output_tokens,
            self.assembly.retry_attempts,
            "kivio-code",
            "kivio-code-compact",
            focus,
        ));
        match compacted {
            Some(compacted) => {
                self.runtime_messages = compacted;
                let after = crate::chat::agent::compaction::estimate_messages_tokens(
                    &self.runtime_messages,
                );
                app.set_context_tokens(Some(after as u64));
                app.push_notice(format!(
                    "Compacted conversation: ~{before} → ~{after} tokens."
                ));
            }
            None => {
                app.push_notice(
                    "Nothing to compact yet (conversation fits in the recent window), or summarization failed.",
                );
            }
        }
    }

    /// 处理一轮结束：忽略过期 generation；否则把 assistant 消息 + 工具调用持久化、累积进
    /// runtime_messages，刷新 footer usage，回到 Idle。返回 footer usage 摘要（None = 不变）。
    fn finish_turn(&mut self, done: TurnDone, app: &mut App) {
        // 过期任务（已被取消并被新一轮取代）直接丢弃。
        let live = self
            .current
            .as_ref()
            .map(|c| c.generation() == done.generation)
            .unwrap_or(false);
        if !live {
            return;
        }
        self.current = None;
        self.current_message_id = None;

        match done.result {
            Ok(result) => {
                // finalize 助手消息（loop 已发过 Done；这里兜底，幂等）。
                app.apply_agent_event(AgentUiEvent::Done {
                    message_id: done.message_id.clone(),
                    reason: result.stream_outcome.clone(),
                });
                self.persist_turn_records(&result);
                self.accumulate_runtime_messages(&result);
                // Context occupancy must reflect the CURRENT conversation size (the
                // prompt that will be sent next), NOT `result.usage.input_tokens` —
                // that value is summed across every model call in the turn (planning +
                // each tool round + synthesis; see RunState::merge_usage), so a
                // multi-round turn inflates it and it jumps around non-monotonically.
                // Estimate from the accumulated transcript using the SAME estimator the
                // agent loop's compaction uses (compaction::estimate_messages_tokens), so
                // the displayed % lines up with the 0.85 compaction trigger.
                app.set_context_tokens(Some(
                    crate::chat::agent::compaction::estimate_messages_tokens(
                        &self.runtime_messages,
                    ) as u64,
                ));
            }
            Err(err) => {
                if err == "cancelled" {
                    app.apply_agent_event(AgentUiEvent::Done {
                        message_id: done.message_id.clone(),
                        reason: "cancelled".to_string(),
                    });
                    app.push_notice("Run cancelled.");
                } else {
                    app.apply_agent_event(AgentUiEvent::Done {
                        message_id: done.message_id.clone(),
                        reason: "error".to_string(),
                    });
                    // Surface a concise, actionable notice instead of the raw provider
                    // JSON / retry-count noise (e.g. a 402 balance error → one Chinese line).
                    app.push_notice(errors::friendly_error(&err));
                }
            }
        }
        app.set_mode(AppMode::Idle);
    }

    /// 把这一轮的 assistant 消息 + 工具调用/结果落盘（best-effort）。
    fn persist_turn_records(&mut self, result: &AgentRunResult) {
        for record in &result.tool_records {
            self.persist_tool_record(record);
        }
        if !result.content.trim().is_empty() {
            self.append_session(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: chrono::Utc::now().to_rfc3339(),
                role: "assistant".to_string(),
                content: result.content.clone(),
            });
        }
    }

    /// 把这一轮产生的 provider-agnostic transcript（含 assistant tool_calls / tool 结果）累积进
    /// runtime_messages，使下一轮带上完整上下文。`api_messages` 是 OpenAI 兼容的隐藏消息序列。
    ///
    /// 本轮若发生过 L2 压缩（`compacted_history` 有值），则用压缩后的完整历史**替换**累积副本
    /// （而非追加），让压缩真正跨轮生效——否则交互模式会一直带着压缩前的全量历史单调膨胀。
    fn accumulate_runtime_messages(&mut self, result: &AgentRunResult) {
        if let Some(compacted) = &result.compacted_history {
            self.runtime_messages = compacted.clone();
        } else if !result.api_messages.is_empty() {
            self.runtime_messages
                .extend(result.api_messages.iter().cloned());
        } else if !result.content.trim().is_empty() {
            self.runtime_messages
                .push(json!({ "role": "assistant", "content": result.content }));
        }
    }

    /// 持久化一条工具调用 + 结果（一个 call_id 只落一次，取其终态）。
    fn persist_tool_record(&mut self, record: &ToolCallRecord) {
        // 仅在终态落盘，且每个 call_id 只落一次。
        if matches!(record.status, ToolCallStatus::Pending | ToolCallStatus::Running) {
            return;
        }
        if !self.persisted_tool_calls.insert(record.id.clone()) {
            return;
        }
        let arguments = serde_json::from_str::<Value>(&record.arguments)
            .unwrap_or_else(|_| Value::String(record.arguments.clone()));
        self.append_session(SessionRecord::ToolCall {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            call_id: record.id.clone(),
            name: record.name.clone(),
            arguments,
        });
        let is_error = matches!(record.status, ToolCallStatus::Error);
        let content = record
            .error
            .clone()
            .or_else(|| record.result_preview.clone())
            .unwrap_or_default();
        self.append_session(SessionRecord::ToolResult {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            call_id: record.id.clone(),
            name: record.name.clone(),
            content,
            is_error,
        });
    }

    /// best-effort 追加一条 session record（无 session 或写失败时静默忽略——持久化不应中断交互）。
    fn append_session(&mut self, record: SessionRecord) {
        if let Some(session) = self.session.as_mut() {
            let _ = session.append(record);
        }
    }
}

/// 启动交互模式，阻塞直到用户退出。**需要一个真实 TTY**（调用方在 bin 里已判断 stdin/stdout 是 TTY）。
///
/// 整个生命周期内 raw 模式经 [`RawModeGuard`] 管理，正常返回或 panic 均会还原终端。自建多线程
/// tokio runtime（后台跑 agent 任务）；settings/cwd/provider 从磁盘 + 进程环境解析（与 print 模式
/// 同源），因此无需调用方再传。
pub fn run(options: InteractiveOptions) -> std::io::Result<()> {
    let _guard = RawModeGuard::enter()?;

    let terminal = CrosstermTerminal::new();
    let (cols, rows) = (terminal.columns(), terminal.rows());

    let cwd_display = options.cwd_display.clone();
    let mut app = App::new(cwd_display, options.model.clone());
    app.set_terminal_rows(rows);
    app.set_terminal_cols(cols);
    app.set_show_reasoning(options.verbose);

    // 输入线程：raw stdin → StdinBuffer → InputEvent channel。
    let (tx, rx) = mpsc::channel::<InputEvent>();
    spawn_input_thread(tx);

    // agent-event 通道（host → 主循环）+ turn-done 通道（agent 任务 → 主循环）。
    let (agent_tx, agent_rx) = mpsc::channel::<AgentUiEvent>();
    let (done_tx, done_rx) = mpsc::channel::<TurnDone>();

    // 自建多线程 runtime 跑后台 agent。
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("failed to start async runtime: {err}"),
            ));
        }
    };

    // 解析 settings → assembly（provider/model + 各项 knob）。失败也启动 shell，只是提交会报错。
    // cwd 直接用 options.cwd（bin 已据 `-C` 解析；不再重新取 current_dir）。
    let cwd = options.cwd.clone();
    let settings = load_settings_from_disk();
    let assembly = TurnAssembly::resolve(
        &settings,
        options.provider_override.as_deref(),
        options.model_override.as_deref(),
        &cwd,
        /* approve_sensitive */ !options.no_approve,
    );

    let mut turn = match assembly {
        Ok(mut assembly) => {
            // session：续跑请求优先（加载已存在的）；否则新建。
            let (session, mut runtime_messages, resumed) =
                resolve_session(&options.resume, &cwd, &assembly, &mut app);
            if runtime_messages.is_empty() {
                // 新会话：system prompt 作为 runtime_messages 的第一条。
                runtime_messages =
                    vec![json!({ "role": "system", "content": assembly.system_prompt.clone() })];
            }
            let state = build_app_state(settings.clone());
            // MCP tools are collected asynchronously (server connection is
            // async), then merged into the per-turn tool set by `into_config`.
            // Block on the runtime since `run()` itself is sync. Stub returns
            // empty, so this is a no-op until MCP is wired up.
            let mcp_tools = runtime
                .handle()
                .block_on(crate::kivio_code::mcp_setup::collect_mcp_tools(
                    &state, &settings,
                ));
            assembly.set_mcp_tools(mcp_tools);
            let timeout_ms = assembly.effective_chat_tools.tool_timeout_ms;
            // The branded welcome header (rendered by App::render) replaces the old
            // bare one-line startup notice. On resume the restored transcript is the
            // focus, so suppress the header there.
            if resumed {
                app.set_show_welcome(false);
            }
            Some(TurnRuntime {
                handle: runtime.handle().clone(),
                state,
                assembly: Arc::new(assembly),
                cwd,
                timeout_ms,
                generations: Generations::default(),
                current: None,
                current_message_id: None,
                runtime_messages,
                session,
                persisted_tool_calls: std::collections::HashSet::new(),
                turn_done_tx: done_tx,
            })
        }
        Err(err) => {
            app.push_notice(format!("No usable model: {err}"));
            app.push_notice("Configure a chat model in the Kivio app, then restart. Ctrl+D exits.");
            None
        }
    };

    let mut tui = Tui::new(terminal);
    tui.set_show_hardware_cursor(true);

    // 把 footer / welcome 的展示串切到人读的 provider name + 上下文窗口（FIX 2 + 3）。
    // `App::new` 初始化时用的是 bin 传入的 id 形式串（也用作选择器定位的解析值），这里在拿到
    // 解析后的 assembly 后，把展示串覆盖为 `<Provider Name> · model`，并把已知上下文窗口填进 footer。
    if let Some(turn) = turn.as_ref() {
        app.set_model(turn.model_label());
        app.set_model_display(turn.model_display_label());
        app.set_context_window(turn.context_window());
        // Seed the footer ctx gauge from the finalized runtime_messages (the fresh
        // [system] for a new session, or the rebuilt messages on resume) so idle shows
        // the correct small baseline (system-prompt size) rather than nothing/stale.
        // (resume_session_path also refreshes this; this covers the fresh-session path.)
        app.set_context_tokens(Some(
            crate::chat::agent::compaction::estimate_messages_tokens(&turn.runtime_messages) as u64,
        ));
    }

    // 首帧。
    render_frame(&mut tui, &mut app, cols);

    let exit = run_loop(
        &mut tui,
        &mut app,
        &rx,
        &agent_rx,
        &done_rx,
        &agent_tx,
        turn.as_mut(),
    );

    // 收尾：取消任何在跑的轮，停渲染并换行让 prompt 干净。
    if let Some(turn) = turn.as_ref() {
        turn.request_cancel();
    }
    tui.stop();
    tui.terminal.write("\r\n");
    // runtime drop 会等后台任务收尾（已发取消信号，loop 很快返回）。
    drop(runtime);

    exit
}

/// 从 footer 的 `provider:model` 串拆回 provider / model override（供 `TurnAssembly::resolve`）。
/// 缺省 / `<no model>` 时返回 `(None, None)`，让 resolve 走 settings 默认。
fn split_model_label(label: &str) -> (Option<String>, Option<String>) {
    if label.is_empty() || label.starts_with('<') {
        return (None, None);
    }
    match label.split_once(':') {
        Some((provider, model)) => (Some(provider.to_string()), Some(model.to_string())),
        None => (None, Some(label.to_string())),
    }
}

/// 解析续跑请求：续跑时加载已有会话并重建 transcript + runtime_messages；否则新建一个会话。
///
/// 返回 `(session, runtime_messages, resumed)`：
/// - `session`：要追加写入的会话（None = 持久化不可用，仍继续运行）。
/// - `runtime_messages`：续跑时由会话记录重建的上下文（含 system）；新会话为空 vec（调用方补 system）。
/// - `resumed`：是否真的从磁盘续跑了一个会话（用于决定欢迎语 / footer）。
fn resolve_session(
    resume: &Option<ResumeRequest>,
    cwd: &PathBuf,
    assembly: &TurnAssembly,
    app: &mut App,
) -> (Option<Session>, Vec<Value>, bool) {
    if let Some(request) = resume {
        if let Some(session) = load_session_for_resume(request, cwd) {
            let mut messages = session.to_runtime_messages();
            // 真的有可恢复对话才算 resume：to_runtime_messages 不含 system，所以非空即代表
            // 至少有一条 user/assistant/tool 记录。空壳（header-only）落到这里 → 不谎报。
            if !messages.is_empty() {
                // 重建 UI transcript + 上下文消息。
                app.rebuild_from_session(&session);
                // to_runtime_messages 不含 system（session 不存 system）；补一条当前 system。
                if !messages.iter().any(|m| m["role"] == "system") {
                    messages.insert(
                        0,
                        json!({ "role": "system", "content": assembly.system_prompt.clone() }),
                    );
                }
                return (Some(session), messages, true);
            }
        }
        app.push_notice("No matching session to resume; starting a new one.");
    }
    // 新会话。
    let session = Session::create(cwd, &assembly.model_label()).ok();
    if session.is_none() {
        app.push_notice("(session persistence unavailable; continuing without it)");
    }
    (session, Vec::new(), false)
}

/// 按续跑请求找到一个会话并加载：`Recent` → cwd 下最近一条；`Reference(s)` → 若 `s` 是存在的
/// `.jsonl` 路径则直接 load，否则按（部分前缀）id 在该 cwd 的会话里匹配。
fn load_session_for_resume(request: &ResumeRequest, cwd: &PathBuf) -> Option<Session> {
    match request {
        ResumeRequest::Recent => crate::kivio_code::session::resume_recent(cwd),
        ResumeRequest::Reference(reference) => {
            let path = PathBuf::from(reference);
            if path.is_file() {
                return Session::load(&path).ok();
            }
            // id（或前缀）匹配：在该 cwd 的会话里找。
            let summary = crate::kivio_code::session::list_sessions(cwd)
                .into_iter()
                .find(|s| s.id == *reference || s.id.starts_with(reference.as_str()))?;
            Session::load(&summary.path).ok()
        }
    }
}

/// 该 cwd 下的会话作为选择器条目（最近优先）：`(jsonl_path, label, preview)`。label 用
/// 创建时间，description 用首条用户消息预览。空会话（无首条用户消息）被隐藏，避免选到
/// header-only 空壳。纯函数，便于单测。
fn session_items_for_cwd(cwd: &PathBuf) -> Vec<(String, String, Option<String>)> {
    crate::kivio_code::session::list_sessions(cwd)
        .into_iter()
        .filter(|s| s.first_user_message.is_some())
        .map(|s| {
            let label = s.created_at.clone();
            let desc = s.first_user_message.clone();
            (s.path.to_string_lossy().into_owned(), label, desc)
        })
        .collect()
}

// ---- compact one-line-per-entry summaries for `/skill` and `/mcp` ----

const DIM: &str = "\x1b[2m";
const DIM_OFF: &str = "\x1b[22m";
const BOLD: &str = "\x1b[1m";
const BOLD_OFF: &str = "\x1b[22m";

/// Hard char cap for a description before width-truncation: keeps a single very wide
/// terminal from printing a whole paragraph on one line.
const DESC_CHAR_CAP: usize = 90;

/// First-line + indent budget the Notice renderer eats: `· ` prefix (2) + padding (1 each
/// side). Subtract it from the terminal width so a formatted line never wraps.
const NOTICE_OVERHEAD: usize = 4;

/// First sentence of `desc` (up to the first `. ` / `。`), capped at `DESC_CHAR_CAP` chars.
/// Collapses internal newlines/runs of whitespace to single spaces so the result is one line.
fn first_sentence(desc: &str) -> String {
    let flat: String = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    // Find the first sentence terminator.
    let mut end = flat.len();
    if let Some(idx) = flat.find(". ") {
        end = end.min(idx + 1); // include the period
    }
    if let Some(idx) = flat.find('。') {
        end = end.min(idx + '。'.len_utf8());
    }
    let sentence = &flat[..end];
    // Hard char cap (char-aware, not byte-aware).
    let capped: String = sentence.chars().take(DESC_CHAR_CAP).collect();
    capped
}

/// Compact `/skill` list: a `Skills · N` header (dim count) then ONE line per skill —
/// bold name + dim first-sentence description, width-truncated (ANSI-aware) to one line.
/// Disabled skills are marked `(off)`. Empty → a single friendly line.
fn format_skill_summary(summaries: &[(String, String, bool)], width: usize) -> String {
    if summaries.is_empty() {
        return "No skills found (drop a SKILL.md under <app_data>/skills/…).".to_string();
    }
    let usable = width.saturating_sub(NOTICE_OVERHEAD).max(20);
    let mut out = format!("Skills {DIM}· {}{DIM_OFF}", summaries.len());
    for (name, description, enabled) in summaries {
        let off = if *enabled {
            String::new()
        } else {
            format!(" {DIM}(off){DIM_OFF}")
        };
        let desc = first_sentence(description);
        // `name  desc` styled, then width-truncate the WHOLE line (ANSI-aware) so it fits.
        let line = format!("  {BOLD}{name}{BOLD_OFF}{off}  {DIM}{desc}{DIM_OFF}");
        out.push('\n');
        out.push_str(&truncate_to_width(&line, usable, "…", false));
    }
    out
}

/// Compact `/mcp` list: a `MCP servers · N` header (dim count) then ONE line per server —
/// bold name, dim `[transport]`, status, and `N tools`, width-truncated to one line.
fn format_mcp_summary(
    statuses: &[crate::kivio_code::mcp_setup::McpServerStatus],
    width: usize,
) -> String {
    if statuses.is_empty() {
        return "No MCP servers configured.".to_string();
    }
    let usable = width.saturating_sub(NOTICE_OVERHEAD).max(20);
    let mut out = format!("MCP servers {DIM}· {}{DIM_OFF}", statuses.len());
    for s in statuses {
        let status = if !s.enabled {
            "off".to_string()
        } else if s.connected {
            format!("connected · {} tools", s.tools.len())
        } else {
            format!("error: {}", s.error.as_deref().unwrap_or("connection failed"))
        };
        let line = format!(
            "  {BOLD}{}{BOLD_OFF}  {DIM}[{}]{DIM_OFF}  {DIM}{}{DIM_OFF}",
            s.name, s.transport, status
        );
        out.push('\n');
        out.push_str(&truncate_to_width(&line, usable, "…", false));
    }
    out
}

/// 发送一条 `mixer_vision` 工具记录到 UI（图片预分析步骤可见，仿 GUI mixer 卡片）。
/// 同一 `id` 在 Running→Success/Skipped 之间多次发送，App 按 id upsert。
fn emit_vision_card(
    tx: &Sender<AgentUiEvent>,
    id: &str,
    status: ToolCallStatus,
    image_count: usize,
    model_label: Option<String>,
    error: Option<String>,
) {
    let summary = match &model_label {
        Some(label) => format!("{image_count} image(s) · {label}"),
        None => format!("analyzing {image_count} image(s)…"),
    };
    let record = ToolCallRecord {
        id: id.to_string(),
        name: "mixer_vision".to_string(),
        source: "native".to_string(),
        server_id: None,
        arguments: json!({ "images": image_count }).to_string(),
        status,
        result_preview: Some(summary),
        error,
        duration_ms: None,
        started_at: None,
        completed_at: None,
        round: 0,
        sensitive: false,
        artifacts: Vec::new(),
        trace_id: None,
        span_id: None,
        structured_content: None,
    };
    let _ = tx.send(AgentUiEvent::ToolRecord(Box::new(record)));
}

/// `apply_effect` 的控制流结果。
enum EffectFlow {
    /// 退出事件循环。
    Quit,
    /// 继续（已就地处理；调用方应重绘）。
    Continue,
}

/// 把一个 [`AppEffect`] 应用到运行时：提交起轮、取消、打开 / 应用选择器、切模型、续会话。
/// 与 `run_loop` 分离以便集中处理新增的 effect 分支（且便于测试覆盖路由约定）。
fn apply_effect(
    effect: AppEffect,
    app: &mut App,
    agent_tx: &Sender<AgentUiEvent>,
    turn: Option<&mut TurnRuntime>,
) -> EffectFlow {
    match effect {
        AppEffect::Quit => return EffectFlow::Quit,
        AppEffect::None => {}
        AppEffect::NewConversation => {
            // Clear the on-screen transcript AND reset the runtime so the next turn
            // does NOT carry the old conversation, and the ctx gauge drops back to the
            // system-prompt baseline (the reported bug: /new only cleared the screen).
            app.clear_transcript();
            app.set_show_welcome(true);
            if let Some(turn) = turn {
                turn.reset_conversation(app);
            }
        }
        AppEffect::Submitted { text, images } => {
            if let Some(turn) = turn {
                if !turn.is_generating() {
                    // 图片附件但既无法直接交给主模型、又没配显式视觉模型：推一条 Notice 并丢弃
                    // 本轮图片（纯文本继续）。同步检查（settings 读），保证 Notice 出现在 transcript
                    // 而非异步事件流里。主模型支持视觉 → 直发；否则有显式视觉模型 → mixer 兜底。
                    let images = if !images.is_empty()
                        && !turn.main_model_supports_vision()
                        && !turn.has_vision_model()
                    {
                        app.push_notice(
                            "No vision support — pick a vision-capable main model, or configure a Mixer/vision model in the Kivio app; images skipped this turn.",
                        );
                        Vec::new()
                    } else {
                        images
                    };
                    let plan_mode = app.agent_mode() == AgentMode::Plan;
                    app.set_mode(AppMode::Generating);
                    turn.begin_turn(text, images, agent_tx, plan_mode);
                }
            } else {
                app.push_notice("No model configured; cannot run.");
            }
        }
        AppEffect::Cancel => {
            if let Some(turn) = turn {
                turn.request_cancel();
            }
        }
        AppEffect::OpenModelSelector => {
            if let Some(turn) = turn {
                let items = turn.enabled_model_items();
                app.open_model_selector(items);
            } else {
                app.push_notice("No model configured.");
            }
        }
        AppEffect::OpenSessionSelector => {
            if let Some(turn) = turn {
                let items = turn.session_items();
                app.open_session_selector(items);
            } else {
                app.push_notice("No sessions available.");
            }
        }
        AppEffect::ModelSelected(value) => {
            if let Some(turn) = turn {
                match turn.switch_model(&value) {
                    Ok(label) => {
                        // `label` is the id-based resolution value (selector positioning);
                        // the footer/notice show the human-readable provider name (FIX 2).
                        let display = turn.model_display_label();
                        app.set_model(label);
                        app.set_model_display(display.clone());
                        app.set_context_window(turn.context_window());
                        app.push_notice(format!("Switched model to {display}."));
                    }
                    Err(err) => app.push_notice(format!("Could not switch model: {err}")),
                }
            }
        }
        AppEffect::SessionSelected(path) => {
            if let Some(turn) = turn {
                if turn.resume_session_path(&path, app) {
                    app.set_model(turn.model_label());
                    app.set_model_display(turn.model_display_label());
                    app.set_context_window(turn.context_window());
                }
            }
        }
        AppEffect::ShowMcp => {
            if let Some(turn) = turn {
                let summary = turn.mcp_status_summary(app.terminal_cols() as usize);
                app.push_notice(summary);
            } else {
                app.push_notice("No model configured; MCP unavailable.");
            }
        }
        AppEffect::ShowSkills => {
            if let Some(turn) = turn {
                let summary = turn.skill_list_summary(app.terminal_cols() as usize);
                app.push_notice(summary);
            } else {
                app.push_notice("No model configured; skills unavailable.");
            }
        }
        AppEffect::OpenSettings => {
            // The settings overlay toggles kivio-code's own config (e.g.
            // read_claude_dir). It does not need the TurnRuntime — the App seeds
            // from the persisted config and re-saves on toggle; the next turn's
            // build_system_prompt reads the saved value.
            app.open_settings_selector();
        }
        AppEffect::Compact { focus } => {
            if let Some(turn) = turn {
                // Force a compaction of the accumulated history regardless of budget.
                // Blocks the UI thread on one summarization model call (the App already
                // gated against running mid-turn), then replaces runtime_messages with
                // the compacted history and refreshes the footer ctx gauge.
                turn.compact_now(focus.as_deref(), app);
            } else {
                app.push_notice("No model configured; cannot compact.");
            }
        }
    }
    EffectFlow::Continue
}

/// 主事件循环。返回 Ok 表示正常退出。
fn run_loop(
    tui: &mut Tui<CrosstermTerminal>,
    app: &mut App,
    rx: &Receiver<InputEvent>,
    agent_rx: &Receiver<AgentUiEvent>,
    done_rx: &Receiver<TurnDone>,
    agent_tx: &Sender<AgentUiEvent>,
    mut turn: Option<&mut TurnRuntime>,
) -> std::io::Result<()> {
    loop {
        let mut dirty = false;
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(InputEvent::Key(data)) => {
                let effect = app.handle_key(&data);
                match apply_effect(effect, app, agent_tx, turn.as_deref_mut()) {
                    EffectFlow::Quit => return Ok(()),
                    EffectFlow::Continue => dirty = true,
                }
            }
            Ok(InputEvent::Paste(content)) => {
                let wrapped = format!("\x1b[200~{content}\x1b[201~");
                let effect = app.handle_key(&wrapped);
                match apply_effect(effect, app, agent_tx, turn.as_deref_mut()) {
                    EffectFlow::Quit => return Ok(()),
                    EffectFlow::Continue => dirty = true,
                }
            }
            Ok(InputEvent::Resize(cols, rows)) => {
                tui.terminal.set_size(cols, rows);
                app.set_terminal_rows(rows);
                app.set_terminal_cols(cols);
                tui.invalidate();
                dirty = true;
            }
            Ok(InputEvent::Eof) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if tui.terminal.refresh_size() {
                    app.set_terminal_rows(tui.terminal.rows());
                    app.set_terminal_cols(tui.terminal.columns());
                    tui.invalidate();
                    dirty = true;
                }
                // 推进 thinking spinner（generating 态）。
                if app.tick_loader() {
                    dirty = true;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }

        // Drain streaming / tool-record / done events (non-blocking).
        while let Ok(event) = agent_rx.try_recv() {
            app.apply_agent_event(event);
            dirty = true;
        }
        // Drain finished turns.
        while let Ok(done) = done_rx.try_recv() {
            if let Some(turn) = turn.as_deref_mut() {
                turn.finish_turn(done, app);
            }
            dirty = true;
        }

        if dirty {
            let width = tui.terminal.columns();
            render_frame(tui, app, width);
        }
    }
}

/// 渲染一帧：把 App 渲染出的行交给差分渲染器。
fn render_frame(tui: &mut Tui<CrosstermTerminal>, app: &mut App, width: u16) {
    let lines = app.render(width);
    tui.clear_children();
    tui.add_child(Box::new(AppFrame { lines }));
    tui.render();
}

/// 转义键消歧超时（ms）：一个孤立的 `ESC`（`\x1b`）是 CSI/SS3 等长序列的合法前缀，
/// 故 [`StdinBuffer`] 把它判为 `Incomplete` 暂存。若不在「无后续字节」时把它 flush 出来，
/// 单独按 Esc（关 overlay / 取消生成）就会被吞掉，直到下一次按键才连带产出（且会和那次
/// 按键被错误地黏成一个 `ESC+char` meta 序列）。PI 用 ~10ms 的定时器消歧；这里在 unix 上用
/// `poll` 的超时实现同一行为。
#[cfg(unix)]
const ESC_DISAMBIGUATION_MS: i32 = 25;

/// 起一条输入线程（unix）：用 `poll(2)` 带超时读 stdin。有数据就读并处理；超时且缓冲里残留
/// 不完整序列（典型是孤立的 `ESC`）时调 [`StdinBuffer::flush`] 把它作为一个完整按键产出 ——
/// 这让单独按 Esc 立即生效。stdin EOF 或 channel 关闭时退出；用户正常退出（Ctrl+D / `/quit`）后
/// 主循环返回、进程结束，本线程随之被回收（调用方不 join）。
#[cfg(unix)]
fn spawn_input_thread(tx: Sender<InputEvent>) {
    use std::os::fd::AsRawFd;

    std::thread::spawn(move || {
        let mut buffer = StdinBuffer::new();
        let stdin = std::io::stdin();
        let fd = stdin.as_raw_fd();
        let mut handle = stdin.lock();
        let mut bytes = [0u8; 1024];

        loop {
            // 若缓冲里有残留（不完整序列），只等 ESC_DISAMBIGUATION_MS；否则无限等待。
            let timeout_ms: i32 = if buffer.pending().is_empty() { -1 } else { ESC_DISAMBIGUATION_MS };
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: 单个有效 pollfd，count=1。
            let ready = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };

            if ready < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                let _ = tx.send(InputEvent::Eof);
                return;
            }

            if ready == 0 {
                // poll 超时：把残留的不完整序列（如孤立 ESC）作为完整按键 flush 出去。
                for seq in buffer.flush() {
                    if tx.send(InputEvent::Key(seq)).is_err() {
                        return;
                    }
                }
                continue;
            }

            // poll 报错位（挂断 / 错误）也按 EOF 处理（先尝试读完剩余数据）。
            let n = match handle.read(&mut bytes) {
                Ok(0) => {
                    let _ = tx.send(InputEvent::Eof);
                    return;
                }
                Ok(n) => n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    let _ = tx.send(InputEvent::Eof);
                    return;
                }
            };

            let chunk = String::from_utf8_lossy(&bytes[..n]);
            let events = buffer.process(&chunk);
            for seq in events.sequences {
                if tx.send(InputEvent::Key(seq)).is_err() {
                    return;
                }
            }
            for paste in events.pastes {
                if tx.send(InputEvent::Paste(paste)).is_err() {
                    return;
                }
            }
        }
    });
}

/// 起一条输入线程（非 unix 回退，如 Windows）：阻塞读 stdin（无 `poll` 超时消歧）。功能等价于
/// unix 路径，但孤立 ESC 会等到下一次按键才一并产出（Windows 上 Esc 通常以单独的 key event
/// 形态到达，影响较小）。
#[cfg(not(unix))]
fn spawn_input_thread(tx: Sender<InputEvent>) {
    std::thread::spawn(move || {
        let mut buffer = StdinBuffer::new();
        let mut stdin = std::io::stdin();
        let mut bytes = [0u8; 1024];

        loop {
            let n = match stdin.read(&mut bytes) {
                Ok(0) => {
                    let _ = tx.send(InputEvent::Eof);
                    return;
                }
                Ok(n) => n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    let _ = tx.send(InputEvent::Eof);
                    return;
                }
            };

            let chunk = String::from_utf8_lossy(&bytes[..n]);
            let events = buffer.process(&chunk);
            for seq in events.sequences {
                if tx.send(InputEvent::Key(seq)).is_err() {
                    return;
                }
            }
            for paste in events.pastes {
                if tx.send(InputEvent::Paste(paste)).is_err() {
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::ToolCallStatus;
    use crate::settings::{ModelProvider, Settings};

    /// 用 fake channel 驱动 run_loop 的逻辑等价物：这里复用 App 直接断言事件→效果，
    /// 因为 run_loop 与真实 Tui<CrosstermTerminal> 绑定（需 TTY），其分发逻辑已在 App 单测覆盖。
    /// 本测试聚焦 InputEvent → App 的分发约定。
    #[test]
    fn key_event_drives_app_handle_key() {
        let mut app = App::new("~/p".to_string(), "m".to_string());
        app.set_terminal_rows(24);
        // 普通字符
        assert_eq!(app.handle_key("h"), AppEffect::None);
        assert_eq!(app.handle_key("i"), AppEffect::None);
        assert_eq!(app.editor_text(), "hi");
        // enter 提交 → Submitted。
        assert_eq!(
            app.handle_key("\r"),
            AppEffect::Submitted {
                text: "hi".to_string(),
                images: Vec::new(),
            }
        );
        assert_eq!(app.last_submitted(), Some("hi"));
    }

    #[test]
    fn paste_event_inserts_into_editor() {
        let mut app = App::new("~/p".to_string(), "m".to_string());
        app.set_terminal_rows(24);
        let wrapped = format!("\x1b[200~{}\x1b[201~", "pasted text");
        app.handle_key(&wrapped);
        assert_eq!(app.editor_text(), "pasted text");
    }

    #[test]
    fn quit_via_ctrl_d() {
        let mut app = App::new("~/p".to_string(), "m".to_string());
        app.set_terminal_rows(24);
        assert_eq!(app.handle_key("\x04"), AppEffect::Quit);
    }

    #[test]
    fn input_event_equality() {
        assert_eq!(InputEvent::Key("a".into()), InputEvent::Key("a".into()));
        assert_ne!(InputEvent::Key("a".into()), InputEvent::Paste("a".into()));
        assert_eq!(InputEvent::Resize(80, 24), InputEvent::Resize(80, 24));
    }

    #[test]
    fn split_model_label_variants() {
        assert_eq!(
            split_model_label("openai:gpt-4o"),
            (Some("openai".to_string()), Some("gpt-4o".to_string()))
        );
        assert_eq!(split_model_label("gpt-4o"), (None, Some("gpt-4o".to_string())));
        assert_eq!(split_model_label("<no model>"), (None, None));
        assert_eq!(split_model_label(""), (None, None));
    }

    // ---- compact /skill + /mcp summaries (one line per entry) ----

    use crate::kivio_code::mcp_setup::McpServerStatus;
    use crate::kivio_code::tui::text_width::visible_width;

    fn skill(name: &str, desc: &str, enabled: bool) -> (String, String, bool) {
        (name.to_string(), desc.to_string(), enabled)
    }

    /// The visible width of every line must fit the usable budget (no wrap), and there
    /// must be exactly one line per skill plus the header.
    #[test]
    fn skill_summary_is_one_line_per_skill() {
        let long = "Discovers and injects project-specific coding guidelines from .trellis/spec/ \
            before implementation begins. Reads spec indexes and resolves the relevant layers, then \
            composes a context block the agent uses for the rest of the turn.";
        let summaries = vec![
            skill("trellis-before-dev", long, true),
            skill("trellis-brainstorm", long, true),
            skill("disabled-one", "A short note.", false),
        ];
        let out = format_skill_summary(&summaries, 80);
        let lines: Vec<&str> = out.lines().collect();
        // header + one line per skill.
        assert_eq!(lines.len(), 1 + summaries.len());
        // header carries the styled count.
        assert!(lines[0].contains("Skills"), "header: {}", lines[0]);
        assert!(lines[0].contains('3'), "header count: {}", lines[0]);
        // every line fits the terminal (usable = 80 - overhead); none wrap.
        let usable = 80usize - NOTICE_OVERHEAD;
        for l in &lines[1..] {
            assert!(
                visible_width(l) <= usable,
                "line too wide ({}): {l}",
                visible_width(l)
            );
        }
    }

    /// A long, multi-sentence description must be reduced to one truncated line ending
    /// with the ellipsis — not the full paragraph.
    #[test]
    fn skill_summary_truncates_long_description() {
        // A long first sentence (no early period) so the char cap + width truncation both bite.
        let long = "This single very long opening sentence rambles on well past any reasonable \
            single line width so it must be cut and never shows later sentences. Second one here.";
        let out = format_skill_summary(&[skill("alpha", long, true)], 60);
        // No second sentence text survives (first-sentence + cap drop it).
        assert!(!out.contains("Second one"), "should not keep later sentences:\n{out}");
        // It was truncated → ellipsis present.
        assert!(out.contains('…'), "long desc should be truncated with ellipsis:\n{out}");
    }

    /// Disabled skills are marked `(off)`.
    #[test]
    fn skill_summary_marks_disabled() {
        let out = format_skill_summary(&[skill("beta", "Nope.", false)], 80);
        assert!(out.contains("(off)"), "disabled marker:\n{out}");
    }

    #[test]
    fn skill_summary_empty_is_one_friendly_line() {
        let out = format_skill_summary(&[], 80);
        assert_eq!(out.lines().count(), 1);
        assert!(out.contains("No skills found"));
    }

    fn mcp_status(name: &str, transport: &str, connected: bool, tools: &[&str]) -> McpServerStatus {
        McpServerStatus {
            id: name.to_string(),
            name: name.to_string(),
            transport: transport.to_string(),
            enabled: true,
            connected,
            tools: tools.iter().map(|t| t.to_string()).collect(),
            error: if connected { None } else { Some("boom".to_string()) },
        }
    }

    #[test]
    fn mcp_summary_is_one_line_per_server() {
        let statuses = vec![
            mcp_status("fs", "stdio", true, &["read", "write", "list"]),
            mcp_status("remote", "streamable_http", false, &[]),
        ];
        let out = format_mcp_summary(&statuses, 80);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 1 + statuses.len());
        assert!(lines[0].contains("MCP servers"));
        let usable = 80usize - NOTICE_OVERHEAD;
        for l in &lines[1..] {
            assert!(visible_width(l) <= usable, "line too wide: {l}");
        }
        // Connected server reports its tool count; failed one shows the error.
        assert!(out.contains("3 tools"));
        assert!(out.contains("error: boom"));
    }

    #[test]
    fn mcp_summary_empty_is_one_friendly_line() {
        let out = format_mcp_summary(&[], 80);
        assert_eq!(out.lines().count(), 1);
        assert!(out.contains("No MCP servers configured"));
    }

    #[test]
    fn first_sentence_stops_at_period_and_caps() {
        assert_eq!(first_sentence("Hello world. More text."), "Hello world.");
        assert_eq!(first_sentence("你好世界。后面还有。"), "你好世界。");
        // Collapses internal whitespace/newlines.
        assert_eq!(first_sentence("a\n  b\tc"), "a b c");
        // Hard char cap.
        let long = "x".repeat(200);
        assert_eq!(first_sentence(&long).chars().count(), DESC_CHAR_CAP);
    }

    /// The footer ctx gauge MUST use the exact same estimator the agent loop's
    /// compaction uses (`compaction::estimate_messages_tokens`), so the displayed %
    /// lines up with the 0.85 compaction trigger. This helper drives that unified
    /// estimator (cast to u64, as the footer does) so the tests assert against it.
    fn ctx_estimate(messages: &[Value]) -> u64 {
        crate::chat::agent::compaction::estimate_messages_tokens(messages) as u64
    }

    #[test]
    fn ctx_estimate_grows_monotonically() {
        // Start with just a system message; appending more messages must never
        // shrink the estimate (the footer ctx gauge must be monotonic per turn).
        let mut messages = vec![json!({ "role": "system", "content": "you are a coding agent" })];
        let s0 = ctx_estimate(&messages);
        messages.push(json!({ "role": "user", "content": "read main.rs and summarize it" }));
        let s1 = ctx_estimate(&messages);
        messages.push(json!({ "role": "assistant", "content": "Here is a summary of the file." }));
        let s2 = ctx_estimate(&messages);
        assert!(s0 < s1, "user message should grow the estimate ({s0} < {s1})");
        assert!(s1 < s2, "assistant message should grow the estimate ({s1} < {s2})");
    }

    #[test]
    fn ctx_estimate_roughly_chars_over_four() {
        // A single message whose content is 400 chars → ~100 tokens (chars/4),
        // plus the small per-message overhead the compaction estimator adds.
        let content = "x".repeat(400);
        let messages = vec![json!({ "role": "user", "content": content })];
        let est = ctx_estimate(&messages);
        // 400 ASCII chars div_ceil 4 = 100, + 4 per-message overhead = 104.
        assert!((100..=110).contains(&est), "estimate {est} should be ~chars/4");
    }

    #[test]
    fn ctx_estimate_counts_tool_calls_and_results() {
        // tool_call function arguments and tool-result content are part of the
        // prompt, so they must count toward the estimate.
        let bare = vec![json!({ "role": "assistant", "content": "calling a tool" })];
        let with_tools = vec![
            json!({
                "role": "assistant",
                "content": "calling a tool",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "read_file", "arguments": "{\"path\":\"a_very_long_file_path/main.rs\"}" }
                }]
            }),
            json!({ "role": "tool", "tool_call_id": "call_1", "content": "the entire contents of the file go here and are long" }),
        ];
        assert!(
            ctx_estimate(&with_tools) > ctx_estimate(&bare),
            "tool_calls + tool results must add to the estimate"
        );
    }

    // ---- TurnRuntime integration (no real model / TTY) ----

    fn provider(id: &str) -> ModelProvider {
        ModelProvider {
            id: id.to_string(),
            name: id.to_string(),
            api_keys: vec!["sk-x".to_string()],
            api_key_legacy: None,
            base_url: "https://example.com/v1".to_string(),
            available_models: vec!["m1".to_string()],
            enabled_models: vec!["m1".to_string()],
            supports_tools: true,
            enabled: true,
            api_format: "openai_chat".to_string(),
            model_overrides: Default::default(),
        }
    }

    fn test_settings() -> Settings {
        let mut s = Settings::default();
        s.providers = vec![provider("chat")];
        s.default_models.chat.provider_id = "chat".to_string();
        s.default_models.chat.model = "m1".to_string();
        s
    }

    fn unique_cwd(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("kivio-code-turn-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp cwd");
        dir
    }

    /// Build a TurnRuntime wired to a real session + headless state, but never
    /// spawns a real model run — tests drive the post-turn logic directly.
    fn turn_runtime(cwd: &PathBuf) -> (TurnRuntime, Receiver<TurnDone>) {
        let settings = test_settings();
        let assembly =
            TurnAssembly::resolve(&settings, None, None, cwd, true).expect("assembly resolves");
        let runtime_messages =
            vec![json!({ "role": "system", "content": assembly.system_prompt.clone() })];
        let session = Session::create(cwd, &assembly.model_label()).expect("session create");
        let state = build_app_state(settings);
        let (done_tx, done_rx) = mpsc::channel::<TurnDone>();
        let rt = TurnRuntime {
            handle: tokio::runtime::Handle::current(),
            state,
            assembly: Arc::new(assembly),
            cwd: cwd.clone(),
            timeout_ms: 120_000,
            generations: Generations::default(),
            current: None,
            current_message_id: None,
            runtime_messages,
            session: Some(session),
            persisted_tool_calls: std::collections::HashSet::new(),
            turn_done_tx: done_tx,
        };
        (rt, done_rx)
    }

    fn result_with(content: &str, api_messages: Vec<Value>, tool_records: Vec<ToolCallRecord>) -> AgentRunResult {
        AgentRunResult {
            content: content.to_string(),
            reasoning: None,
            tool_records,
            segments: Vec::new(),
            api_messages,
            steps: Vec::new(),
            stream_outcome: "completed".to_string(),
            usage: None,
            compacted_history: None,
        }
    }

    fn tool_record(id: &str, name: &str) -> ToolCallRecord {
        ToolCallRecord {
            id: id.to_string(),
            name: name.to_string(),
            source: "native".to_string(),
            server_id: None,
            arguments: serde_json::json!({ "path": "a.txt" }).to_string(),
            status: ToolCallStatus::Success,
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

    #[tokio::test]
    async fn session_create_append_roundtrip_after_simulated_turn() {
        let cwd = unique_cwd("roundtrip");
        let (mut rt, _done) = turn_runtime(&cwd);
        let path = rt.session.as_ref().unwrap().path.clone();

        // Simulate a user submit (without spawning): persist + accumulate.
        rt.append_session(SessionRecord::Message {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            role: "user".to_string(),
            content: "read a.txt".to_string(),
        });
        rt.runtime_messages
            .push(json!({ "role": "user", "content": "read a.txt" }));

        // Simulate the agent finishing with one tool call + an answer.
        let result = result_with(
            "Read it.",
            vec![json!({ "role": "assistant", "content": "Read it." })],
            vec![tool_record("call_1", "read")],
        );
        rt.persist_turn_records(&result);

        // Reload the session from disk and assert the records landed.
        let reloaded = Session::load(&path).expect("reload");
        let roles: Vec<&str> = reloaded
            .records
            .iter()
            .map(|r| match r {
                SessionRecord::Message { role, .. } => role.as_str(),
                SessionRecord::ToolCall { .. } => "tool_call",
                SessionRecord::ToolResult { .. } => "tool_result",
                _ => "other",
            })
            .collect();
        assert_eq!(roles, vec!["user", "tool_call", "tool_result", "assistant"]);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn multi_turn_runtime_messages_accumulate() {
        let cwd = unique_cwd("multiturn");
        let (mut rt, _done) = turn_runtime(&cwd);
        let base = rt.runtime_messages.len(); // 1 (system)

        // Turn 1.
        rt.runtime_messages
            .push(json!({ "role": "user", "content": "first" }));
        let r1 = result_with("answer one", vec![json!({ "role": "assistant", "content": "answer one" })], Vec::new());
        rt.accumulate_runtime_messages(&r1);

        // Turn 2 carries turn-1 context.
        rt.runtime_messages
            .push(json!({ "role": "user", "content": "second" }));
        let r2 = result_with("answer two", vec![json!({ "role": "assistant", "content": "answer two" })], Vec::new());
        rt.accumulate_runtime_messages(&r2);

        // system + (user1 + assistant1) + (user2 + assistant2) = base + 4
        assert_eq!(rt.runtime_messages.len(), base + 4);
        assert_eq!(rt.runtime_messages[0]["role"], "system");
        assert_eq!(rt.runtime_messages[1]["content"], "first");
        assert_eq!(rt.runtime_messages[2]["content"], "answer one");
        assert_eq!(rt.runtime_messages[3]["content"], "second");
        assert_eq!(rt.runtime_messages[4]["content"], "answer two");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    /// R3: when a turn returns `compacted_history`, the interactive accumulator
    /// REPLACES its runtime_messages with the compacted history (rather than
    /// appending api_messages), so compaction shrinks the cross-turn context
    /// instead of the history growing monotonically.
    #[tokio::test]
    async fn compacted_history_replaces_accumulated_runtime_messages() {
        let cwd = unique_cwd("compacted");
        let (mut rt, _done) = turn_runtime(&cwd);

        // Simulate a turn that ballooned then got compacted: pre-fill a large
        // accumulated history, then feed a result carrying a small compacted one.
        for i in 0..40 {
            rt.runtime_messages
                .push(json!({ "role": "tool", "tool_call_id": format!("c{i}"), "content": "X".repeat(2_000) }));
        }
        let bloated_len = rt.runtime_messages.len();

        let compacted = vec![
            json!({ "role": "system", "content": "sys" }),
            json!({ "role": "user", "content": "[context summary] 摘要" }),
            json!({ "role": "assistant", "content": "已了解摘要" }),
            json!({ "role": "assistant", "content": "最终回答" }),
        ];
        let mut result = result_with(
            "最终回答",
            // api_messages is the full per-turn delta — must be IGNORED when
            // compacted_history is present (otherwise we'd grow, not shrink).
            vec![json!({ "role": "assistant", "content": "最终回答" })],
            Vec::new(),
        );
        result.compacted_history = Some(compacted.clone());

        rt.accumulate_runtime_messages(&result);

        // Replaced wholesale with the compacted history (NOT extended).
        assert_eq!(rt.runtime_messages, compacted);
        assert!(
            rt.runtime_messages.len() < bloated_len,
            "compaction must shrink the cross-turn history (was {bloated_len}, now {})",
            rt.runtime_messages.len()
        );
        assert_eq!(rt.runtime_messages[0]["role"], "system");
        assert_eq!(rt.runtime_messages.last().unwrap()["content"], "最终回答");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn tool_record_persisted_once_per_call_id() {
        let cwd = unique_cwd("toolonce");
        let (mut rt, _done) = turn_runtime(&cwd);
        let path = rt.session.as_ref().unwrap().path.clone();

        // Same call_id persisted twice (e.g. emitted again) → only one pair.
        rt.persist_tool_record(&tool_record("call_1", "read"));
        rt.persist_tool_record(&tool_record("call_1", "read"));

        let reloaded = Session::load(&path).expect("reload");
        let tool_calls = reloaded
            .records
            .iter()
            .filter(|r| matches!(r, SessionRecord::ToolCall { .. }))
            .count();
        assert_eq!(tool_calls, 1);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn finish_turn_ignores_stale_generation() {
        let cwd = unique_cwd("stale");
        let (mut rt, _done) = turn_runtime(&cwd);
        let mut app = App::new("~".to_string(), "chat:m1".to_string());
        app.set_terminal_rows(24);
        app.set_mode(AppMode::Generating);

        // Live run is generation 5; a done for generation 3 must be ignored.
        rt.current = Some(RunCancel::new(5));
        let stale = TurnDone {
            generation: 3,
            result: Ok(result_with("ignored", Vec::new(), Vec::new())),
            message_id: "m3".to_string(),
        };
        rt.finish_turn(stale, &mut app);
        // Still generating; the live run was not cleared by a stale done.
        assert!(rt.is_generating());
        assert_eq!(app.mode(), AppMode::Generating);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn finish_turn_live_generation_finalizes_and_idles() {
        let cwd = unique_cwd("live");
        let (mut rt, _done) = turn_runtime(&cwd);
        let mut app = App::new("~".to_string(), "chat:m1".to_string());
        app.set_terminal_rows(24);
        app.set_mode(AppMode::Generating);
        // Stream some content for message m7, then finish that generation.
        app.apply_agent_event(AgentUiEvent::StreamDelta {
            message_id: "m7".to_string(),
            delta: "done answer".to_string(),
            reasoning: String::new(),
        });
        rt.current = Some(RunCancel::new(7));
        let done = TurnDone {
            generation: 7,
            result: Ok(result_with("done answer", Vec::new(), Vec::new())),
            message_id: "m7".to_string(),
        };
        rt.finish_turn(done, &mut app);
        assert!(!rt.is_generating());
        assert_eq!(app.mode(), AppMode::Idle);
        assert!(!app.assistant_streaming());

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    /// Regression guard for the reported footer ctx bug: a turn whose summed usage
    /// `input_tokens` is huge (900k — the loop accumulates planning + every tool round +
    /// synthesis via RunState::merge_usage) must NOT set the ctx gauge to that number.
    /// The ctx gauge reflects the *current conversation size*, which for a tiny 2-message
    /// transcript is small — even though the billed `in` usage is huge.
    #[tokio::test]
    async fn finish_turn_sets_ctx_from_conversation_not_summed_usage() {
        let cwd = unique_cwd("ctxfromconv");
        let (mut rt, _done) = turn_runtime(&cwd);
        // Conversation is just system + a short user message → small ctx estimate.
        rt.runtime_messages
            .push(json!({ "role": "user", "content": "hi" }));

        let mut app = App::new("~".to_string(), "chat:m1".to_string());
        app.set_terminal_rows(24);
        app.set_mode(AppMode::Generating);
        rt.current = Some(RunCancel::new(9));

        // The agent reports a massive SUMMED input usage for the turn.
        let mut result = result_with(
            "ok",
            vec![json!({ "role": "assistant", "content": "ok" })],
            Vec::new(),
        );
        result.usage = Some(crate::chat::model::ModelUsage {
            input_tokens: Some(900_000),
            output_tokens: Some(120),
            ..Default::default()
        });
        let done = TurnDone {
            generation: 9,
            result: Ok(result),
            message_id: "m9".to_string(),
        };
        rt.finish_turn(done, &mut app);

        let ctx = app.context_tokens().expect("ctx set after turn");
        // Must be the small conversation estimate, NOT the 900k summed usage.
        assert!(ctx < 1_000, "ctx {ctx} must reflect the small conversation, not summed usage");
        // And it must match the compaction estimator over the accumulated transcript.
        assert_eq!(ctx, ctx_estimate(&rt.runtime_messages));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    /// `/new` (NewConversation): resetting the conversation must drop the runtime
    /// back to a single system message, start a fresh session file, clear persisted
    /// tool calls, and pull the ctx gauge back to the small system-prompt baseline —
    /// NOT leave it at the prior large value.
    #[tokio::test]
    async fn reset_conversation_drops_to_system_baseline() {
        let cwd = unique_cwd("resetconv");
        let (mut rt, _done) = turn_runtime(&cwd);

        // Build up a big fake conversation and a stale large ctx value.
        let old_session_path = rt.session.as_ref().unwrap().path.clone();
        for i in 0..20 {
            rt.runtime_messages
                .push(json!({ "role": "user", "content": "x".repeat(500) }));
            rt.runtime_messages
                .push(json!({ "role": "assistant", "content": "y".repeat(500) }));
            rt.persisted_tool_calls.insert(format!("call_{i}"));
        }
        let big = ctx_estimate(&rt.runtime_messages);

        let mut app = App::new("~".to_string(), "chat:m1".to_string());
        app.set_terminal_rows(24);
        app.set_context_tokens(Some(big));
        app.set_mode(AppMode::Generating);

        rt.reset_conversation(&mut app);

        // runtime_messages is back to a single system message.
        assert_eq!(rt.runtime_messages.len(), 1);
        assert_eq!(rt.runtime_messages[0]["role"], "system");
        // persisted tool calls cleared.
        assert!(rt.persisted_tool_calls.is_empty());
        // a fresh session file was created (different from the old one).
        let new_session_path = rt.session.as_ref().unwrap().path.clone();
        assert_ne!(new_session_path, old_session_path);
        // mode back to Idle.
        assert_eq!(app.mode(), AppMode::Idle);
        // ctx gauge is the small system-prompt baseline, NOT the prior large value.
        let ctx = app.context_tokens().expect("ctx set after reset");
        assert_eq!(ctx, ctx_estimate(&rt.runtime_messages));
        assert!(ctx < big, "ctx {ctx} must drop below the prior large value {big}");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    /// `apply_effect(AppEffect::NewConversation, …)` clears the transcript, re-shows
    /// the welcome header, and resets the runtime.
    #[tokio::test]
    async fn apply_effect_new_conversation_resets_runtime() {
        let cwd = unique_cwd("applyreset");
        let (mut rt, _done) = turn_runtime(&cwd);
        rt.runtime_messages
            .push(json!({ "role": "user", "content": "old turn" }));

        let mut app = App::new("~".to_string(), "chat:m1".to_string());
        app.set_terminal_rows(24);
        app.push_assistant("old answer");
        app.set_show_welcome(false);
        app.set_context_tokens(Some(99_999));
        let (agent_tx, _agent_rx) = mpsc::channel::<AgentUiEvent>();

        let flow = apply_effect(
            AppEffect::NewConversation,
            &mut app,
            &agent_tx,
            Some(&mut rt),
        );
        assert!(matches!(flow, EffectFlow::Continue));
        // transcript cleared.
        assert_eq!(app.transcript_len(), 0);
        // runtime reset to system-only; ctx back to the small baseline.
        assert_eq!(rt.runtime_messages.len(), 1);
        let ctx = app.context_tokens().expect("ctx set");
        assert_eq!(ctx, ctx_estimate(&rt.runtime_messages));
        assert!(ctx < 99_999);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    /// Startup must seed the ctx gauge from the fresh session's runtime_messages
    /// (a single system message → a small baseline), not leave it unset/stale.
    #[tokio::test]
    async fn startup_seeds_ctx_from_fresh_runtime_messages() {
        let cwd = unique_cwd("startupctx");
        let (rt, _done) = turn_runtime(&cwd);
        // Fresh session: runtime_messages is just [system].
        assert_eq!(rt.runtime_messages.len(), 1);

        let mut app = App::new("~".to_string(), "chat:m1".to_string());
        app.set_terminal_rows(24);
        // Mirror what run() does after constructing the TurnRuntime.
        app.set_context_tokens(Some(ctx_estimate(&rt.runtime_messages)));

        let ctx = app.context_tokens().expect("startup ctx set");
        assert_eq!(ctx, ctx_estimate(&rt.runtime_messages));
        // The system-prompt baseline is small (a few k at most), not nothing/huge.
        assert!(ctx < 5_000, "startup ctx {ctx} should be the small system-prompt baseline");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    /// Settings with two enabled providers, each with two enabled models.
    fn multi_provider_settings() -> Settings {
        let mut s = Settings::default();
        let mut p1 = provider("chat");
        p1.enabled_models = vec!["m1".to_string(), "m2".to_string()];
        let mut p2 = provider("alt");
        p2.enabled_models = vec!["x1".to_string()];
        s.providers = vec![p1, p2];
        s.default_models.chat.provider_id = "chat".to_string();
        s.default_models.chat.model = "m1".to_string();
        s
    }

    fn turn_runtime_with(settings: Settings, cwd: &PathBuf) -> (TurnRuntime, Receiver<TurnDone>) {
        let assembly =
            TurnAssembly::resolve(&settings, None, None, cwd, true).expect("assembly resolves");
        let runtime_messages =
            vec![json!({ "role": "system", "content": assembly.system_prompt.clone() })];
        let session = Session::create(cwd, &assembly.model_label()).expect("session create");
        let state = build_app_state(settings);
        let (done_tx, done_rx) = mpsc::channel::<TurnDone>();
        let rt = TurnRuntime {
            handle: tokio::runtime::Handle::current(),
            state,
            assembly: Arc::new(assembly),
            cwd: cwd.clone(),
            timeout_ms: 120_000,
            generations: Generations::default(),
            current: None,
            current_message_id: None,
            runtime_messages,
            session: Some(session),
            persisted_tool_calls: std::collections::HashSet::new(),
            turn_done_tx: done_tx,
        };
        (rt, done_rx)
    }

    #[tokio::test]
    async fn enabled_model_items_lists_all_enabled() {
        let cwd = unique_cwd("modelitems");
        let (rt, _done) = turn_runtime_with(multi_provider_settings(), &cwd);
        let items = rt.enabled_model_items();
        let values: Vec<String> = items.iter().map(|(v, _, _)| v.clone()).collect();
        assert!(values.contains(&"chat:m1".to_string()));
        assert!(values.contains(&"chat:m2".to_string()));
        assert!(values.contains(&"alt:x1".to_string()));
        assert_eq!(values.len(), 3);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn switch_model_reresolves_assembly_and_records_change() {
        let cwd = unique_cwd("switchmodel");
        let (mut rt, _done) = turn_runtime_with(multi_provider_settings(), &cwd);
        assert_eq!(rt.model_label(), "chat:m1");

        let label = rt.switch_model("alt:x1").expect("switch ok");
        assert_eq!(label, "alt:x1");
        assert_eq!(rt.model_label(), "alt:x1");

        // A ModelChange record was persisted.
        let path = rt.session.as_ref().unwrap().path.clone();
        let reloaded = Session::load(&path).expect("reload");
        assert!(reloaded
            .records
            .iter()
            .any(|r| matches!(r, SessionRecord::ModelChange { model, .. } if model == "alt:x1")));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn switch_model_unknown_provider_errors() {
        let cwd = unique_cwd("switchbad");
        let (mut rt, _done) = turn_runtime_with(multi_provider_settings(), &cwd);
        assert!(rt.switch_model("nope:zzz").is_err());
        // unchanged
        assert_eq!(rt.model_label(), "chat:m1");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    // ---- FIX 2: display label uses provider name; resolution stays id-based ----

    /// Settings whose provider id is an opaque token but has a friendly name.
    fn named_provider_settings() -> Settings {
        let mut s = Settings::default();
        let mut p = provider("provider-1780492912291");
        p.name = "DeepSeek Pool".to_string();
        p.enabled_models = vec!["deepseek-v4-flash".to_string()];
        s.providers = vec![p];
        s.default_models.chat.provider_id = "provider-1780492912291".to_string();
        s.default_models.chat.model = "deepseek-v4-flash".to_string();
        s
    }

    #[tokio::test]
    async fn model_display_label_uses_name_while_resolution_stays_id() {
        let cwd = unique_cwd("displaylabel");
        let (rt, _done) = turn_runtime_with(named_provider_settings(), &cwd);
        // Resolution value (selector / resume) is id-based.
        assert_eq!(rt.model_label(), "provider-1780492912291:deepseek-v4-flash");
        // Display label is the human-readable provider name.
        assert_eq!(rt.model_display_label(), "DeepSeek Pool · deepseek-v4-flash");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn switch_model_still_resolves_with_id_based_value() {
        // /model selector emits the id-based value (enabled_model_items); switching
        // must resolve it back to the provider even though the footer shows the name.
        let cwd = unique_cwd("switchresolves");
        let (mut rt, _done) = turn_runtime_with(named_provider_settings(), &cwd);
        // The selector item value is the id-based pair.
        let items = rt.enabled_model_items();
        let value = items[0].0.clone();
        assert_eq!(value, "provider-1780492912291:deepseek-v4-flash");
        // Switching with that value resolves and keeps the id-based label.
        let label = rt.switch_model(&value).expect("switch resolves");
        assert_eq!(label, "provider-1780492912291:deepseek-v4-flash");
        // …and the display label is still name-based.
        assert_eq!(rt.model_display_label(), "DeepSeek Pool · deepseek-v4-flash");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    // ---- FIX 3: context window resolution ----

    #[tokio::test]
    async fn context_window_known_model_is_some() {
        let cwd = unique_cwd("ctxwindow");
        // deepseek* resolves to a fallback (is_fallback=true) → None per our policy,
        // but a model name carrying an explicit window (e.g. "...-128k") is known.
        let mut s = Settings::default();
        let mut p = provider("p");
        p.enabled_models = vec!["my-model-128k".to_string()];
        s.providers = vec![p];
        s.default_models.chat.provider_id = "p".to_string();
        s.default_models.chat.model = "my-model-128k".to_string();
        let (rt, _done) = turn_runtime_with(s, &cwd);
        assert_eq!(rt.context_window(), Some(128_000));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn context_window_unknown_model_is_none() {
        let cwd = unique_cwd("ctxunknown");
        let mut s = Settings::default();
        let mut p = provider("p");
        p.enabled_models = vec!["totally-unknown-model".to_string()];
        s.providers = vec![p];
        s.default_models.chat.provider_id = "p".to_string();
        s.default_models.chat.model = "totally-unknown-model".to_string();
        let (rt, _done) = turn_runtime_with(s, &cwd);
        // Fallback guess → None, so the footer degrades to raw tokens.
        assert_eq!(rt.context_window(), None);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    // ---- Phase 5c: session resume ----

    #[tokio::test]
    async fn session_items_and_resolve_recent() {
        let cwd = unique_cwd("resumeitems");
        // No sessions yet.
        assert!(session_items_for_cwd(&cwd).is_empty());

        // Create a session with a user message so it shows a preview.
        let mut s = Session::create(&cwd, "chat:m1").unwrap();
        s.append(SessionRecord::Message {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            role: "user".to_string(),
            content: "summarize the readme".to_string(),
        })
        .unwrap();

        let items = session_items_for_cwd(&cwd);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, s.path.to_string_lossy());
        assert!(items[0].2.as_deref().unwrap().contains("summarize"));

        // resolve_session with Recent rebuilds messages + transcript.
        let settings = test_settings();
        let assembly = TurnAssembly::resolve(&settings, None, None, &cwd, true).unwrap();
        let mut app = App::new("~".into(), "chat:m1".into());
        app.set_terminal_rows(24);
        let (session, messages, resumed) =
            resolve_session(&Some(ResumeRequest::Recent), &cwd, &assembly, &mut app);
        assert!(resumed);
        assert!(session.is_some());
        // system + user
        assert!(messages.iter().any(|m| m["role"] == "system"));
        assert!(messages.iter().any(|m| m["content"] == "summarize the readme"));
        // transcript shows the user message.
        assert!(app.render(80).join("\n").contains("summarize the readme"));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn resolve_session_reference_by_id() {
        let cwd = unique_cwd("resumeid");
        let mut s = Session::create(&cwd, "chat:m1").unwrap();
        // Append a real message so the session materializes (lazy creation) and
        // counts as resumable (non-empty).
        s.append(SessionRecord::Message {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            role: "user".to_string(),
            content: "earlier question".to_string(),
        })
        .unwrap();
        let id = s.id.clone();
        let settings = test_settings();
        let assembly = TurnAssembly::resolve(&settings, None, None, &cwd, true).unwrap();
        let mut app = App::new("~".into(), "chat:m1".into());
        app.set_terminal_rows(24);
        // partial id prefix resolves.
        let prefix: String = id.chars().take(8).collect();
        let (session, _messages, resumed) = resolve_session(
            &Some(ResumeRequest::Reference(prefix)),
            &cwd,
            &assembly,
            &mut app,
        );
        assert!(resumed);
        assert_eq!(session.unwrap().id, id);

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn resolve_session_no_match_starts_new() {
        let cwd = unique_cwd("resumenone");
        let settings = test_settings();
        let assembly = TurnAssembly::resolve(&settings, None, None, &cwd, true).unwrap();
        let mut app = App::new("~".into(), "chat:m1".into());
        app.set_terminal_rows(24);
        let (session, messages, resumed) =
            resolve_session(&Some(ResumeRequest::Recent), &cwd, &assembly, &mut app);
        // No existing session → falls back to a brand-new one.
        assert!(!resumed);
        assert!(session.is_some());
        assert!(messages.is_empty()); // caller seeds system

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn resolve_session_only_empty_does_not_lie_about_resume() {
        let cwd = unique_cwd("resumeemptyonly");
        std::fs::create_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd)).unwrap();
        // A header-only shell on disk (the old bug's "empty session").
        let empty = Session::create(&cwd, "chat:m1").unwrap();
        let header = crate::kivio_code::session::SessionRecord::Header {
            version: crate::kivio_code::session::SESSION_VERSION,
            id: empty.id.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            created_at: chrono::Utc::now().to_rfc3339(),
            model: "chat:m1".to_string(),
        };
        std::fs::write(&empty.path, format!("{}\n", serde_json::to_string(&header).unwrap()))
            .unwrap();

        let settings = test_settings();
        let assembly = TurnAssembly::resolve(&settings, None, None, &cwd, true).unwrap();
        let mut app = App::new("~".into(), "chat:m1".into());
        app.set_terminal_rows(24);
        let (session, messages, resumed) =
            resolve_session(&Some(ResumeRequest::Recent), &cwd, &assembly, &mut app);
        // Only-empty sessions → not resumed, fresh start, no false "Resumed session."
        assert!(!resumed);
        assert!(session.is_some());
        assert!(messages.is_empty());
        let rendered = app.render(80).join("\n");
        assert!(!rendered.contains("Resumed session."), "no false resume notice");

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn session_items_excludes_empty_sessions() {
        let cwd = unique_cwd("itemsexcludeempty");
        std::fs::create_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd)).unwrap();

        // An empty shell (no user message).
        let empty = Session::create(&cwd, "chat:m1").unwrap();
        let header = crate::kivio_code::session::SessionRecord::Header {
            version: crate::kivio_code::session::SESSION_VERSION,
            id: empty.id.clone(),
            cwd: cwd.to_string_lossy().into_owned(),
            created_at: chrono::Utc::now().to_rfc3339(),
            model: "chat:m1".to_string(),
        };
        std::fs::write(&empty.path, format!("{}\n", serde_json::to_string(&header).unwrap()))
            .unwrap();

        // A real session with a user message.
        let mut real = Session::create(&cwd, "chat:m1").unwrap();
        real.append(SessionRecord::Message {
            id: String::new(),
            parent_id: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            role: "user".to_string(),
            content: "real one".to_string(),
        })
        .unwrap();

        let items = session_items_for_cwd(&cwd);
        // Only the non-empty session is offered.
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, real.path.to_string_lossy());

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    #[tokio::test]
    async fn resume_session_path_swaps_runtime_messages() {
        let cwd = unique_cwd("resumepath");
        let (mut rt, _done) = turn_runtime_with(test_settings(), &cwd);

        // Build a separate session on disk with two messages.
        let mut other = Session::create(&cwd, "chat:m1").unwrap();
        other
            .append(SessionRecord::Message {
                id: String::new(),
                parent_id: None,
                timestamp: chrono::Utc::now().to_rfc3339(),
                role: "user".to_string(),
                content: "earlier question".to_string(),
            })
            .unwrap();
        let other_path = other.path.to_string_lossy().into_owned();

        let mut app = App::new("~".into(), "chat:m1".into());
        app.set_terminal_rows(24);
        assert!(rt.resume_session_path(&other_path, &mut app));
        // runtime_messages rebuilt from the chosen session (system + user).
        assert!(rt.runtime_messages.iter().any(|m| m["content"] == "earlier question"));
        assert!(rt.runtime_messages.iter().any(|m| m["role"] == "system"));
        // session now points at the resumed file.
        assert_eq!(rt.session.as_ref().unwrap().path.to_string_lossy(), other_path);
        // ctx gauge is refreshed from the resumed conversation size immediately,
        // using the same compaction estimator.
        assert_eq!(
            app.context_tokens(),
            Some(ctx_estimate(&rt.runtime_messages))
        );

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }

    /// In plan mode, begin_turn appends the plan-mode system note to the PER-TURN
    /// message clone only — the stored `runtime_messages` must grow by exactly one
    /// (the user message), NOT by the plan note too.
    #[tokio::test]
    async fn begin_turn_plan_mode_does_not_grow_stored_runtime_messages() {
        let cwd = unique_cwd("planturn");
        let (mut rt, _done) = turn_runtime(&cwd);
        let before = rt.runtime_messages.len(); // [system]
        let (agent_tx, _agent_rx) = mpsc::channel::<AgentUiEvent>();

        rt.begin_turn(
            "plan this".to_string(),
            Vec::new(),
            &agent_tx,
            /* plan_mode */ true,
        );

        // Only the user message was appended to the stored context — the transient
        // plan-mode system note lives on the spawned task's clone, not here.
        assert_eq!(rt.runtime_messages.len(), before + 1);
        assert_eq!(rt.runtime_messages.last().unwrap()["content"], "plan this");
        assert_eq!(rt.runtime_messages.last().unwrap()["role"], "user");
        // No plan-mode system note leaked into the stored messages.
        assert!(
            !rt.runtime_messages
                .iter()
                .any(|m| m["content"] == crate::kivio_code::PLAN_SYSTEM_NOTE),
            "plan note must not be stored in runtime_messages"
        );

        rt.request_cancel();
        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_dir_all(crate::kivio_code::session::session_dir_for_cwd(&cwd));
    }
}
