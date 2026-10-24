use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

#[path = "provider_settings.rs"]
pub(crate) mod provider;
pub use provider::{
    format_chat_login_required_error,
    CacheRetention, DefaultModelSelection, DefaultModelsConfig, ModelCapabilities, ModelInfo,
    ModelPricing, ModelProvider, OAuthConfig, ProviderApiFormat, ProviderCustomHeader,
    ProviderRequestConfig, default_chat_max_output_tokens, is_reserved_header_key,
    is_usable_header, is_valid_header_key, is_valid_header_value, sanitize_default_model_selection,
};

#[path = "tool_settings.rs"]
pub(crate) mod tools;
pub use tools::{
    CHAT_TOOL_MAX_OUTPUT_CHARS, CHAT_TOOL_MAX_TIMEOUT_MS, CHAT_TOOL_MIN_OUTPUT_CHARS,
    CHAT_TOOL_MIN_TIMEOUT_MS, ChatMcpServer, ChatNativeToolsConfig, ChatToolsConfig, ConnectorAuth,
    DEFAULT_MAX_TOOL_OUTPUT_CHARS, HOOK_DEFAULT_TIMEOUT_MS, HOOK_MAX_TIMEOUT_MS,
    HOOK_MIN_TIMEOUT_MS, HookDef, MCP_IDLE_TIMEOUT_MAX_MS, MCP_IDLE_TIMEOUT_MIN_MS,
    SubAgentModelSelection, default_chat_working_directory,
};
pub(crate) use tools::is_cua_mcp_server_id;
use tools::{default_chat_approval_policy, default_skill_fallback_mode, sanitize_hooks};

#[path = "settings_config.rs"]
pub(crate) mod config;
pub use config::*;
use config::{
    BUILTIN_THEME_IDS, canonicalize_theme_color, clamp_chat_max_output_tokens,
    default_deepseek_base_url, default_exa_mcp_url, default_grok_base_url, default_grok_model,
    default_rapid_ocr_tier, default_rapid_ocr_tier_high, default_theme, default_tinyfish_mcp_url,
    default_ui_font_scale, default_web_search_depth, sanitize_custom_themes,
};

// 设置存储文件名
const SETTINGS_STORE: &str = "settings.json";
const LEGACY_APPLE_INTELLIGENCE_BASE_URL: &str = "applefoundation://local";
pub(crate) const SETTINGS_CHANGED_EVENT: &str = "kivio-settings-changed";

/// Serializes full saves and their asynchronous workspace migrations. Lightweight
/// writers intentionally do not acquire this gate; revision CAS detects them.
#[derive(Default)]
pub(crate) struct SettingsPersistenceGate {
    full_save: tokio::sync::Mutex<()>,
}

impl SettingsPersistenceGate {
    pub(crate) async fn begin_full_save(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.full_save.lock().await
    }
}

#[cfg(test)]
mod persistence_gate_tests {
    use super::*;
    #[tokio::test]
    async fn full_saves_are_serialized_and_cancellation_releases_the_gate() {
        let gate = std::sync::Arc::new(SettingsPersistenceGate::default());
        let first = gate.begin_full_save().await;
        let next_gate = gate.clone();
        let next = tokio::spawn(async move {
            let _guard = next_gate.begin_full_save().await;
        });
        tokio::task::yield_now().await;
        assert!(!next.is_finished());
        next.abort();
        assert!(next.await.unwrap_err().is_cancelled());
        drop(first);
        let _guard =
            tokio::time::timeout(std::time::Duration::from_secs(1), gate.begin_full_save())
                .await
                .unwrap();
    }
}

/// Opaque identity of one canonical settings value. `epoch` changes on every backend process
/// start; `revision` advances for every successful in-memory/durable settings transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SettingsVersion {
    pub epoch: String,
    pub revision: u64,
}

/// Settings and the exact version that owns them. Keeping these together prevents callers from
/// accidentally pairing an old full object with a revision fetched after it was edited.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub settings: Settings,
    pub version: SettingsVersion,
}

/// Structured IPC error used by settings writes. Frontends must branch on `code`, never parse the
/// human-readable message.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum SettingsError {
    VersionConflict {
        message: String,
        #[serde(rename = "expectedVersion")]
        expected_version: SettingsVersion,
        #[serde(rename = "actualVersion")]
        actual_version: SettingsVersion,
    },
    OperationFailed {
        message: String,
    },
}

impl SettingsError {
    pub(crate) fn version_conflict(
        expected_version: SettingsVersion,
        actual_version: SettingsVersion,
    ) -> Self {
        Self::VersionConflict {
            message: format!(
                "settings changed since this edit began (expected {}:{}, actual {}:{}); refresh and retry",
                expected_version.epoch,
                expected_version.revision,
                actual_version.epoch,
                actual_version.revision
            ),
            expected_version,
            actual_version,
        }
    }
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VersionConflict { message, .. } | Self::OperationFailed { message } => {
                formatter.write_str(message)
            }
        }
    }
}

impl From<String> for SettingsError {
    fn from(message: String) -> Self {
        Self::OperationFailed { message }
    }
}

impl From<&str> for SettingsError {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

impl From<SettingsError> for String {
    fn from(error: SettingsError) -> Self {
        error.to_string()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsChangedEvent {
    version: SettingsVersion,
}

/// The pre-green-light default. `sanitize_settings`' one-shot migration only
/// preserves this policy on older installs; explicit auto choices also survive.
#[cfg(test)]
const LEGACY_DEFAULT_APPROVAL_POLICY: &str = "readonly_auto_sensitive_confirm";


/**
 * 设置数据清理与迁移
 *
 * 执行以下操作：
 * 1. 从旧版单提供商配置迁移到多提供商体系
 * 2. 确保空 provider 字段有默认值
 * 3. 如果当前模型不在 enabled_models 中则清空或切到第一个启用模型
 * 4. 规范化快捷键字符串
 * 5. 确保必要字段不为空
 */


pub fn chat_memory_tools_enabled(settings: &Settings) -> bool {
    settings.chat_memory.enabled
}

pub fn chat_image_generation_enabled_for_session(
    settings: &Settings,
    session: Option<SessionModel<'_>>,
) -> bool {
    crate::chat::model_metadata::image_generation_model_for_session(settings, session).is_some()
}



/// Global skill gate: Settings enable list + 插件门闸。
/// 插件附属 skill 仅在对应插件「仍受支持、已安装且启用」时可用。


/// When [`skill_globally_available`] is false, returns a loop/UI-friendly error.


include!("settings_skill_policy.rs");
include!("settings_normalize.rs");

/**
 * 持久化设置到存储文件
 * 从 v2.4 起 API Key 直接保存在 settings.json 的 api_keys 数组中
 *
 * 降级兼容：写盘前把 api_keys[0] 镜像到 api_key_legacy（serde rename = "apiKey"）字段，
 * 这样老版本（v2.3.x）反序列化时仍能从 apiKey 字段读到主 key 不丢。
 * 新版加载时 sanitize_settings 会把 api_key_legacy.take() 合并回 api_keys 并去重，无副作用。
 */
fn settings_for_persistence(settings: &Settings) -> (Settings, Settings) {
    let canonical = sanitize_settings(settings.clone());
    let mut to_persist = canonical.clone();

    // Keep legacy top-level chat fields from turning Lens/Translator fallback into
    // an explicit defaultModels.chat selection on the next load.
    mirror_explicit_chat_default_for_persistence(&mut to_persist);

    for provider in &mut to_persist.providers {
        if let Some(primary) = provider.preferred_api_key() {
            provider.api_key_legacy = Some(primary.to_string());
        }
    }

    // 降级镜像：把 ocr_mode 投影回 use_system_ocr，让降级到 v2.5.x 的版本仍能从 useSystemOcr 字段
    // 读到对应行为。RapidOcr 模式镜像为 false（v2.5.x 没有 RapidOCR 概念，落回 CloudVision）。
    let ocr_mode = to_persist
        .screenshot_translation
        .ocr_mode
        .unwrap_or(OcrMode::CloudVision);
    to_persist.screenshot_translation.use_system_ocr = matches!(ocr_mode, OcrMode::System);
    to_persist.screenshot_translation.ocr_mode = Some(ocr_mode);

    (canonical, to_persist)
}

pub(crate) fn settings_snapshot(state: &crate::state::AppState) -> SettingsSnapshot {
    // Writers advance the generation while holding this same write lock, so reading the value
    // and generation under a read guard produces one coherent snapshot.
    let settings = state.settings_read();
    let revision = state.settings_revision();
    SettingsSnapshot {
        settings: settings.clone(),
        version: state.settings_version_at(revision),
    }
}

fn commit_settings_with(
    state: &crate::state::AppState,
    expected_version: SettingsVersion,
    next: Settings,
    persist: impl FnOnce(&Settings) -> Result<(), String>,
) -> Result<SettingsSnapshot, SettingsError> {
    state.publish_settings_transaction(
        Some(expected_version),
        |_| Ok(sanitize_settings(next)),
        persist,
    )
}

fn update_settings_with(
    state: &crate::state::AppState,
    update: impl FnOnce(&mut Settings) -> Result<(), String>,
    persist: impl FnOnce(&Settings) -> Result<(), String>,
) -> Result<SettingsSnapshot, SettingsError> {
    state.publish_settings_transaction(
        None,
        |current| {
            let mut next = current.clone();
            update(&mut next)?;
            Ok(sanitize_settings(next))
        },
        persist,
    )
}

/// Canonical in-memory update for headless/runtime paths that intentionally have no AppHandle.
/// It still participates in the same revisioned serial history as durable settings writes.
pub(crate) fn update_settings_in_memory(
    state: &crate::state::AppState,
    update: impl FnOnce(&mut Settings) -> Result<(), String>,
) -> Result<SettingsSnapshot, SettingsError> {
    update_settings_with(state, update, |_| Ok(()))
}

fn emit_settings_changed(app: &AppHandle, version: &SettingsVersion) {
    // Notification failure must not roll back a durable commit. Consumers recover by fetching the
    // latest snapshot when their webview is activated; the event intentionally carries no values
    // or credentials.
    let _ = app.emit(
        SETTINGS_CHANGED_EVENT,
        SettingsChangedEvent {
            version: version.clone(),
        },
    );
}

/// Atomically canonicalize, persist, and publish a settings mutation. Every writer that both
/// changes AppState and saves settings must use this interface so disk and memory advance as one
/// serial history.
pub(crate) fn update_settings(
    app: &AppHandle,
    state: &crate::state::AppState,
    update: impl FnOnce(&mut Settings) -> Result<(), String>,
) -> Result<SettingsSnapshot, SettingsError> {
    let snapshot =
        update_settings_with(state, update, |canonical| persist_settings(app, canonical))?;
    emit_settings_changed(app, &snapshot.version);
    Ok(snapshot)
}

/// Publish a full settings snapshot only if no side writer committed since the client's original
/// read. Callers may safely do asynchronous preparation between [`settings_snapshot`] and this
/// call; the process epoch plus revision rejects both concurrent writes and pre-restart tokens.
pub(crate) fn commit_settings(
    app: &AppHandle,
    state: &crate::state::AppState,
    expected_version: SettingsVersion,
    next: Settings,
) -> Result<SettingsSnapshot, SettingsError> {
    let snapshot = commit_settings_with(state, expected_version, next, |canonical| {
        persist_settings(app, canonical)
    })?;
    emit_settings_changed(app, &snapshot.version);
    Ok(snapshot)
}

fn persist_then_apply_with_rollback(
    persist: impl FnOnce() -> Result<(), String>,
    rollback_unpersisted: impl FnOnce() -> Result<(), String>,
    apply: impl FnOnce() -> Result<(), String>,
    rollback_persisted: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if let Err(error) = persist() {
        return match rollback_unpersisted() {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}; cache rollback failed: {rollback_error}")),
        };
    }
    if let Err(error) = apply() {
        return match rollback_persisted() {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!("{error}; rollback failed: {rollback_error}")),
        };
    }
    Ok(())
}

/// 只有外部 CLI 配置本身变了才需要重新落地原生配置。
///
/// `materialize_all` 会合并写 `~/.grok/config.toml`、`~/.kimi-code/config.toml`、OpenCode / Pi
/// 的原生配置和 dsh 的 profile —— 都是用户家目录里的文件。`persist_settings` 是所有设置写盘的
/// 总出口（收藏模型、热键、主题……），无条件物化等于「改任何设置都重写一遍外部 CLI 的用户配置」，
/// 与 `provider_profile` 模块头写的「物化时机是保存 / 切换供应商那一次」不符。
/// `materialize` 读的全部输入都来自 `overrides`（即 `chat.external_cli_agents` 的镜像），
/// 所以这一个字段不变就没有任何东西需要重写。
fn external_cli_agents_changed(previous: &Settings, next: &Settings) -> bool {
    previous.chat.external_cli_agents != next.chat.external_cli_agents
}

fn apply_external_agent_settings(previous: &Settings, settings: &Settings) -> Result<(), String> {
    crate::external_agents::overrides::sync_from_settings(settings);
    if !external_cli_agents_changed(previous, settings) {
        return Ok(());
    }
    crate::external_agents::provider_profile::materialize_all()
}

fn read_settings_document(
    path: &std::path::Path,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    match std::fs::read(path) {
        Ok(data) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                    |error| format!("Cannot restrict settings file permissions: {error}"),
                )?;
            }
            serde_json::from_slice(&data)
                .map_err(|_| "Invalid settings JSON; existing file preserved".into())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(format!("Cannot read settings JSON: {error}")),
    }
}
fn write_settings_document(
    path: &std::path::Path,
    document: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let data = serde_json::to_string_pretty(document).map_err(|error| error.to_string())?;
    crate::chat::storage::atomic_write(path, &data, "settings")
}

