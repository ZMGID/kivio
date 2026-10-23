use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

#[path = "provider_settings.rs"]
pub(crate) mod provider;
pub use provider::{
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
use tools::{default_chat_approval_policy, default_hook_method, default_skill_fallback_mode};

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

fn sanitize_hooks(hooks: &mut Vec<HookDef>) {
    hooks.retain_mut(|hook| {
        hook.event = hook.event.trim().to_string();
        hook.kind = hook.kind.trim().to_lowercase();
        if crate::chat::hooks::HookEvent::parse(&hook.event).is_none() {
            return false;
        }
        match hook.kind.as_str() {
            "command" => {
                if hook.script.trim().is_empty() {
                    return false;
                }
            }
            "http" => {
                if hook.url.trim().is_empty() {
                    return false;
                }
                hook.method = hook.method.trim().to_uppercase();
                if hook.method.is_empty() {
                    hook.method = default_hook_method();
                }
            }
            _ => return false,
        }
        if hook.id.trim().is_empty() {
            hook.id = uuid::Uuid::new_v4().to_string();
        }
        hook.timeout_ms = hook
            .timeout_ms
            .clamp(HOOK_MIN_TIMEOUT_MS, HOOK_MAX_TIMEOUT_MS);
        true
    });
}

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
pub fn chat_native_tools_enabled(chat_tools: &ChatToolsConfig) -> bool {
    chat_tools.native_tools.any_enabled()
}

pub fn chat_memory_tools_enabled(settings: &Settings) -> bool {
    settings.chat_memory.enabled
}

pub fn chat_image_generation_enabled_for_session(
    settings: &Settings,
    session: Option<SessionModel<'_>>,
) -> bool {
    crate::chat::model_metadata::image_generation_model_for_session(settings, session).is_some()
}

pub fn is_skill_enabled(chat_tools: &ChatToolsConfig, skill_id: &str) -> bool {
    let skill_id = skill_id.trim();
    if skill_id.is_empty() {
        return false;
    }
    !chat_tools
        .disabled_skill_ids
        .iter()
        .any(|disabled| disabled == skill_id)
}

/// Global skill gate: Settings enable list + 插件门闸。
/// 插件附属 skill 仅在对应插件「仍受支持、已安装且启用」时可用。
pub fn skill_globally_available(chat_tools: &ChatToolsConfig, skill_id: &str) -> bool {
    if !crate::plugins::plugin_skill_available(skill_id) {
        return false;
    }
    is_skill_enabled(chat_tools, skill_id)
}

/// When [`skill_globally_available`] is false, returns a loop/UI-friendly error.
pub fn skill_global_unavailable_error(
    chat_tools: &ChatToolsConfig,
    skill_id: &str,
    skill_name: &str,
) -> Option<String> {
    if !crate::plugins::plugin_skill_available(skill_id) {
        if let Some(plugin_id) = crate::plugins::skill_owned_by_plugin(skill_id) {
            if crate::plugins::catalog_plugin(plugin_id).is_none() {
                return Some(format!(
                    "Operation tool «{plugin_id}» has been retired; its Skill is unavailable: {skill_name}"
                ));
            }
            return Some(format!(
                "Skill belongs to operation tool «{plugin_id}» — enable it in 设置 → 电脑操控: {skill_name}"
            ));
        }
        return Some(format!("Skill is unavailable: {skill_name}"));
    }
    if !is_skill_enabled(chat_tools, skill_id) {
        return Some(format!("Skill is disabled in Settings: {skill_name}"));
    }
    None
}

fn sync_legacy_chat_model_fields(settings: &mut Settings) {
    let (provider_id, model) = settings.effective_chat_model();
    settings.chat_provider_id = provider_id;
    settings.chat_model = model;
}

fn mirror_explicit_chat_default_for_persistence(settings: &mut Settings) {
    if settings.default_models.chat.is_configured() {
        settings.chat_provider_id = settings.default_models.chat.provider_id.clone();
        settings.chat_model = settings.default_models.chat.model.clone();
    } else {
        settings.chat_provider_id.clear();
        settings.chat_model.clear();
    }
}

pub fn sanitize_settings(mut settings: Settings) -> Settings {
    // 旧的自动中英互译只属于输入翻译；阅读翻译始终有一个明确目标。
    let reading = settings.reading_target_lang.trim();
    let reading = if reading.is_empty() {
        settings.target_lang.trim()
    } else {
        reading
    };
    settings.reading_target_lang = match reading {
        "zh-Hant" | "en" | "ja" | "ko" | "fr" | "de" | "es" | "pt" | "it" | "ru" | "ar" | "hi"
        | "th" | "vi" | "id" | "ms" | "tr" | "nl" | "pl" | "uk" | "sv" | "da" | "no" | "fi" => {
            reading.to_string()
        }
        _ => "zh".to_string(),
    };
    let token = settings.chat_tools.playwright_extension_token.trim();
    let token = token.strip_prefix("export ").unwrap_or(token).trim();
    let token = token
        .strip_prefix("PLAYWRIGHT_MCP_EXTENSION_TOKEN=")
        .unwrap_or(token)
        .trim();
    let token = token
        .strip_prefix('\"')
        .and_then(|s| s.strip_suffix('\"'))
        .or_else(|| token.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(token);
    settings.chat_tools.playwright_extension_token = token.trim().to_owned();
    // RapidOCR 档位归一:非法值回落到各自默认(截图=standard,文档处理=high)。
    if settings.screenshot_translation.rapid_ocr_tier != "standard"
        && settings.screenshot_translation.rapid_ocr_tier != "high"
    {
        settings.screenshot_translation.rapid_ocr_tier = default_rapid_ocr_tier();
    }
    if settings.document_processing.rapid_ocr_tier != "standard"
        && settings.document_processing.rapid_ocr_tier != "high"
    {
        settings.document_processing.rapid_ocr_tier = default_rapid_ocr_tier_high();
    }
    // 1. 从旧版配置迁移
    if settings.providers.is_empty() {
        // 迁移翻译提供商
        if let Some(old_openai) = settings.openai.take() {
            let legacy_key = old_openai.api_key.trim().to_string();
            let api_keys = if legacy_key.is_empty() {
                vec![]
            } else {
                vec![legacy_key]
            };
            settings.providers.push(ModelProvider {
                id: "default-translator".to_string(),
                name: "OpenAI (Translator)".to_string(),
                api_keys,
                api_key_legacy: None,
                base_url: old_openai.base_url,
                available_models: vec![],
                enabled_models: vec![old_openai.model.clone()],
                enabled: true,
                api_format: "openai".to_string(),
                model_overrides: std::collections::HashMap::new(),
                compress_request_body: false,
                request: Default::default(),
                active_key_index: 0,
            });
            settings.translator_provider_id = "default-translator".to_string();
            settings.translator_model = old_openai.model;
        }
        // 迁移 OCR 提供商
        if let Some(old_ocr) = settings.screenshot_translation.openai.take() {
            let legacy_key = old_ocr.api_key.trim().to_string();
            let api_keys = if legacy_key.is_empty() {
                vec![]
            } else {
                vec![legacy_key]
            };
            settings.providers.push(ModelProvider {
                id: "default-ocr".to_string(),
                name: "OpenAI (OCR)".to_string(),
                api_keys,
                api_key_legacy: None,
                base_url: old_ocr.base_url,
                available_models: vec![],
                enabled_models: vec![old_ocr.model.clone()],
                enabled: true,
                api_format: "openai".to_string(),
                model_overrides: std::collections::HashMap::new(),
                compress_request_body: false,
                request: Default::default(),
                active_key_index: 0,
            });
            settings.screenshot_translation.provider_id = "default-ocr".to_string();
            settings.screenshot_translation.model = old_ocr.model;
        }
    }

    for provider in &mut settings.providers {
        provider::sanitize_provider(provider);
    }

    let removed_legacy_local_provider_ids: std::collections::HashSet<String> = settings
        .providers
        .iter()
        .filter(|provider| provider.base_url == LEGACY_APPLE_INTELLIGENCE_BASE_URL)
        .map(|provider| provider.id.clone())
        .collect();
    if !removed_legacy_local_provider_ids.is_empty() {
        settings
            .providers
            .retain(|provider| provider.base_url != LEGACY_APPLE_INTELLIGENCE_BASE_URL);
        let fallback = settings.providers.iter().find(|p| p.enabled).map(|p| {
            (
                p.id.clone(),
                p.enabled_models.first().cloned().unwrap_or_default(),
            )
        });

        if removed_legacy_local_provider_ids.contains(&settings.chat_provider_id) {
            if let Some((id, model)) = fallback.as_ref() {
                settings.chat_provider_id = id.clone();
                settings.chat_model = model.clone();
            } else {
                settings.chat_provider_id.clear();
                settings.chat_model.clear();
            }
        }
        if removed_legacy_local_provider_ids.contains(&settings.translator_provider_id) {
            if let Some((id, model)) = fallback.as_ref() {
                settings.translator_provider_id = id.clone();
                settings.translator_model = model.clone();
            } else {
                settings.translator_provider_id.clear();
                settings.translator_model.clear();
            }
        }
        if removed_legacy_local_provider_ids.contains(&settings.screenshot_translation.provider_id)
        {
            if let Some((id, model)) = fallback.as_ref() {
                settings.screenshot_translation.provider_id = id.clone();
                settings.screenshot_translation.model = model.clone();
            } else {
                settings.screenshot_translation.provider_id.clear();
                settings.screenshot_translation.model.clear();
            }
        }
        if !settings.lens.provider_id.is_empty()
            && removed_legacy_local_provider_ids.contains(&settings.lens.provider_id)
        {
            settings.lens.provider_id.clear();
            settings.lens.model.clear();
        }
        for selection in [
            &mut settings.default_models.chat,
            &mut settings.default_models.vision,
            &mut settings.default_models.video_analysis,
            &mut settings.default_models.title_summary,
            &mut settings.default_models.compression,
            &mut settings.default_models.image_generation,
            &mut settings.default_models.prompt_optimize,
        ] {
            if removed_legacy_local_provider_ids.contains(&selection.provider_id) {
                if let Some((id, model)) = fallback.as_ref() {
                    selection.provider_id = id.clone();
                    selection.model = model.clone();
                } else {
                    selection.provider_id.clear();
                    selection.model.clear();
                }
            }
        }
    }

    // Consume the legacy global override once; TASK is its sole new owner.
    let legacy_provider = std::mem::take(&mut settings.chat_tools.sub_agent_provider_id);
    let legacy_model = std::mem::take(&mut settings.chat_tools.sub_agent_model);
    if !legacy_provider.trim().is_empty() && !legacy_model.trim().is_empty() {
        settings
            .chat_tools
            .sub_agent_models
            .entry("task".into())
            .or_insert(SubAgentModelSelection {
                provider_id: legacy_provider,
                model: legacy_model,
                thinking_level: None,
            });
    }
    for selection in settings.chat_tools.sub_agent_models.values_mut() {
        selection.provider_id = selection.provider_id.trim().to_string();
        selection.model = selection.model.trim().to_string();
        let provider = settings
            .providers
            .iter()
            .find(|p| p.id == selection.provider_id && p.enabled);
        if provider.is_none() || selection.model.is_empty() {
            *selection = SubAgentModelSelection::default();
            continue;
        }
        if let Some(level) = selection.thinking_level.as_deref() {
            let levels = crate::chat::model_metadata::reasoning_efforts_for_model(
                provider,
                &selection.model,
            );
            if level != "off" && !levels.iter().any(|candidate| candidate == level) {
                selection.thinking_level = None;
            }
        }
    }

    let provider_exists = |id: &str| settings.providers.iter().any(|p| p.id == id);
    let provider_selectable = |id: &str| settings.providers.iter().any(|p| p.id == id && p.enabled);
    let first_selectable_provider = || settings.providers.iter().find(|p| p.enabled);

    // 2. 为空字段设置默认值
    if settings.translator_provider_id.is_empty() {
        if let Some(first) = first_selectable_provider() {
            settings.translator_provider_id = first.id.clone();
        }
    }
    if settings.screenshot_translation.provider_id.is_empty() {
        if let Some(first) = first_selectable_provider() {
            settings.screenshot_translation.provider_id = first.id.clone();
        }
    }
    if !settings.chat_provider_id.trim().is_empty()
        && settings.default_models.chat.provider_id.trim().is_empty()
    {
        settings.default_models.chat.provider_id = settings.chat_provider_id.clone();
        settings.default_models.chat.model = settings.chat_model.clone();
    }

    if settings.providers.is_empty() {
        settings.translator_provider_id.clear();
        settings.default_models = DefaultModelsConfig::default();
        settings.screenshot_translation.provider_id.clear();
        settings.lens.provider_id.clear();
        settings.chat_tools.sub_agent_models.clear();
    } else {
        if !provider_selectable(&settings.translator_provider_id) {
            if let Some(first) = first_selectable_provider() {
                settings.translator_provider_id = first.id.clone();
                if let Some(model) = first.enabled_models.first() {
                    settings.translator_model = model.clone();
                }
            } else if !provider_exists(&settings.translator_provider_id) {
                settings.translator_provider_id.clear();
                settings.translator_model.clear();
            }
        }
        if !provider_selectable(&settings.screenshot_translation.provider_id) {
            if let Some(first) = first_selectable_provider() {
                settings.screenshot_translation.provider_id = first.id.clone();
                if let Some(model) = first.enabled_models.first() {
                    settings.screenshot_translation.model = model.clone();
                }
            } else if !provider_exists(&settings.screenshot_translation.provider_id) {
                settings.screenshot_translation.provider_id.clear();
                settings.screenshot_translation.model.clear();
            }
        }
        // lens provider 可空（空时 call_vision_api 走 translator_provider_id fallback）；
        // 但若用户填了一个不存在或已禁用的，重置为空让其走 fallback。
        if !settings.lens.provider_id.is_empty()
            && (!provider_exists(&settings.lens.provider_id)
                || !provider_selectable(&settings.lens.provider_id))
        {
            settings.lens.provider_id.clear();
            settings.lens.model.clear();
        }

        sanitize_default_model_selection(&mut settings.default_models.chat, &settings.providers);
        sanitize_default_model_selection(&mut settings.default_models.vision, &settings.providers);
        sanitize_default_model_selection(
            &mut settings.default_models.video_analysis,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.title_summary,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.compression,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.image_generation,
            &settings.providers,
        );
        sanitize_default_model_selection(
            &mut settings.default_models.prompt_optimize,
            &settings.providers,
        );
        sanitize_default_model_selection(&mut settings.default_models.advisor, &settings.providers);
    }

    // 3. 确保当前使用的模型确实在该 provider 的 enabled_models 中。
    // enabled_models 可以为空：预设 provider 不再自带模型。
    for provider in &mut settings.providers {
        if settings.translator_provider_id == provider.id
            && !provider.enabled_models.contains(&settings.translator_model)
        {
            settings.translator_model =
                provider.enabled_models.first().cloned().unwrap_or_default();
        }
        if settings.screenshot_translation.provider_id == provider.id
            && !provider
                .enabled_models
                .contains(&settings.screenshot_translation.model)
        {
            settings.screenshot_translation.model =
                provider.enabled_models.first().cloned().unwrap_or_default();
        }
        if !settings.lens.provider_id.is_empty()
            && settings.lens.provider_id == provider.id
            && !settings.lens.model.is_empty()
            && !provider.enabled_models.contains(&settings.lens.model)
        {
            settings.lens.model = provider.enabled_models.first().cloned().unwrap_or_default();
        }
    }

    sync_legacy_chat_model_fields(&mut settings);

    // 4. 规范化快捷键字符串
    settings.hotkey = normalize_hotkey(&settings.hotkey);
    settings.chat_hotkey = normalize_hotkey(&settings.chat_hotkey);
    settings.close_chat_hotkey = normalize_hotkey(&settings.close_chat_hotkey);
    settings.screenshot_translation.hotkey =
        normalize_hotkey(&settings.screenshot_translation.hotkey);
    settings.screenshot_translation.text_hotkey =
        normalize_hotkey(&settings.screenshot_translation.text_hotkey);
    settings.screenshot_translation.replace_hotkey =
        normalize_hotkey(&settings.screenshot_translation.replace_hotkey);
    settings.lens.hotkey = normalize_hotkey(&settings.lens.hotkey);
    settings.screenshot_annotate.hotkey = normalize_hotkey(&settings.screenshot_annotate.hotkey);

    // 规范化提示词（去除首尾空白，空值转为 None）
    settings.translator_prompt = normalize_optional_prompt(settings.translator_prompt.take());
    settings.screenshot_translation.prompt =
        normalize_optional_prompt(settings.screenshot_translation.prompt.take());
    settings.screenshot_translation.text_prompt =
        normalize_optional_prompt(settings.screenshot_translation.text_prompt.take());
    settings.screenshot_translation.replace_prompt =
        normalize_optional_prompt(settings.screenshot_translation.replace_prompt.take());
    // 翻译卡宽度单一真源：import / 手改 settings.json 也在此兜底到 360–720，
    // 与 set_translate_card_size 命令、设置页输入框、Lens 缩放 clamp 同域。
    settings.screenshot_translation.card_width =
        settings.screenshot_translation.card_width.clamp(360, 720);

    // 5. 其他字段验证。自定义主题先规范化，themeColor 才能指向仍然存在的 id。
    if !matches!(settings.theme.as_str(), "system" | "light" | "dark") {
        settings.theme = default_theme();
    }
    sanitize_custom_themes(&mut settings.custom_themes);
    settings.theme_color = canonicalize_theme_color(&settings.theme_color, &settings.custom_themes);
    settings.ui_font_scale = if settings.ui_font_scale.is_finite() {
        settings.ui_font_scale.clamp(0.8, 1.4)
    } else {
        default_ui_font_scale()
    };
    if settings.lens.message_order != "asc" && settings.lens.message_order != "desc" {
        settings.lens.message_order = "asc".to_string();
    }
    settings.lens.web_search.tavily_api_key =
        settings.lens.web_search.tavily_api_key.trim().to_string();
    settings.lens.web_search.exa_api_key = settings.lens.web_search.exa_api_key.trim().to_string();
    settings.lens.web_search.ollama_api_key =
        settings.lens.web_search.ollama_api_key.trim().to_string();
    settings.lens.web_search.grok_api_key =
        settings.lens.web_search.grok_api_key.trim().to_string();
    settings.lens.web_search.deepseek_api_key =
        settings.lens.web_search.deepseek_api_key.trim().to_string();
    settings.lens.web_search.brave_api_key =
        settings.lens.web_search.brave_api_key.trim().to_string();
    settings.lens.web_search.serper_api_key =
        settings.lens.web_search.serper_api_key.trim().to_string();
    settings.lens.web_search.bocha_api_key =
        settings.lens.web_search.bocha_api_key.trim().to_string();
    settings.lens.web_search.zhipu_api_key =
        settings.lens.web_search.zhipu_api_key.trim().to_string();
    settings.lens.web_search.tinyfish_api_key =
        settings.lens.web_search.tinyfish_api_key.trim().to_string();
    settings.lens.web_search.tinyfish_mcp_url = {
        let trimmed = settings.lens.web_search.tinyfish_mcp_url.trim();
        if trimmed.is_empty() {
            default_tinyfish_mcp_url()
        } else {
            trimmed.to_string()
        }
    };
    if let Some(auth) = settings.lens.web_search.tinyfish_mcp_auth.as_mut() {
        auth.access_token = auth.access_token.trim().to_string();
    }
    if settings
        .lens
        .web_search
        .tinyfish_mcp_auth
        .as_ref()
        .is_some_and(|auth| auth.access_token.is_empty())
    {
        settings.lens.web_search.tinyfish_mcp_auth = None;
    }
    settings.lens.web_search.searxng_base_url =
        settings.lens.web_search.searxng_base_url.trim().to_string();
    settings.lens.web_search.grok_model = {
        let trimmed = settings.lens.web_search.grok_model.trim();
        if trimmed.is_empty() {
            default_grok_model()
        } else {
            trimmed.to_string()
        }
    };
    settings.lens.web_search.grok_base_url = {
        let trimmed = settings.lens.web_search.grok_base_url.trim();
        if trimmed.is_empty() {
            default_grok_base_url()
        } else {
            trimmed.to_string()
        }
    };
    if settings
        .lens
        .web_search
        .grok_system_prompt
        .trim()
        .is_empty()
    {
        settings.lens.web_search.grok_system_prompt = default_grok_system_prompt();
    }
    settings.lens.web_search.deepseek_base_url = {
        let trimmed = settings.lens.web_search.deepseek_base_url.trim();
        if trimmed.is_empty() {
            default_deepseek_base_url()
        } else {
            trimmed.to_string()
        }
    };
    settings.lens.web_search.exa_mcp_url = {
        let trimmed = settings.lens.web_search.exa_mcp_url.trim();
        if trimmed.is_empty() {
            default_exa_mcp_url()
        } else {
            trimmed.to_string()
        }
    };
    // 未知/占位服务商回退到 Tavily，避免选中尚未接入的源导致搜索直接报错。
    if matches!(
        settings.lens.web_search.provider,
        WebSearchProvider::Unknown
    ) {
        settings.lens.web_search.provider = WebSearchProvider::Tavily;
    }
    // Fetch 源未设 = 跟随搜索。未知、或没有 URL fetch 接口的源清掉，避免配了却永远走直连。
    if settings
        .lens
        .web_search
        .fetch_provider
        .is_some_and(|provider| {
            matches!(provider, WebSearchProvider::Unknown)
                || !crate::web_search::provider_supports_fetch(provider)
                || provider == settings.lens.web_search.provider
        })
    {
        settings.lens.web_search.fetch_provider = None;
    }
    settings.lens.web_search.max_results = settings.lens.web_search.max_results.clamp(1, 10);
    if !matches!(
        settings.lens.web_search.search_depth.as_str(),
        "ultra-fast" | "fast" | "basic" | "advanced"
    ) {
        settings.lens.web_search.search_depth = default_web_search_depth();
    }

    // ponytail: 流式输出 / 思考模式的设置项已从 UI 移除，恒为开启（老配置里的 false 在此归位）
    settings.chat.stream_enabled = true;
    settings.chat.thinking_enabled = true;

    if !settings.chat_behavior_migrated_from_lens {
        if settings.lens.default_language.trim().is_empty() {
            // keep chat.default_language empty → inherit chain unchanged
        } else {
            settings.chat.default_language = settings.lens.default_language.clone();
        }
        settings.chat_behavior_migrated_from_lens = true;
    }
    if !matches!(
        settings.chat.default_language.trim(),
        "" | "zh" | "zh-Hant" | "en"
    ) {
        settings.chat.default_language.clear();
    }
    settings.chat.max_output_tokens = clamp_chat_max_output_tokens(settings.chat.max_output_tokens);
    settings.chat.system_prompt = settings.chat.system_prompt.trim().to_string();

    settings.chat_tools.tool_timeout_ms = settings
        .chat_tools
        .tool_timeout_ms
        .clamp(CHAT_TOOL_MIN_TIMEOUT_MS, CHAT_TOOL_MAX_TIMEOUT_MS);
    settings.chat_tools.sub_agent_concurrency = settings.chat_tools.sub_agent_concurrency.clamp(
        crate::chat::sub_agent::SUB_AGENT_CONCURRENCY_MIN,
        crate::chat::sub_agent::SUB_AGENT_CONCURRENCY_MAX,
    );
    settings.chat_tools.mcp_idle_timeout_ms = settings
        .chat_tools
        .mcp_idle_timeout_ms
        .clamp(MCP_IDLE_TIMEOUT_MIN_MS, MCP_IDLE_TIMEOUT_MAX_MS);
    // 工具输出截断：None（旧的"不截断"）归一到默认值，Some 值钳到合法区间。
    // 旧逻辑在此无条件置 None（等于永不截断 → 上下文撑爆主因），现改为始终保底截断。
    settings.chat_tools.max_tool_output_chars = Some(
        settings
            .chat_tools
            .max_tool_output_chars
            .unwrap_or(DEFAULT_MAX_TOOL_OUTPUT_CHARS)
            .clamp(CHAT_TOOL_MIN_OUTPUT_CHARS, CHAT_TOOL_MAX_OUTPUT_CHARS),
    );
    if !matches!(
        settings.chat_tools.approval_policy.trim(),
        "readonly_auto_sensitive_confirm" | "always_confirm" | "auto"
    ) {
        settings.chat_tools.approval_policy = default_chat_approval_policy();
    }
    sanitize_hooks(&mut settings.chat_tools.hooks);
    // One-shot green-light migration: bring a pre-green-light install (native
    // tools defaulted OFF) to the capability baseline. Approval is never weakened. Idempotent
    // via `chat_tools_greenlit_v1` so a user who later turns a tool back off, or
    // picks a stricter policy, is never re-flipped. Explicit approval choices survive.
    if !settings.chat_tools_greenlit_v1 {
        let native = &mut settings.chat_tools.native_tools;
        native.read_file = true;
        native.write_file = true;
        native.edit_file = true;
        native.run_command = true;
        native.web_fetch = true;
        native.web_search = true;
        settings.chat_tools_greenlit_v1 = true;
    }
    settings.chat_tools.skill_scan_paths = settings
        .chat_tools
        .skill_scan_paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    if !matches!(
        settings.chat_tools.skill_fallback_mode.trim(),
        "progressive" | "skill_md_only" | "legacy_full_body"
    ) {
        settings.chat_tools.skill_fallback_mode = default_skill_fallback_mode();
    }
    settings.chat_tools.disabled_skill_ids = settings
        .chat_tools
        .disabled_skill_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    settings.chat_tools.native_tools.workspace_roots = settings
        .chat_tools
        .native_tools
        .workspace_roots
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();
    let mut seen_roots = std::collections::HashSet::new();
    settings
        .chat_tools
        .native_tools
        .workspace_roots
        .retain(|path| seen_roots.insert(path.clone()));
    settings.chat_tools.native_tools.working_directory = settings
        .chat_tools
        .native_tools
        .working_directory
        .trim()
        .to_string();
    if settings
        .chat_tools
        .native_tools
        .working_directory
        .is_empty()
    {
        settings.chat_tools.native_tools.working_directory = settings
            .chat_tools
            .native_tools
            .workspace_roots
            .first()
            .cloned()
            .unwrap_or_else(default_chat_working_directory);
    }
    // Persist only the new single-directory setting after legacy migration.
    settings.chat_tools.native_tools.workspace_roots.clear();
    // Cua now runs through its Skill + CLI. Retire only Kivio-managed MCP
    // entries; preserve the user's disabled state and unrelated MCP servers.
    let mut cua_enabled = None;
    settings.chat_tools.servers.retain(|server| {
        // Retire the managed integration on load/import/save, including renamed
        // entries. Independently configured user servers remain user-owned.
        if server.id.trim() == "plugin-officecli"
            || server.connector_id.as_deref().map(str::trim) == Some("plugin:officecli")
        {
            return false;
        }
        let managed_cua = crate::computer_control::is_cua_mcp_server_id(server.id.trim())
            || matches!(
                server.connector_id.as_deref().map(str::trim),
                Some("computer-control:cua" | "plugin:cua-driver")
            );
        if managed_cua {
            cua_enabled = Some(cua_enabled.unwrap_or(false) || server.enabled);
        }
        !managed_cua
    });
    if cua_enabled == Some(false)
        && !settings
            .chat_tools
            .disabled_skill_ids
            .iter()
            .any(|id| id == "cua-driver")
    {
        settings
            .chat_tools
            .disabled_skill_ids
            .push("cua-driver".to_string());
    }
    for server in &mut settings.chat_tools.servers {
        server.id = server.id.trim().to_string();
        if server.id.is_empty() {
            server.id = format!("mcp-{}", uuid::Uuid::new_v4());
        }
        server.name = server.name.trim().to_string();
        if server.name.is_empty() {
            server.name = server.id.clone();
        }
        server.transport = server.transport.trim().to_ascii_lowercase();
        if server.transport == "http" || server.transport == "sse" {
            server.transport = "streamable_http".to_string();
        }
        if server.transport != "stdio" && server.transport != "streamable_http" {
            server.transport = "stdio".to_string();
        }
        server.url = server.url.trim().to_string();
        server.command = server.command.trim().to_string();
        server.args = server
            .args
            .iter()
            .map(|arg| arg.trim().to_string())
            .filter(|arg| !arg.is_empty())
            .collect();
        server.env = server
            .env
            .iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some((key.to_string(), value.clone()))
                }
            })
            .collect();
        server.headers = server
            .headers
            .iter()
            .filter_map(|(key, value)| {
                let key = key.trim();
                if key.is_empty() {
                    None
                } else {
                    Some((key.to_string(), value.trim().to_string()))
                }
            })
            .collect();
        server.cwd = server.cwd.take().and_then(|cwd| {
            let trimmed = cwd.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        server.enabled_tools = server
            .enabled_tools
            .iter()
            .map(|tool| tool.trim().to_string())
            .filter(|tool| !tool.is_empty())
            .collect();
        // 连接器 id 去空白；空串归一为 None，使其退回普通 MCP server。
        server.connector_id = server.connector_id.take().and_then(|cid| {
            let trimmed = cid.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
    }
    let mut seen_server_ids = std::collections::HashSet::new();
    settings
        .chat_tools
        .servers
        .retain(|server| seen_server_ids.insert(server.id.clone()));

    // 清理归档目录路径（去除首尾空白）
    settings.image_archive_path = settings.image_archive_path.trim().to_string();

    settings.retry_attempts = clamp_retry_attempts(settings.retry_attempts);

    // 系统 OCR 依赖平台本地 OCR 能力（macOS Apple Vision / Windows.Media.Ocr）。其它平台
    // 同步来的旧配置必须关闭，否则截图翻译会误入不可用分支。
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        settings.screenshot_translation.use_system_ocr = false;
    }

    // OCR 引擎模式迁移（vNext+）：
    // 1. 反序列化兜底变体 OcrMode::Legacy（如旧版 "tesseract" 字符串）→ RapidOcr，
    //    保留用户此前选择离线 OCR 的隐私边界；模型未下载时由前端引导下载。
    // 2. 若 ocr_mode 缺省（老版本数据），按 use_system_ocr 反推：
    //    true→System，false→CloudVision
    // 3. Linux 不支持 System / RapidOcr，强制落回 CloudVision
    if matches!(
        settings.screenshot_translation.ocr_mode,
        Some(OcrMode::Legacy)
    ) {
        settings.screenshot_translation.ocr_mode = Some(OcrMode::RapidOcr);
    }
    if settings.screenshot_translation.ocr_mode.is_none() {
        settings.screenshot_translation.ocr_mode =
            Some(if settings.screenshot_translation.use_system_ocr {
                OcrMode::System
            } else {
                OcrMode::CloudVision
            });
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if matches!(
            settings.screenshot_translation.ocr_mode,
            Some(OcrMode::System) | Some(OcrMode::RapidOcr)
        ) {
            settings.screenshot_translation.ocr_mode = Some(OcrMode::CloudVision);
        }
    }

    settings.onboarding_status = normalize_onboarding_status(&settings);

    sanitize_external_cli_agents(&mut settings.chat.external_cli_agents);

    settings
}

/// settings.json 是用户可手改的文件：环境变量名/模型 id 空白或全空格会被原样塞进子进程环境，
/// 表现成难查的「CLI 启动即报错」。这里统一 trim 并丢掉空条目，未知 agent id 的整条配置也丢掉。
fn sanitize_external_cli_agents(
    agents: &mut std::collections::HashMap<String, ExternalCliAgentConfig>,
) {
    agents.retain(|id, cfg| {
        if crate::external_agents::registry::get_agent_def(id).is_none() {
            return false;
        }
        cfg.path = cfg.path.trim().to_string();
        for pair in cfg.env.iter_mut() {
            pair.key = pair.key.trim().to_string();
            pair.value = pair.value.trim().to_string();
        }
        cfg.env.retain(|pair| !pair.key.is_empty());
        for model in cfg.custom_models.iter_mut() {
            model.id = model.id.trim().to_string();
            model.label = model.label.trim().to_string();
        }
        cfg.custom_models.retain(|model| !model.id.is_empty());
        for provider in cfg.providers.iter_mut() {
            provider.id = provider.id.trim().to_string();
            provider.name = provider.name.trim().to_string();
            for pair in provider.env.iter_mut() {
                pair.key = pair.key.trim().to_string();
                pair.value = pair.value.trim().to_string();
            }
            provider.env.retain(|pair| !pair.key.is_empty());
        }
        cfg.providers
            .retain(|provider| !provider.id.is_empty() && !provider.name.is_empty());
        // 悬空的 current_provider（供应商被删了 / 手改错了）必须归零：留着会让注入层
        // 找不到条目而**静默什么都不注入**，用户看到的是「选了供应商但没生效」。
        cfg.current_provider = cfg.current_provider.trim().to_string();
        if !cfg
            .providers
            .iter()
            .any(|provider| provider.id == cfg.current_provider && !provider.disabled)
        {
            cfg.current_provider = String::new();
        }
        true
    });
}

fn onboarding_status_is_set(raw: &str) -> bool {
    matches!(raw.trim(), "pending" | "completed" | "skipped")
}

fn provider_has_usable_config(provider: &ModelProvider) -> bool {
    provider.enabled && provider.authentication_ready() && !provider.enabled_models.is_empty()
}

fn settings_has_usable_provider_config(settings: &Settings) -> bool {
    settings.providers.iter().any(provider_has_usable_config)
}

fn normalize_onboarding_status(settings: &Settings) -> String {
    let raw = settings.onboarding_status.trim();
    if onboarding_status_is_set(raw) {
        return raw.to_string();
    }
    if settings_has_usable_provider_config(settings) {
        "completed".to_string()
    } else {
        "pending".to_string()
    }
}

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

fn clamp_retry_attempts(value: u8) -> u8 {
    value.clamp(1, 11)
}

/**
 * 规范化可选提示词：去除空白，空字符串转为 None
 */
fn normalize_optional_prompt(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

/**
 * 规范化快捷键字符串：去除各部分首尾空白并过滤空部分
 */
fn normalize_hotkey(value: &str) -> String {
    value
        .split('+')
        .map(|part| {
            let trimmed = part.trim();
            match trimmed.to_lowercase().as_str() {
                "cmd" | "command" | "commandorcontrol" => "CommandOrControl".to_string(),
                "ctrl" | "control" => "Control".to_string(),
                "opt" | "option" | "alt" => "Alt".to_string(),
                "shift" => "Shift".to_string(),
                "super" | "meta" => "Super".to_string(),
                "plus" => "Plus".to_string(),
                _ => trimmed.to_string(),
            }
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("+")
}

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
