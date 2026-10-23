//! Persist staged Zeron bytes using the existing Kivio attachment transaction.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use kivio_model_runtime::attachment_files::{
    self, AttachmentBatch, PastedAttachmentSave, TemporaryAttachmentFiles,
};

pub(super) fn prepare(directory: PathBuf, params: &Value) -> Result<AttachmentBatch, RpcError> {
    let mut temporary = TemporaryAttachmentFiles::new();
    if let Some(items) = params.get("attachments") {
        let items = items
            .as_array()
            .ok_or_else(|| failed("invalid attachments"))?;
        if items.len() > 32 {
            return Err(failed("单次最多添加 32 个附件"));
        }
        for item in items {
            let name = item["name"]
                .as_str()
                .ok_or_else(|| failed("attachment name missing"))?;
            let data = item["data"]
                .as_str()
                .ok_or_else(|| failed("attachment data missing"))?;
            if data.len() > 32 * 1024 * 1024 {
                return Err(failed("附件超过 24 MB"));
            }
            match attachment_files::save_pasted_attachment(name, data).map_err(failed)? {
                PastedAttachmentSave::Saved { path, .. } => temporary.push(path),
                PastedAttachmentSave::Failed { error } => return Err(failed(error)),
            }
        }
    }
    let batch = AttachmentBatch::prepare(
        directory.clone(),
        temporary
            .paths()
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    )
    .map_err(failed)?;
    attachment_files::validate_attachment_media(&directory, batch.attachments()).map_err(failed)?;
    Ok(batch)
}

pub(super) fn read(directory: &std::path::Path, params: &Value) -> Result<RpcReply, RpcError> {
    use std::io::{Read, Seek, SeekFrom};
    let path = params["path"]
        .as_str()
        .ok_or_else(|| failed("path missing"))?;
    let path = std::fs::canonicalize(path).map_err(failed)?;
    let catalog = std::fs::canonicalize(directory.join("conversations")).map_err(failed)?;
    let parent = path
        .parent()
        .ok_or_else(|| failed("invalid attachment path"))?;
    if parent.parent() != Some(catalog.as_path())
        || !parent
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|s| s.ends_with("_attachments"))
    {
        return Err(failed("文件不属于当前 Kivio 附件目录"));
    }
    let mut file = std::fs::File::open(&path).map_err(failed)?;
    let size = file.metadata().map_err(failed)?.len();
    if size > 24 * 1024 * 1024 {
        return Err(failed("附件超过 24 MB"));
    }
    let offset = params["offset"].as_u64().unwrap_or(0);
    if offset > size {
        return Err(failed("invalid attachment offset"));
    }
    file.seek(SeekFrom::Start(offset)).map_err(failed)?;
    // Multiple of three so concatenated base64 chunks remain valid.
    let mut bytes = vec![0; (size - offset).min(192 * 1024) as usize];
    file.read_exact(&mut bytes).map_err(failed)?;
    let next = offset + bytes.len() as u64;
    let name = path.file_name().unwrap().to_string_lossy();
    let mime = match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "md" => "text/plain",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    };
    RpcReply::value(
        &json!({"name":name,"mimeType":mime,"data":STANDARD.encode(bytes),"nextOffset":next,"done":next == size}),
    )
}
