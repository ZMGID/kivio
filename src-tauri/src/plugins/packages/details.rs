//! Read-only component inspection shared by installed packages and marketplace previews.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    pub author: Option<String>,
    pub version: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub groups: Vec<Group>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub kind: String,
    pub items: Vec<Item>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub name: String,
    pub description: String,
}

fn markdown_item(root: &Path, path: &Path) -> Result<Item, String> {
    let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
    let path = contained(root, &relative.to_string_lossy())?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("Component exceeds 1 MiB".into());
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let fields: BTreeMap<String, serde_yaml::Value> =
        if let Some(rest) = raw.trim_start().strip_prefix("---") {
            let end = rest.find("\n---").ok_or("Unclosed component frontmatter")?;
            serde_yaml::from_str(&rest[..end]).map_err(|e| e.to_string())?
        } else {
            BTreeMap::new()
        };
    let fallback = if path.file_name().is_some_and(|n| n == "SKILL.md") {
        path.parent().and_then(Path::file_name)
    } else {
        path.file_stem()
    }
    .and_then(|n| n.to_str())
    .unwrap_or("component");
    Ok(Item {
        name: fields
            .get("name")
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or(fallback)
            .into(),
        description: fields
            .get("description")
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or("")
            .into(),
    })
}

fn read(root: &Path, normalized: bool) -> Result<Details, String> {
    let canonical = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let root = canonical.as_path();
    let (_, file) = manifest_location(root, normalized)?;
    let manifest = read_json(&contained(root, file)?)?;
    let metadata = manifest.get("metadata").unwrap_or(&manifest);
    let field = |name: &str| manifest.get(name).or_else(|| metadata.get(name));
    let string = |name: &str| field(name).and_then(Value::as_str).map(str::to_owned);
    let mut details = Details {
        author: field("author")
            .and_then(|v| v.as_str().or_else(|| v.get("name").and_then(Value::as_str)))
            .map(str::to_owned),
        version: string("version"),
        homepage: string("homepage"),
        license: string("license"),
        groups: vec![],
        diagnostics: vec![],
    };
    for (kind, key, default) in [
        ("mcp", "mcpServers", ".mcp.json"),
        ("skills", "skills", "skills"),
        ("commands", "commands", "commands"),
        ("agents", "agents", "agents"),
        ("hooks", "hooks", "hooks/hooks.json"),
    ] {
        let collected = (|| -> Result<Vec<Item>, String> {
            let mut items = Vec::new();
            if kind == "mcp" || kind == "hooks" {
                let mut values = configs(root, manifest.get(key), default)?;
                if file.starts_with(".claude-plugin")
                    && !normalized
                    && manifest.get(key).is_some()
                    && root.join(default).is_file()
                {
                    values.extend(configs(root, None, default)?);
                }
                for value in values {
                    if let Some(entries) = value
                        .get(key)
                        .or_else(|| {
                            if kind == "mcp" {
                                value.get("mcp_servers")
                            } else {
                                None
                            }
                        })
                        .unwrap_or(&value)
                        .as_object()
                    {
                        for (name, config) in entries {
                            items.push(Item {
                                name: name.clone(),
                                description: config
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .into(),
                            });
                        }
                    }
                }
            } else {
                let mut roots = paths(root, manifest.get(key), default)?;
                if kind == "skills"
                    && file.starts_with(".claude-plugin")
                    && !normalized
                    && manifest.get(key).is_some()
                    && root.join(default).exists()
                {
                    roots.push(contained(root, default)?);
                }
                for path in markdown_files(&roots) {
                    if kind == "skills" && !path.file_name().is_some_and(|n| n == "SKILL.md") {
                        continue;
                    }
                    match markdown_item(root, &path) {
                        Ok(item) => items.push(item),
                        Err(error) => details.diagnostics.push(format!(
                            "{}: {error}",
                            path.strip_prefix(root).unwrap_or(&path).display()
                        )),
                    }
                }
            }
            items.sort_by(|a, b| a.name.cmp(&b.name));
            items.dedup_by(|a, b| a.name == b.name);
            Ok(items)
        })();
        match collected {
            Ok(items) if !items.is_empty() => details.groups.push(Group {
                kind: kind.into(),
                items,
            }),
            Err(error) => details.diagnostics.push(format!("{kind}: {error}")),
            _ => {}
        }
    }
    Ok(details)
}

/// Inspect shipped native packages without staging, installing or executing them.
pub(crate) fn describe_bundled(root: &Path) -> Result<Details, String> {
    read(root, false)
}

#[tauri::command]
pub async fn plugin_packages_describe(id: String) -> Result<Details, String> {
    let dir = package_dir(&id)?;
    let record: Package =
        serde_json::from_value(read_json(&dir.join("record.json"))?).map_err(|e| e.to_string())?;
    read(&dir.join("content"), record.marketplace.is_some())
}

struct PreviewDirectory(PathBuf);
impl Drop for PreviewDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) async fn describe_source(
    source: &str,
    subdirectory: Option<&str>,
    revision: Option<&str>,
    entry: &Value,
) -> Result<Details, String> {
    let root = packages_root()?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let dir = PreviewDirectory(root.join(format!(".preview-{}", uuid::Uuid::new_v4())));
    fs::create_dir(&dir.0).map_err(|e| e.to_string())?;
    stage_content(source, subdirectory, revision, &dir.0).await?;
    let content = dir.0.join("content");
    prepare_marketplace_manifest(&content, entry)?;
    read(&content, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn partial_contents_and_failed_preview_leave_no_installed_records() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let installed = dir.path().join("installed");
        let _packages = TestPackagesRoot::new(&installed);
        fs::create_dir_all(source.join("skills/valid")).unwrap();
        fs::create_dir_all(source.join("skills/broken")).unwrap();
        fs::write(
            source.join("skills/valid/SKILL.md"),
            "---\nname: valid\ndescription: Useful skill\n---\nInstructions",
        )
        .unwrap();
        fs::write(source.join("skills/broken/SKILL.md"), "---\nunclosed").unwrap();
        let entry = json!({"name":"demo"});
        let details = describe_source(source.to_str().unwrap(), None, None, &entry)
            .await
            .unwrap();
        assert_eq!(details.groups[0].items[0].name, "valid");
        assert_eq!(details.diagnostics.len(), 1);
        assert!(!source.join(".claude-plugin").exists());
        assert_eq!(fs::read_dir(&installed).unwrap().count(), 0);
        assert!(
            describe_source(source.to_str().unwrap(), Some("../missing"), None, &entry)
                .await
                .is_err()
        );
        assert_eq!(fs::read_dir(&installed).unwrap().count(), 0);
    }
}
