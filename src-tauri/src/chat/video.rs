//! Local video inputs. Bytes are read only for the sending view, never stored in chat JSON.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use std::{fs::File, io::Read, path::Path};

/// Leaves room for base64 expansion inside a 20 MB request.
pub(crate) const MAX_VIDEO_BYTES: usize = 14 * 1024 * 1024;
// Generate Content documents 20 MB, not 20 MiB. Use decimal bytes conservatively.
pub(crate) const MAX_VIDEO_REQUEST_BYTES: usize = 20_000_000;

pub(crate) fn has_video(request: &super::model::GenerateRequest) -> bool {
    request
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .any(|p| matches!(p, super::model::MessagePart::Video { .. }))
}

pub(crate) fn mime_for_name(name: &str) -> Option<&'static str> {
    let extension = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "mp4" => "video/mp4",
        "mpeg" | "mpg" => "video/mpeg",
        "mov" => "video/mov",
        "avi" => "video/avi",
        "flv" => "video/x-flv",
        "webm" => "video/webm",
        "wmv" => "video/wmv",
        "3gp" | "3gpp" => "video/3gpp",
        _ => return None,
    })
}

pub(crate) fn content_part(path: &Path, remaining: &mut usize) -> Result<Value, String> {
    let mime = mime_for_name(&path.to_string_lossy()).ok_or("不支持的视频格式")?;
    let file = File::open(path).map_err(|e| format!("无法读取视频 {}: {e}", path.display()))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size == 0 || size > *remaining as u64 {
        return Err(
            "视频为空或过大：当前上下文的视频总大小不能超过 14 MiB，请裁剪视频或清空上下文。"
                .into(),
        );
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(*remaining as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > *remaining {
        return Err("视频文件大小发生变化，请重新添加视频。".into());
    }
    *remaining -= bytes.len();
    Ok(
        json!({"type":"video_url", "video_url":{"url":format!("data:{mime};base64,{}", STANDARD.encode(bytes))}}),
    )
}

pub(crate) fn validate_request(
    provider: &crate::settings::ModelProvider,
    request: &super::model::GenerateRequest,
) -> Result<(), super::model::ModelError> {
    if !has_video(request) {
        return Ok(());
    }
    validate_model(provider, &request.model)
}

pub(crate) fn validate_model(
    provider: &crate::settings::ModelProvider,
    model: &str,
) -> Result<(), super::model::ModelError> {
    use super::model::ModelError;
    use crate::settings::ProviderApiFormat;
    if !matches!(
        provider.api_format_kind(),
        ProviderApiFormat::OpenAiChat | ProviderApiFormat::Gemini
    ) || provider.request.oauth.is_some()
    {
        return Err(ModelError::new(
            "视频输入需要 API Key 供应商，使用 Kimi OpenAI Chat 或 Gemini 原生协议。",
        ));
    }
    if super::model_metadata::model_supports_video(provider, model) != Some(true) {
        return Err(ModelError::new(format!(
            "模型 {model} 未启用视频输入，请选择支持视频的模型，或在模型详情中启用视频输入。"
        )));
    }
    Ok(())
}

pub(crate) fn validate_body(body: &Value) -> Result<(), super::model::ModelError> {
    // A counting writer avoids allocating another full copy of the encoded video.
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, body)
        .map_err(|e| super::model::ModelError::new(e.to_string()))?;
    if counter.0 > MAX_VIDEO_REQUEST_BYTES {
        return Err(super::model::ModelError::new(
            "包含视频的请求超过 20 MB，请裁剪视频或减少附件及上下文。",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_body_limit_counts_json_bytes_in_decimal_mb() {
        // JSON quotes also count: the first body is exactly 20,000,000 bytes.
        assert!(validate_body(&Value::String("x".repeat(19_999_998))).is_ok());
        assert!(validate_body(&Value::String("x".repeat(19_999_999))).is_err());
    }
}
