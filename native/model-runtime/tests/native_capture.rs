use kivio_model_runtime::{
    capture_geometry::CaptureRect,
    native_capture::{CaptureRuntime, CapturedImage, FrozenFrame},
};

#[tokio::test]
async fn local_rapidocr_reports_missing_pack_without_cloud_fallback() {
    let root = tempfile::tempdir().unwrap();
    let image = root.path().join("selection.png");
    image::RgbImage::new(2, 2).save(&image).unwrap();
    let error = kivio_model_runtime::native_capture::recognize(
        root.path(), &image, kivio_model_runtime::settings::OcrMode::RapidOcr,
    ).await.unwrap_err();
    assert_eq!(error, "rapidocr_models_missing");
    assert!(image.exists(), "a failed recognition must preserve the selected image");
    std::fs::write(root.path().join("settings.json"), r#"{"settings":{"screenshotTranslation":{"ocrMode":"tesseract"}}}"#).unwrap();
    let error = kivio_model_runtime::native_translation::translate_image(
        root.path(), &image, &mut |_| Ok(()),
    ).await.err().expect("legacy offline mode requires a local model pack");
    assert_eq!(error, "rapidocr_models_missing");
}

#[test]
fn frozen_region_uses_selected_pixels_and_releases_session_images() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("freeze.png");
    let pixels = image::RgbImage::from_fn(8, 4, |x, y| image::Rgb([x as u8, y as u8, 9]));
    pixels.save(&path).unwrap();
    let runtime = CaptureRuntime::default();
    let session = runtime.session.begin_session(false).unwrap();
    runtime
        .session
        .register_freeze_frame(session, "freeze".into(), path.clone())
        .unwrap();
    let frame = FrozenFrame {
        image: CapturedImage {
            id: "freeze".into(),
            path: path.clone(),
            width: 8,
            height: 4,
        },
        bounds: CaptureRect {
            x: -100.,
            y: 200.,
            width: 4.,
            height: 2.,
        },
    };
    let region = CaptureRect {
        x: -99.,
        y: 200.,
        width: 1.,
        height: 1.,
    };
    let selected = runtime
        .capture_region(session, region, Some(&frame))
        .unwrap();
    assert_eq!((selected.width, selected.height), (2, 2));
    let crop = image::open(&selected.path).unwrap().to_rgb8();
    assert_eq!(*crop.get_pixel(0, 0), image::Rgb([2, 0, 9]));
    assert_eq!(*crop.get_pixel(1, 1), image::Rgb([3, 1, 9]));
    assert!(path.exists());
    let mut settings = kivio_model_runtime::settings::Settings::default();
    settings.image_archive_enabled = true;
    settings.image_archive_path = root.path().join("归档 测试").to_string_lossy().into_owned();
    let archived = kivio_model_runtime::capture_runtime::archive_captured_image(
        &settings, &selected.path, &selected.id,
    ).unwrap().unwrap();
    runtime.close();
    assert!(!path.exists());
    assert!(!selected.path.exists());
    assert_eq!(image::open(&archived).unwrap().to_rgb8(), crop);
    settings.image_archive_path = archived.to_string_lossy().into_owned();
    assert!(kivio_model_runtime::capture_runtime::archive_captured_image(
        &settings, &archived, "another",
    ).is_err());
    assert!(archived.exists());
    assert!(
        runtime
            .capture_region(session, region, Some(&frame))
            .is_err()
    );
    let reopened = runtime.session.begin_session(false).unwrap();
    assert!(
        runtime
            .session
            .register_freeze_frame(session, "late".into(), root.path().join("late.png"))
            .is_err()
    );
    assert!(runtime.session.is_session_active(reopened));
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires the built Apple Vision helper in src-tauri/binaries"]
async fn native_ocr_helper_handles_concurrent_requests_errors_and_shutdown() {
    use kivio_model_runtime::macos_ocr::MacOcrClient;
    use std::time::Duration;
    let helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../src-tauri/binaries/kivio-ocr-helper-{}-apple-darwin",
        std::env::consts::ARCH
    ));
    assert!(
        helper.is_file(),
        "build the original Swift OCR helper first"
    );
    let root = tempfile::tempdir().unwrap();
    let image = root.path().join("blank.png");
    image::RgbImage::from_pixel(400, 200, image::Rgb([255, 255, 255]))
        .save(&image)
        .unwrap();
    let client = MacOcrClient::new(helper);
    let image_path = image.to_string_lossy();
    let (first, second) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(client.ocr_image(&image_path), client.ocr_image(&image_path))
    })
    .await
    .unwrap();
    assert_eq!(first.unwrap(), "");
    assert_eq!(second.unwrap(), "");
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        client.ocr_image("/nonexistent/kivio-ocr.png"),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.contains("无法读取图像"), "{error}");
    client.shutdown();
    client.shutdown();
    assert!(
        client
            .ocr_image(&image_path)
            .await
            .unwrap_err()
            .contains("已关闭")
    );
}
