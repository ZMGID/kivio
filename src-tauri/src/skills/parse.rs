use std::{collections::HashMap, path::Path};

use super::types::{parse_bool, slugify, SkillDetail, SkillMeta, SkillRecord};

pub fn split_frontmatter(raw: &str) -> (HashMap<String, String>, &str) {
    let mut map = HashMap::new();
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (map, raw);
    }
    let rest = &trimmed[3..];
    let Some(end) = rest.find("\n---") else {
        return (map, raw);
    };
    let fm = &rest[..end];
    let body = &rest[end + 4..];
    let mut current_key: Option<String> = None;
    let mut current_items: Vec<String> = Vec::new();
    for line in fm.lines() {
        let line = line.trim_end();
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            if current_key.is_some() {
                current_items.push(item.trim().to_string());
            }
            continue;
        }
        if let Some(key) = current_key.take() {
            if !current_items.is_empty() {
                map.insert(key, current_items.join(","));
                current_items.clear();
            }
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        if value.is_empty() {
            current_key = Some(key);
        } else {
            map.insert(key, value);
        }
    }
    if let Some(key) = current_key {
        if !current_items.is_empty() {
            map.insert(key, current_items.join(","));
        }
    }
    (map, body)
}

pub fn parse_list_value(value: Option<&String>) -> Vec<String> {
    value
        .map(|value| {
            value
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
                .filter(|item| !item.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_allowed_tools(frontmatter: &HashMap<String, String>) -> Vec<String> {
    let mut tools = parse_list_value(frontmatter.get("recommended-tools"));
    tools.extend(parse_list_value(frontmatter.get("mcp-tools")));
    if let Some(raw) = frontmatter.get("allowed-tools") {
        tools.extend(
            raw.split_whitespace()
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty()),
        );
    }
    tools.sort();
    tools.dedup();
    tools
}

pub fn parse_skill_markdown(
    raw: &str,
    source: &str,
    path: Option<String>,
    files: Vec<super::types::SkillFileEntry>,
) -> Result<SkillDetail, String> {
    let (frontmatter, body) = split_frontmatter(raw);
    let name = frontmatter
        .get("name")
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Skill name is required".to_string())?;
    let description = frontmatter
        .get("description")
        .cloned()
        .filter(|desc| !desc.trim().is_empty())
        .ok_or_else(|| "Skill description is required".to_string())?;
    let recommended_tools = parse_allowed_tools(&frontmatter);
    let disable_model_invocation = parse_bool(
        frontmatter
            .get("disable-model-invocation")
            .map(String::as_str),
    );
    let id = slugify(frontmatter.get("id").map(String::as_str).unwrap_or(&name));
    let triggers = parse_list_value(frontmatter.get("triggers"))
        .iter()
        .map(|t| super::types::normalize_trigger(t))
        .filter(|t| t.len() > 1)
        .collect();
    let argument_hint = frontmatter
        .get("argument-hint")
        .map(|hint| hint.trim().to_string())
        .filter(|hint| !hint.is_empty());
    let arguments = parse_list_value(frontmatter.get("arguments"));
    Ok(SkillDetail {
        meta: SkillMeta {
            id,
            name,
            description,
            source: source.to_string(),
            path,
            recommended_tools,
            disable_model_invocation,
            files,
            triggers,
            argument_hint,
            arguments,
        },
        body: body.trim().to_string(),
    })
}

pub fn parse_skill_record(
    skill_md_path: &Path,
    raw: &str,
    source: &str,
    files: Vec<super::types::SkillFileEntry>,
    warnings: &mut Vec<String>,
) -> Result<SkillRecord, String> {
    let base_dir = skill_md_path
        .parent()
        .ok_or_else(|| "Skill path has no parent directory".to_string())?
        .to_path_buf();
    let folder_name = base_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let detail = parse_skill_markdown(
        raw,
        source,
        Some(skill_md_path.display().to_string()),
        files,
    )?;
    if slugify(folder_name) != detail.meta.id && slugify(&detail.meta.name) != slugify(folder_name)
    {
        warnings.push(format!(
            "Skill folder {folder_name} does not match frontmatter name {}",
            detail.meta.name
        ));
    }
    let allowed_tools = detail.meta.recommended_tools.clone();
    Ok(SkillRecord {
        meta: detail.meta,
        location: skill_md_path.to_path_buf(),
        base_dir,
        body: detail.body,
        allowed_tools,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_allowed_tools, parse_list_value, parse_skill_markdown, split_frontmatter};
    use std::collections::HashMap;

    /// The vendored Obsidian skills must parse cleanly and their ids must match
    /// `OBSIDIAN_CONNECTOR_SKILL_IDS`, or the connector gate silently stops
    /// covering a skill (it would then always be visible, or never).
    #[test]
    fn vendored_obsidian_skills_parse_and_ids_match_gate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/skills");
        for id in crate::settings::OBSIDIAN_CONNECTOR_SKILL_IDS {
            let skill_md = root.join(id).join("SKILL.md");
            let raw = std::fs::read_to_string(&skill_md)
                .unwrap_or_else(|e| panic!("read {}: {e}", skill_md.display()));
            let parsed = parse_skill_markdown(&raw, "builtin", None, Vec::new())
                .unwrap_or_else(|e| panic!("parse {}: {e}", skill_md.display()));
            assert_eq!(
                &parsed.meta.id,
                id,
                "id mismatch for {}",
                skill_md.display()
            );
            assert!(!parsed.meta.description.trim().is_empty());
            assert!(!parsed.body.trim().is_empty());
        }
    }

    #[test]
    fn split_frontmatter_returns_raw_when_missing_delimiters() {
        let raw = "# Title\n\nBody";
        let (fm, body) = split_frontmatter(raw);
        assert!(fm.is_empty());
        assert_eq!(body, raw);
    }

    #[test]
    fn split_frontmatter_parses_scalar_fields_and_body() {
        let raw = "---\nname: PDF Skill\ndescription: Parse PDF files\n---\n\n# Instructions\n";
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm.get("name").map(String::as_str), Some("PDF Skill"));
        assert_eq!(
            fm.get("description").map(String::as_str),
            Some("Parse PDF files")
        );
        assert!(body.contains("# Instructions"));
    }

    #[test]
    fn split_frontmatter_parses_list_style_recommended_tools() {
        let raw = "---\nrecommended-tools:\n  - read_file\n  - write_file\n---\n\nBody\n";
        let (fm, _) = split_frontmatter(raw);
        assert_eq!(
            fm.get("recommended-tools").map(String::as_str),
            Some("read_file,write_file")
        );
    }

    #[test]
    fn parse_list_value_handles_bracketed_csv() {
        assert_eq!(
            parse_list_value(Some(&" [\"read_file\", 'write_file'] ".to_string())),
            vec!["read_file".to_string(), "write_file".to_string()]
        );
        assert!(parse_list_value(None).is_empty());
    }

    #[test]
    fn parse_allowed_tools_merges_recommended_mcp_and_allowed_tools() {
        let mut frontmatter = HashMap::new();
        frontmatter.insert(
            "recommended-tools".to_string(),
            "read_file,write_file".to_string(),
        );
        frontmatter.insert("mcp-tools".to_string(), "web_fetch".to_string());
        frontmatter.insert(
            "allowed-tools".to_string(),
            "edit_file read_file".to_string(),
        );
        assert_eq!(
            parse_allowed_tools(&frontmatter),
            vec![
                "edit_file".to_string(),
                "read_file".to_string(),
                "web_fetch".to_string(),
                "write_file".to_string(),
            ]
        );
    }

    #[test]
    fn parse_skill_markdown_reads_triggers_and_arguments() {
        let raw = r#"---
name: Commit Helper
description: Build a commit message.
triggers:
  - /commit
  - "ci"
argument-hint: <message>
arguments:
  - title
  - scope
---
# Body with $ARGUMENTS
"#;
        let parsed = super::parse_skill_markdown(raw, "user", None, Vec::new()).unwrap();
        assert_eq!(parsed.meta.triggers, vec!["/commit", "/ci"]);
        assert_eq!(parsed.meta.argument_hint.as_deref(), Some("<message>"));
        assert_eq!(parsed.meta.arguments, vec!["title", "scope"]);
    }
}
