//! Manifest-based, user-scoped plugins. Package files are copied into Kivio ownership;
//! importing never executes an installer. Existing curated CLI plugins are independent.
use crate::{
    chat::workflow_hooks::{self, Hook},
    settings::ChatMcpServer,
    state::AppState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};
use tauri::{AppHandle, State};

fn packages_root() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(root) = TEST_PACKAGES_ROOT.with(|root| root.borrow().clone()) {
        return Ok(root);
    }
    super::plugins_root()
        .map(|p| p.join("packages"))
        .ok_or("Application data directory unavailable".into())
}

// Scoped to the test thread so runtime tests can use real package records without
// touching installed plugins or changing process-wide environment variables.
#[cfg(test)]
thread_local! {
    static TEST_PACKAGES_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct TestPackagesRoot(Option<PathBuf>);

#[cfg(test)]
impl TestPackagesRoot {
    pub(crate) fn new(root: &Path) -> Self {
        Self(TEST_PACKAGES_ROOT.with(|slot| slot.replace(Some(root.to_path_buf()))))
    }
}

#[cfg(test)]
impl Drop for TestPackagesRoot {
    fn drop(&mut self) {
        TEST_PACKAGES_ROOT.with(|slot| slot.replace(self.0.take()));
    }
}
fn package_dir(id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid package id")?;
    Ok(packages_root()?.join(id))
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}
fn load(id: &str) -> Result<Resolved, String> {
    let dir = package_dir(id)?;
    let package: Package =
        serde_json::from_value(read_json(&dir.join("record.json"))?).map_err(|e| e.to_string())?;
    if package.id != id {
        return Err("Plugin record identity mismatch".into());
    }
    resolve(&dir.join("content"), package, &dir.join("data"))
}
pub fn active() -> Vec<Resolved> {
    load_active(false).unwrap_or_default()
}

/// A reload must not silently replace a working catalog with partial resources.
pub(crate) fn discover_active() -> Result<Vec<Resolved>, String> {
    load_active(true)
}

fn load_active(strict: bool) -> Result<Vec<Resolved>, String> {
    super::package_catalog::load_active(&packages_root()?, strict)
}
pub fn owner_enabled(id: &str) -> bool {
    packages_root().is_ok_and(|root| super::package_content::owner_enabled(&root, id))
}
pub(crate) fn list() -> Result<Vec<Package>, String> {
    let root = packages_root()?;
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())?.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if uuid::Uuid::parse_str(&id).is_err() {
            continue;
        }
        match load(&id) {
            Ok(p) => out.push(p.package),
            Err(e) => {
                if let Ok(value) = read_json(&entry.path().join("record.json")) {
                    if let Ok(mut p) = serde_json::from_value::<Package>(value) {
                        p.diagnostics = vec![e];
                        out.push(p);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(out)
}
fn copy_tree(source: &Path, target: &Path, budget: &mut u64, depth: usize) -> Result<(), String> {
    if depth > 32 {
        return Err("Package nesting exceeds 32 directories".into());
    }
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() == ".git" || entry.file_name() == "node_modules" {
            continue;
        }
        let meta = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("Plugin imports cannot contain symlinks".into());
        }
        let canonical = fs::canonicalize(entry.path()).map_err(|e| e.to_string())?;
        if !canonical.starts_with(fs::canonicalize(source).map_err(|e| e.to_string())?) {
            return Err("Plugin junction escapes source".into());
        }
        let dest = target.join(entry.file_name());
        if meta.is_dir() {
            copy_tree(&entry.path(), &dest, budget, depth + 1)?;
        } else if meta.is_file() {
            *budget = budget
                .checked_sub(meta.len().max(1))
                .ok_or("Plugin exceeds 100 MiB import limit")?;
            fs::copy(entry.path(), dest).map_err(|e| e.to_string())?;
        } else {
            return Err("Unsupported package file type".into());
        }
    }
    Ok(())
}

/// Normalize only the owned copy. Keep catalog overlays out of the runtime loader.
fn prepare_marketplace_manifest(root: &Path, entry: &Value) -> Result<(), String> {
    let file = root.join(".claude-plugin/plugin.json");
    let exists = file.exists();
    let mut manifest = if exists {
        read_json(&contained(root, ".claude-plugin/plugin.json")?)?
    } else {
        entry.clone()
    };
    if !manifest.is_object() {
        return Err("plugin.json must be an object".into());
    }
    let components = [
        "commands",
        "agents",
        "skills",
        "hooks",
        "outputStyles",
        "themes",
    ];
    if exists && entry["strict"] == false && components.iter().any(|k| entry.get(k).is_some()) {
        return Err("strict:false 条目与 plugin.json 同时声明组件，无法安装".into());
    }
    if entry.get("hooks").is_some_and(|v| !v.is_object()) {
        return Err("市场条目的 hooks 必须为内联事件对象".into());
    }
    // Component paths supplement conventional folders, except selective root skills.
    for key in ["commands", "agents", "skills", "outputStyles", "themes"] {
        let mut values = Vec::new();
        let selective = key == "skills" && entry["source"] == "." && entry.get(key).is_some();
        if root.join(key).exists() && !selective {
            values.push(Value::String(format!("./{key}")));
        }
        for value in [manifest.get(key), exists.then(|| entry.get(key)).flatten()]
            .into_iter()
            .flatten()
        {
            let items = value
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![value.clone()]);
            for item in items {
                if !item.is_string() {
                    return Err(format!("{key} 必须为路径或路径数组"));
                }
                if !values.contains(&item) {
                    values.push(item);
                }
            }
        }
        if !values.is_empty() {
            manifest[key] = Value::Array(values);
        }
    }
    if let Some(overlay) = entry.get("hooks") {
        let mut events = serde_json::Map::new();
        let mut base = configs(root, None, "hooks/hooks.json")?;
        if exists {
            base.extend(configs(root, manifest.get("hooks"), "hooks/hooks.json")?);
        }
        for config in base {
            let map = config
                .get("hooks")
                .unwrap_or(&config)
                .as_object()
                .ok_or("hooks must be an event map")?;
            events.extend(map.clone());
        }
        events.extend(
            overlay
                .get("hooks")
                .unwrap_or(overlay)
                .as_object()
                .ok_or("hooks must be an event map")?
                .clone(),
        );
        fs::create_dir_all(root.join("hooks")).map_err(|e| e.to_string())?;
        write_json(&root.join("hooks/hooks.json"), &json!({"hooks":events}))?;
        manifest["hooks"] = json!("./hooks/hooks.json");
    }
    for key in [
        "name",
        "description",
        "displayName",
        "author",
        "homepage",
        "repository",
        "license",
        "keywords",
    ] {
        if let Some(value) = entry.get(key) {
            manifest[key] = value.clone();
        }
    }
    if exists && manifest.get("version").is_none() {
        if let Some(value) = entry.get("version") {
            manifest["version"] = value.clone();
        }
    }
    // Dependency declarations must remain visible to the unsupported-capability diagnostic.
    if let Some(value) = entry.get("dependencies") {
        manifest["dependencies"] = value.clone();
    }
    if let Some(map) = manifest.as_object_mut() {
        for key in [
            "source",
            "strict",
            "category",
            "tags",
            "defaultEnabled",
            "relevance",
            "headers",
            "headersHelper",
        ] {
            map.remove(key);
        }
    } else {
        return Err("plugin.json must be an object".into());
    }
    fs::create_dir_all(file.parent().unwrap()).map_err(|e| e.to_string())?;
    write_json(&file, &manifest)
}

pub(super) fn https_url(source: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(source).map_err(|_| "请输入有效的 HTTPS 地址")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("来源须使用 HTTPS，且不能包含用户名或密码".into());
    }
    Ok(url)
}

