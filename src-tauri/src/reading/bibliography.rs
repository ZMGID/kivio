//! Bibliographic exchange and DOI resolution; never executes or downloads source URLs.
use super::{Bibliography, ReadingDocument};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Lookup {
    pub name: String,
    pub bibliography: Bibliography,
    pub tags: Vec<String>,
}
pub async fn lookup(input: &str) -> Result<Lookup, String> {
    let doi = input
        .trim()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi:")
        .trim();
    if !doi.starts_with("10.") || !doi.contains('/') || doi.chars().any(char::is_whitespace) {
        return Err("请输入有效 DOI，例如 10.1038/nature14539".into());
    }
    if doi.to_lowercase().starts_with("10.48550/") {
        return datacite(doi).await;
    }
    let mut url =
        reqwest::Url::parse("https://api.crossref.org/works/").map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "无法构造 DOI 请求")?
        .pop_if_empty()
        .push(doi);
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("Kivio/3.1 (literature metadata)")
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("资料查询失败，请检查网络或手动录入：{e}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return datacite(doi).await;
    }
    if !response.status().is_success() {
        return Err(format!(
            "Crossref 未返回此 DOI 的资料（{}），可以手动录入",
            response.status()
        ));
    }
    let data: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let m = &data["message"];
    let name = m["title"][0]
        .as_str()
        .filter(|t| !t.trim().is_empty())
        .ok_or("此 DOI 没有可用标题")?
        .to_owned();
    let authors = m["author"]
        .as_array()
        .map(|authors| {
            authors
                .iter()
                .map(|a| {
                    a["name"].as_str().map(str::to_owned).unwrap_or_else(|| {
                        format!(
                            "{} {}",
                            a["given"].as_str().unwrap_or(""),
                            a["family"].as_str().unwrap_or("")
                        )
                        .trim()
                        .to_owned()
                    })
                })
                .filter(|a| !a.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let year = m["published"]["date-parts"][0][0]
        .as_i64()
        .map(|y| y.to_string())
        .unwrap_or_default();
    Ok(Lookup {
        name,
        bibliography: Bibliography {
            authors,
            year,
            publication: m["container-title"][0].as_str().unwrap_or("").into(),
            volume: m["volume"].as_str().unwrap_or("").into(),
            issue: m["issue"].as_str().unwrap_or("").into(),
            pages: m["page"].as_str().unwrap_or("").into(),
            publisher: m["publisher"].as_str().unwrap_or("").into(),
            isbn: m["ISBN"][0].as_str().unwrap_or("").into(),
            doi: m["DOI"].as_str().unwrap_or(doi).into(),
            url: m["URL"].as_str().unwrap_or("").into(),
            abstract_text: plain_text(m["abstract"].as_str().unwrap_or("")),
            item_type: match m["type"].as_str() {
                Some("book") => "book",
                Some("proceedings-article") => "conference",
                Some("posted-content") => "preprint",
                Some("journal-article") => "article",
                _ => "document",
            }
            .into(),
            ..Default::default()
        },
        tags: vec![],
    })
}
async fn datacite(doi: &str) -> Result<Lookup, String> {
    let mut url =
        reqwest::Url::parse("https://api.datacite.org/dois/").map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "无效 DOI")?
        .pop_if_empty()
        .push(doi);
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("DataCite 查询失败：{e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "未找到此 DOI 的公开资料（{}），请检查 DOI 或手动录入",
            response.status()
        ));
    }
    let data: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let m = &data["data"]["attributes"];
    let name = m["titles"][0]["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("此 DOI 没有可用标题")?
        .to_owned();
    Ok(Lookup {
        name,
        bibliography: Bibliography {
            authors: m["creators"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            year: m["publicationYear"]
                .as_i64()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            publication: m["publisher"].as_str().unwrap_or("").into(),
            doi: doi.into(),
            url: m["url"].as_str().unwrap_or("").into(),
            abstract_text: m["descriptions"]
                .as_array()
                .and_then(|a| a.iter().find(|d| d["descriptionType"] == "Abstract"))
                .and_then(|d| d["description"].as_str())
                .unwrap_or("")
                .into(),
            item_type: if doi.to_lowercase().starts_with("10.48550/")
                || m["types"]["resourceType"]
                    .as_str()
                    .is_some_and(|t| t.eq_ignore_ascii_case("preprint"))
            {
                "preprint"
            } else {
                "document"
            }
            .into(),
            ..Default::default()
        },
        tags: vec![],
    })
}
fn plain_text(text: &str) -> String {
    let mut inside = false;
    text.chars()
        .filter(|c| match c {
            '<' => {
                inside = true;
                false
            }
            '>' => {
                inside = false;
                false
            }
            _ => !inside,
        })
        .collect()
}
pub fn parse_ris(text: &str) -> Result<Vec<Lookup>, String> {
    let mut records = vec![];
    let mut current: Option<Lookup> = None;
    let mut last = String::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        if line.as_bytes().get(2..6) == Some(b"  - ") {
            let tag = &line[..2];
            let value = line[6..].trim();
            if tag == "TY" {
                if current.is_some() {
                    return Err("RIS 条目缺少 ER 结束标记".into());
                }
                current = Some(Lookup {
                    name: String::new(),
                    bibliography: Bibliography {
                        item_type: match value {
                            "BOOK" => "book",
                            "CONF" | "CPAPER" => "conference",
                            "THES" => "thesis",
                            "ELEC" => "webpage",
                            "UNPB" => "preprint",
                            "JOUR" | "JFULL" => "article",
                            _ => "document",
                        }
                        .into(),
                        ..Default::default()
                    },
                    tags: vec![],
                });
            } else if tag == "ER" {
                let item = current.take().ok_or("RIS 缺少 TY 起始标记")?;
                if item.name.is_empty() {
                    return Err("RIS 存在无标题条目，请补齐后导入".into());
                }
                records.push(item);
            } else if let Some(item) = &mut current {
                match tag {
                    "TI" | "T1" => item.name = value.into(),
                    "AU" | "A1" => item.bibliography.authors.push(value.into()),
                    "PY" | "Y1" => {
                        item.bibliography.year = value.split('/').next().unwrap_or(value).into()
                    }
                    "JO" | "JF" | "T2" => item.bibliography.publication = value.into(),
                    "DO" => {
                        item.bibliography.doi = value.trim_start_matches("https://doi.org/").into()
                    }
                    "VL" => item.bibliography.volume = value.into(),
                    "IS" => item.bibliography.issue = value.into(),
                    "SP" => item.bibliography.pages = value.into(),
                    "EP" => {
                        if !value.is_empty() {
                            item.bibliography.pages.push_str(&format!("--{value}"));
                        }
                    }
                    "PB" => item.bibliography.publisher = value.into(),
                    "SN" => item.bibliography.isbn = value.into(),
                    "UR" => item.bibliography.url = value.into(),
                    "AB" | "N2" => item.bibliography.abstract_text = value.into(),
                    "KW" => item.tags.push(value.into()),
                    _ => {}
                }
            }
            last = tag.into();
        } else if !line.trim().is_empty() {
            if let Some(item) = &mut current {
                let field = match last.as_str() {
                    "TI" | "T1" => Some(&mut item.name),
                    "AB" | "N2" => Some(&mut item.bibliography.abstract_text),
                    _ => None,
                };
                if let Some(field) = field {
                    field.push(' ');
                    field.push_str(line.trim());
                }
            }
        }
    }
    if current.is_some() || records.is_empty() {
        return Err("没有完整的 RIS 条目（需要 TY / ER 标记）".into());
    }
    Ok(records)
}
pub fn export_ris(docs: &[ReadingDocument]) -> String {
    let mut out = String::new();
    for doc in docs {
        let b = &doc.bibliography;
        let kind = match b.item_type.as_str() {
            "book" => "BOOK",
            "conference" => "CONF",
            "thesis" => "THES",
            "webpage" => "ELEC",
            "preprint" => "UNPB",
            "article" => "JOUR",
            _ => "GEN",
        };
        let mut field = |key: &str, value: &str| {
            if !value.is_empty() {
                out.push_str(&format!("{key}  - {}\n", value.replace(['\r', '\n'], " ")));
            }
        };
        field("TY", kind);
        field("TI", &doc.name);
        for author in &b.authors {
            field("AU", author);
        }
        field("PY", &b.year);
        field("T2", &b.publication);
        field("VL", &b.volume);
        field("IS", &b.issue);
        field("SP", &b.pages);
        field("PB", &b.publisher);
        field("SN", &b.isbn);
        field("DO", &b.doi);
        field("UR", &b.url);
        field("AB", &b.abstract_text);
        for tag in &doc.tags {
            field("KW", tag);
        }
        out.push_str("ER  - \n\n");
    }
    out
}
