use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};

use reqwest::Client;
use serde::Serialize;
use tokio::sync::oneshot;

#[cfg(target_os = "macos")]
use crate::macos_ocr::MacOcrClient;
use crate::mcp::manager::McpSession;
use crate::mcp::types::PythonRunResult;
use crate::mcp::ChatToolDefinition;
use crate::native_tools::SandboxExportContext;
use crate::rapidocr::RapidOcrClient;
use crate::settings::Settings;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChatExternalAttachment {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug)]
pub struct PendingPythonRun {
    pub sender: oneshot::Sender<PythonRunResult>,
    pub export_ctx: SandboxExportContext,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChatExternalMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChatExternalSend {
    pub id: String,
    pub content: String,
    pub attachments: Vec<PendingChatExternalAttachment>,
    /// 可选的多轮历史。为空 → 旧的「单条消息」交接路径；非空 → 用历史预置一个新会话，
    /// 不触发回复（截图作为首个 user 轮的附件，见 attachments）。
    #[serde(default)]
    pub messages: Vec<PendingChatExternalMessage>,
}

/// 应用全局状态
/// 使用 RwLock 保护 settings，允许多读单写；
/// Mutex 用于 explain_images 等需要独占访问的数据；
/// AtomicBool 标记 lens 是否正在进行，防止并发热键触发。
pub struct AppState {
    pub settings: RwLock<Settings>,
    pub explain_images: Mutex<HashMap<String, PathBuf>>,
    pub current_explain_image_id: Mutex<Option<String>>,
    pub lens_busy: AtomicBool,
    /// macOS：打开浮窗前记下的前台 App PID（0 = 无 / 前台就是 Kivio 自己），关闭浮窗时据此把
    /// 前台交还给原来的 App，避免 Kivio 变成"前台却无窗口"而触发 RunEvent::Reopen 误开 Chat。
    /// lens（含截图/选词翻译）与输入翻译是各自独立、可同时存在的浮窗，各占一个槽，避免相互覆盖。
    /// 详见 spec/backend/window-lifecycle.md。
    pub prev_frontmost_pid_lens: AtomicI32,
    pub prev_frontmost_pid_main: AtomicI32,
    /// 流式取消代号：每开新的流就 +1，跑流的循环检测到代号变了就立即结束。
    pub explain_stream_generation: AtomicU64,
    /// Chat 流式取消代号分配器。仅作**单调递增的 generation 号分配器**（never 重用）：
    /// 每条 run 取一个全局唯一号，不表达「活跃」语义。会话内唯一即够用（号只用于集合成员
    /// 判定），故用一个进程级 `AtomicU64` 计数器即可，无需按 conversation_id 分桶。
    /// 「哪些 generation 当前有效」由 `chat_active_generations` 表达——这样同一会话可同时
    /// 有多条并发 run（多模型一问多答），开新 run 不再作废兄弟 run。
    pub chat_stream_generation: AtomicU64,
    /// 每个 conversation_id 当前**活跃**的 generation 集合。`next_chat_generation` 往里加，
    /// run 结束 `end_chat_generation` 移除，`cancel_chat_generation` 整会话清空（取消全部在跑 run）。
    /// 单 run 时集合恒为单元素，语义与旧「单 u64」等价。
    pub chat_active_generations: Mutex<HashMap<String, HashSet<u64>>>,
    /// 正在进行 assistant 回复生成的 (conversation_id → run_id 集合)，防止同对话同一 run 重复，
    /// 但允许同一会话多条 run 并存（多模型一问多答）。
    pub chat_active_replies: Mutex<HashMap<String, HashSet<String>>>,
    /// 等待用户确认的敏感 Chat tool 调用。
    pub pending_chat_tool_approvals: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    /// 本会话(conversation_id)已授予「文件/命令」工具的会话级授权集合。
    /// 仅内存、不持久化:重启后重新授权(也是一道轻量安全属性)。
    pub chat_session_consent: Mutex<HashSet<String>>,
    /// 等待用户响应的会话级授权请求(按 conversation_id,同一会话同时至多一个)。
    pub pending_chat_session_consents: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    /// 串行化会话授权弹窗:同一时刻全局只发一个授权请求。首轮多个并行只读工具
    /// (read/grep/find/ls)同时触发授权时,避免互相覆盖 pending sender 导致「假拒绝」——
    /// 拿到锁后先复查 has_chat_consent,领头者授权后其余直接复用、不再弹窗。
    pub chat_consent_prompt_lock: tokio::sync::Mutex<()>,
    /// 等待用户回答的 Chat ask_user 澄清卡片。
    pub pending_chat_user_prompts:
        Mutex<HashMap<String, crate::chat::ask_user::PendingAskUserPrompt>>,
    /// 等待前端 Pyodide 完成的 run_python 调用。
    pub pending_python_runs: Mutex<HashMap<String, PendingPythonRun>>,
    /// 保护 Chat 空白会话复用的短临界区，避免快速多次新建时并发创建多个空白对话。
    pub chat_create_conversation_lock: Mutex<()>,
    /// Chat MCP/native tool 列表缓存。key 由工具相关 settings 生成，避免每轮对话重复冷启动 server。
    pub chat_tool_list_cache: Mutex<HashMap<String, (Instant, Vec<ChatToolDefinition>)>>,
    /// 外部 CLI 斜杠命令探测缓存（agent_id:cwd → 命令列表）。
    pub external_slash_commands_cache:
        Mutex<HashMap<String, (Instant, Vec<crate::external_agents::types::ExternalCliSlashCommand>)>>,
    /// 外部 CLI 模型列表探测缓存（agent_id → 模型选项）。
    pub external_agent_models_cache: Mutex<
        HashMap<String, (Instant, Vec<crate::external_agents::types::RuntimeModelOption>)>,
    >,
    /// 外部 CLI 全量检测结果缓存（available/version/auth/models）。避免 RuntimePicker / 设置页
    /// 每次打开都串行重探 8 个 CLI（含 auth 探测超时）。force_refresh 时跳过。
    pub external_detected_agents_cache:
        Mutex<Option<(Instant, Vec<crate::external_agents::types::DetectedAgent>)>>,
    /// Phase 2 持久会话注册表：conversation_id → 活会话（仅持有控制通道，不持有 Child）。
    /// 仅在 get/insert/remove 时短暂持锁，绝不跨 turn await 持锁。
    pub external_live_sessions:
        Mutex<HashMap<String, crate::external_agents::session::live::LiveSession>>,
    /// 外部入口（例如 Lens）交给 Chat 前端发送的待处理消息。
    /// 后端只负责保存请求和打开窗口，实际发送必须走 Chat 前端的手动发送状态机。
    pub pending_chat_external_sends: Mutex<Vec<PendingChatExternalSend>>,
    /// Lens 启动前抓到的选中文本：放在这里等前端 enterSelect 来取走。
    /// 取一次清一次（take 语义）。无选中 / 取过 / translate 模式 = None。
    pub pending_selection: Mutex<Option<String>>,
    /// Windows 冻结帧选择模式的临时截图 id。仅在进入 select 态前预抓屏幕时使用。
    pub lens_freeze_frame_image_id: Mutex<Option<String>>,
    /// Lens 进入 select 态的复位载荷（frame + freezeFrameImageId 的 JSON）。前端冷挂载时
    /// 主动 take 来兜底：Windows 关闭即销毁后,下次冷启的 webview 可能晚于 Rust 的 lens:reset
    /// eval 才挂上监听 → 事件被丢。改放 AppState 供拉取,丢事件也不丢冻结帧。take 语义。
    pub lens_pending_reset: Mutex<Option<String>>,
    /// API Key 多 key failover 状态：(provider_id, key_idx) → 冷却到期时间。
    /// 某个 key 触发 quota/rate-limit/auth 失败时进入冷却，KEY_COOLDOWN 秒内不再选用。
    pub key_cooldowns: Mutex<HashMap<(String, usize), Instant>>,
    /// 每个 provider 当前活跃 key idx：上一次成功的 key 优先继续用。
    pub active_key_idx: Mutex<HashMap<String, usize>>,
    /// 运行时学习到的"该端点拒绝 `prompt_cache_key`"集合（按 base_url）。
    /// 某端点首次因该字段 400 后记入，本会话后续请求不再发，避免重复触发 + 无谓重试。
    pub prompt_cache_key_unsupported: Mutex<HashSet<String>>,
    /// MCP 持久连接池：server_id → 该 server 的长连接会话。
    /// 每会话独立 `Arc<Mutex>`，A 服务器握手不阻塞 B；外层 `tokio::sync::Mutex`
    /// 只在命中判断 / 插入 / 移除时短暂持有，绝不跨握手 await。
    pub mcp_sessions: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<McpSession>>>>,
    /// Token usage ledger directory under app data. Model providers can append records
    /// without needing an AppHandle threaded through every call path.
    pub usage_dir: PathBuf,
    pub http: Client,
    /// macOS Apple Vision OCR sidecar 客户端。只有系统 OCR 路径会拉起。
    #[cfg(target_os = "macos")]
    pub macos_ocr: std::sync::Arc<MacOcrClient>,
    /// RapidOCR 离线 OCR 客户端。模型 + onnxruntime dylib 都由用户在设置页面下载到 app data 目录,
    /// 安装包不带任何 ONNX Runtime 二进制。`status()` 检查 4 个文件齐不齐, 不齐让前端引导下载。
    pub rapidocr: std::sync::Arc<RapidOcrClient>,
    /// 多 agent / 子 agent 任务表（P3）：spawn 的子 agent 状态、按名寻址、并发上限。
    pub sub_agents: crate::chat::sub_agent::SubAgentManager,
    /// 后台 run_command 进程注册表：job_id → 跟踪中的后台命令。
    /// 与后台 subagent 不同：这些命令**跨 turn 存活**，只由显式 `kill_background`
    /// 或 app 退出 sweep 清理（对齐 Claude Code background bash，dev-server 友好），
    /// 不随发起的 run 取消。仅在 insert/lookup/sweep 时短暂持锁。
    pub background_commands:
        Arc<Mutex<HashMap<String, crate::native_tools::BackgroundCommand>>>,
    /// 开发者「请求调试」内存环形缓冲：最近 [`REQUEST_DEBUG_CAPACITY`] 条 provider 调用的
    /// 请求（脱敏 headers + body）+ 响应摘要。默认关闭（`chat_tools.request_debug_enabled`），
    /// 关闭时 adapter 短路、不构造记录。仅内存、不落盘，进程退出即清。
    pub request_debug: Mutex<VecDeque<crate::chat::request_debug::RequestDebugRecord>>,
}

/// 单个 key 触发 failover 后的冷却时长。
pub const KEY_COOLDOWN: Duration = Duration::from_secs(60);

/// 共享的 TTL 缓存读取：命中且未过期返回克隆，否则移除过期条目并返回 None。
fn get_cached<V: Clone>(
    cache: &Mutex<HashMap<String, (Instant, V)>>,
    key: &str,
    ttl: Duration,
) -> Option<V> {
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((created_at, value)) = cache.get(key) {
        if created_at.elapsed() <= ttl {
            return Some(value.clone());
        }
    }
    cache.remove(key);
    None
}

/// 共享的 TTL 缓存写入：以当前时刻为创建时间插入。
fn set_cached<V>(cache: &Mutex<HashMap<String, (Instant, V)>>, key: String, value: V) {
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, (Instant::now(), value));
}

