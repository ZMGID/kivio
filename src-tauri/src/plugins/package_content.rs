//! Read-only package content shared by desktop hosts.
use std::{collections::BTreeMap, fs, path::Path};
use serde_json::Value;
use crate::chat::workflow_hooks;

pub fn owner_enabled(packages_root: &Path, id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
        && read_json(&packages_root.join(id).join("record.json"))
            .is_ok_and(|value| value["enabled"] == true)
}

pub fn expand_skill_body(packages_root: &Path, id: &str, location: &Path, body: &str) -> Result<String, String> {
    let owner = id.strip_prefix("pkg-").and_then(|id| id.get(..36))
        .ok_or("Invalid package skill id")?;
    uuid::Uuid::parse_str(owner).map_err(|_| "Invalid package id")?;
    let root = packages_root.join(owner).join("content");
    if !owner_enabled(packages_root, owner) || !location.starts_with(&root) {
        return Err(format!("Skill plugin is unavailable: {id}"));
    }
    Ok(expand_body_at(&root, body))
}

pub(super) fn read_json(path: &Path) -> Result<Value, String> {
    if fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len()
        > 1024 * 1024
    {
        return Err("JSON file exceeds 1 MiB".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))
}
pub(super) fn expand_body_at(root: &Path, body: &str) -> String {
    let data = root.parent().unwrap_or(root).join("data");
    let env = [
        ("PLUGIN_ROOT", root),
        ("CLAUDE_PLUGIN_ROOT", root),
        ("PLUGIN_DATA", data.as_path()),
        ("CLAUDE_PLUGIN_DATA", data.as_path()),
    ]
    .into_iter()
    .map(|(key, path)| (key.into(), path.display().to_string()))
    .collect();
    workflow_hooks::expand(body, &env)
}

pub fn component_markdown(path: &Path) -> Result<String, String> {
    let raw = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let trimmed = raw.trim_start();
    let (mut fields, body) = if let Some(rest) = trimmed.strip_prefix("---") {
        let end = rest.find("\n---").ok_or("Unclosed component frontmatter")?;
        let fields: BTreeMap<String, serde_yaml::Value> = serde_yaml::from_str(&rest[..end])
            .map_err(|e| format!("Invalid component YAML: {e}"))?;
        (fields, &rest[end + 4..])
    } else {
        (BTreeMap::new(), raw.as_str())
    };
    if fields.contains_key("hooks")
        || fields.get("context").and_then(serde_yaml::Value::as_str) == Some("fork")
    {
        return Err("Component-local hooks and context: fork are not supported".into());
    }
    let fallback = if path.file_name().is_some_and(|n| n == "SKILL.md") {
        path.parent().and_then(Path::file_name)
    } else {
        path.file_stem()
    }
    .and_then(|s| s.to_str())
    .unwrap_or("component");
    fields
        .entry("name".into())
        .or_insert_with(|| serde_yaml::Value::String(fallback.into()));
    fields.entry("description".into()).or_insert_with(|| {
        serde_yaml::Value::String(format!("Run the {fallback} plugin component"))
    });
    let mut header = Vec::new();
    for (key, value) in fields {
        let text = match value {
            serde_yaml::Value::String(s) => s.replace(['\n', '\r'], " "),
            serde_yaml::Value::Bool(b) => b.to_string(),
            serde_yaml::Value::Sequence(items) => items
                .iter()
                .filter_map(serde_yaml::Value::as_str)
                .collect::<Vec<_>>()
                .join(","),
            _ => continue,
        };
        header.push(format!("{key}: {text}"));
    }
    Ok(format!("---\n{}\n---\n{body}", header.join("\n")))
}
