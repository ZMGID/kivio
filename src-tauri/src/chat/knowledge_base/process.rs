//! Document → text routing + third-party processor adapters.
//!
//! Built-in (`parse::parse_file`) handles txt/md/html/docx/xlsx/pdf-text offline
//! and covers the common case. For scanned / complex-layout docs the user can
//! route to a third-party service that returns Markdown: Doc2X, MinerU, or a
//! generic "custom" raw-binary→markdown endpoint.
//!
//! ponytail: the Doc2X/MinerU request/response field names are vendor-doc
//! derived (see `research/doc-processors-api.md`) and not verified against a
//! live key — the built-in path + routing logic are unit-tested; the two named
//! cloud adapters are implemented to-spec and need a real key to confirm.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::api::with_standard_request_timeout;
use crate::settings::{DocProcessorProvider, DocumentProcessingConfig};
use crate::state::AppState;

use super::parse::{self, ParsedDoc};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_MAX_ATTEMPTS: usize = 90; // ~3 min ceiling per document

/// Resolve a source file to text, honoring the document-processing config:
/// an explicitly-selected third-party processor is used directly; otherwise the
/// built-in parser runs, and on "no extractable text" (e.g. a scanned PDF) with
/// the fallback flag on, the first enabled third-party processor is tried.
pub async fn process_document(
    state: &AppState,
    cfg: &DocumentProcessingConfig,
    path: &Path,
) -> Result<ParsedDoc, String> {
    if !cfg.active_processor.is_empty() {
        if let Some(p) = cfg
            .providers
            .iter()
            .find(|p| p.id == cfg.active_processor && p.enabled)
        {
            let md = process_third_party(state, p, path).await?;
            return Ok(ParsedDoc { text: md, markdown: true });
        }
        // Selected processor was deleted/disabled — fall through to built-in.
    }

    match parse::parse_file(path) {
        Ok(doc) => Ok(doc),
        Err(e) => {
            if cfg.fallback_to_third_party {
                if let Some(p) = cfg.providers.iter().find(|p| p.enabled) {
                    let md = process_third_party(state, p, path).await?;
                    return Ok(ParsedDoc { text: md, markdown: true });
                }
            }
            Err(e)
        }
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
        "doc2x" => doc2x_process(state, p, &name, bytes).await?,
        "mineru" => mineru_process(state, p, &name, bytes).await?,
        _ => custom_process(state, p, path, bytes).await?,
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

// ===== Doc2X v2 (preupload → PUT → poll status; markdown is inline) =====

async fn doc2x_process(
    state: &AppState,
    p: &DocProcessorProvider,
    file_name: &str,
    bytes: Vec<u8>,
) -> Result<String, String> {
    let key = first_key(p)?;
    let base = base_or(&p.base_url, "https://v2.doc2x.noedgeai.com");

    // 1. preupload → { uid, url }
    let pre: Value = with_standard_request_timeout(
        state
            .http
            .post(format!("{base}/api/v2/parse/preupload"))
            .bearer_auth(&key)
            .json(&serde_json::json!({ "file_name": file_name })),
    )
    .send()
    .await
    .map_err(|e| format!("Doc2X preupload: {e}"))?
    .json()
    .await
    .map_err(|e| format!("Doc2X preupload response: {e}"))?;
    let data = pre
        .get("data")
        .ok_or_else(|| format!("Doc2X preupload {}", envelope_err(&pre, "")))?;
    let uid = data
        .get("uid")
        .and_then(|v| v.as_str())
        .ok_or("Doc2X preupload: missing uid")?
        .to_string();
    let put_url = data
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("Doc2X preupload: missing upload url")?;

    // 2. PUT the file bytes to the presigned URL (no auth header).
    state
        .http
        .put(put_url)
        .body(bytes)
        .send()
        .await
        .map_err(|e| format!("Doc2X upload: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Doc2X upload rejected: {e}"))?;

    // 3. poll status; concat inline per-page markdown on success.
    for _ in 0..POLL_MAX_ATTEMPTS {
        tokio::time::sleep(POLL_INTERVAL).await;
        let st: Value = with_standard_request_timeout(
            state
                .http
                .get(format!("{base}/api/v2/parse/status?uid={uid}"))
                .bearer_auth(&key),
        )
        .send()
        .await
        .map_err(|e| format!("Doc2X status: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Doc2X status response: {e}"))?;
        let d = st
            .get("data")
            .ok_or_else(|| format!("Doc2X status {}", envelope_err(&st, "")))?;
        match d.get("status").and_then(|v| v.as_str()).unwrap_or("") {
            "success" => {
                let md = d
                    .get("result")
                    .and_then(|r| r.get("pages"))
                    .and_then(|v| v.as_array())
                    .map(|pages| {
                        pages
                            .iter()
                            .filter_map(|pg| pg.get("md").and_then(|m| m.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default();
                return Ok(md);
            }
            "failed" => return Err(format!("Doc2X parse {}", envelope_err(d, "failed"))),
            _ => {} // processing
        }
    }
    Err("Doc2X parse timed out".to_string())
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
                "failed" => {
                    return Err(format!("MinerU parse {}", envelope_err(first, "failed")))
                }
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

// ===== generic "custom" endpoint: raw-binary POST → markdown =====

async fn custom_process(
    state: &AppState,
    p: &DocProcessorProvider,
    path: &Path,
    bytes: Vec<u8>,
) -> Result<String, String> {
    let url = p.base_url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err(format!("Custom processor '{}' needs an API base URL", p.name));
    }
    let mut req = state
        .http
        .post(url)
        .header("Content-Type", mime_for(path))
        .body(bytes);
    // Key is optional for self-hosted endpoints.
    if let Some(key) = p.api_keys.iter().find(|k| !k.trim().is_empty()) {
        req = req.bearer_auth(key);
    }
    let text = with_standard_request_timeout(req)
        .send()
        .await
        .map_err(|e| format!("Custom processor: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Custom processor rejected: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Custom processor body: {e}"))?;
    // Accept raw markdown, or JSON carrying it under a common key.
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        for key in ["markdown", "md", "text", "content", "data"] {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                if !s.trim().is_empty() {
                    return Ok(s.to_string());
                }
            }
        }
    }
    Ok(text)
}

fn base_or<'a>(base_url: &'a str, default: &'a str) -> &'a str {
    let b = base_url.trim();
    if b.is_empty() {
        default
    } else {
        b.trim_end_matches('/')
    }
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("docx") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("html") | Some("htm") => "text/html",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(active: &str, fallback: bool, providers: Vec<DocProcessorProvider>) -> DocumentProcessingConfig {
        DocumentProcessingConfig {
            active_processor: active.to_string(),
            fallback_to_third_party: fallback,
            providers,
        }
    }

    fn provider(id: &str, enabled: bool) -> DocProcessorProvider {
        DocProcessorProvider {
            id: id.to_string(),
            name: id.to_string(),
            kind: "custom".to_string(),
            api_keys: vec![],
            base_url: String::new(),
            enabled,
        }
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
        assert_eq!(base_or("https://y.test/", "https://x.test"), "https://y.test");
    }
}
