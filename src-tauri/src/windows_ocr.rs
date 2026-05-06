#[cfg(target_os = "windows")]
use std::path::Path;

#[cfg(target_os = "windows")]
use ::windows::{
  core::HSTRING,
  Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
  },
  Media::Ocr::OcrEngine,
  Storage::{FileAccessMode, Streams::FileRandomAccessStream},
  Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
};

#[cfg(target_os = "windows")]
pub async fn ocr_image(image_path: &Path) -> Result<String, String> {
  ocr_image_blocking(image_path)
}

#[cfg(target_os = "windows")]
fn ocr_image_blocking(image_path: &Path) -> Result<String, String> {
  struct WinRtGuard(bool);

  impl Drop for WinRtGuard {
    fn drop(&mut self) {
      if self.0 {
        unsafe {
          RoUninitialize();
        }
      }
    }
  }

  // WinRT initialization is per-thread. If Tauri/WebView already initialized the thread
  // differently, the OCR calls can still proceed, so do not fail hard on this hint.
  let _guard = WinRtGuard(unsafe { RoInitialize(RO_INIT_MULTITHREADED).is_ok() });

  let path = HSTRING::from(image_path);
  let stream = FileRandomAccessStream::OpenAsync(&path, FileAccessMode::Read)
    .map_err(|e| format!("Windows OCR open image: {e}"))?
    .get()
    .map_err(|e| format!("Windows OCR open image: {e}"))?;
  let decoder = BitmapDecoder::CreateAsync(&stream)
    .map_err(|e| format!("Windows OCR decode image: {e}"))?
    .get()
    .map_err(|e| format!("Windows OCR decode image: {e}"))?;

  let width = decoder
    .PixelWidth()
    .map_err(|e| format!("Windows OCR read image width: {e}"))?;
  let height = decoder
    .PixelHeight()
    .map_err(|e| format!("Windows OCR read image height: {e}"))?;
  let max_dimension = OcrEngine::MaxImageDimension()
    .map_err(|e| format!("Windows OCR max image size unavailable: {e}"))?
    .max(1);

  let bitmap = if width > max_dimension || height > max_dimension {
    let scale = max_dimension as f64 / width.max(height) as f64;
    let scaled_width = ((width as f64 * scale).round() as u32).max(1);
    let scaled_height = ((height as f64 * scale).round() as u32).max(1);
    let transform = BitmapTransform::new()
      .map_err(|e| format!("Windows OCR create bitmap transform: {e}"))?;
    transform
      .SetScaledWidth(scaled_width)
      .map_err(|e| format!("Windows OCR set scaled width: {e}"))?;
    transform
      .SetScaledHeight(scaled_height)
      .map_err(|e| format!("Windows OCR set scaled height: {e}"))?;
    transform
      .SetInterpolationMode(BitmapInterpolationMode::Fant)
      .map_err(|e| format!("Windows OCR set interpolation: {e}"))?;
    decoder
      .GetSoftwareBitmapTransformedAsync(
        BitmapPixelFormat::Bgra8,
        BitmapAlphaMode::Premultiplied,
        &transform,
        ExifOrientationMode::RespectExifOrientation,
        ColorManagementMode::DoNotColorManage,
      )
      .map_err(|e| format!("Windows OCR convert image: {e}"))?
      .get()
      .map_err(|e| format!("Windows OCR convert image: {e}"))?
  } else {
    decoder
      .GetSoftwareBitmapConvertedAsync(
        BitmapPixelFormat::Bgra8,
        BitmapAlphaMode::Premultiplied,
      )
      .map_err(|e| format!("Windows OCR convert image: {e}"))?
      .get()
      .map_err(|e| format!("Windows OCR convert image: {e}"))?
  };

  let engine = OcrEngine::TryCreateFromUserProfileLanguages()
    .map_err(|e| format!("Windows OCR engine unavailable: {e}"))?;
  let result = engine
    .RecognizeAsync(&bitmap)
    .map_err(|e| format!("Windows OCR recognize: {e}"))?
    .get()
    .map_err(|e| format!("Windows OCR recognize: {e}"))?;

  Ok(result
    .Text()
    .map_err(|e| format!("Windows OCR read result: {e}"))?
    .to_string_lossy())
}
