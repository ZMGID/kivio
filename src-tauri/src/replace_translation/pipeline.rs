//! Shared replacement geometry, parallel background fill, and partial-result recovery.
use super::{layout::*, mask::*};
use std::{path::PathBuf, sync::Arc};
pub struct Replacement {
    pub groups: Vec<TranslationGroup>,
    pub slots: Vec<RenderSlot>,
    pub cleaned_png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub warning: Option<String>,
}
pub async fn prepare<F, Fut>(
    path: PathBuf,
    lines: Vec<crate::rapidocr::RapidOcrLine>,
    translate: F,
) -> Result<Replacement, String>
where
    F: FnOnce(Vec<TranslationGroup>) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let (source, mut geometry, spans) = tokio::task::spawn_blocking(move || {
        let source = image::open(path)
            .map_err(|error| error.to_string())?
            .to_rgb8();
        let spans = filter_replaceable_spans(source.width(), &lines);
        if spans.is_empty() {
            return Err("OCR 未识别到可替换文字".to_string());
        }
        let geometry = build_replace_geometry(&source, &spans);
        if geometry.groups.is_empty() || geometry.slots.is_empty() {
            return Err("layout_no_regions".into());
        }
        Ok((source, geometry, spans))
    })
    .await
    .map_err(|error| format!("Replace layout worker failed: {error}"))??;
    let (width, height) = source.dimensions();
    let source = Arc::new(source);
    let fill_source = source.clone();
    let blocks = blocks_from_groups(&geometry.groups, &spans);
    let fill = tokio::task::spawn_blocking(move || plate_fill(&fill_source, &blocks));
    let (translated, cleaned) = tokio::join!(translate(geometry.groups.clone()), fill);
    let ids = geometry
        .groups
        .iter()
        .map(|group| group.id.as_str())
        .collect::<Vec<_>>();
    let translations = crate::prompts::parse_replace_translation_json(&translated?, &ids)?;
    if translations.is_empty() {
        return Err("翻译结果为空：没有任何区域被翻译".into());
    }
    let missing = geometry
        .groups
        .iter()
        .filter(|group| !translations.contains_key(&group.id))
        .count();
    let warning = (missing > 0).then(|| {
        format!(
            "{missing}/{} 个区域缺少译文，已回退显示原文",
            geometry.groups.len()
        )
    });
    for group in &mut geometry.groups {
        if let Some(text) = translations.get(&group.id) {
            group.translated = text.clone();
        }
    }
    let mut cleaned =
        cleaned.map_err(|error| format!("Replace background worker failed: {error}"))?;
    let groups = geometry.groups.clone();
    let cleaned_png = tokio::task::spawn_blocking(move || {
        restore_unchanged_groups(&source, &mut cleaned, &groups, &spans);
        super::encode_rgb_png(cleaned)
    })
    .await
    .map_err(|error| format!("Replace encoding worker failed: {error}"))??;
    Ok(Replacement {
        groups: geometry.groups,
        slots: geometry.slots,
        cleaned_png,
        width,
        height,
        warning,
    })
}
