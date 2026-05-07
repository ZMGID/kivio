//! RapidOCR 离线 OCR：跨平台 PaddleOCR ONNX pipeline。
//!
//! 设计原则:用户责任挂在「点一下下载」按钮上,代码这边只负责:
//! 1. 检测必备文件齐不齐(runtime + det + rec + keys；Windows 另需 provider shared DLL)
//! 2. 不齐就 install():逐个 HTTP GET 到 .tmp 后 atomic rename
//! 3. 齐了就在首次 OCR 时一次性 ort::init_from + OAROCRBuilder.build,缓存 pipeline
//!
//! 不做 SHA 校验、不做断点续传、不做进度事件——用户明确要求保持简单。
//! 失败就整体重下,留下的 .tmp 文件不污染最终路径。
//!
//! ONNX Runtime dylib 通过 `ort` 的 `load-dynamic` feature 在运行时加载,
//! 安装包不带任何 ONNX Runtime 二进制。

#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::collections::HashMap;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[cfg(target_os = "macos")]
use flate2::read::GzDecoder;
use oar_ocr::oarocr::{OAROCR, OAROCRBuilder};
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex, OnceCell};

#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
#[cfg(target_os = "windows")]
use windows::{
    core::PCWSTR,
    Win32::System::LibraryLoader::{
        LoadLibraryExW, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    },
};

/// 当前平台 dylib 在 app data 目录里的本地文件名。下载完落到 model_dir/DYLIB_NAME。
#[cfg(target_os = "macos")]
const DYLIB_NAME: &str = "libonnxruntime.dylib";
#[cfg(target_os = "windows")]
const DYLIB_NAME: &str = "onnxruntime.dll";

// ONNX Runtime 官方 release 不提供裸 dylib/dll,这里下载平台包后只抽取 runtime 文件。
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const DYLIB_URL: &str =
  "https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/onnxruntime-osx-arm64-1.24.4.tgz";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const DYLIB_URL: &str =
  "https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/onnxruntime-osx-x86_64-1.24.4.tgz";
#[cfg(target_os = "windows")]
const DYLIB_URL: &str =
  "https://github.com/microsoft/onnxruntime/releases/download/v1.24.4/onnxruntime-win-x64-1.24.4.zip";

#[cfg(target_os = "macos")]
const DYLIB_ARCHIVE_PATH: &str = "lib/libonnxruntime.dylib";
#[cfg(target_os = "windows")]
const DYLIB_ARCHIVE_PATH: &str = "lib/onnxruntime.dll";
#[cfg(target_os = "windows")]
const PROVIDERS_SHARED_NAME: &str = "onnxruntime_providers_shared.dll";
#[cfg(target_os = "windows")]
const PROVIDERS_SHARED_ARCHIVE_PATH: &str = "lib/onnxruntime_providers_shared.dll";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(5 * 60);

// PP-OCRv5 mobile zh+en——覆盖中英主场景,模型最小(det ~5MB + rec ~10MB + dict ~5KB)。
const DET_URL: &str =
    "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/pp-ocrv5_mobile_det.onnx";
const REC_URL: &str =
    "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/pp-ocrv5_mobile_rec.onnx";
const KEYS_URL: &str =
    "https://github.com/GreatV/oar-ocr/releases/download/v0.3.0/ppocrv5_dict.txt";

enum DownloadSource {
    File {
        url: &'static str,
    },
    Archive {
        url: &'static str,
        entry_suffix: &'static str,
    },
}

struct DownloadFile {
    name: &'static str,
    source: DownloadSource,
}

