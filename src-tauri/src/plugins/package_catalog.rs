//! Package parsing and component discovery; no desktop commands or installation.
use std::{collections::BTreeMap, fs, path::{Component, Path, PathBuf}};
use serde::{Deserialize,Serialize};
use serde_json::{json,Value};
use crate::{chat::workflow_hooks::{self, Hook}, settings::ChatMcpServer};
use super::package_content::{read_json, component_markdown};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub format: String,
    pub source: String,
    pub revision: Option<String>,
    pub enabled: bool,
    pub components: BTreeMap<String, usize>,
    pub diagnostics: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<PackageMarketplace>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageMarketplace {
    pub source: String,
    pub name: String,
    pub plugin: String,
}
pub struct Resolved {
    pub package: Package,
    pub root: PathBuf,
    pub skills: Vec<PathBuf>,
    pub commands: Vec<PathBuf>,
    pub agents: Vec<PathBuf>,
    pub hooks: Vec<Hook>,
    pub servers: Vec<ChatMcpServer>,
}

pub fn contained(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("Component path must stay inside the plugin".into());
    }
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    let target = fs::canonicalize(root.join(relative)).map_err(|e| e.to_string())?;
    if !target.starts_with(&root) {
        return Err("Plugin path escapes its root".into());
    }
    Ok(target)
}
pub(super) fn paths(root: &Path, value: Option<&Value>, default: &str) -> Result<Vec<PathBuf>, String> {
    let values = match value {
        None => {
            return Ok(if root.join(default).exists() {
                vec![contained(root, default)?]
            } else {
                vec![]
            })
        }
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().ok_or("Component paths must be strings"))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("Component must be a path or array of paths".into()),
    };
    values.into_iter().map(|v| contained(root, v)).collect()
}
pub(super) fn configs(root: &Path, value: Option<&Value>, default: &str) -> Result<Vec<Value>, String> {
    match value {
        None => {
            if root.join(default).exists() {
                Ok(vec![read_json(&contained(root, default)?)?])
            } else {
                Ok(vec![])
            }
        }
        Some(Value::String(path)) => Ok(vec![read_json(&contained(root, path)?)?]),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                out.extend(configs(root, Some(item), default)?);
            }
            Ok(out)
        }
        Some(v) if v.is_object() => Ok(vec![v.clone()]),
        _ => Err("Configuration must be a path, object, or array".into()),
    }
}
/// Native v1 deliberately uses path declarations, keeping executable configuration
/// in component files and rejecting misspelled or unsupported manifest fields.
pub(super) fn validate_native_manifest(value: &Value) -> Result<(), String> {
    let fields = value
        .as_object()
        .ok_or("Kivio manifest must be an object")?;
    for key in fields.keys() {
        if ![
            "$schema",
            "schemaVersion",
            "name",
            "version",
            "description",
            "metadata",
            "skills",
            "commands",
            "agents",
            "hooks",
            "mcpServers",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("Unknown Kivio v1 manifest field: {key}"));
        }
    }
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err("Kivio schemaVersion must be 1".into());
    }
    let name = value.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty()
        || name.len() > 64
        || !name.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
    {
        return Err(
            "Kivio name must be 1–64 lowercase letters/digits separated by single hyphens".into(),
        );
    }
    if value
        .get("version")
        .and_then(Value::as_str)
        .is_none_or(|s| s.trim().is_empty())
    {
        return Err("Kivio version must be a nonempty string".into());
    }
    for key in ["$schema", "description"] {
        if value.get(key).is_some_and(|v| !v.is_string()) {
            return Err(format!("{key} must be a string"));
        }
    }
    if value.get("metadata").is_some_and(|v| !v.is_object()) {
        return Err("metadata must be an object".into());
    }
    for key in ["skills", "commands", "agents", "hooks", "mcpServers"] {
        if let Some(v) = value.get(key) {
            let entries = if let Some(items) = v.as_array() {
                items.iter().collect()
            } else {
                vec![v]
            };
            for entry in entries {
                let path = entry
                    .as_str()
                    .ok_or_else(|| format!("{key} must be a path or array of paths"))?;
                if !path.starts_with("./")
                    || path.len() <= 2
                    || path.contains('\\')
                    || path.contains(':')
                    || path.split('/').any(|p| p == "..")
                {
                    return Err(format!("{key} paths must start with ./ and stay inside the plugin using forward slashes"));
                }
            }
        }
    }
    Ok(())
}

/// A marketplace record pins the Claude format across preview, installation and reload.
/// Direct imports retain native > Codex > Claude detection without merging formats.
pub(super) fn manifest_location(
    root: &Path,
    marketplace: bool,
) -> Result<(&'static str, &'static str), String> {
    if marketplace {
        return Ok(("claude", ".claude-plugin/plugin.json"));
    }
    [("kivio", ".kivio-plugin/plugin.json"), ("codex", ".codex-plugin/plugin.json"), ("claude", ".claude-plugin/plugin.json")]
        .into_iter().find(|(_, file)| root.join(file).is_file())
        .ok_or_else(|| "No .kivio-plugin/plugin.json, .codex-plugin/plugin.json or .claude-plugin/plugin.json at the selected root".into())
}

