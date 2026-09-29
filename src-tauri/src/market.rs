//! 插件市场：内置清单（`resources/plugins/catalog.json`）里的插件由市场直接安装。
//!
//! 每个插件由这些组件组成，全部写进 `~/.kivio/skills`：
//! - `<id>-setup`：随应用发布的环境检查 Skill（带 `kivio-market-managed: true` 标记）；
//! - `entry`：可选，随应用发布的主 Skill；
//! - `skills`：可选，从 GitHub 固定 revision 下载的官方 Skill（带 `.kivio-market-owner.json`）；
//! - `command`：可选，`resources/plugins/market-companions/<id>` 里的 Kivio 插件包，提供检查命令；
//! - `presetPluginId`：可选，联动 `crate::plugins` 内置目录插件的启用开关。
//!
//! 安装状态记在 `{app_data}/market/<id>-state.json`；“加载”开关通过
//! `chatTools.disabledSkillIds` 让这些 Skill 在对话中可见或隐藏。
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};

/// 市场状态变化通知；前端据此刷新本地安装状态。
const CHANGED_EVENT: &str = "kivio-market-changed";

#[derive(Clone)]
struct BuiltIn {
    id: String,
    name: String,
    skill_id: String,
    summary: String,
    welcome: String,
    input_hint: String,
    start_prompt: String,
    setup: String,
    entry: Option<String>,
    command: Option<String>,
    icon: String,
    required_files: Vec<String>,
    repository: String,
    revision: String,
    skills: Vec<String>,
    category_ids: Vec<String>,
    unpack: String,
    skip: Vec<String>,
    preset_plugin_id: Option<String>,
}

struct Catalog {
    categories: Vec<(String, String)>,
    plugins: Vec<BuiltIn>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogFile {
    categories: Vec<CatalogCategory>,
    plugins: Vec<CatalogPlugin>,
}

#[derive(Deserialize)]
struct CatalogCategory {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogPlugin {
    id: String,
    name: String,
    skill_id: String,
    summary: String,
    welcome: String,
    input_hint: String,
    start_prompt: String,
    setup: String,
    #[serde(default)]
    entry: Option<String>,
    #[serde(default)]
    command: Option<String>,
    icon: String,
    #[serde(default)]
    required_files: Vec<String>,
    repository: String,
    revision: String,
    #[serde(default)]
    skills: Vec<String>,
    category_ids: Vec<String>,
    #[serde(default = "default_unpack")]
    unpack: String,
    #[serde(default)]
    skip: Vec<String>,
    #[serde(default)]
    preset_plugin_id: Option<String>,
}

fn default_unpack() -> String {
    "skills".into()
}

fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.split('-').all(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn catalog_text(dir: &Path, relative: &str) -> Result<String, String> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative
            .split(['/', '\\'])
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("目录文件路径无效：{relative}"));
    }
    fs::read_to_string(dir.join(relative)).map_err(|e| format!("无法读取 {relative}：{e}"))
}

fn load_catalog_from(dir: &Path) -> Result<Catalog, String> {
    let file: CatalogFile = serde_json::from_str(&catalog_text(dir, "catalog.json")?)
        .map_err(|e| format!("插件目录无效：{e}"))?;
    let mut plugins = Vec::new();
    for plugin in file.plugins {
        let needs_download = !plugin.skills.is_empty() || plugin.unpack == "root";
        if !id_ok(&plugin.id)
            || (plugin.unpack != "skills" && plugin.unpack != "root")
            || (needs_download && plugin.repository.is_empty())
            || plugin
                .preset_plugin_id
                .as_deref()
                .is_some_and(|id| id != plugin.id || crate::plugins::catalog_plugin(id).is_none())
        {
            return Err(format!("插件目录条目无效：{}", plugin.id));
        }
        plugins.push(BuiltIn {
            setup: catalog_text(dir, &plugin.setup)?,
            entry: plugin
                .entry
                .as_deref()
                .map(|path| catalog_text(dir, path))
                .transpose()?,
            icon: catalog_text(dir, &plugin.icon)?,
            id: plugin.id,
            name: plugin.name,
            skill_id: plugin.skill_id,
            summary: plugin.summary,
            welcome: plugin.welcome,
            input_hint: plugin.input_hint,
            start_prompt: plugin.start_prompt,
            command: plugin.command,
            required_files: plugin.required_files,
            repository: plugin.repository,
            revision: plugin.revision,
            skills: plugin.skills,
            category_ids: plugin.category_ids,
            unpack: plugin.unpack,
            skip: plugin.skip,
            preset_plugin_id: plugin.preset_plugin_id,
        });
    }
    Ok(Catalog {
        categories: file
            .categories
            .into_iter()
            .map(|category| (category.id, category.name))
            .collect(),
        plugins,
    })
}