async fn git_command(directory: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(["-c", "core.hooksPath=", "-c", "protocol.file.allow=never"]);
    if let Some(dir) = directory {
        cmd.arg("-C").arg(dir);
    }
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        use crate::proc::NoConsoleWindow;
        cmd.no_console_window();
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.output())
        .await
        .map_err(|_| "Git 操作超时")?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("Git: {}", String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

pub(super) async fn clone_repository(
    source: &str,
    target: &Path,
    revision: Option<&str>,
) -> Result<String, String> {
    https_url(source)?;
    if revision
        .is_some_and(|r| r.is_empty() || r.starts_with('-') || r.chars().any(char::is_control))
    {
        return Err("无效的 Git revision".into());
    }
    git_command(
        None,
        &[
            "clone",
            "--depth",
            "1",
            "--no-checkout",
            "--",
            source,
            &target.to_string_lossy(),
        ],
    )
    .await?;
    let head = if let Some(revision) = revision {
        git_command(
            Some(target),
            &["fetch", "--depth", "1", "--", "origin", revision],
        )
        .await?;
        "FETCH_HEAD"
    } else {
        "HEAD"
    };
    git_command(Some(target), &["checkout", "--detach", head]).await?;
    git_command(Some(target), &["rev-parse", "HEAD"]).await
}
#[tauri::command]
pub async fn plugin_packages_list() -> Result<Vec<Package>, String> {
    tauri::async_runtime::spawn_blocking(move || list())
        .await
        .map_err(|error| format!("plugin_packages_list failed: {error}"))?
}

#[tauri::command]
pub async fn plugin_packages_import(
    source: String,
    subdirectory: Option<String>,
) -> Result<Package, String> {
    import_package(source, subdirectory, None, None).await
}

pub mod details;

async fn stage_content(
    source: &str,
    subdirectory: Option<&str>,
    git_revision: Option<&str>,
    staging: &Path,
) -> Result<Option<String>, String> {
    let local = PathBuf::from(source.trim());
    let mut revision = None;
    if local.is_dir() {
        let local = fs::canonicalize(&local).map_err(|e| e.to_string())?;
        if fs::canonicalize(packages_root()?)
            .map_err(|e| e.to_string())?
            .starts_with(&local)
        {
            return Err("Source cannot contain Kivio's plugin storage".into());
        }
        let selected = match subdirectory.filter(|s| !s.trim().is_empty()) {
            Some(relative) => contained(&local, relative)?,
            None => local,
        };
        copy_tree(
            &selected,
            &staging.join("content"),
            &mut (100 * 1024 * 1024),
            0,
        )?;
    } else {
        let clone = staging.join("clone");
        revision = Some(clone_repository(source.trim(), &clone, git_revision).await?);
        let selected = match subdirectory.filter(|s| !s.trim().is_empty()) {
            Some(relative) => contained(&clone, relative)?,
            None => clone.clone(),
        };
        copy_tree(
            &selected,
            &staging.join("content"),
            &mut (100 * 1024 * 1024),
            0,
        )?;
        fs::remove_dir_all(&clone).map_err(|e| e.to_string())?;
    }
    Ok(revision)
}

pub(super) async fn import_package(
    source: String,
    subdirectory: Option<String>,
    git_revision: Option<String>,
    marketplace: Option<(Value, PackageMarketplace)>,
) -> Result<Package, String> {
    let root = packages_root()?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let staging = root.join(format!(".staging-{id}"));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = async {
        let revision = stage_content(
            &source,
            subdirectory.as_deref(),
            git_revision.as_deref(),
            &staging,
        )
        .await?;
        if let Some((entry, _)) = &marketplace {
            prepare_marketplace_manifest(&staging.join("content"), entry)?;
        }
        let source = match subdirectory.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(dir) => format!("{} :: {dir}", source.trim()),
            None => source.trim().into(),
        };
        let package = Package {
            id: id.clone(),
            name: String::new(),
            description: String::new(),
            version: None,
            format: String::new(),
            source,
            revision,
            enabled: false,
            components: BTreeMap::new(),
            diagnostics: vec![],
            marketplace: marketplace.map(|(_, origin)| origin),
        };
        let resolved = resolve(&staging.join("content"), package, &staging.join("data"))?;
        write_json(&staging.join("record.json"), &resolved.package)?;
        fs::rename(&staging, root.join(&id)).map_err(|e| e.to_string())?;
        Ok(resolved.package)
    }
    .await;
    if result.is_err() && staging.is_dir() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

#[tauri::command]
pub async fn plugin_packages_set_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Package, String> {
    let _guard = mutation_lock().lock().await;
    set_enabled(&app, &state, &id, enabled).await
}
fn mutation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(Default::default)
}
async fn set_enabled(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    enabled: bool,
) -> Result<Package, String> {
    // Broken manifests must still be removable and disableable.
    let mut resolved = match load(id) {
        Ok(resolved) => resolved,
        Err(error) if !enabled => {
            let package: Package =
                serde_json::from_value(read_json(&package_dir(id)?.join("record.json"))?)
                    .map_err(|e| e.to_string())?;
            Resolved {
                package,
                root: package_dir(id)?.join("content"),
                skills: vec![],
                commands: vec![],
                agents: vec![],
                hooks: vec![],
                servers: vec![],
            }
            .with_diagnostic(error)
        }
        Err(error) => return Err(error),
    };
    if enabled && !resolved.package.diagnostics.is_empty() {
        return Err(format!(
            "Cannot enable plugin with unsupported or invalid components: {}",
            resolved.package.diagnostics.join("; ")
        ));
    }
    if enabled
        && active()
            .iter()
            .any(|p| p.package.id != id && p.package.name == resolved.package.name)
    {
        return Err(format!(
            "Another version/source of {} is enabled; disable it first to avoid command conflicts",
            resolved.package.name
        ));
    }
    let prefix = format!("plugin-package-{id}-");
    // Persist package metadata first, then publish settings through the unified settings commit.
    // On settings failure the metadata write is rolled back to keep the two stores aligned.
    let old_package = resolved.package.clone();
    resolved.package.enabled = enabled;
    write_json(&package_dir(&id)?.join("record.json"), &resolved.package)?;
    let disconnect = std::cell::RefCell::new(Vec::new());
    let settings_result = crate::settings::update_settings(&app, &state, |settings| {
        *disconnect.borrow_mut() = settings
            .chat_tools
            .servers
            .iter()
            .filter(|s| s.id.starts_with(&prefix))
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        settings
            .chat_tools
            .servers
            .retain(|s| !s.id.starts_with(&prefix));
        if enabled {
            settings.chat_tools.enabled = true;
            settings.chat_tools.native_tools.skill_runtime = true;
            settings.chat_tools.servers.extend(resolved.servers.clone());
        }
        Ok(())
    });
    if let Err(error) = settings_result {
        let _ = write_json(&package_dir(&id)?.join("record.json"), &old_package);
        return Err(error.into());
    }
    state.chat_runtime().invalidate_resources();
    // IDs are deterministic; disconnect both enabled and disabled snapshots.
    for server in disconnect.into_inner() {
        state.mcp_disconnect_server(&server).await;
    }
    Ok(resolved.package)
}

