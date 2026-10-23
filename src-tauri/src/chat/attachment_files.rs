//! Existing attachment filesystem, previews and request hints without a window host.
//! Blocking file operations are run on a worker by native callers.
use super::conversation_data::Attachment;
use base64::{Engine as _, engine::general_purpose};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
use uuid::Uuid;
const ARTIFACT_THUMBNAIL_MAX_DIM: u32 = 256;

const MAX_ATTACHMENT_PREVIEW_BYTES: u64 = 12 * 1024 * 1024;
const MAX_PASTED_IMAGE_BYTES: usize = 12 * 1024 * 1024;
const MAX_PASTED_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

/// 内存文本附件（粘贴长文本自动生成的虚拟 txt）：正文只存在于请求内存、不落盘。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextAttachmentInput {
    pub name: String,
    pub content: String,
}

/// 判断是否为内存虚拟文本附件：path 以 `memory://` 标记（正文不落盘，仅持久化附件记录）。
pub fn is_memory_text_attachment(attachment: &Attachment) -> bool {
    attachment.path.starts_with("memory://")
}

pub enum PastedImageSave {
    Saved {
        path: PathBuf,
        name: String,
        mime_type: &'static str,
    },
    Failed {
        error: String,
    },
}

pub enum PastedAttachmentSave {
    Saved { path: PathBuf, name: String },
    Failed { error: String },
}

pub fn save_pasted_image(
    name: &str,
    mime_type: &str,
    data_base64: &str,
) -> Result<PastedImageSave, String> {
    let mime = normalize_pasted_image_mime(mime_type)?;
    let ext = extension_for_image_mime(mime);
    let mut safe_name = sanitize_attachment_name(name);
    if attachment_type_for_name(&safe_name) != "image" {
        safe_name = format!("{safe_name}.{ext}");
    }

    let payload = data_base64.trim();
    if payload.is_empty() {
        return Ok(PastedImageSave::Failed {
            error: "剪贴板图片为空".to_string(),
        });
    }

    let bytes = match general_purpose::STANDARD.decode(payload) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Ok(PastedImageSave::Failed {
                error: format!("解析剪贴板图片失败: {err}"),
            });
        }
    };
    if bytes.len() > MAX_PASTED_IMAGE_BYTES {
        return Ok(PastedImageSave::Failed {
            error: "剪贴板图片过大，无法添加".to_string(),
        });
    }

    let (path, saved_name) = write_pasted_attachment_bytes(&safe_name, &bytes)
        .map_err(|e| format!("保存剪贴板图片失败: {e}"))?;

    Ok(PastedImageSave::Saved {
        path,
        name: saved_name,
        mime_type: mime,
    })
}

pub fn save_pasted_attachment(
    name: &str,
    data_base64: &str,
) -> Result<PastedAttachmentSave, String> {
    let safe_name = sanitize_attachment_name(name);
    if !is_attachable_file_name(&safe_name) {
        return Ok(PastedAttachmentSave::Failed {
            error: "无效的文件名".to_string(),
        });
    }

    let payload = data_base64.trim();
    if payload.is_empty() {
        return Ok(PastedAttachmentSave::Failed {
            error: "剪贴板附件为空".to_string(),
        });
    }

    let bytes = match general_purpose::STANDARD.decode(payload) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Ok(PastedAttachmentSave::Failed {
                error: format!("解析剪贴板附件失败: {err}"),
            });
        }
    };
    if bytes.len() > MAX_PASTED_ATTACHMENT_BYTES {
        return Ok(PastedAttachmentSave::Failed {
            error: "剪贴板附件过大，无法添加".to_string(),
        });
    }

    let (path, saved_name) = write_pasted_attachment_bytes(&safe_name, &bytes)?;
    Ok(PastedAttachmentSave::Saved {
        path,
        name: saved_name,
    })
}

fn write_pasted_attachment_bytes(name: &str, bytes: &[u8]) -> Result<(PathBuf, String), String> {
    let dir = std::env::temp_dir().join("kivio-chat-paste");
    fs::create_dir_all(&dir).map_err(|e| format!("创建临时附件目录失败: {e}"))?;
    let file_name = format!("paste-{}-{}", Uuid::new_v4(), name);
    let path = dir.join(&file_name);
    fs::write(&path, bytes).map_err(|e| format!("保存剪贴板附件失败: {e}"))?;
    Ok((path, name.to_string()))
}

