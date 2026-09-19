use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use reqwest::Client;

#[cfg(target_os = "macos")]
use crate::macos_ocr::MacOcrClient;
use crate::mcp::types::McpTool;
use crate::offline_models::OfflineModelManager;
use crate::rapidocr::RapidOcrClient;
use crate::settings::{Settings, SettingsError, SettingsSnapshot, SettingsVersion};

/// 应用全局状态
/// 使用 RwLock 保护 settings，允许多读单写；
/// Lens 的图片身份、流 generation 与会话状态由 `lens::LensRuntimeState` 持有；
/// 组合根只保存领域句柄。
pub struct AppState {
    settings: RwLock<Settings>,
    /// Per-process namespace for settings revisions. A renderer carrying a snapshot from a
    /// previous backend process must never match a new process that happens to reuse revision 0.
    settings_epoch: String,
    /// Monotonic generation within `settings_epoch`. Full settings saves carry the version from
    /// the client's original read, validate it before runtime work, and compare it again after
    /// async workspace migration so no newer write can be overwritten by a stale full object.
    settings_revision: AtomicU64,
    /// Serializes full async saves (including workspace migration) without blocking lightweight
    /// writers. Lightweight writers advance `settings_revision`, so the full save still detects
    /// them at its final CAS commit.
    settings_persistence: crate::settings::SettingsPersistenceGate,
    lens_runtime: crate::lens::LensRuntimeState,
    /// macOS：打开浮窗前记下的前台 App PID（0 = 无 / 前台就是 Kivio 自己），关闭浮窗时据此把
    /// 前台交还给原来的 App，避免 Kivio 变成"前台却无窗口"而触发 RunEvent::Reopen 误开 Chat。
    /// lens（含截图/选词翻译）与输入翻译是各自独立、可同时存在的浮窗，各占一个槽，避免相互覆盖。
    /// 详见 spec/backend/window-lifecycle.md。
    #[cfg(target_os = "macos")]
    frontmost_apps: crate::window_focus::FrontmostAppState,
    /// Chat 运行、输入信箱与创建协调；索引由 chat 领域私有持有。
    chat_runtime: crate::chat::runtime_state::ChatRuntimeState,
    /// Sequenced replay hub and live IPC subscribers share one protocol owner.
    chat_protocol: crate::chat::protocol::ChatProtocolState,
    /// Chat 审批、会话授权与 ask-user 的瞬态状态。领域句柄私有持有所有 map/lock，
    /// 组合根只负责生命周期，调用方只能走原子行为方法。
    chat_interactions: crate::chat::interaction_state::ChatInteractionState,
    external_discovery: crate::external_agents::discovery_state::ExternalDiscoveryState,
    /// Phase 2 持久会话注册表：conversation_id → 活会话（仅持有控制通道，不持有 Child）。
    /// 仅在 get/insert/remove 时短暂持锁，绝不跨 turn await 持锁。
    external_live_sessions: crate::external_agents::session::live::LiveSessionRegistry,
    /// 外部入口（例如 Lens）交给 Chat 前端发送的待处理消息。
    /// 后端只负责保存请求和打开窗口，实际发送必须走 Chat 前端的手动发送状态机。
    pending_chat_external_sends: crate::chat::external_send::ChatExternalSendMailbox,
    provider_runtime: crate::chat::provider_runtime::ProviderRuntimeState,
    /// MCP 会话池与持久化工具 schema 快照的领域句柄；内部索引只由 mcp 模块拥有。
    mcp_runtime: crate::mcp::McpRuntimeState,
    /// Token usage ledger directory under app data. Model providers can append records
    /// without needing an AppHandle threaded through every call path.
    pub usage_dir: PathBuf,
    pub http: Client,
    /// 直连客户端（忽略系统/环境代理）。只有当某个供应商关掉「跟随系统代理」时才构造，
    /// 默认全跟随系统代理的用户不会多出一个连接池。
    http_direct: std::sync::OnceLock<Client>,
    /// macOS Apple Vision OCR sidecar 客户端。只有系统 OCR 路径会拉起。
    #[cfg(target_os = "macos")]
    pub macos_ocr: std::sync::Arc<MacOcrClient>,
    /// RapidOCR 与替换翻译共用的离线模型清单、下载器和 ONNX Runtime 生命周期。
    pub offline_models: std::sync::Arc<OfflineModelManager>,
    /// RapidOCR 离线 OCR 客户端。模型 + onnxruntime dylib 都由用户在设置页面下载到 app data 目录,
    /// 安装包不带任何 ONNX Runtime 二进制。`status()` 检查 4 个文件齐不齐, 不齐让前端引导下载。
    pub rapidocr: std::sync::Arc<RapidOcrClient>,
    /// 多 agent / 子 agent 任务表（P3）：spawn 的子 agent 状态、按名寻址、并发上限。
    pub sub_agents: crate::chat::sub_agent::SubAgentManager,
    /// 后台 run_command 进程注册表：job_id → 跟踪中的后台命令。
    /// 与后台 subagent 不同：这些命令**跨 turn 存活**，只由显式 `kill_background`
    /// 或 app 退出 sweep 清理（对齐 Claude Code background bash，dev-server 友好），
    /// 不随发起的 run 取消。仅在 insert/lookup/sweep 时短暂持锁。
    background_commands: Arc<crate::native_tools::background_registry::BackgroundCommandRegistry>,
    /// 外部 CLI 自报的后台任务注册表（目前只有 claude：后台 Bash / 后台子代理，
    /// task_id → 条目）。由 `run.rs` 消费 `UnifiedAgentEvent::BackgroundTask` 时 upsert，
    /// Background tasks 面板轮询读取。仅内存：任务活在 CLI 进程里，Kivio 重启即失效。
    external_background_tasks: crate::external_agents::background_tasks::ExternalBackgroundTasks,
    /// 开发者「请求调试」内存环形缓冲：最近 [`REQUEST_DEBUG_CAPACITY`] 条 provider 调用的
    /// 请求（脱敏 headers + body）+ 响应摘要。默认关闭（`chat_tools.request_debug_enabled`），
    /// 关闭时 adapter 短路、不构造记录。领域 owner 管理环形缓冲及既有磁盘镜像。
    request_debug: crate::chat::request_debug::RequestDebugState,
    /// 自动化运行生命周期的领域句柄；内部索引与转换只由 automation 模块拥有。
    pub(crate) automation_runs: crate::automation::AutomationRunState,
}

