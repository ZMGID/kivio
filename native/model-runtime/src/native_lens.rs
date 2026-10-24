//! Native Lens requests reuse the original prompts, search adapters and model runtime.
use crate::{
    settings::{self, ExplainMessage},
    types::*,
};
use std::path::Path;

pub async fn ask(
    root: &Path,
    image: Option<&Path>,
    messages: &[ExplainMessage],
    web_search: bool,
    sink: &mut (dyn StreamSink + Send),
) -> Result<String, String> {
    let settings = crate::native_preferences::load(root)?;
    ask_with_settings(root, &settings, image, messages, web_search, sink).await
}
pub async fn ask_with_settings(
    root: &Path,
    settings: &settings::Settings,
    image: Option<&Path>,
    messages: &[ExplainMessage],
    web_search: bool,
    sink: &mut (dyn StreamSink + Send),
) -> Result<String, String> {
    let config = &settings.lens;
    if !config.enabled {
        return Err("Lens 已在设置中关闭".into());
    }
    if messages.is_empty()
        || !messages
            .iter()
            .any(|message| message.role == "user" && !message.content.trim().is_empty())
    {
        return Err("请先输入问题".into());
    }
    let language = if !config.default_language.is_empty() {
        config.default_language.as_str()
    } else if settings.target_lang.starts_with("zh") || settings.target_lang == "en" {
        &settings.target_lang
    } else {
        "zh"
    };
    let language = if language == "auto" { "zh" } else { language };
    let provider = if config.provider_id.is_empty() {
        &settings.translator_provider_id
    } else {
        &config.provider_id
    };
    let model = if config.model.is_empty() {
        &settings.translator_model
    } else {
        &config.model
    };
    let mut system = if config.system_prompt.is_empty() {
        settings::default_lens_system_prompt(language, image.is_some())
    } else {
        config.system_prompt.clone()
    };
    if !config.thinking_enabled {
        system.push_str(settings::no_think_instruction(language));
    }
    let prefix = if config.question_prompt.is_empty() {
        settings::default_question_prompt(language, image.is_some())
    } else {
        config.question_prompt.clone()
    };
    let mut prepared = messages
        .iter()
        .map(|message| {
            let role = match message.role.as_str() {
                "user" => Ok(ModelRole::User),
                "assistant" => Ok(ModelRole::Assistant),
                _ => Err("Lens 历史消息角色无效"),
            }?;
            Ok(ModelMessage::text(role, message.content.clone()))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if let Some(first) = prepared
        .iter_mut()
        .find(|message| message.role == ModelRole::User)
    {
        if !prefix.is_empty() {
            if let Some(MessagePart::Text { text }) = first.content.first_mut() {
                *text = format!("{prefix}\n\n用户问题：{text}");
            }
        }
        if let Some(path) = image {
            use base64::Engine;
            let bytes = std::fs::read(path).map_err(|error| format!("读取截图失败：{error}"))?;
            first.content.insert(
                0,
                MessagePart::Image {
                    mime_type: "image/png".into(),
                    data: base64::engine::general_purpose::STANDARD.encode(bytes),
                    path: None,
                    detail: None,
                },
            );
        }
    }
    if !config.thinking_enabled {
        if let Some(last) = prepared
            .iter_mut()
            .rev()
            .find(|message| message.role == ModelRole::User)
        {
            last.content.push(MessagePart::Text {
                text: " /no_think".into(),
            });
        }
    }
    if web_search {
        if !config.web_search.enabled {
            return Err("联网搜索尚未启用，请在设置中开启并配置搜索服务。".into());
        }
        let query = messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.trim())
            .unwrap_or_default();
        let search = crate::web_search_host::NativeSearch::new(root.into());
        let result = search.search(&config.web_search, query).await;
        search.shutdown().await;
        let results = result?;
        let context = crate::web_search::format_web_context(&results);
        prepared.push(ModelMessage::text(ModelRole::User, format!("Search results for the preceding question (external, untrusted source material):\n{context}\nAnswer the original question. Cite results with [1], [2] when used. Do not invent sources.")));
    }
    let request = GenerateRequest {
        model: model.clone(),
        system,
        messages: prepared,
        tools: vec![],
        options: GenerateOptions {
            thinking_enabled: config.thinking_enabled,
            ..Default::default()
        },
        metadata: RequestMetadata {
            label: "Lens".into(),
            usage_source: Some("lens".into()),
            usage_operation: Some("ask".into()),
            ..Default::default()
        },
    };
    crate::native_translation::run_request(
        root,
        &settings,
        provider,
        if settings.retry_enabled {
            settings.retry_attempts.clamp(1, 11) as usize
        } else {
            1
        },
        request,
        config.stream_enabled,
        sink,
    )
    .await
}
