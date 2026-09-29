//! User-added Claude-compatible catalogs. Installed package state belongs to packages.rs.
use super::packages::{self, Package, PackageMarketplace};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const MANIFEST: &str = ".claude-plugin/marketplace.json";
const MAX_JSON: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Marketplace {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub plugins: Vec<MarketplacePlugin>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePlugin {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub version: Option<String>,
    pub category: String,
    pub unavailable_reason: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct SavedMarket {
    id: String,
    source: String,
    manifest: Value,
    root: Option<PathBuf>,
    cache: Option<String>,
}
#[derive(Debug, PartialEq)]
enum Source {
    Local(PathBuf),
    Json(String),
    Git(String, Option<String>),
}

fn storage() -> Result<PathBuf, String> {
    super::plugins_root()
        .map(|p| p.join("marketplaces"))
        .ok_or("Application data directory unavailable".into())
}
fn lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(Default::default)
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        && name != "."
        && name != ".."
}
fn relative(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains(['\\', ':'])
        || Path::new(path).is_absolute()
        || Path::new(path).components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir
                    | std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
            )
        })
    {
        return Err("插件路径必须位于市场目录内，不能使用绝对路径或 ..".into());
    }
    Ok(())
}
fn github(repo: &str) -> Result<String, String> {
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2 || !parts.iter().all(|p| valid_name(p)) {
        return Err("GitHub 来源须为 owner/repo".into());
    }
    Ok(format!(
        "https://github.com/{}.git",
        repo.trim_end_matches(".git")
    ))
}
fn parse_source(input: &str) -> Result<Source, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("请输入插件市场来源".into());
    }
    let path = Path::new(input);
    if path.exists() {
        return Ok(Source::Local(
            fs::canonicalize(path).map_err(|e| e.to_string())?,
        ));
    }
    let (base, revision) = input
        .rsplit_once('#')
        .map(|(s, r)| (s, Some(r.to_string())))
        .unwrap_or((input, None));
    if base.starts_with("https://") {
        let url = packages::https_url(base)?;
        if url.path().ends_with(".json") {
            if revision.is_some() {
                return Err("JSON 市场地址不接受 Git revision".into());
            }
            return Ok(Source::Json(base.into()));
        }
        return Ok(Source::Git(base.trim_end_matches('/').into(), revision));
    }
    let (repo, revision) = if revision.is_none() {
        base.rsplit_once('@')
            .map(|(s, r)| (s, Some(r.to_string())))
            .unwrap_or((base, revision))
    } else {
        (base, revision)
    };
    Ok(Source::Git(github(repo)?, revision))
}
fn canonical_source(source: &Source) -> String {
    match source {
        Source::Local(p) => p.to_string_lossy().into(),
        Source::Json(url) => url.clone(),
        Source::Git(url, revision) => format!(
            "{}{}",
            url.trim_end_matches(".git"),
            revision
                .as_ref()
                .map(|r| format!("#{r}"))
                .unwrap_or_default()
        ),
    }
}
fn read_json(path: &Path) -> Result<Value, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_JSON as u64 {
        return Err("市场清单超过 4 MiB".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("市场 JSON 无效：{e}"))
}
fn load(store: &Path) -> Result<Vec<SavedMarket>, String> {
    let file = store.join("sources.json");
    if !file.exists() {
        return Ok(vec![]);
    }
    // The registry contains multiple catalogs; the per-catalog size limit is checked on fetch.
    serde_json::from_slice(&fs::read(file).map_err(|e| e.to_string())?)
        .map_err(|e| format!("市场记录无法读取：{e}"))
}
fn save(store: &Path, markets: &[SavedMarket]) -> Result<(), String> {
    fs::create_dir_all(store).map_err(|e| e.to_string())?;
    let tmp = store.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(markets).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let result = fs::rename(&tmp, store.join("sources.json")).map_err(|e| e.to_string());
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
fn cleanup_cache(store: &Path, id: &str) {
    // Only generated UUID cache directories are owned by this module, never local sources.
    if uuid::Uuid::parse_str(id).is_ok() {
        let _ = fs::remove_dir_all(store.join("cache").join(id));
    }
}
fn validate_manifest(value: &Value) -> Result<(), String> {
    if !valid_name(text(value, "name")) {
        return Err("marketplace.json 缺少有效的 name".into());
    }
    if text(&value["owner"], "name").trim().is_empty() {
        return Err("marketplace.json 缺少 owner.name".into());
    }
    let entries = value["plugins"]
        .as_array()
        .ok_or("marketplace.json 的 plugins 必须是数组")?;
    if entries.len() > 5000 {
        return Err("单个市场最多支持 5000 个插件".into());
    }
    let mut names = HashSet::new();
    for entry in entries {
        let name = text(entry, "name");
        if !valid_name(name) || !names.insert(name) {
            return Err(format!("插件名称无效或重复：{name}"));
        }
        if !entry["source"].is_string() && !entry["source"].is_object() {
            return Err(format!("{name} 缺少 source"));
        }
        if entry.get("strict").is_some_and(|v| !v.is_boolean()) {
            return Err(format!("{name} 的 strict 必须为布尔值"));
        }
    }
    Ok(())
}

// Source resolution is used by both listing (availability) and installation.
fn plugin_source(
    market: &SavedMarket,
    entry: &Value,
) -> Result<(String, Option<String>, Option<String>), String> {
    let source = &entry["source"];
    if let Some(path) = source.as_str() {
        let path = if path == "." || path.starts_with("./") {
            path.to_string()
        } else if !path.contains('/')
            && !text(&market.manifest["metadata"], "pluginRoot").is_empty()
        {
            format!(
                "{}/{path}",
                text(&market.manifest["metadata"], "pluginRoot").trim_end_matches('/')
            )
        } else {
            return Err("相对插件来源必须以 ./ 开头".into());
        };
        relative(&path)?;
        let root = market
            .root
            .as_ref()
            .ok_or("JSON 地址市场没有仓库文件；请改用 Git 仓库添加此市场")?;
        // Check boundary even if a local market contains a directory junction.
        let selected = packages::contained(root, &path)?;
        if !selected.is_dir() {
            return Err("插件来源不是目录".into());
        }
        return Ok((root.to_string_lossy().into(), Some(path), None));
    }
    let kind = text(source, "source");
    let url = match kind {
        "github" => github(text(source, "repo"))?,
        "url" => text(source, "url").to_string(),
        "git-subdir" => {
            let url = text(source, "url");
            if url.starts_with("https://") {
                url.into()
            } else {
                github(url)?
            }
        }
        _ => {
            return Err(format!(
                "暂不支持插件来源类型：{kind}（支持相对目录、github、url、git-subdir）"
            ))
        }
    };
    packages::https_url(&url)?;
    let path = if kind == "git-subdir" || source.get("path").is_some() {
        let path = source
            .get("path")
            .and_then(Value::as_str)
            .ok_or("插件子目录 path 必须为字符串")?;
        relative(path)?;
        Some(path.to_string())
    } else {
        None
    };
    let sha = text(source, "sha");
    if !sha.is_empty() && (sha.len() != 40 || !sha.bytes().all(|c| c.is_ascii_hexdigit())) {
        return Err("sha 必须为完整的 40 位 Git 提交 ID".into());
    }
    let revision = if !sha.is_empty() {
        sha
    } else {
        text(source, "ref")
    };
    if revision.starts_with('-') || revision.chars().any(char::is_control) {
        return Err("无效的 Git revision".into());
    }
    Ok((
        url,
        path,
        (!revision.is_empty()).then(|| revision.to_string()),
    ))
}
fn summary(market: &SavedMarket) -> Marketplace {
    Marketplace {
        id: market.id.clone(),
        name: text(&market.manifest, "name").into(),
        description: text(&market.manifest, "description").into(),
        source: market.source.clone(),
        plugins: market.manifest["plugins"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| MarketplacePlugin {
                name: text(entry, "name").into(),
                display_name: entry["displayName"]
                    .as_str()
                    .unwrap_or(text(entry, "name"))
                    .into(),
                description: text(entry, "description").into(),
                version: entry["version"].as_str().map(Into::into),
                category: text(entry, "category").into(),
                unavailable_reason: plugin_source(market, entry).err(),
            })
            .collect(),
    }
}
async fn fetch_json(url: &str) -> Result<Value, String> {
    packages::https_url(url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .https_only(true)
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > MAX_JSON {
            return Err("市场清单超过 4 MiB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|e| format!("市场 JSON 无效：{e}"))
}
async fn fetch_market(store: &Path, input: &str) -> Result<SavedMarket, String> {
    let source = parse_source(input)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut cache = None;
    let result = async {
        let (manifest, root) = match &source {
            Source::Local(path) => {
                let file = if path.is_dir() {
                    path.join(MANIFEST)
                } else {
                    path.clone()
                };
                let parent = file.parent().ok_or("市场文件缺少父目录")?;
                let root = if parent.file_name().is_some_and(|p| p == ".claude-plugin") {
                    parent.parent().ok_or("市场根目录无效")?
                } else {
                    parent
                };
                (read_json(&file)?, Some(root.to_path_buf()))
            }
            Source::Json(url) => (fetch_json(url).await?, None),
            Source::Git(url, revision) => {
                let root = store.join("cache").join(&id);
                fs::create_dir_all(root.parent().unwrap()).map_err(|e| e.to_string())?;
                cache = Some(id.clone());
                packages::clone_repository(url, &root, revision.as_deref()).await?;
                let file = packages::contained(&root, MANIFEST)?;
                (read_json(&file)?, Some(root))
            }
        };
        validate_manifest(&manifest)?;
        Ok(SavedMarket {
            id: id.clone(),
            source: canonical_source(&source),
            manifest,
            root,
            cache: cache.clone(),
        })
    }
    .await;
    if result.is_err() {
        if let Some(cache) = cache {
            cleanup_cache(store, &cache);
        }
    }
    result
}
async fn add_or_refresh(
    store: &Path,
    source: &str,
    id: Option<&str>,
) -> Result<Vec<Marketplace>, String> {
    let mut markets = load(store)?;
    let mut next = fetch_market(store, source).await?;
    let result = (|| {
        let index = if let Some(id) = id {
            Some(
                markets
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or("市场已移除")?,
            )
        } else {
            markets.iter().position(|m| m.source == next.source)
        };
        if let Some(index) = index {
            if text(&markets[index].manifest, "name") != text(&next.manifest, "name") {
                return Err("市场名称发生变化，请移除旧来源后重新添加".into());
            }
            next.id = markets[index].id.clone();
        }
        if markets.iter().enumerate().any(|(i, m)| {
            Some(i) != index && text(&m.manifest, "name") == text(&next.manifest, "name")
        }) {
            return Err("已有同名市场来自其他来源，请先检查原有来源".into());
        }
        let old_cache = index.and_then(|i| markets[i].cache.clone());
        if let Some(i) = index {
            markets[i] = next.clone();
        } else {
            markets.push(next.clone());
        }
        save(store, &markets)?;
        if let Some(cache) = old_cache {
            cleanup_cache(store, &cache);
        }
        Ok(markets.iter().map(summary).collect())
    })();
    if result.is_err() {
        if let Some(cache) = next.cache {
            cleanup_cache(store, &cache);
        }
    }
    result
}
#[tauri::command]
pub async fn plugin_marketplaces_list() -> Result<Vec<Marketplace>, String> {
    let _guard = lock().lock().await;
    Ok(load(&storage()?)?.iter().map(summary).collect())
}
#[tauri::command]
pub async fn plugin_marketplaces_add(source: String) -> Result<Vec<Marketplace>, String> {
    let _guard = lock().lock().await;
    add_or_refresh(&storage()?, &source, None).await
}
#[tauri::command]
pub async fn plugin_marketplaces_refresh(id: String) -> Result<Vec<Marketplace>, String> {
    let _guard = lock().lock().await;
    let store = storage()?;
    let saved = load(&store)?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or("找不到这个市场")?;
    add_or_refresh(&store, &saved.source, Some(&id)).await
}
fn remove(store: &Path, id: &str) -> Result<Vec<Marketplace>, String> {
    let mut markets = load(store)?;
    let index = markets
        .iter()
        .position(|m| m.id == id)
        .ok_or("找不到这个市场")?;
    let removed = markets.remove(index);
    save(store, &markets)?;
    if let Some(cache) = removed.cache {
        cleanup_cache(store, &cache);
    }
    Ok(markets.iter().map(summary).collect())
}
#[tauri::command]
pub async fn plugin_marketplaces_remove(id: String) -> Result<Vec<Marketplace>, String> {
    let _guard = lock().lock().await;
    remove(&storage()?, &id)
}
async fn install(store: &Path, id: &str, plugin: &str) -> Result<Package, String> {
    let market = load(store)?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or("找不到这个市场")?;
    let entry = market.manifest["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| text(p, "name") == plugin)
        .ok_or("市场中找不到这个插件")?;
    let origin = PackageMarketplace {
        source: market.source.clone(),
        name: text(&market.manifest, "name").into(),
        plugin: plugin.into(),
    };
    if let Some(existing) = packages::plugin_packages_list()?
        .into_iter()
        .find(|p| p.marketplace.as_ref() == Some(&origin))
    {
        return Ok(existing);
    }
    let (source, subdirectory, revision) = plugin_source(&market, entry)?;
    packages::import_package(
        source,
        subdirectory,
        revision,
        Some((entry.clone(), origin)),
    )
    .await
}
#[tauri::command]
pub async fn plugin_marketplaces_install(id: String, plugin: String) -> Result<Package, String> {
    let _guard = lock().lock().await;
    install(&storage()?, &id, &plugin).await
}

async fn describe(
    store: &Path,
    id: &str,
    plugin: &str,
) -> Result<packages::details::Details, String> {
    let market = load(store)?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or("找不到这个市场")?;
    let entry = market.manifest["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| text(p, "name") == plugin)
        .ok_or("市场中找不到这个插件")?;
    if let Some(existing) = packages::plugin_packages_list()?.into_iter().find(|p| {
        p.marketplace
            .as_ref()
            .is_some_and(|origin| origin.source == market.source && origin.plugin == plugin)
    }) {
        return packages::details::plugin_packages_describe(existing.id).await;
    }
    let (source, subdirectory, revision) = plugin_source(&market, entry)?;
    packages::details::describe_source(&source, subdirectory.as_deref(), revision.as_deref(), entry)
        .await
}
#[tauri::command]
pub async fn plugin_marketplaces_describe(
    id: String,
    plugin: String,
) -> Result<packages::details::Details, String> {
    let _guard = lock().lock().await;
    describe(&storage()?, &id, &plugin).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn local_market(root: &Path, name: &str) {
        fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        fs::create_dir_all(root.join("plugins/demo/skills/demo")).unwrap();
        fs::write(
            root.join("plugins/demo/skills/demo/SKILL.md"),
            "---\nname: demo\ndescription: Demo\n---\nDemo skill",
        )
        .unwrap();
        fs::write(root.join(MANIFEST), serde_json::to_vec(&json!({"name":name,"owner":{"name":"Tester"},"plugins":[{"name":"demo","source":"./plugins/demo","description":"Demo plugin"}]})).unwrap()).unwrap();
    }

    #[tokio::test]
    async fn local_add_refresh_install_and_remove_preserve_installed_copy() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        let store = dir.path().join("registry");
        let _packages = packages::TestPackagesRoot::new(&dir.path().join("installed"));
        local_market(&root, "team");
        let markets = add_or_refresh(&store, root.to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(markets[0].plugins.len(), 1);
        assert!(markets[0].plugins[0].unavailable_reason.is_none());
        let id = &markets[0].id;
        let duplicate = add_or_refresh(&store, root.to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(duplicate.len(), 1);
        assert_eq!(&duplicate[0].id, id);
        let package = install(&store, id, "demo").await.unwrap();
        assert!(!package.enabled);
        assert_eq!(package.components["skills"], 1);
        assert_eq!(package.marketplace.as_ref().unwrap().name, "team");
        assert_eq!(install(&store, id, "demo").await.unwrap().id, package.id);
        assert!(!root
            .join("plugins/demo/.claude-plugin/plugin.json")
            .exists());
        fs::write(root.join(MANIFEST), "invalid JSON").unwrap();
        assert!(add_or_refresh(&store, root.to_str().unwrap(), Some(id))
            .await
            .is_err());
        assert_eq!(load(&store).unwrap()[0].manifest["name"], "team");
        assert!(remove(&store, id).unwrap().is_empty());
        assert!(root.exists());
        assert_eq!(packages::plugin_packages_list().unwrap()[0].id, package.id);
    }

    #[tokio::test]
    async fn identity_changes_and_same_name_other_source_do_not_replace_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        let store = dir.path().join("registry");
        local_market(&first, "team");
        local_market(&second, "team");
        let markets = add_or_refresh(&store, first.to_str().unwrap(), None)
            .await
            .unwrap();
        assert!(add_or_refresh(&store, second.to_str().unwrap(), None)
            .await
            .is_err());
        local_market(&first, "renamed");
        assert!(
            add_or_refresh(&store, first.to_str().unwrap(), Some(&markets[0].id))
                .await
                .is_err()
        );
        assert_eq!(load(&store).unwrap()[0].manifest["name"], "team");
    }

    #[test]
    fn source_resolution_rejects_escape_and_reports_unsupported_entries() {
        let dir = tempfile::tempdir().unwrap();
        local_market(dir.path(), "team");
        let mut market = SavedMarket {
            id: "id".into(),
            source: "local".into(),
            root: Some(dir.path().into()),
            cache: None,
            manifest: read_json(&dir.path().join(MANIFEST)).unwrap(),
        };
        for source in ["./../outside", "./C:/outside", "./plugins/../../outside"] {
            assert!(plugin_source(&market, &json!({"source":source})).is_err());
        }
        market.manifest["metadata"] = json!({"pluginRoot":"./plugins"});
        assert_eq!(
            plugin_source(&market, &json!({"source":"demo"}))
                .unwrap()
                .1
                .as_deref(),
            Some("./plugins/demo")
        );
        market.root = None;
        assert!(plugin_source(&market, &json!({"source":"./plugins/demo"}))
            .unwrap_err()
            .contains("JSON"));
        assert!(plugin_source(
            &market,
            &json!({"source":{"source":"npm","package":"demo"}})
        )
        .unwrap_err()
        .contains("npm"));
        let sha = "a".repeat(40);
        let resolved = plugin_source(&market, &json!({"source":{"source":"git-subdir","url":"owner/repo","path":"plugins/demo","ref":"obsolete","sha":sha}})).unwrap();
        assert_eq!(
            resolved,
            (
                "https://github.com/owner/repo.git".into(),
                Some("plugins/demo".into()),
                Some(sha)
            )
        );
        assert!(plugin_source(
            &market,
            &json!({"source":{"source":"url","url":"https://user:secret@example.com/repo"}})
        )
        .is_err());
        assert_eq!(
            canonical_source(&parse_source("owner/repo#main").unwrap()),
            "https://github.com/owner/repo#main"
        );
    }

    #[test]
    fn remote_sources_honor_subdirectories_and_reject_invalid_paths() {
        let market = SavedMarket {
            id: "test".into(),
            source: "https://example.com/market".into(),
            manifest: json!({}),
            root: None,
            cache: None,
        };
        for kind in ["url", "github", "git-subdir"] {
            let entry = json!({"source":{"source":kind,"repo":"owner/repo","url":"https://github.com/owner/repo.git","path":"plugins/demo"}});
            assert_eq!(
                plugin_source(&market, &entry).unwrap().1.as_deref(),
                Some("plugins/demo")
            );
            for invalid in [
                json!("../outside"),
                json!("C:/outside"),
                json!(42),
                json!(null),
                json!(""),
            ] {
                let mut bad = entry.clone();
                bad["source"]["path"] = invalid;
                assert!(plugin_source(&market, &bad).is_err());
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires access to GitHub"]
    async fn official_representative_details_smoke() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("registry");
        let _packages = packages::TestPackagesRoot::new(&dir.path().join("installed"));
        let markets = add_or_refresh(&store, "anthropics/claude-plugins-official", None)
            .await
            .unwrap();
        let saved = load(&store).unwrap();
        let mut names = saved[0].manifest["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["source"].is_string())
            .map(|e| text(e, "name").to_string())
            .collect::<Vec<_>>();
        names.extend(["alloydb", "superpowers", "atomic-agents", "zilliz"].map(str::to_owned));
        let mut failed = Vec::new();
        for name in &names {
            match describe(&store, &markets[0].id, name).await {
                Ok(details) => println!(
                    "{name}: {:?}; {} component read diagnostics",
                    details
                        .groups
                        .iter()
                        .map(|g| (&g.kind, g.items.len()))
                        .collect::<Vec<_>>(),
                    details.diagnostics.len()
                ),
                Err(error) => failed.push(format!("{name}: {error}")),
            }
        }
        assert!(packages::plugin_packages_list().unwrap().is_empty());
        assert!(failed.is_empty(), "Preview failures: {failed:?}");
        println!("{} representative previews passed", names.len());
    }

    #[tokio::test]
    async fn describe_is_read_only_and_installed_details_survive_source_removal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("source");
        let store = dir.path().join("registry");
        let installed = dir.path().join("installed");
        let _packages = packages::TestPackagesRoot::new(&installed);
        local_market(&root, "team");
        let content = root.join("plugins/demo");
        fs::create_dir_all(content.join(".claude-plugin")).unwrap();
        fs::write(content.join(".claude-plugin/plugin.json"), r#"{"name":"demo","author":{"name":"Developer"},"version":"2.0","homepage":"https://example.com","license":"MIT"}"#).unwrap();
        fs::write(content.join(".mcp.json"), r#"{"mcpServers":{"Demo MCP":{"url":"https://example.com/${MISSING_DETAIL_TEST_TOKEN}"}}}"#).unwrap();
        fs::create_dir_all(content.join("hooks")).unwrap();
        fs::write(
            content.join("hooks/hooks.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"exit 99"}]}]}}"#,
        )
        .unwrap();
        fs::write(content.join("skills/demo/README.md"), "Not a skill").unwrap();
        let markets = add_or_refresh(&store, root.to_str().unwrap(), None)
            .await
            .unwrap();
        let details = describe(&store, &markets[0].id, "demo").await.unwrap();
        assert_eq!(details.author.as_deref(), Some("Developer"));
        assert_eq!(details.version.as_deref(), Some("2.0"));
        assert!(details.diagnostics.is_empty(), "{:?}", details.diagnostics);
        assert_eq!(
            details
                .groups
                .iter()
                .find(|g| g.kind == "skills")
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(
            details
                .groups
                .iter()
                .find(|g| g.kind == "mcp")
                .unwrap()
                .items[0]
                .name,
            "Demo MCP"
        );
        assert_eq!(
            details
                .groups
                .iter()
                .find(|g| g.kind == "hooks")
                .unwrap()
                .items[0]
                .name,
            "SessionStart"
        );
        assert!(packages::plugin_packages_list().unwrap().is_empty());
        assert_eq!(fs::read_dir(&installed).unwrap().count(), 0);
        let package = install(&store, &markets[0].id, "demo").await.unwrap();
        remove(&store, &markets[0].id).unwrap();
        let local = packages::details::plugin_packages_describe(package.id)
            .await
            .unwrap();
        assert_eq!(local.author, details.author);
        assert_eq!(local.groups.len(), details.groups.len());
    }

    #[tokio::test]
    async fn marketplace_multi_manifest_uses_claude_for_preview_install_and_reload() {
        for has_claude in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("source");
            let store = dir.path().join("registry");
            let _packages = packages::TestPackagesRoot::new(&dir.path().join("installed"));
            local_market(&root, "team");
            let content = root.join("plugins/demo");
            for kind in ["kivio", "codex"] {
                fs::create_dir_all(content.join(format!(".{kind}-plugin"))).unwrap();
                fs::write(
                    content.join(format!(".{kind}-plugin/plugin.json")),
                    "invalid unrelated manifest",
                )
                .unwrap();
            }
            if has_claude {
                fs::create_dir_all(content.join(".claude-plugin")).unwrap();
                fs::write(
                    content.join(".claude-plugin/plugin.json"),
                    r#"{"name":"demo","version":"2.0","author":"Claude developer"}"#,
                )
                .unwrap();
            }
            let markets = add_or_refresh(&store, root.to_str().unwrap(), None)
                .await
                .unwrap();
            let details = describe(&store, &markets[0].id, "demo").await.unwrap();
            assert_eq!(details.groups[0].items[0].name, "demo");
            if has_claude {
                assert_eq!(details.author.as_deref(), Some("Claude developer"));
            }
            assert!(packages::plugin_packages_list().unwrap().is_empty());
            let installed = install(&store, &markets[0].id, "demo").await.unwrap();
            assert_eq!(installed.format, "claude");
            assert_eq!(installed.components["skills"], 1);
            assert!(!installed.enabled);
            let reloaded = packages::plugin_packages_list().unwrap();
            assert_eq!(reloaded[0].format, "claude");
            assert!(reloaded[0].diagnostics.is_empty());
            let local = packages::details::plugin_packages_describe(installed.id)
                .await
                .unwrap();
            assert_eq!(local.groups[0].items[0].name, "demo");
            assert_eq!(
                fs::read_to_string(content.join(".codex-plugin/plugin.json")).unwrap(),
                "invalid unrelated manifest"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires access to GitHub"]
    async fn official_data_agent_details_and_install_smoke() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("registry");
        let _packages = packages::TestPackagesRoot::new(&dir.path().join("installed"));
        let markets = add_or_refresh(&store, "anthropics/claude-plugins-official", None)
            .await
            .unwrap();
        let name = "data-agent-kit-starter-pack";
        let details = describe(&store, &markets[0].id, name).await.unwrap();
        let count = details
            .groups
            .iter()
            .find(|g| g.kind == "skills")
            .unwrap()
            .items
            .len();
        assert!(count > 0);
        assert!(packages::plugin_packages_list().unwrap().is_empty());
        let installed = install(&store, &markets[0].id, name).await.unwrap();
        assert_eq!(installed.format, "claude");
        assert!(!installed.enabled);
        assert_eq!(installed.components["skills"], count);
        println!("data-agent-kit: {count} skills; preview and disabled installation passed; diagnostics: {:?}", installed.diagnostics);
    }

    #[tokio::test]
    #[ignore = "requires access to GitHub"]
    async fn official_adobe_details_smoke() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("registry");
        let _packages = packages::TestPackagesRoot::new(&dir.path().join("installed"));
        let markets = add_or_refresh(&store, "anthropics/claude-plugins-official", None)
            .await
            .unwrap();
        let details = describe(&store, &markets[0].id, "adobe-for-creativity")
            .await
            .unwrap();
        println!(
            "Adobe preview: {}",
            serde_json::to_string(&details).unwrap()
        );
        assert!(details
            .groups
            .iter()
            .any(|g| g.kind == "skills" && !g.items.is_empty()));
        assert!(details
            .groups
            .iter()
            .any(|g| g.kind == "mcp" && !g.items.is_empty()));
        assert!(packages::plugin_packages_list().unwrap().is_empty());
    }

    // Explicit opt-in network smoke test, using temporary storage and disabled packages only.
    #[tokio::test]
    #[ignore = "requires access to GitHub"]
    async fn official_market_smoke() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("registry");
        let _packages = packages::TestPackagesRoot::new(&dir.path().join("installed"));
        let markets = add_or_refresh(&store, "anthropics/claude-plugins-official", None)
            .await
            .unwrap();
        assert_eq!(markets[0].name, "claude-plugins-official");
        assert!(markets[0].plugins.len() > 10);
        let package = install(&store, &markets[0].id, "frontend-design")
            .await
            .unwrap();
        assert!(!package.enabled);
        assert!(package.components["skills"] > 0);
        println!(
            "Official market: {} entries; frontend-design installed disabled, {} skills",
            markets[0].plugins.len(),
            package.components["skills"]
        );
    }
}
