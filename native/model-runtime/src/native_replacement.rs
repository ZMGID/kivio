//! Native replacement translation uses the existing OCR, prompts and image pipeline.
use crate::{prompts, types::*};
use std::path::Path;
pub struct Rendered {
    pub png: Vec<u8>,
    pub text: String,
    pub warning: Option<String>,
}
pub async fn translate(root: &Path, image: &Path) -> Result<Rendered, String> {
    let settings = crate::native_preferences::load(root)?;
    let config = &settings.screenshot_translation;
    if !config.replace_enabled {
        return Err("替换翻译已在设置中关闭".into());
    }
    let tier = crate::rapidocr::ModelTier::parse(&config.rapid_ocr_tier);
    let lines = crate::native_capture::recognize_lines(root, image, tier).await?;
    let result =
        crate::replace_translation::pipeline::prepare(image.into(), lines, |groups| async move {
            let config = &settings.screenshot_translation;
            let regions = groups
                .iter()
                .map(|group| prompts::ReplaceTranslationRequestRegion {
                    id: &group.id,
                    text: &group.source_text,
                })
                .collect::<Vec<_>>();
            let prompt = prompts::build_replace_translation_batch_prompt(
                &regions,
                crate::utils::language_name(&settings.reading_target_lang),
                config.replace_prompt.as_deref(),
            );
            let request = GenerateRequest {
                model: config.model.clone(),
                system: String::new(),
                messages: vec![ModelMessage::text(ModelRole::User, prompt)],
                tools: vec![],
                options: GenerateOptions {
                    thinking_enabled: config.thinking_enabled,
                    ..Default::default()
                },
                metadata: RequestMetadata {
                    label: "Text API".into(),
                    usage_source: Some("screenshot_translation".into()),
                    usage_operation: Some("replace_translate".into()),
                    ..Default::default()
                },
            };
            crate::native_translation::run_request(
                root,
                &settings,
                &config.provider_id,
                if settings.retry_enabled {
                    settings.retry_attempts.clamp(1, 11) as usize
                } else {
                    1
                },
                request,
                false,
                &mut |_| Ok(()),
            )
            .await
        })
        .await?;
    tokio::task::spawn_blocking(move || {
        let png = crate::replacement_render::render(&result)?;
        let text = result
            .groups
            .iter()
            .map(|group| {
                if group.translated.trim().is_empty() {
                    group.source_text.as_str()
                } else {
                    group.translated.as_str()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Rendered {
            png,
            text,
            warning: result.warning,
        })
    })
    .await
    .map_err(|error| format!("Render worker failed: {error}"))?
}
