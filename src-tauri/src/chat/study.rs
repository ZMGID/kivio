//! Isolated, tool-free Study requests over the existing provider adapters.
//! No chat conversation, agent tools, memory, or Lens global stream is involved.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, State, WebviewWindow};
use tokio::sync::Notify;

use super::model::{
    stream_with_chat_provider, GenerateOptions, GenerateRequest, MessagePart, ModelError,
    ModelErrorKind, ModelMessage, ModelRole, RequestMetadata, StreamPart, StreamSink,
};
use super::model_metadata::{
    chat_max_output_tokens_on_wire, model_can_generate_images_directly, model_supports_vision,
};
use crate::settings::ModelProvider;
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum StudyHistoryRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StudyHistoryMessage {
    role: StudyHistoryRole,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StudyCompletionInput {
    request_id: String,
    transport_id: String,
    provider_id: String,
    model: String,
    system_prompt: String,
    user_prompt: String,
    #[serde(default)]
    history: Vec<StudyHistoryMessage>,
    image_data_url: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub(crate) enum StudyStreamEvent {
    Started {
        request_id: String,
        transport_id: String,
    },
    Delta {
        request_id: String,
        transport_id: String,
        delta: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StudyCompletionResult {
    request_id: String,
    content: String,
}

type RequestKey = (String, String);
type ActiveRequests = Mutex<HashMap<RequestKey, Arc<Notify>>>;

fn active_requests() -> &'static ActiveRequests {
    static REQUESTS: OnceLock<ActiveRequests> = OnceLock::new();
    REQUESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct StudyRequestGuard {
    key: RequestKey,
    cancelled: Arc<Notify>,
}

impl StudyRequestGuard {
    fn register(window: &str, transport_id: &str) -> Result<Self, String> {
        let key = (window.to_string(), transport_id.to_string());
        let mut requests = active_requests().lock();
        if requests.contains_key(&key) {
            return Err("This study request is already running.".into());
        }
        let cancelled = Arc::new(Notify::new());
        requests.insert(key.clone(), cancelled.clone());
        Ok(Self { key, cancelled })
    }
}

impl Drop for StudyRequestGuard {
    fn drop(&mut self) {
        active_requests().lock().remove(&self.key);
    }
}

fn cancel_request(window: &str, transport_id: &str) {
    if let Some(cancelled) = active_requests()
        .lock()
        .get(&(window.to_string(), transport_id.to_string()))
    {
        // A permit is retained even when cancellation arrives before select! polls.
        cancelled.notify_one();
    }
}

#[tauri::command]
pub(crate) fn study_cancel_request(window: WebviewWindow, transport_id: String) {
    cancel_request(window.label(), &transport_id);
}

fn validate_input(input: &StudyCompletionInput) -> Result<(), String> {
    if input.request_id.trim().is_empty()
        || input.request_id.len() > 200
        || uuid::Uuid::parse_str(&input.transport_id).is_err()
    {
        return Err("Invalid study request identity.".into());
    }
    if input.system_prompt.trim().is_empty()
        || input.user_prompt.trim().is_empty()
        || input.system_prompt.chars().count() > 12_000
        || input.user_prompt.chars().count() > 100_000
        || input.history.len() > 12
        || input.history.iter().any(|message| message.content.chars().count() > 4_000)
    {
        return Err("Study context is empty or too large. Select a smaller part to study.".into());
    }
    if input.image_data_url.as_deref().is_none_or(|image| image.trim().is_empty()) {
        return Err("Study requires the original page or selected-region image. Wait for it to finish loading.".into());
    }
    Ok(())
}

fn image_part(data_url: &str) -> Result<MessagePart, String> {
    // Images are sent in-memory, never written to an ordinary Chat attachment directory.
    const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
    if data_url.len() > MAX_IMAGE_BYTES * 4 / 3 + 64 {
        return Err("The selected image is too large. Select a smaller region.".into());
    }
    let (header, data) = data_url.split_once(',').ok_or("Invalid study image.")?;
    let (mime, expected_format) = match header {
        "data:image/png;base64" => ("image/png", image::ImageFormat::Png),
        "data:image/jpeg;base64" => ("image/jpeg", image::ImageFormat::Jpeg),
        _ => return Err("Study images must be PNG or JPEG data URLs.".into()),
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "Invalid study image encoding.")?;
    if bytes.len() > MAX_IMAGE_BYTES
        || image::guess_format(&bytes).ok() != Some(expected_format)
    {
        return Err("Invalid or oversized study image.".into());
    }
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "Invalid study image.")?
        .into_dimensions()
        .map_err(|_| "The selected image could not be read.")?;
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 24_000_000 {
        return Err("The selected image is too large. Select a smaller region.".into());
    }
    let prepared = super::image_prep::prepare_image_bytes_for_model(&bytes);
    let (bytes, mime) = match prepared {
        Some((prepared, mime)) => (prepared, mime),
        None => (bytes, mime),
    };
    Ok(MessagePart::Image {
        mime_type: mime.into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        path: None,
        detail: Some("high".into()),
    })
}

fn build_request(
    input: &StudyCompletionInput,
    provider: &ModelProvider,
) -> Result<GenerateRequest, String> {
    validate_input(input)?;
    if !provider.enabled || !provider.enabled_models.contains(&input.model) {
        return Err("Choose an enabled model in Settings before asking for study help.".into());
    }
    if !provider.authentication_ready() {
        return Err("Sign in to the selected model provider in Settings first.".into());
    }
    if model_can_generate_images_directly(provider, &input.model) {
        return Err("Choose a vision-capable model for study help, not an image-generation model.".into());
    }
    // OpenAI Chat merges user model extraBody after the generated request. Reject
    // overrides that could replace the selected context/model or enable server tools.
    // Leave provider settings unchanged and continue allowing harmless vendor knobs.
    if let Some(extra) = provider.model_overrides.get(&input.model)
        .and_then(|info| info.extra_body.as_ref())
        .and_then(|value| value.as_object())
    {
        const RESERVED: &[&str] = &[
            "model", "messages", "input", "system", "instructions", "contents",
            "systemInstruction", "system_instruction", "tools", "tool_choice",
            "toolChoice", "toolConfig", "tool_config", "functions", "function_call",
            "web_search_options", "stream", "stream_options", "previous_response_id",
            "conversation", "conversation_id",
        ];
        if let Some(key) = RESERVED.iter().find(|key| extra.contains_key(**key)) {
            return Err(format!("Study cannot use the model's extra-body override '{key}'. Choose a model configuration without context, tool, or streaming overrides."));
        }
    }
    if model_supports_vision(Some(provider), &input.model) != Some(true) {
        return Err("This model has no confirmed image-input support. Choose a vision-capable model for the original page image.".into());
    }
    let image = input.image_data_url.as_deref().ok_or("Study requires the original page image.")?;
    let mut messages: Vec<ModelMessage> = input
        .history
        .iter()
        .map(|message| ModelMessage::text(
            match message.role {
                StudyHistoryRole::User => ModelRole::User,
                StudyHistoryRole::Assistant => ModelRole::Assistant,
            },
            message.content.clone(),
        ))
        .collect();
    let content = vec![MessagePart::Text { text: input.user_prompt.clone() }, image_part(image)?];
    messages.push(ModelMessage { role: ModelRole::User, content });
    let (thinking_enabled, thinking_level) = super::commands::reasoning::resolve_thinking(
        None, true, Some(provider), &input.model,
    );
    Ok(GenerateRequest {
        model: input.model.clone(),
        system: input.system_prompt.clone(),
        messages,
        tools: Vec::new(),
        options: GenerateOptions {
            max_tokens: chat_max_output_tokens_on_wire(Some(provider), &input.model, 8192).min(8192),
            thinking_enabled,
            thinking_level,
            builtin_web_search: false,
            ..Default::default()
        },
        metadata: RequestMetadata {
            label: "Study help".into(),
            usage_source: Some("study".into()),
            usage_operation: Some("study_help".into()),
            // Do not manufacture a Chat conversation or put source filenames in usage metadata.
            ..Default::default()
        },
    })
}

struct StudySink {
    channel: Channel<StudyStreamEvent>,
    request_id: String,
    transport_id: String,
}

impl StreamSink for StudySink {
    fn emit(&mut self, part: StreamPart) -> Result<(), ModelError> {
        match part {
            StreamPart::TextDelta { delta } => self.channel.send(StudyStreamEvent::Delta {
                request_id: self.request_id.clone(),
                transport_id: self.transport_id.clone(),
                delta,
            }).map_err(|_| ModelError::with_kind("Study view closed.", ModelErrorKind::Cancelled)),
            StreamPart::Error { message } => Err(ModelError::new(message)),
            _ => Ok(()),
        }
    }
}

#[tauri::command]
pub(crate) async fn study_request_help(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: StudyCompletionInput,
    channel: Channel<StudyStreamEvent>,
) -> Result<StudyCompletionResult, String> {
    validate_input(&input)?;
    let guard = StudyRequestGuard::register(window.label(), &input.transport_id)?;
    // Acknowledgement closes the pre-registration cancellation race on the frontend.
    channel.send(StudyStreamEvent::Started {
        request_id: input.request_id.clone(),
        transport_id: input.transport_id.clone(),
    }).map_err(|_| "The study view is no longer available.")?;
    let settings = state.settings_read().clone();
    let provider = settings.get_provider(&input.provider_id)
        .ok_or("The selected study provider is no longer configured.")?
        .clone();
    let request = build_request(&input, &provider)?;
    let retry_attempts = if settings.retry_enabled {
        (settings.retry_attempts as usize).clamp(1, 3)
    } else {
        1
    };
    let mut sink = StudySink {
        channel,
        request_id: input.request_id.clone(),
        transport_id: input.transport_id.clone(),
    };
    let output = tokio::select! {
        biased;
        _ = guard.cancelled.notified() => return Err("Study request cancelled.".into()),
        output = tokio::time::timeout(
            Duration::from_secs(180),
            stream_with_chat_provider(state.inner(), &provider, retry_attempts, request, &mut sink),
        ) => output.map_err(|_| "Study request timed out. Please retry.")?
            .map_err(|error| error.to_string())?,
    };
    if output.cancelled {
        return Err("Study request cancelled.".into());
    }
    if output.text.trim().is_empty() {
        return Err("The model returned no study answer. Please retry or select another model.".into());
    }
    Ok(StudyCompletionResult { request_id: input.request_id, content: output.text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_image() -> String {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()))
    }

    fn input() -> StudyCompletionInput {
        StudyCompletionInput {
            request_id: "original-study-request".into(),
            transport_id: uuid::Uuid::new_v4().to_string(),
            provider_id: "test-provider".into(),
            model: "study-test-model".into(),
            system_prompt: "Give one hint, never the final answer.".into(),
            user_prompt: "Page 2: x + 3 = 5".into(),
            history: vec![],
            image_data_url: Some(page_image()),
        }
    }

    fn provider() -> ModelProvider {
        serde_json::from_value(serde_json::json!({
            "id": "test-provider", "name": "Mock provider", "apiKeys": [],
            "baseUrl": "http://localhost:1/v1", "availableModels": ["study-test-model"],
            "enabledModels": ["study-test-model"], "enabled": true, "apiFormat": "openai_chat",
            "modelOverrides": { "study-test-model": { "capabilities": { "vision": true } } }
        })).unwrap()
    }

    #[test]
    fn request_has_no_tools_web_search_or_chat_conversation() {
        let mut input = input();
        input.history.push(StudyHistoryMessage { role: StudyHistoryRole::Assistant, content: "Try isolating x.".into() });
        let request = build_request(&input, &provider()).unwrap();
        assert!(request.tools.is_empty());
        assert!(!request.options.builtin_web_search);
        assert!(request.metadata.conversation_id.is_none());
        assert_eq!(request.messages.len(), 2);
        assert_eq!(request.messages[0].role, ModelRole::Assistant);
        assert_eq!(request.messages[1].role, ModelRole::User);
        assert_eq!(request.system, input.system_prompt);
        assert_eq!(request.messages[1].content.len(), 2);
        assert!(matches!(request.messages[1].content[1], MessagePart::Image { path: None, .. }));
    }

    #[test]
    fn rejects_disabled_models_and_unconfirmed_vision_before_any_network_call() {
        let mut provider = provider();
        provider.enabled = false;
        assert!(build_request(&input(), &provider).unwrap_err().contains("enabled model"));
        provider.enabled = true;
        provider.model_overrides.clear();
        assert!(build_request(&input(), &provider).unwrap_err().contains("image-input support"));
        provider.model_overrides.insert("study-test-model".into(), serde_json::from_value(serde_json::json!({ "capabilities": { "vision": false } })).unwrap());
        assert!(build_request(&input(), &provider).unwrap_err().contains("image-input support"));
    }

    #[test]
    fn rejects_missing_image_even_when_text_and_vision_are_available() {
        for image in [None, Some(String::new()), Some("   ".into())] {
            let mut input = input();
            input.image_data_url = image;
            assert!(validate_input(&input).unwrap_err().contains("original page"));
            assert!(build_request(&input, &provider()).unwrap_err().contains("original page"));
        }
    }

    #[test]
    fn rejects_extra_body_context_and_tool_bypasses_without_changing_settings() {
        let mut provider = provider();
        let settings: crate::settings::ModelInfo = serde_json::from_value(serde_json::json!({
            "extraBody": { "tools": [{ "type": "web_search" }] },
            "capabilities": { "vision": true }
        })).unwrap();
        provider.model_overrides.insert("study-test-model".into(), settings);
        assert!(build_request(&input(), &provider).unwrap_err().contains("extra-body override 'tools'"));
        assert!(provider.model_overrides["study-test-model"].extra_body.as_ref().unwrap().get("tools").is_some());
        provider.model_overrides.get_mut("study-test-model").unwrap().extra_body = Some(serde_json::json!({ "chat_template_kwargs": { "enable_thinking": true } }));
        assert!(build_request(&input(), &provider).is_ok());
    }

    #[test]
    fn rejects_unbounded_context_and_invalid_image_data() {
        let mut input = input();
        input.history = (0..13).map(|_| StudyHistoryMessage { role: StudyHistoryRole::User, content: "prior turn".into() }).collect();
        assert!(validate_input(&input).is_err());
        assert!(image_part("https://example.com/page.png").is_err());
        assert!(image_part("data:image/png;base64,AAAA").is_err());
    }

    #[test]
    fn image_is_an_in_memory_model_part() {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes.into_inner()));
        assert!(matches!(image_part(&url).unwrap(), MessagePart::Image { path: None, .. }));
    }

    #[tokio::test]
    async fn cancellation_is_scoped_to_window_and_attempt_and_cleanup_allows_retry() {
        let transport_id = uuid::Uuid::new_v4().to_string();
        let first = StudyRequestGuard::register("study-window", &transport_id).unwrap();
        assert!(StudyRequestGuard::register("study-window", &transport_id).is_err());
        cancel_request("other-window", &transport_id);
        assert!(tokio::time::timeout(Duration::from_millis(5), first.cancelled.notified()).await.is_err());
        cancel_request("study-window", &transport_id);
        // Cancellation issued before awaiting still wakes the request.
        tokio::time::timeout(Duration::from_millis(50), first.cancelled.notified()).await.unwrap();
        drop(first);
        let retry = StudyRequestGuard::register("study-window", &transport_id).unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(5), retry.cancelled.notified()).await.is_err());
        drop(retry);
        assert!(!active_requests().lock().contains_key(&("study-window".into(), transport_id)));
    }
}