impl AppState {
    pub(crate) async fn begin_settings_save(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.settings_persistence.begin_full_save().await
    }
    #[cfg(target_os = "macos")]
    pub(crate) fn frontmost_apps(&self) -> &crate::window_focus::FrontmostAppState {
        &self.frontmost_apps
    }
    pub(crate) fn external_background_tasks(
        &self,
    ) -> &crate::external_agents::background_tasks::ExternalBackgroundTasks {
        &self.external_background_tasks
    }
    pub(crate) fn request_debug(&self) -> &crate::chat::request_debug::RequestDebugState {
        &self.request_debug
    }
    /// External entrypoint -> Chat renderer handoff. Request ownership remains
    /// in the mailbox until the renderer acknowledges it or startup fails.
    pub(crate) fn enqueue_chat_external_send(
        &self,
        request: crate::chat::external_send::PendingChatExternalSend,
    ) {
        self.pending_chat_external_sends.enqueue(request);
    }

    pub(crate) fn rollback_chat_external_send(&self, request_id: &str) -> bool {
        self.pending_chat_external_sends.rollback(request_id)
    }

    pub(crate) fn chat_external_send_mailbox(
        &self,
    ) -> &crate::chat::external_send::ChatExternalSendMailbox {
        &self.pending_chat_external_sends
    }

    #[cfg(debug_assertions)]
    pub(crate) fn external_live_session_diagnostic(
        &self,
        conversation_id: &str,
    ) -> crate::external_agents::session::live::LiveSessionDiagnostic {
        self.external_live_sessions.diagnostic(conversation_id)
    }

    pub(crate) fn provider_runtime(&self) -> &crate::chat::provider_runtime::ProviderRuntimeState {
        &self.provider_runtime
    }

    pub(crate) fn external_discovery(
        &self,
    ) -> &crate::external_agents::discovery_state::ExternalDiscoveryState {
        &self.external_discovery
    }

    pub(crate) fn chat_runtime(&self) -> &crate::chat::runtime_state::ChatRuntimeState {
        &self.chat_runtime
    }

    pub(crate) fn chat_protocol(&self) -> &crate::chat::protocol::ChatProtocolState {
        &self.chat_protocol
    }

    pub(crate) fn chat_interactions(
        &self,
    ) -> &crate::chat::interaction_state::ChatInteractionState {
        &self.chat_interactions
    }

    pub(crate) fn external_live_sessions(
        &self,
    ) -> &crate::external_agents::session::live::LiveSessionRegistry {
        &self.external_live_sessions
    }

    /// 集中构造点：`lib.rs::run` 的 `app.manage`、`new_headless`、以及测试用 `test_app_state`
    /// 三处唯一的差异只有 `settings` / `usage_dir` / `http` 与两个 OCR 客户端；其余字段全是
    /// 同样的空默认值。这里统一构造，三处只提供差异字段，避免同一份 ~40 行字面量重复三次。
    pub(crate) fn base(
        settings: Settings,
        usage_dir: PathBuf,
        http: Client,
        #[cfg(target_os = "macos")] macos_ocr: std::sync::Arc<MacOcrClient>,
        offline_models: std::sync::Arc<OfflineModelManager>,
        rapidocr: std::sync::Arc<RapidOcrClient>,
    ) -> Self {
        let mcp_runtime = crate::mcp::McpRuntimeState::load(&usage_dir);
        let request_debug = crate::chat::request_debug::RequestDebugState::new(&usage_dir);
        let provider_runtime = crate::chat::provider_runtime::ProviderRuntimeState::new(&settings);
        AppState {
            settings: RwLock::new(settings),
            settings_epoch: uuid::Uuid::new_v4().to_string(),
            settings_revision: AtomicU64::new(0),
            settings_persistence: crate::settings::SettingsPersistenceGate::default(),
            lens_runtime: crate::lens::LensRuntimeState::default(),
            #[cfg(target_os = "macos")]
            frontmost_apps: crate::window_focus::FrontmostAppState::default(),
            chat_runtime: crate::chat::runtime_state::ChatRuntimeState::default(),
            chat_protocol: crate::chat::protocol::ChatProtocolState::default(),
            chat_interactions: crate::chat::interaction_state::ChatInteractionState::default(),
            external_discovery:
                crate::external_agents::discovery_state::ExternalDiscoveryState::default(),
            external_live_sessions:
                crate::external_agents::session::live::LiveSessionRegistry::default(),
            pending_chat_external_sends:
                crate::chat::external_send::ChatExternalSendMailbox::default(),
            provider_runtime,
            mcp_runtime,
            usage_dir,
            http,
            http_direct: std::sync::OnceLock::new(),
            #[cfg(target_os = "macos")]
            macos_ocr,
            offline_models,
            rapidocr,
            sub_agents: crate::chat::sub_agent::SubAgentManager::default(),
            background_commands: Arc::new(
                crate::native_tools::background_registry::BackgroundCommandRegistry::default(),
            ),
            external_background_tasks:
                crate::external_agents::background_tasks::ExternalBackgroundTasks::default(),
            request_debug,
            automation_runs: crate::automation::AutomationRunState::default(),
        }
    }

