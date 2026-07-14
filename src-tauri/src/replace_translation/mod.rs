pub mod layout;
pub mod mask;

#[cfg(all(test, target_os = "macos"))]
mod e2e_tests {
    #[tokio::test]
    #[ignore]
    async fn replace_pipeline_real_fixture() {
        let model_dir = std::env::var_os("KIVIO_REPLACE_MODEL_DIR")
            .map(std::path::PathBuf::from)
            .expect("set KIVIO_REPLACE_MODEL_DIR");
        let image_path = std::env::var_os("KIVIO_REPLACE_IMAGE")
            .map(std::path::PathBuf::from)
            .expect("set KIVIO_REPLACE_IMAGE");
        let output_path = std::env::var_os("KIVIO_REPLACE_OUTPUT")
            .map(std::path::PathBuf::from)
            .expect("set KIVIO_REPLACE_OUTPUT");
        let manager = crate::offline_models::OfflineModelManager::with_model_dir(
            model_dir,
            reqwest::Client::new(),
        );
        let ocr = crate::rapidocr::RapidOcrClient::new(manager.clone());
        let spans = ocr
            .ocr_image_lines(&image_path, crate::rapidocr::ModelTier::Standard)
            .await
            .expect("run OCR");
        let image = image::open(&image_path).expect("open image").to_rgb8();
        let spans = super::layout::filter_replaceable_spans(image.width(), &spans);
        let geometry = super::layout::build_replace_geometry(&image, &spans);
        let analysis = super::mask::analyze_text_regions(&image, &spans).expect("analyze");
        let complexity = analysis.complexity;
        let mask = analysis.mask;
        eprintln!("background complexity: {complexity:?}");
        let started = std::time::Instant::now();
        let png = if complexity == super::mask::BackgroundComplexity::Low {
            let filled = super::mask::deterministic_fill(&image, &spans, &mask);
            crate::inpainting::encode_rgb_png(filled).expect("encode deterministic fill")
        } else {
            crate::inpainting::InpaintingClient::new(manager)
                .inpaint_png(std::sync::Arc::new(image), mask)
                .await
                .expect("run inpainting")
                .png
        };
        eprintln!(
            "replace pipeline: {} spans, {} regions, inpainting {:?}",
            spans.len(),
            geometry.groups.len(),
            started.elapsed()
        );
        std::fs::write(output_path, png).expect("write result");
    }
}