pub fn is_attachable_file_name(name: &str) -> bool {
    !name.trim().is_empty()
}

fn normalize_pasted_image_mime(mime_type: &str) -> Result<&'static str, String> {
    match mime_type.trim().to_ascii_lowercase().as_str() {
        "image/png" => Ok("image/png"),
        "image/jpeg" | "image/jpg" => Ok("image/jpeg"),
        "image/gif" => Ok("image/gif"),
        "image/webp" => Ok("image/webp"),
        "image/bmp" => Ok("image/bmp"),
        "image/tiff" => Ok("image/tiff"),
        "image/heic" => Ok("image/heic"),
        "image/heif" => Ok("image/heif"),
        _ => Err("仅支持粘贴图片".to_string()),
    }
}

pub(crate) fn extension_for_image_mime(mime_type: &str) -> &'static str {
    match mime_type {
        "image/svg+xml" => "svg",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/heic" => "heic",
        "image/heif" => "heif",
        _ => "png",
    }
}

fn mime_type_for_attachment(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xlsm" => "application/vnd.ms-excel.sheet.macroenabled.12",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "txt" => "text/plain",
        "md" => "text/markdown",
        _ => "application/octet-stream",
    }
}

pub fn read_attachment_as_data_url(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("读取附件信息失败: {e}"))?;
    if metadata.len() > MAX_ATTACHMENT_PREVIEW_BYTES {
        return Err("附件过大，无法在界面内预览".to_string());
    }
    let bytes = fs::read(path).map_err(|e| format!("读取附件失败: {e}"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    let mime = mime_type_for_attachment(file_name);
    let encoded = general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// 用已有的 `image` crate 生成 PNG 缩略图的内联 data URL。解码/编码失败返回 None。
/// 也被 `native_tools::sandbox_exports` 用于成果卡：图片在出生点就只内联缩略图。
pub fn make_thumbnail_data_url(bytes: &[u8]) -> Option<String> {
    Some(format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(make_thumbnail_png(bytes)?)
    ))
}

pub fn make_thumbnail_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = super::image_prep::decode_oriented(bytes)?;
    let thumb = img.thumbnail(ARTIFACT_THUMBNAIL_MAX_DIM, ARTIFACT_THUMBNAIL_MAX_DIM);
    let mut buf = Cursor::new(Vec::new());
    thumb.write_to(&mut buf, image::ImageFormat::Png).ok()?;
    Some(buf.into_inner())
}

pub fn attachment_thumbnail(path: &Path) -> Option<Vec<u8>> {
    if fs::metadata(path).ok()?.len() > MAX_ATTACHMENT_PREVIEW_BYTES {
        return None;
    }
    make_thumbnail_png(&fs::read(path).ok()?)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedAttachment {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub attachment_type: String,
}

pub fn inspect_attachment_sources<I, S>(paths: I) -> Vec<InspectedAttachment>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    paths
        .into_iter()
        .filter_map(|source| inspect_attachment_source(Path::new(source.as_ref())))
        .collect()
}

fn inspect_attachment_source(source_path: &Path) -> Option<InspectedAttachment> {
    let original_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment")
        .to_string();
    let attachment_type = if source_path.is_dir() {
        "folder".to_string()
    } else if source_path.is_file() {
        if !is_attachable_file_name(&original_name) {
            return None;
        }
        attachment_type_for_name(&original_name).to_string()
    } else {
        return None;
    };
    Some(InspectedAttachment {
        path: source_path.to_string_lossy().into_owned(),
        name: original_name,
        attachment_type,
    })
}