/// 必备文件:本地名 + 下载 URL。任一缺失则视为「未就绪」。
fn download_files() -> Vec<DownloadFile> {
    let mut files = vec![
        DownloadFile {
            name: DYLIB_NAME,
            source: DownloadSource::Archive {
                url: DYLIB_URL,
                entry_suffix: DYLIB_ARCHIVE_PATH,
            },
        },
    ];

    // Windows 官方 CPU 包会带 shared provider 运行时。ONNX Runtime 会从
    // onnxruntime.dll 同目录查 provider shared libs,所以跟主 DLL 放在一起。
    #[cfg(target_os = "windows")]
    files.push(DownloadFile {
        name: PROVIDERS_SHARED_NAME,
        source: DownloadSource::Archive {
            url: DYLIB_URL,
            entry_suffix: PROVIDERS_SHARED_ARCHIVE_PATH,
        },
    });

    files.extend([
        DownloadFile {
            name: "det.onnx",
            source: DownloadSource::File { url: DET_URL },
        },
        DownloadFile {
            name: "rec.onnx",
            source: DownloadSource::File { url: REC_URL },
        },
        DownloadFile {
            name: "keys.txt",
            source: DownloadSource::File { url: KEYS_URL },
        },
    ]);
    files
}

/// 前端拉状态用:模型是否就绪 + 模型目录(供 UI 显示)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrStatus {
    pub models_available: bool,
    pub model_dir: Option<String>,
}

/// install() 返回值:成功/失败 + 人类可读 message。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RapidOcrInstallResult {
    pub success: bool,
    pub message: String,
}

/// AppState 持有的 RapidOCR 客户端。线程安全,Tauri command 间共享。
pub struct RapidOcrClient {
    app: Option<AppHandle>,
    http: reqwest::Client,
    /// 首次 ocr_image 调用时初始化:ort::init_from + OAROCRBuilder.build,后续复用。
    /// init_from 全进程一次,所以这里也只能初始化一次;失败需重启 app 重试。
    pipeline: OnceCell<Arc<OAROCR>>,
    /// 防双击 Download 并发竞争 .tmp 文件。
    install_lock: Mutex<()>,
}

impl RapidOcrClient {
    pub fn new(app: &AppHandle, http: reqwest::Client) -> Arc<Self> {
        Arc::new(Self {
            app: Some(app.clone()),
            http,
            pipeline: OnceCell::new(),
            install_lock: Mutex::new(()),
        })
    }

