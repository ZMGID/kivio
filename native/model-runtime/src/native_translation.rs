//! Native host for the original input translation policy and model adapters.
use crate::{settings::ModelProvider, types::*};
use std::path::Path;
pub async fn translate(root: &Path, text: &str) -> Result<String, String> {
    let settings = crate::native_preferences::load(root)?;
    let Some(prepared) = crate::input_translation::prepare(&settings, text)? else {
        return Ok(String::new());
    };
    execute(
        root,
        &settings,
        prepared,
        false,
        false,
        "translator",
        "translate_text",
        None,
        &mut |_| Ok(()),
    )
    .await
}

pub async fn translate_selection(
    root: &Path,
    text: &str,
    sink: &mut (dyn StreamSink + Send),
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("没有选中的文字".into());
    }
    let settings = crate::native_preferences::load(root)?;
    let config = &settings.screenshot_translation;
    let prepared = crate::input_translation::InputTranslation {
        provider_id: config.provider_id.clone(),
        model: config.model.clone(),
        prompt: crate::prompts::build_selected_text_translation_prompt(
            text.trim(),
            crate::utils::language_name(&settings.reading_target_lang),
            config.text_prompt.as_deref(),
        ),
        attempts: if settings.retry_enabled {
            settings.retry_attempts.clamp(1, 11) as usize
        } else {
            1
        },
    };
    execute(
        root,
        &settings,
        prepared,
        config.thinking_enabled,
        config.stream_enabled,
        "screenshot_translation",
        if config.stream_enabled {
            "text_selection_stream"
        } else {
            "text_selection"
        },
        None,
        sink,
    )
    .await
}

async fn execute(
    root: &Path,
    settings: &crate::settings::Settings,
    prepared: crate::input_translation::InputTranslation,
    thinking: bool,
    stream: bool,
    source: &str,
    operation: &str,
    image: Option<&Path>,
    sink: &mut (dyn StreamSink + Send),
) -> Result<String, String> {
    let mut message = ModelMessage::text(ModelRole::User, prepared.prompt);
    if let Some(image) = image {
        use base64::Engine;
        let bytes = std::fs::read(image).map_err(|error| error.to_string())?;
        message.content.insert(0, MessagePart::Image { mime_type: "image/png".into(), data: base64::engine::general_purpose::STANDARD.encode(bytes), path: None, detail: None });
    }
    let request = GenerateRequest {
        model: prepared.model,
        system: String::new(),
        messages: vec![message],
        tools: vec![],
        options: GenerateOptions {
            thinking_enabled: thinking,
            ..Default::default()
        },
        metadata: RequestMetadata {
            label: if image.is_some() { "Vision API" } else { "Text API" }.into(),
            usage_source: Some(source.into()),
            usage_operation: Some(operation.into()),
            ..Default::default()
        },
    };
    run_request(root, settings, &prepared.provider_id, prepared.attempts, request, stream, sink).await

}

/// The shared native model/OAuth/usage owner for translation and Lens requests.
pub(crate) async fn run_request(
    root: &Path, settings: &crate::settings::Settings, provider_id: &str,
    attempts: usize, request: GenerateRequest, stream: bool,
    sink: &mut (dyn StreamSink + Send),
) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct Catalog {
        providers: Vec<ModelProvider>,
    }
    let catalog: Catalog = serde_json::from_slice(
        &std::fs::read(root.join("conversations/providers.json"))
            .map_err(|e| format!("无法读取供应商配置：{e}"))?,
    )
    .map_err(|e| format!("供应商配置无效：{e}"))?;
    let provider = catalog
        .providers
        .into_iter()
        .find(|provider| provider.id == provider_id && provider.enabled)
        .ok_or("翻译供应商未启用，请在设置中选择模型")?;
    if !provider.enabled_models.contains(&request.model) {
        return Err("翻译模型未启用，请在设置中选择模型".into());
    }
    if !provider.authentication_ready() {
        return Err("请先登录翻译供应商".into());
    }
    let provider = crate::provider_oauth::resolve_provider(&provider).await?;
    let host = crate::RuntimeHost::shared(&provider, root);
    host.set_request_debug_enabled(settings.chat_tools.request_debug_enabled);
    let output = if stream {
        crate::dispatch::stream_resolved(&*host, &provider, attempts, request, sink).await
    } else {
        crate::dispatch::generate_resolved(&*host, &provider, attempts, request).await
    }
    .map_err(|error| error.to_string())?;
    Ok(output.text.trim().into())
}

