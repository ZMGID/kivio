//! Document → text routing: Kivio built-in parse + optional third-party
//! parsing services (MinerU / LlamaParse), per the document-processing config.
//!
//! Built-in (`parse::parse_file`) handles txt/md/html/docx/xlsx/pdf-text
//! offline. Image files are OCR'd via the configured engine (system /
//! RapidOCR). For scanned / complex-layout docs the user can route to a
//! third-party service that returns Markdown: an explicitly selected
//! processor is used directly; otherwise built-in runs, and on failure with
//! the fallback flag on, the first enabled processor is tried.
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::api::with_standard_request_timeout;
use crate::settings::{DocProcessorProvider, DocumentProcessingConfig};
use crate::state::AppState;

use super::parse::{self, ParsedDoc};

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif"];

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_MAX_ATTEMPTS: usize = 90; // ~3 min ceiling per document

fn ext_lower(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn is_image_ext(path: &Path) -> bool {
    IMAGE_EXTS.contains(&ext_lower(path).as_str())
}

pub async fn process_document(
    state: &AppState,
    cfg: &DocumentProcessingConfig,
    path: &Path,
) -> Result<ParsedDoc, String> {
    // Images always go through the local OCR path — the third-party services
    // are document parsers, and OCR engine selection stays authoritative.
    if is_image_ext(path) {
        let text = ocr_image(state, path, &cfg.ocr_engine).await?;
        if text.trim().is_empty() {
            return Err("OCR 未识别到文字".into());
        }
        return Ok(ParsedDoc {
            text,
            markdown: false,
        });
    }

    // Explicitly selected third-party processor → use it directly.
    // Selected-but-deleted/disabled falls through to built-in.
    if !cfg.active_processor.is_empty() {
        if let Some(p) = cfg
            .providers
            .iter()
            .find(|p| p.id == cfg.active_processor && p.enabled)
        {
            let md = process_third_party(state, p, path).await?;
            return Ok(ParsedDoc {
                text: md,
                markdown: true,
            });
        }
    }

    let builtin = if ext_lower(path) == "pdf" && cfg.pdf_strategy == "force_ocr" {
        // 内置无法栅格化扫描版 PDF（未引入 pdfium）——但配置了第三方回退时可以救。
        Err("强制 OCR 重扫扫描版 PDF 暂未启用（内置仅支持 PDF 文字层）".to_string())
    } else {
        parse::parse_file(path)
    };
    match builtin {
        Ok(doc) => Ok(doc),
        Err(e) => {
            if cfg.fallback_to_third_party {
                if let Some(p) = cfg.providers.iter().find(|p| p.enabled) {
                    let md = process_third_party(state, p, path).await?;
                    return Ok(ParsedDoc {
                        text: md,
                        markdown: true,
                    });
                }
            }
            Err(e)
        }
    }
}

/// OCR one image via the selected engine, mirroring lens_commands' dispatch.
async fn ocr_image(state: &AppState, path: &Path, engine: &str) -> Result<String, String> {
    match engine {
        "system" => {
            #[cfg(target_os = "macos")]
            {
                return state.macos_ocr.ocr_image(&path.to_string_lossy()).await;
            }
            #[cfg(target_os = "windows")]
            {
                return crate::windows_ocr::ocr_image(path).await;
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            {
                let _ = (state, path);
                return Err("系统 OCR 在此平台不可用".into());
            }
        }
        "rapid_ocr" => state.rapidocr.ocr_image(path).await,
        _ => Err("图片入库需先在「文档处理」选择 OCR 引擎".into()),
    }
}

fn first_key(p: &DocProcessorProvider) -> Result<String, String> {
    p.api_keys
        .iter()
        .find(|k| !k.trim().is_empty())
        .cloned()
        .ok_or_else(|| format!("Document processor '{}' has no API key", p.name))
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".to_string())
}

async fn process_third_party(
    state: &AppState,
    p: &DocProcessorProvider,
    path: &Path,
) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let name = file_name_of(path);
    let md = match p.kind.as_str() {
        "mineru" => mineru_process(state, p, &name, bytes).await?,
        "llamaparse" => llamaparse_process(state, p, &name, bytes).await?,
        other => return Err(format!("Unknown document processor kind '{other}'")),
    };
    if md.trim().is_empty() {
        return Err(format!("Processor '{}' returned empty markdown", p.name));
    }
    Ok(md)
}

fn envelope_err(v: &Value, step: &str) -> String {
    let msg = v
        .get("msg")
        .or_else(|| v.get("detail"))
        .or_else(|| v.get("error"))
        .and_then(|m| m.as_str())
        .unwrap_or("unexpected response");
    format!("{step} error: {msg}")
}

// ===== MinerU cloud (file-urls/batch → PUT → poll batch → download zip) =====

async fn mineru_process(
    state: &AppState,
    p: &DocProcessorProvider,
    file_name: &str,
    bytes: Vec<u8>,
) -> Result<String, String> {
    let key = first_key(p)?;
    let base = base_or(&p.base_url, "https://mineru.net");

    // 1. request a presigned upload URL.
    let req: Value = with_standard_request_timeout(
        state
            .http
            .post(format!("{base}/api/v4/file-urls/batch"))
            .bearer_auth(&key)
            .json(&serde_json::json!({
                "enable_formula": true,
                "enable_table": true,
                "language": "auto",
                "files": [ { "name": file_name, "is_ocr": true } ]
            })),
    )
    .send()
    .await
    .map_err(|e| format!("MinerU upload-url: {e}"))?
    .json()
    .await
    .map_err(|e| format!("MinerU upload-url response: {e}"))?;
    let data = req
        .get("data")
        .ok_or_else(|| format!("MinerU upload-url {}", envelope_err(&req, "")))?;
    let batch_id = data
        .get("batch_id")
        .and_then(|v| v.as_str())
        .ok_or("MinerU: missing batch_id")?
        .to_string();
    let put_url = data
        .get("file_urls")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .ok_or("MinerU: missing file_urls")?;

    // 2. PUT the file bytes — NO Content-Type header (OSS presign gotcha).
    state
        .http
        .put(put_url)
        .body(bytes)
        .send()
        .await
        .map_err(|e| format!("MinerU upload: {e}"))?
        .error_for_status()
        .map_err(|e| format!("MinerU upload rejected: {e}"))?;

    // 3. poll the batch; on done, download the result zip and read full.md.
    for _ in 0..POLL_MAX_ATTEMPTS {
        tokio::time::sleep(POLL_INTERVAL).await;
        let st: Value = with_standard_request_timeout(
            state
                .http
                .get(format!("{base}/api/v4/extract-results/batch/{batch_id}"))
                .bearer_auth(&key),
        )
        .send()
        .await
        .map_err(|e| format!("MinerU status: {e}"))?
        .json()
        .await
        .map_err(|e| format!("MinerU status response: {e}"))?;
        let first = st
            .get("data")
            .and_then(|d| d.get("extract_result"))
            .and_then(|v| v.as_array())
            .and_then(|a| a.first());
        if let Some(first) = first {
            match first.get("state").and_then(|v| v.as_str()).unwrap_or("") {
                "done" => {
                    let zip_url = first
                        .get("full_zip_url")
                        .and_then(|v| v.as_str())
                        .ok_or("MinerU: done but no full_zip_url")?;
                    return mineru_download_md(state, zip_url).await;
                }
                "failed" => return Err(format!("MinerU parse {}", envelope_err(first, "failed"))),
                _ => {}
            }
        }
    }
    Err("MinerU parse timed out".to_string())
}

/// Download MinerU's result zip and extract its markdown (`full.md`, else the
/// first `.md` entry).
async fn mineru_download_md(state: &AppState, zip_url: &str) -> Result<String, String> {
    let bytes = state
        .http
        .get(zip_url)
        .send()
        .await
        .map_err(|e| format!("MinerU zip download: {e}"))?
        .error_for_status()
        .map_err(|e| format!("MinerU zip: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("MinerU zip body: {e}"))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("MinerU zip open: {e}"))?;
    let names: Vec<String> = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();
    let target = names
        .iter()
        .find(|n| n.ends_with("full.md"))
        .or_else(|| names.iter().find(|n| n.ends_with(".md")))
        .ok_or("MinerU zip has no markdown")?
        .clone();
    let mut s = String::new();
    zip.by_name(&target)
        .map_err(|e| format!("MinerU zip read: {e}"))?
        .read_to_string(&mut s)
        .map_err(|e| format!("MinerU md read: {e}"))?;
    Ok(s)
}

// ===== LlamaParse (multipart upload → poll job → fetch markdown result) =====

async fn llamaparse_process(
    state: &AppState,
    p: &DocProcessorProvider,
    file_name: &str,
    bytes: Vec<u8>,
) -> Result<String, String> {
    let key = first_key(p)?;
    let base = base_or(&p.base_url, "https://api.cloud.llamaindex.ai");

    // 1. multipart upload → { id }.
    let part = reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_string());
    let form = reqwest::multipart::Form::new().part("file", part);
    let up: Value = with_standard_request_timeout(
        state
            .http
            .post(format!("{base}/api/parsing/upload"))
            .bearer_auth(&key)
            .multipart(form),
    )
    .send()
    .await
    .map_err(|e| format!("LlamaParse upload: {e}"))?
    .json()
    .await
    .map_err(|e| format!("LlamaParse upload response: {e}"))?;
    let job_id = up
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("LlamaParse upload {}", envelope_err(&up, "")))?
        .to_string();

    // 2. poll job status until SUCCESS / ERROR.
    for _ in 0..POLL_MAX_ATTEMPTS {
        tokio::time::sleep(POLL_INTERVAL).await;
        let st: Value = with_standard_request_timeout(
            state
                .http
                .get(format!("{base}/api/parsing/job/{job_id}"))
                .bearer_auth(&key),
        )
        .send()
        .await
        .map_err(|e| format!("LlamaParse status: {e}"))?
        .json()
        .await
        .map_err(|e| format!("LlamaParse status response: {e}"))?;
        match st
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase()
            .as_str()
        {
            "SUCCESS" => {
                // 3. fetch the markdown result.
                let res: Value = with_standard_request_timeout(
                    state
                        .http
                        .get(format!("{base}/api/parsing/job/{job_id}/result/markdown"))
                        .bearer_auth(&key),
                )
                .send()
                .await
                .map_err(|e| format!("LlamaParse result: {e}"))?
                .json()
                .await
                .map_err(|e| format!("LlamaParse result response: {e}"))?;
                return res
                    .get("markdown")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .ok_or_else(|| format!("LlamaParse result {}", envelope_err(&res, "")));
            }
            "ERROR" | "CANCELED" => {
                return Err(format!("LlamaParse parse {}", envelope_err(&st, "failed")))
            }
            _ => {} // PENDING
        }
    }
    Err("LlamaParse parse timed out".to_string())
}

fn base_or<'a>(base_url: &'a str, default: &'a str) -> &'a str {
    let b = base_url.trim();
    if b.is_empty() {
        default
    } else {
        b.trim_end_matches('/')
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::state::AppState;
    use std::path::PathBuf;

    fn cfg(
        active: &str,
        fallback: bool,
        providers: Vec<DocProcessorProvider>,
    ) -> DocumentProcessingConfig {
        DocumentProcessingConfig {
            ocr_engine: "off".into(),
            pdf_strategy: "text".into(),
            active_processor: active.to_string(),
            fallback_to_third_party: fallback,
            providers,
        }
    }

    fn provider(id: &str, enabled: bool) -> DocProcessorProvider {
        DocProcessorProvider {
            id: id.to_string(),
            name: id.to_string(),
            kind: "mineru".to_string(),
            api_keys: vec![],
            base_url: String::new(),
            enabled,
        }
    }

    #[test]
    fn image_ext_detection() {
        assert!(is_image_ext(&PathBuf::from("a.png")));
        assert!(is_image_ext(&PathBuf::from("photo.JPG")));
        assert!(is_image_ext(&PathBuf::from("scan.tiff")));
        assert!(!is_image_ext(&PathBuf::from("doc.pdf")));
        assert!(!is_image_ext(&PathBuf::from("notes.md")));
        assert!(!is_image_ext(&PathBuf::from("noext")));
    }

    // Routing decisions are pure given the config; assert which branch a config
    // selects without hitting the network (a disabled/missing processor must
    // fall through to built-in, not error).
    fn picks_third_party(cfg: &DocumentProcessingConfig, builtin_failed: bool) -> bool {
        if !cfg.active_processor.is_empty()
            && cfg
                .providers
                .iter()
                .any(|p| p.id == cfg.active_processor && p.enabled)
        {
            return true;
        }
        builtin_failed && cfg.fallback_to_third_party && cfg.providers.iter().any(|p| p.enabled)
    }

    #[test]
    fn routing_branches() {
        // Built-in only.
        let c = cfg("", false, vec![]);
        assert!(!picks_third_party(&c, false));
        assert!(!picks_third_party(&c, true));

        // Explicit enabled processor → third-party even when built-in would work.
        let c = cfg("p1", false, vec![provider("p1", true)]);
        assert!(picks_third_party(&c, false));

        // Selected-but-disabled → fall through to built-in.
        let c = cfg("p1", false, vec![provider("p1", false)]);
        assert!(!picks_third_party(&c, false));

        // Fallback only kicks in when built-in failed AND a processor is enabled.
        let c = cfg("", true, vec![provider("p1", true)]);
        assert!(!picks_third_party(&c, false));
        assert!(picks_third_party(&c, true));

        // Fallback on but no enabled processor → stay built-in (surface the error).
        let c = cfg("", true, vec![provider("p1", false)]);
        assert!(!picks_third_party(&c, true));
    }

    #[test]
    fn base_url_defaulting() {
        assert_eq!(base_or("", "https://x.test"), "https://x.test");
        assert_eq!(base_or("  ", "https://x.test"), "https://x.test");
        assert_eq!(
            base_or("https://y.test/", "https://x.test"),
            "https://y.test"
        );
    }

    // engine "off" returns before touching state/disk, so a headless state is
    // enough — no network, no OCR engine needed.
    #[tokio::test]
    async fn off_engine_routes_image_to_select_engine_error() {
        let state = AppState::new_headless(Settings::default(), std::env::temp_dir());
        let c = cfg("", false, vec![]);
        let err = match process_document(&state, &c, &PathBuf::from("/tmp/x.png")).await {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert!(err.contains("选择 OCR 引擎"), "got: {err}");
    }

    #[tokio::test]
    async fn pdf_force_ocr_returns_honest_error() {
        let state = AppState::new_headless(Settings::default(), std::env::temp_dir());
        let mut c = cfg("", false, vec![]);
        c.pdf_strategy = "force_ocr".into();
        let err = match process_document(&state, &c, &PathBuf::from("/tmp/scan.pdf")).await {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert!(err.contains("强制 OCR"), "got: {err}");
    }

    // Explicitly selecting a third-party processor with no API key must fail
    // fast at the key check — proves the third-party branch was taken.
    #[tokio::test]
    async fn explicit_processor_routes_third_party() {
        let state = AppState::new_headless(Settings::default(), std::env::temp_dir());
        let c = cfg("p1", false, vec![provider("p1", true)]);
        let dir = std::env::temp_dir().join("kivio-test-docs");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("route-check.pdf");
        std::fs::write(&f, b"%PDF-1.4 not really").unwrap();
        let err = match process_document(&state, &c, &f).await {
            Ok(_) => panic!("expected error"),
            Err(e) => e,
        };
        assert!(err.contains("no API key"), "got: {err}");
        let _ = std::fs::remove_file(&f);
    }
}