    /// 测试用:不持有 AppHandle 的占位实例。所有 OCR 操作立即 Err。
    #[cfg(test)]
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            app: None,
            http: reqwest::Client::new(),
            pipeline: OnceCell::new(),
            install_lock: Mutex::new(()),
        })
    }

    /// 模型 + dylib 落盘位置:`{app_data_dir}/rapidocr-models/`。
    fn model_dir(&self) -> Result<PathBuf, String> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| "rapidocr disabled".to_string())?;
        let base = app.path().app_data_dir().map_err(|e| e.to_string())?;
        Ok(base.join("rapidocr-models"))
    }

    /// 必备文件全在才算就绪。任一缺失 → 前端渲染下载按钮。
    pub fn status(&self) -> RapidOcrStatus {
        let Ok(dir) = self.model_dir() else {
            return RapidOcrStatus {
                models_available: false,
                model_dir: None,
            };
        };
        let all_present = download_files()
            .iter()
            .all(|file| file_is_ready(&dir.join(file.name)));
        RapidOcrStatus {
            models_available: all_present,
            model_dir: Some(dir.to_string_lossy().into_owned()),
        }
    }

    /// 顺序下载 4 个文件:GET → 写 .tmp → rename。任一失败立刻返回 fail,不留半成品。
    /// install_lock 防止双击并发。
    pub async fn install(&self) -> RapidOcrInstallResult {
        let _guard = self.install_lock.lock().await;
        let dir = match self.model_dir() {
            Ok(d) => d,
            Err(e) => return fail(e),
        };
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            return fail(format!("mkdir failed: {e}"));
        }

        let mut archive_cache: HashMap<&'static str, Arc<Vec<u8>>> = HashMap::new();
        for file in download_files() {
            let name = file.name;
            let final_path = dir.join(name);
            if file_is_ready(&final_path) {
                continue;
            }
            let tmp_path = dir.join(format!("{name}.tmp"));
            let write_result = match file.source {
                DownloadSource::File { url } => {
                    match download_bytes(&self.http, name, url).await {
                        Ok(bytes) => tokio::fs::write(&tmp_path, bytes)
                            .await
                            .map_err(|e| format!("write {name}: {e}")),
                        Err(e) => Err(e),
                    }
                }
                DownloadSource::Archive { url, entry_suffix } => {
                    let bytes = match archive_cache.get(url) {
                        Some(bytes) => Arc::clone(bytes),
                        None => match download_bytes(&self.http, name, url).await {
                            Ok(bytes) => {
                                let bytes = Arc::new(bytes);
                                archive_cache.insert(url, Arc::clone(&bytes));
                                bytes
                            }
                            Err(e) => return fail(e),
                        },
                    };
                    let tmp_path = tmp_path.clone();
                    tokio::task::spawn_blocking(move || {
                        extract_archive_entry(bytes.as_slice(), entry_suffix, &tmp_path)
                    })
                    .await
                    .map_err(|e| format!("extract {name}: {e}"))
                    .and_then(|r| r.map_err(|e| format!("extract {name}: {e}")))
                }
            };
            if let Err(e) = write_result {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return fail(e);
            }
            if final_path.exists() {
                if final_path.is_file() {
                    let _ = tokio::fs::remove_file(&final_path).await;
                } else {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                    return fail(format!("{name} path exists but is not a file"));
                }
            }
            if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
                return fail(format!("rename {name}: {e}"));
            }
        }

        RapidOcrInstallResult {
            success: true,
            message: "RapidOCR 包下载完成".into(),
        }
    }

    /// OCR 主入口。文件不齐 → `rapidocr_models_missing` 错误码,前端渲染下载提示。
    /// 首次调用走 OnceCell init:ort::init_from 加载 dylib + 构建 pipeline(~1-3s)。
    /// 后续调用直接复用 pipeline,~200-500ms/张。
    pub async fn ocr_image(
        self: &Arc<Self>,
        image_path: &std::path::Path,
    ) -> Result<String, String> {
        let dir = self.model_dir()?;
        if !download_files()
            .iter()
            .all(|file| file_is_ready(&dir.join(file.name)))
        {
            return Err("rapidocr_models_missing".into());
        }

        let pipeline = self
            .pipeline
            .get_or_try_init(|| async {
                // 必须在所有其他 ort API 之前调用,且全进程一次。
                prepare_onnxruntime_dll_dir(&dir)?;
                ort::init_from(dir.join(DYLIB_NAME))
                    .map_err(|e| format!("ort init_from failed: {e}"))?
                    .commit();
                let p = OAROCRBuilder::new(
                    dir.join("det.onnx").to_string_lossy().into_owned(),
                    dir.join("rec.onnx").to_string_lossy().into_owned(),
                    dir.join("keys.txt").to_string_lossy().into_owned(),
                )
                .build()
                .map_err(|e| format!("OAROCRBuilder failed: {e}"))?;
                Ok::<_, String>(Arc::new(p))
            })
            .await?;

        // oar-ocr 同步 API,spawn_blocking 避免阻塞 tokio 调度器。
        let pipeline = pipeline.clone();
        let path = image_path.to_owned();
        let text = tokio::task::spawn_blocking(move || -> Result<String, String> {
            let img = oar_ocr::utils::load_image(&path).map_err(|e| format!("load_image: {e}"))?;
            let results = pipeline
                .predict(vec![img])
                .map_err(|e| format!("predict: {e}"))?;
            Ok(join_text_regions(&results))
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))??;

        Ok(text)
    }
}

async fn download_bytes(
    http: &reqwest::Client,
    name: &str,
    url: &str,
) -> Result<Vec<u8>, String> {
    match http.get(url).timeout(DOWNLOAD_TIMEOUT).send().await {
        Ok(resp) => match resp.error_for_status() {
            Ok(r) => r
                .bytes()
                .await
                .map(|b| b.to_vec())
                .map_err(|e| format!("read {name}: {e}")),
            Err(e) => Err(format!("HTTP {name}: {e}")),
        },
        Err(e) => Err(format!("connect {name}: {e}")),
    }
}