/// Screenshot translation keeps the original engine routing and prompt semantics.
pub struct ImageTranslation { pub original: String, pub translated: String }

pub async fn translate_image(root: &Path, image: &Path, sink: &mut (dyn StreamSink + Send)) -> Result<ImageTranslation, String> {
    use crate::{settings::OcrMode, prompts, translation_stream::{CombinedTranslateSplitter, CombinedSplitPiece}};
    let settings = crate::native_preferences::load(root)?;
    let config = &settings.screenshot_translation;
    let language = crate::utils::language_name(&settings.reading_target_lang);
    let mode = config.resolved_ocr_mode();
    let local = matches!(mode, OcrMode::System | OcrMode::RapidOcr);
    let original = if local {
        let text = crate::native_capture::recognize(root, image, mode).await?;
        if text.trim().is_empty() { return Err("OCR 未识别到文字".into()); }
        prompts::compact_ocr_text(&text)
    } else { String::new() };
    let combined = !local && !config.direct_translate;
    let prompt = if local {
        prompts::build_screenshot_translation_prompt(&original, language, config.prompt.as_deref())
    } else if config.direct_translate {
        prompts::build_ocr_direct_translation_prompt(language, config.prompt.as_deref())
    } else {
        prompts::build_combined_translate_prompt(language, config.prompt.as_deref())
    };
    let operation = match (local, combined, config.stream_enabled) {
        (true, _, true) => "local_ocr_translate_stream", (true, _, false) => "local_ocr_translate",
        (false, true, true) => "image_combined_stream", (false, true, false) => "image_combined",
        (false, false, true) => "image_direct_stream", (false, false, false) => "image_direct",
    };
    let prepared = crate::input_translation::InputTranslation { provider_id: config.provider_id.clone(), model: config.model.clone(), prompt,
        attempts: if settings.retry_enabled { settings.retry_attempts.clamp(1,11) as usize } else {1} };
    let mut splitter = CombinedTranslateSplitter::new();
    let mut finish = None;
    let mut bridge = |part: StreamPart| {
        match part {
            StreamPart::TextDelta { delta } if combined => {
                for piece in splitter.push(&delta) { if let CombinedSplitPiece::Translated(delta) = piece { sink.emit(StreamPart::TextDelta { delta })?; } }
                Ok(())
            }
            // Finish belongs after the splitter's held suffix has been drained.
            part @ StreamPart::Finish { .. } if combined => { finish = Some(part); Ok(()) },
            part => sink.emit(part),
        }
    };
    let full = execute(root, &settings, prepared, config.thinking_enabled, config.stream_enabled, "screenshot_translation", operation,
        (!local).then_some(image), &mut bridge).await?;
    drop(bridge);
    if combined {
        if !config.stream_enabled { splitter.push(&full); }
        if let Some(tail) = splitter.flush_tail() { if config.stream_enabled { sink.emit(StreamPart::TextDelta { delta: tail }).map_err(|e| e.to_string())?; } }
        if let Some(finish) = finish { sink.emit(finish).map_err(|error| error.to_string())?; }
        Ok(ImageTranslation { original: splitter.original().trim().into(), translated: splitter.translated().trim().into() })
    } else {
        Ok(ImageTranslation { original: if config.direct_translate { String::new() } else { original }, translated: full })
    }
}
