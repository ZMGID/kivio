//! Document parsing: extract plain text from a file by extension.
//! Built-in (Rust, offline): txt/md + text-like, pdf (`pdf-extract`),
//! docx (zip + WordprocessingML text), xlsx (`calamine`), html (`scraper`,
//! reusing the `web_fetch` article extractor). Image files are accepted here
//! but OCR'd upstream by `process.rs` (third-party processors are suspended).

use std::io::Read;
use std::path::Path;

/// Hard cap on a single source file. Mirrors the PRD's MVP guard (~20MB).
pub const MAX_DOC_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug)]
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
    "html", "htm", // image exts: accepted at upload time, OCR'd by process_document before parse.
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif",
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
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff" | "gif" => {
            Err("图片需经 OCR 处理（不应直接走 parse_file）".to_string())
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

/// Real end-to-end validation of `parse_file` against every supported extension
/// (plus the documented error paths). Each format gets a genuinely crafted file
/// on disk — no mocking of `parse_file` itself — and the extracted text is
/// printed for manual review. Gated behind `KB_PARSE_E2E=1` (mirrors
/// `rapidocr.rs::rapidocr_e2e`): unset, this is a no-op so normal `cargo test`
/// stays green and fast.
#[cfg(test)]
mod parse_e2e {
    use super::*;
    use std::io::Write;

    const CN: &str = "文档解析测试";
    const EN: &str = "Document Parsing Test";

    fn gated() -> bool {
        if std::env::var("KB_PARSE_E2E").as_deref() != Ok("1") {
            eprintln!("[parse-e2e] KB_PARSE_E2E != 1, skipping real end-to-end parser test");
            return false;
        }
        true
    }

    fn tmp_path(suffix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "kivio-parse-e2e-{}{suffix}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    fn show(label: &str, doc: &ParsedDoc) {
        eprintln!(
            "\n===== [{label}] markdown={} =====\n{}\n===== end [{label}] =====",
            doc.markdown, doc.text
        );
    }