fn fail(msg: String) -> RapidOcrInstallResult {
    RapidOcrInstallResult {
        success: false,
        message: msg,
    }
}

fn file_is_ready(path: &Path) -> bool {
    path.is_file() && path.metadata().is_ok_and(|m| m.len() > 0)
}

#[cfg(target_os = "windows")]
fn prepare_onnxruntime_dll_dir(dir: &Path) -> Result<(), String> {
    unsafe {
        for dll in [DYLIB_NAME, PROVIDERS_SHARED_NAME] {
            let mut path: Vec<u16> = dir.join(dll).as_os_str().encode_wide().collect();
            path.push(0);
            let _ = LoadLibraryExW(
                PCWSTR::from_raw(path.as_ptr()),
                None,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
            .map_err(|e| format!("preload {dll} failed: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn prepare_onnxruntime_dll_dir(_dir: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn extract_archive_entry(bytes: &[u8], entry_suffix: &str, output: &Path) -> Result<(), String> {
    let reader = GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?;
        let path = path.to_string_lossy();
        if path.ends_with(entry_suffix) {
            let mut out = Vec::new();
            entry.read_to_end(&mut out).map_err(|e| e.to_string())?;
            std::fs::write(output, out).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("{entry_suffix} not found in archive"))
}

#[cfg(target_os = "windows")]
fn extract_archive_entry(bytes: &[u8], entry_suffix: &str, output: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        if file.name().ends_with(entry_suffix) {
            let mut out = Vec::new();
            file.read_to_end(&mut out).map_err(|e| e.to_string())?;
            std::fs::write(output, out).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("{entry_suffix} not found in archive"))
}

#[derive(Debug, Clone)]
struct OcrSpan {
    text: String,
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
}

impl OcrSpan {
    fn height(&self) -> f32 {
        (self.y_max - self.y_min).max(1.0)
    }

    fn center_x(&self) -> f32 {
        (self.x_min + self.x_max) / 2.0
    }

    fn center_y(&self) -> f32 {
        (self.y_min + self.y_max) / 2.0
    }
}

#[derive(Debug, Clone)]
struct OcrLine {
    text: String,
    x_min: f32,
    y_min: f32,
    x_max: f32,
    y_max: f32,
}

impl OcrLine {
    fn from_spans(mut spans: Vec<OcrSpan>) -> Self {
        spans.sort_by(|a, b| cmp_f32(a.center_x(), b.center_x()));
        let text = join_line_spans(&spans);
        let x_min = spans.iter().map(|s| s.x_min).fold(f32::INFINITY, f32::min);
        let y_min = spans.iter().map(|s| s.y_min).fold(f32::INFINITY, f32::min);
        let x_max = spans.iter().map(|s| s.x_max).fold(f32::NEG_INFINITY, f32::max);
        let y_max = spans.iter().map(|s| s.y_max).fold(f32::NEG_INFINITY, f32::max);
        Self {
            text,
            x_min,
            y_min,
            x_max,
            y_max,
        }
    }

    fn height(&self) -> f32 {
        (self.y_max - self.y_min).max(1.0)
    }

    fn center_y(&self) -> f32 {
        (self.y_min + self.y_max) / 2.0
    }
}

/// 把所有 OCR 结果按阅读顺序拼成 Markdown-friendly 纯文本。
///
/// DBNet 返回的是一组文本框,不是排版树。这里做轻量几何后处理:
/// 1. 先按动态行高聚合同一视觉行,避免固定 30px 桶把大/小字号混在一起。
/// 2. 再按行距和右边界判断软换行/段落断开。
/// 3. 常见项目符号转成 Markdown list,让前端原文区和翻译模型都更容易理解结构。
fn join_text_regions(results: &[oar_ocr::oarocr::OAROCRResult]) -> String {
    let mut spans = Vec::new();
    let mut fallback = Vec::new();

    for r in results {
        for region in &r.text_regions {
            let Some(text) = region.text.as_ref() else {
                continue;
            };
            let s = text.trim();
            if s.is_empty() {
                continue;
            }

            let bbox = &region.bounding_box;
            if bbox.points.is_empty() {
                fallback.push(s.to_string());
                continue;
            }

            let x_min = bbox.x_min();
            let y_min = bbox.y_min();
            let x_max = bbox.x_max();
            let y_max = bbox.y_max();
            if x_max <= x_min || y_max <= y_min {
                fallback.push(s.to_string());
                continue;
            }

            spans.push(OcrSpan {
                text: s.to_string(),
                x_min,
                y_min,
                x_max,
                y_max,
            });
        }
    }

    if spans.is_empty() {
        return fallback.join("\n\n");
    }

    let median_height = median(spans.iter().map(OcrSpan::height).collect()).unwrap_or(20.0);
    spans.sort_by(|a, b| cmp_f32(a.center_y(), b.center_y()).then(cmp_f32(a.x_min, b.x_min)));

    let mut grouped: Vec<Vec<OcrSpan>> = Vec::new();
    for span in spans {
        let same_line = grouped
            .last()
            .is_some_and(|line| span_belongs_to_line(line, &span, median_height));
        if same_line {
            grouped.last_mut().expect("line exists").push(span);
        } else {
            grouped.push(vec![span]);
        }
    }

    let mut lines: Vec<OcrLine> = grouped.into_iter().map(OcrLine::from_spans).collect();
    lines.sort_by(|a, b| cmp_f32(a.center_y(), b.center_y()).then(cmp_f32(a.x_min, b.x_min)));

    let mut out = format_ocr_lines(&lines);
    if !fallback.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&fallback.join("\n\n"));
    }
    out
}

fn span_belongs_to_line(line: &[OcrSpan], span: &OcrSpan, median_height: f32) -> bool {
    let x_min = line.iter().map(|s| s.x_min).fold(f32::INFINITY, f32::min);
    let y_min = line.iter().map(|s| s.y_min).fold(f32::INFINITY, f32::min);
    let x_max = line.iter().map(|s| s.x_max).fold(f32::NEG_INFINITY, f32::max);
    let y_max = line.iter().map(|s| s.y_max).fold(f32::NEG_INFINITY, f32::max);
    let line_height = (y_max - y_min).max(1.0);
    let span_height = span.height();
    let vertical_overlap = (y_max.min(span.y_max) - y_min.max(span.y_min)).max(0.0);
    let overlap_ratio = vertical_overlap / line_height.min(span_height);
    let center_delta = (((y_min + y_max) / 2.0) - span.center_y()).abs();
    if overlap_ratio < 0.45 && center_delta > line_height.min(span_height).max(median_height) * 0.55
    {
        return false;
    }

    let horizontal_gap = if span.x_min > x_max {
        span.x_min - x_max
    } else if x_min > span.x_max {
        x_min - span.x_max
    } else {
        0.0
    };
    horizontal_gap <= (median_height * 8.0).max(120.0)
}

fn format_ocr_lines(lines: &[OcrLine]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let median_height = median(lines.iter().map(OcrLine::height).collect()).unwrap_or(20.0);
    let doc_left = lines.iter().map(|l| l.x_min).fold(f32::INFINITY, f32::min);
    let doc_right = lines.iter().map(|l| l.x_max).fold(f32::NEG_INFINITY, f32::max);
    let doc_width = (doc_right - doc_left).max(1.0);

    let mut out = String::new();
    let mut prev_line: Option<&OcrLine> = None;
    let mut current_block_is_list = false;

    for line in lines {
        let text = normalize_line_text(&line.text);
        if text.is_empty() {
            continue;
        }
        let is_list = is_list_item(&text);

        if out.is_empty() {
            out.push_str(&text);
            current_block_is_list = is_list;
            prev_line = Some(line);
            continue;
        }

        let Some(prev) = prev_line else {
            out.push_str(&text);
            prev_line = Some(line);
            continue;
        };

        if should_merge_visual_line(
            prev,
            line,
            current_block_is_list,
            &out,
            &text,
            median_height,
            doc_left,
            doc_width,
        ) {
            append_inline(&mut out, &text);
        } else {
            if should_separate_blocks(
                prev,
                line,
                current_block_is_list,
                is_list,
                &out,
                median_height,
            ) {
                ensure_blank_line(&mut out);
            } else {
                ensure_line_break(&mut out);
            }
            out.push_str(&text);
            current_block_is_list = is_list;
        }
        prev_line = Some(line);
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn should_merge_visual_line(
    prev: &OcrLine,
    current: &OcrLine,
    current_block_is_list: bool,
    out: &str,
    current_text: &str,
    median_height: f32,
    doc_left: f32,
    doc_width: f32,
) -> bool {
    if is_list_item(current_text) {
        return false;
    }

    let vertical_gap = current.y_min - prev.y_max;
    if vertical_gap < 0.0 || vertical_gap > median_height * 1.6 {
        return false;
    }

    if current_block_is_list && current.x_min > prev.x_min + median_height * 1.2 {
        return true;
    }

    let starts_near_previous = current.x_min <= prev.x_min + median_height * 1.2;
    let prev_reaches_line_end = prev.x_max >= doc_left + doc_width * 0.68;
    let previous_text = out.rsplit(['\n', '\r']).next().unwrap_or(out);
    let previous_looks_heading = looks_like_heading(previous_text, prev.height(), median_height);
    if previous_looks_heading {
        return false;
    }

    starts_near_previous && prev_reaches_line_end && !ends_with_sentence_break(previous_text)
}

fn should_separate_blocks(
    prev: &OcrLine,
    current: &OcrLine,
    previous_block_is_list: bool,
    current_is_list: bool,
    out: &str,
    median_height: f32,
) -> bool {
    let previous_text = out.rsplit(['\n', '\r']).find(|s| !s.trim().is_empty()).unwrap_or(out);

    // Keep Markdown lists tight internally. Use a blank line only when entering or
    // leaving a list block so ReactMarkdown does not render every item as loose.
    if previous_block_is_list || current_is_list {
        return previous_block_is_list != current_is_list;
    }

    let vertical_gap = (current.y_min - prev.y_max).max(0.0);
    vertical_gap > median_height * 1.25
        || looks_like_heading(previous_text, prev.height(), median_height)
}

fn join_line_spans(spans: &[OcrSpan]) -> String {
    let mut out = String::new();
    let mut prev: Option<&OcrSpan> = None;
    for span in spans {
        let text = collapse_spaces(&span.text);
        if text.is_empty() {
            continue;
        }
        if let Some(prev_span) = prev {
            let gap = (span.x_min - prev_span.x_max).max(0.0);
            if should_insert_inline_space(&out, &text, gap, prev_span.height().max(span.height())) {
                out.push(' ');
            }
        }
        out.push_str(&text);
        prev = Some(span);
    }
    out
}

fn normalize_line_text(text: &str) -> String {
    let text = collapse_spaces(text);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let Some(ordered) = normalize_ordered_list_item(trimmed) {
        return ordered;
    }

    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let rest = chars.as_str().trim_start();
    let marker = matches!(first, '•' | '·' | '●' | '○' | '◦' | '▪' | '▫' | '-' | '*' | '–' | '—')
        || ((first == 'O' || first == 'o' || first == '0')
            && !rest.is_empty()
            && rest.chars().next().is_some_and(|c| c.is_uppercase() || !c.is_ascii()));

    if marker && !rest.is_empty() {
        format!("- {rest}")
    } else {
        trimmed.to_string()
    }
}

fn normalize_ordered_list_item(text: &str) -> Option<String> {
    let mut digit_end = 0;
    let mut digit_count = 0;
    for (idx, c) in text.char_indices() {
        if c.is_ascii_digit() {
            digit_count += 1;
            digit_end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if digit_count == 0 || digit_count > 3 || digit_end >= text.len() {
        return None;
    }

    let marker_text = &text[digit_end..];
    let marker = marker_text.chars().next()?;
    if !matches!(marker, '.' | ')' | '）' | '、') {
        return None;
    }

    let after_marker = &marker_text[marker.len_utf8()..];
    let had_space = after_marker
        .chars()
        .next()
        .is_some_and(|c| c.is_whitespace());
    let rest = after_marker.trim_start();
    if rest.is_empty() {
        return None;
    }

    // Avoid turning version-like lines such as "1.24.4" into a list item.
    if !had_space && rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }

    Some(format!("{}. {}", &text[..digit_end], rest))
}

fn should_insert_inline_space(prev_text: &str, next_text: &str, gap: f32, line_height: f32) -> bool {
    if prev_text.is_empty() || next_text.is_empty() || gap <= 1.0 {
        return false;
    }
    let Some(prev) = prev_text.chars().last() else {
        return false;
    };
    let Some(next) = next_text.chars().next() else {
        return false;
    };
    if prev.is_whitespace()
        || next.is_whitespace()
        || matches!(next, ',' | '.' | ':' | ';' | ')' | ']' | '}' | '，' | '。' | '、' | '：' | '；' | '）' | '】')
        || matches!(prev, '(' | '[' | '{' | '（' | '【')
    {
        return false;
    }
    gap > line_height * 0.15 || (is_ascii_word(prev) && is_ascii_word(next))
}

fn append_inline(out: &mut String, text: &str) {
    if out.ends_with('-') {
        out.pop();
    } else if out
        .chars()
        .last()
        .is_some_and(|c| !c.is_whitespace() && needs_space_before(text, c))
    {
        out.push(' ');
    }
    out.push_str(text);
}

fn needs_space_before(next_text: &str, previous: char) -> bool {
    let Some(next) = next_text.chars().next() else {
        return false;
    };
    !is_cjk(previous)
        && !is_cjk(next)
        && !matches!(next, ',' | '.' | ':' | ';' | ')' | ']' | '}' | '，' | '。' | '、' | '：' | '；' | '）' | '】')
}

fn ensure_blank_line(out: &mut String) {
    if out.ends_with("\n\n") {
        return;
    }
    if out.ends_with('\n') {
        out.push('\n');
    } else {
        out.push_str("\n\n");
    }
}

fn ensure_line_break(out: &mut String) {
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

fn is_list_item(text: &str) -> bool {
    is_markdown_bullet(text) || is_ordered_list_item(text)
}

fn is_markdown_bullet(text: &str) -> bool {
    text.trim_start().starts_with("- ")
}

fn is_ordered_list_item(text: &str) -> bool {
    let text = text.trim_start();
    let mut digit_end = 0;
    let mut digit_count = 0;
    for (idx, c) in text.char_indices() {
        if c.is_ascii_digit() {
            digit_count += 1;
            digit_end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    if digit_count == 0 || digit_count > 3 || digit_end >= text.len() {
        return false;
    }
    text[digit_end..].starts_with(". ")
}

fn looks_like_heading(text: &str, height: f32, median_height: f32) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text.chars().count() <= 80
        && height >= median_height * 1.2
        && !ends_with_sentence_break(text)
        && !is_list_item(text)
}

fn ends_with_sentence_break(text: &str) -> bool {
    text.trim_end()
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '.' | '!' | '?' | ':' | ';' | '。' | '！' | '？' | '：' | '；'))
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn median(mut values: Vec<f32>) -> Option<f32> {
    values.retain(|v| v.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| cmp_f32(*a, *b));
    Some(values[values.len() / 2])
}

fn cmp_f32(a: f32, b: f32) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x3040..=0x30ff | 0xac00..=0xd7af
    )
}
