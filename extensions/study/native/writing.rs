//! Writing uses the saved reading library as its sole source of bibliographic truth.
use super::{directory, list, ReadingDocument};
use serde::Serialize;
use std::io::Write;
use tauri::AppHandle;
pub mod word;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reference {
    pub id: String,
    pub name: String,
    pub bibliography: super::Bibliography,
}
pub(super) fn references(root: &std::path::Path) -> Result<Vec<Reference>, String> {
    Ok(list(root)?
        .into_iter()
        .filter(|d| d.trashed_at.is_none())
        .map(|d| Reference {
            id: d.id,
            name: d.name,
            bibliography: d.bibliography,
        })
        .collect())
}
fn bib_escape(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\\' => "\\textbackslash{}".into(),
            '{' => "\\{".into(),
            '}' => "\\}".into(),
            '%' => "\\%".into(),
            '&' => "\\&".into(),
            '_' => "\\_".into(),
            '#' => "\\#".into(),
            '$' => "\\$".into(),
            '~' => "\\textasciitilde{}".into(),
            '^' => "\\textasciicircum{}".into(),
            '\n' | '\r' => " ".into(),
            _ => c.to_string(),
        })
        .collect()
}
fn bibtex(docs: &[ReadingDocument]) -> String {
    let mut out = String::new();
    for doc in docs {
        let b = &doc.bibliography;
        let kind = match b.item_type.as_str() {
            "article" => "article",
            "conference" => "inproceedings",
            "book" => "book",
            "thesis" => "phdthesis",
            _ => "misc",
        };
        out.push_str(&format!("@{kind}{{kivio{},\n", doc.id.replace('-', "")));
        let mut field = |key: &str, value: &str| {
            if !value.trim().is_empty() {
                out.push_str(&format!("  {key} = {{{}}},\n", bib_escape(value)));
            }
        };
        field("title", &doc.name);
        field("year", &b.year);
        field(
            if kind == "inproceedings" {
                "booktitle"
            } else {
                "journal"
            },
            &b.publication,
        );
        field("volume", &b.volume);
        field("number", &b.issue);
        field("pages", &b.pages);
        field("publisher", &b.publisher);
        field("isbn", &b.isbn);
        field("doi", &b.doi);
        field("url", &b.url);
        if !b.authors.is_empty() {
            let authors = b
                .authors
                .iter()
                .filter(|a| !a.trim().is_empty())
                .map(|a| {
                    // Comma-separated names retain BibTeX family/given semantics; other names are literal.
                    if a.contains(',') {
                        bib_escape(a)
                    } else {
                        format!("{{{}}}", bib_escape(a))
                    }
                })
                .collect::<Vec<_>>()
                .join(" and ");
            out.push_str(&format!("  author = {{{authors}}},\n"));
        }
        out.push_str("}\n\n");
    }
    out
}
#[tauri::command]
pub async fn reading_export_bib(
    app: AppHandle,
    ids: Vec<String>,
    path: String,
) -> Result<(), String> {
    let _lease = crate::extension_sdk::command_lease("study").await?;
    tauri::async_runtime::spawn_blocking(move || {
        let all = list(&directory(&app)?)?;
        let docs = ids
            .iter()
            .map(|id| {
                all.iter()
                    .find(|d| &d.id == id && d.trashed_at.is_none())
                    .cloned()
                    .ok_or("选中的文献已移除，请刷新后重试".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let destination = std::path::Path::new(&path);
        if destination.extension().and_then(|s| s.to_str()) != Some("bib") {
            return Err("请选择 .bib 文件".into());
        }
        let mut staged =
            tempfile::NamedTempFile::new_in(destination.parent().ok_or("导出路径无效")?)
                .map_err(|e| e.to_string())?;
        staged
            .write_all(bibtex(&docs).as_bytes())
            .map_err(|e| e.to_string())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        staged.persist(destination).map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bibtex_preserves_identity_metadata_and_escapes_tex() {
        let mut doc = super::super::blank("中文 & {Research}_50%".into());
        doc.id = "11111111-2222-3333-4444-555555555555".into();
        doc.bibliography = super::super::Bibliography {
            authors: vec!["Smith, Ada".into(), "研究院 and Lab".into()],
            year: "2024".into(),
            item_type: "article".into(),
            volume: "8".into(),
            pages: "10--20".into(),
            ..Default::default()
        };
        let output = bibtex(&[doc.clone()]);
        assert!(output.starts_with("@article{kivio11111111222233334444555555555555,"));
        assert!(output.contains(r"中文 \& \{Research\}\_50\%"));
        assert!(output.contains("author = {Smith, Ada and {研究院 and Lab}}"));
        assert!(output.contains("volume = {8}"));
        doc.name = "Renamed".into();
        assert_eq!(output.lines().next(), bibtex(&[doc]).lines().next());
    }
}