pub fn resolve(root: &Path, mut package: Package, data: &Path) -> Result<Resolved, String> {
    let (format, manifest) = manifest_location(root, package.marketplace.is_some())?;
    let manifest = read_json(&contained(root, manifest)?)?;
    if format == "kivio" {
        validate_native_manifest(&manifest)?;
    }
    package.format = format.into();
    package.name = manifest
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("Plugin name is required")?
        .into();
    if !package
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("Plugin name must contain only letters, numbers and hyphens".into());
    }
    package.description = manifest
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .into();
    package.version = manifest
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    package.diagnostics.clear();
    for key in [
        "apps",
        "lspServers",
        "outputStyles",
        "dependencies",
        "experimental",
        "themes",
        "userConfig",
        "channels",
    ] {
        if manifest.get(key).is_some() {
            package
                .diagnostics
                .push(format!("Unsupported component: {key}"));
        }
    }
    for file in [".app.json", ".lsp.json", "settings.json"] {
        if root.join(file).exists() {
            package
                .diagnostics
                .push(format!("Unsupported host configuration: {file}"));
        }
    }
    let mut skills = paths(root, manifest.get("skills"), "skills")?;
    if format == "claude"
        && package.marketplace.is_none()
        && manifest.get("skills").is_some()
        && root.join("skills").exists()
    {
        let default = contained(root, "skills")?;
        if !skills.contains(&default) {
            skills.insert(0, default);
        }
    }
    let commands = paths(root, manifest.get("commands"), "commands")?;
    let agents = paths(root, manifest.get("agents"), "agents")?;
    let mut unsupported_agents = agents.clone();
    unsupported_agents.extend(component_files_bounded(&agents, 0, "toml"));
    unsupported_agents.sort();
    unsupported_agents.dedup();
    for path in &unsupported_agents {
        if path.is_file() && path.extension().is_some_and(|e| e != "md") {
            package.diagnostics.push(format!(
                "Only Markdown agent definitions are supported: {}",
                path.display()
            ));
        }
    }
    // Discover each component once. A skill's resource tree is not a component tree.
    let skills = skill_files(&skills, 0);
    let commands = markdown_files(&commands);
    let agents = markdown_files(&agents);
    let component_files = commands.iter().chain(&agents).chain(&skills);
    for file in component_files {
        if let Err(error) = component_markdown(file) {
            package
                .diagnostics
                .push(format!("{}: {error}", file.display()));
        }
    }
    let mut hooks = Vec::new();
    let mut hook_configs = configs(root, manifest.get("hooks"), "hooks/hooks.json")?;
    if format == "claude"
        && manifest.get("hooks").is_some()
        && root.join("hooks/hooks.json").exists()
    {
        let default = read_json(&contained(root, "hooks/hooks.json")?)?;
        if !hook_configs.contains(&default) {
            hook_configs.insert(0, default);
        }
    }
    for value in hook_configs {
        match workflow_hooks::parse(&value, &package.id, root, data) {
            Ok(mut h) => {
                if format == "kivio" {
                    for hook in &mut h {
                        hook.foreign_tool_names = false;
                    }
                }
                hooks.extend(h);
            }
            Err(e) => package.diagnostics.push(e),
        }
    }
    let mut env: BTreeMap<String, String> = std::env::vars().collect();
    for key in ["PLUGIN_ROOT", "CLAUDE_PLUGIN_ROOT"] {
        env.insert(key.into(), root.display().to_string());
    }
    for key in ["PLUGIN_DATA", "CLAUDE_PLUGIN_DATA"] {
        env.insert(key.into(), data.display().to_string());
    }
    let mut servers = Vec::new();
    let mut mcp_configs = configs(root, manifest.get("mcpServers"), ".mcp.json")?;
    if format == "claude" && manifest.get("mcpServers").is_some() && root.join(".mcp.json").exists()
    {
        let default = read_json(&contained(root, ".mcp.json")?)?;
        if !mcp_configs.contains(&default) {
            mcp_configs.insert(0, default);
        }
    }
    for config in mcp_configs {
        for (name, raw) in config
            .get("mcpServers")
            .or_else(|| config.get("mcp_servers"))
            .unwrap_or(&config)
            .as_object()
            .ok_or("mcpServers must be an object")?
        {
            let raw = expand_json(raw, &env);
            if raw.to_string().contains("${") {
                package
                    .diagnostics
                    .push(format!("MCP {name}: unresolved environment variable"));
                continue;
            }
            let transport = raw
                .get("type")
                .or_else(|| raw.get("transport"))
                .and_then(Value::as_str)
                .unwrap_or(if raw.get("url").is_some() {
                    "http"
                } else {
                    "stdio"
                });
            if !["stdio", "http", "streamable-http", "streamable_http"].contains(&transport) {
                package
                    .diagnostics
                    .push(format!("Unsupported MCP transport: {transport}"));
                continue;
            }
            let mut server: ChatMcpServer =
                serde_json::from_value(raw.clone()).map_err(|e| format!("MCP {name}: {e}"))?;
            server.id = format!(
                "plugin-package-{}-{}",
                package.id,
                crate::skills::slugify(name)
            );
            server.name = format!("{} / {name}", package.name);
            server.enabled = true;
            server.transport = if transport == "stdio" {
                "stdio"
            } else {
                "streamable_http"
            }
            .into();
            server.connector_id = Some(format!("plugin:package:{}", package.id));
            server.cwd = Some(root.display().to_string());
            if transport == "stdio" && server.command.trim().is_empty() {
                return Err(format!("MCP {name} requires command"));
            }
            if transport != "stdio" && server.url.trim().is_empty() {
                return Err(format!("MCP {name} requires url"));
            }
            for (key, value) in &env {
                if key.starts_with("PLUGIN_") || key.starts_with("CLAUDE_PLUGIN_") {
                    server.env.insert(key.clone(), value.clone());
                }
            }
            servers.retain(|s: &ChatMcpServer| s.id != server.id);
            servers.push(server);
        }
    }
    package.components = [
        ("skills", skills.len()),
        ("commands", commands.len()),
        ("agents", agents.len()),
        ("hooks", hooks.len()),
        ("mcp", servers.len()),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v))
    .collect();
    Ok(Resolved {
        package,
        root: root.into(),
        skills,
        commands,
        agents,
        hooks,
        servers,
    })
}
pub(super) fn expand_json(value: &Value, env: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(s) => Value::String(workflow_hooks::expand(s, env)),
        Value::Array(a) => Value::Array(a.iter().map(|v| expand_json(v, env)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), expand_json(v, env)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub(super) fn skill_files(paths: &[PathBuf], depth: usize) -> Vec<PathBuf> {
    if depth > 6 {
        return vec![];
    }
    let mut files = Vec::new();
    for path in paths {
        let Ok(meta) = fs::symlink_metadata(path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_file() {
            if path.file_name().is_some_and(|name| name == "SKILL.md") {
                files.push(path.clone());
            }
        } else if meta.is_dir() {
            let skill = path.join("SKILL.md");
            if fs::symlink_metadata(&skill).is_ok_and(|meta| meta.is_file()) {
                files.push(skill);
                continue;
            }
            if let Ok(entries) = fs::read_dir(path) {
                let children = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .collect::<Vec<_>>();
                files.extend(skill_files(&children, depth + 1));
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

pub fn markdown_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            files.push(path.clone());
        } else if let Ok(entries) = fs::read_dir(path) {
            let children = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink()))
                .collect::<Vec<_>>();
            files.extend(component_files_bounded(&children, 1, "md"));
        }
    }
    files.sort();
    files.dedup();
    files
}

pub(super) fn component_files_bounded(paths: &[PathBuf], depth: usize, extension: &str) -> Vec<PathBuf> {
    if depth > 6 {
        return vec![];
    }
    let mut out = Vec::new();
    for path in paths {
        if path.is_file() && path.extension().is_some_and(|e| e == extension) {
            out.push(path.clone());
        } else if let Ok(entries) = fs::read_dir(path) {
            let children = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| !m.file_type().is_symlink()))
                .collect::<Vec<_>>();
            out.extend(component_files_bounded(&children, depth + 1, extension));
        }
    }
    out
}

pub fn load_active(root: &Path, strict: bool) -> Result<Vec<Resolved>, String> {
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.to_string()),
    };
    let mut packages = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let id = entry.file_name().to_string_lossy().into_owned();
        if uuid::Uuid::parse_str(&id).is_err() {
            continue;
        }
        let load = || -> Result<Option<Resolved>, String> {
            let package: Package =
                serde_json::from_value(read_json(&entry.path().join("record.json"))?)
                    .map_err(|error| error.to_string())?;
            if !package.enabled {
                return Ok(None);
            }
            if package.id != id {
                return Err("Plugin record identity mismatch".into());
            }
            let resolved = resolve(
                &entry.path().join("content"),
                package,
                &entry.path().join("data"),
            )?;
            if !resolved.package.diagnostics.is_empty() {
                return Err(resolved.package.diagnostics.join("; "));
            }
            Ok(Some(resolved))
        };
        match load() {
            Ok(Some(package)) => packages.push(package),
            Err(error) if strict => return Err(format!("Plugin {id}: {error}")),
            _ => {}
        }
    }
    packages.sort_by(|a, b| a.package.id.cmp(&b.package.id));
    Ok(packages)
}