impl AppState {
    /// 集中构造点：`lib.rs::run` 的 `app.manage`、`new_headless`、以及测试用 `test_app_state`
    /// 三处唯一的差异只有 `settings` / `usage_dir` / `http` 与两个 OCR 客户端；其余字段全是
    /// 同样的空默认值。这里统一构造，三处只提供差异字段，避免同一份 ~40 行字面量重复三次。
    pub(crate) fn base(
        settings: Settings,
        usage_dir: PathBuf,
        http: Client,
        #[cfg(target_os = "macos")] macos_ocr: std::sync::Arc<MacOcrClient>,
        rapidocr: std::sync::Arc<RapidOcrClient>,
    ) -> Self {
        AppState {
            settings: RwLock::new(settings),
            explain_images: Mutex::new(HashMap::new()),
            current_explain_image_id: Mutex::new(None),
            lens_busy: AtomicBool::new(false),
            prev_frontmost_pid_lens: AtomicI32::new(0),
            prev_frontmost_pid_main: AtomicI32::new(0),
            explain_stream_generation: AtomicU64::new(0),
            chat_stream_generation: AtomicU64::new(0),
            chat_active_generations: Mutex::new(HashMap::new()),
            chat_active_replies: Mutex::new(HashMap::new()),
            pending_chat_tool_approvals: Mutex::new(HashMap::new()),
            chat_session_consent: Mutex::new(HashSet::new()),
            pending_chat_session_consents: Mutex::new(HashMap::new()),
            chat_consent_prompt_lock: tokio::sync::Mutex::new(()),
            pending_chat_user_prompts: Mutex::new(HashMap::new()),
            pending_python_runs: Mutex::new(HashMap::new()),
            chat_create_conversation_lock: Mutex::new(()),
            chat_tool_list_cache: Mutex::new(HashMap::new()),
            external_slash_commands_cache: Mutex::new(HashMap::new()),
            external_agent_models_cache: Mutex::new(HashMap::new()),
            external_detected_agents_cache: Mutex::new(None),
            external_live_sessions: Mutex::new(HashMap::new()),
            pending_chat_external_sends: Mutex::new(Vec::new()),
            pending_selection: Mutex::new(None),
            lens_freeze_frame_image_id: Mutex::new(None),
            lens_pending_reset: Mutex::new(None),
            key_cooldowns: Mutex::new(HashMap::new()),
            active_key_idx: Mutex::new(HashMap::new()),
            prompt_cache_key_unsupported: Mutex::new(HashSet::new()),
            mcp_sessions: tokio::sync::Mutex::new(HashMap::new()),
            usage_dir,
            http,
            #[cfg(target_os = "macos")]
            macos_ocr,
            rapidocr,
            sub_agents: crate::chat::sub_agent::SubAgentManager::default(),
            background_commands: Arc::new(Mutex::new(HashMap::new())),
            request_debug: Mutex::new(VecDeque::new()),
        }
    }

