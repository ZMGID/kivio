use std::path::PathBuf;

#[cfg(target_os = "windows")]
use uuid::Uuid;
#[cfg(target_os = "windows")]
use xcap::Monitor;

#[cfg(target_os = "windows")]
use crate::capture_geometry::{
    monitor_for_region, windows_monitor_region, CaptureMonitor, CaptureRect,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct RegionCaptureRequest {
    pub absolute_x: i32,
    pub absolute_y: i32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub exclude_self_pid: Option<i32>,
}

/// 平台区域截图端口。调用方只表达选区；平台细节在本模块内分发。
#[cfg(target_os = "windows")]
pub(crate) fn capture_region_image(request: RegionCaptureRequest) -> Result<PathBuf, String> {
    let _ = (request.x, request.y, request.scale_factor);
    let started = std::time::Instant::now();
    let monitors = Monitor::all().map_err(|e| e.to_string())?;
    eprintln!(
        "[lens-timing]     ...Monitor::all +{}ms",
        started.elapsed().as_millis()
    );
    let monitor_geometry = monitors
        .iter()
        .map(|m| {
            Ok(CaptureMonitor {
                x: m.x().map_err(|e| e.to_string())?,
                y: m.y().map_err(|e| e.to_string())?,
                width: m.width().map_err(|e| e.to_string())?,
                height: m.height().map_err(|e| e.to_string())?,
                scale_factor: m.scale_factor().map_err(|e| e.to_string())? as f64,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let region = CaptureRect {
        x: request.absolute_x as f64,
        y: request.absolute_y as f64,
        width: request.width as f64,
        height: request.height as f64,
    };
    let monitor_index = monitor_for_region(region, &monitor_geometry)
        .ok_or_else(|| "No monitor found for capture region".to_string())?;
    let capture_region = windows_monitor_region(region, monitor_geometry[monitor_index])
        .ok_or_else(|| "Invalid capture region".to_string())?;
    let monitor = &monitors[monitor_index];

    let capture_started = std::time::Instant::now();
    let image = monitor
        .capture_region(
            capture_region.x,
            capture_region.y,
            capture_region.width,
            capture_region.height,
        )
        .map_err(|e| e.to_string())?;
    eprintln!(
        "[lens-timing]     ...xcap.capture_region +{}ms",
        capture_started.elapsed().as_millis()
    );

    let temp_path = std::env::temp_dir().join(format!("screenshot-{}.png", Uuid::new_v4()));
    let save_started = std::time::Instant::now();
    write_png_fast(&temp_path, image.as_raw(), image.width(), image.height())?;
    eprintln!(
        "[lens-timing]     ...png.save +{}ms",
        save_started.elapsed().as_millis()
    );
    Ok(temp_path)
}

#[cfg(target_os = "windows")]
fn write_png_fast(
    path: &std::path::Path,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<(), String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let writer = std::io::BufWriter::new(file);
    PngEncoder::new_with_quality(writer, CompressionType::Fast, FilterType::NoFilter)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())
}

/// macOS 区域截图走 ScreenCaptureKit，可用 exclude_self_pid 排除 Lens 自身窗口。
#[cfg(target_os = "macos")]
pub(crate) fn capture_region_image(request: RegionCaptureRequest) -> Result<PathBuf, String> {
    let _ = (request.x, request.y, request.scale_factor);
    crate::sck::capture_region(
        request.absolute_x as f64,
        request.absolute_y as f64,
        request.width as f64,
        request.height as f64,
        request.exclude_self_pid,
    )
}

/// 其他平台保持显式不支持；UI 必须通过 get_platform_capabilities 呈现降级状态。
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub(crate) fn capture_region_image(request: RegionCaptureRequest) -> Result<PathBuf, String> {
    let _ = (
        request.absolute_x,
        request.absolute_y,
        request.x,
        request.y,
        request.width,
        request.height,
        request.scale_factor,
        request.exclude_self_pid,
    );
    Err("Region capture is not supported on this platform".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn non_windows_macos_region_capture_is_explicitly_unsupported() {
        let err = capture_region_image(RegionCaptureRequest {
            absolute_x: 0,
            absolute_y: 0,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            scale_factor: 1.0,
            exclude_self_pid: None,
        })
        .unwrap_err();
        assert_eq!(err, "Region capture is not supported on this platform");
    }
}
