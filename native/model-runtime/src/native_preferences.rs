//! Native settings mutations preserve the original settings.json envelope and unknown fields.
use crate::settings_config::Settings;
use serde_json::{Value, json};
use std::path::Path;
static WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    UserName,
    AgentLanguage,
}
impl TextPreference {
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
            Self::UserName => &["chat", "userDisplayName"],
            Self::AgentLanguage => &["chat", "defaultLanguage"],
        }
    }
}
#[derive(Clone)]
pub enum Change {
    WebSearch {
        before: crate::settings_config::LensWebSearchConfig,
        after: crate::settings_config::LensWebSearchConfig,
    },
    Text(TextPreference, String),
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
        Change::Text(field, value) => {
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
    Ok(confirmed)
}
