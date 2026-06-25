//! Document parsing: extract plain text from a file by extension.
//! Built-in (Rust, offline): txt/md + text-like, pdf (`pdf-extract`),
//! docx (zip + WordprocessingML text), xlsx (`calamine`), html (`scraper`,
//! reusing the `web_fetch` article extractor). Scanned/image PDFs and complex
//! layouts route to a third-party processor — see `process.rs`.

use std::io::Read;
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
    "txt", "text", "log", "csv", "tsv", "md", "markdown", "mdown", "mkd", "pdf", "docx", "xlsx",
    "html", "htm",
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
        "html" | "htm" => Ok(ParsedDoc {
            text: crate::native_tools::html_to_text(&read_text(path)?),
            markdown: true,
        }),
        "docx" => Ok(ParsedDoc {
            text: parse_docx(path)?,
            markdown: false,
        }),
        "xlsx" => Ok(ParsedDoc {
            text: parse_xlsx(path)?,
            markdown: true,
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

/// docx = zip; the body text lives in `word/document.xml` as WordprocessingML.
fn parse_docx(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open docx: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("open docx zip: {e}"))?;
    let mut xml = String::new();
    zip.by_name("word/document.xml")
        .map_err(|e| format!("docx missing document.xml: {e}"))?
        .read_to_string(&mut xml)
        .map_err(|e| format!("read document.xml: {e}"))?;
    let text = docx_xml_to_text(&xml);
    if text.trim().is_empty() {
        return Err("docx has no extractable text".to_string());
    }
    Ok(text)
}

/// Extract visible text from a WordprocessingML body: `<w:t>` runs are the text,
/// `</w:p>` ends a paragraph (newline), `<w:tab/>`/`<w:br/>` are whitespace.
/// ponytail: tag-scan, no XML dep — we only want text, not structure. O(n²) on
/// the byte length via repeated `find`; fine under the 20MB cap. quick-xml if
/// we ever need tables/styles.
fn docx_xml_to_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let after = &rest[lt..];
        let Some(gt) = after.find('>') else { break };
        let tag = &after[1..gt]; // tag body without the angle brackets
        let name = tag.split([' ', '/', '>']).next().unwrap_or("");
        match name {
            "w:t" if !tag.ends_with('/') => {
                // text run: capture char data up to </w:t>
                let content_start = lt + gt + 1;
                if let Some(end) = rest[content_start..].find("</w:t>") {
                    let raw = &rest[content_start..content_start + end];
                    out.push_str(&html_escape::decode_html_entities(raw));
                    rest = &rest[content_start + end + "</w:t>".len()..];
                    continue;
                }
            }
            "w:tab" => out.push('\t'),
            "w:br" | "w:cr" => out.push('\n'),
            _ if tag == "/w:p" => out.push('\n'),
            _ => {}
        }
        rest = &after[gt + 1..];
    }
    out
}

/// xlsx via calamine: each sheet becomes an `# Sheet` section, rows are
/// tab-joined cells. Empty cells/sheets are skipped.
fn parse_xlsx(path: &Path) -> Result<String, String> {
    use calamine::{open_workbook_auto, Reader};
    let mut wb = open_workbook_auto(path).map_err(|e| format!("open xlsx: {e}"))?;
    let mut out = String::new();
    for name in wb.sheet_names() {
        let Ok(range) = wb.worksheet_range(&name) else {
            continue;
        };
        if range.is_empty() {
            continue;
        }
        out.push_str(&format!("# {name}\n"));
        for row in range.rows() {
            let line = row
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("\t");
            if !line.trim().is_empty() {
                out.push_str(&line);
                out.push('\n');
            }
        }
        out.push('\n');
    }
    if out.trim().is_empty() {
        return Err("xlsx has no extractable cells".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_xml_extracts_paragraph_text() {
        let xml = r#"<w:document><w:body>
            <w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t xml:space="preserve"> world</w:t></w:r></w:p>
            <w:p><w:r><w:t>第二段 &amp; 实体</w:t></w:r></w:p>
            </w:body></w:document>"#;
        let text = docx_xml_to_text(xml);
        assert!(text.contains("Hello world"), "got: {text:?}");
        assert!(text.contains("第二段 & 实体"), "got: {text:?}");
        // paragraph boundary preserved
        assert!(text.contains("world\n第二段") || text.contains("world \n第二段"), "got: {text:?}");
    }
}