pub fn persist_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let (canonical, to_persist) = settings_for_persistence(settings);
    use tauri::Manager;
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join(SETTINGS_STORE);
    let previous = read_settings_document(&path)?;
    let previous_settings = previous
        .get("settings")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .map(sanitize_settings)
        .unwrap_or_default();
    let mut next = previous.clone();
    next.insert(
        "settings".into(),
        serde_json::to_value(&to_persist).map_err(|e| e.to_string())?,
    );
    let external_agents_changed = external_cli_agents_changed(&previous_settings, &canonical);
    persist_then_apply_with_rollback(
        || write_settings_document(&path, &next),
        || Ok(()),
        || apply_external_agent_settings(&previous_settings, &canonical),
        || {
            let disk_rollback = write_settings_document(&path, &previous);
            crate::external_agents::overrides::sync_from_settings(&previous_settings);
            // apply 只在配置变了时才写过原生文件；没写过就没有什么要还原。
            let profile_rollback = if external_agents_changed {
                crate::external_agents::provider_profile::materialize_all()
            } else {
                Ok(())
            };
            match (disk_rollback, profile_rollback) {
                (Ok(()), Ok(())) => Ok(()),
                (disk, profile) => Err(format!(
                    "settings: {}; provider: {}",
                    disk.err().unwrap_or_else(|| "none".to_string()),
                    profile.err().unwrap_or_else(|| "none".to_string())
                )),
            }
        },
    )?;

    // 供应商可能变了：模型列表（300s）与可用性（600s）两个探测缓存都得作废。
    // 首次启动的内置专家迁移会在 AppState manage 之前保存设置，此时还没有缓存可清。
    if let Some(state) = app.try_state::<crate::state::AppState>() {
        state
            .external_discovery()
            .clear_all_external_agent_models_cache();
        state.external_discovery().clear_detected_agents_cache();
    }
    Ok(())
}

/**
 * 一次性数据迁移：v2.4.5 把 identifier 从 com.zmair.keylingo 改为 com.zmair.kivio。
 * Tauri 的 app_data_dir 直接由 identifier 派生，改名后新目录是空的，
 * 老用户升级会丢失 settings.json / lens-history。这里在新目录还没数据时，
 * 把同级的旧目录整个递归拷贝过来。
 *
 * 幂等：新目录已存在 settings.json → 跳过；旧目录不存在 → 跳过（全新安装）。
 */
fn migrate_legacy_app_data(app: &AppHandle) {
    // Development installations must never import existing production data.
    if crate::app_data::is_dev() {
        return;
    }
    use tauri::Manager;
    let new_dir = match app.path().app_data_dir() {
        Ok(d) => d,
        Err(err) => {
            eprintln!("[migrate-app-data] app_data_dir unavailable: {err}");
            return;
        }
    };
    if new_dir.join(SETTINGS_STORE).exists() {
        return;
    }

    let Some(parent) = new_dir.parent() else {
        return;
    };
    // 旧 identifier 的目录名就是 identifier 本身（macOS / Windows / Linux 都一致）
    let legacy_dir = parent.join("com.zmair.keylingo");
    if !legacy_dir.is_dir() {
        return;
    }

    if let Err(err) = std::fs::create_dir_all(&new_dir) {
        eprintln!("[migrate-app-data] mkdir new dir failed: {err}");
        return;
    }

    match copy_dir_recursive(&legacy_dir, &new_dir) {
        Ok(()) => eprintln!(
            "[migrate-app-data] copied legacy app data: {} → {}",
            legacy_dir.display(),
            new_dir.display()
        ),
        Err(err) => eprintln!("[migrate-app-data] copy failed: {err}"),
    }
}