fn source_resources_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/plugins")
}

/// 打包后的资源目录优先；开发模式回退到源码树。
fn resources_dir(app: &AppHandle) -> PathBuf {
    app.path()
        .resource_dir()
        .ok()
        .map(|dir| dir.join("plugins"))
        .filter(|dir| dir.join("catalog.json").is_file())
        .unwrap_or_else(source_resources_dir)
}

fn load_market_catalog(app: &AppHandle) -> Result<Catalog, String> {
    load_catalog_from(&resources_dir(app))
}

// ---------------------------------------------------------------------------
// 安装状态
// ---------------------------------------------------------------------------

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltInState {
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    plugin_id: Option<String>,
    #[serde(default)]
    owned_skills: Vec<String>,
}

fn root() -> Result<PathBuf, String> {
    crate::app_data::app_data_dir()
        .map(|p| p.join("market"))
        .ok_or_else(|| "应用数据目录不可用".into())
}

fn built_in_state_path(item: &BuiltIn) -> Result<PathBuf, String> {
    Ok(root()?.join(format!("{}-state.json", item.id)))
}

fn built_in_state(item: &BuiltIn) -> BuiltInState {
    built_in_state_path(item)
        .ok()
        .and_then(|p| fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_built_in_state(item: &BuiltIn, state: &BuiltInState) -> Result<(), String> {
    let path = built_in_state_path(item)?;
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    crate::chat::storage::atomic_write(&path, &text, "market state")
}

fn skills_root() -> Result<PathBuf, String> {
    crate::skills::kivio_skills_dir().ok_or_else(|| "用户目录不可用".into())
}

fn built_in_ready(item: &BuiltIn, state: &BuiltInState) -> bool {
    let skills_ready = skills_root().is_ok_and(|root| built_in_ready_at(item, state, &root));
    let command_ready = if item.command.is_some() {
        state
            .plugin_id
            .as_deref()
            .and_then(|id| market_companion_package(item, id).ok().flatten())
            .is_some_and(|package| {
                package.enabled
                    && package.diagnostics.is_empty()
                    && package
                        .components
                        .get("commands")
                        .copied()
                        .unwrap_or_default()
                        > 0
            })
    } else {
        state.plugin_id.is_none()
    };
    skills_ready && command_ready
}

fn built_in_ready_at(item: &BuiltIn, state: &BuiltInState, root: &Path) -> bool {
    state.revision.as_deref() == Some(item.revision.as_str())
        && market_skill_installed_at(&root.join(built_in_setup_id(item)))
        && item.entry.as_deref().is_none_or(|entry| {
            let dir = root.join(&item.skill_id);
            market_skill_installed_at(&dir)
                && fs::read_to_string(dir.join("SKILL.md")).is_ok_and(|content| content == entry)
        })
        && item
            .skills
            .iter()
            .all(|skill| root.join(skill).join("SKILL.md").is_file())
        && built_in_skill_installed_at(item, root)
}

fn built_in_skill_installed_at(item: &BuiltIn, root: &Path) -> bool {
    let skill = root.join(&item.skill_id);
    skill.join("SKILL.md").is_file()
        && item
            .required_files
            .iter()
            .all(|file| skill.join(file).is_file())
}

fn built_in_setup_id(item: &BuiltIn) -> String {
    format!("{}-setup", item.id)
}

/// 市场管理的单文件 Skill（setup / entry）：只认带标记且非符号链接的文件，绝不覆盖用户自己的 Skill。
fn market_skill_installed_at(dir: &Path) -> bool {
    if is_symlink(dir) || is_symlink(&dir.join("SKILL.md")) {
        return false;
    }
    fs::read_to_string(dir.join("SKILL.md"))
        .ok()
        .is_some_and(|text| text.contains("\nkivio-market-managed: true\n"))
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

fn install_market_skill(dir: &Path, content: &str) -> Result<(), String> {
    let file = dir.join("SKILL.md");
    if is_symlink(dir) || is_symlink(&file) {
        return Err(format!("{} 包含符号链接，请先检查该目录", dir.display()));
    }
    if file.is_file() {
        if !market_skill_installed_at(dir) {
            return Err(format!(
                "{} 已存在其他 Skill，请先检查该目录",
                dir.display()
            ));
        }
        return fs::write(file, content).map_err(|e| e.to_string());
    }
    if dir.exists() {
        return Err(format!("{} 已存在，请先检查该目录", dir.display()));
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::write(file, content).map_err(|e| e.to_string())
}

fn remove_market_skill(dir: &Path) -> Result<(), String> {
    if !market_skill_installed_at(dir) {
        return Ok(());
    }
    fs::remove_file(dir.join("SKILL.md")).map_err(|e| e.to_string())?;
    if fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .next()
        .is_none()
    {
        fs::remove_dir(dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn built_in_skill_ids(item: &BuiltIn) -> Vec<String> {
    let mut ids = vec![built_in_setup_id(item)];
    if item.entry.is_some() {
        ids.push(item.skill_id.clone());
    }
    ids.extend(item.skills.iter().cloned());
    ids
}

// ---------------------------------------------------------------------------
// 快照（前端契约见 src/chat/market/types.ts）
// ---------------------------------------------------------------------------

fn icon_data_url(item: &BuiltIn) -> String {
    use base64::Engine;
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(item.icon.as_bytes())
    )
}

fn built_in_manifest(item: &BuiltIn) -> Value {
    let mut skill_ids = item.skills.clone();
    if item.entry.is_some() {
        skill_ids.insert(0, item.skill_id.clone());
    }
    json!({
        "id": item.id,
        "name": item.name,
        "summary": item.summary,
        "categoryIds": item.category_ids,
        "icon": icon_data_url(item),
        "welcome": item.welcome,
        "inputHint": item.input_hint,
        "startPrompt": item.start_prompt,
        "setupSkillId": built_in_setup_id(item),
        "mainSkillId": item.skill_id,
        "skillIds": skill_ids,
        "checkCommand": item.command,
        "repository": (!item.repository.is_empty()).then(|| item.repository.clone()),
        "revision": item.revision,
    })
}

/// 未安装且没有残留记录时返回 None；残留但组件不全时返回 `status: failed`，前端显示“重新配置”。
fn built_in_local(
    item: &BuiltIn,
    state: &BuiltInState,
    installed: bool,
    disabled_skill_ids: &[String],
) -> Option<Value> {
    if !installed && state.plugin_id.is_none() && state.revision.is_none() {
        return None;
    }
    let preset_active = item
        .preset_plugin_id
        .as_deref()
        .is_none_or(|id| !crate::plugins::is_installed(id) || crate::plugins::is_enabled(id));
    // The Skill Center and runtime use these same settings. Do not persist a
    // second enabled flag in market state that can disagree with them.
    let skills_enabled = built_in_skill_ids(item)
        .iter()
        .all(|id| !disabled_skill_ids.contains(id));
    Some(json!({
        "status": if installed { "ready" } else { "failed" },
        "enabled": installed && skills_enabled && preset_active,
        "error": if installed { None } else { Some("插件组件缺失或未启用") },
    }))
}

fn snapshot_of(catalog: &Catalog, disabled_skill_ids: &[String]) -> Value {
    json!({
        "categories": catalog.categories.iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect::<Vec<_>>(),
        "plugins": catalog.plugins.iter().map(|item| {
            let state = built_in_state(item);
            json!({
                "manifest": built_in_manifest(item),
                "local": built_in_local(item, &state, built_in_ready(item, &state), disabled_skill_ids),
            })
        }).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// 下载与解包
// ---------------------------------------------------------------------------

async fn download(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent("kivio-market/1")
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("无法连接插件源：{e}"))?
        .error_for_status()
        .map_err(|e| format!("插件源暂不可用：{e}"))?;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("下载内容超过限制".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > limit {
            return Err("下载内容超过限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn unpack_built_in_skills(item: &BuiltIn, bytes: Vec<u8>, stage: &Path) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut total = 0u64;
    let mut count = 0usize;
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(|e| e.to_string())?;
        let Some((_, archive_path)) = file.name().split_once('/') else {
            continue;
        };
        let selected = if item.unpack == "root" {
            if archive_path.is_empty()
                || item
                    .skip
                    .iter()
                    .any(|prefix| archive_path.starts_with(prefix))
            {
                continue;
            }
            format!("{}/{archive_path}", item.skill_id)
        } else {
            let Some(path) = archive_path.strip_prefix("skills/") else {
                continue;
            };
            let Some(skill) = path.split('/').next() else {
                continue;
            };
            if !item.skills.iter().any(|name| name == skill) {
                continue;
            }
            path.to_string()
        };
        if selected.starts_with('/')
            || selected.contains('\\')
            || selected.contains(':')
            || selected.split('/').any(|part| part == "." || part == "..")
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Skill 压缩包包含不安全路径".into());
        }
        let target = stage.join(&selected);
        if file.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        total = total.checked_add(file.size()).ok_or("Skill 内容过大")?;
        count += 1;
        if total > 30 * 1024 * 1024 || count > 2000 {
            return Err("Skill 内容超过限制".into());
        }
        fs::create_dir_all(target.parent().ok_or("无效 Skill 路径")?).map_err(|e| e.to_string())?;
        let mut output = fs::File::create(&target).map_err(|e| e.to_string())?;
        std::io::copy(&mut file, &mut output).map_err(|e| e.to_string())?;
    }
    for skill in &item.skills {
        if !stage.join(skill).join("SKILL.md").is_file() {
            return Err(format!("官方包缺少 {skill}/SKILL.md"));
        }
    }
    if item.unpack == "root" {
        let dir = stage.join(&item.skill_id);
        for relative in &item.required_files {
            if !dir.join(relative).is_file() {
                return Err(format!("官方包缺少 {relative}"));
            }
        }
    }
    Ok(())
}

fn market_owned_skill(item: &BuiltIn, state: &BuiltInState, root: &Path, skill: &str) -> bool {
    if !state.owned_skills.iter().any(|owned| owned == skill) {
        return false;
    }
    fs::read(root.join(skill).join(".kivio-market-owner.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value["id"] == item.id)
}

/// 返回（需下载的 Skill，其中需替换市场自有残缺副本的 Skill）。同名用户 Skill 一律拒绝覆盖。
fn built_in_skill_plan(
    item: &BuiltIn,
    state: &BuiltInState,
    root: &Path,
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut missing = Vec::new();
    let mut replace = Vec::new();
    for skill in &item.skills {
        let dir = root.join(skill);
        let metadata = match fs::symlink_metadata(&dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(skill.clone());
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("{} 已存在其他文件，请先检查", dir.display()));
        }
        let complete = dir.join("SKILL.md").is_file()
            && (skill != &item.skill_id
                || item
                    .required_files
                    .iter()
                    .all(|file| dir.join(file).is_file()));
        if complete {
            continue;
        }
        if !market_owned_skill(item, state, root, skill) {
            return Err(format!(
                "{} 已存在不完整的用户 Skill，请先检查",
                dir.display()
            ));
        }
        missing.push(skill.clone());
        replace.push(skill.clone());
    }
    Ok((missing, replace))
}

// ---------------------------------------------------------------------------
// 配套命令包（可选）
// ---------------------------------------------------------------------------

fn built_in_companion_source(app: &AppHandle, item: &BuiltIn) -> Result<PathBuf, String> {
    let source = resources_dir(app).join("market-companions").join(&item.id);
    if !source.join(".kivio-plugin/plugin.json").is_file() {
        return Err(format!("{} 的内置命令资源缺失", item.name));
    }
    Ok(source)
}

fn market_companion_package(
    item: &BuiltIn,
    id: &str,
) -> Result<Option<crate::plugins::packages::Package>, String> {
    let package = crate::plugins::packages::plugin_packages_list()?
        .into_iter()
        .find(|package| package.id == id);
    if let Some(package) = &package {
        let suffix = format!("/market-companions/{}", item.id);
        if package.format != "kivio" || !package.source.replace('\\', "/").ends_with(&suffix) {
            return Err("市场记录指向了其他插件，未执行操作".into());
        }
    }
    Ok(package)
}

// ---------------------------------------------------------------------------
// 操作
// ---------------------------------------------------------------------------

async fn set_built_in_enabled(
    app: &AppHandle,
    state: &AppState,
    item: &BuiltIn,
    enabled: bool,
) -> Result<(), String> {
    let saved = built_in_state(item);
    if !built_in_ready(item, &saved) {
        return Err(format!("{} 尚未安装", item.name));
    }
    if let Some(id) = item.preset_plugin_id.as_deref() {
        if !enabled || crate::plugins::is_installed(id) {
            crate::plugins::set_plugin_enabled(app, state, id, enabled).await?;
        }
    }
    let ids = built_in_skill_ids(item);
    crate::settings::update_settings(app, state, |next| {
        for id in &ids {
            next.chat_tools
                .disabled_skill_ids
                .retain(|skill| skill != id);
            if !enabled {
                next.chat_tools.disabled_skill_ids.push(id.clone());
            }
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    if let Some(id) = saved.plugin_id.clone() {
        if market_companion_package(item, &id)?.is_some() {
            crate::plugins::packages::plugin_packages_set_enabled(
                app.clone(),
                app.state::<AppState>(),
                id,
                enabled,
            )
            .await?;
        }
    }
    Ok(())
}

async fn install_plugin(app: &AppHandle, item: &BuiltIn) -> Result<(), String> {
    let previous = built_in_state(item);
    if built_in_ready(item, &previous) {
        return Ok(());
    }
    let skills_root = crate::skills::user_skills_dir(app)?;
    let (missing, replace) = built_in_skill_plan(item, &previous, &skills_root)?;
    let stage = root()?
        .join("staging")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let setup_dir = skills_root.join(built_in_setup_id(item));
    let entry_dir = skills_root.join(&item.skill_id);
    let had_setup = market_skill_installed_at(&setup_dir);
    let had_entry = item.entry.is_some() && market_skill_installed_at(&entry_dir);
    let mut moved = Vec::<String>::new();
    let mut backed_up = Vec::<String>::new();
    let mut new_plugin = None::<String>;
    let mut result: Result<(), String> = async {
        if !missing.is_empty() {
            let url = format!(
                "https://codeload.github.com/{}/zip/{}",
                item.repository, item.revision
            );
            let bytes = download(&url, 30 * 1024 * 1024).await?;
            unpack_built_in_skills(item, bytes, &stage)?;
            for skill in &missing {
                let from = stage.join(skill);
                let owner = json!({ "id": item.id, "revision": item.revision });
                fs::write(
                    from.join(".kivio-market-owner.json"),
                    serde_json::to_vec(&owner).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                let target = skills_root.join(skill);
                if replace.contains(skill) {
                    if fs::symlink_metadata(&target).is_err()
                        || !market_owned_skill(item, &previous, &skills_root, skill)
                    {
                        return Err(format!("{} 安装期间已发生变化", target.display()));
                    }
                    let backup = stage.join("backup").join(skill);
                    fs::create_dir_all(backup.parent().ok_or("无效备份路径")?)
                        .map_err(|e| e.to_string())?;
                    fs::rename(&target, backup).map_err(|e| e.to_string())?;
                    backed_up.push(skill.clone());
                } else if fs::symlink_metadata(&target).is_ok() {
                    return Err(format!("{} 安装期间已出现同名 Skill", target.display()));
                }
                fs::rename(&from, target).map_err(|e| e.to_string())?;
                moved.push(skill.clone());
            }
        }
        install_market_skill(&setup_dir, &item.setup)?;
        if let Some(content) = item.entry.as_deref() {
            install_market_skill(&entry_dir, content)?;
        }
        let plugin_id = if item.command.is_some() {
            let existing = previous
                .plugin_id
                .as_deref()
                .map(|id| market_companion_package(item, id).map(|p| p.map(|_| id.to_string())))
                .transpose()?
                .flatten();
            let id = match existing {
                Some(id) => id,
                None => {
                    let source = built_in_companion_source(app, item)?;
                    let package = crate::plugins::packages::plugin_packages_import(
                        source.display().to_string(),
                        None,
                    )
                    .await?;
                    new_plugin = Some(package.id.clone());
                    package.id
                }
            };
            if !crate::plugins::packages::owner_enabled(&id) {
                crate::plugins::packages::plugin_packages_set_enabled(
                    app.clone(),
                    app.state::<AppState>(),
                    id.clone(),
                    true,
                )
                .await?;
            }
            Some(id)
        } else {
            None
        };
        let mut owned_skills = previous.owned_skills.clone();
        for skill in &moved {
            if !owned_skills.contains(skill) {
                owned_skills.push(skill.clone());
            }
        }
        let state = BuiltInState {
            revision: Some(item.revision.clone()),
            plugin_id,
            owned_skills,
        };
        if !built_in_ready(item, &state) {
            return Err(format!("{} 的组件没有完成注册", item.name));
        }
        if let Some(id) = item.preset_plugin_id.as_deref() {
            if crate::plugins::is_installed(id) {
                crate::plugins::set_plugin_enabled(app, &app.state::<AppState>(), id, true).await?;
            }
        }
        // 重新安装等于重新加载：清掉可能残留的停用记录。
        let ids = built_in_skill_ids(item);
        crate::settings::update_settings(app, &app.state::<AppState>(), |next| {
            next.chat_tools
                .disabled_skill_ids
                .retain(|skill| !ids.contains(skill));
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        save_built_in_state(item, &state)
    }
    .await;
    let mut keep_stage = false;
    if result.is_err() {
        if let Some(id) = new_plugin {
            let _ = crate::plugins::packages::plugin_packages_remove(
                app.clone(),
                app.state::<AppState>(),
                id,
            )
            .await;
        }
        for skill in moved {
            if let Err(error) = fs::remove_dir_all(skills_root.join(&skill)) {
                keep_stage = true;
                result = Err(format!("安装失败，移除新 Skill {skill} 时失败：{error}"));
            }
        }
        for skill in backed_up {
            if let Err(error) =
                fs::rename(stage.join("backup").join(&skill), skills_root.join(&skill))
            {
                keep_stage = true;
                result = Err(format!(
                    "安装失败，原 Skill 备份保留在 {}：{error}",
                    stage.display()
                ));
            }
        }
        if !had_setup {
            let _ = remove_market_skill(&setup_dir);
        }
        if !had_entry && item.entry.is_some() {
            let _ = remove_market_skill(&entry_dir);
        }
    }
    if !keep_stage {
        let _ = fs::remove_dir_all(stage);
    }
    result
}

async fn uninstall_plugin(app: &AppHandle, item: &BuiltIn) -> Result<(), String> {
    let state = built_in_state(item);
    if state.revision.is_none() && state.plugin_id.is_none() {
        return Err(format!("{} 尚未安装", item.name));
    }
    if let Some(id) = item.preset_plugin_id.as_deref() {
        crate::plugins::set_plugin_enabled(app, &app.state::<AppState>(), id, false).await?;
    }
    if let Some(plugin_id) = state.plugin_id.as_ref() {
        if market_companion_package(item, plugin_id)?.is_some() {
            crate::plugins::packages::plugin_packages_remove(
                app.clone(),
                app.state::<AppState>(),
                plugin_id.clone(),
            )
            .await?;
        }
    }
    let skills_root = skills_root()?;
    for skill in &state.owned_skills {
        if !item.skills.iter().any(|name| name == skill) {
            continue;
        }
        let dir = skills_root.join(skill);
        if is_symlink(&dir) {
            continue;
        }
        if market_owned_skill(item, &state, &skills_root, skill) {
            fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }
    remove_market_skill(&skills_root.join(built_in_setup_id(item)))?;
    if item.entry.is_some() {
        remove_market_skill(&skills_root.join(&item.skill_id))?;
    }
    // 卸载后不留停用记录，下次安装直接可用。
    let ids = built_in_skill_ids(item);
    crate::settings::update_settings(app, &app.state::<AppState>(), |next| {
        next.chat_tools
            .disabled_skill_ids
            .retain(|skill| !ids.contains(skill));
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    save_built_in_state(item, &BuiltInState::default())
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

fn mutation_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(Default::default)
}

fn find<'a>(catalog: &'a Catalog, id: &str) -> Result<&'a BuiltIn, String> {
    catalog
        .plugins
        .iter()
        .find(|item| item.id == id)
        .ok_or_else(|| format!("找不到插件：{id}"))
}

/// 读取清单与本地安装状态。
#[tauri::command]
pub fn market_snapshot(app: AppHandle) -> Result<Value, String> {
    Ok(snapshot_of(
        &load_market_catalog(&app)?,
        &app.state::<AppState>()
            .settings_read()
            .chat_tools
            .disabled_skill_ids,
    ))
}

/// 安装或修复插件；已就绪时直接返回。完成后返回最新快照。
#[tauri::command]
pub async fn market_install(app: AppHandle, id: String) -> Result<Value, String> {
    let catalog = load_market_catalog(&app)?;
    let item = find(&catalog, &id)?;
    let _guard = mutation_lock().lock().await;
    let result = install_plugin(&app, item).await;
    let _ = app.emit(CHANGED_EVENT, ());
    result?;
    Ok(snapshot_of(
        &catalog,
        &app.state::<AppState>()
            .settings_read()
            .chat_tools
            .disabled_skill_ids,
    ))
}

#[tauri::command]
pub async fn market_uninstall(app: AppHandle, id: String) -> Result<Value, String> {
    let catalog = load_market_catalog(&app)?;
    let item = find(&catalog, &id)?;
    let _guard = mutation_lock().lock().await;
    uninstall_plugin(&app, item).await?;
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(snapshot_of(
        &catalog,
        &app.state::<AppState>()
            .settings_read()
            .chat_tools
            .disabled_skill_ids,
    ))
}

/// 切换“加载”：控制插件的 Skill 是否在对话中可用。
#[tauri::command]
pub async fn market_set_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<Value, String> {
    let catalog = load_market_catalog(&app)?;
    let item = find(&catalog, &id)?;
    let _guard = mutation_lock().lock().await;
    set_built_in_enabled(&app, &state, item, enabled).await?;
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(snapshot_of(
        &catalog,
        &state.settings_read().chat_tools.disabled_skill_ids,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        load_catalog_from(&source_resources_dir()).unwrap()
    }

    fn plugin(id: &str) -> BuiltIn {
        catalog()
            .plugins
            .into_iter()
            .find(|item| item.id == id)
            .unwrap_or_else(|| panic!("missing {id}"))
    }

    fn test_item(skills: &[&str], unpack: &str) -> BuiltIn {
        BuiltIn {
            skills: skills.iter().map(|s| s.to_string()).collect(),
            unpack: unpack.into(),
            skill_id: skills.first().copied().unwrap_or("demo").into(),
            entry: None,
            repository: "example/demo".into(),
            ..plugin("feishu-cli")
        }
    }

    #[test]
    fn bundled_catalog_loads_feishu_and_wecom() {
        let catalog = catalog();
        let ids: Vec<_> = catalog.plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["feishu-cli", "wecom-cli"]);
        for item in &catalog.plugins {
            assert!(
                item.setup.contains("\nkivio-market-managed: true\n"),
                "{}",
                item.id
            );
            let entry = item.entry.as_deref().expect("entry skill");
            assert!(
                entry.contains("\nkivio-market-managed: true\n"),
                "{}",
                item.id
            );
            assert!(!item.setup.contains("Dsivio"), "{}", item.id);
            assert!(item.icon.contains("<svg"), "{}", item.id);
            assert!(item.skills.is_empty() && item.command.is_none());
        }
        let snapshot = snapshot_of(&catalog, &[]);
        assert_eq!(snapshot["categories"][0]["id"], "productivity");
        let feishu = &snapshot["plugins"][0]["manifest"];
        assert_eq!(feishu["setupSkillId"], "feishu-cli-setup");
        assert_eq!(feishu["skillIds"], json!(["feishu-cli"]));
        assert!(feishu["icon"]
            .as_str()
            .unwrap()
            .starts_with("data:image/svg+xml;base64,"));
    }

    #[test]
    fn entry_plugin_is_ready_only_with_matching_entry_and_setup() {
        let item = plugin("feishu-cli");
        let dir = tempfile::tempdir().unwrap();
        let state = BuiltInState {
            revision: Some(item.revision.clone()),
            ..Default::default()
        };
        install_market_skill(&dir.path().join(built_in_setup_id(&item)), &item.setup).unwrap();
        assert!(!built_in_ready_at(&item, &state, dir.path()));
        let entry = dir.path().join(&item.skill_id);
        install_market_skill(&entry, item.entry.as_deref().unwrap()).unwrap();
        assert!(built_in_ready_at(&item, &state, dir.path()));
        // entry 内容被篡改 → 需修复
        fs::write(
            entry.join("SKILL.md"),
            "---\nkivio-market-managed: true\n---\n",
        )
        .unwrap();
        assert!(!built_in_ready_at(&item, &state, dir.path()));
        // revision 变化 → 需修复
        install_market_skill(&entry, item.entry.as_deref().unwrap()).unwrap();
        let stale = BuiltInState {
            revision: Some("old".into()),
            ..Default::default()
        };
        assert!(!built_in_ready_at(&item, &stale, dir.path()));
        assert!(built_in_local(&item, &stale, false, &[]).is_some());
        assert!(built_in_local(&item, &BuiltInState::default(), false, &[]).is_none());
    }

    #[test]
    fn loaded_state_follows_shared_skill_settings_instead_of_legacy_market_flag() {
        let item = plugin("feishu-cli");
        // Old market records may contain a stale enabled flag after a Skill Center edit.
        let saved: BuiltInState = serde_json::from_value(json!({
            "revision": item.revision, "enabled": true,
        }))
        .unwrap();
        for skill in built_in_skill_ids(&item) {
            let local = built_in_local(&item, &saved, true, &[skill]).unwrap();
            assert_eq!(local["status"], "ready");
            assert_eq!(local["enabled"], false);
        }
        let saved: BuiltInState = serde_json::from_value(json!({
            "revision": item.revision, "enabled": false,
        }))
        .unwrap();
        assert_eq!(
            built_in_local(&item, &saved, true, &[]).unwrap()["enabled"],
            true
        );
    }

    #[test]
    fn market_skill_never_overwrites_user_skill() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("feishu-cli");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: mine\n---\n").unwrap();
        assert!(install_market_skill(&skill, "new").is_err());
        remove_market_skill(&skill).unwrap();
        assert!(skill.join("SKILL.md").is_file());
    }

    #[test]
    fn repair_replaces_only_incomplete_market_owned_skills() {
        let item = test_item(&["demo"], "skills");
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("demo");
        fs::create_dir_all(&skill).unwrap();
        let mut state = BuiltInState::default();
        state.owned_skills.push("demo".into());
        assert!(built_in_skill_plan(&item, &state, dir.path()).is_err());
        fs::write(
            skill.join(".kivio-market-owner.json"),
            format!(r#"{{"id":"{}"}}"#, item.id),
        )
        .unwrap();
        let (missing, replace) = built_in_skill_plan(&item, &state, dir.path()).unwrap();
        assert_eq!(missing, vec!["demo"]);
        assert_eq!(replace, vec!["demo"]);
        fs::write(skill.join("SKILL.md"), "# Demo").unwrap();
        assert_eq!(
            built_in_skill_plan(&item, &state, dir.path()).unwrap(),
            (vec![], vec![])
        );
    }

    #[test]
    fn archive_extracts_only_listed_skills_and_rejects_escape() {
        use std::io::Write;
        let item = test_item(&["demo"], "skills");
        let build = |entries: &[(&str, &str)]| {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let options = zip::write::SimpleFileOptions::default();
            for (name, body) in entries {
                zip.start_file(*name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap().into_inner()
        };
        let dir = tempfile::tempdir().unwrap();
        let bytes = build(&[
            ("repo-rev/skills/demo/SKILL.md", "# Demo"),
            ("repo-rev/skills/other/SKILL.md", "# Other"),
            ("repo-rev/README.md", "readme"),
        ]);
        unpack_built_in_skills(&item, bytes, dir.path()).unwrap();
        assert!(dir.path().join("demo/SKILL.md").is_file());
        assert!(!dir.path().join("other").exists());
        assert!(!dir.path().join("README.md").exists());

        let root_item = test_item(&["demo"], "root");
        let bad = build(&[("repo-rev/../escape.md", "x")]);
        assert!(
            unpack_built_in_skills(&root_item, bad, tempfile::tempdir().unwrap().path()).is_err()
        );
    }
}