/// Persist one dropped/picked path. Files are copied into the conversation
/// attachments dir; folders stay at their native path (no tree copy).
pub(crate) fn persist_source_attachment(
    source_path: &Path,
    dest_dir: &Path,
) -> Result<Attachment, String> {
    let source = source_path.display().to_string();
    if source_path.is_dir() {
        let original_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment");
        return Ok(Attachment {
            id: format!("att_{}", Uuid::new_v4()),
            attachment_type: "folder".to_string(),
            name: original_name.to_string(),
            path: source,
            content: None,
        });
    }
    if !source_path.is_file() {
        return Err(format!("附件不存在或不是文件: {source}"));
    }

    let id = format!("att_{}", Uuid::new_v4());
    let original_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    let safe_name = sanitize_attachment_name(original_name);
    let stored_name = format!("{}-{}", id, safe_name);
    let dest = dest_dir.join(&stored_name);
    if let Err(error) = fs::copy(source_path, &dest) {
        let _ = fs::remove_file(&dest);
        return Err(format!("保存附件失败: {error}"));
    }

    Ok(Attachment {
        id,
        attachment_type: attachment_type_for_name(original_name).to_string(),
        name: original_name.to_string(),
        path: stored_name,
        content: None,
    })
}

pub(crate) fn sanitize_attachment_name(name: &str) -> String {
    // Keep Unicode letters/digits (CJK, etc.). Only replace path separators and
    // other non-identifier junk — ASCII-only used to store 销售报表.xlsx as
    // att_<id>-________.xlsx (or even lose the extension after trim).
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(['.', ' ', '_']).trim();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn attachment_type_for_name(name: &str) -> &'static str {
    if super::video::mime_for_name(name).is_some() {
        return "video";
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" | "heic" | "heif" => {
            "image"
        }
        _ => "file",
    }
}

fn attachment_type_label(attachment_type: &str) -> &'static str {
    match attachment_type {
        "image" => "图片",
        "video" => "视频",
        "folder" => "文件夹",
        _ => "文件",
    }
}

fn is_folder_attachment(attachment: &Attachment) -> bool {
    attachment.attachment_type == "folder"
}

fn attachment_extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .unwrap_or_default()
}

fn attachment_skill_for_name(name: &str) -> Option<&'static str> {
    match attachment_extension(name).as_str() {
        "pdf" => Some("pdf"),
        "doc" | "docx" => Some("docx"),
        "xls" | "xlsx" | "xlsm" | "csv" | "tsv" => Some("xlsx"),
        _ => None,
    }
}

fn attachment_format_label(attachment: &Attachment) -> &'static str {
    if attachment.attachment_type == "image" {
        return "图片";
    }
    if is_folder_attachment(attachment) {
        return "文件夹";
    }

    match attachment_extension(&attachment.name).as_str() {
        "pdf" => "PDF",
        "doc" | "docx" => "Word 文档",
        "xls" | "xlsx" | "xlsm" => "Excel 工作簿",
        "csv" => "CSV 表格",
        "tsv" => "TSV 表格",
        "txt" | "md" => "文本文件",
        _ => attachment_type_label(&attachment.attachment_type),
    }
}

fn stored_attachment_path_for_prompt(
    attachment: &Attachment,
    attachment_dir: Option<&Path>,
) -> String {
    let stored = Path::new(&attachment.path);
    if stored.is_absolute() || is_folder_attachment(attachment) {
        return attachment.path.clone();
    }
    attachment_dir
        .map(|dir| dir.join(stored).display().to_string())
        .unwrap_or_else(|| attachment.path.clone())
}

fn attachment_processing_hint(attachment: &Attachment) -> String {
    if is_folder_attachment(attachment) {
        return "此文件夹按本机路径附加，未复制。用 glob/read/grep 等工具按该绝对路径查看内容，不要仅凭文件夹名臆测。".to_string();
    }
    if attachment.attachment_type == "video" {
        return "内置运行时会将视频内容发送给支持视频输入的模型。".to_string();
    }
    if attachment.attachment_type == "image" {
        return "图片附件会随本轮请求发送给视觉模型。".to_string();
    }

    if let Some(skill) = attachment_skill_for_name(&attachment.name) {
        format!(
            "推荐复用现成 `{skill}` Skill：需要读取或分析该文件时，先调用 skill(name=\"{skill}\")，再按该 Skill 的 SKILL.md / reference / scripts 流程处理安全副本路径。"
        )
    } else {
        "此文件已保存为 Kivio 安全副本；仅在有可用读取工具或对应 Skill 时处理正文。".to_string()
    }
}

