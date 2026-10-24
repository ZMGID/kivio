//! Read-only automation catalog shared by desktop hosts. Never rewrites graphs.
use super::types::{Automation, SCHEMA_VERSION};
use std::{fs, path::Path};

pub fn read_one(path: &Path) -> Result<Automation, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("read automation failed: {err}"))?;
    let automation: Automation =
        serde_json::from_str(&raw).map_err(|err| format!("parse automation failed: {err}"))?;
    if automation.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported automation schemaVersion {}",
            automation.schema_version
        ));
    }
    Ok(automation)
}

/// Unreadable entries stay on disk and are reported independently of good graphs.
pub fn list_in(directory: &Path) -> Result<(Vec<Automation>, Vec<String>), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((vec![], vec![])),
        Err(err) => return Err(format!("list automations failed: {err}")),
    };
    let mut items = vec![];
    let mut warnings = vec![];
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                warnings.push(e.to_string());
                continue;
            }
        };
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        match read_one(&path) {
            Ok(item) => items.push(item),
            Err(error) => warnings.push(format!("{}: {error}", path.display())),
        }
    }
    items.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok((items, warnings))
}
