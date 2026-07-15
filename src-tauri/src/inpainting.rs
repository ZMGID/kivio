//! Local MI-GAN image inpainting for replace translation.

use std::io::Cursor;
use std::sync::{Arc, Mutex as StdMutex};

use image::{DynamicImage, ImageFormat, RgbImage};
use ort::session::Session;
use ort::value::Tensor;
use serde::Serialize;
use tokio::sync::OnceCell;

use crate::offline_models::{ensure_ort_init, OfflineModelManager};

#[derive(Debug, Clone)]
pub struct InpaintMask {
    pub width: u32,
    pub height: u32,
    /// One byte per pixel. Zero preserves the source pixel; non-zero marks a hole.
    pub data: Vec<u8>,
}

impl InpaintMask {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self, InpaintingError> {
        let expected = width as usize * height as usize;
        if width == 0 || height == 0 || data.len() != expected {
            return Err(InpaintingError::new(
                InpaintingErrorCode::InvalidMask,
                format!(
                    "mask dimensions {width}x{height} require {expected} bytes, got {}",
                    data.len()
                ),
            ));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    fn binary_data(&self) -> Vec<u8> {
        self.data
            .iter()
            .map(|value| if *value == 0 { 0 } else { 255 })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct InpaintedPng {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InpaintingErrorCode {
    ModelMissing,
    RuntimeMissing,
    InvalidMask,
    ImageLoad,
    SessionInit,
    Inference,
    OutputInvalid,
    Encode,
    Worker,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InpaintingError {
    pub code: InpaintingErrorCode,
    pub message: String,
}

impl InpaintingError {
    fn new(code: InpaintingErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub struct InpaintingClient {
    models: Arc<OfflineModelManager>,
    session: OnceCell<Arc<StdMutex<Session>>>,
}

impl InpaintingClient {
    pub fn new(models: Arc<OfflineModelManager>) -> Arc<Self> {
        Arc::new(Self {
            models,
            session: OnceCell::new(),
        })
    }

    async fn session(&self) -> Result<Arc<StdMutex<Session>>, InpaintingError> {
        if !self.models.migan_ready() {
            return Err(InpaintingError::new(
                InpaintingErrorCode::ModelMissing,
                "migan_model_missing",
            ));
        }
        ensure_ort_init(&self.models).await.map_err(|error| {
            let code = if error == "onnxruntime_missing" {
                InpaintingErrorCode::RuntimeMissing
            } else {
                InpaintingErrorCode::SessionInit
            };
            InpaintingError::new(code, error)
        })?;

        let model_path = self
            .models
            .migan_path()
            .map_err(|error| InpaintingError::new(InpaintingErrorCode::ModelMissing, error))?;
        let session = self
            .session
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(move || {
                    Session::builder()
                        .map_err(|e| e.to_string())?
                        .with_intra_threads(4)
                        .map_err(|e| e.to_string())?
                        // ORT calls this option "parallel execution"; false selects sequential mode.
                        .with_parallel_execution(false)
                        .map_err(|e| e.to_string())?
                        .commit_from_file(model_path)
                        .map(|session| Arc::new(StdMutex::new(session)))
                        .map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| {
                    InpaintingError::new(
                        InpaintingErrorCode::Worker,
                        format!("session worker failed: {e}"),
                    )
                })?
                .map_err(|error| InpaintingError::new(InpaintingErrorCode::SessionInit, error))
            })
            .await?;
        Ok(session.clone())
    }

    /// `image`：调用方已解码的原图（替换翻译主流程本来就持有解码结果，
    /// 这里不再重新读盘/解码 PNG）。
    pub async fn inpaint_png(
        self: &Arc<Self>,
        image: Arc<RgbImage>,
        mask: InpaintMask,
    ) -> Result<InpaintedPng, InpaintingError> {
        let session = self.session().await?;
        tokio::task::spawn_blocking(move || run_inpainting(&session, &image, mask))
            .await
            .map_err(|error| {
                InpaintingError::new(
                    InpaintingErrorCode::Worker,
                    format!("inpainting worker failed: {error}"),
                )
            })?
    }
}

fn run_inpainting(
    session: &Arc<StdMutex<Session>>,
    source: &RgbImage,
    mask: InpaintMask,
) -> Result<InpaintedPng, InpaintingError> {
    let (width, height) = source.dimensions();
    if mask.width != width || mask.height != height {
        return Err(InpaintingError::new(
            InpaintingErrorCode::InvalidMask,
            format!(
                "mask is {}x{}, image is {width}x{height}",
                mask.width, mask.height
            ),
        ));
    }

    let pixel_count = width as usize * height as usize;
    let source_raw = source.as_raw();
    let mut image_nchw = vec![0u8; pixel_count * 3];
    for (index, pixel) in source_raw.chunks_exact(3).enumerate() {
        image_nchw[index] = pixel[0];
        image_nchw[pixel_count + index] = pixel[1];
        image_nchw[pixel_count * 2 + index] = pixel[2];
    }
    let binary_mask = mask.binary_data();
    // The public Kivio mask uses 255=hole. MI-GAN's exported pipeline uses the
    // opposite convention (255=known, 0=hole) for bbox detection and blending.
    let model_mask: Vec<u8> = binary_mask
        .iter()
        .map(|value| if *value == 0 { 255 } else { 0 })
        .collect();
    let image_tensor =
        Tensor::from_array((vec![1usize, 3, height as usize, width as usize], image_nchw))
            .map_err(|e| InpaintingError::new(InpaintingErrorCode::Inference, e.to_string()))?;
    let mask_tensor =
        Tensor::from_array((vec![1usize, 1, height as usize, width as usize], model_mask))
            .map_err(|e| InpaintingError::new(InpaintingErrorCode::Inference, e.to_string()))?;

    let mut session = session.lock().unwrap_or_else(|error| error.into_inner());
    let outputs = session
        .run(ort::inputs!["image" => image_tensor, "mask" => mask_tensor])
        .map_err(|e| InpaintingError::new(InpaintingErrorCode::Inference, e.to_string()))?;
    let (shape, output) = outputs["result"]
        .try_extract_tensor::<u8>()
        .map_err(|e| InpaintingError::new(InpaintingErrorCode::OutputInvalid, e.to_string()))?;
    if shape.as_ref() != [1, 3, height as i64, width as i64] || output.len() != pixel_count * 3 {
        return Err(InpaintingError::new(
            InpaintingErrorCode::OutputInvalid,
            format!("unexpected MI-GAN output shape {shape:?}"),
        ));
    }

    let image = blend_inpaint_output(source, &binary_mask, output)?;
    let png = encode_rgb_png(image)
        .map_err(|error| InpaintingError::new(InpaintingErrorCode::Encode, error))?;
    Ok(InpaintedPng { width, height, png })
}

/// Composites the MI-GAN result back onto the source, enforcing the mask-outside
/// invariant: every pixel where `binary_mask == 0` is copied byte-for-byte from
/// `source`, and only mask-inside pixels take the model output. `binary_mask` is
/// one byte per pixel (0 = keep source, non-zero = hole); `model_output_nchw` is
/// the MI-GAN NCHW u8 buffer (planar R, G, B, length == pixel_count * 3).
fn blend_inpaint_output(
    source: &RgbImage,
    binary_mask: &[u8],
    model_output_nchw: &[u8],
) -> Result<RgbImage, InpaintingError> {
    let (width, height) = source.dimensions();
    let pixel_count = width as usize * height as usize;
    if binary_mask.len() != pixel_count || model_output_nchw.len() != pixel_count * 3 {
        return Err(InpaintingError::new(
            InpaintingErrorCode::OutputInvalid,
            format!(
                "blend expects mask len {pixel_count} and output len {}, got mask {} output {}",
                pixel_count * 3,
                binary_mask.len(),
                model_output_nchw.len()
            ),
        ));
    }
    let source_raw = source.as_raw();
    let mut rgb = vec![0u8; pixel_count * 3];
    for index in 0..pixel_count {
        if binary_mask[index] == 0 {
            rgb[index * 3..index * 3 + 3].copy_from_slice(&source_raw[index * 3..index * 3 + 3]);
        } else {
            rgb[index * 3] = model_output_nchw[index];
            rgb[index * 3 + 1] = model_output_nchw[pixel_count + index];
            rgb[index * 3 + 2] = model_output_nchw[pixel_count * 2 + index];
        }
    }
    RgbImage::from_raw(width, height, rgb).ok_or_else(|| {
        InpaintingError::new(
            InpaintingErrorCode::OutputInvalid,
            "failed to construct output image",
        )
    })
}

/// RGB → PNG 的唯一编码入口；替换翻译的确定性填充路径也复用它。
pub(crate) fn encode_rgb_png(image: RgbImage) -> Result<Vec<u8>, String> {
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(|error| format!("encode cleaned image: {error}"))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_requires_exact_dimensions() {
        let error = InpaintMask::new(2, 2, vec![0; 3]).expect_err("invalid mask");
        assert_eq!(error.code, InpaintingErrorCode::InvalidMask);
        assert!(InpaintMask::new(2, 2, vec![0; 4]).is_ok());
    }

    #[test]
    fn mask_is_normalized_to_binary_255() {
        let mask = InpaintMask::new(3, 1, vec![0, 1, 128]).expect("valid mask");
        assert_eq!(mask.binary_data(), vec![0, 255, 255]);
    }

    /// Deterministic proof of the mask-outside invariant (AC-E3) without ORT, a
    /// real model, or the network: mask-outside pixels must equal the source
    /// byte-for-byte and mask-inside pixels must equal the decoded model output.
    #[test]
    fn blend_keeps_source_outside_mask_and_model_inside() {
        // 2x2 source, distinct byte per channel so mismatches are obvious.
        let source = RgbImage::from_raw(
            2,
            2,
            vec![
                10, 11, 12, // index 0 (0,0)
                20, 21, 22, // index 1 (1,0)
                30, 31, 32, // index 2 (0,1)
                40, 41, 42, // index 3 (1,1)
            ],
        )
        .expect("source image");
        let pixel_count = 4usize;
        // Keep indices 0 and 3; punch holes at indices 1 and 2.
        let binary_mask = vec![0u8, 255, 255, 0];
        // Fake NCHW planar output: R plane, then G plane, then B plane.
        let mut model_output = vec![0u8; pixel_count * 3];
        for i in 0..pixel_count {
            model_output[i] = 100 + i as u8;
            model_output[pixel_count + i] = 150 + i as u8;
            model_output[pixel_count * 2 + i] = 200 + i as u8;
        }
        let blended =
            blend_inpaint_output(&source, &binary_mask, &model_output).expect("blend output");
        let raw = blended.as_raw();
        // (a) mask==0 pixels are the source, byte-for-byte.
        assert_eq!(&raw[0..3], &[10, 11, 12]);
        assert_eq!(&raw[9..12], &[40, 41, 42]);
        // (b) mask!=0 pixels are the decoded fake model output.
        assert_eq!(&raw[3..6], &[101, 151, 201]);
        assert_eq!(&raw[6..9], &[102, 152, 202]);
    }

    #[test]
    fn blend_rejects_wrong_length_model_output() {
        let source = RgbImage::from_raw(2, 2, vec![0u8; 12]).expect("source image");
        let binary_mask = vec![0u8, 255, 255, 0];
        // (c) one byte short of pixel_count * 3.
        let short_output = vec![0u8; 11];
        let error = blend_inpaint_output(&source, &binary_mask, &short_output)
            .expect_err("wrong-length output");
        assert_eq!(error.code, InpaintingErrorCode::OutputInvalid);
    }

    /// Manual model integration check. Set KIVIO_MIGAN_TEST_DIR to a directory
    /// containing verified runtime files and inpainting/migan_pipeline_v2.onnx.
    #[tokio::test]
    #[ignore]
    async fn inpainting_real_e2e() {
        let dir = std::env::var_os("KIVIO_MIGAN_TEST_DIR")
            .map(std::path::PathBuf::from)
            .expect("set KIVIO_MIGAN_TEST_DIR");
        let input = std::env::var_os("KIVIO_MIGAN_TEST_IMAGE")
            .map(std::path::PathBuf::from)
            .expect("set KIVIO_MIGAN_TEST_IMAGE");
        let image = image::open(&input).expect("open input image").to_rgb8();
        let (width, height) = image.dimensions();
        let mut data = vec![0u8; width as usize * height as usize];
        let x0 = width / 3;
        let x1 = width * 2 / 3;
        let y0 = height / 3;
        let y1 = height * 2 / 3;
        for y in y0..y1 {
            for x in x0..x1 {
                data[y as usize * width as usize + x as usize] = 255;
            }
        }
        let manager = OfflineModelManager::with_model_dir(dir, reqwest::Client::new());
        let client = InpaintingClient::new(manager);
        let mask = InpaintMask::new(width, height, data).unwrap();
        let source = Arc::new(image.clone());
        let cold_started = std::time::Instant::now();
        let result = client
            .inpaint_png(source.clone(), mask.clone())
            .await
            .expect("run MI-GAN");
        let cold_elapsed = cold_started.elapsed();
        let hot_started = std::time::Instant::now();
        let hot = client
            .inpaint_png(source.clone(), mask.clone())
            .await
            .expect("run hot MI-GAN");
        let hot_elapsed = hot_started.elapsed();
        eprintln!("MI-GAN cold: {cold_elapsed:?}, hot: {hot_elapsed:?}");
        assert_eq!((result.width, result.height), (width, height));
        assert!(result.png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(hot_elapsed < std::time::Duration::from_millis(500));
        let output = image::load_from_memory(&hot.png)
            .expect("decode output")
            .to_rgb8();
        let mut changed_inside_mask = false;
        for (index, (source_pixel, output_pixel)) in image.pixels().zip(output.pixels()).enumerate()
        {
            if mask.data[index] == 0 {
                assert_eq!(output_pixel, source_pixel);
            } else if output_pixel != source_pixel {
                changed_inside_mask = true;
            }
        }
        assert!(
            changed_inside_mask,
            "MI-GAN must alter at least one masked pixel"
        );
    }
}
