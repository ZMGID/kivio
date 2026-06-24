use std::path::Path;

use tauri::State;

use crate::settings::OcrMode;
use crate::state::AppState;

#[cfg(target_os = "windows")]
use crate::windows_ocr;

/// 系统 OCR 是平台原生能力：macOS Apple Vision 或 Windows.Media.Ocr。
/// Linux 当前没有系统 OCR 端口，避免被旧设置误路由到不可用分支。
pub(crate) fn system_ocr_available_on_current_platform() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// RapidOCR 走本地 ONNX Runtime + 模型文件，当前只承诺 macOS/Windows/Linux。
pub(crate) fn rapidocr_available_on_current_platform() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    ))
}

pub(crate) fn normalize_ocr_mode_for_current_platform(mode: OcrMode) -> OcrMode {
    match mode {
        OcrMode::System if !system_ocr_available_on_current_platform() => OcrMode::CloudVision,
        OcrMode::RapidOcr if !rapidocr_available_on_current_platform() => OcrMode::CloudVision,
        other => other,
    }
}

pub(crate) async fn run_local_ocr(
    state: &State<'_, AppState>,
    image_path: &Path,
    engine: OcrMode,
) -> Result<String, String> {
    match engine {
        OcrMode::System => run_system_ocr(state, image_path).await,
        OcrMode::RapidOcr => state.rapidocr.ocr_image(image_path).await,
        // 路由层只把 System / RapidOcr 派发到这里。这里保留兜底，
        // 防止后续重构漏掉分支时静默进入错误 OCR 引擎。
        OcrMode::CloudVision | OcrMode::Legacy => {
            Err("internal: non-local OCR mode reached local OCR port".to_string())
        }
    }
}

async fn run_system_ocr(state: &State<'_, AppState>, image_path: &Path) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        return state
            .macos_ocr
            .ocr_image(&image_path.to_string_lossy())
            .await;
    }

    #[cfg(target_os = "windows")]
    {
        let _ = state;
        return windows_ocr::ocr_image(image_path).await;
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (state, image_path);
        Err("System OCR is not available on this platform".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn linux_keeps_rapidocr_available_without_system_ocr() {
        assert!(!system_ocr_available_on_current_platform());
        assert!(rapidocr_available_on_current_platform());
        assert_eq!(
            normalize_ocr_mode_for_current_platform(OcrMode::System),
            OcrMode::CloudVision
        );
        assert_eq!(
            normalize_ocr_mode_for_current_platform(OcrMode::RapidOcr),
            OcrMode::RapidOcr
        );
    }

    #[test]
    fn cloud_vision_is_never_changed_by_platform_normalization() {
        assert_eq!(
            normalize_ocr_mode_for_current_platform(OcrMode::CloudVision),
            OcrMode::CloudVision
        );
    }
}