    /// Build a headless `AppState` for the `kivio-code` terminal agent — no
    /// `AppHandle`, no Tauri runtime. Differs from the live construction in
    /// `lib.rs::run` only in the two OCR clients (`headless()` constructors) and
    /// `usage_dir` (passed in). The agent loop only touches `settings`, the
    /// chat-generation state, session-consent set, `http`, and `usage_dir`; the
    /// rest are inert defaults kept for struct completeness.
    pub fn new_headless(settings: Settings, usage_dir: PathBuf) -> Self {
        let offline_models = OfflineModelManager::headless(crate::api::build_http_client());
        Self::base(
            settings,
            usage_dir,
            crate::api::build_http_client(),
            #[cfg(target_os = "macos")]
            MacOcrClient::headless(),
            offline_models.clone(),
            RapidOcrClient::headless(offline_models),
        )
    }
    /// 该供应商应当使用的 HTTP 客户端。默认跟随系统代理（与加这个开关之前一致），
    /// 关掉时用忽略代理的直连客户端。
    pub fn client_for(&self, provider: &crate::settings::ModelProvider) -> &Client {
        if provider.request.oauth.is_some() {
            return crate::provider_oauth::inference_client(provider.request.use_system_proxy);
        }
        if provider.request.use_system_proxy {
            &self.http
        } else {
            self.http_direct
                .get_or_init(crate::api::build_direct_http_client)
        }
    }

    /// 安全读取设置（锁中毒时返回内部数据，不 panic）
    pub fn settings_read(&self) -> std::sync::RwLockReadGuard<'_, Settings> {
        self.settings.read().unwrap_or_else(|e| e.into_inner())
    }
    /// Non-blocking read for optional work such as desktop notifications.
    pub(crate) fn try_settings_read(
        &self,
    ) -> Result<
        std::sync::RwLockReadGuard<'_, Settings>,
        std::sync::TryLockError<std::sync::RwLockReadGuard<'_, Settings>>,
    > {
        self.settings.try_read()
    }
    /// The only production write primitive for the in-memory settings value. It keeps the
    /// revision check, durable action and publication under one serialization lock, so sibling
    /// modules cannot obtain a raw write guard and silently bypass the settings transaction.
    pub(crate) fn publish_settings_transaction(
        &self,
        expected_version: Option<SettingsVersion>,
        build_canonical: impl FnOnce(&Settings) -> Result<Settings, String>,
        persist: impl FnOnce(&Settings) -> Result<(), String>,
    ) -> Result<SettingsSnapshot, SettingsError> {
        let mut current = self.settings.write().unwrap_or_else(|e| e.into_inner());
        let actual_revision = self.settings_revision();
        let actual_version = self.settings_version_at(actual_revision);
        if let Some(expected_version) = expected_version {
            if actual_version != expected_version {
                return Err(SettingsError::version_conflict(
                    expected_version,
                    actual_version,
                ));
            }
        }

        let canonical = build_canonical(&current).map_err(SettingsError::from)?;
        persist(&canonical).map_err(SettingsError::from)?;
        *current = canonical.clone();
        let revision = self.advance_settings_revision();
        Ok(SettingsSnapshot {
            settings: canonical,
            version: self.settings_version_at(revision),
        })
    }
    #[cfg(test)]
    pub(crate) fn update_settings_for_test(&self, update: impl FnOnce(&mut Settings)) {
        let mut settings = self.settings.write().unwrap_or_else(|e| e.into_inner());
        update(&mut settings);
        self.advance_settings_revision();
    }
    pub(crate) fn settings_revision(&self) -> u64 {
        self.settings_revision.load(Ordering::Acquire)
    }
    pub(crate) fn advance_settings_revision(&self) -> u64 {
        self.settings_revision.fetch_add(1, Ordering::Release) + 1
    }
    pub(crate) fn settings_version_at(&self, revision: u64) -> SettingsVersion {
        SettingsVersion {
            epoch: self.settings_epoch.clone(),
            revision,
        }
    }
    /// 开发者「请求调试」开关。关时 adapter 短路，不构造任何记录（零开销）。
    pub fn request_debug_enabled(&self) -> bool {
        self.settings_read().chat_tools.request_debug_enabled
    }
    /// Lens 领域句柄。字段保持私有；调用方只能通过领域操作推进状态。
    pub(crate) fn lens(&self) -> &crate::lens::LensRuntimeState {
        &self.lens_runtime
    }

    /// 取消指定 conversation 的**所有**当前 Chat 运行，并级联停止其子 agent。
    /// 跨域编排留在组合根：generation 归 Chat runtime，子 agent 表归 SubAgentManager。
    pub fn cancel_chat_generation(&self, conversation_id: &str) {
        self.sub_agents.stop_conversation(conversation_id);
        self.chat_runtime.cancel_conversation(conversation_id);
    }

    /// 对话被删除时清理其按 conversation_id 累积的运行态痕迹：活跃 generation 集合、
    /// 会话级工具同意标记、按工具名的「总是允许」集合。
    pub fn forget_chat_conversation_runtime(&self, conversation_id: &str) {
        self.chat_runtime.forget_conversation(conversation_id);
        self.chat_interactions.forget_conversation(conversation_id);
    }

    fn mcp_manager(&self) -> crate::mcp::McpManager<'_> {
        let config = {
            let settings = self.settings_read();
            crate::mcp::McpManagerConfig::from_settings(&settings)
        };
        crate::mcp::McpManager::new(&self.mcp_runtime, &self.http, config, self)
    }

    pub fn mcp_idle_timeout(&self) -> Duration {
        self.mcp_manager().mcp_idle_timeout()
    }

    pub async fn mcp_get_or_connect(
        &self,
        sink: Option<&tauri::AppHandle>,
        server: &crate::settings::ChatMcpServer,
    ) -> Result<Arc<tokio::sync::Mutex<crate::mcp::manager::McpSession>>, String> {
        self.mcp_manager().mcp_get_or_connect(sink, server).await
    }

    pub async fn mcp_call_tool(
        &self,
        sink: Option<&tauri::AppHandle>,
        server: &crate::settings::ChatMcpServer,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::mcp::types::McpToolCallResult, String> {
        self.mcp_manager()
            .mcp_call_tool(sink, server, name, arguments)
            .await
    }

    pub async fn mcp_list_tools(
        &self,
        sink: Option<&tauri::AppHandle>,
        server: &crate::settings::ChatMcpServer,
    ) -> Result<Vec<McpTool>, String> {
        self.mcp_manager().mcp_list_tools(sink, server).await
    }

    pub async fn mcp_cached_tools(
        &self,
        server: &crate::settings::ChatMcpServer,
    ) -> Option<Vec<McpTool>> {
        self.mcp_manager().mcp_cached_tools(server).await
    }

    pub async fn mcp_display_tools(
        &self,
        server: &crate::settings::ChatMcpServer,
    ) -> (Vec<McpTool>, bool) {
        self.mcp_manager().mcp_display_tools(server).await
    }

    pub async fn mcp_unreachable_server_ids(&self) -> Vec<String> {
        self.mcp_manager().mcp_unreachable_server_ids().await
    }

    pub async fn mcp_server_state(
        &self,
        server_id: &str,
    ) -> crate::mcp::manager::McpServerStatusSnapshot {
        self.mcp_manager().mcp_server_state(server_id).await
    }

    pub async fn mcp_reload_server(&self, sink: Option<&tauri::AppHandle>, server_id: &str) {
        self.mcp_manager().mcp_reload_server(sink, server_id).await;
    }

    pub async fn mcp_reap_idle(
        &self,
        idle_timeout: Duration,
    ) -> Vec<(
        String,
        Arc<tokio::sync::Mutex<crate::mcp::manager::McpSession>>,
    )> {
        self.mcp_manager().mcp_reap_idle(idle_timeout).await
    }

    pub async fn mcp_keepalive_http(&self, sink: Option<&tauri::AppHandle>) {
        self.mcp_manager().mcp_keepalive_http(sink).await;
    }

    pub async fn mcp_disconnect_all(&self) {
        self.mcp_manager().mcp_disconnect_all().await;
    }

    pub async fn mcp_disconnect_server(&self, server_id: &str) {
        self.mcp_manager().mcp_disconnect_server(server_id).await;
    }

    pub fn kill_mcp_children_now(&self) -> usize {
        self.mcp_manager().kill_mcp_children_now()
    }

    #[cfg(test)]
    pub(crate) async fn mcp_test_insert_session(
        &self,
        server_id: &str,
        session: Arc<tokio::sync::Mutex<crate::mcp::manager::McpSession>>,
    ) {
        self.mcp_runtime
            .get_or_insert_session(server_id, || session)
            .await;
    }

    #[cfg(test)]
    pub(crate) async fn mcp_test_session(
        &self,
        server_id: &str,
    ) -> Option<Arc<tokio::sync::Mutex<crate::mcp::manager::McpSession>>> {
        self.mcp_runtime.session(server_id).await
    }

    #[cfg(test)]
    pub(crate) async fn mcp_test_sessions_empty(&self) -> bool {
        self.mcp_runtime.sessions_empty().await
    }

    #[cfg(test)]
    pub(crate) async fn mcp_test_session_count(&self) -> usize {
        self.mcp_runtime.session_count().await
    }

    #[cfg(test)]
    pub(crate) async fn mcp_test_has_session(&self, server_id: &str) -> bool {
        self.mcp_runtime.has_session(server_id).await
    }

    pub fn get_mcp_tool_snapshot(
        &self,
        server_id: &str,
        config_fingerprint: &str,
    ) -> Option<Vec<McpTool>> {
        self.mcp_runtime
            .tool_snapshot(server_id, config_fingerprint)
    }

    pub fn set_mcp_tool_snapshot(
        &self,
        server_id: String,
        config_fingerprint: String,
        tools: Vec<McpTool>,
    ) {
        self.mcp_runtime
            .set_tool_snapshot(server_id, config_fingerprint, tools);
    }

    /// Shared handle to the background-command registry. Returned as a cloned
    /// `Arc` so a detached waiter task can update job status after the spawning
    /// stack frame is gone (background commands survive across turns).
    pub(crate) fn background_commands_handle(
        &self,
    ) -> Arc<crate::native_tools::background_registry::BackgroundCommandRegistry> {
        Arc::clone(&self.background_commands)
    }
}

