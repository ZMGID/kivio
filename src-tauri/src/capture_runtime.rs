//! Pixel-accurate cropping shared by desktop capture hosts.
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageCropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub fn crop_freeze_frame_image(
    path: &Path,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
) -> Result<PathBuf, String> {
    let image = image::open(path).map_err(|e| e.to_string())?;
    let rect = freeze_frame_crop_rect(
        x,
        y,
        width,
        height,
        scale_factor,
        image.width(),
        image.height(),
    )
    .ok_or_else(|| "Invalid freeze-frame capture region".to_string())?;
    let cropped = image.crop_imm(rect.x, rect.y, rect.width, rect.height);
    let temp_path = std::env::temp_dir().join(format!("screenshot-{}.png", Uuid::new_v4()));
    #[cfg(target_os = "windows")]
    {
        let rgba = cropped.into_rgba8();
        write_png_fast(&temp_path, rgba.as_raw(), rgba.width(), rgba.height())?;
    }
    #[cfg(not(target_os = "windows"))]
    cropped.save(&temp_path).map_err(|e| e.to_string())?;
    Ok(temp_path)
}

pub fn freeze_frame_crop_rect(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
    image_width: u32,
    image_height: u32,
) -> Option<ImageCropRect> {
    if width == 0 || height == 0 || image_width == 0 || image_height == 0 {
        return None;
    }
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    // Round shared edges, not origin and extent independently: adjacent logical
    // rectangles must meet at the same physical pixel at fractional DPI.
    let left = (x as f64 * scale).round().clamp(0.0, image_width as f64) as u32;
    let top = (y as f64 * scale).round().clamp(0.0, image_height as f64) as u32;
    let right = ((x as f64 + width as f64) * scale)
        .round()
        .clamp(0.0, image_width as f64) as u32;
    let bottom = ((y as f64 + height as f64) * scale)
        .round()
        .clamp(0.0, image_height as f64) as u32;

    if right <= left || bottom <= top {
        return None;
    }

    Some(ImageCropRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

/// 快速无损 PNG 编码：`image` 默认编码器对全屏 4MP 图做自适应滤波 + 默认 zlib 压缩，
/// 单帧编码约 350ms，是冻结帧/截图首帧出现的主要延迟。冻结帧只需「无损 + 快」，
/// Fast + Sub 保留无损像素，并利用屏幕相邻像素的相似性降低压缩工作量。
/// 实测 2560×1600 桌面：NoFilter 约 270ms/13MB，Sub 约 105ms/2.5MB。
#[cfg(target_os = "windows")]
pub fn write_png_fast(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let writer = std::io::BufWriter::new(file);
    PngEncoder::new_with_quality(writer, CompressionType::Fast, FilterType::Sub)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())
}
/// Archive only a completed capture, independently from session-owned temp files.
/// An empty destination means unconfigured, matching the original settings contract.
pub fn archive_captured_image(
    settings: &crate::settings::Settings,
    source: &std::path::Path,
    image_id: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    if !settings.image_archive_enabled || settings.image_archive_path.trim().is_empty() {
        return Ok(None);
    }
    let directory = std::path::Path::new(&settings.image_archive_path);
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let short_id: String = image_id.chars().take(8).collect();
    let destination = directory.join(format!("kivio-{}-{}.png",
        chrono::Local::now().format("%Y-%m-%d-%H%M%S"), short_id));
    std::fs::copy(source, &destination).map_err(|error| error.to_string())?;
    Ok(Some(destination))
}