pub fn compose_user_content_for_api(
    content: &str,
    attachments: &[Attachment],
    attachment_dir: Option<&Path>,
) -> String {
    let trimmed = content.trim();
    // 虚拟文本附件（memory://）正文已由 text_attachments 通道内联进 API content，
    // 这里过滤掉，避免再输出一段「Kivio 安全副本路径」造成重复。
    let real_attachments: Vec<&Attachment> = attachments
        .iter()
        .filter(|attachment| !is_memory_text_attachment(attachment))
        .collect();
    if real_attachments.is_empty() {
        return trimmed.to_string();
    }

    let has_images = real_attachments
        .iter()
        .any(|attachment| attachment.attachment_type == "image");
    let has_files = real_attachments
        .iter()
        .any(|attachment| !matches!(attachment.attachment_type.as_str(), "image" | "video"));
    let attachment_lines = real_attachments
        .iter()
        .map(|attachment| {
            let stored_path = stored_attachment_path_for_prompt(attachment, attachment_dir);
            format!(
                "- {} ({})\n  - 附件 ID：{}\n  - Kivio 安全副本路径：{}\n  - 处理建议：{}",
                attachment.name,
                attachment_format_label(attachment),
                attachment.id,
                stored_path,
                attachment_processing_hint(attachment)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let capability_note = match (has_images, has_files) {
        (true, true) => {
            "图片附件会随本轮请求发送给视觉模型；文档/表格附件不会直接随模型请求内联正文，必须复用对应 Agent Skill 或可用工具实际读取安全副本后再分析。"
        }
        (true, false) => "图片附件会随本轮请求发送给视觉模型。",
        (false, true) => {
            "文档/表格附件不会直接随模型请求内联正文，必须复用对应 Agent Skill 或可用工具实际读取安全副本后再分析；不要仅凭文件名臆测内容。"
        }
        (false, false) => "",
    };
    let attachment_note = format!(
        "[已添加附件]\n{}\n\n注意：{}",
        attachment_lines, capability_note
    );
    let attachment_note = if real_attachments
        .iter()
        .any(|a| a.attachment_type == "video")
    {
        format!("{attachment_note}\n视频附件随内置运行时请求发送；外部 CLI 代理仅收到文件路径。")
    } else {
        attachment_note
    };

    if trimmed.is_empty() {
        attachment_note
    } else {
        format!("{trimmed}\n\n{attachment_note}")
    }
}

/// 把内存文本附件（虚拟 txt）正文直接内联进 API 内容：模型直接可见、无需工具读取，
/// 与磁盘附件的「安全副本 + 工具读取」路径不同。虚拟附件不落盘、不持久化到历史消息。
pub fn compose_text_attachments_for_api(
    content: &str,
    text_attachments: &[TextAttachmentInput],
) -> String {
    if text_attachments.is_empty() {
        return content.to_string();
    }

    let attachment_blocks = text_attachments
        .iter()
        .map(|attachment| {
            format!(
                "--- 文本附件：{} ---\n{}",
                attachment.name, attachment.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let note = format!(
        "[已添加文本附件]\n{}\n\n注意：文本附件正文已直接内联在本消息中，直接阅读使用即可，无需工具读取。",
        attachment_blocks
    );

    let trimmed = content.trim();
    if trimmed.is_empty() {
        note
    } else {
        format!("{trimmed}\n\n{note}")
    }
}

/// 从已持久化的附件记录还原虚拟文本附件（正文只存在 `attachment.content` 里，
/// 落库消息的原始 content 不含正文）：发送失败重试 / 重新生成时据此重建请求体。
pub fn text_attachments_from_attachments(attachments: &[Attachment]) -> Vec<TextAttachmentInput> {
    attachments
        .iter()
        .filter_map(|attachment| {
            attachment
                .content
                .as_ref()
                .map(|content| TextAttachmentInput {
                    name: attachment.name.clone(),
                    content: content.clone(),
                })
        })
        .collect()
}

pub fn title_source_for_user_message(content: &str, attachments: &[Attachment]) -> String {
    let trimmed = content.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }

    let names = attachments
        .iter()
        .map(|attachment| attachment.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if names.is_empty() {
        String::new()
    } else {
        format!("附件: {names}")
    }
}

pub fn resolve_attachment_file_path_in_dir(
    directory: Option<&Path>,
    path: &str,
) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("附件路径为空".to_string());
    }

    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        if candidate.is_file() || candidate.is_dir() {
            return Ok(candidate);
        }
        return Err(format!("附件不存在: {path}"));
    }

    if let Some(dir) = directory {
        if path.contains('/') || path.contains('\\') {
            return Err("无效的附件路径".to_string());
        }
        let full = dir.join(path);
        if !full.is_file() && !full.is_dir() {
            return Err(format!("附件不存在: {path}"));
        }
        return Ok(full);
    }

    if !candidate.is_file() && !candidate.is_dir() {
        return Err(format!("文件不存在: {path}"));
    }
    Ok(candidate)
}

pub fn save_message_attachments_in_dir(
    dir: &Path,
    attachment_paths: Vec<String>,
) -> Result<Vec<Attachment>, String> {
    Ok(AttachmentBatch::prepare(dir.to_path_buf(), attachment_paths)?.commit())
}

/// A not-yet-submitted copy batch. Cancellation, a failed media check, or a
/// dropped UI task removes only newly copied files, never the user's source paths.
pub struct AttachmentBatch {
    directory: PathBuf,
    attachments: Option<Vec<Attachment>>,
}
impl AttachmentBatch {
    pub fn prepare(directory: PathBuf, paths: Vec<String>) -> Result<Self, String> {
        let mut batch = Self {
            directory,
            attachments: Some(Vec::new()),
        };
        if !paths.is_empty() {
            fs::create_dir_all(&batch.directory)
                .map_err(|error| format!("创建附件目录失败: {error}"))?;
        }
        for path in paths {
            let attachment = persist_source_attachment(Path::new(&path), &batch.directory)?;
            batch.attachments.as_mut().unwrap().push(attachment);
        }
        Ok(batch)
    }
    pub fn attachments(&self) -> &[Attachment] {
        self.attachments.as_deref().unwrap_or_default()
    }
    pub fn commit(mut self) -> Vec<Attachment> {
        self.attachments.take().unwrap_or_default()
    }
}
impl Drop for AttachmentBatch {
    fn drop(&mut self) {
        if let Some(attachments) = &self.attachments {
            for attachment in attachments {
                let path = Path::new(&attachment.path);
                if attachment.attachment_type != "folder"
                    && !path.is_absolute()
                    && path.components().count() == 1
                {
                    let _ = fs::remove_file(self.directory.join(path));
                }
            }
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

pub fn stored_image_paths_for_attachments_in_dir(
    dir: &Path,
    attachments: &[Attachment],
) -> Result<Vec<PathBuf>, String> {
    let image_attachments = attachments
        .iter()
        .filter(|attachment| attachment.attachment_type == "image")
        .collect::<Vec<_>>();
    if image_attachments.is_empty() {
        return Ok(Vec::new());
    }

    image_attachments
        .into_iter()
        .map(|attachment| {
            let stored = Path::new(&attachment.path);
            if stored.components().count() != 1 {
                return Err(format!("Invalid attachment path: {}", attachment.path));
            }
            let path = dir.join(stored);
            if !path.is_file() {
                return Err(format!("图片附件不存在: {}", attachment.name));
            }
            Ok(path)
        })
        .collect()
}

pub fn stored_file_paths_for_attachments_in_dir(
    dir: &Path,
    attachments: &[Attachment],
) -> Result<Vec<PathBuf>, String> {
    let file_attachments = attachments
        .iter()
        .filter(|attachment| attachment.attachment_type != "image")
        .collect::<Vec<_>>();
    if file_attachments.is_empty() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for attachment in file_attachments {
        // 虚拟文本附件（memory://）无磁盘文件，跳过。
        if is_memory_text_attachment(attachment) {
            continue;
        }
        let stored = Path::new(&attachment.path);
        let path = if stored.is_absolute() || is_folder_attachment(attachment) {
            stored.to_path_buf()
        } else {
            if stored.components().count() != 1 {
                return Err(format!("Invalid attachment path: {}", attachment.path));
            }
            dir.join(stored)
        };
        if path.is_file() || path.is_dir() {
            paths.push(path);
        }
    }
    Ok(paths)
}

pub fn image_bytes_content_part(bytes: &[u8]) -> Result<serde_json::Value, String> {
    // 入口降采样（治本）：超尺寸/超体积的图在进上下文前收敛到有界大小，
    // 历史层的字节预算（compaction::prune_image_parts）只剩保险作用。
    let (bytes, mime) = match super::image_prep::prepare_image_bytes_for_model(bytes)? {
        Some((processed, mime)) => (processed, mime),
        None => (
            bytes.to_vec(),
            image::guess_format(bytes)
                .map_err(|e| format!("图片格式无效: {e}"))?
                .to_mime_type(),
        ),
    };
    let base64 = general_purpose::STANDARD.encode(bytes);
    Ok(serde_json::json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{mime};base64,{base64}") },
    }))
}

/// Prepare the same user wire content used by the original host. Documents stay
/// referenced through their safe path and tool/Skill hints; only images/video and
/// explicit memory text become inline model input.
pub fn user_message_parts(
    content: &str,
    attachments: &[Attachment],
    directory: &Path,
    remaining_video_bytes: &mut usize,
) -> Result<Vec<serde_json::Value>, String> {
    let text = compose_user_content_for_api(content, attachments, Some(directory));
    let text =
        compose_text_attachments_for_api(&text, &text_attachments_from_attachments(attachments));
    let mut parts = vec![serde_json::json!({"type":"text","text":text})];
    for path in stored_image_paths_for_attachments_in_dir(directory, attachments)? {
        let bytes = fs::read(&path)
            .map_err(|error| format!("读取图片附件失败 {}: {error}", path.display()))?;
        parts.push(image_bytes_content_part(&bytes)?);
    }
    for attachment in attachments
        .iter()
        .filter(|attachment| attachment.attachment_type == "video")
    {
        let path = resolve_attachment_file_path_in_dir(Some(directory), &attachment.path)?;
        parts.push(super::video::content_part(&path, remaining_video_bytes)?);
    }
    Ok(parts)
}

/// Validate new user media before accepting the draft. Disk documents are
/// validated as paths here; their content remains owned by the existing tools.
pub fn validate_attachment_media(
    directory: &Path,
    attachments: &[Attachment],
) -> Result<(), String> {
    let mut budget = super::video::MAX_VIDEO_BYTES;
    user_message_parts("", attachments, directory, &mut budget).map(|_| ())
}

/// Clipboard bytes enter the same original paste limits, naming and temp store.
pub fn save_clipboard_image(name: &str, mime: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    match save_pasted_image(name, mime, &general_purpose::STANDARD.encode(bytes))? {
        PastedImageSave::Saved { path, .. } => Ok(path),
        PastedImageSave::Failed { error } => Err(error),
    }
}

/// Temporary paste files transfer to the pane only after its async callback succeeds.
#[derive(Default)]
pub struct TemporaryAttachmentFiles(Vec<PathBuf>);
impl TemporaryAttachmentFiles {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, path: PathBuf) {
        self.0.push(path);
    }
    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }
    pub fn commit(mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.0)
    }
}
impl Drop for TemporaryAttachmentFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One batch owns only its new copies. A later invalid path and cancellation
    // must not leak earlier copies or remove source files / prior attachments.
    #[test]
    fn uncommitted_attachment_batches_clean_only_new_owned_files() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("原文.txt");
        fs::write(&source, "private document body").unwrap();
        let directory = root.path().join("attachments");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("existing.txt"), "keep").unwrap();
        assert!(
            AttachmentBatch::prepare(
                directory.clone(),
                vec![
                    source.display().to_string(),
                    root.path().join("missing").display().to_string()
                ]
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        {
            let batch =
                AttachmentBatch::prepare(directory.clone(), vec![source.display().to_string()])
                    .unwrap();
            assert_eq!(
                fs::read(directory.join(&batch.attachments()[0].path)).unwrap(),
                b"private document body"
            );
        }
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert_eq!(fs::read(&source).unwrap(), b"private document body");
        let batch = AttachmentBatch::prepare(directory.clone(), vec![source.display().to_string()])
            .unwrap();
        let saved = batch.commit();
        assert!(directory.join(&saved[0].path).is_file());
        let parts =
            user_message_parts("read this", &saved, &directory, &mut (14 * 1024 * 1024)).unwrap();
        let prompt = parts[0]["text"].as_str().unwrap();
        assert!(prompt.contains(&directory.join(&saved[0].path).display().to_string()));
        assert!(
            !prompt.contains("private document body"),
            "disk file bodies remain owned by the existing tools"
        );
    }
}
