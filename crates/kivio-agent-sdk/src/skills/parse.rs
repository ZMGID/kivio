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
    // 块标量（description: > / |）：收集其后的缩进行，遇到下一个顶层键再合并。
    let mut block: Option<(String, String, Vec<String>)> = None;
    for line in fm.lines() {
        let line = line.trim_end();
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if block.is_some() {
            if indented || line.is_empty() {
                if let Some((_, _, lines)) = block.as_mut() {
                    lines.push(line.trim().to_string());
                }
                continue;
            }
            if let Some((key, indicator, lines)) = block.take() {
                map.insert(key, finish_block_scalar(&indicator, &lines));
            }
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            if current_key.is_some() {
                current_items.push(item.trim().to_string());
            }
            continue;
        }
        // 缩进的 key: value 属于某个顶层键下的嵌套映射（如 metadata.*.description），
        // 不能当成顶层字段，否则会覆盖真正的 name / description。
        if indented {
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
        let raw_value = value.trim();
        if matches!(raw_value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
            block = Some((key, raw_value.to_string(), Vec::new()));
            continue;
        }
        let value = unquote_scalar(raw_value);
        if value.is_empty() {
            current_key = Some(key);
        } else {
            map.insert(key, value);
        }
    }
    if let Some((key, indicator, lines)) = block {
        map.insert(key, finish_block_scalar(&indicator, &lines));
    }
    if let Some(key) = current_key {
        if !current_items.is_empty() {
            map.insert(key, current_items.join(","));
        }
    }
    (map, body)
}

/// 合并块标量的行：折叠式（>）把相邻行接成一行、空行保留为换行；字面式（|）保留换行。
/// 后面没有缩进内容时不是块标量（例如笔记标题恰好是 ">"），按原值返回。
fn finish_block_scalar(indicator: &str, lines: &[String]) -> String {
    if lines.iter().all(|line| line.is_empty()) {
        return indicator.to_string();
    }
    if !indicator.starts_with('>') {
        return lines.join("\n").trim().to_string();
    }
    let mut out = String::new();
    for line in lines {
        if line.is_empty() {
            out.push('\n');
        } else {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            out.push_str(line);
        }
    }
    out.trim().to_string()
}

/// YAML 单行标量去引号：双引号按转义序列还原（\" \\ \n \t …），单引号把 '' 还原成 '；
/// 只去掉一对首尾引号。未加引号的值原样返回。
fn unquote_scalar(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                out.push(ch);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other @ ('"' | '\\' | '/' | '\'')) => out.push(other),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        }
        return out;
    }
    if bytes.len() >= 2 && bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'' {
        return value[1..value.len() - 1].replace("''", "'");
    }
    value.to_string()
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

pub fn parse_recommended_tools(frontmatter: &HashMap<String, String>) -> Vec<String> {
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
    let recommended_tools = parse_recommended_tools(&frontmatter);
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
    Ok(SkillRecord {
        meta: detail.meta,
        location: skill_md_path.to_path_buf(),
        base_dir,
        body: detail.body,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        parse_list_value, parse_recommended_tools, parse_skill_markdown, split_frontmatter,
    };
    use std::collections::HashMap;

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
    fn split_frontmatter_ignores_nested_keys_and_folds_block_scalars() {
        let raw = concat!(
            "---\n",
            "name: demo\n",
            "description: >-\n",
            "  Use when building a custom UI that\n",
            "  wakes a bot.\n",
            "metadata:\n",
            "  envVars:\n",
            "    - name: TOKEN\n",
            "      description: Nested, not the skill description\n",
            "---\nbody",
        );
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm.get("name").unwrap(), "demo");
        assert_eq!(
            fm.get("description").unwrap(),
            "Use when building a custom UI that wakes a bot."
        );
        assert_eq!(body.trim(), "body");

        let (fm, _) = split_frontmatter("---\ntitle: >\nupdated_at: x\n---\n");
        assert_eq!(fm.get("title").unwrap(), ">");
        assert_eq!(fm.get("updated_at").unwrap(), "x");
    }

    #[test]
    fn split_frontmatter_unquotes_yaml_scalars() {
        let raw = concat!(
            "---\n",
            "name: 'it''s'\n",
            "description: \"Use for \\\"automate me\\\", or \\\"x\\\"\"\n",
            "plain: a \"quoted\" word\n",
            "---\nbody",
        );
        let (fm, _) = split_frontmatter(raw);
        assert_eq!(fm.get("name").unwrap(), "it's");
        assert_eq!(
            fm.get("description").unwrap(),
            "Use for \"automate me\", or \"x\""
        );
        assert_eq!(fm.get("plain").unwrap(), "a \"quoted\" word");
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
    fn parse_recommended_tools_accepts_legacy_tool_declarations() {
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
            parse_recommended_tools(&frontmatter),
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
    #[test]
    fn parse_skill_supports_recommended_tools_and_mcp_tools_alias() {
        let raw = r#"---
name: test-skill
description: Uses selected tools.
recommended-tools:
  - web_search
mcp-tools:
  - fetch
  - web_search
allowed-tools: Bash(git:*)
---
# Body
"#;

        let parsed = parse_skill_markdown(raw, "user", None, Vec::new()).unwrap();

        assert!(parsed.meta.recommended_tools.contains(&"fetch".to_string()));
        assert!(parsed
            .meta
            .recommended_tools
            .contains(&"web_search".to_string()));
        assert!(parsed
            .meta
            .recommended_tools
            .iter()
            .any(|tool| tool.contains("Bash")));
    }

    #[test]
    fn parse_skill_requires_name_and_description() {
        let err = parse_skill_markdown(
            r#"---
description: Missing name.
---
# Body
"#,
            "user",
            None,
            Vec::new(),
        )
        .expect_err("missing name should be rejected");

        assert!(err.contains("name"));
    }

    #[test]
    fn disable_model_invocation_parses_from_frontmatter() {
        let parsed = parse_skill_markdown(
            r#"---
name: manual-only
description: Only when invoked explicitly.
disable-model-invocation: true
---
"#,
            "user",
            None,
            Vec::new(),
        )
        .unwrap();
        assert!(parsed.meta.disable_model_invocation);
    }
}