#[tauri::command]
pub async fn plugin_packages_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let _guard = mutation_lock().lock().await;
    set_enabled(&app, &state, &id, false).await?;
    let dir = package_dir(&id)?;
    let root = fs::canonicalize(packages_root()?).map_err(|e| e.to_string())?;
    let target = fs::canonicalize(&dir).map_err(|e| e.to_string())?;
    if target.parent() != Some(root.as_path()) {
        return Err("Invalid managed plugin directory".into());
    }
    fs::remove_dir_all(target).map_err(|e| e.to_string())
}

impl Resolved {
    fn with_diagnostic(mut self, error: String) -> Self {
        self.package.diagnostics = vec![error];
        self
    }
    pub fn expand_body(&self, body: &str) -> String {
        expand_body_at(&self.root, body)
    }
}

/// Resolve a known package skill without rediscovering any package components.
pub(crate) fn expand_skill_body(id: &str, location: &Path, body: &str) -> Result<String, String> {
    super::package_content::expand_skill_body(&packages_root()?, id, location, body)
}
fn hooks_path() -> Result<PathBuf, String> {
    super::plugins_root()
        .map(|p| p.join("workflow-hooks.json"))
        .ok_or("Application data unavailable".into())
}
#[tauri::command]
pub fn workflow_hooks_get() -> Result<Value, String> {
    let path = hooks_path()?;
    if path.exists() {
        read_json(&path)
    } else {
        Ok(json!({"enabled": false, "hooks": {}}))
    }
}
#[tauri::command]
pub fn workflow_hooks_save(config: Value) -> Result<(), String> {
    let path = hooks_path()?;
    let root = path.parent().ok_or("No hooks root")?;
    workflow_hooks::parse(
        config.get("hooks").ok_or("hooks object required")?,
        "user",
        root,
        &root.join("hook-data"),
    )?;
    if config.get("enabled").and_then(Value::as_bool).is_none() {
        return Err("enabled boolean required".into());
    }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    write_json(&path, &config)
}
pub(crate) fn hook_runtime(
    plugin_hooks: &[Hook],
    cwd: PathBuf,
    agent_type: String,
    prompt: Option<String>,
) -> workflow_hooks::Runtime {
    let mut hooks = plugin_hooks.to_vec();
    if let Ok(value) = workflow_hooks_get() {
        if value.get("enabled").and_then(Value::as_bool) == Some(true) {
            if let Ok(path) = hooks_path() {
                if let Some(root) = path.parent() {
                    if let Ok(user) =
                        workflow_hooks::parse(&value, "user", root, &root.join("hook-data"))
                    {
                        hooks.extend(user);
                    }
                }
            }
        }
    }
    workflow_hooks::Runtime {
        hooks,
        cwd,
        agent_type,
        prompt,
    }
}

