use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

use crate::chat::attachments::{
    inspect_attachment_sources, read_attachment_as_data_url, resolve_attachment_file_path,
    save_pasted_attachment, save_pasted_image, InspectedAttachment, PastedAttachmentSave,
    PastedImageSave,
};

/// 读取附件为 data URL，供前端 `<img>` 预览。`conversation_id` 为空时按本机绝对路径读取（发送前预览）。
#[tauri::command]
pub(crate) fn chat_read_attachment(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<serde_json::Value, String> {
    let full = resolve_attachment_file_path(&app, conversation_id.as_deref(), &path)?;
    let data_url = read_attachment_as_data_url(&full)?;
    Ok(serde_json::json!({
        "success": true,
        "data": data_url,
    }))
}

/// 用系统默认应用打开附件。
#[tauri::command]
#[allow(deprecated)]
pub(crate) fn chat_open_attachment(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<(), String> {
    let full = resolve_attachment_file_path(&app, conversation_id.as_deref(), &path)?;
    let path_str = full.to_string_lossy().into_owned();
    app.shell().open(path_str, None).map_err(|e| e.to_string())
}

/// 用系统默认应用打开生成产物文件。
#[tauri::command]
#[allow(deprecated)]
pub(crate) fn chat_open_generated_artifact(app: AppHandle, path: String) -> Result<(), String> {
    let full = crate::native_tools::resolve_sandbox_export_file_path(&path)?;
    let path_str = full.to_string_lossy().into_owned();
    app.shell().open(path_str, None).map_err(|e| e.to_string())
}

/// 在文件管理器中定位生成产物。
#[tauri::command]
pub(crate) fn chat_reveal_generated_artifact(path: String) -> Result<(), String> {
    let full = crate::native_tools::resolve_sandbox_export_file_path(&path)?;
    crate::dock::fs::reveal_file_in_manager(&full)
}

/// 定位对话中持久化的附件；复用预览/打开附件的路径解析。
#[tauri::command]
pub(crate) fn chat_reveal_attachment(
    app: AppHandle,
    conversation_id: Option<String>,
    path: String,
) -> Result<(), String> {
    let full = resolve_attachment_file_path(&app, conversation_id.as_deref(), &path)?;
    crate::dock::fs::reveal_file_in_manager(&full)
}

#[tauri::command]
pub(crate) fn chat_save_pasted_image(
    name: String,
    mime_type: String,
    data_base64: String,
) -> Result<serde_json::Value, String> {
    match save_pasted_image(&name, &mime_type, &data_base64)? {
        PastedImageSave::Saved {
            path,
            name,
            mime_type,
        } => Ok(serde_json::json!({
            "success": true,
            "path": path.to_string_lossy(),
            "name": name,
            "mimeType": mime_type,
        })),
        PastedImageSave::Failed { error } => Ok(serde_json::json!({
            "success": false,
            "error": error,
        })),
    }
}

#[tauri::command]
pub(crate) fn chat_save_pasted_attachment(
    name: String,
    data_base64: String,
) -> Result<serde_json::Value, String> {
    match save_pasted_attachment(&name, &data_base64)? {
        PastedAttachmentSave::Saved { path, name } => Ok(serde_json::json!({
            "success": true,
            "path": path.to_string_lossy(),
            "name": name,
        })),
        PastedAttachmentSave::Failed { error } => Ok(serde_json::json!({
            "success": false,
            "error": error,
        })),
    }
}

/// 读取系统剪贴板中的文件路径（Finder / 资源管理器复制文件）。
#[tauri::command]
pub(crate) fn chat_read_clipboard_files() -> Result<serde_json::Value, String> {
    use arboard::Clipboard;

    let mut clipboard = Clipboard::new().map_err(|e| format!("读取剪贴板失败: {e}"))?;
    let paths = match clipboard.get().file_list() {
        Ok(paths) => paths,
        Err(_) => {
            return Ok(serde_json::json!({
                "success": true,
                "files": [],
            }));
        }
    };

    let files: Vec<Value> =
        inspect_attachment_sources(paths.iter().map(|path| path.to_string_lossy().into_owned()))
            .into_iter()
            .filter_map(|item| serde_json::to_value(item).ok())
            .collect();

    Ok(serde_json::json!({
        "success": true,
        "files": files,
    }))
}

#[tauri::command]
pub(crate) fn chat_inspect_attachment_paths(paths: Vec<String>) -> Vec<InspectedAttachment> {
    inspect_attachment_sources(paths)
}

// 菜单粘贴的同步快路只认输入框规则的安全子集：制表符或 <table 单词边界。
// 完整选择规则留在输入框；这里仅跳过明确无用的 PNG 编码，不扩大文本优先范围。
fn has_spreadsheet_clipboard_text(text: &str, html: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    if text.contains('\t') {
        return true;
    }
    let html = html.to_ascii_lowercase();
    html.match_indices("<table").any(|(index, _)| {
        html.as_bytes()
            .get(index + "<table".len())
            .is_some_and(|next| !next.is_ascii_alphanumeric() && *next != b'_')
    })
}

/// Explicit paste action in the desktop editor. Read through the OS clipboard,
/// never through WebView clipboard permissions. File lists take priority; image
/// results also carry text/HTML so the composer can prefer spreadsheet cells.
#[tauri::command]
pub(crate) async fn chat_read_clipboard() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        use base64::Engine;
        use image::{ExtendedColorType, ImageEncoder};
        use image::codecs::png::{CompressionType, FilterType, PngEncoder};

        let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        if let Ok(paths) = clipboard.get().file_list() {
            let paths: Vec<_> = paths
                .into_iter()
                .filter(|path| path.is_file() || path.is_dir())
                .collect();
            if !paths.is_empty() {
                return Ok(serde_json::json!({ "kind": "files", "paths": paths }));
            }
        }
        let text_result = clipboard.get_text();
        let html = if matches!(text_result, Ok(ref text) if !text.is_empty()) {
            clipboard.get().html().ok().unwrap_or_default()
        } else {
            String::new()
        };
        if let Ok(ref text) = text_result {
            if has_spreadsheet_clipboard_text(text, &html) {
                return Ok(serde_json::json!({ "kind": "text", "text": text }));
            }
        }
        match clipboard.get_image() {
            Ok(image) => {
                let width = u32::try_from(image.width).map_err(|e| e.to_string())?;
                let height = u32::try_from(image.height).map_err(|e| e.to_string())?;
                let mut png = Vec::new();
                PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::NoFilter)
                    .write_image(&image.bytes, width, height, ExtendedColorType::Rgba8)
                    .map_err(|e| e.to_string())?;
                return Ok(serde_json::json!({
                    "kind": "image", "dataBase64": base64::engine::general_purpose::STANDARD.encode(png),
                    "text": text_result.as_ref().ok(), "html": html,
                }));
            }
            Err(arboard::Error::ContentNotAvailable) => {}
            Err(error) => return Err(error.to_string()),
        }
        match text_result {
            Ok(text) => Ok(serde_json::json!({ "kind": "text", "text": text })),
            Err(arboard::Error::ContentNotAvailable) => Ok(serde_json::json!({ "kind": "empty" })),
            Err(error) => Err(error.to_string()),
        }
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn chat_write_clipboard_text(text: String) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod spreadsheet_clipboard_tests {
    use super::has_spreadsheet_clipboard_text;

    #[test]
    fn host_fast_path_only_accepts_clear_table_evidence() {
        assert!(has_spreadsheet_clipboard_text("A\tB", ""));
        assert!(has_spreadsheet_clipboard_text(
            "A",
            "<table><tr><td>A</td></tr></table>"
        ));
        assert!(has_spreadsheet_clipboard_text(
            "A",
            "<table class=\"excel\">"
        ));
        assert!(!has_spreadsheet_clipboard_text("A", "<tableau>"));
        assert!(!has_spreadsheet_clipboard_text("", "<table>"));
    }
}
