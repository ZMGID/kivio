use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU64},
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
pub struct PendingChatExternalSend {
    pub id: String,
    pub content: String,
    pub attachments: Vec<PendingChatExternalAttachment>,
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
    /// Chat 流式取消代号，按 conversation_id 隔离，避免 Lens 与 Chat 互相取消。
    pub chat_stream_generations: Mutex<HashMap<String, u64>>,
    /// 正在进行 assistant 回复生成的 conversation_id 集合，防止同对话并发写盘。
    pub chat_active_replies: Mutex<HashSet<String>>,
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
    /// 外部入口（例如 Lens）交给 Chat 前端发送的待处理消息。
    /// 后端只负责保存请求和打开窗口，实际发送必须走 Chat 前端的手动发送状态机。
    pub pending_chat_external_sends: Mutex<Vec<PendingChatExternalSend>>,
    /// Lens 启动前抓到的选中文本：放在这里等前端 enterSelect 来取走。
    /// 取一次清一次（take 语义）。无选中 / 取过 / translate 模式 = None。
    pub pending_selection: Mutex<Option<String>>,
    /// Windows 冻结帧选择模式的临时截图 id。仅在进入 select 态前预抓屏幕时使用。
    pub lens_freeze_frame_image_id: Mutex<Option<String>>,
    /// API Key 多 key failover 状态：(provider_id, key_idx) → 冷却到期时间。
    /// 某个 key 触发 quota/rate-limit/auth 失败时进入冷却，KEY_COOLDOWN 秒内不再选用。
    pub key_cooldowns: Mutex<HashMap<(String, usize), Instant>>,
    /// 每个 provider 当前活跃 key idx：上一次成功的 key 优先继续用。
    pub active_key_idx: Mutex<HashMap<String, usize>>,
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
}

/// 单个 key 触发 failover 后的冷却时长。
pub const KEY_COOLDOWN: Duration = Duration::from_secs(60);

impl AppState {
    /// Build a headless `AppState` for the `kivio-code` terminal agent — no
    /// `AppHandle`, no Tauri runtime. Every field mirrors the live construction
    /// in `lib.rs::run` (the `app.manage(AppState { .. })` block) except the two
    /// OCR clients use their `headless()` constructors and `usage_dir` is passed
    /// in. The agent loop only touches `settings`, the chat-generation maps,
    /// session-consent set, `http`, and `usage_dir`; the rest are inert defaults
    /// kept for struct completeness.
    pub fn new_headless(settings: Settings, usage_dir: PathBuf) -> Self {
        AppState {
            settings: RwLock::new(settings),
            explain_images: Mutex::new(HashMap::new()),
            current_explain_image_id: Mutex::new(None),
            lens_busy: AtomicBool::new(false),
            prev_frontmost_pid_lens: AtomicI32::new(0),
            prev_frontmost_pid_main: AtomicI32::new(0),
            explain_stream_generation: AtomicU64::new(0),
            chat_stream_generations: Mutex::new(HashMap::new()),
            chat_active_replies: Mutex::new(HashSet::new()),
            pending_chat_tool_approvals: Mutex::new(HashMap::new()),
            chat_session_consent: Mutex::new(HashSet::new()),
            pending_chat_session_consents: Mutex::new(HashMap::new()),
            chat_consent_prompt_lock: tokio::sync::Mutex::new(()),
            pending_chat_user_prompts: Mutex::new(HashMap::new()),
            pending_python_runs: Mutex::new(HashMap::new()),
            chat_create_conversation_lock: Mutex::new(()),
            chat_tool_list_cache: Mutex::new(HashMap::new()),
            pending_chat_external_sends: Mutex::new(Vec::new()),
            pending_selection: Mutex::new(None),
            lens_freeze_frame_image_id: Mutex::new(None),
            key_cooldowns: Mutex::new(HashMap::new()),
            active_key_idx: Mutex::new(HashMap::new()),
            mcp_sessions: tokio::sync::Mutex::new(HashMap::new()),
            usage_dir,
            http: crate::api::build_http_client(),
            #[cfg(target_os = "macos")]
            macos_ocr: MacOcrClient::headless(),
            rapidocr: RapidOcrClient::headless(crate::api::build_http_client()),
            sub_agents: crate::chat::sub_agent::SubAgentManager::default(),
        }
    }