    /// Build a headless `AppState` for the `kivio-code` terminal agent — no
    /// `AppHandle`, no Tauri runtime. Differs from the live construction in
    /// `lib.rs::run` only in the two OCR clients (`headless()` constructors) and
    /// `usage_dir` (passed in). The agent loop only touches `settings`, the
    /// chat-generation state, session-consent set, `http`, and `usage_dir`; the
    /// rest are inert defaults kept for struct completeness.
    pub fn new_headless(settings: Settings, usage_dir: PathBuf) -> Self {
        Self::base(
            settings,
            usage_dir,
            crate::api::build_http_client(),
            #[cfg(target_os = "macos")]
            MacOcrClient::headless(),
            RapidOcrClient::headless(crate::api::build_http_client()),
        )
    }
    /// 安全读取设置（锁中毒时返回内部数据，不 panic）
    pub fn settings_read(&self) -> std::sync::RwLockReadGuard<'_, Settings> {
        self.settings.read().unwrap_or_else(|e| e.into_inner())
    }
    /// 安全写入设置（锁中毒时返回内部数据，不 panic）
    pub fn settings_write(&self) -> std::sync::RwLockWriteGuard<'_, Settings> {
        self.settings.write().unwrap_or_else(|e| e.into_inner())
    }
    /// 开发者「请求调试」开关。关时 adapter 短路，不构造任何记录（零开销）。
    pub fn request_debug_enabled(&self) -> bool {
        self.settings_read().chat_tools.request_debug_enabled
    }
    /// 安全获取解释图片映射锁
    pub fn images_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PathBuf>> {
        self.explain_images
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }
    /// 安全获取当前解释图片 ID 锁
    pub fn current_id_lock(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.current_explain_image_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// 选择一个可用的 API Key 索引：
    /// 优先返回 active_key_idx 记录的 idx；若它在冷却中或已被试过，退回到下一个非冷却 idx；
    /// 全部冷却或 tried 已穷举时返回 None（调用方决定是否报错）。
    pub fn pick_active_key(
        &self,
        provider_id: &str,
        total: usize,
        tried: &HashSet<usize>,
    ) -> Option<usize> {
        if total == 0 {
            return None;
        }
        let now = Instant::now();
        let cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
        let active = self
            .active_key_idx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(provider_id)
            .copied()
            .unwrap_or(0)
            .min(total.saturating_sub(1));

        let in_cooldown = |idx: usize| {
            cooldowns
                .get(&(provider_id.to_string(), idx))
                .map(|until| *until > now)
                .unwrap_or(false)
        };

        // 1) 优先 active idx（未试过 + 未冷却）
        if !tried.contains(&active) && !in_cooldown(active) {
            return Some(active);
        }
        // 2) 从 active+1 开始环绕扫描
        for offset in 1..total {
            let idx = (active + offset) % total;
            if !tried.contains(&idx) && !in_cooldown(idx) {
                return Some(idx);
            }
        }
        // 3) 全部冷却 → 兜底找一个未试过的（无视冷却，避免完全无 key 可用）
        for offset in 0..total {
            let idx = (active + offset) % total;
            if !tried.contains(&idx) {
                return Some(idx);
            }
        }
        None
    }

    /// 为某个 Chat conversation 开启一轮新的可取消运行，返回本轮 generation。
    /// 分配一个进程内从未用过的 generation 号（全局单调递增），并登记到活跃集合。
    /// **不**作废同会话其它在跑 run —— 多模型一问多答时 N 条 run 各持自己的 generation 并存。
    pub fn next_chat_generation(&self, conversation_id: &str) -> u64 {
        let next = self.chat_stream_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.chat_active_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(conversation_id.to_string())
            .or_default()
            .insert(next);
        next
    }

    /// 取消指定 conversation 的**所有**当前 Chat 运行：清空其活跃 generation 集合，
    /// 使任何持旧 generation 的 run（含同会话并发的多模型 run）在下一个检查点判失效。
    /// 直接移除键：空集合与无键语义等价（is_active 都判 false），且 sub-agent 每次
    /// spawn 用一次性合成 conversation_id，留空集合会无界累积。
    pub fn cancel_chat_generation(&self, conversation_id: &str) {
        self.chat_active_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id);
    }

    /// 单条 run 自然结束时退役其 generation（不影响同会话其它在跑 run）。
    /// 单模型路径下集合恒只含本 run 的一个号，移除后即变空，与旧「cancel 推代号」等价。
    /// 集合变空时移除键，防止一次性 conversation_id（sub-agent 合成会话）留下空条目。
    pub fn end_chat_generation(&self, conversation_id: &str, generation: u64) {
        let mut map = self
            .chat_active_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(active) = map.get_mut(conversation_id) {
            active.remove(&generation);
            if active.is_empty() {
                map.remove(conversation_id);
            }
        }
    }

    /// 该会话是否已授予文件/命令工具的会话级授权。
    pub fn has_chat_consent(&self, conversation_id: &str) -> bool {
        self.chat_session_consent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(conversation_id)
    }

    /// 记录该会话已授予文件/命令工具的会话级授权(本进程内有效)。
    pub fn grant_chat_consent(&self, conversation_id: &str) {
        self.chat_session_consent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(conversation_id.to_string());
    }

    /// 判断指定 conversation 的某条 Chat 运行是否仍然有效（其 generation 仍在活跃集合内）。
    pub fn is_chat_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
        self.chat_active_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(conversation_id)
            .map(|active| active.contains(&generation))
            .unwrap_or(false)
    }

    /// 对话被删除时清理其按 conversation_id 累积的运行态痕迹：活跃 generation 集合与
    /// 会话级工具同意标记。两者都严格按 conversation_id 取键，对话删除后再不会被
    /// 引用，是最无歧义的有界清理点（不影响其它活跃对话）。generation 号本身来自进程级
    /// 全局计数器（不分桶），无需在此清理。
    pub fn forget_chat_conversation_runtime(&self, conversation_id: &str) {
        self.chat_active_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id);
        self.chat_session_consent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id);
    }

    /// 尝试占用某个对话的某条 run 回复槽位。同会话允许多条 run 并存（多模型一问多答）；
    /// 仅当同一 (conversation_id, run_id) 已在进行中时返回 false（防同一 run 重复进入）。
    pub fn try_begin_chat_reply(&self, conversation_id: &str, run_id: &str) -> bool {
        let mut active = self
            .chat_active_replies
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let runs = active.entry(conversation_id.to_string()).or_default();
        if runs.contains(run_id) {
            return false;
        }
        runs.insert(run_id.to_string());
        true
    }

    /// 原子地「检查 busy + 占用一个哨兵槽位」：在同一把锁内，若该会话已有任意 run 在跑则返回
    /// false，否则注册 `run_id` 哨兵槽位并返回 true。命令入口用它替代「先 check 后 register」
    /// 的两步，关闭 busy 判定与槽位注册之间的 TOCTOU 窗口（防止同会话并发发送同时通过 busy
    /// 检查）。哨兵只占 `chat_active_replies`，不碰 `chat_active_generations`（不参与取消），
    /// 由命令退出时 `end_chat_reply` 释放。
    pub fn try_reserve_chat_send(&self, conversation_id: &str, run_id: &str) -> bool {
        let mut active = self
            .chat_active_replies
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let runs = active.entry(conversation_id.to_string()).or_default();
        if !runs.is_empty() {
            return false;
        }
        runs.insert(run_id.to_string());
        true
    }

    /// 该会话当前是否有任意一条 run 正在回复（用于「生成中拒绝新发送」的 busy 判定）。
    pub fn conversation_has_active_reply(&self, conversation_id: &str) -> bool {
        self.chat_active_replies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(conversation_id)
            .map(|runs| !runs.is_empty())
            .unwrap_or(false)
    }

    /// 释放某个对话某条 run 的回复槽位（run 集合空了则移除该会话条目）。
    pub fn end_chat_reply(&self, conversation_id: &str, run_id: &str) {
        let mut active = self
            .chat_active_replies
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(runs) = active.get_mut(conversation_id) {
            runs.remove(run_id);
            if runs.is_empty() {
                active.remove(conversation_id);
            }
        }
    }

    pub fn get_cached_chat_tools(
        &self,
        cache_key: &str,
        ttl: Duration,
    ) -> Option<Vec<ChatToolDefinition>> {
        get_cached(&self.chat_tool_list_cache, cache_key, ttl)
    }

    pub fn set_cached_chat_tools(&self, cache_key: String, tools: Vec<ChatToolDefinition>) {
        set_cached(&self.chat_tool_list_cache, cache_key, tools);
    }

    pub fn get_cached_external_slash_commands(
        &self,
        cache_key: &str,
        ttl: Duration,
    ) -> Option<Vec<crate::external_agents::types::ExternalCliSlashCommand>> {
        get_cached(&self.external_slash_commands_cache, cache_key, ttl)
    }

    pub fn set_cached_external_slash_commands(
        &self,
        cache_key: String,
        commands: Vec<crate::external_agents::types::ExternalCliSlashCommand>,
    ) {
        set_cached(&self.external_slash_commands_cache, cache_key, commands);
    }

    pub fn get_cached_external_agent_models(
        &self,
        agent_id: &str,
        ttl: Duration,
    ) -> Option<Vec<crate::external_agents::types::RuntimeModelOption>> {
        get_cached(&self.external_agent_models_cache, agent_id, ttl)
    }

    pub fn set_cached_external_agent_models(
        &self,
        agent_id: String,
        models: Vec<crate::external_agents::types::RuntimeModelOption>,
    ) {
        set_cached(&self.external_agent_models_cache, agent_id, models);
    }

    pub fn get_cached_detected_agents(
        &self,
        ttl: Duration,
    ) -> Option<Vec<crate::external_agents::types::DetectedAgent>> {
        let mut cache = self
            .external_detected_agents_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((created_at, agents)) = cache.as_ref() {
            if created_at.elapsed() <= ttl {
                return Some(agents.clone());
            }
        }
        *cache = None;
        None
    }

    pub fn set_cached_detected_agents(
        &self,
        agents: Vec<crate::external_agents::types::DetectedAgent>,
    ) {
        *self
            .external_detected_agents_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), agents));
    }

    /// Phase 2: return the control channel of a reusable live session for this conversation
    /// (same agent + cwd, actor still alive). Removes a stale/mismatched entry as a side effect.
    pub fn external_live_session_control(
        &self,
        conversation_id: &str,
        agent_id: &str,
        cwd: &str,
    ) -> Option<tokio::sync::mpsc::Sender<crate::external_agents::session::live::SessionCommand>> {
        let mut map = self
            .external_live_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(session) = map.get_mut(conversation_id) {
            if session.is_reusable(agent_id, cwd) {
                session.last_activity = Instant::now();
                return Some(session.control.clone());
            }
        }
        // Dropping the removed entry closes its control channel → the actor shuts the child down.
        map.remove(conversation_id);
        None
    }

    pub fn register_external_live_session(
        &self,
        conversation_id: String,
        session: crate::external_agents::session::live::LiveSession,
    ) {
        const IDLE_TTL: Duration = Duration::from_secs(600);
        const MAX_LIVE_SESSIONS: usize = 6;
        let mut map = self
            .external_live_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Reclaim idle sessions (dropping each entry closes its actor + child) and any whose
        // actor already exited.
        map.retain(|_, s| !s.is_idle(IDLE_TTL));
        // Bound concurrent live processes: evict least-recently-used until under the cap.
        while map.len() >= MAX_LIVE_SESSIONS {
            let Some(oldest) = map
                .iter()
                .min_by_key(|(_, s)| s.last_activity)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            map.remove(&oldest);
        }
        map.insert(conversation_id, session);
    }

    pub fn remove_external_live_session(&self, conversation_id: &str) {
        self.external_live_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id);
    }

    /// Reclaim every idle/dead live session (e.g. from a periodic sweeper). Returns how many
    /// were dropped. Dropping each entry closes its actor + child process.
    pub fn sweep_idle_external_live_sessions(&self, idle_ttl: Duration) -> usize {
        let mut map = self
            .external_live_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let before = map.len();
        map.retain(|_, s| !s.is_idle(idle_ttl));
        before - map.len()
    }

    /// Drop all live sessions (e.g. on app shutdown). Each actor closes its child process.
    pub fn close_all_external_live_sessions(&self) {
        self.external_live_sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    /// Shared handle to the background-command registry. Returned as a cloned
    /// `Arc` so a detached waiter task can update job status after the spawning
    /// stack frame is gone (background commands survive across turns).
    pub fn background_commands_handle(
        &self,
    ) -> Arc<Mutex<HashMap<String, crate::native_tools::BackgroundCommand>>> {
        Arc::clone(&self.background_commands)
    }

    /// Register a tracked background command. Reaps already-terminated entries
    /// opportunistically so the map does not grow unbounded across a long
    /// session, but keeps the most recent terminal jobs so `bash_output` can
    /// still return their final output + exit code right after they finish.
    pub fn register_background_command(
        &self,
        job: crate::native_tools::BackgroundCommand,
    ) {
        const MAX_TRACKED_BACKGROUND_COMMANDS: usize = 64;
        let mut map = self
            .background_commands
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // Evict oldest terminal jobs first if we are over the cap; never evict a
        // still-running job (it owns a live process group).
        while map.len() >= MAX_TRACKED_BACKGROUND_COMMANDS {
            let oldest_terminal = map
                .values()
                .filter(|j| !matches!(j.status, crate::native_tools::BackgroundCommandStatus::Running))
                .min_by_key(|j| j.started_at)
                .map(|j| j.job_id.clone());
            match oldest_terminal {
                Some(id) => {
                    // Remove the evicted job's per-job log too, otherwise long
                    // sessions that churn >64 short-lived background commands
                    // leak one small (now-unreachable) log file per eviction.
                    if let Some(job) = map.remove(&id) {
                        let _ = std::fs::remove_file(&job.log_path);
                    }
                }
                // All remaining jobs are still running; stop evicting.
                None => break,
            }
        }
        map.insert(job.job_id.clone(), job);
    }

    /// Kill all tracked background command process groups and clear the registry
    /// (e.g. on app shutdown). Each running job's process group is SIGKILLed /
    /// taskkill'd; their log files are best-effort removed. Returns how many
    /// process groups were killed.
    pub fn kill_all_background_commands(&self) -> usize {
        let mut map = self
            .background_commands
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut killed = 0;
        for job in map.values_mut() {
            if matches!(
                job.status,
                crate::native_tools::BackgroundCommandStatus::Running
            ) {
                // Prefer signaling the owning waiter task (which still holds the
                // live Child) so the kill targets the live process group and
                // never a reaped/reused pid (TOCTOU). Fall back to a direct
                // process-group kill only when there is no waiter (seeded test
                // jobs).
                match job.kill_tx.take() {
                    Some(kill_tx) => {
                        let _ = kill_tx.send(());
                        killed += 1;
                    }
                    None => {
                        if let Some(pid) = job.pid {
                            crate::native_tools::kill_process_group(pid);
                            killed += 1;
                        }
                    }
                }
            }
            let _ = std::fs::remove_file(&job.log_path);
        }
        map.clear();
        killed
    }

    /// 该 base_url 是否已被学习为"拒绝 prompt_cache_key"。
    pub fn prompt_cache_key_unsupported(&self, base_url: &str) -> bool {
        self.prompt_cache_key_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(base_url)
    }

    /// 记住该 base_url 拒绝 prompt_cache_key（首次 400 后调用）。
    pub fn mark_prompt_cache_key_unsupported(&self, base_url: &str) {
        self.prompt_cache_key_unsupported
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(base_url.to_string());
    }

    /// 标记某个 key 失败：进入冷却 + 不变更 active_key_idx
    pub fn mark_key_failed(&self, provider_id: &str, idx: usize) {
        let mut cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
        cooldowns.insert(
            (provider_id.to_string(), idx),
            Instant::now() + KEY_COOLDOWN,
        );
    }

    /// 标记某个 key 成功：清除该 idx 的冷却 + 设为 active
    pub fn mark_key_ok(&self, provider_id: &str, idx: usize) {
        let mut cooldowns = self.key_cooldowns.lock().unwrap_or_else(|e| e.into_inner());
        cooldowns.remove(&(provider_id.to_string(), idx));
        drop(cooldowns);
        let mut active = self
            .active_key_idx
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        active.insert(provider_id.to_string(), idx);
    }
}

