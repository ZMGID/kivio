//! Native settings mutations preserve the original settings.json envelope and unknown fields.
use crate::settings_config::Settings;
use serde_json::{Value, json};
use std::path::Path;
static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());
static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Process-local change signal. Persisted settings remain authoritative.
pub fn revision() -> u64 { REVISION.load(std::sync::atomic::Ordering::Acquire) }


fn document(directory: &Path) -> Result<Value, String> {
    match std::fs::read(directory.join("settings.json")) {
        Ok(bytes) => {
            let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if !value.is_object() || !value.get("settings").is_some_and(Value::is_object) {
                return Err("settings.json 的 settings 字段无效；未覆盖原文件。".into());
            }
            Ok(value)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({"settings": {}})),
        Err(e) => Err(e.to_string()),
    }
}
pub fn load(directory: &Path) -> Result<Settings, String> {
    let mut settings: Settings = serde_json::from_value(document(directory)?["settings"].clone())
        .map_err(|e| e.to_string())?;
    settings.reading_target_lang = crate::settings_config::resolve_reading_language(&settings);
    Ok(settings)
}

/// Runtime projection combines authoritative documents without writing either.
/// Model selections and native video enablement are committed together by providers.
pub fn load_runtime(directory: &Path) -> Result<Settings, String> {
    let mut settings = load(directory)?;
    let bytes = match std::fs::read(directory.join("conversations/providers.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(settings),
        Err(error) => return Err(error.to_string()),
    };
    let catalog: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if !catalog.is_object() {
        return Err("模型配置文件格式无效".into());
    }
    if let Some(providers) = catalog.get("providers") {
        settings.providers = serde_json::from_value(providers.clone()).map_err(|error| error.to_string())?;
    }
    if let Some(defaults) = catalog.get("defaultModels") {
        settings.default_models = serde_json::from_value(defaults.clone()).map_err(|error| error.to_string())?;
    }
    if let Some(enabled) = catalog.get("videoAnalysisEnabled") {
        settings.chat.video_analysis_enabled = serde_json::from_value(enabled.clone()).map_err(|error| error.to_string())?;
    }
    Ok(settings)
}
/// Only explicit legacy choices override native preferences on upgrade.
pub fn appearance_selection(directory: &Path) -> Result<(Option<String>, Option<String>), String> {
    let document = document(directory)?;
    let settings = &document["settings"];
    Ok((
        settings
            .get("theme")
            .and_then(Value::as_str)
            .map(str::to_owned),
        settings
            .get("themeColor")
            .and_then(Value::as_str)
            .map(str::to_owned),
    ))
}
/// Editable textual preferences, scoped to existing Rust settings fields.
/// Host registration and runtime enablement remain with their platform owners.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextPreference {
    ReadingLanguage,
    InputLanguage,
    InputPrompt,
    LensLanguage,
    LensSystemPrompt,
    LensQuestionPrompt,
    LensOrder,
    ScreenshotPrompt,
    TextPrompt,
    ReplacementPrompt,
    AgentPrompt,
    ChatPrompt,
    PromptOptimize,
    UserName,
    AgentLanguage,
}
impl TextPreference {
    /// The confirmed settings value, not the generated prompt shown by the editor.
    pub fn value(self, settings: &Settings) -> Value {
        if self == Self::ReadingLanguage {
            return json!(crate::settings_config::resolve_reading_language(settings));
        }
        let document = serde_json::to_value(settings).expect("serializable settings");
        let path = format!("/{}", self.path().join("/"));
        document.pointer(&path).cloned().unwrap_or(Value::Null)
    }
    pub fn path(self) -> &'static [&'static str] {
        match self {
            Self::ReadingLanguage => &["readingTargetLang"],
            Self::InputLanguage => &["targetLang"],
            Self::InputPrompt => &["translatorPrompt"],
            Self::LensLanguage => &["lens", "defaultLanguage"],
            Self::LensSystemPrompt => &["lens", "systemPrompt"],
            Self::LensQuestionPrompt => &["lens", "questionPrompt"],
            Self::LensOrder => &["lens", "messageOrder"],
            Self::ScreenshotPrompt => &["screenshotTranslation", "prompt"],
            Self::TextPrompt => &["screenshotTranslation", "textPrompt"],
            Self::ReplacementPrompt => &["screenshotTranslation", "replacePrompt"],
            Self::AgentPrompt => &["chat", "systemPrompt"],
            Self::ChatPrompt => &["chat", "chatMode", "systemPrompt"],
            Self::PromptOptimize => &["chat", "promptOptimizePrompt"],
            Self::UserName => &["chat", "userDisplayName"],
            Self::AgentLanguage => &["chat", "defaultLanguage"],
        }
    }
}
#[derive(Clone)]
pub enum Change {
    RefreshedMcpAuth(crate::settings::ChatMcpServer),
    Theme {
        before: Option<crate::settings::ThemeDefinition>,
        after: Option<crate::settings::ThemeDefinition>,
    },
    Cli {
        id: String,
        before: Option<crate::settings_config::ExternalCliAgentConfig>,
        after: crate::settings_config::ExternalCliAgentConfig,
    },
    SubAgent {
        role: String,
        before: Option<crate::tool_settings::SubAgentModelSelection>,
        after: crate::tool_settings::SubAgentModelSelection,
    },
    /// Compare-and-set a small group of related fields (for example a model pair).
    /// Values are checked against the existing Settings schema before writing.
    Fields(Vec<FieldEdit>),
    WebSearch {
        before: crate::settings_config::LensWebSearchConfig,
        after: crate::settings_config::LensWebSearchConfig,
    },
    Text {
        field: TextPreference,
        before: Value,
        after: String,
    },
    Hook {
        before: Option<crate::tool_settings::HookDef>,
        after: Option<crate::tool_settings::HookDef>,
    },
    Retry(bool),
    RetryAttempts(u8),
    Thinking(bool),
    SystemPrompt(String),
}
pub fn save(directory: &Path, change: Change) -> Result<Settings, String> {
    let _guard = WRITES.lock().map_err(|e| e.to_string())?;
    let mut doc = document(directory)?;
    serde_json::from_value::<Settings>(doc["settings"].clone()).map_err(|e| e.to_string())?;
    let settings = &mut doc["settings"];
    match change {
        Change::RefreshedMcpAuth(server) => {
            let mut current: Settings =
                serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            let before = serde_json::to_value(&current).map_err(|e| e.to_string())?;
            crate::mcp::manager::apply_refreshed_auth_to_settings(&mut current, &server);
            let after = serde_json::to_value(current).map_err(|e| e.to_string())?;
            // Patch only authorization fields while holding the document lock.
            // Keep other connectors and future per-server fields untouched.
            if let Some(servers) = settings
                .pointer_mut("/chatTools/servers")
                .and_then(Value::as_array_mut)
            {
                for (index, entry) in servers.iter_mut().enumerate() {
                    for field in ["auth", "headers"] {
                        if before["chatTools"]["servers"][index][field]
                            != after["chatTools"]["servers"][index][field]
                        {
                            entry[field] = after["chatTools"]["servers"][index][field].clone();
                        }
                    }
                }
            }
            if before["lens"]["webSearch"]["tinyfishMcpAuth"]
                != after["lens"]["webSearch"]["tinyfishMcpAuth"]
            {
                if settings.get("lens").is_none() {
                    settings["lens"] = json!({});
                }
                if settings["lens"].get("webSearch").is_none() {
                    settings["lens"]["webSearch"] = json!({});
                }
                settings["lens"]["webSearch"]["tinyfishMcpAuth"] =
                    after["lens"]["webSearch"]["tinyfishMcpAuth"].clone();
            }
        }
        Change::Theme { before, after } => {
            let current: Settings =
                serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            let id = after
                .as_ref()
                .or(before.as_ref())
                .ok_or("缺少主题")?
                .id
                .clone();
            if current.custom_themes.iter().find(|theme| theme.id == id) != before.as_ref() {
                return Err("主题已被其他操作修改，请重新读取".into());
            }
            let mut themes = current.custom_themes;
            themes.retain(|theme| theme.id != id);
            if let Some(after) = after {
                let mut validated = vec![after];
                crate::settings_config::sanitize_custom_themes(&mut validated);
                let value = validated
                    .pop()
                    .ok_or("主题名称或颜色无效，每种颜色必须为 #rrggbb")?;
                themes.push(value);
            } else if current.theme_color == id {
                settings["themeColor"] = json!("neutral");
            }
            settings["customThemes"] = serde_json::to_value(themes).map_err(|e| e.to_string())?;
        }

        Change::Cli { id, before, after } => {
            if id.is_empty() || id.contains(['/', '\\']) {
                return Err("无效 CLI 标识。".into());
            }
            let current: Settings =
                serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            if serde_json::to_value(current.chat.external_cli_agents.get(&id))
                .map_err(|e| e.to_string())?
                != serde_json::to_value(&before).map_err(|e| e.to_string())?
            {
                return Err("此 CLI 配置已被其他操作修改，请重新读取。".into());
            }
            if after
                .env
                .iter()
                .any(|entry| entry.key.trim().is_empty() || entry.key.contains(['=', '\n', '\0']))
            {
                return Err("环境变量名称不能为空，也不能包含等号或换行。".into());
            }
            if after
                .custom_models
                .iter()
                .any(|model| model.id.trim().is_empty())
            {
                return Err("模型 ID 不能为空。".into());
            }
            if settings.get("chat").is_none() {
                settings["chat"] = json!({});
            }
            if settings["chat"].get("externalCliAgents").is_none() {
                settings["chat"]["externalCliAgents"] = json!({});
            }
            // Preserve future fields owned by a newer client.
            let target = &mut settings["chat"]["externalCliAgents"][&id];
            if target.is_null() {
                *target = json!({});
            }
            let baseline =
                serde_json::to_value(before.unwrap_or_default()).map_err(|e| e.to_string())?;
            let fields = serde_json::to_value(after).map_err(|e| e.to_string())?;
            for (key, value) in fields.as_object().ok_or("CLI 配置无效。")? {
                if baseline.get(key) != Some(value) {
                    target[key] = value.clone();
                }
            }
        }
        Change::SubAgent {
            role,
            before,
            after,
        } => {
            if !matches!(role.as_str(), "task" | "smol" | "slow") {
                return Err("未知子代理角色。".into());
            }
            let current: Settings =
                serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            if serde_json::to_value(current.chat_tools.sub_agent_models.get(&role))
                .map_err(|e| e.to_string())?
                != serde_json::to_value(before).map_err(|e| e.to_string())?
            {
                return Err("此角色已被其他操作修改，请重新读取。".into());
            }
            if after.provider_id.is_empty() != after.model.is_empty() {
                return Err("请同时选择供应商和模型。".into());
            }
            if settings.get("chatTools").is_none() {
                settings["chatTools"] = json!({});
            }
            if settings["chatTools"].get("subAgentModels").is_none() {
                settings["chatTools"]["subAgentModels"] = json!({});
            }
            settings["chatTools"]["subAgentModels"][role] =
                serde_json::to_value(after).map_err(|e| e.to_string())?;
        }
        Change::Fields(edits) => {
            let current: Settings =
                serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            let current = serde_json::to_value(current).map_err(|e| e.to_string())?;
            for edit in edits {
                let baseline = current.pointer(&edit.path).ok_or("设置字段不存在。")?;
                if baseline != &edit.before {
                    return Err("此设置已被其他操作修改，请重新读取后再编辑。".into());
                }
                if !(matches!(edit.path.as_str(), "/chatTools/skillScanPaths" | "/chatTools/disabledSkillIds")
                    && baseline.is_array()
                    && edit
                        .after
                        .as_array()
                        .is_some_and(|values| values.iter().all(Value::is_string))
                    || baseline.is_boolean() && edit.after.is_boolean()
                    || baseline.is_string() && edit.after.is_string()
                    || baseline.is_number() && edit.after.is_number()
                    || baseline.is_null() && (edit.after.is_string() || edit.after.is_null()))
                {
                    return Err("设置值类型不正确。".into());
                }
                // Providers and model purposes have their own document owner.
                if edit.path.starts_with("/providers") || edit.path.starts_with("/defaultModels") || edit.path == "/chat/videoAnalysisEnabled" {
                    return Err("请通过模型设置修改供应商与模型用途。".into());
                }
                let keys: Vec<_> = edit.path.trim_start_matches('/').split('/').collect();
                let mut target = &mut *settings;
                for key in &keys[..keys.len() - 1] {
                    if target.get(*key).is_none_or(Value::is_null) {
                        target[*key] = json!({});
                    }
                    target = &mut target[*key];
                }
                target[keys[keys.len() - 1]] = edit.after;
            }
            let checked: Settings =
                serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            if !matches!(checked.settings_language.as_deref(), None | Some("zh" | "en")) {
                return Err("界面语言必须为中文或英文。".into());
            }
            if !(360..=720).contains(&checked.screenshot_translation.card_width) {
                return Err("翻译卡宽度必须为 360–720 px。".into());
            }
            if !(1..=11).contains(&checked.retry_attempts) {
                return Err("重试次数必须为 0–10。".into());
            }
            if !matches!(checked.lens.message_order.as_str(), "asc" | "desc") {
                return Err("消息顺序无效。".into());
            }
            if !(crate::tool_settings::SUB_AGENT_CONCURRENCY_MIN
                ..=crate::tool_settings::SUB_AGENT_CONCURRENCY_MAX)
                .contains(&checked.chat_tools.sub_agent_concurrency)
            {
                return Err("子代理并发数必须为 1–64。".into());
            }
        }
        Change::WebSearch { before, after } => {
            if !(1..=10).contains(&after.max_results) {
                return Err("结果数量必须在 1–10 之间。".into());
            }
            if !matches!(
                after.search_depth.as_str(),
                "ultra-fast" | "fast" | "basic" | "advanced"
            ) {
                return Err("不支持的搜索深度。".into());
            }
            if matches!(
                after.provider,
                crate::settings_config::WebSearchProvider::Unknown
            ) {
                return Err("不支持的搜索服务商。".into());
            }
            let current: Settings =
                serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            let current =
                serde_json::to_value(current.lens.web_search).map_err(|e| e.to_string())?;
            let before = serde_json::to_value(before).map_err(|e| e.to_string())?;
            let after = serde_json::to_value(after).map_err(|e| e.to_string())?;
            if settings.get("lens").is_none() {
                settings["lens"] = json!({});
            }
            if settings["lens"].get("webSearch").is_none() {
                settings["lens"]["webSearch"] = json!({});
            }
            let keys: std::collections::BTreeSet<_> = before
                .as_object()
                .ok_or("搜索配置无效。")?
                .keys()
                .chain(after.as_object().ok_or("搜索配置无效。")?.keys())
                .collect();
            for key in keys {
                let value = after.get(key).unwrap_or(&Value::Null);
                let baseline = before.get(key).unwrap_or(&Value::Null);
                if baseline == value {
                    continue;
                }
                if current.get(key).unwrap_or(&Value::Null) != baseline {
                    return Err("搜索配置已被其他操作修改，请重载后再编辑。".into());
                }
                settings["lens"]["webSearch"][key] = value.clone();
            }
        }

        Change::Hook { before, mut after } => {
            let id = before
                .as_ref()
                .or(after.as_ref())
                .ok_or("缺少 Hook。")?
                .id
                .clone();
            if let Some(hook) = &after {
                if hook.name.trim().is_empty() || hook.id.trim().is_empty() {
                    return Err("请填写 Hook 名称。".into());
                }
                if !(crate::tool_settings::HOOK_MIN_TIMEOUT_MS
                    ..=crate::tool_settings::HOOK_MAX_TIMEOUT_MS)
                    .contains(&hook.timeout_ms)
                {
                    return Err("超时必须在 1000–600000 ms 之间。".into());
                }
                let mut checked = vec![hook.clone()];
                crate::tool_settings::sanitize_hooks(&mut checked);
                if checked.is_empty() {
                    return Err("请检查事件、执行方式以及脚本或 URL。".into());
                }
                after = checked.pop();
            }
            if settings.get("chatTools").is_none() {
                settings["chatTools"] = json!({});
            }
            if settings["chatTools"]
                .get("hooks")
                .is_none_or(Value::is_null)
            {
                settings["chatTools"]["hooks"] = json!([]);
            }
            let hooks = settings["chatTools"]["hooks"]
                .as_array_mut()
                .ok_or("Hooks 配置无效。")?;
            let index = hooks
                .iter()
                .position(|hook| hook["id"].as_str() == Some(&id));
            let current = index
                .map(|i| serde_json::from_value::<crate::tool_settings::HookDef>(hooks[i].clone()))
                .transpose()
                .map_err(|e| e.to_string())?;
            if serde_json::to_value(&current).map_err(|e| e.to_string())?
                != serde_json::to_value(&before).map_err(|e| e.to_string())?
            {
                return Err("此 Hook 已被其他操作修改，请刷新后再编辑。".into());
            }
            match (index, after) {
                (Some(i), Some(hook)) => {
                    // Preserve fields written by a newer client.
                    let fields = serde_json::to_value(hook).map_err(|e| e.to_string())?;
                    for (key, value) in fields.as_object().ok_or("Hook 配置无效。")? {
                        hooks[i][key] = value.clone();
                    }
                }
                (None, Some(hook)) => {
                    hooks.push(serde_json::to_value(hook).map_err(|e| e.to_string())?)
                }
                (Some(i), None) => {
                    hooks.remove(i);
                }
                (None, None) => return Err("Hook 已不存在。".into()),
            }
        }
        Change::Text { field, before, after: value } => {
            let current: Settings = serde_json::from_value(settings.clone()).map_err(|e| e.to_string())?;
            if field.value(&current) != before {
                return Err("此设置已被其他操作修改，请重新读取后再编辑。".into());
            }
            if field == TextPreference::ReadingLanguage {
                let mut candidate = current.clone();
                candidate.reading_target_lang = value.clone();
                if crate::settings_config::resolve_reading_language(&candidate) != value {
                    return Err("请选择明确的阅读目标语言。".into());
                }
            }
            // Legacy empty readingTargetLang inherits targetLang. Materialize its
            // current meaning before changing input translation so this independent
            // preference does not silently follow the new input language.
            if field == TextPreference::InputLanguage {
                settings["readingTargetLang"] = json!(crate::settings_config::resolve_reading_language(&current));
            }
            match field {
                TextPreference::LensOrder if !matches!(value.as_str(), "asc" | "desc") => {
                    return Err("消息顺序必须为 asc 或 desc。".into());
                }
                TextPreference::LensLanguage | TextPreference::AgentLanguage
                    if !matches!(value.as_str(), "" | "zh" | "zh-Hant" | "en") =>
                {
                    return Err("不支持的响应语言。".into());
                }
                _ => {}
            }
            let path = field.path();
            let mut target = &mut *settings;
            for key in &path[..path.len() - 1] {
                if target.get(*key).is_none() {
                    target[*key] = json!({});
                }
                target = &mut target[*key];
            }
            target[path[path.len() - 1]] = json!(value);
        }

        Change::Retry(value) => settings["retryEnabled"] = json!(value),
        Change::RetryAttempts(value) => {
            if !(1..=11).contains(&value) {
                return Err("重试次数必须为 0–10。".into());
            }
            settings["retryAttempts"] = json!(value);
        }
        Change::Thinking(value) => {
            if settings.get("chat").is_none() {
                settings["chat"] = json!({});
            }
            settings["chat"]["thinkingEnabled"] = json!(value);
        }
        Change::SystemPrompt(value) => {
            if settings.get("chat").is_none() {
                settings["chat"] = json!({});
            }
            settings["chat"]["systemPrompt"] = json!(value);
        }
    }
    let mut confirmed =
        serde_json::from_value::<Settings>(settings.clone()).map_err(|e| e.to_string())?;
    confirmed.reading_target_lang = crate::settings_config::resolve_reading_language(&confirmed);
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    crate::file_io::atomic_write(&directory.join("settings.json"), &bytes, "native-settings")
        .map_err(|e| e.to_string())?;
    REVISION.fetch_add(1, std::sync::atomic::Ordering::Release);
    Ok(confirmed)
}

#[derive(Clone)]
pub struct FieldEdit {
    pub path: String,
    pub before: Value,
    pub after: Value,
}
