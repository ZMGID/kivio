use serde_json::Value;
pub(crate) const MAX_VIDEO_REQUEST_BYTES: usize = 20_000_000;
pub(crate) fn has_video(request: &crate::chat::model::GenerateRequest) -> bool {
    request
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .any(|p| matches!(p, crate::chat::model::MessagePart::Video { .. }))
}

/// Adapters without a video encoder must not silently replace the video with text.
/// This is a transport limitation, separate from model capability and authentication.
pub(crate) fn reject_unimplemented_transport(
    request: &crate::chat::model::GenerateRequest,
    adapter: &str,
) -> Result<(), crate::chat::model::ModelError> {
    if has_video(request) {
        return Err(crate::chat::model::ModelError::new(format!(
            "Kivio 的 {adapter} 适配器尚未实现视频传输；请为此模型配置 OpenAI Chat 或 Gemini 传输方式。"
        )));
    }
    Ok(())
}
pub(crate) fn validate_request(
    provider: &crate::settings::ModelProvider,
    request: &crate::chat::model::GenerateRequest,
) -> Result<(), crate::chat::model::ModelError> {
    if !has_video(request) {
        return Ok(());
    }
    validate_model(provider, &request.model)
}

pub(crate) fn validate_model(
    provider: &crate::settings::ModelProvider,
    model: &str,
) -> Result<(), crate::chat::model::ModelError> {
    use crate::chat::model::ModelError;
    if crate::chat::model_metadata::model_supports_video(provider, model) != Some(true) {
        return Err(ModelError::new(format!(
            "模型 {model} 未启用视频输入，请选择支持视频的模型，或在模型详情中启用视频输入。"
        )));
    }
    Ok(())
}

pub(crate) fn validate_body(body: &Value) -> Result<(), crate::chat::model::ModelError> {
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
        .map_err(|e| crate::chat::model::ModelError::new(e.to_string()))?;
    if counter.0 > MAX_VIDEO_REQUEST_BYTES {
        return Err(crate::chat::model::ModelError::new(
            "包含视频的请求超过 20 MB，请裁剪视频或减少附件及上下文。",
        ));
    }
    Ok(())
}