    /// 安全读取设置（锁中毒时返回内部数据，不 panic）
    pub fn settings_read(&self) -> std::sync::RwLockReadGuard<'_, Settings> {
        self.settings.read().unwrap_or_else(|e| e.into_inner())
    }
    /// 安全写入设置（锁中毒时返回内部数据，不 panic）
    pub fn settings_write(&self) -> std::sync::RwLockWriteGuard<'_, Settings> {
        self.settings.write().unwrap_or_else(|e| e.into_inner())
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
    pub fn next_chat_generation(&self, conversation_id: &str) -> u64 {
        let mut generations = self
            .chat_stream_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let next = generations
            .get(conversation_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        generations.insert(conversation_id.to_string(), next);
        next
    }

    /// 取消指定 conversation 的当前 Chat 运行。
    pub fn cancel_chat_generation(&self, conversation_id: &str) {
        let mut generations = self
            .chat_stream_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let next = generations
            .get(conversation_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        generations.insert(conversation_id.to_string(), next);
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

    /// 判断指定 conversation 的 Chat 运行是否仍然有效。
    pub fn is_chat_generation_active(&self, conversation_id: &str, generation: u64) -> bool {
        self.chat_stream_generations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(conversation_id)
            .copied()
            .unwrap_or(0)
            == generation
    }

    /// 尝试占用某个对话的回复生成槽位；同对话已有进行中的回复时返回 false。
    pub fn try_begin_chat_reply(&self, conversation_id: &str) -> bool {
        let mut active = self
            .chat_active_replies
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if active.contains(conversation_id) {
            return false;
        }
        active.insert(conversation_id.to_string());
        true
    }

    /// 释放某个对话的回复生成槽位。
    pub fn end_chat_reply(&self, conversation_id: &str) {
        self.chat_active_replies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(conversation_id);
    }

    pub fn get_cached_chat_tools(
        &self,
        cache_key: &str,
        ttl: Duration,
    ) -> Option<Vec<ChatToolDefinition>> {
        let mut cache = self
            .chat_tool_list_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some((created_at, tools)) = cache.get(cache_key) {
            if created_at.elapsed() <= ttl {
                return Some(tools.clone());
            }
        }
        cache.remove(cache_key);
        None
    }

    pub fn set_cached_chat_tools(&self, cache_key: String, tools: Vec<ChatToolDefinition>) {
        self.chat_tool_list_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(cache_key, (Instant::now(), tools));
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
    AppState {
        settings: RwLock::new(Settings::default()),
        explain_images: Mutex::new(HashMap::new()),
        current_explain_image_id: Mutex::new(None),
        lens_busy: AtomicBool::new(false),
        prev_frontmost_pid_lens: AtomicI32::new(0),
        prev_frontmost_pid_main: AtomicI32::new(0),
        explain_stream_generation: AtomicU64::new(0),
        chat_stream_generations: Mutex::new(HashMap::new()),
        chat_active_replies: Mutex::new(HashSet::new()),
        pending_chat_tool_approvals: Mutex::new(HashMap::new()),
        chat_session_consent: Mutex::new(HashSet::new()),
        pending_chat_session_consents: Mutex::new(HashMap::new()),
        chat_consent_prompt_lock: tokio::sync::Mutex::new(()),
        pending_chat_user_prompts: Mutex::new(HashMap::new()),
        pending_python_runs: Mutex::new(HashMap::new()),
        chat_create_conversation_lock: Mutex::new(()),
        chat_tool_list_cache: Mutex::new(HashMap::new()),
        pending_chat_external_sends: Mutex::new(Vec::new()),
        pending_selection: Mutex::new(None),
        lens_freeze_frame_image_id: Mutex::new(None),
        key_cooldowns: Mutex::new(HashMap::new()),
        active_key_idx: Mutex::new(HashMap::new()),
        mcp_sessions: tokio::sync::Mutex::new(HashMap::new()),
        usage_dir: std::env::temp_dir().join("kivio-test-usage"),
        http: Client::new(),
        #[cfg(target_os = "macos")]
        macos_ocr: MacOcrClient::disabled(),
        rapidocr: RapidOcrClient::disabled(),
        sub_agents: crate::chat::sub_agent::SubAgentManager::default(),
    }
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
}
