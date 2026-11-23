//! DSH replaces an entry's entire config when applying a Cordis patch.
//! Keep the native document and higher-priority home patch separate so a write
//! cannot silently change a global override or flatten unrelated plugin rows.
use super::{yaml_key, Mapping, KIVIO_PROFILE};
use serde_yaml::Value;
use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

const PATCH_FILE: &str = "cordis.patch.yml";
const SHELL_ID: &str = if cfg!(windows) {
    "pwsh-sandbox"
} else {
    "bash-sandbox"
};

pub(crate) fn native_settings_path(home: &Path) -> PathBuf {
    let legacy = home.join("settings.yaml");
    if legacy.is_file() {
        legacy
    } else {
        home.join("profiles").join(KIVIO_PROFILE).join(PATCH_FILE)
    }
}

pub(super) fn read_optional(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("读取 dsh 配置 {} 失败：{e}", path.display())),
    }
}

// serde_yaml recognizes local tags, whereas Cordis uses a global !!js tag.
// Rewrite only YAML tag tokens so a string or a comment containing !!js stays literal.
fn rewrite_tag(text: &str, from: &str, to: &str) -> String {
    if !text.contains(from) {
        return text.to_owned();
    }
    yaml_edit::lex(text)
        .into_iter()
        .map(|(kind, token)| {
            if kind == yaml_edit::SyntaxKind::TAG && token == from {
                to
            } else {
                token
            }
        })
        .collect()
}

pub(crate) fn parse_cordis_yaml(text: &str) -> Result<Value, String> {
    serde_yaml::from_str(&rewrite_tag(text, "!!js", "!cordis-js"))
        .map_err(|err| format!("解析 dsh Cordis 配置失败：{err}"))
}

pub(crate) fn render_cordis_yaml(value: &Value) -> Result<String, String> {
    let raw = serde_yaml::to_string(value).map_err(|err| format!("序列化 dsh 配置失败：{err}"))?;
    Ok(rewrite_tag(&raw, "!cordis-js", "!!js"))
}

fn entry_id(ns: &str) -> &str {
    if ns == "shell" {
        SHELL_ID
    } else {
        ns
    }
}

fn namespace(id: &str) -> Option<&str> {
    match id {
        SHELL_ID => Some("shell"),
        "agent-loop"
        | "web-search-deepseek"
        | "llm-deepseek"
        | "llm-deepseek-account"
        | "agent-preset-registry"
        | "llm-pi-ai"
        | "agent-default-model"
        | "api-gateway" => Some(id),
        _ => None,
    }
}

fn module_matches(id: &str, name: Option<&str>) -> bool {
    name.is_none_or(|name| {
        let package_id = if id == "llm-deepseek" {
            "llm-deepseek-api-key"
        } else {
            id
        };
        name.strip_prefix("@deepseek-ai/dsh-") == Some(package_id)
    })
}

fn collect_configs(rows: &[Value], configs: &mut Mapping) {
    for row in rows {
        if let Some(inserted) = row.get("insert").and_then(Value::as_sequence) {
            collect_configs(inserted, configs);
        } else if row.get("group").and_then(Value::as_bool) == Some(true) {
            if let Some(children) = row.get("config").and_then(Value::as_sequence) {
                collect_configs(children, configs);
            }
        } else if let (Some(id), Some(config)) =
            (row.get("id").and_then(Value::as_str), row.get("config"))
        {
            if let Some(ns) = namespace(id)
                .filter(|_| module_matches(id, row.get("name").and_then(Value::as_str)))
            {
                configs.insert(yaml_key(ns), config.clone());
            }
        }
    }
}

fn patch_configs(text: &str) -> Result<Mapping, String> {
    let mut configs = Mapping::new();
    if !text.trim().is_empty() {
        let value = parse_cordis_yaml(text)?;
        let rows = value
            .as_sequence()
            .ok_or("dsh Cordis patch 必须是 YAML 数组")?;
        collect_configs(rows, &mut configs);
    }
    Ok(configs)
}

