//! Document parsing: extract plain text from a file by extension.
//! MVP formats: txt/md (+ a few text-like) read directly; pdf via `pdf-extract`.
//! Scanned/image PDFs (no extractable text) and docx/xlsx are V2.

use std::path::Path;

/// Hard cap on a single source file. Mirrors the PRD's MVP guard (~20MB).
pub const MAX_DOC_BYTES: u64 = 20 * 1024 * 1024;

pub struct ParsedDoc {
    pub text: String,
    /// Whether to treat the text as markdown (heading-aware chunking).
    pub markdown: bool,
}

pub fn is_supported_ext(path: &Path) -> bool {
    matches!(ext_of(path).as_deref(), Some(e) if SUPPORTED.contains(&e))
}

const SUPPORTED: &[&str] = &[
    "txt", "text", "log", "csv", "tsv", "md", "markdown", "mdown", "mkd", "pdf",
];

fn ext_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

pub fn parse_file(path: &Path) -> Result<ParsedDoc, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?;
    if meta.len() > MAX_DOC_BYTES {
        return Err(format!(
            "file too large: {} bytes (max {})",
            meta.len(),
            MAX_DOC_BYTES
        ));
    }
    let ext = ext_of(path).unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" | "mdown" | "mkd" => Ok(ParsedDoc {
            text: read_text(path)?,
            markdown: true,
        }),
        "txt" | "text" | "log" | "csv" | "tsv" => Ok(ParsedDoc {
            text: read_text(path)?,
            markdown: false,
        }),
        "pdf" => {
            let text = pdf_extract::extract_text(path)
                .map_err(|e| format!("PDF text extraction failed: {e}"))?;
            if text.trim().is_empty() {
                return Err(
                    "No extractable text (scanned/image PDF — OCR import is not yet supported)"
                        .to_string(),
                );
            }
            Ok(ParsedDoc {
                text,
                markdown: false,
            })
        }
        other => Err(format!("Unsupported file type: .{other}")),
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
