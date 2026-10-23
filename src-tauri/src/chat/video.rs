#[path = "video_transport.rs"]
mod transport;
pub(crate) use transport::{
    has_video, reject_unimplemented_transport, validate_body, validate_model, validate_request,
};
// Local video inputs. Bytes are read only for the sending view, never stored in chat JSON.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{fs::File, io::Read, path::Path};

/// Leaves room for base64 expansion inside a 20 MB request.
pub(crate) const MAX_VIDEO_BYTES: usize = 14 * 1024 * 1024;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_model_gate_depends_on_capability_not_provider_or_auth() {
        for format in [
            "openai_chat",
            "gemini",
            "anthropic_messages",
            "openai_responses",
            "xai_responses",
        ] {
            for oauth in [Value::Null, json!({"provider": "kimi"})] {
                for capability in [Value::Null, json!(false), json!(true)] {
                    let provider = serde_json::from_value(json!({
                        "id": "custom", "name": "Custom", "baseUrl": "https://relay.example",
                        "apiFormat": format, "request": {"oauth": oauth},
                        "modelOverrides": {"private-video-model": {"capabilities": {"videoInput": capability}}}
                    })).unwrap();
                    let result = validate_model(&provider, "private-video-model");
                    assert_eq!(
                        result.is_ok(),
                        capability == json!(true),
                        "{format}: {result:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn video_body_limit_counts_json_bytes_in_decimal_mb() {
        // JSON quotes also count: the first body is exactly 20,000,000 bytes.
        assert!(validate_body(&Value::String("x".repeat(19_999_998))).is_ok());
        assert!(validate_body(&Value::String("x".repeat(19_999_999))).is_err());
    }
}
