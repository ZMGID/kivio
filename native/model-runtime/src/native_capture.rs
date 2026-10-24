//! Native capture host. Call blocking capture methods off the UI thread.
//! A frozen display is a selection background; only the selected crop is published.
use crate::{capture_geometry::CaptureRect, lens_runtime::LensRuntimeState};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct CapturedImage {
    pub id: String,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone)]
pub struct FrozenFrame {
    pub image: CapturedImage,
    pub bounds: CaptureRect,
}
#[derive(Default)]
pub struct CaptureRuntime {
    pub session: Arc<LensRuntimeState>,
}
impl CaptureRuntime {
    pub fn freeze_display(&self, session: u64, bounds: CaptureRect) -> Result<FrozenFrame, String> {
        self.require_session(session)?;
        let image = captured(capture_pixels(bounds)?)?;
        match self
            .session
            .register_freeze_frame(session, image.id.clone(), image.path.clone())
        {
            Ok(previous) => {
                if let Some((_, path)) = previous {
                    let _ = std::fs::remove_file(path);
                }
                Ok(FrozenFrame { image, bounds })
            }
            Err(path) => {
                let _ = std::fs::remove_file(path);
                Err("截图会话已结束".into())
            }
        }
    }
    pub fn capture_region(
        &self,
        session: u64,
        bounds: CaptureRect,
        frozen: Option<&FrozenFrame>,
    ) -> Result<CapturedImage, String> {
        self.require_session(session)?;
        validate(bounds)?;
        let path = if let Some(frame) =
            frozen.filter(|frame| self.session.is_current_freeze_frame(&frame.image.id))
        {
            let scale = frame.image.width as f64 / frame.bounds.width;
            crate::capture_runtime::crop_freeze_frame_image(
                &frame.image.path,
                (bounds.x - frame.bounds.x).round() as i32,
                (bounds.y - frame.bounds.y).round() as i32,
                bounds.width.round() as u32,
                bounds.height.round() as u32,
                scale,
            )?
        } else {
            capture_pixels(bounds)?
        };
        self.publish(session, path)
    }
    pub fn capture_window(&self, session: u64, window_id: u32) -> Result<CapturedImage, String> {
        self.require_session(session)?;
        self.publish(session, crate::lens::capture_window(window_id)?)
    }
    fn require_session(&self, session: u64) -> Result<(), String> {
        if self.session.is_session_active(session) {
            Ok(())
        } else {
            Err("截图会话已结束".into())
        }
    }
    fn publish(&self, session: u64, path: PathBuf) -> Result<CapturedImage, String> {
        let image = captured(path)?;
        match self
            .session
            .register_current_image(session, image.id.clone(), image.path.clone())
        {
            Ok(previous) => {
                if let Some(id) = previous {
                    if let Some(path) = self.session.remove_image(&id) {
                        let _ = std::fs::remove_file(path);
                    }
                }
                Ok(image)
            }
            Err(path) => {
                let _ = std::fs::remove_file(path);
                Err("截图会话已结束".into())
            }
        }
    }
    pub fn close(&self) {
        self.session.cancel_stream();
        if let Some((_, path)) = self.session.close_session_and_take_current() {
            let _ = std::fs::remove_file(path);
        }
        if let Some(id) = self.session.take_freeze_frame() {
            if let Some(path) = self.session.remove_image(&id) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
impl Drop for CaptureRuntime {
    fn drop(&mut self) {
        self.close();
    }
}
fn validate(rect: CaptureRect) -> Result<(), String> {
    if ![rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|v| v.is_finite())
        || rect.width < 1.
        || rect.height < 1.
        || rect.width > 8192.
        || rect.height > 8192.
    {
        return Err("截图区域尺寸无效".into());
    }
    Ok(())
}
fn captured(path: PathBuf) -> Result<CapturedImage, String> {
    let dimensions = image::image_dimensions(&path).map_err(|error| error.to_string());
    match dimensions {
        Ok((width, height))
            if width > 0
                && height > 0
                && width <= 8192
                && height <= 8192
                && width as u64 * height as u64 <= 32 * 1024 * 1024 =>
        {
            Ok(CapturedImage {
                id: uuid::Uuid::new_v4().to_string(),
                path,
                width,
                height,
            })
        }
        result => {
            let _ = std::fs::remove_file(path);
            Err(result
                .err()
                .unwrap_or_else(|| "截图像素尺寸超出限制".into()))
        }
    }
}
fn capture_pixels(rect: CaptureRect) -> Result<PathBuf, String> {
    validate(rect)?;
    #[cfg(target_os = "macos")]
    {
        crate::sck::capture_region(
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            Some(std::process::id() as i32),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("此平台尚未接入原生区域截图".into())
    }
}

/// Honors local OCR routing. Cloud vision must be chosen by the model caller;
/// this boundary never uploads an image as a fallback for a local failure.
pub async fn recognize(
    root: &std::path::Path,
    image: &std::path::Path,
    mode: crate::settings::OcrMode,
) -> Result<String, String> {
    match mode {
        crate::settings::OcrMode::System => {
            #[cfg(target_os = "macos")]
            {
                let _ = root;
                system_ocr()?.ocr_image(&image.to_string_lossy()).await
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (root, image);
                Err("此平台尚未接入系统 OCR".into())
            }
        }
        crate::settings::OcrMode::RapidOcr => {
            // Serialize across callers and data roots: two simultaneous workers
            // would double the model's transient memory footprint.
            let _worker = OCR_WORKER.lock().await;
            let settings = crate::native_preferences::load(root)?;
            let tier = crate::rapidocr::ModelTier::parse(
                &settings.screenshot_translation.rapid_ocr_tier,
            );
            rapid_ocr_client(root)?.ocr_image(image, tier).await
        }
        crate::settings::OcrMode::CloudVision | crate::settings::OcrMode::Legacy => Err("云端视觉识别应由已选模型处理".into()),
    }
}

static OCR_WORKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn rapid_ocr_client(root: &std::path::Path) -> Result<Arc<crate::rapidocr::RapidOcrClient>, String> {
            let models = crate::offline_models::OfflineModelManager::with_model_dir(
                root.join("rapidocr-models"), crate::transport::build_http_client(),
            );
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let mut worker = executable.parent().ok_or("无法定位 OCR worker")?
                .join(if cfg!(windows) { "kivio-ocr-worker.exe" } else { "kivio-ocr-worker" });
            #[cfg(debug_assertions)]
            if !worker.is_file() {
                worker = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../ocr-worker/target/debug")
                    .join(if cfg!(windows) { "kivio-ocr-worker.exe" } else { "kivio-ocr-worker" });
            }
    Ok(crate::rapidocr::RapidOcrClient::with_worker(models, worker))
}
pub async fn recognize_lines(root: &std::path::Path, image: &std::path::Path, tier: crate::rapidocr::ModelTier) -> Result<Vec<crate::rapidocr::RapidOcrLine>, String> {
    let _worker = OCR_WORKER.lock().await;
    rapid_ocr_client(root)?.ocr_image_lines(image, tier).await
}

#[cfg(target_os = "macos")]
pub fn system_ocr() -> Result<Arc<crate::macos_ocr::MacOcrClient>, String> {
    static CLIENT: std::sync::OnceLock<Arc<crate::macos_ocr::MacOcrClient>> =
        std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let mut helper = crate::macos_ocr::MacOcrClient::bundled_helper()?;
    #[cfg(debug_assertions)]
    if !helper.is_file() {
        helper = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../src-tauri/binaries/kivio-ocr-helper-{}-apple-darwin",
            std::env::consts::ARCH
        ));
    }
    if !helper.is_file() {
        return Err("未找到系统 OCR helper，请重新安装完整应用。".into());
    }
    Ok(CLIENT
        .get_or_init(|| crate::macos_ocr::MacOcrClient::new(helper))
        .clone())
}
