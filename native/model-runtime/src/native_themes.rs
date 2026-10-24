//! The version-1 Kivio palette document, independent of the UI toolkit.
use crate::settings::{ThemeDefinition, ThemePalette};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    theme: Imported,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Imported {
    name: String,
    light: serde_json::Value,
    dark: serde_json::Value,
}
pub fn parse(text: &str) -> Result<ThemeDefinition, String> {
    let document: Document = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if document.version != 1 {
        return Err("不支持的主题版本".into());
    }
    let fields = serde_json::to_value(ThemePalette::default()).map_err(|e| e.to_string())?;
    for palette in [&document.theme.light, &document.theme.dark] {
        let object = palette.as_object().ok_or("无效色板")?;
        if object.len() != fields.as_object().unwrap().len()
            || object.keys().any(|key| fields.get(key).is_none())
        {
            return Err("色板字段不完整或含未知颜色".into());
        }
    }
    let theme = ThemeDefinition {
        id: uuid::Uuid::new_v4().to_string(),
        name: document.theme.name,
        light: serde_json::from_value(document.theme.light).map_err(|e| e.to_string())?,
        dark: serde_json::from_value(document.theme.dark).map_err(|e| e.to_string())?,
    };
    let mut themes = vec![theme];
    crate::settings_config::sanitize_custom_themes(&mut themes);
    themes
        .pop()
        .ok_or("主题名称或颜色无效，每种颜色必须为 #rrggbb".into())
}

pub fn export(theme: &ThemeDefinition, path: &std::path::Path) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&serde_json::json!({"version":1,"theme":{"name":theme.name,"light":theme.light,"dark":theme.dark}})).map_err(|e| e.to_string())?;
    crate::file_io::atomic_write(path, &text, "theme export")
}