#[cfg(test)]
/// 构造一个最小可用的 AppState 用于单测（cooldown / MCP 连接池等）。
/// 不涉及网络，Client::new() 即可（不会发请求）。供 state / mcp::manager 测试复用。
pub(crate) fn test_app_state() -> AppState {
    AppState::base(
        Settings::default(),
        std::env::temp_dir().join("kivio-test-usage"),
        Client::new(),
        #[cfg(target_os = "macos")]
        MacOcrClient::disabled(),
        RapidOcrClient::disabled(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> AppState {
        test_app_state()
    }

    #[test]
    fn pick_active_key_returns_none_when_total_zero() {
        let st = test_state();
        assert_eq!(st.pick_active_key("p", 0, &HashSet::new()), None);
    }

    #[test]
    fn pick_active_key_starts_at_idx_zero_when_no_active_recorded() {
        let st = test_state();
        assert_eq!(st.pick_active_key("p", 3, &HashSet::new()), Some(0));
    }

    #[test]
    fn pick_active_key_prefers_last_known_good_idx() {
        let st = test_state();
        st.mark_key_ok("p", 2);
        assert_eq!(st.pick_active_key("p", 3, &HashSet::new()), Some(2));
    }

    #[test]
    fn pick_active_key_skips_tried_indices() {
        let st = test_state();
        let mut tried = HashSet::new();
        tried.insert(0);
        // active 是 0（没记录过 ok），但 0 已 tried → 应返回 1（环绕扫描下一个）
        assert_eq!(st.pick_active_key("p", 3, &tried), Some(1));
    }

    #[test]
    fn pick_active_key_skips_cooled_down_indices() {
        let st = test_state();
        st.mark_key_failed("p", 0); // 0 进入冷却
                                    // active 默认 0；0 在冷却 → 应跳到 1
        assert_eq!(st.pick_active_key("p", 3, &HashSet::new()), Some(1));
    }

    #[test]
    fn pick_active_key_falls_back_to_cooled_when_all_cooled_but_untried() {
        let st = test_state();
        // 三个 key 全部冷却
        st.mark_key_failed("p", 0);
        st.mark_key_failed("p", 1);
        st.mark_key_failed("p", 2);
        // 但都没试过 → 兜底返回某个 idx（不是 None），让用户至少有 key 用
        assert!(st.pick_active_key("p", 3, &HashSet::new()).is_some());
    }

    #[test]
    fn pick_active_key_returns_none_when_all_tried() {
        let st = test_state();
        let mut tried = HashSet::new();
        tried.insert(0);
        tried.insert(1);
        tried.insert(2);
        assert_eq!(st.pick_active_key("p", 3, &tried), None);
    }

    #[test]
    fn mark_key_ok_clears_cooldown() {
        let st = test_state();
        st.mark_key_failed("p", 0);
        // 此时 0 在冷却
        assert_ne!(st.pick_active_key("p", 2, &HashSet::new()), Some(0));
        // 标记成功后冷却被清除 + active 设为 0
        st.mark_key_ok("p", 0);
        assert_eq!(st.pick_active_key("p", 2, &HashSet::new()), Some(0));
    }

    #[test]
    fn cooldowns_are_per_provider() {
        let st = test_state();
        st.mark_key_failed("p1", 0);
        // p1 idx 0 冷却不影响 p2 idx 0
        assert_eq!(st.pick_active_key("p2", 2, &HashSet::new()), Some(0));
    }

    #[test]
    fn pick_active_key_handles_active_idx_out_of_bounds() {
        // 用户原来有 5 个 key，active=4；删了 3 个，现在 total=2
        // pick_active_key 应该 clamp 到 total-1，不 panic
        let st = test_state();
        st.mark_key_ok("p", 4);
        let result = st.pick_active_key("p", 2, &HashSet::new());
        assert!(result.is_some());
        assert!(result.unwrap() < 2);
    }

    #[test]
    fn chat_session_consent_is_per_conversation() {
        let st = test_state();
        assert!(!st.has_chat_consent("conv-1"));
        st.grant_chat_consent("conv-1");
        assert!(st.has_chat_consent("conv-1"));
        // Consent is scoped to a single conversation, not global.
        assert!(!st.has_chat_consent("conv-2"));
    }

    // --- 多模型一问多答：并发护栏 per-run 化（任务 06-30 步骤 1） ---

    #[test]
    fn single_run_generation_equivalence() {
        // 单 run（单模型）行为必须与改前等价：分配 → 活跃 → 取消 → 失活。
        let st = test_state();
        let gen = st.next_chat_generation("conv");
        assert!(st.is_chat_generation_active("conv", gen));
        st.cancel_chat_generation("conv");
        assert!(!st.is_chat_generation_active("conv", gen));
    }

    #[test]
    fn single_run_end_generation_retires_only_self() {
        let st = test_state();
        let gen = st.next_chat_generation("conv");
        assert!(st.is_chat_generation_active("conv", gen));
        st.end_chat_generation("conv", gen);
        assert!(!st.is_chat_generation_active("conv", gen));
    }

    #[test]
    fn new_run_does_not_invalidate_sibling_run() {
        // 同会话开第二条 run（多模型并发）不得作废第一条。
        let st = test_state();
        let gen_a = st.next_chat_generation("conv");
        let gen_b = st.next_chat_generation("conv");
        assert_ne!(gen_a, gen_b);
        assert!(st.is_chat_generation_active("conv", gen_a));
        assert!(st.is_chat_generation_active("conv", gen_b));
    }

    #[test]
    fn cancel_kills_all_runs_in_conversation() {
        // R4：cancel 一刀切该会话所有在跑 run。
        let st = test_state();
        let gen_a = st.next_chat_generation("conv");
        let gen_b = st.next_chat_generation("conv");
        let gen_c = st.next_chat_generation("conv");
        st.cancel_chat_generation("conv");
        assert!(!st.is_chat_generation_active("conv", gen_a));
        assert!(!st.is_chat_generation_active("conv", gen_b));
        assert!(!st.is_chat_generation_active("conv", gen_c));
    }

    #[test]
    fn cancel_is_per_conversation() {
        // 取消 conv-1 不影响 conv-2（含 sub-agent 用独立合成 conversation_id 的级联语义）。
        let st = test_state();
        let gen1 = st.next_chat_generation("conv-1");
        let gen2 = st.next_chat_generation("conv-2");
        st.cancel_chat_generation("conv-1");
        assert!(!st.is_chat_generation_active("conv-1", gen1));
        assert!(st.is_chat_generation_active("conv-2", gen2));
    }

    #[test]
    fn end_one_run_keeps_sibling_active() {
        let st = test_state();
        let gen_a = st.next_chat_generation("conv");
        let gen_b = st.next_chat_generation("conv");
        st.end_chat_generation("conv", gen_a);
        assert!(!st.is_chat_generation_active("conv", gen_a));
        assert!(st.is_chat_generation_active("conv", gen_b));
    }

    #[test]
    fn reply_slot_allows_multiple_runs_same_conversation() {
        // 同会话允许多条 run 并存；同一 (conv, run) 重复进入才拒绝。
        let st = test_state();
        assert!(!st.conversation_has_active_reply("conv"));
        assert!(st.try_begin_chat_reply("conv", "run-1"));
        assert!(st.try_begin_chat_reply("conv", "run-2"));
        // 同一 run 重复注册被拒。
        assert!(!st.try_begin_chat_reply("conv", "run-1"));
        assert!(st.conversation_has_active_reply("conv"));
    }

    #[test]
    fn reply_slot_release_is_per_run() {
        let st = test_state();
        st.try_begin_chat_reply("conv", "run-1");
        st.try_begin_chat_reply("conv", "run-2");
        st.end_chat_reply("conv", "run-1");
        // 仍有 run-2 在跑 → 会话仍 busy。
        assert!(st.conversation_has_active_reply("conv"));
        st.end_chat_reply("conv", "run-2");
        // 全部释放 → 会话不再 busy，且可重新注册同名 run。
        assert!(!st.conversation_has_active_reply("conv"));
        assert!(st.try_begin_chat_reply("conv", "run-1"));
    }

    #[test]
    fn forget_conversation_clears_active_generations() {
        let st = test_state();
        let gen = st.next_chat_generation("conv");
        assert!(st.is_chat_generation_active("conv", gen));
        st.forget_chat_conversation_runtime("conv");
        assert!(!st.is_chat_generation_active("conv", gen));
    }

    #[test]
    fn reserve_send_is_atomic_busy_check_and_reserve() {
        // 命令入口哨兵：首个预留成功并占槽；同会话第二个预留（哨兵或真实 run 在跑）被拒。
        let st = test_state();
        assert!(st.try_reserve_chat_send("conv", "send-1"));
        assert!(st.conversation_has_active_reply("conv"));
        // 任意第二个预留在哨兵存活期间被拒（关闭并发发送的 TOCTOU）。
        assert!(!st.try_reserve_chat_send("conv", "send-2"));
        // 哨兵存活期间，真实 per-run 槽位仍可与之共存（fan-out 各臂注册自己的 run）。
        assert!(st.try_begin_chat_reply("conv", "run-arm-1"));
        assert!(st.try_begin_chat_reply("conv", "run-arm-2"));
        // 释放哨兵后仍有 run 在跑 → 仍 busy；新预留仍被拒。
        st.end_chat_reply("conv", "send-1");
        assert!(st.conversation_has_active_reply("conv"));
        assert!(!st.try_reserve_chat_send("conv", "send-3"));
        // 全部 run 释放后才能再次预留。
        st.end_chat_reply("conv", "run-arm-1");
        st.end_chat_reply("conv", "run-arm-2");
        assert!(!st.conversation_has_active_reply("conv"));
        assert!(st.try_reserve_chat_send("conv", "send-4"));
    }
}