fn copy_dir_recursive(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if src.is_file() && !dst.exists() {
            // 不覆盖已有目标文件：避免与用户在新路径下手动建/写过的内容冲突
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/**
 * 从存储文件加载设置
 * 执行清理迁移（legacy identifier 目录、sanitize）
 */
pub fn load_settings(app: &AppHandle) -> Settings {
    // 入口先把旧 identifier 目录的数据搬到新目录（幂等）
    migrate_legacy_app_data(app);
    use tauri::Manager;
    let settings = app
        .path()
        .app_data_dir()
        .ok()
        .and_then(|root| read_settings_document(&root.join(SETTINGS_STORE)).ok())
        .and_then(|mut document| document.remove("settings"))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let settings = sanitize_settings(settings);
    crate::external_agents::overrides::sync_from_settings(&settings);
    settings
}

/**
 * 规范化可选提示词：去除空白，空字符串转为 None
 */
#[cfg(test)]
mod tests {
    #[test]
    fn settings_json_owner_preserves_additional_keys_and_uses_private_atomic_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original =
            serde_json::json!({"settings": {"themeMode":"dark"}, "other-plugin-key": "retained"});
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        let mut document = super::read_settings_document(&path).unwrap();
        document.insert("settings".into(), serde_json::json!({"themeMode":"light"}));
        super::write_settings_document(&path, &document).unwrap();
        let reopened = super::read_settings_document(&path).unwrap();
        assert_eq!(reopened["settings"]["themeMode"], "light");
        assert_eq!(reopened["other-plugin-key"], "retained");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, "invalid-json").unwrap();
        assert!(super::read_settings_document(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid-json");
    }
    use std::cell::RefCell;

    use crate::state::test_app_state;

    #[test]
    fn reading_translation_language_migrates_once_and_stays_independent() {
        for (legacy, expected, prompt_language) in [
            ("auto", "zh", "Simplified Chinese"),
            ("ja", "ja", "Japanese"),
            ("zh-Hant", "zh-Hant", "Traditional Chinese"),
            ("es", "es", "Spanish"),
            ("ar", "ar", "Arabic"),
            ("th", "th", "Thai"),
        ] {
            let old: Settings =
                serde_json::from_value(serde_json::json!({"targetLang": legacy})).unwrap();
            let migrated = sanitize_settings(old);
            let mut saved = serde_json::to_value(migrated).unwrap();
            assert_eq!(saved["readingTargetLang"], expected);
            saved["targetLang"] = serde_json::json!("ko");
            let reopened = sanitize_settings(serde_json::from_value(saved).unwrap());
            assert_eq!(
                crate::utils::language_name(&reopened.reading_target_lang),
                prompt_language
            );
            let saved = serde_json::to_value(reopened).unwrap();
            assert_eq!(saved["readingTargetLang"], expected);
            assert_eq!(saved["targetLang"], "ko");
        }
    }

    #[test]
    fn mixer_roles_migrate_legacy_selection_once_and_keep_effort_with_model() {
        let mut settings = Settings::default();
        let provider: ModelProvider = serde_json::from_value(serde_json::json!({
            "id": "worker", "name": "Worker", "enabled": true,
            "baseUrl": "http://localhost", "enabledModels": ["gpt-5.5"]
        }))
        .unwrap();
        settings.providers = vec![provider];
        settings.chat_tools.sub_agent_provider_id = "worker".into();
        settings.chat_tools.sub_agent_model = "gpt-5.5".into();
        let mut settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.sub_agent_models["task"].model,
            "gpt-5.5"
        );
        assert!(settings.chat_tools.sub_agent_provider_id.is_empty());
        assert!(settings.chat_tools.sub_agent_model.is_empty());
        settings
            .chat_tools
            .sub_agent_models
            .get_mut("task")
            .unwrap()
            .thinking_level = Some("medium".into());
        settings.chat_tools.sub_agent_model = "obsolete".into();
        settings.chat_tools.sub_agent_provider_id = "worker".into();
        let saved = serde_json::to_value(sanitize_settings(settings)).unwrap();
        let restored = sanitize_settings(serde_json::from_value(saved).unwrap());
        assert_eq!(
            restored.chat_tools.sub_agent_models["task"].model,
            "gpt-5.5"
        );
        assert_eq!(
            restored.chat_tools.sub_agent_models["task"]
                .thinking_level
                .as_deref(),
            Some("medium")
        );
    }

    #[test]
    fn authentication_readiness_accepts_anonymous_keys_but_requires_oauth_login() {
        let mut provider: ModelProvider = serde_json::from_value(serde_json::json!({
            "id": "anonymous",
            "name": "Anonymous",
            "baseUrl": "http://localhost:1234/v1",
            "enabledModels": ["local-model"]
        }))
        .unwrap();
        for format in [
            "openai_chat",
            "openai_responses",
            "xai_responses",
            "anthropic_messages",
            "gemini",
        ] {
            provider.api_format = format.into();
            for keys in [
                Vec::new(),
                vec![" ".into(), "\t".into()],
                vec!["key".into()],
            ] {
                provider.api_keys = keys;
                assert!(provider.authentication_ready());
                assert!(provider_has_usable_config(&provider));
            }
        }
        provider.request.oauth = Some(
            serde_json::from_value(serde_json::json!({
                "provider": "codex"
            }))
            .unwrap(),
        );
        assert!(!provider.authentication_ready());
        assert!(!provider_has_usable_config(&provider));
        provider.request.oauth.as_mut().unwrap().credential_id = Some("saved-login".into());
        assert!(provider.authentication_ready());
        assert!(provider_has_usable_config(&provider));
        provider.enabled = false;
        assert!(!provider_has_usable_config(&provider));
        provider.enabled = true;
        provider.enabled_models.clear();
        assert!(!provider_has_usable_config(&provider));
    }

    #[test]
    fn failed_durable_save_does_not_apply_external_settings_side_effects() {
        let applied = std::cell::Cell::new(false);
        let cache_rolled_back = std::cell::Cell::new(false);
        let durable_rolled_back = std::cell::Cell::new(false);

        let result = persist_then_apply_with_rollback(
            || Err("disk full".to_string()),
            || {
                cache_rolled_back.set(true);
                Ok(())
            },
            || {
                applied.set(true);
                Ok(())
            },
            || {
                durable_rolled_back.set(true);
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "disk full");
        assert!(!applied.get());
        assert!(cache_rolled_back.get());
        assert!(!durable_rolled_back.get());
    }

    #[test]
    fn failed_external_settings_apply_rolls_back_the_durable_save() {
        let cache_rolled_back = std::cell::Cell::new(false);
        let rolled_back = std::cell::Cell::new(false);
        let result = persist_then_apply_with_rollback(
            || Ok(()),
            || {
                cache_rolled_back.set(true);
                Ok(())
            },
            || Err("profile write failed".to_string()),
            || {
                rolled_back.set(true);
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "profile write failed");
        assert!(!cache_rolled_back.get());
        assert!(rolled_back.get());
    }

    #[test]
    fn settings_side_write_commits_the_same_canonical_value_that_is_persisted() {
        let state = test_app_state();
        let persisted = RefCell::new(None);

        let committed = update_settings_with(
            &state,
            |settings| {
                settings.chat_tools.servers.push(ChatMcpServer {
                    id: " spaced ".into(),
                    name: " Spaced server ".into(),
                    command: " npx ".into(),
                    args: vec!["  package-name  ".into()],
                    ..Default::default()
                });
                Ok(())
            },
            |canonical| {
                *persisted.borrow_mut() = Some(canonical.clone());
                Ok(())
            },
        )
        .expect("side write should commit");

        let in_memory = state.settings_read().clone();
        let persisted = persisted.into_inner().expect("persist called");
        assert_eq!(committed.settings.chat_tools.servers[0].id, "spaced");
        assert_eq!(committed.settings.chat_tools.servers[0].command, "npx");
        assert_eq!(
            committed.settings.chat_tools.servers[0].args,
            ["package-name"]
        );
        assert_eq!(in_memory.chat_tools.servers[0].id, "spaced");
        assert_eq!(persisted.chat_tools.servers[0].id, "spaced");
    }

    #[test]
    fn settings_reads_remain_available_until_durable_commit() {
        let state = test_app_state();
        let before = settings_snapshot(&state);
        let committed = update_settings_with(
            &state,
            |settings| {
                settings.theme = "light".into();
                Ok(())
            },
            |_| {
                let current = state
                    .try_settings_read()
                    .expect("slow persistence must not block snapshot readers");
                assert_eq!(current.theme, before.settings.theme);
                assert_eq!(state.settings_revision(), before.version.revision);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(settings_snapshot(&state).settings.theme, "light");
        assert_eq!(committed.version.revision, before.version.revision + 1);
    }

    #[test]
    fn failed_settings_persist_does_not_publish_or_advance_the_version() {
        let state = test_app_state();
        let before = settings_snapshot(&state);

        let error = update_settings_with(
            &state,
            |settings| {
                settings.theme = "light".into();
                Ok(())
            },
            |_| Err("disk full".into()),
        )
        .expect_err("a failed durable action must abort the transaction");

        assert!(matches!(error, SettingsError::OperationFailed { .. }));
        let after = settings_snapshot(&state);
        assert_eq!(after.version, before.version);
        assert_eq!(after.settings.theme, before.settings.theme);
    }

    #[test]
    fn settings_change_event_contains_only_the_version() {
        let version = SettingsVersion {
            epoch: "process-epoch".into(),
            revision: 7,
        };
        let payload = serde_json::to_value(SettingsChangedEvent {
            version: version.clone(),
        })
        .expect("event payload should serialize");

        assert_eq!(payload, serde_json::json!({ "version": version }));
        assert!(payload.get("settings").is_none());
    }

    #[test]
    fn settings_conflict_error_has_a_stable_tagged_wire_shape() {
        let expected = SettingsVersion {
            epoch: "old-process".into(),
            revision: 3,
        };
        let actual = SettingsVersion {
            epoch: "new-process".into(),
            revision: 1,
        };
        let payload = serde_json::to_value(SettingsError::version_conflict(
            expected.clone(),
            actual.clone(),
        ))
        .expect("settings errors should serialize");

        assert_eq!(payload["code"], "versionConflict");
        assert_eq!(payload["expectedVersion"], serde_json::json!(expected));
        assert_eq!(payload["actualVersion"], serde_json::json!(actual));
        assert!(
            payload["message"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
    }

    #[test]
    fn stale_full_save_cannot_overwrite_a_newer_side_write() {
        let state = test_app_state();
        let full_save = settings_snapshot(&state);
        let persisted = RefCell::new(Settings::default());

        update_settings_with(
            &state,
            |settings| {
                settings.favorite_models = vec!["provider:newer".into()];
                Ok(())
            },
            |canonical| {
                *persisted.borrow_mut() = canonical.clone();
                Ok(())
            },
        )
        .expect("side write should commit first");

        let mut stale = full_save.settings;
        stale.theme = "light".into();
        let error = commit_settings_with(&state, full_save.version, stale, |canonical| {
            *persisted.borrow_mut() = canonical.clone();
            Ok(())
        })
        .expect_err("stale full save must report a conflict");

        assert!(matches!(error, SettingsError::VersionConflict { .. }));
        assert_eq!(
            state.settings_read().favorite_models,
            ["provider:newer"],
            "the newer in-memory side write must survive"
        );
        assert_eq!(
            persisted.borrow().favorite_models,
            ["provider:newer"],
            "the newer persisted side write must survive"
        );
    }

    #[test]
    fn two_full_saves_from_the_same_client_version_cannot_both_commit() {
        let state = test_app_state();
        let first = settings_snapshot(&state);
        let second = first.clone();

        let mut first_settings = first.settings;
        first_settings.theme = "light".into();
        let committed = commit_settings_with(&state, first.version, first_settings, |_| Ok(()))
            .expect("the first full save should commit");

        let mut second_settings = second.settings;
        second_settings.target_lang = "ja".into();
        let error =
            commit_settings_with(&state, second.version.clone(), second_settings, |_| Ok(()))
                .expect_err("the queued stale full save must conflict");

        assert!(matches!(
            error,
            SettingsError::VersionConflict {
                expected_version,
                actual_version,
                ..
            } if expected_version == second.version && actual_version == committed.version
        ));
        assert_eq!(state.settings_read().theme, "light");
        assert_ne!(state.settings_read().target_lang, "ja");
    }

    #[test]
    fn settings_version_rejects_a_snapshot_from_another_process_epoch() {
        let previous_process = test_app_state();
        let restarted_process = test_app_state();
        let foreign = settings_snapshot(&previous_process);
        let current = settings_snapshot(&restarted_process);
        assert_eq!(foreign.version.revision, current.version.revision);
        assert_ne!(foreign.version.epoch, current.version.epoch);

        let error = commit_settings_with(
            &restarted_process,
            foreign.version.clone(),
            foreign.settings,
            |_| Ok(()),
        )
        .expect_err("a previous process snapshot must not be accepted");

        assert!(matches!(
            error,
            SettingsError::VersionConflict {
                expected_version,
                actual_version,
                ..
            } if expected_version == foreign.version && actual_version == current.version
        ));
    }

    use super::*;

    #[test]
    fn legacy_settings_disable_translucent_sidebar_by_default() {
        let settings: Settings =
            serde_json::from_str("{}").expect("legacy settings should deserialize");
        assert!(!settings.translucent_sidebar);
    }

    #[test]
    fn legacy_settings_keep_chat_window_alive_off_by_default() {
        let settings: Settings =
            serde_json::from_str("{}").expect("legacy settings should deserialize");
        assert!(!settings.keep_chat_window_alive);
    }

    #[test]
    fn chat_completion_notifications_are_disabled_for_new_and_legacy_settings() {
        assert!(!Settings::default().chat_completion_notifications);
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.chat_completion_notifications);
    }

    #[test]
    fn chat_completion_notifications_preserve_explicit_setting_on_save_and_reload() {
        for enabled in [false, true] {
            let settings: Settings = serde_json::from_value(serde_json::json!({
                "chatCompletionNotifications": enabled
            }))
            .unwrap();
            let saved = serde_json::to_value(&settings).unwrap();
            assert_eq!(saved["chatCompletionNotifications"], enabled);
            let reloaded: Settings = serde_json::from_value(saved).unwrap();
            assert_eq!(reloaded.chat_completion_notifications, enabled);
        }
    }

    #[test]
    fn legacy_model_info_without_temperature_deserializes_as_absent() {
        let info: ModelInfo = serde_json::from_str(
            r#"{"displayName":"Legacy","contextWindow":8192,"maxOutput":2048}"#,
        )
        .expect("legacy model info should deserialize");
        assert_eq!(info.temperature, None);
        assert_eq!(info.omit_temperature, None);
    }

    // ===== normalize_hotkey =====

    #[test]
    fn normalize_hotkey_canonicalizes_aliases() {
        // 仅规范修饰键名（cmd/ctrl/opt/super/meta），按键名 case 透传
        assert_eq!(normalize_hotkey("cmd+shift+a"), "CommandOrControl+Shift+a");
        assert_eq!(normalize_hotkey("Command+Alt+T"), "CommandOrControl+Alt+T");
        assert_eq!(normalize_hotkey("ctrl+shift+G"), "Control+Shift+G");
        assert_eq!(normalize_hotkey("opt+space"), "Alt+space");
        assert_eq!(normalize_hotkey("option+x"), "Alt+x");
        assert_eq!(normalize_hotkey("super+L"), "Super+L");
        assert_eq!(normalize_hotkey("meta+L"), "Super+L");
    }

    #[test]
    fn normalize_hotkey_trims_whitespace() {
        assert_eq!(
            normalize_hotkey(" cmd + shift + a "),
            "CommandOrControl+Shift+a"
        );
    }

    #[test]
    fn normalize_hotkey_filters_empty_parts() {
        assert_eq!(normalize_hotkey("cmd++a"), "CommandOrControl+a");
        assert_eq!(normalize_hotkey("+cmd+a+"), "CommandOrControl+a");
    }

    #[test]
    fn normalize_hotkey_preserves_unknown_keys_verbatim() {
        // F1, Backspace 等键名直接透传，不做 case 转换
        assert_eq!(normalize_hotkey("cmd+F1"), "CommandOrControl+F1");
        assert_eq!(normalize_hotkey("ctrl+Backspace"), "Control+Backspace");
    }

    // ===== sanitize_settings =====

    #[test]
    fn sanitize_settings_retires_managed_officecli_on_load_and_round_trip() {
        let mut settings = Settings::default();
        settings.chat_tools.servers = vec![
            ChatMcpServer {
                id: "plugin-officecli".into(),
                enabled: true,
                ..Default::default()
            },
            ChatMcpServer {
                id: "renamed".into(),
                connector_id: Some("plugin:officecli".into()),
                enabled: true,
                ..Default::default()
            },
            ChatMcpServer {
                id: "user-office".into(),
                command: "officecli".into(),
                enabled: true,
                ..Default::default()
            },
        ];
        let settings = sanitize_settings(settings);
        assert_eq!(settings.chat_tools.servers.len(), 1);
        assert_eq!(settings.chat_tools.servers[0].id, "user-office");
        assert!(settings.chat_tools.servers[0].enabled);
        let restored = sanitize_settings(
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap(),
        );
        assert_eq!(restored.chat_tools.servers.len(), 1);
        assert_eq!(restored.chat_tools.servers[0].id, "user-office");
    }

    #[test]
    fn sanitize_settings_clamps_retry_attempts() {
        assert_eq!(Settings::default().retry_attempts, 11);
        for (stored, expected) in [(0, 1), (5, 5), (11, 11), (99, 11)] {
            let mut settings = Settings::default();
            settings.retry_attempts = stored;
            assert_eq!(sanitize_settings(settings).retry_attempts, expected);
        }
    }

    #[test]
    fn sanitize_settings_clamps_chat_max_output_tokens() {
        let mut s = Settings::default();
        s.chat.max_output_tokens = 0;
        let s = sanitize_settings(s);
        assert_eq!(s.chat.max_output_tokens, 16_384);

        let mut s = Settings::default();
        s.chat.max_output_tokens = 32_768;
        let s = sanitize_settings(s);
        assert_eq!(s.chat.max_output_tokens, 16_384);
    }

    #[test]
    fn sanitize_settings_clamps_ui_font_scale() {
        let mut s = Settings::default();
        s.ui_font_scale = 5.0;
        assert_eq!(sanitize_settings(s).ui_font_scale, 1.4);
        let mut s = Settings::default();
        s.ui_font_scale = 0.1;
        assert_eq!(sanitize_settings(s).ui_font_scale, 0.8);
        let mut s = Settings::default();
        s.ui_font_scale = f32::NAN;
        assert_eq!(sanitize_settings(s).ui_font_scale, 1.0);
    }

    #[test]
    fn sanitize_settings_normalizes_hotkeys() {
        let mut s = Settings::default();
        s.hotkey = "cmd+alt+T".to_string();
        s.chat_hotkey = "cmd+shift+K".to_string();
        s.close_chat_hotkey = "cmd+shift+W".to_string();
        s.screenshot_translation.hotkey = "ctrl+shift+A".to_string();
        s.screenshot_translation.text_hotkey = "cmd+shift+T".to_string();
        s.lens.hotkey = "cmd+shift+G".to_string();
        let s = sanitize_settings(s);
        assert_eq!(s.hotkey, "CommandOrControl+Alt+T");
        assert_eq!(s.chat_hotkey, "CommandOrControl+Shift+K");
        assert_eq!(s.close_chat_hotkey, "CommandOrControl+Shift+W");
        assert_eq!(s.screenshot_translation.hotkey, "Control+Shift+A");
        assert_eq!(
            s.screenshot_translation.text_hotkey,
            "CommandOrControl+Shift+T"
        );
        assert_eq!(s.lens.hotkey, "CommandOrControl+Shift+G");
    }

    #[test]
    fn sanitize_settings_preserves_empty_hotkeys() {
        let mut s = Settings::default();
        s.hotkey = String::new();
        s.screenshot_translation.hotkey = String::new();
        s.screenshot_translation.text_hotkey = String::new();
        s.lens.hotkey = String::new();
        let s = sanitize_settings(s);
        assert_eq!(s.hotkey, "");
        assert_eq!(s.screenshot_translation.hotkey, "");
        assert_eq!(s.screenshot_translation.text_hotkey, "");
        assert_eq!(s.lens.hotkey, "");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    #[test]
    fn sanitize_settings_disables_system_ocr_on_unsupported_platforms() {
        let mut s = Settings::default();
        s.screenshot_translation.ocr_mode = Some(OcrMode::System);
        let s = sanitize_settings(s);
        assert_eq!(
            s.screenshot_translation.ocr_mode,
            Some(OcrMode::CloudVision)
        );
    }

    #[test]
    fn sanitize_settings_migrates_use_system_ocr_true_to_system_mode() {
        // 老版本数据：useSystemOcr=true 但没有 ocr_mode 字段
        let mut s = Settings::default();
        s.screenshot_translation.use_system_ocr = true;
        s.screenshot_translation.ocr_mode = None;
        let s = sanitize_settings(s);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert_eq!(s.screenshot_translation.ocr_mode, Some(OcrMode::System));
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(
            s.screenshot_translation.ocr_mode,
            Some(OcrMode::CloudVision)
        );
    }

    #[test]
    fn sanitize_settings_migrates_use_system_ocr_false_to_cloud_vision_mode() {
        let mut s = Settings::default();
        s.screenshot_translation.use_system_ocr = false;
        s.screenshot_translation.ocr_mode = None;
        let s = sanitize_settings(s);
        assert_eq!(
            s.screenshot_translation.ocr_mode,
            Some(OcrMode::CloudVision)
        );
    }

    #[test]
    fn rapid_ocr_tier_defaults_for_legacy_configs() {
        // 旧版 settings.json 没有 rapid_ocr_tier 字段:截图翻译默认 "standard"(现有 v5
        // mobile 用户零感知),知识库文档处理默认 "high"(入库要识别质量)。
        let screenshot: ScreenshotTranslationConfig =
            serde_json::from_str("{}").expect("empty screenshot config should load");
        assert_eq!(screenshot.rapid_ocr_tier, "standard");

        let doc_processing: DocumentProcessingConfig =
            serde_json::from_str("{}").expect("empty document processing config should load");
        assert_eq!(doc_processing.rapid_ocr_tier, "high");
    }

    #[test]
    fn sanitize_settings_normalizes_invalid_rapid_ocr_tier() {
        let mut s = Settings::default();
        s.screenshot_translation.rapid_ocr_tier = "garbage".to_string();
        s.document_processing.rapid_ocr_tier = "garbage".to_string();
        let s = sanitize_settings(s);
        assert_eq!(s.screenshot_translation.rapid_ocr_tier, "standard");
        assert_eq!(s.document_processing.rapid_ocr_tier, "high");
    }

    #[test]
    fn sanitize_settings_migrates_legacy_tesseract_to_rapidocr() {
        // 旧版本 settings.json 含 "ocrMode": "tesseract"——序列化后落到 OcrMode::Legacy
        // 兜底变体,sanitize_settings 把它迁移到 RapidOcr,避免从本地 OCR 静默变成云端视觉。
        let json = r#"{"ocrMode":"tesseract"}"#;
        let cfg: ScreenshotTranslationConfig =
            serde_json::from_str(json).expect("legacy variant should deserialize");
        assert_eq!(cfg.ocr_mode, Some(OcrMode::Legacy));

        let mut s = Settings::default();
        s.screenshot_translation.ocr_mode = Some(OcrMode::Legacy);
        let s = sanitize_settings(s);
        assert_eq!(s.screenshot_translation.ocr_mode, Some(OcrMode::RapidOcr));
    }

    #[test]
    fn ocr_mode_serializes_with_snake_case() {
        // ocrMode 在 settings.json 里是 snake_case 字符串(cloud_vision / system / rapid_ocr)。
        // 前端 union type 'cloud_vision' | 'system' | 'rapid_ocr' 直接对齐。
        let modes = [
            (OcrMode::CloudVision, "\"cloud_vision\""),
            (OcrMode::System, "\"system\""),
            (OcrMode::RapidOcr, "\"rapid_ocr\""),
        ];
        for (mode, expected) in modes {
            assert_eq!(serde_json::to_string(&mode).unwrap(), expected);
        }
    }

    #[test]
    fn sanitize_settings_removes_legacy_apple_local_provider() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "apple".to_string(),
            name: "Legacy Apple Local".to_string(),
            api_keys: vec!["__on_device__".to_string()],
            api_key_legacy: None,
            base_url: LEGACY_APPLE_INTELLIGENCE_BASE_URL.to_string(),
            available_models: vec![],
            enabled_models: vec!["apple-foundation".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "cloud".to_string(),
            name: "Cloud".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4o".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.translator_provider_id = "apple".to_string();
        s.translator_model = "apple-foundation".to_string();
        s.screenshot_translation.provider_id = "apple".to_string();
        s.screenshot_translation.model = "apple-foundation".to_string();
        s.lens.provider_id = "apple".to_string();
        s.lens.model = "apple-foundation".to_string();
        s.chat_provider_id = "apple".to_string();
        s.chat_model = "apple-foundation".to_string();
        s.default_models.chat.provider_id = "apple".to_string();
        s.default_models.chat.model = "apple-foundation".to_string();
        s.default_models.vision.provider_id = "apple".to_string();
        s.default_models.vision.model = "apple-foundation".to_string();
        s.default_models.title_summary.provider_id = "apple".to_string();
        s.default_models.title_summary.model = "apple-foundation".to_string();
        s.default_models.compression.provider_id = "apple".to_string();
        s.default_models.compression.model = "apple-foundation".to_string();
        s.default_models.image_generation.provider_id = "apple".to_string();
        s.default_models.image_generation.model = "apple-foundation".to_string();

        let s = sanitize_settings(s);
        assert!(s.providers.iter().all(|provider| provider.id != "apple"));
        assert_eq!(s.translator_provider_id, "cloud");
        assert_eq!(s.translator_model, "gpt-4o");
        assert_eq!(s.screenshot_translation.provider_id, "cloud");
        assert_eq!(s.screenshot_translation.model, "gpt-4o");
        assert_eq!(s.lens.provider_id, "");
        assert_eq!(s.lens.model, "");
        assert_eq!(s.default_models.chat.provider_id, "cloud");
        assert_eq!(s.default_models.chat.model, "gpt-4o");
        assert_eq!(s.default_models.vision.provider_id, "cloud");
        assert_eq!(s.default_models.title_summary.provider_id, "cloud");
        assert_eq!(s.default_models.compression.provider_id, "cloud");
        assert_eq!(s.default_models.image_generation.provider_id, "cloud");
    }

    #[test]
    fn sanitize_settings_migrates_legacy_apikey_to_apikeys() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec![],
            api_key_legacy: Some("sk-legacy".to_string()),
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(p.api_keys, vec!["sk-legacy".to_string()]);
        assert!(p.api_key_legacy.is_none(), "legacy field should be drained");
    }

    #[test]
    fn sanitize_settings_dedupes_apikey_legacy_against_apikeys() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["sk-1".to_string(), "sk-2".to_string()],
            api_key_legacy: Some("sk-1".to_string()), // 已在 api_keys 中
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(
            p.api_keys.len(),
            2,
            "duplicate legacy key should not be inserted"
        );
    }

    #[test]
    fn sanitize_settings_filters_empty_apikeys() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["sk-1".to_string(), "  ".to_string(), String::new()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(p.api_keys, vec!["sk-1".to_string()]);
    }

    #[test]
    fn sanitize_settings_remaps_active_key_index_when_empty_keys_dropped() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec![
                "  ".to_string(),
                "sk-keep".to_string(),
                "sk-current".to_string(),
            ],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 2,
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(
            p.api_keys,
            vec!["sk-keep".to_string(), "sk-current".to_string()]
        );
        assert_eq!(p.active_key_index, 1);
    }

    #[test]
    fn sanitize_settings_clamps_active_key_index_past_end() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["sk-a".to_string(), "sk-b".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 9,
        });
        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert_eq!(p.active_key_index, 1);
    }

    #[test]
    fn legacy_tool_round_limits_are_ignored_and_removed_on_save() {
        for limit in [
            serde_json::Value::Null,
            serde_json::json!(1),
            serde_json::json!(10),
            serde_json::json!(100),
        ] {
            let settings: Settings = serde_json::from_value(serde_json::json!({
                "chatTools": {"maxToolRounds": limit, "maxToolOutputChars": 12000},
                "toolRoundsUnlimitedMigrated": true
            }))
            .unwrap();
            let saved = serde_json::to_value(sanitize_settings(settings)).unwrap();
            assert!(saved["chatTools"].get("maxToolRounds").is_none());
            assert!(saved.get("toolRoundsUnlimitedMigrated").is_none());
            assert_eq!(saved["chatTools"]["maxToolOutputChars"], 12000);
        }
        let cfg: ChatToolsConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(
            cfg.max_tool_output_chars,
            Some(DEFAULT_MAX_TOOL_OUTPUT_CHARS)
        );
    }

    #[test]
    fn greenlight_migration_enables_tools_without_weakening_approval() {
        // Simulate a pre-green-light install: flag unset, native tools off, old policy.
        let mut settings = Settings::default();
        settings.chat_tools_greenlit_v1 = false;
        settings.chat_tools.native_tools = ChatNativeToolsConfig {
            skill_runtime: true,
            ..Default::default()
        };
        settings.chat_tools.native_tools.read_file = false;
        settings.chat_tools.native_tools.write_file = false;
        settings.chat_tools.native_tools.run_command = false;
        settings.chat_tools.approval_policy = LEGACY_DEFAULT_APPROVAL_POLICY.to_string();

        let settings = sanitize_settings(settings);

        assert!(settings.chat_tools_greenlit_v1);
        assert!(settings.chat_tools.native_tools.read_file);
        assert!(settings.chat_tools.native_tools.write_file);
        assert!(settings.chat_tools.native_tools.run_command);
        assert_eq!(
            settings.chat_tools.approval_policy,
            LEGACY_DEFAULT_APPROVAL_POLICY
        );
        assert_eq!(ChatToolsConfig::default().approval_policy, "always_confirm");
    }

    #[test]
    fn greenlight_migration_is_idempotent_and_keeps_explicit_choices() {
        // Already migrated: a user-disabled tool and an explicit policy must survive.
        let mut settings = Settings::default();
        settings.chat_tools_greenlit_v1 = true;
        settings.chat_tools.native_tools.run_command = false;
        settings.chat_tools.approval_policy = "always_confirm".to_string();

        let settings = sanitize_settings(settings);

        assert!(!settings.chat_tools.native_tools.run_command);
        assert_eq!(settings.chat_tools.approval_policy, "always_confirm");
    }

    #[test]
    fn sanitize_settings_migrates_prompt_cache_retention() {
        let mut settings = Settings::default();
        settings.providers = vec![ModelProvider {
            id: "p1".into(),
            name: "P".into(),
            api_keys: vec!["k".into()],
            api_key_legacy: None,
            base_url: "https://api.openai.com/v1".into(),
            available_models: vec![],
            enabled_models: vec![],
            enabled: true,
            api_format: "openai_chat".into(),
            model_overrides: Default::default(),
            compress_request_body: false,
            request: ProviderRequestConfig {
                prompt_caching: Some(false),
                prompt_cache_retention: "garbage".into(),
                ..Default::default()
            },
            active_key_index: 0,
        }];
        // 非法 retention + bool false → none
        let s = sanitize_settings(settings.clone());
        assert_eq!(s.providers[0].request.prompt_cache_retention, "none");
        assert!(s.providers[0].request.prompt_caching.is_none());

        // 合法 retention 优先：false + long → 保留 long
        settings.providers[0].request.prompt_caching = Some(false);
        settings.providers[0].request.prompt_cache_retention = "long".into();
        let s = sanitize_settings(settings.clone());
        assert_eq!(s.providers[0].request.prompt_cache_retention, "long");

        // true + 非法 → short
        settings.providers[0].request.prompt_caching = Some(true);
        settings.providers[0].request.prompt_cache_retention = "???".into();
        let s = sanitize_settings(settings.clone());
        assert_eq!(s.providers[0].request.prompt_cache_retention, "short");

        // 无 bool、非法 → short
        settings.providers[0].request.prompt_caching = None;
        settings.providers[0].request.prompt_cache_retention = "".into();
        let s = sanitize_settings(settings);
        assert_eq!(s.providers[0].request.prompt_cache_retention, "short");
    }

    #[test]
    fn hook_def_wire_shape_matches_the_frontend_type() {
        // 前端 `HookDef`（src/api/tauri.ts）逐字段镜像这个结构。字段名/大小写漂移了，
        // 保存时会静默丢字段（serde default 兜住，用户只看到「配了但没生效」）。
        let hook = HookDef {
            id: "h1".to_string(),
            name: "lint-guard".to_string(),
            description: "d".to_string(),
            event: "tool_execution_start".to_string(),
            enabled: true,
            kind: "http".to_string(),
            script: String::new(),
            url: "https://example.test/h".to_string(),
            method: "POST".to_string(),
            headers: [("X-A".to_string(), "1".to_string())].into_iter().collect(),
            timeout_ms: 60_000,
        };
        let value = serde_json::to_value(&hook).expect("serialize");
        let keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            vec![
                "description",
                "enabled",
                "event",
                "headers",
                "id",
                "method",
                "name",
                "script",
                "timeoutMs",
                "type",
                "url",
            ],
            "wire field names drifted from the TS HookDef"
        );

        // 前端写回的 JSON 也必须原样读回来。
        let round_tripped: HookDef = serde_json::from_value(value).expect("deserialize");
        assert_eq!(round_tripped.kind, "http");
        assert_eq!(round_tripped.timeout_ms, 60_000);
        assert_eq!(
            round_tripped.headers.get("X-A").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn sanitize_hooks_drops_invalid_entries_and_clamps_timeout() {
        let mut settings = Settings::default();
        settings.chat_tools.hooks = vec![
            // 合法 command：补 id、钳制超时下限。
            HookDef {
                name: "log".to_string(),
                event: " agent_end ".to_string(),
                enabled: true,
                kind: "Command".to_string(),
                script: "echo hi".to_string(),
                timeout_ms: 1,
                ..Default::default()
            },
            // 合法 http：方法归一到大写。
            HookDef {
                id: "http-1".to_string(),
                event: "turn_start".to_string(),
                kind: "http".to_string(),
                url: "https://example.test/hook".to_string(),
                method: "post".to_string(),
                timeout_ms: 10_000_000,
                ..Default::default()
            },
            // 事件名非法 → 丢弃（没有合理的「就近」事件可猜）。
            HookDef {
                event: "agent_started".to_string(),
                kind: "command".to_string(),
                script: "echo x".to_string(),
                ..Default::default()
            },
            // 类型非法 → 丢弃。
            HookDef {
                event: "agent_end".to_string(),
                kind: "webhook".to_string(),
                ..Default::default()
            },
            // command 但脚本为空 / http 但 url 为空 → 丢弃。
            HookDef {
                event: "agent_end".to_string(),
                kind: "command".to_string(),
                script: "   ".to_string(),
                ..Default::default()
            },
            HookDef {
                event: "agent_end".to_string(),
                kind: "http".to_string(),
                ..Default::default()
            },
        ];

        let settings = sanitize_settings(settings);
        let hooks = &settings.chat_tools.hooks;

        assert_eq!(hooks.len(), 2, "only the two valid hooks survive");
        assert_eq!(hooks[0].event, "agent_end", "event is trimmed");
        assert_eq!(hooks[0].kind, "command", "kind is lowercased");
        assert!(!hooks[0].id.is_empty(), "blank id is filled in");
        assert_eq!(hooks[0].timeout_ms, HOOK_MIN_TIMEOUT_MS);
        assert_eq!(hooks[1].method, "POST", "method is uppercased");
        assert_eq!(hooks[1].timeout_ms, HOOK_MAX_TIMEOUT_MS);
    }

    #[test]
    fn greenlight_migration_does_not_stomp_explicit_policy_on_first_run() {
        for policy in ["always_confirm", "auto"] {
            let mut settings = Settings::default();
            settings.chat_tools_greenlit_v1 = false;
            settings.chat_tools.approval_policy = policy.into();
            let settings = sanitize_settings(settings);
            assert!(settings.chat_tools_greenlit_v1);
            assert!(settings.chat_tools.native_tools.read_file);
            assert_eq!(settings.chat_tools.approval_policy, policy);
        }
    }

    #[test]
    fn sanitize_settings_normalizes_and_clamps_tool_output_chars() {
        // None（旧的"不截断"）→ 归一到默认截断值，绝不再保留 None（上下文撑爆根因）。
        let mut settings = Settings::default();
        settings.chat_tools.max_tool_output_chars = None;
        let settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.max_tool_output_chars,
            Some(DEFAULT_MAX_TOOL_OUTPUT_CHARS)
        );

        // 过小钳到下限。
        let mut settings = Settings::default();
        settings.chat_tools.max_tool_output_chars = Some(1);
        let settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.max_tool_output_chars,
            Some(CHAT_TOOL_MIN_OUTPUT_CHARS)
        );

        // 过大钳到上限。
        let mut settings = Settings::default();
        settings.chat_tools.max_tool_output_chars = Some(usize::MAX);
        let settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.max_tool_output_chars,
            Some(CHAT_TOOL_MAX_OUTPUT_CHARS)
        );
    }

    #[test]
    fn sanitize_settings_clamps_mcp_idle_timeout_and_keeps_default() {
        // 默认值保持不变（在范围内）。
        assert_eq!(ChatToolsConfig::default().mcp_idle_timeout_ms, 600_000);

        // 太小钳到下限 60s。
        let mut settings = Settings::default();
        settings.chat_tools.mcp_idle_timeout_ms = 1_000;
        let settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.mcp_idle_timeout_ms,
            MCP_IDLE_TIMEOUT_MIN_MS
        );

        // 太大钳到上限 24h。
        let mut settings = Settings::default();
        settings.chat_tools.mcp_idle_timeout_ms = u64::MAX;
        let settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.mcp_idle_timeout_ms,
            MCP_IDLE_TIMEOUT_MAX_MS
        );

        // 缺省（旧 settings.json 无此字段）走 serde default 600000。
        let cfg: ChatToolsConfig =
            serde_json::from_str("{}").expect("ChatToolsConfig defaults from empty object");
        assert_eq!(cfg.mcp_idle_timeout_ms, 600_000);
    }

    #[test]
    fn sanitize_settings_keeps_empty_models_for_unfetched_provider() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "p".to_string(),
            name: "P".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec![],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.translator_provider_id = "p".to_string();
        s.screenshot_translation.provider_id = "p".to_string();

        let s = sanitize_settings(s);
        let p = s.get_provider("p").unwrap();
        assert!(p.available_models.is_empty());
        assert!(p.enabled_models.is_empty());
        assert!(s.translator_model.is_empty());
        assert!(s.screenshot_translation.model.is_empty());
    }

    #[test]
    fn unset_auxiliary_models_inherit_effective_chat_model() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "translator".to_string(),
            name: "Translator".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4o".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "lens".to_string(),
            name: "Lens".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["vision-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.translator_provider_id = "translator".to_string();
        s.translator_model = "gpt-4o".to_string();
        s.lens.provider_id = "lens".to_string();
        s.lens.model = "vision-model".to_string();

        let s = sanitize_settings(s);

        assert_eq!(
            s.effective_chat_model(),
            ("lens".to_string(), "vision-model".to_string())
        );
        assert!(!s.has_explicit_vision_model());
        assert_eq!(s.effective_vision_model(), s.effective_chat_model());
        assert_eq!(
            s.effective_title_summary_model_for_session(None),
            s.effective_chat_model()
        );
        assert_eq!(
            s.effective_compression_model_for_session(None),
            s.effective_chat_model()
        );
        assert_eq!(
            s.effective_prompt_optimize_model_for_session(None),
            s.effective_chat_model()
        );
        assert!(s.image_generation_model().is_none());
        assert!(s.default_models.vision.provider_id.is_empty());
        assert!(s.default_models.title_summary.provider_id.is_empty());
        assert!(s.default_models.compression.provider_id.is_empty());
        assert!(s.default_models.image_generation.provider_id.is_empty());
    }

    #[test]
    fn effective_side_models_auto_prefer_session_over_global_chat_default() {
        let mut settings = Settings::default();
        settings.providers.push(ModelProvider {
            id: "global".to_string(),
            name: "Global".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gemini-3.1-flash-lite".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        settings.providers.push(ModelProvider {
            id: "session".to_string(),
            name: "Session".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4.1".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        settings.default_models.chat.provider_id = "global".to_string();
        settings.default_models.chat.model = "gemini-3.1-flash-lite".to_string();

        let session = SessionModel {
            provider_id: "session",
            model: "gpt-4.1",
        };

        assert_eq!(
            settings.effective_title_summary_model_for_session(Some(session)),
            ("session".to_string(), "gpt-4.1".to_string())
        );
        assert_eq!(
            settings.effective_compression_model_for_session(Some(session)),
            ("session".to_string(), "gpt-4.1".to_string())
        );
        assert_eq!(
            settings.effective_vision_model_for_session(Some(session)),
            ("session".to_string(), "gpt-4.1".to_string())
        );
        assert_eq!(
            settings.effective_prompt_optimize_model_for_session(Some(session)),
            ("session".to_string(), "gpt-4.1".to_string())
        );
    }

    #[test]
    fn sanitize_settings_keeps_valid_chat_model() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "chat".to_string(),
            name: "Chat".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m1".to_string(), "m2".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.chat_provider_id = "chat".to_string();
        s.chat_model = "m2".to_string();

        let s = sanitize_settings(s);
        assert_eq!(s.chat_provider_id, "chat");
        assert_eq!(s.chat_model, "m2");
        assert_eq!(s.default_models.chat.provider_id, "chat");
        assert_eq!(s.default_models.chat.model, "m2");
    }

    #[test]
    fn explicit_default_model_slots_are_independent() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "chat".to_string(),
            name: "Chat".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["chat-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "vision".to_string(),
            name: "Vision".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["vision-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "title".to_string(),
            name: "Title".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["title-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "compression".to_string(),
            name: "Compression".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["compression-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "image".to_string(),
            name: "Image".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["image-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.translator_provider_id = "chat".to_string();
        s.translator_model = "chat-model".to_string();
        s.default_models.chat.provider_id = "chat".to_string();
        s.default_models.chat.model = "chat-model".to_string();
        s.default_models.vision.provider_id = "vision".to_string();
        s.default_models.vision.model = "vision-model".to_string();
        s.default_models.title_summary.provider_id = "title".to_string();
        s.default_models.title_summary.model = "title-model".to_string();
        s.default_models.compression.provider_id = "compression".to_string();
        s.default_models.compression.model = "compression-model".to_string();
        s.default_models.image_generation.provider_id = "image".to_string();
        s.default_models.image_generation.model = "image-model".to_string();

        let s = sanitize_settings(s);

        assert_eq!(
            s.effective_chat_model(),
            ("chat".to_string(), "chat-model".to_string())
        );
        assert_eq!(
            s.effective_title_summary_model_for_session(None),
            ("title".to_string(), "title-model".to_string())
        );
        assert!(s.has_explicit_vision_model());
        assert_eq!(
            s.effective_vision_model(),
            ("vision".to_string(), "vision-model".to_string())
        );
        assert_eq!(
            s.effective_compression_model_for_session(None),
            ("compression".to_string(), "compression-model".to_string())
        );
        assert_eq!(
            s.effective_prompt_optimize_model_for_session(None),
            s.effective_chat_model()
        );
        assert_eq!(
            s.image_generation_model(),
            Some(("image".to_string(), "image-model".to_string()))
        );
    }

    #[test]
    fn sanitize_settings_repairs_invalid_default_model_slots() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "chat".to_string(),
            name: "Chat".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m1".to_string(), "m2".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.translator_provider_id = "chat".to_string();
        s.translator_model = "m1".to_string();
        s.default_models.chat.provider_id = "chat".to_string();
        s.default_models.chat.model = "removed".to_string();
        s.default_models.vision.provider_id = "chat".to_string();
        s.default_models.vision.model = String::new();
        s.default_models.title_summary.provider_id = "deleted-provider".to_string();
        s.default_models.title_summary.model = "ghost".to_string();
        s.default_models.compression.provider_id = "chat".to_string();
        s.default_models.compression.model = String::new();
        s.default_models.image_generation.provider_id = "chat".to_string();
        s.default_models.image_generation.model = String::new();

        let s = sanitize_settings(s);

        assert_eq!(s.default_models.chat.provider_id, "chat");
        assert_eq!(s.default_models.chat.model, "m1");
        assert_eq!(s.default_models.vision.provider_id, "chat");
        assert_eq!(s.default_models.vision.model, "m1");
        assert!(s.default_models.title_summary.provider_id.is_empty());
        assert!(s.default_models.title_summary.model.is_empty());
        assert_eq!(s.default_models.compression.provider_id, "chat");
        assert_eq!(s.default_models.compression.model, "m1");
        assert_eq!(s.default_models.image_generation.provider_id, "chat");
        assert_eq!(s.default_models.image_generation.model, "m1");
        assert_eq!(s.chat_provider_id, "chat");
        assert_eq!(s.chat_model, "m1");
    }

    #[test]
    fn persistence_mirror_keeps_unset_chat_default_unset() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "translator".to_string(),
            name: "Translator".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["gpt-4o".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "lens".to_string(),
            name: "Lens".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["vision-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.translator_provider_id = "translator".to_string();
        s.translator_model = "gpt-4o".to_string();
        s.lens.provider_id = "lens".to_string();
        s.lens.model = "vision-model".to_string();

        let mut s = sanitize_settings(s);
        assert_eq!(s.chat_provider_id, "lens");
        assert_eq!(s.chat_model, "vision-model");
        assert!(s.default_models.chat.provider_id.is_empty());

        mirror_explicit_chat_default_for_persistence(&mut s);

        assert!(s.chat_provider_id.is_empty());
        assert!(s.chat_model.is_empty());
        assert!(s.default_models.chat.provider_id.is_empty());
    }

    #[test]
    fn persistence_projection_canonicalizes_before_adding_legacy_mirrors() {
        let mut raw = Settings::default();
        raw.theme = "sepia".to_string();
        raw.theme_color = "mint".to_string();
        raw.retry_attempts = 0;
        raw.screenshot_translation.ocr_mode = Some(OcrMode::RapidOcr);
        raw.screenshot_translation.use_system_ocr = true;

        let (canonical, persisted) = settings_for_persistence(&raw);

        assert_eq!(canonical.theme, "system");
        assert_eq!(canonical.theme_color, "neutral");
        assert_eq!(canonical.retry_attempts, 1);
        assert_eq!(
            canonical.screenshot_translation.ocr_mode,
            Some(OcrMode::RapidOcr)
        );
        assert!(!persisted.screenshot_translation.use_system_ocr);
        assert_eq!(
            persisted.screenshot_translation.ocr_mode,
            Some(OcrMode::RapidOcr)
        );
    }

    #[test]
    fn default_models_serialize_as_structured_camel_case_settings() {
        let mut s = Settings::default();
        s.default_models.vision.provider_id = "vision-provider".to_string();
        s.default_models.vision.model = "vision-model".to_string();
        s.default_models.title_summary.provider_id = "title-provider".to_string();
        s.default_models.title_summary.model = "title-model".to_string();
        s.default_models.image_generation.provider_id = "image-provider".to_string();
        s.default_models.image_generation.model = "image-model".to_string();
        let value = serde_json::to_value(&s).expect("settings should serialize");

        assert_eq!(
            value["defaultModels"]["vision"]["providerId"],
            "vision-provider"
        );
        assert_eq!(value["defaultModels"]["vision"]["model"], "vision-model");
        assert_eq!(
            value["defaultModels"]["titleSummary"]["providerId"],
            "title-provider"
        );
        assert_eq!(
            value["defaultModels"]["titleSummary"]["model"],
            "title-model"
        );
        assert_eq!(
            value["defaultModels"]["imageGeneration"]["providerId"],
            "image-provider"
        );
        assert_eq!(
            value["defaultModels"]["imageGeneration"]["model"],
            "image-model"
        );
        assert_eq!(value["defaultModels"]["promptOptimize"]["providerId"], "");
        assert_eq!(value["defaultModels"]["promptOptimize"]["model"], "");
        assert!(
            value["defaultModels"]["chat"]["providerId"]
                .as_str()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn sanitize_settings_preserves_streamable_http_mcp_server() {
        let mut s = Settings::default();
        let mut headers = std::collections::HashMap::new();
        headers.insert(" Authorization ".to_string(), " Bearer token ".to_string());
        s.chat_tools.servers.push(ChatMcpServer {
            id: " http-server ".to_string(),
            name: " Remote ".to_string(),
            enabled: true,
            transport: "sse".to_string(),
            url: " https://example.com/mcp ".to_string(),
            command: " ignored ".to_string(),
            args: vec![" ".to_string(), "--unused".to_string()],
            env: std::collections::HashMap::new(),
            headers,
            cwd: None,
            enabled_tools: vec![" fetch ".to_string(), "".to_string()],
            connector_id: None,
            auth: None,
        });

        let s = sanitize_settings(s);
        let server = &s.chat_tools.servers[0];
        assert_eq!(server.id, "http-server");
        assert_eq!(server.name, "Remote");
        assert_eq!(server.transport, "streamable_http");
        assert_eq!(server.url, "https://example.com/mcp");
        assert_eq!(
            server.headers.get("Authorization").map(String::as_str),
            Some("Bearer token"),
        );
        assert_eq!(server.enabled_tools, vec!["fetch".to_string()]);
    }

    #[test]
    fn sanitize_settings_resets_unknown_mcp_transport_to_stdio() {
        let mut s = Settings::default();
        s.chat_tools.servers.push(ChatMcpServer {
            id: "mcp-1".to_string(),
            name: "Local".to_string(),
            enabled: false,
            transport: "websocket".to_string(),
            url: String::new(),
            command: " npx ".to_string(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            cwd: None,
            enabled_tools: Vec::new(),
            connector_id: None,
            auth: None,
        });

        let s = sanitize_settings(s);
        let server = &s.chat_tools.servers[0];
        assert_eq!(server.transport, "stdio");
        assert_eq!(server.command, "npx");
    }

    #[test]
    fn sanitize_settings_retires_cua_mcp_and_preserves_skill_choice() {
        for (id, connector) in [
            ("plugin-cua-driver", None),
            ("computer-control-cua-driver", None),
            ("custom-id", Some("plugin:cua-driver")),
            ("custom-id", Some("computer-control:cua")),
        ] {
            for enabled in [false, true] {
                for skill_disabled in [false, true] {
                    let mut s = Settings::default();
                    if skill_disabled {
                        s.chat_tools
                            .disabled_skill_ids
                            .push("cua-driver".to_string());
                    }
                    s.chat_tools.servers = vec![
                        ChatMcpServer {
                            id: id.to_string(),
                            enabled,
                            connector_id: connector.map(str::to_string),
                            command: "cua-driver".to_string(),
                            args: vec!["mcp".to_string()],
                            ..Default::default()
                        },
                        ChatMcpServer {
                            id: "other".to_string(),
                            ..Default::default()
                        },
                    ];
                    let s = sanitize_settings(s);
                    assert_eq!(s.chat_tools.servers.len(), 1);
                    assert_eq!(s.chat_tools.servers[0].id, "other");
                    assert_eq!(
                        s.chat_tools
                            .disabled_skill_ids
                            .contains(&"cua-driver".to_string()),
                        skill_disabled || !enabled
                    );
                    let saved = serde_json::to_value(&s).unwrap();
                    let reopened =
                        sanitize_settings(serde_json::from_value(saved.clone()).unwrap());
                    assert_eq!(serde_json::to_value(reopened).unwrap(), saved);
                }
            }
        }
    }

    #[test]
    fn sanitize_settings_clamps_unknown_message_order() {
        let mut s = Settings::default();
        s.lens.message_order = "garbage".to_string();
        let s = sanitize_settings(s);
        assert_eq!(s.lens.message_order, "asc");
    }

    #[test]
    fn lens_capture_hint_defaults_to_enabled() {
        let s = Settings::default();
        assert!(s.lens.show_capture_hint);

        let cfg: LensConfig = serde_json::from_str("{}").expect("empty lens config should load");
        assert!(cfg.show_capture_hint);
    }

    #[test]
    fn lens_send_to_chat_defaults_to_enabled() {
        let s = Settings::default();
        assert!(s.lens.send_to_chat);

        let cfg: LensConfig = serde_json::from_str("{}").expect("empty lens config should load");
        assert!(cfg.send_to_chat);
    }

    #[test]
    fn sanitize_settings_resets_lens_provider_when_pointing_to_nonexistent() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "real".to_string(),
            name: "Real".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://api.example.com/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["m".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.lens.provider_id = "nonexistent".to_string();
        s.lens.model = "ghost-model".to_string();
        let s = sanitize_settings(s);
        assert_eq!(s.lens.provider_id, "");
        assert_eq!(s.lens.model, "");
    }

    #[test]
    fn sanitize_settings_marks_onboarding_completed_for_anonymous_provider_config() {
        let mut s = Settings::default();
        s.onboarding_status.clear();
        s.providers.push(ModelProvider {
            id: "active".to_string(),
            name: "Active".to_string(),
            api_keys: Vec::new(),
            api_key_legacy: None,
            base_url: "https://active.example/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["live-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        let s = sanitize_settings(s);
        assert_eq!(s.onboarding_status, "completed");
    }

    #[test]
    fn sanitize_settings_keeps_pending_onboarding_for_fresh_install() {
        let s = sanitize_settings(Settings::default());
        assert_eq!(s.onboarding_status, "pending");
    }

    #[test]
    fn sanitize_settings_keeps_explicit_pending_for_restart_onboarding() {
        let mut s = Settings::default();
        s.onboarding_status = "pending".to_string();
        s.providers.push(ModelProvider {
            id: "active".to_string(),
            name: "Active".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://active.example/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["live-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        let s = sanitize_settings(s);
        assert_eq!(s.onboarding_status, "pending");
    }

    #[test]
    fn sanitize_settings_reassigns_disabled_provider_selections() {
        let mut s = Settings::default();
        s.providers.push(ModelProvider {
            id: "disabled".to_string(),
            name: "Disabled".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://disabled.example/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["off-model".to_string()],
            api_format: "openai".to_string(),
            enabled: false,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.providers.push(ModelProvider {
            id: "active".to_string(),
            name: "Active".to_string(),
            api_keys: vec!["sk".to_string()],
            api_key_legacy: None,
            base_url: "https://active.example/v1".to_string(),
            available_models: vec![],
            enabled_models: vec!["live-model".to_string()],
            api_format: "openai".to_string(),
            enabled: true,
            model_overrides: std::collections::HashMap::new(),
            compress_request_body: false,
            request: Default::default(),
            active_key_index: 0,
        });
        s.translator_provider_id = "disabled".to_string();
        s.translator_model = "off-model".to_string();
        s.screenshot_translation.provider_id = "disabled".to_string();
        s.screenshot_translation.model = "off-model".to_string();
        s.lens.provider_id = "disabled".to_string();
        s.lens.model = "off-model".to_string();
        s.default_models.chat.provider_id = "disabled".to_string();
        s.default_models.chat.model = "off-model".to_string();

        let s = sanitize_settings(s);

        assert_eq!(s.translator_provider_id, "active");
        assert_eq!(s.translator_model, "live-model");
        assert_eq!(s.screenshot_translation.provider_id, "active");
        assert_eq!(s.screenshot_translation.model, "live-model");
        assert_eq!(s.lens.provider_id, "");
        assert_eq!(s.lens.model, "");
        assert_eq!(s.default_models.chat.provider_id, "");
        assert_eq!(s.default_models.chat.model, "");
    }

    #[test]
    fn chat_current_datetime_context_uses_local_clock() {
        let now = Local::now();
        let zh = chat_current_datetime_context("zh");
        assert!(zh.contains("系统时钟"));
        assert!(zh.contains(&format!("{}年", now.year())));
        let en = chat_current_datetime_context("en");
        assert!(en.contains("system clock"));
        assert!(en.contains(&format!("{}-", now.year())));
    }

    #[test]
    fn chat_current_datetime_context_is_date_only_prefix_stable() {
        // 前缀稳定性：不含时分（HH:MM 会让同一对话每轮系统提示词都变，打穿 prompt cache）。
        // 同一天内多次调用必须逐字节一致。
        let has_hh_mm = |s: &str| {
            s.as_bytes().windows(5).any(|w| {
                w[0].is_ascii_digit()
                    && w[1].is_ascii_digit()
                    && w[2] == b':'
                    && w[3].is_ascii_digit()
                    && w[4].is_ascii_digit()
            })
        };
        for lang in ["zh", "en"] {
            let a = chat_current_datetime_context(lang);
            let b = chat_current_datetime_context(lang);
            assert_eq!(a, b, "same-day calls must be byte-identical ({lang})");
            assert!(
                !has_hh_mm(&a),
                "no HH:MM clock in the prompt prefix ({lang}): {a}"
            );
        }
    }

    #[test]
    fn standalone_obsidian_skills_follow_normal_enable_rules() {
        let mut legacy = serde_json::to_value(Settings::default()).unwrap();
        legacy["obsidianVaultPath"] = serde_json::json!("/old/vault");
        let restored: Settings = serde_json::from_value(legacy).unwrap();
        assert!(
            serde_json::to_value(&restored)
                .unwrap()
                .get("obsidianVaultPath")
                .is_none()
        );
        let mut config = restored.chat_tools;
        assert!(skill_globally_available(&config, "obsidian-markdown"));
        assert_eq!(
            skill_global_unavailable_error(&config, "obsidian-markdown", "Obsidian"),
            None
        );
        config.disabled_skill_ids = vec!["obsidian-markdown".into()];
        assert!(!skill_globally_available(&config, "obsidian-markdown"));
        assert_eq!(
            skill_global_unavailable_error(&config, "obsidian-markdown", "Obsidian").as_deref(),
            Some("Skill is disabled in Settings: Obsidian")
        );
    }

    #[test]
    fn sanitize_native_tools_migrates_first_legacy_workspace_root() {
        let mut settings = Settings::default();
        settings.chat_tools.native_tools.working_directory.clear();
        settings.chat_tools.native_tools.workspace_roots = vec![
            "  C:/legacy/workspace  ".to_string(),
            "C:/ignored".to_string(),
        ];
        let settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.native_tools.working_directory,
            "C:/legacy/workspace"
        );
        assert!(settings.chat_tools.native_tools.workspace_roots.is_empty());
    }

    #[test]
    fn sanitize_native_tools_uses_platform_default_when_directory_is_missing() {
        let mut settings = Settings::default();
        settings.chat_tools.native_tools.working_directory.clear();
        settings.chat_tools.native_tools.workspace_roots.clear();
        let settings = sanitize_settings(settings);
        assert_eq!(
            settings.chat_tools.native_tools.working_directory,
            default_chat_working_directory()
        );
    }
}

#[cfg(test)]
mod hooks_disk_compat_tests {
    use super::*;

    /// 回归：hooks 字段刚上线时前端漏传，`invoke` 把它序列化成 null 写进了
    /// settings.json。serde 的 `default` 只兜「键不存在」，遇到 null 会报
    /// `invalid type: null, expected a sequence` —— 整个 chatTools 解析失败，
    /// 连 MCP 服务器和原生工具开关一起丢。
    #[test]
    fn null_hooks_on_disk_does_not_break_chat_tools() {
        let json = serde_json::json!({
            "enabled": true,
            "hooks": null,
            "servers": [{ "id": "s1", "name": "srv" }],
        });
        let config: ChatToolsConfig =
            serde_json::from_value(json).expect("null hooks must not break chatTools");
        assert!(config.hooks.is_empty());
        assert!(config.enabled, "sibling fields must survive");
        assert_eq!(config.servers.len(), 1, "sibling fields must survive");
    }

    /// 键完全不存在（更旧的磁盘文件）同样要能读。
    #[test]
    fn missing_hooks_key_defaults_to_empty() {
        let config: ChatToolsConfig =
            serde_json::from_value(serde_json::json!({ "enabled": true }))
                .expect("missing hooks key must parse");
        assert!(config.hooks.is_empty());
    }
}

#[cfg(test)]
mod external_cli_materialize_gate_tests {
    use super::*;

    fn with_provider(current: &str) -> Settings {
        let mut settings = Settings::default();
        settings.chat.external_cli_agents.insert(
            "grok".to_string(),
            ExternalCliAgentConfig {
                current_provider: current.to_string(),
                providers: vec![ExternalCliProvider {
                    id: "relay".to_string(),
                    name: "Relay".to_string(),
                    config_toml: "[models]\nx = 1".to_string(),
                    ..ExternalCliProvider::default()
                }],
                ..ExternalCliAgentConfig::default()
            },
        );
        settings
    }

    /// 回归：`persist_settings` 是所有设置写盘的总出口。收藏模型 / 热键 / 主题这类
    /// 与外部 CLI 无关的保存，不得触发 `materialize_all` 重写 `~/.grok/config.toml`
    /// 之类的用户家目录文件。
    #[test]
    fn unrelated_settings_change_does_not_rematerialize() {
        let previous = with_provider("relay");
        let mut next = previous.clone();
        next.favorite_models.push("p:m".to_string());
        next.hotkey = "Alt+Shift+Z".to_string();
        assert!(!external_cli_agents_changed(&previous, &next));
    }

    #[test]
    fn switching_provider_rematerializes() {
        let previous = with_provider("relay");
        let next = with_provider("");
        assert!(external_cli_agents_changed(&previous, &next));
    }

    #[test]
    fn editing_provider_body_rematerializes() {
        let previous = with_provider("relay");
        let mut next = previous.clone();
        next.chat
            .external_cli_agents
            .get_mut("grok")
            .unwrap()
            .providers[0]
            .config_toml = "[models]\nx = 2".to_string();
        assert!(external_cli_agents_changed(&previous, &next));
    }

    #[test]
    fn adding_or_removing_agent_entry_rematerializes() {
        let previous = Settings::default();
        let next = with_provider("relay");
        assert!(external_cli_agents_changed(&previous, &next));
        assert!(external_cli_agents_changed(&next, &previous));
    }
}

#[cfg(test)]
mod custom_theme_tests {
    use super::*;

    fn filled_palette(hex: &str) -> ThemePalette {
        let hex = hex.to_string();
        ThemePalette {
            surface: hex.clone(),
            surface_soft: hex.clone(),
            surface_muted: hex.clone(),
            surface_hover: hex.clone(),
            surface_active: hex.clone(),
            surface_titlebar: hex.clone(),
            border: hex.clone(),
            border_strong: hex.clone(),
            text: hex.clone(),
            text_muted: hex.clone(),
            text_faint: hex.clone(),
            accent: hex.clone(),
            accent_hover: hex.clone(),
            accent_soft: hex.clone(),
            on_accent: hex.clone(),
            danger: hex.clone(),
            danger_soft: hex,
        }
    }

    fn numbered_palette(start: u8) -> ThemePalette {
        let mut n = start;
        let mut next = || {
            let color = format!("#{n:02x}{n:02x}{n:02x}");
            n += 1;
            color
        };
        ThemePalette {
            surface: next(),
            surface_soft: next(),
            surface_muted: next(),
            surface_hover: next(),
            surface_active: next(),
            surface_titlebar: next(),
            border: next(),
            border_strong: next(),
            text: next(),
            text_muted: next(),
            text_faint: next(),
            accent: next(),
            accent_hover: next(),
            accent_soft: next(),
            on_accent: next(),
            danger: next(),
            danger_soft: next(),
        }
    }

    /// 36-char UUID, the only id shape the UI generates via `crypto.randomUUID()`.
    fn uid(n: u32) -> String {
        format!("00000000-0000-4000-8000-{n:012x}")
    }

    fn theme(id: &str, name: &str) -> ThemeDefinition {
        ThemeDefinition {
            id: id.to_string(),
            name: name.to_string(),
            light: filled_palette("#112233"),
            dark: filled_palette("#445566"),
        }
    }

    fn legacy_body() -> serde_json::Value {
        serde_json::json!({
            "hotkey": "Alt+T",
            "theme": "dark",
            "themeColor": "cool",
            "translucentSidebar": true,
            "favoriteModels": ["prov:model"],
            "retryAttempts": 6,
            "providers": [{
                "id": "keep-me",
                "name": "Keep",
                "baseUrl": "https://example.test/v1",
                "apiKeys": ["sk-test"]
            }]
        })
    }

    fn assert_legacy_fields(settings: &Settings) {
        assert_eq!(settings.hotkey, "Alt+T");
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.theme_color, "cool");
        assert!(settings.translucent_sidebar);
        assert_eq!(settings.favorite_models, vec!["prov:model".to_string()]);
        assert_eq!(settings.retry_attempts, 6);
        assert_eq!(settings.providers.len(), 1);
        assert_eq!(settings.providers[0].id, "keep-me");
        assert_eq!(settings.providers[0].api_keys, vec!["sk-test".to_string()]);
        assert!(settings.custom_themes.is_empty());
    }

    #[test]
    fn legacy_settings_without_custom_themes_keep_other_fields() {
        let missing: Settings = serde_json::from_value(legacy_body()).expect("old settings parse");
        assert_legacy_fields(&sanitize_settings(missing));

        let empty: Settings = serde_json::from_str("{}").expect("empty settings parse");
        let empty = sanitize_settings(empty);
        assert!(empty.custom_themes.is_empty());
        assert_eq!(empty.theme, "system");
        assert_eq!(empty.theme_color, "neutral");

        let stored = serde_json::to_value(Settings::default()).expect("default serializes");
        assert_eq!(stored["customThemes"], serde_json::json!([]));
        assert!(stored.get("custom_themes").is_none());
        assert_eq!(stored["themeColor"], "neutral");

        for broken in [
            serde_json::Value::Null,
            serde_json::json!("nope"),
            serde_json::json!({}),
        ] {
            let mut body = legacy_body();
            body["customThemes"] = broken;
            let settings: Settings =
                serde_json::from_value(body).expect("broken customThemes must not reject settings");
            assert_legacy_fields(&sanitize_settings(settings));
        }
    }

    #[test]
    fn valid_custom_theme_survives_save_and_load() {
        let mut light = numbered_palette(1);
        light.accent = light.accent.to_ascii_uppercase();
        let ocean = uid(1);
        let grove = uid(2);
        let max_name = "名".repeat(80);
        let mut settings = Settings::default();
        settings.theme = "dark".to_string();
        settings.theme_color = format!("  {} ", ocean.to_ascii_uppercase());
        settings.custom_themes = vec![
            ThemeDefinition {
                id: ocean.to_ascii_uppercase(),
                name: format!("  {max_name}  "),
                light,
                dark: numbered_palette(40),
            },
            theme(&grove, "Grove"),
        ];
        settings.favorite_models = vec!["prov:model".to_string()];

        let (canonical, persisted) = settings_for_persistence(&settings);
        assert_eq!(canonical.theme, "dark");
        assert_eq!(canonical.theme_color, ocean);
        assert_eq!(persisted.theme_color, ocean);
        assert_eq!(canonical.custom_themes, persisted.custom_themes);
        assert_eq!(canonical.custom_themes.len(), 2);
        assert_eq!(canonical.custom_themes[0].id, ocean);
        assert_eq!(canonical.custom_themes[0].id.len(), 36);
        assert_eq!(canonical.custom_themes[0].name, max_name);
        assert_eq!(canonical.custom_themes[0].name.encode_utf16().count(), 80);
        assert_eq!(canonical.custom_themes[0].light, numbered_palette(1));
        assert_eq!(canonical.custom_themes[0].dark, numbered_palette(40));
        assert_eq!(canonical.custom_themes[1].id, grove);
        assert_eq!(canonical.favorite_models, vec!["prov:model".to_string()]);

        let value = serde_json::to_value(&persisted).expect("persisted settings serialize");
        assert!(value.get("custom_themes").is_none());
        assert_eq!(value["customThemes"][0]["light"]["surface"], "#010101");
        assert_eq!(value["customThemes"][0]["light"]["surfaceSoft"], "#020202");
        assert_eq!(value["customThemes"][0]["light"]["onAccent"], "#0f0f0f");
        assert_eq!(value["customThemes"][0]["light"]["dangerSoft"], "#111111");
        assert!(
            value["customThemes"][0]["light"]
                .get("surface_soft")
                .is_none()
        );
        assert!(value["customThemes"][0].get("css").is_none());

        let reloaded: Settings = serde_json::from_value(value).expect("saved settings should load");
        let reloaded = sanitize_settings(reloaded);
        assert_eq!(reloaded.custom_themes, canonical.custom_themes);
        assert_eq!(reloaded.theme_color, ocean);
        assert_eq!(reloaded.theme, "dark");
        assert_eq!(reloaded.favorite_models, vec!["prov:model".to_string()]);
    }

    #[test]
    fn save_keeps_every_valid_theme_with_no_count_cap() {
        let mut settings = Settings::default();
        settings.custom_themes = (1..=100)
            .map(|n| theme(&uid(n), &format!("Theme {n}")))
            .collect();
        settings.theme_color = uid(100);
        settings.favorite_models = vec!["prov:model".to_string()];

        let (canonical, persisted) = settings_for_persistence(&settings);
        assert_eq!(canonical.custom_themes.len(), 100);
        assert_eq!(persisted.custom_themes.len(), 100);
        assert_eq!(canonical.custom_themes, persisted.custom_themes);
        assert_eq!(canonical.theme_color, uid(100));
        assert_eq!(canonical.custom_themes[99].id, uid(100));
        assert_eq!(canonical.custom_themes[99].name, "Theme 100");
        assert_eq!(canonical.favorite_models, vec!["prov:model".to_string()]);

        let reloaded = sanitize_settings(
            serde_json::from_value(serde_json::to_value(&persisted).expect("serialize"))
                .expect("reload"),
        );
        assert_eq!(reloaded.custom_themes.len(), 100);
        assert_eq!(reloaded.theme_color, uid(100));
    }

    #[test]
    fn duplicate_save_keeps_the_requested_theme() {
        let id = uid(7);
        let mut older = theme(&id, "Older");
        older.light = filled_palette("#111111");
        older.dark = filled_palette("#222222");
        let mut requested = theme(&id.to_ascii_uppercase(), "Requested");
        requested.light = filled_palette("#abcdef");
        requested.dark = filled_palette("#fedcba");
        let mut settings = Settings::default();
        settings.theme_color = id.clone();
        settings.custom_themes = vec![older, theme(&uid(8), "Other"), requested];

        let (canonical, persisted) = settings_for_persistence(&settings);
        assert_eq!(canonical.custom_themes.len(), 2);
        assert_eq!(canonical.custom_themes[0].id, uid(8));
        assert_eq!(canonical.custom_themes[1].id, id);
        assert_eq!(canonical.custom_themes[1].name, "Requested");
        assert_eq!(canonical.custom_themes[1].light, filled_palette("#abcdef"));
        assert_eq!(canonical.custom_themes[1].dark, filled_palette("#fedcba"));
        assert_eq!(canonical.theme_color, id);
        assert_eq!(canonical.custom_themes, persisted.custom_themes);
        let stored = serde_json::to_string(&persisted.custom_themes).expect("serialize");
        assert!(!stored.contains("111111"));
        assert!(stored.contains("abcdef"));
    }

    #[test]
    fn invalid_colors_cannot_enter_the_active_palette() {
        let attacks = [
            "#ffffff;background:url(https://evil.test/x)",
            "#fff",
            "rgb(0,0,0)",
            "expression(alert(1))",
            "#aabbccdd",
            "url(#112233)",
            "#aa bbcc",
        ];
        for bad in attacks {
            let mut light = filled_palette("#112233");
            light.danger_soft = bad.to_string();
            let mut settings = Settings::default();
            settings.theme_color = uid(2);
            settings.custom_themes = vec![
                theme(&uid(1), "Grove"),
                ThemeDefinition {
                    id: uid(2),
                    name: "Ocean".to_string(),
                    light,
                    dark: filled_palette("#112233"),
                },
            ];
            let settings = sanitize_settings(settings);
            assert_eq!(settings.custom_themes.len(), 1, "{bad}");
            assert_eq!(settings.custom_themes[0].id, uid(1), "{bad}");
            assert_eq!(settings.theme_color, "neutral", "{bad}");
            let stored = serde_json::to_string(&settings.custom_themes).expect("serialize");
            assert!(!stored.contains(bad), "{bad}");
            assert!(!stored.contains("url("), "{bad}");
            assert!(!stored.contains("expression"), "{bad}");
        }

        let mut light = serde_json::to_value(filled_palette("#112233")).expect("palette");
        light.as_object_mut().expect("object").remove("dangerSoft");
        light.as_object_mut().expect("object").insert(
            "css".to_string(),
            serde_json::json!("body{background:url(https://evil.test)}"),
        );
        let raw = serde_json::json!({
            "themeColor": uid(2),
            "customThemes": [{
                "id": uid(2),
                "name": "Ocean",
                "light": light,
                "dark": filled_palette("#445566")
            }]
        });
        let settings: Settings = serde_json::from_value(raw).expect("partial palette still parses");
        let settings = sanitize_settings(settings);
        assert!(settings.custom_themes.is_empty());
        assert_eq!(settings.theme_color, "neutral");

        let mut light = serde_json::to_value(filled_palette("  #AABBCC ")).expect("palette");
        light.as_object_mut().expect("object").insert(
            "css".to_string(),
            serde_json::json!("body{background:url(https://evil.test)}"),
        );
        let raw = serde_json::json!({
            "themeColor": uid(2),
            "customThemes": [{
                "id": uid(2),
                "name": "Ocean",
                "light": light,
                "dark": filled_palette("#445566")
            }]
        });
        let settings = sanitize_settings(
            serde_json::from_value::<Settings>(raw).expect("extra css key still parses"),
        );
        assert_eq!(settings.custom_themes.len(), 1);
        assert_eq!(settings.custom_themes[0].light, filled_palette("#aabbcc"));
        assert_eq!(settings.custom_themes[0].dark, filled_palette("#445566"));
        assert_eq!(settings.theme_color, uid(2));
        let stored = serde_json::to_value(&settings.custom_themes[0]).expect("theme");
        assert!(stored["light"].get("css").is_none());
        assert!(!stored.to_string().contains("url("));
    }

    #[test]
    fn reserved_and_non_uuid_ids_do_not_override_builtins() {
        let max_name = "m".repeat(80);
        let over_name = "n".repeat(81);
        let mut hijack = theme("neutral", "Hijack");
        hijack.light = filled_palette("#ff00ff");
        hijack.dark = filled_palette("#ff00ff");
        let mut warm = theme("warm", "Hijack");
        warm.light = filled_palette("#ff00ff");
        warm.dark = filled_palette("#ff00ff");
        let mut cool = theme("cool", "Hijack");
        cool.light = filled_palette("#ff00ff");
        cool.dark = filled_palette("#ff00ff");

        let mut settings = Settings::default();
        settings.theme_color = "Neutral".to_string();
        settings.custom_themes = vec![
            hijack,
            warm,
            cool,
            theme("grove", "Slug"),
            theme(&"c".repeat(36), "Not a uuid"),
            theme(&format!("{}0", uid(1)), "Too long"),
            theme("00000000-0000-4000-8000-00000000000", "Too short"),
            theme(&uid(3), "   "),
            theme(&uid(4), &over_name),
            theme(&uid(5), &max_name),
            theme(&format!("  {}  ", uid(6)), "  海湾  "),
        ];

        let settings = sanitize_settings(settings);
        let ids: Vec<String> = settings
            .custom_themes
            .iter()
            .map(|item| item.id.clone())
            .collect();
        assert_eq!(ids, vec![uid(5), uid(6)]);
        assert_eq!(settings.custom_themes[0].name, max_name);
        assert_eq!(settings.custom_themes[0].name.encode_utf16().count(), 80);
        assert_eq!(settings.custom_themes[1].name, "海湾");
        assert_eq!(settings.theme_color, "neutral");
        let stored = serde_json::to_string(&settings.custom_themes).expect("serialize");
        assert!(!stored.contains("ff00ff"));
        assert!(
            !settings
                .custom_themes
                .iter()
                .any(|item| BUILTIN_THEME_IDS.contains(&item.id.as_str()))
        );
    }

    #[test]
    fn builtin_selections_survive_settings_roundtrip_without_custom_overrides() {
        for id in BUILTIN_THEME_IDS {
            let mut settings = Settings::default();
            settings.theme_color = format!("  {}  ", id.to_ascii_uppercase());
            settings.custom_themes = vec![theme(id, "Override"), theme(&uid(1), "User")];
            let settings = sanitize_settings(settings);
            assert_eq!(settings.theme_color, *id);
            assert_eq!(settings.custom_themes.len(), 1);
            assert_eq!(settings.custom_themes[0].id, uid(1));

            let body = serde_json::to_string(&settings).expect("serialize settings");
            let restored = sanitize_settings(
                serde_json::from_str::<Settings>(&body).expect("reload settings"),
            );
            assert_eq!(restored.theme_color, *id);
            assert_eq!(restored.custom_themes, settings.custom_themes);
        }
    }

    #[test]
    fn invalid_theme_color_resolves_neutral_and_custom_id_is_kept() {
        let mut settings = Settings::default();
        settings.theme_color = "mint".to_string();
        let settings = sanitize_settings(settings);
        assert_eq!(settings.theme_color, "neutral");

        let mut settings = Settings::default();
        settings.theme_color = "  WARM ".to_string();
        settings.custom_themes = vec![theme(&uid(1), "海湾")];
        let settings = sanitize_settings(settings);
        assert_eq!(settings.theme_color, "warm");
        assert_eq!(settings.custom_themes[0].id, uid(1));

        let mut settings = Settings::default();
        settings.theme_color = format!("  {} ", uid(1).to_ascii_uppercase());
        settings.custom_themes = vec![theme(&uid(1), "海湾")];
        let settings = sanitize_settings(settings);
        assert_eq!(settings.theme_color, uid(1));
        assert_eq!(settings.custom_themes.len(), 1);

        let mut poison = theme(&uid(9), "Poison");
        poison.light.surface = "#112233;background:url(https://evil.test)".to_string();
        let mut settings = Settings::default();
        settings.theme_color = uid(9);
        settings.favorite_models = vec!["prov:model".to_string()];
        settings.custom_themes = vec![poison, theme(&uid(1), "Grove")];
        let settings = sanitize_settings(settings);
        assert_eq!(settings.theme_color, "neutral");
        assert_eq!(settings.custom_themes.len(), 1);
        assert_eq!(settings.custom_themes[0].id, uid(1));
        assert_eq!(settings.favorite_models, vec!["prov:model".to_string()]);
    }

    #[test]
    fn malformed_custom_theme_items_do_not_discard_other_settings() {
        let ocean = uid(2);
        let mut body = legacy_body();
        body["themeColor"] = serde_json::json!(ocean);
        body["customThemes"] = serde_json::json!([
            "not-a-theme",
            { "id": 12, "name": "nope" },
            { "id": "bad", "name": "Bad", "light": "red", "dark": {} },
            null,
            {
                "id": ocean,
                "name": " Ocean ",
                "light": filled_palette("#112233"),
                "dark": filled_palette("#445566")
            }
        ]);
        let settings: Settings = serde_json::from_value(body).expect("mixed array still parses");
        let settings = sanitize_settings(settings);
        assert_eq!(settings.hotkey, "Alt+T");
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.theme_color, ocean);
        assert_eq!(settings.providers[0].id, "keep-me");
        assert_eq!(settings.providers[0].api_keys, vec!["sk-test".to_string()]);
        assert_eq!(settings.favorite_models, vec!["prov:model".to_string()]);
        assert_eq!(settings.custom_themes.len(), 1);
        assert_eq!(settings.custom_themes[0].id, ocean);
        assert_eq!(settings.custom_themes[0].name, "Ocean");
        assert_eq!(settings.custom_themes[0].light, filled_palette("#112233"));
        assert_eq!(settings.custom_themes[0].dark, filled_palette("#445566"));
    }
}