// Before the profile has been initialized, use the raw rows shipped by the rc.5
// base bundle. Schema defaults apply only when the entry has no raw value.
fn bundle_settings(profile: &Path) -> Result<Mapping, String> {
    let mut inherited = Mapping::new();
    if !cfg!(windows) {
        inherited.insert(
            yaml_key("shell"),
            serde_yaml::to_value(serde_json::json!({"timeoutMs": 60000})).unwrap(),
        );
    }
    inherited.insert(
        yaml_key("agent-loop"),
        serde_yaml::to_value(serde_json::json!({"agents": []})).unwrap(),
    );
    inherited.insert(
        yaml_key("web-search-deepseek"),
        serde_yaml::to_value(serde_json::json!({"apiKeyEnv":"DEEPSEEK_API_KEY"})).unwrap(),
    );
    let executable = crate::external_agents::overrides::custom_path("dsh")
        .or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH")?)
                .flat_map(|dir| ["dsh", "dsh.cmd", "dsh.exe"].map(|name| dir.join(name)))
                .find(|path| path.is_file())
        })
        .and_then(|path| path.canonicalize().ok());
    for path in bundle_patch_files(profile, executable.as_deref()) {
        inherited.extend(cached_bundle_settings(&path)?);
    }
    Ok(inherited)
}

pub(super) fn bundle_patch_files(profile: &Path, bin: Option<&Path>) -> Vec<PathBuf> {
    let manifest = read_manifest(profile);
    let bundles = manifest
        .as_ref()
        .and_then(|m| m.pointer("/dsh/profile/bundles"))
        .and_then(|v| v.as_array());
    let names = bundles
        .map(|items| items.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_else(|| vec!["@deepseek-ai/dsh-base"]);
    let executable = bin.and_then(|path| path.canonicalize().ok());
    let mut files = Vec::new();
    for name in names {
        let dir = if name.starts_with("./") {
            Some(profile.join(name))
        } else {
            executable
                .as_deref()
                .into_iter()
                .flat_map(Path::ancestors)
                .chain(profile.ancestors())
                .flat_map(|anchor| {
                    [
                        anchor.join("node_modules").join(name),
                        anchor
                            .join("node_modules/@deepseek-ai/dsh/node_modules")
                            .join(name),
                    ]
                })
                .find(|dir| dir.join("package.json").is_file())
        };
        let Some(dir) = dir else { continue };
        let Some(package) = read_manifest(&dir) else {
            continue;
        };
        let Some(patches) = package.pointer("/dsh/bundle/patch") else {
            continue;
        };
        for patch in patches.as_str().into_iter().chain(
            patches
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str()),
        ) {
            let path = dir.join(patch);
            if path.is_file() {
                files.push(path);
            }
        }
    }
    files
}

fn read_manifest(dir: &Path) -> Option<serde_json::Value> {
    serde_json::from_slice(&std::fs::read(dir.join("package.json")).ok()?).ok()
}

fn cached_bundle_settings(path: &Path) -> Result<Mapping, String> {
    use std::{
        collections::HashMap,
        sync::{LazyLock, Mutex},
        time::SystemTime,
    };
    type BundleEntry = ((u64, Option<SystemTime>), Mapping);
    static CACHE: LazyLock<Mutex<HashMap<PathBuf, BundleEntry>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(Mapping::new());
    };
    let version = (meta.len(), meta.modified().ok());
    let mut cache = CACHE.lock().map_err(|_| "读取 dsh bundle 缓存失败")?;
    if let Some((cached, settings)) = cache.get(path) {
        if *cached == version {
            return Ok(settings.clone());
        }
    }
    let settings = patch_configs(&read_optional(path)?)?;
    cache.insert(path.to_owned(), (version, settings.clone()));
    Ok(settings)
}

pub(super) struct SettingsDocument {
    pub path: PathBuf,
    pub effective: Mapping,
    pub inherited: Mapping,
    home_overrides: Mapping,
    text: String,
    legacy: bool,
}