    #[test]
    fn builtin_formats_real_e2e() {
        if !gated() {
            return;
        }

        // ---- txt ----
        {
            let path = tmp_path(".txt");
            std::fs::write(&path, format!("{CN} Kivio 2026\n{EN} line two.\n")).unwrap();
            let doc = parse_file(&path).expect("txt parse");
            show("txt", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(!doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- md ----
        {
            let path = tmp_path(".md");
            std::fs::write(&path, format!("# {CN} Kivio 2026\n\n{EN} paragraph.\n")).unwrap();
            let doc = parse_file(&path).expect("md parse");
            show("md", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- csv ----
        {
            let path = tmp_path(".csv");
            std::fs::write(&path, format!("标题,Title\n{CN},{EN}\n")).unwrap();
            let doc = parse_file(&path).expect("csv parse");
            show("csv", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(!doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- tsv ----
        {
            let path = tmp_path(".tsv");
            std::fs::write(&path, format!("标题\tTitle\n{CN}\t{EN}\n")).unwrap();
            let doc = parse_file(&path).expect("tsv parse");
            show("tsv", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(!doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- html ----
        {
            let path = tmp_path(".html");
            let html = format!(
                "<html><head><title>{CN}标题</title></head><body><h1>{CN} Kivio 2026</h1><p>{EN} paragraph body text.</p></body></html>"
            );
            std::fs::write(&path, &html).unwrap();
            let doc = parse_file(&path).expect("html parse");
            show("html", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(doc.markdown);
            assert!(
                !doc.text.contains("<h1>") && !doc.text.contains("<p>"),
                "tags leaked: {:?}",
                doc.text
            );
            let _ = std::fs::remove_file(&path);
        }

        // ---- htm ----
        {
            let path = tmp_path(".htm");
            let html = format!(
                "<html><head><title>{CN}标题</title></head><body><h1>{CN} Kivio 2026</h1><p>{EN} paragraph body text.</p></body></html>"
            );
            std::fs::write(&path, &html).unwrap();
            let doc = parse_file(&path).expect("htm parse");
            show("htm", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- docx (real zip + WordprocessingML) ----
        {
            let path = tmp_path(".docx");
            build_docx(&path, CN, EN);
            let doc = parse_file(&path).expect("docx parse");
            show("docx", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(doc.text.contains("Tab-separated"), "got: {:?}", doc.text);
            assert!(!doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- xlsx (real zip + OOXML, calamine-readable) ----
        {
            let path = tmp_path(".xlsx");
            build_xlsx(&path, CN, EN);
            let doc = parse_file(&path).expect("xlsx parse");
            show("xlsx", &doc);
            assert!(doc.text.contains(CN) && doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- pdf (real text layer, byte-exact xref) ----
        {
            let path = tmp_path(".pdf");
            // Base-14 Helvetica has no CJK glyphs/encoding, so a hand-rolled PDF
            // can't carry Chinese text honestly — English-only keyword here.
            let bytes = build_pdf(&format!("BT /F1 14 Tf 20 150 Td ({EN} Kivio 2026) Tj ET"));
            std::fs::write(&path, &bytes).unwrap();
            let doc = parse_file(&path).expect("pdf parse");
            show("pdf", &doc);
            assert!(doc.text.contains(EN), "got: {:?}", doc.text);
            assert!(!doc.markdown);
            let _ = std::fs::remove_file(&path);
        }

        // ---- pdf with no text objects ("scanned" error path) ----
        {
            let path = tmp_path("-scanned.pdf");
            let bytes = build_pdf("");
            std::fs::write(&path, &bytes).unwrap();
            let err = parse_file(&path).expect_err("empty-text pdf must error");
            eprintln!("[parse-e2e] scanned-pdf error: {err}");
            assert!(
                err.contains("scanned") && err.contains("No extractable text"),
                "got: {err}"
            );
            let _ = std::fs::remove_file(&path);
        }

        // ---- unsupported ext: .rtf ----
        {
            let path = tmp_path(".rtf");
            std::fs::write(&path, b"{\\rtf1 not really rtf}").unwrap();
            let err = parse_file(&path).expect_err("rtf must be rejected");
            eprintln!("[parse-e2e] rtf error: {err}");
            assert!(err.contains("Unsupported file type"), "got: {err}");
            let _ = std::fs::remove_file(&path);
        }

        // ---- unsupported ext: .pptx ----
        {
            let path = tmp_path(".pptx");
            std::fs::write(&path, b"PK fake pptx bytes, not a real ooxml package").unwrap();
            let err = parse_file(&path).expect_err("pptx must be rejected");
            eprintln!("[parse-e2e] pptx error: {err}");
            assert!(err.contains("Unsupported file type"), "got: {err}");
            let _ = std::fs::remove_file(&path);
        }

        // ---- image ext: .png (must NOT be parsed here — OCR happens upstream) ----
        {
            let path = tmp_path(".png");
            std::fs::write(&path, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
            let err = parse_file(&path).expect_err("image must not be parsed by parse_file");
            eprintln!("[parse-e2e] png error: {err}");
            assert!(err.contains("OCR"), "got: {err}");
            let _ = std::fs::remove_file(&path);
        }
    }

    fn build_docx(path: &std::path::Path, cn: &str, en: &str) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();

        z.start_file("[Content_Types].xml", opts).unwrap();
        z.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
        )
        .unwrap();

        z.start_file("_rels/.rels", opts).unwrap();
        z.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#,
        )
        .unwrap();

        z.start_file("word/document.xml", opts).unwrap();
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>{cn} Kivio 2026</w:t></w:r></w:p>
<w:p><w:r><w:t>第二段：</w:t></w:r><w:r><w:t>{en}</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>Tab-separated</w:t></w:r></w:p>
</w:body></w:document>"#
        );
        z.write_all(xml.as_bytes()).unwrap();

        z.finish().unwrap();
    }

    fn build_xlsx(path: &std::path::Path, cn: &str, en: &str) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();

        z.start_file("[Content_Types].xml", opts).unwrap();
        z.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
        )
        .unwrap();

        z.start_file("_rels/.rels", opts).unwrap();
        z.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
        )
        .unwrap();

        z.start_file("xl/workbook.xml", opts).unwrap();
        z.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
        )
        .unwrap();

        z.start_file("xl/_rels/workbook.xml.rels", opts).unwrap();
        z.write_all(
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#,
        )
        .unwrap();

        z.start_file("xl/worksheets/sheet1.xml", opts).unwrap();
        let sheet = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>{cn}</t></is></c><c r="B1" t="inlineStr"><is><t>Kivio 2026</t></is></c></row>
<row r="2"><c r="A2" t="inlineStr"><is><t>{en}</t></is></c><c r="B2" t="inlineStr"><is><t>中文</t></is></c></row>
</sheetData>
</worksheet>"#
        );
        z.write_all(sheet.as_bytes()).unwrap();

        z.finish().unwrap();
    }

    /// Hand-rolled minimal single-page PDF with byte-exact xref offsets so
    /// `pdf_extract::extract_text` can actually parse it (real object/xref/
    /// trailer structure, not a stub). `content_stream` is the raw page content
    /// (e.g. `BT ... Tj ET`); an empty string simulates a text-less ("scanned")
    /// page — same code path a real scanned PDF (image XObject, no text) hits,
    /// since `pdf_extract` only ever returns text-operator output either way.
    fn build_pdf(content_stream: &str) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();
        let mut offsets: Vec<usize> = vec![0]; // index 0 unused (free object)
        buf.extend_from_slice(b"%PDF-1.4\n");

        offsets.push(buf.len());
        buf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

        offsets.push(buf.len());
        buf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

        offsets.push(buf.len());
        buf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 300 200] /Contents 5 0 R >>\nendobj\n",
        );

        offsets.push(buf.len());
        buf.extend_from_slice(
            b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        );

        offsets.push(buf.len());
        let stream_bytes = content_stream.as_bytes();
        buf.extend_from_slice(
            format!("5 0 obj\n<< /Length {} >>\nstream\n", stream_bytes.len()).as_bytes(),
        );
        buf.extend_from_slice(stream_bytes);
        buf.extend_from_slice(b"\nendstream\nendobj\n");

        let xref_offset = buf.len();
        let obj_count = offsets.len(); // 6 (objects 0..=5)
        buf.extend_from_slice(format!("xref\n0 {obj_count}\n").as_bytes());
        buf.extend_from_slice(b"0000000000 65535 f \n");
        for off in offsets.iter().skip(1) {
            buf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        buf.extend_from_slice(
            format!("trailer\n<< /Size {obj_count} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF")
                .as_bytes(),
        );
        buf
    }
}