/// Adapt YAML metadata without relocating component files or their relative resources.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_resources_are_not_discovered_as_plugin_components() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".kivio-plugin")).unwrap();
        fs::create_dir_all(root.join("skills/demo/resources/example")).unwrap();
        write_json(
            &root.join(".kivio-plugin/plugin.json"),
            &json!({
                "schemaVersion": 1, "name": "demo", "version": "1"
            }),
        )
        .unwrap();
        fs::write(
            root.join("skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: Demo\n---\nUse resources as needed.",
        )
        .unwrap();
        // An embedded example is data, even if it resembles an invalid skill.
        fs::write(
            root.join("skills/demo/resources/example/SKILL.md"),
            "---\ninvalid yaml: [\n---\nExample",
        )
        .unwrap();
        let resolved = resolve(root, fixture(), root).unwrap();
        assert!(
            resolved.package.diagnostics.is_empty(),
            "{:?}",
            resolved.package.diagnostics
        );
        assert_eq!(resolved.package.components["skills"], 1);
    }

    #[test]
    fn native_manifest_rejects_unknown_versions_fields_and_bad_paths() {
        let base = json!({"schemaVersion":1,"name":"native-example","version":"1.0.0"});
        validate_native_manifest(&base).unwrap();
        for (key, value) in [
            ("schemaVersion", json!(2)),
            ("schemaVersion", json!("1")),
            ("name", json!("Bad_Name")),
            ("version", json!(" ")),
            ("metadata", json!(false)),
            ("skills", json!("../escape")),
            ("hooks", json!({})),
            ("commands", json!(["./ok", 42])),
            ("mcpServers", json!("./../escape")),
            ("hook", json!("./hooks.json")),
        ] {
            let mut invalid = base.clone();
            invalid[key] = value;
            assert!(
                validate_native_manifest(&invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }

    #[tokio::test]
    async fn native_fixture_has_priority_and_hooks_use_native_tool_input() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests/fixtures/plugins/kivio-example");
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        copy_tree(&source, &content, &mut (1024 * 1024), 0).unwrap();
        fs::create_dir(content.join(".codex-plugin")).unwrap();
        fs::write(
            content.join(".codex-plugin/plugin.json"),
            "invalid ignored alternative",
        )
        .unwrap();
        let mut resolved = resolve(&content, fixture(), dir.path()).unwrap();
        assert_eq!(resolved.package.format, "kivio");
        assert!(
            resolved.package.diagnostics.is_empty(),
            "{:?}",
            resolved.package.diagnostics
        );
        assert_eq!(resolved.package.components["commands"], 1);
        assert_eq!(resolved.package.components["agents"], 1);
        for hook in &mut resolved.hooks {
            assert!(!hook.foreign_tool_names);
            hook.owner = "fixture".into();
        }
        let runtime = workflow_hooks::Runtime {
            hooks: resolved.hooks,
            cwd: content,
            ..Default::default()
        };
        let mut input = runtime.input("PreToolUse", "native-session");
        input["tool_name"] = json!("write");
        input["tool_input"] = json!({"path":"protected/example.txt", "content":"hello"});
        assert!(runtime
            .run("PreToolUse", input.clone())
            .await
            .unwrap()
            .denied
            .is_some());
        input["tool_input"]["path"] = json!("ordinary/example.txt");
        assert!(runtime
            .run("PreToolUse", input)
            .await
            .unwrap()
            .denied
            .is_none());
    }

    #[test]
    fn native_empty_components_override_defaults_and_codex_mcp_wrapper_loads() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".kivio-plugin")).unwrap();
        fs::create_dir(dir.path().join("skills")).unwrap();
        fs::write(
            dir.path().join(".kivio-plugin/plugin.json"),
            r#"{"schemaVersion":1,"name":"example","version":"1.0.0","skills":[],"hooks":[]}"#,
        )
        .unwrap();
        fs::create_dir(dir.path().join("hooks")).unwrap();
        fs::write(
            dir.path().join("hooks/hooks.json"),
            "invalid disabled hooks",
        )
        .unwrap();
        fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcp_servers":{"helper":{"command":"node"}}}"#,
        )
        .unwrap();
        let resolved = resolve(dir.path(), fixture(), dir.path()).unwrap();
        assert!(resolved.skills.is_empty());
        assert!(resolved.hooks.is_empty());
        assert_eq!(resolved.servers.len(), 1);
        fs::write(
            dir.path().join(".kivio-plugin/plugin.json"),
            r#"{"schemaVersion":2}"#,
        )
        .unwrap();
        assert!(resolve(dir.path(), fixture(), dir.path()).is_err());
    }

    #[tokio::test]
    async fn portable_fixture_loads_commands_skills_and_executes_its_hook() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests/fixtures/plugins/portable-example");
        let dir = tempfile::Builder::new()
            .prefix("plugin spaces ")
            .tempdir()
            .unwrap();
        let content = dir.path().join("content");
        copy_tree(&source, &content, &mut (1024 * 1024), 0).unwrap();
        let mut resolved = resolve(&content, fixture(), &dir.path().join("data")).unwrap();
        assert!(
            resolved.package.diagnostics.is_empty(),
            "{:?}",
            resolved.package.diagnostics
        );
        assert_eq!(resolved.skills.len(), 1);
        let command_path = resolved.commands.last().unwrap();
        let command = component_markdown(&command_path).unwrap();
        let detail = crate::skills::parse_skill_markdown(&command, "plugin", None, vec![]).unwrap();
        assert_eq!(detail.meta.name, "check");
        assert!(detail.meta.description.contains("portable plugin command"));
        assert!(detail.body.contains("$ARGUMENTS"));
        // Resolve without registering in the real application data directory.
        for hook in &mut resolved.hooks {
            hook.owner = "fixture".into();
        }
        let runtime = workflow_hooks::Runtime {
            hooks: resolved.hooks,
            cwd: content,
            ..Default::default()
        };
        let out = runtime
            .run(
                "SessionStart",
                runtime.input("SessionStart", "fixture-session"),
            )
            .await
            .unwrap();
        assert_eq!(
            out.context,
            ["Portable example loaded for session fixture-session."]
        );
    }

    #[test]
    fn multiple_mcp_servers_have_distinct_ids_and_relative_environment() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".codex-plugin")).unwrap();
        fs::write(dir.path().join(".codex-plugin/plugin.json"), r#"{"name":"multi","mcpServers":{"one":{"command":"node","args":["${PLUGIN_ROOT}/one.cjs"]},"two":{"type":"http","url":"https://example.com/mcp"}}}"#).unwrap();
        let resolved = resolve(dir.path(), fixture(), dir.path()).unwrap();
        assert_eq!(resolved.servers.len(), 2);
        assert_ne!(resolved.servers[0].id, resolved.servers[1].id);
        assert!(resolved
            .servers
            .iter()
            .any(|s| s.transport == "streamable_http"));
        assert!(resolved
            .servers
            .iter()
            .any(|s| s.args.first().is_some_and(|a| a
                .contains(&dir.path().display().to_string())
                && !a.contains("${"))));
    }
    fn fixture() -> Package {
        Package {
            id: uuid::Uuid::new_v4().to_string(),
            name: "".into(),
            description: "".into(),
            format: "".into(),
            source: "fixture".into(),
            revision: None,
            enabled: false,
            version: None,
            components: BTreeMap::new(),
            diagnostics: vec![],
            marketplace: None,
        }
    }
    #[test]
    fn marketplace_overlays_preserve_manifest_version_and_replace_hooks_per_event() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        fs::create_dir_all(root.join("commands")).unwrap();
        fs::create_dir_all(root.join("extra")).unwrap();
        fs::create_dir_all(root.join("hooks")).unwrap();
        fs::write(root.join("commands/default.md"), "Default").unwrap();
        fs::write(root.join("extra/added.md"), "Added").unwrap();
        write_json(&root.join("hooks/hooks.json"), &json!({"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo old"}]}]}})).unwrap();
        write_json(
            &root.join(".claude-plugin/plugin.json"),
            &json!({"name":"original","version":"2"}),
        )
        .unwrap();
        let entry = json!({"name":"demo","source":"./demo","version":"1","commands":"./extra","mcpServers":{"ignored":{"command":"node"}},"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo new"}]}]}});
        prepare_marketplace_manifest(root, &entry).unwrap();
        let resolved = resolve(root, fixture(), root).unwrap();
        assert_eq!(resolved.package.version.as_deref(), Some("2"));
        assert_eq!(resolved.package.name, "demo");
        assert_eq!(resolved.package.components["commands"], 2);
        assert_eq!(resolved.hooks.len(), 1);
        assert!(read_json(&root.join("hooks/hooks.json"))
            .unwrap()
            .to_string()
            .contains("echo new"));
        assert!(resolved.servers.is_empty());
    }
    #[test]
    fn marketplace_strict_conflicts_fail_and_missing_manifest_works_in_either_mode() {
        for strict in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            prepare_marketplace_manifest(
                dir.path(),
                &json!({"name":"demo","source":".","strict":strict,"lspServers":{}}),
            )
            .unwrap();
            let resolved = resolve(dir.path(), fixture(), dir.path()).unwrap();
            assert!(resolved
                .package
                .diagnostics
                .iter()
                .any(|s| s.contains("lspServers")));
            assert!(prepare_marketplace_manifest(
                dir.path(),
                &json!({"name":"demo","strict":false,"skills":[]})
            )
            .is_err());
        }
    }
    #[test]
    fn manifests_keep_custom_paths_and_do_not_double_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".codex-plugin")).unwrap();
        fs::create_dir(dir.path().join(".claude-plugin")).unwrap();
        fs::create_dir(dir.path().join("custom")).unwrap();
        fs::write(dir.path().join("custom/SKILL.md"), "Custom skill").unwrap();
        fs::write(dir.path().join(".codex-plugin/plugin.json"), r#"{"name":"example","skills":"./custom","hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hello"}]}]}}"#).unwrap();
        fs::write(
            dir.path().join(".claude-plugin/plugin.json"),
            "invalid ignored alternate manifest",
        )
        .unwrap();
        let resolved = resolve(dir.path(), fixture(), dir.path()).unwrap();
        assert_eq!(resolved.package.format, "codex");
        assert_eq!(resolved.hooks.len(), 1);
        assert_eq!(resolved.skills.len(), 1);
        assert!(resolved.skills[0].ends_with("custom/SKILL.md"));
    }
    #[test]
    fn directory_components_are_counted_and_toml_agents_are_diagnosed() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".codex-plugin")).unwrap();
        fs::create_dir(dir.path().join("commands")).unwrap();
        fs::create_dir_all(dir.path().join("agents/nested")).unwrap();
        fs::write(
            dir.path().join(".codex-plugin/plugin.json"),
            r#"{"name":"example"}"#,
        )
        .unwrap();
        fs::write(dir.path().join("commands/one.md"), "First command").unwrap();
        fs::write(dir.path().join("commands/two.md"), "Second command").unwrap();
        fs::write(
            dir.path().join("agents/nested/reviewer.toml"),
            "name = 'reviewer'",
        )
        .unwrap();
        let resolved = resolve(dir.path(), fixture(), dir.path()).unwrap();
        assert_eq!(resolved.package.components["commands"], 2);
        assert_eq!(resolved.package.components["agents"], 0);
        assert!(resolved
            .package
            .diagnostics
            .iter()
            .any(|d| d.contains("reviewer.toml")));
    }
    #[test]
    fn path_escape_and_unknown_capabilities_are_diagnosed() {
        let dir = tempfile::tempdir().unwrap();
        assert!(contained(dir.path(), "../outside").is_err());
        fs::create_dir(dir.path().join(".claude-plugin")).unwrap();
        fs::write(
            dir.path().join(".claude-plugin/plugin.json"),
            r#"{"name":"example","lspServers":{}}"#,
        )
        .unwrap();
        assert!(!resolve(dir.path(), fixture(), dir.path())
            .unwrap()
            .package
            .diagnostics
            .is_empty());
    }
}

use super::package_content::{read_json, expand_body_at};
pub use super::package_content::component_markdown;

pub use super::package_catalog::{Package, PackageMarketplace, Resolved, contained, resolve, markdown_files};
use super::package_catalog::*;