impl SettingsDocument {
    pub fn open(path: &Path) -> Result<Self, String> {
        let text = read_optional(path)?;
        let legacy = path.file_name().is_some_and(|name| name == "settings.yaml");
        let mut home_overrides = Mapping::new();
        let mut inherited = Mapping::new();
        let effective = if legacy {
            if text.trim().is_empty() {
                Mapping::new()
            } else {
                serde_yaml::from_str::<Option<Mapping>>(&text)
                    .map_err(|e| format!("解析 dsh settings.yaml 失败：{e}"))?
                    .unwrap_or_default()
            }
        } else {
            let home = path.ancestors().nth(3).ok_or("dsh profile 路径无效")?;
            let native = if crate::external_agents::dsh_profile::looks_like_legacy_profile(&text) {
                let config =
                    crate::external_agents::overrides::agent_config("dsh").unwrap_or_default();
                let filtered = crate::external_agents::dsh_profile::legacy_native_patch(
                    &text,
                    &config.providers,
                )?;
                patch_configs(filtered.as_deref().unwrap_or(&text))?
            } else {
                patch_configs(&text)?
            };
            home_overrides = patch_configs(&read_optional(&home.join(PATCH_FILE))?)?;
            inherited = bundle_settings(path.parent().ok_or("dsh profile 路径无效")?)?;
            let mut full = inherited.clone();
            full.extend(native);
            full.extend(home_overrides.clone());
            full
        };
        Ok(Self {
            path: path.to_owned(),
            effective,
            inherited,
            home_overrides,
            text,
            legacy,
        })
    }

    pub fn save(&self, next: Mapping) -> Result<(), String> {
        if self.effective == next {
            return Ok(());
        }
        if self.legacy {
            return super::persist_settings(&self.path, next);
        }
        let raw = if self.text.trim().is_empty() {
            "[]\n"
        } else {
            &self.text
        };
        let doc = yaml_edit::Document::from_str(raw)
            .map_err(|e| format!("解析 dsh profile 失败：{e}"))?;
        let rows = doc
            .as_sequence()
            .ok_or("dsh Cordis patch 必须是 YAML 数组")?;
        let changed = self
            .effective
            .keys()
            .chain(next.keys())
            .collect::<std::collections::HashSet<_>>();
        for key in changed {
            let previous = self.effective.get(key);
            let new = next.get(key);
            if previous == new {
                continue;
            }
            let ns = key.as_str().ok_or("dsh 配置项缺少名称")?;
            if self.home_overrides.contains_key(key) {
                return Err(format!(
                    "{} 中的 {} 覆盖了当前 profile；请在该文件中修改",
                    self.path
                        .ancestors()
                        .nth(3)
                        .unwrap()
                        .join(PATCH_FILE)
                        .display(),
                    entry_id(ns)
                ));
            }
            let id = entry_id(ns);
            let inherited = self.inherited.get(key);
            let config = new
                .or(inherited)
                .cloned()
                .unwrap_or(Value::Mapping(Mapping::new()));
            let mut row = Mapping::new();
            row.insert(yaml_key("id"), yaml_key(id));
            row.insert(yaml_key("config"), config);
            let single = yaml_edit::Document::from_str(&render_cordis_yaml(&Value::Mapping(row))?)
                .map_err(|e| e.to_string())?;
            let entry = single.as_mapping().ok_or("dsh patch 条目无效")?;
            let original_generated =
                crate::external_agents::dsh_profile::looks_like_legacy_profile(&self.text);
            let targets: Vec<_> = rows
                .values()
                .enumerate()
                .filter_map(|(i, row)| Some((i, row.as_mapping()?.clone())))
                .filter(|(_, row)| {
                    row.get("id").is_some_and(|v| v == id)
                        && row.get("insert").is_none()
                        && module_matches(
                            id,
                            row.get("name")
                                .and_then(|n| n.as_scalar().map(|s| s.as_string()))
                                .as_deref(),
                        )
                })
                .collect();
            if !original_generated && (new.is_none() || new == inherited) {
                // Settings reset must keep following future changes in the underlying bundle.
                for (index, row) in targets.iter().rev() {
                    if row.remove("config").is_some()
                        && row.len() == 1 + usize::from(row.get("name").is_some())
                    {
                        rows.remove(*index);
                    }
                }
                let remaining = patch_configs(&doc.to_string())?;
                if remaining.get(ns).or(inherited) == new.or(inherited) {
                    continue;
                }
            }
            if let Some((_, existing)) = targets
                .last()
                .filter(|_| !original_generated && new.is_some() && new != inherited)
            {
                existing.set(
                    "config",
                    entry.get("config").ok_or("dsh 配置行缺少 config")?,
                );
            } else {
                rows.push(entry);
            }
        }
        let rendered = doc.to_string();
        // Verify the resulting document before replacing a file that may contain credentials or expressions.
        let mut configs = self.inherited.clone();
        configs.extend(patch_configs(&rendered)?);
        configs.extend(self.home_overrides.clone());
        for key in self.effective.keys().chain(next.keys()) {
            if self.effective.get(key) != next.get(key) {
                let expected = next.get(key).or(self.inherited.get(key));
                if configs.get(key) != expected
                    && !(expected.is_none()
                        && configs
                            .get(key)
                            .and_then(Value::as_mapping)
                            .is_some_and(Mapping::is_empty))
                {
                    return Err("无法保留 dsh Cordis 配置的结构".into());
                }
            }
        }
        let dir = self.path.parent().ok_or("dsh profile 路径无效")?;
        with_dsh_writer_lock(&dir.join("package.json.lock"), || {
            if read_optional(&self.path)? != self.text {
                return Err("dsh profile 配置刚刚变更，请刷新后重试".into());
            }
            crate::external_agents::provider_profile::write_private_atomic(&self.path, &rendered)
        })
    }
}