impl crate::mcp::McpSettingsPersistence for AppState {
    fn store_refreshed_server(
        &self,
        sink: Option<&tauri::AppHandle>,
        server: &crate::settings::ChatMcpServer,
    ) {
        let Some(app) = sink else {
            if let Err(error) = crate::settings::update_settings_in_memory(self, |settings| {
                crate::mcp::manager::apply_refreshed_auth_to_settings(settings, server);
                Ok(())
            }) {
                eprintln!("Failed to update refreshed OAuth token in memory: {error}");
            }
            return;
        };

        let mut probe = self.settings_read().clone();
        if !crate::mcp::manager::apply_refreshed_auth_to_settings(&mut probe, server) {
            return;
        }
        if let Err(error) = crate::settings::update_settings(app, self, |settings| {
            crate::mcp::manager::apply_refreshed_auth_to_settings(settings, server);
            Ok(())
        }) {
            eprintln!("Failed to persist refreshed OAuth token: {error}");
        }
    }
}

#[cfg(test)]
/// 构造一个最小可用的 AppState 用于单测（cooldown / MCP 连接池等）。
/// 不涉及网络，Client::new() 即可（不会发请求）。供 state / mcp::manager 测试复用。
pub(crate) fn test_app_state() -> AppState {
    let offline_models = OfflineModelManager::headless(Client::new());
    AppState::base(
        Settings::default(),
        std::env::temp_dir().join(format!("kivio-test-usage-{}", uuid::Uuid::new_v4())),
        Client::new(),
        #[cfg(target_os = "macos")]
        MacOcrClient::disabled(),
        offline_models.clone(),
        RapidOcrClient::new(offline_models),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn test_state() -> AppState {
        test_app_state()
    }

    #[test]
    fn pick_active_key_returns_none_when_total_zero() {
        let st = test_state();
        assert_eq!(
            st.provider_runtime()
                .pick_active_key("p", 0, &HashSet::new()),
            None
        );
    }

    #[test]
    fn external_agent_detection_cache_is_scoped_by_cwd() {
        let st = test_state();
        st.external_discovery()
            .set_cached_detected_agents("/project-a".to_string(), Vec::new());

        assert!(st
            .external_discovery()
            .get_cached_detected_agents("/project-a", Duration::from_secs(60))
            .is_some());
        assert!(st
            .external_discovery()
            .get_cached_detected_agents("/project-b", Duration::from_secs(60))
            .is_none());
    }

    #[tokio::test]
    async fn model_probe_lock_is_shared_per_key_and_distinct_across_keys() {
        let st = test_state();
        let a = st
            .external_discovery()
            .acquire_model_probe("claude:/proj")
            .await;
        let _b = tokio::time::timeout(
            Duration::from_secs(1),
            st.external_discovery().acquire_model_probe("codex:/proj"),
        )
        .await
        .unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(1),
            st.external_discovery().acquire_model_probe("claude:/proj")
        )
        .await
        .is_err());
        drop(a);
        let _retry = tokio::time::timeout(
            Duration::from_secs(1),
            st.external_discovery().acquire_model_probe("claude:/proj"),
        )
        .await
        .unwrap();
    }

    #[test]
    fn external_agent_models_cache_applies_source_aware_ttl() {
        use crate::external_agents::types::{CachedAgentModels, ModelSource, RuntimeModelOption};
        let st = test_state();
        let one = |id: &str| RuntimeModelOption {
            id: id.to_string(),
            label: id.to_string(),
            context_window_tokens: None,
        };

        // probed 条目在长 TTL 内命中，短 fallback TTL 不影响它。
        st.external_discovery().set_cached_external_agent_models(
            "claude:/p".to_string(),
            CachedAgentModels {
                models: vec![one("gpt-5")],
                source: ModelSource::Probed,
                reasoning_options: vec![],
                reasoning_by_model: Default::default(),
                current_model: None,
                current_reasoning: None,
            },
        );
        assert!(st
            .external_discovery()
            .get_cached_external_agent_models("claude:/p", Duration::from_secs(300), Duration::ZERO)
            .is_some());

        // fallback 条目按短 TTL 裁定：TTL=0 立即视为过期（负缓存到点即重探）。
        st.external_discovery().set_cached_external_agent_models(
            "codex:/p".to_string(),
            CachedAgentModels {
                models: vec![one("default")],
                source: ModelSource::Fallback,
                reasoning_options: vec![],
                reasoning_by_model: Default::default(),
                current_model: None,
                current_reasoning: None,
            },
        );
        assert!(st
            .external_discovery()
            .get_cached_external_agent_models("codex:/p", Duration::from_secs(300), Duration::ZERO)
            .is_none());
        // 同一 fallback 条目在足够长的 fallback TTL 内仍命中。
        st.external_discovery().set_cached_external_agent_models(
            "codex:/p".to_string(),
            CachedAgentModels {
                models: vec![one("default")],
                source: ModelSource::Fallback,
                reasoning_options: vec![],
                reasoning_by_model: Default::default(),
                current_model: None,
                current_reasoning: None,
            },
        );
        let hit = st.external_discovery().get_cached_external_agent_models(
            "codex:/p",
            Duration::from_secs(300),
            Duration::from_secs(30),
        );
        assert!(matches!(hit.map(|c| c.source), Some(ModelSource::Fallback)));
    }

    #[test]
    fn external_slash_commands_cache_negative_caches_empty_with_short_ttl() {
        use crate::external_agents::types::ExternalCliSlashCommand;
        let st = test_state();
        let cmd = |name: &str| ExternalCliSlashCommand {
            slash: format!("/{name}"),
            name: name.to_string(),
            description: None,
            argument_hint: None,
        };

        // 非空命令列表走长 TTL：短(empty) TTL=0 不影响它，仍命中。
        st.external_discovery()
            .set_cached_external_slash_commands("kimi:/g".to_string(), vec![cmd("compact")]);
        assert!(st
            .external_discovery()
            .get_cached_external_slash_commands("kimi:/g", Duration::from_secs(300), Duration::ZERO)
            .is_some());

        // 空列表（负缓存）按短 TTL 裁定：empty TTL=0 立即过期 → 到点重探。
        st.external_discovery()
            .set_cached_external_slash_commands("grok:/g".to_string(), Vec::new());
        assert!(st
            .external_discovery()
            .get_cached_external_slash_commands("grok:/g", Duration::from_secs(300), Duration::ZERO)
            .is_none());
        // 但空列表在足够长的 empty TTL 内仍命中（TTL 内不重探）。
        let hit = st.external_discovery().get_cached_external_slash_commands(
            "grok:/g",
            Duration::from_secs(300),
            Duration::from_secs(30),
        );
        assert!(matches!(hit, Some(ref v) if v.is_empty()));
    }

    fn sample_mcp_tool(name: &str) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: format!("{name} tool"),
            input_schema: serde_json::json!({ "type": "object" }),
            output_schema: None,
            annotations: None,
        }
    }

    #[test]
    fn mcp_tool_snapshot_persists_across_state_restart() {
        let st = test_state();
        let usage_dir = st.usage_dir.clone();
        st.set_mcp_tool_snapshot("srv".into(), "fp-1".into(), vec![sample_mcp_tool("echo")]);

        // 内存命中
        let hit = st
            .get_mcp_tool_snapshot("srv", "fp-1")
            .expect("in-memory hit");
        assert_eq!(hit[0].name, "echo");

        // 模拟重启：新 AppState 从同一 usage_dir 灌入落盘快照
        let restarted = AppState::new_headless(Settings::default(), usage_dir.clone());
        let reloaded = restarted
            .get_mcp_tool_snapshot("srv", "fp-1")
            .expect("snapshot reloads from disk after restart");
        assert_eq!(reloaded[0].name, "echo");
        assert_eq!(reloaded[0].description, "echo tool");

        let _ = std::fs::remove_dir_all(&usage_dir);
    }

    #[test]
    fn mcp_tool_snapshot_fingerprint_mismatch_misses() {
        let st = test_state();
        let usage_dir = st.usage_dir.clone();
        st.set_mcp_tool_snapshot("srv".into(), "fp-old".into(), vec![sample_mcp_tool("echo")]);

        assert!(st.get_mcp_tool_snapshot("srv", "fp-new").is_none());
        // 落盘后重启同样不命中改配置的旧快照
        let restarted = AppState::new_headless(Settings::default(), usage_dir.clone());
        assert!(restarted.get_mcp_tool_snapshot("srv", "fp-new").is_none());
        assert!(restarted.get_mcp_tool_snapshot("srv", "fp-old").is_some());

        let _ = std::fs::remove_dir_all(&usage_dir);
    }

    #[test]
    fn mcp_tool_snapshot_ignores_corrupt_disk_file() {
        let usage_dir =
            std::env::temp_dir().join(format!("kivio-test-usage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&usage_dir).expect("create usage dir");
        std::fs::write(usage_dir.join("mcp-tool-snapshots.json"), "{ not json !!")
            .expect("write corrupt snapshot file");

        // 损坏文件 = 视为无缓存，不 panic
        let st = AppState::new_headless(Settings::default(), usage_dir.clone());
        assert!(st.get_mcp_tool_snapshot("srv", "fp").is_none());

        // 后续写入照常覆盖损坏文件
        st.set_mcp_tool_snapshot("srv".into(), "fp".into(), vec![sample_mcp_tool("echo")]);
        let restarted = AppState::new_headless(Settings::default(), usage_dir.clone());
        assert!(restarted.get_mcp_tool_snapshot("srv", "fp").is_some());

        let _ = std::fs::remove_dir_all(&usage_dir);
    }

    #[test]
    fn mcp_tool_snapshot_file_contains_no_secrets_and_empty_tools_are_not_stored() {
        let st = test_state();
        let usage_dir = st.usage_dir.clone();
        // 空工具列表不入缓存（也不落盘）
        st.set_mcp_tool_snapshot("empty".into(), "fp".into(), Vec::new());
        assert!(st.get_mcp_tool_snapshot("empty", "fp").is_none());

        st.set_mcp_tool_snapshot("srv".into(), "fp".into(), vec![sample_mcp_tool("echo")]);
        let raw = std::fs::read_to_string(usage_dir.join("mcp-tool-snapshots.json"))
            .expect("snapshot file exists");
        // 落盘内容只有工具 schema + 指纹哈希：不该出现 headers/env/token 之类的键
        assert!(raw.contains("config_fingerprint"));
        assert!(raw.contains("echo"));
        for secret_marker in ["Authorization", "Bearer", "headers", "api_key"] {
            assert!(
                !raw.contains(secret_marker),
                "snapshot file must not contain {secret_marker}: {raw}"
            );
        }

        let _ = std::fs::remove_dir_all(&usage_dir);
    }

    #[test]
    fn pick_active_key_starts_at_idx_zero_when_no_active_recorded() {
        let st = test_state();
        assert_eq!(
            st.provider_runtime()
                .pick_active_key("p", 3, &HashSet::new()),
            Some(0)
        );
    }

    #[test]
    fn learned_provider_capabilities_are_endpoint_scoped_and_independent() {
        let st = test_state();
        st.provider_runtime()
            .mark_prompt_cache_key_unsupported("endpoint-a");
        assert!(st
            .provider_runtime()
            .prompt_cache_key_unsupported("endpoint-a"));
        assert!(!st
            .provider_runtime()
            .prompt_cache_retention_unsupported("endpoint-a"));
        assert!(!st
            .provider_runtime()
            .reasoning_replay_unsupported("endpoint-a"));
        assert!(!st
            .provider_runtime()
            .prompt_cache_key_unsupported("endpoint-b"));
        st.provider_runtime()
            .mark_prompt_cache_retention_unsupported("endpoint-a");
        st.provider_runtime()
            .mark_reasoning_replay_unsupported("endpoint-b");
        assert!(st
            .provider_runtime()
            .prompt_cache_retention_unsupported("endpoint-a"));
        assert!(st
            .provider_runtime()
            .reasoning_replay_unsupported("endpoint-b"));
        assert!(!st
            .provider_runtime()
            .reasoning_replay_unsupported("endpoint-a"));
    }

    #[test]
    fn pick_active_key_prefers_last_known_good_idx() {
        let st = test_state();
        st.provider_runtime().mark_key_ok("p", 2);
        assert_eq!(
            st.provider_runtime()
                .pick_active_key("p", 3, &HashSet::new()),
            Some(2)
        );
    }

    #[test]
    fn prefer_key_sets_active_and_clears_cooldowns() {
        let st = test_state();
        st.provider_runtime().mark_key_failed("p", 1);
        st.provider_runtime().prefer_key("p", 1);
        assert_eq!(
            st.provider_runtime()
                .pick_active_key("p", 3, &HashSet::new()),
            Some(1)
        );
    }

    #[test]
    fn pick_active_key_skips_tried_indices() {
        let st = test_state();
        let mut tried = HashSet::new();
        tried.insert(0);
        // active 是 0（没记录过 ok），但 0 已 tried → 应返回 1（环绕扫描下一个）
        assert_eq!(
            st.provider_runtime().pick_active_key("p", 3, &tried),
            Some(1)
        );
    }

    #[test]
    fn pick_active_key_skips_cooled_down_indices() {
        let st = test_state();
        st.provider_runtime().mark_key_failed("p", 0); // 0 进入冷却
                                                       // active 默认 0；0 在冷却 → 应跳到 1
        assert_eq!(
            st.provider_runtime()
                .pick_active_key("p", 3, &HashSet::new()),
            Some(1)
        );
    }

    #[test]
    fn pick_active_key_falls_back_to_cooled_when_all_cooled_but_untried() {
        let st = test_state();
        // 三个 key 全部冷却
        st.provider_runtime().mark_key_failed("p", 0);
        st.provider_runtime().mark_key_failed("p", 1);
        st.provider_runtime().mark_key_failed("p", 2);
        // 但都没试过 → 兜底返回某个 idx（不是 None），让用户至少有 key 用
        assert!(st
            .provider_runtime()
            .pick_active_key("p", 3, &HashSet::new())
            .is_some());
    }

    #[test]
    fn pick_active_key_returns_none_when_all_tried() {
        let st = test_state();
        let mut tried = HashSet::new();
        tried.insert(0);
        tried.insert(1);
        tried.insert(2);
        assert_eq!(st.provider_runtime().pick_active_key("p", 3, &tried), None);
    }

    #[test]
    fn mark_key_ok_clears_cooldown() {
        let st = test_state();
        st.provider_runtime().mark_key_failed("p", 0);
        // 此时 0 在冷却
        assert_ne!(
            st.provider_runtime()
                .pick_active_key("p", 2, &HashSet::new()),
            Some(0)
        );
        // 标记成功后冷却被清除 + active 设为 0
        st.provider_runtime().mark_key_ok("p", 0);
        assert_eq!(
            st.provider_runtime()
                .pick_active_key("p", 2, &HashSet::new()),
            Some(0)
        );
    }

    #[test]
    fn cooldowns_are_per_provider() {
        let st = test_state();
        st.provider_runtime().mark_key_failed("p1", 0);
        // p1 idx 0 冷却不影响 p2 idx 0
        assert_eq!(
            st.provider_runtime()
                .pick_active_key("p2", 2, &HashSet::new()),
            Some(0)
        );
    }

    #[test]
    fn pick_active_key_handles_active_idx_out_of_bounds() {
        // 用户原来有 5 个 key，active=4；删了 3 个，现在 total=2
        // pick_active_key 应该 clamp 到 total-1，不 panic
        let st = test_state();
        st.provider_runtime().mark_key_ok("p", 4);
        let result = st
            .provider_runtime()
            .pick_active_key("p", 2, &HashSet::new());
        assert!(result.is_some());
        assert!(result.unwrap() < 2);
    }

    #[test]
    fn chat_session_consent_is_per_conversation() {
        let st = test_state();
        assert!(!st.chat_interactions().has_session_consent("conv-1"));
        st.chat_interactions().grant_session_consent("conv-1");
        assert!(st.chat_interactions().has_session_consent("conv-1"));
        // Consent is scoped to a single conversation, not global.
        assert!(!st.chat_interactions().has_session_consent("conv-2"));
    }

    #[test]
    fn chat_tool_always_allow_is_per_conversation_and_tool() {
        let st = test_state();
        assert!(!st
            .chat_interactions()
            .has_tool_always_allow("conv-1", "write"));
        st.chat_interactions()
            .grant_tool_always_allow("conv-1", "write");
        assert!(st
            .chat_interactions()
            .has_tool_always_allow("conv-1", "write"));
        // 外部 CLI 报 PascalCase，必须命中同一条。
        assert!(st
            .chat_interactions()
            .has_tool_always_allow("conv-1", "Write"));
        // 只放行按下的那个工具，不是整会话放行。
        assert!(!st
            .chat_interactions()
            .has_tool_always_allow("conv-1", "read"));
        // 不跨对话。
        assert!(!st
            .chat_interactions()
            .has_tool_always_allow("conv-2", "write"));
        st.forget_chat_conversation_runtime("conv-1");
        assert!(!st
            .chat_interactions()
            .has_tool_always_allow("conv-1", "write"));
    }

    // --- 多模型一问多答：并发护栏 per-run 化（任务 06-30 步骤 1） ---

    #[test]
    fn single_run_generation_equivalence() {
        // 单 run（单模型）行为必须与改前等价：分配 → 活跃 → 取消 → 失活。
        let st = test_state();
        let gen = st.chat_runtime().begin_generation("conv");
        assert!(st.chat_runtime().is_generation_active("conv", gen));
        st.cancel_chat_generation("conv");
        assert!(!st.chat_runtime().is_generation_active("conv", gen));
    }

    #[test]
    fn single_run_end_generation_retires_only_self() {
        let st = test_state();
        let gen = st.chat_runtime().begin_generation("conv");
        assert!(st.chat_runtime().is_generation_active("conv", gen));
        st.chat_runtime().end_generation("conv", gen);
        assert!(!st.chat_runtime().is_generation_active("conv", gen));
    }

    #[test]
    fn new_run_does_not_invalidate_sibling_run() {
        // 同会话开第二条 run（多模型并发）不得作废第一条。
        let st = test_state();
        let gen_a = st.chat_runtime().begin_generation("conv");
        let gen_b = st.chat_runtime().begin_generation("conv");
        assert_ne!(gen_a, gen_b);
        assert!(st.chat_runtime().is_generation_active("conv", gen_a));
        assert!(st.chat_runtime().is_generation_active("conv", gen_b));
    }

    #[test]
    fn cancel_kills_all_runs_in_conversation() {
        // R4：cancel 一刀切该会话所有在跑 run。
        let st = test_state();
        let gen_a = st.chat_runtime().begin_generation("conv");
        let gen_b = st.chat_runtime().begin_generation("conv");
        let gen_c = st.chat_runtime().begin_generation("conv");
        st.cancel_chat_generation("conv");
        assert!(!st.chat_runtime().is_generation_active("conv", gen_a));
        assert!(!st.chat_runtime().is_generation_active("conv", gen_b));
        assert!(!st.chat_runtime().is_generation_active("conv", gen_c));
    }

    #[test]
    fn cancel_is_per_conversation() {
        // 取消 conv-1 不影响 conv-2（含 sub-agent 用独立合成 conversation_id 的级联语义）。
        let st = test_state();
        let gen1 = st.chat_runtime().begin_generation("conv-1");
        let gen2 = st.chat_runtime().begin_generation("conv-2");
        st.cancel_chat_generation("conv-1");
        assert!(!st.chat_runtime().is_generation_active("conv-1", gen1));
        assert!(st.chat_runtime().is_generation_active("conv-2", gen2));
    }

    #[test]
    fn end_one_run_keeps_sibling_active() {
        let st = test_state();
        let gen_a = st.chat_runtime().begin_generation("conv");
        let gen_b = st.chat_runtime().begin_generation("conv");
        st.chat_runtime().end_generation("conv", gen_a);
        assert!(!st.chat_runtime().is_generation_active("conv", gen_a));
        assert!(st.chat_runtime().is_generation_active("conv", gen_b));
    }

    #[test]
    fn reply_slot_allows_multiple_runs_same_conversation() {
        // 同会话允许多条 run 并存；同一 (conv, run) 重复进入才拒绝。
        let st = test_state();
        assert!(!st.chat_runtime().has_active_reply("conv"));
        assert!(st.chat_runtime().try_begin_reply("conv", "run-1"));
        assert!(st.chat_runtime().try_begin_reply("conv", "run-2"));
        // 同一 run 重复注册被拒。
        assert!(!st.chat_runtime().try_begin_reply("conv", "run-1"));
        assert!(st.chat_runtime().has_active_reply("conv"));
    }

    #[test]
    fn reply_slot_release_is_per_run() {
        let st = test_state();
        st.chat_runtime().try_begin_reply("conv", "run-1");
        st.chat_runtime().try_begin_reply("conv", "run-2");
        st.chat_runtime().end_reply("conv", "run-1");
        // 仍有 run-2 在跑 → 会话仍 busy。
        assert!(st.chat_runtime().has_active_reply("conv"));
        st.chat_runtime().end_reply("conv", "run-2");
        // 全部释放 → 会话不再 busy，且可重新注册同名 run。
        assert!(!st.chat_runtime().has_active_reply("conv"));
        assert!(st.chat_runtime().try_begin_reply("conv", "run-1"));
    }

    #[test]
    fn forget_conversation_clears_active_generations() {
        let st = test_state();
        let gen = st.chat_runtime().begin_generation("conv");
        assert!(st.chat_runtime().is_generation_active("conv", gen));
        st.forget_chat_conversation_runtime("conv");
        assert!(!st.chat_runtime().is_generation_active("conv", gen));
    }

    #[test]
    fn reserve_send_is_atomic_busy_check_and_reserve() {
        // 命令入口哨兵：首个预留成功并占槽；同会话第二个预留（哨兵或真实 run 在跑）被拒。
        let st = test_state();
        assert!(st.chat_runtime().try_reserve_send("conv", "send-1"));
        assert!(st.chat_runtime().has_active_reply("conv"));
        // 任意第二个预留在哨兵存活期间被拒（关闭并发发送的 TOCTOU）。
        assert!(!st.chat_runtime().try_reserve_send("conv", "send-2"));
        // 哨兵存活期间，真实 per-run 槽位仍可与之共存（fan-out 各臂注册自己的 run）。
        assert!(st.chat_runtime().try_begin_reply("conv", "run-arm-1"));
        assert!(st.chat_runtime().try_begin_reply("conv", "run-arm-2"));
        // 释放哨兵后仍有 run 在跑 → 仍 busy；新预留仍被拒。
        st.chat_runtime().end_reply("conv", "send-1");
        assert!(st.chat_runtime().has_active_reply("conv"));
        assert!(!st.chat_runtime().try_reserve_send("conv", "send-3"));
        // 全部 run 释放后才能再次预留。
        st.chat_runtime().end_reply("conv", "run-arm-1");
        st.chat_runtime().end_reply("conv", "run-arm-2");
        assert!(!st.chat_runtime().has_active_reply("conv"));
        assert!(st.chat_runtime().try_reserve_send("conv", "send-4"));
    }

    #[test]
    fn app_state_does_not_reintroduce_domain_forwarding() {
        let source = include_str!("state.rs");
        let impl_end = source
            .find("impl crate::mcp::McpSettingsPersistence for AppState")
            .expect("AppState impl precedes the persistence adapter");
        let impl_source = &source[..impl_end];
        for method in [
            "fn next_chat_generation(",
            "fn end_chat_generation(",
            "fn is_chat_generation_active(",
            "fn has_chat_consent(",
            "fn grant_chat_consent(",
            "fn has_tool_always_allow(",
            "fn grant_tool_always_allow(",
            "fn push_chat_steering(",
            "fn take_chat_steering(",
            "fn push_chat_follow_up(",
            "fn take_chat_follow_up(",
            "fn try_begin_chat_reply(",
            "fn try_reserve_chat_send(",
            "fn conversation_has_active_reply(",
            "fn end_chat_reply(",
            "fn finish_chat_reply_generation(",
            "fn pick_active_key(",
            "fn mark_key_failed(",
            "fn mark_key_ok(",
            "fn prefer_key(",
            "fn sync_preferred_api_keys(",
            "fn prompt_cache_key_unsupported(",
            "fn get_cached_external_slash_commands(",
            "fn get_cached_detected_agents(",
            "fn acquire_model_probe(",
            "fn external_live_session_control(",
            "fn register_external_live_session(",
            "fn upsert_external_background_task(",
            "fn register_background_command(",
            "fn kill_all_background_commands(",
        ] {
            assert!(
                !impl_source.contains(method),
                "AppState must not grow domain forwarding again: {method}"
            );
        }
        assert!(
            impl_source.contains("fn cancel_chat_generation("),
            "cross-domain cancel stays on the composition root"
        );
        assert!(
            impl_source.contains("fn forget_chat_conversation_runtime("),
            "cross-domain conversation cleanup stays on the composition root"
        );
    }
}