/// Cooperate with dsh-atomic-write's exclusive <filename>.lock PID record.
pub(super) fn with_dsh_writer_lock(
    lock_path: &Path,
    write: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    use std::io::Write;
    let dir = lock_path.parent().ok_or("dsh 配置路径无效")?;
    std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let mut lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
        .map_err(|e| format!("无法锁定 dsh 配置（{}）：{e}", lock_path.display()))?;
    let result = writeln!(lock, "{}", std::process::id())
        .map_err(|e| e.to_string())
        .and_then(|_| write());
    drop(lock);
    result.and(std::fs::remove_file(lock_path).map_err(|e| e.to_string()))
}

pub(crate) fn read_profile_settings(home: &Path) -> Result<Mapping, String> {
    Ok(
        SettingsDocument::open(&home.join("profiles").join(KIVIO_PROFILE).join(PATCH_FILE))?
            .effective,
    )
}

pub(crate) fn read_effective_settings(home: &Path) -> Result<Mapping, String> {
    Ok(SettingsDocument::open(&native_settings_path(home))?.effective)
}

/// Preserve settings whose later launch overlay would make rc.5's built-in
/// one-time settings import fail its effective-value check.
pub(crate) fn preserve_legacy_launch_settings(home: &Path) -> Result<(), String> {
    let legacy = home.join("settings.yaml");
    if !legacy.is_file() {
        return Ok(());
    }
    let old = SettingsDocument::open(&legacy)?.effective;
    let profile =
        SettingsDocument::open(&home.join("profiles").join(KIVIO_PROFILE).join(PATCH_FILE))?;
    let mut next = profile.effective.clone();
    for id in ["llm-pi-ai", "llm-deepseek", "agent-preset-registry"] {
        if profile.home_overrides.contains_key(id) {
            continue;
        }
        if let Some(config) = old.get(id) {
            let entry = next
                .entry(yaml_key(id))
                .or_insert_with(|| Value::Mapping(Mapping::new()));
            if let (Some(current), Some(legacy)) = (entry.as_mapping_mut(), config.as_mapping()) {
                let mut merged = legacy.clone();
                merged.extend(current.clone());
                if id == "llm-pi-ai" {
                    if let Some(legacy_providers) =
                        legacy.get("providers").and_then(Value::as_mapping)
                    {
                        let mut routes = legacy_providers.clone();
                        if let Some(existing) = current.get("providers").and_then(Value::as_mapping)
                        {
                            routes.extend(existing.clone());
                        }
                        merged.insert(yaml_key("providers"), Value::Mapping(routes));
                    }
                }
                *current = merged;
            }
        }
    }
    profile.save(next)
}
