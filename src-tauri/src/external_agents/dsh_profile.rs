//! Kivio owns only `profiles/kivio`; native credentials and sessions stay in DSH_HOME.
//! The profile composes version-matched official preset declarations and the stdio bridge.

use crate::external_agents::spawn::CliProcessOutput;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::provider_profile::write_private_atomic;
use serde_yaml::{Mapping, Value as Yaml};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::proc::NoConsoleWindow;
use crate::settings::ExternalCliProvider;

/// Kivio 私有 profile 名。与用户的 `web` / `tui` 并存。
pub const KIVIO_PROFILE: &str = "kivio";

pub(super) const PROFILE_BUNDLE: &str = "./kivio-profile";
const PATCH_FILE: &str = "cordis.patch.yml";
const LEGACY_HEADER: &str = "# Kivio-managed profile.";
const BRIDGE_FILENAME: &str = "kivio-dsh-bridge.mjs";
const BRIDGE_SOURCE: &str = include_str!("../../resources/dsh/kivio-dsh-bridge.mjs");

/// Bridge 复用官方 server/transport；dsh core API 由运行中的 profile 提供，避免安装第二份 core。
///
/// SDK 与声明式 preset 必须安装当前 core 的精确版本。
const REQUIRED_PACKAGES: &[&str] = &[
    "@deepseek-ai/dsh-sdk-jsonrpc-server",
    "@deepseek-ai/dsh-sdk-protocol",
    "@deepseek-ai/dsh-web-app",
    "@deepseek-ai/dsh-mcp-client",
];

/// Remove the retired directory-discovery plugin from existing profiles.
const PROFILE_PRESET_PACKAGE: &str = "@deepseek-ai/dsh-agent-presets";

/// pnpm 在 profile 目录里读这份，盖过用户家目录 `.npmrc` 的淘宝镜像。
const PROFILE_NPMRC: &str = "registry=https://registry.npmjs.org/\n\
@deepseek-ai:registry=https://registry.npmjs.org/\n";

/// dsh web 把这些 host 平面工具关掉，改由 agent preset 按会话组装。
const HOST_PLANE_TOOL_IDS: &[&str] = &[
    "tool-bash",
    "tool-pwsh",
    "tool-jobs",
    "tool-fs",
    "tool-fs-search",
    "skill-filesystem",
    "tool-skill",
    "command-goal",
    "tool-goal",
    "plan-mode",
    "compaction-basic",
    "command-compact",
    "tool-result-pruner",
    "tool-subagent-control",
    "tool-subagent",
    "tool-subagent-fork",
    "tool-workflow",
    "agent-instructions",
    "tool-todo",
    "tool-web",
];

/// `$DSH_HOME`，未设时 `~/.dsh`（与上游 `resolveDshHome` 同序）。
pub(crate) fn dsh_home() -> Option<PathBuf> {
    super::provider_profile::effective_dsh_home(
        &super::overrides::agent_config("dsh").unwrap_or_default(),
    )
    .filter(|path| path.is_absolute())
}

/// `$DSH_HOME/profiles/kivio/`。
pub fn profile_dir() -> Option<PathBuf> {
    dsh_home().map(|home| home.join("profiles").join(KIVIO_PROFILE))
}

const USER_PRESET_DIR: &str = ".agent-presets";
const COMPOSITION_FILE: &str = "agent.cordis.yml";
const METADATA_FILE: &str = "preset.yml";
const BUILTIN_PRESET_IDS: &[&str] = &["standard", "ptc", "minimal", "cordis"];

/// 底栏胶囊里一条用户 preset（官方四档由前端写死，不走这条）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DshAgentPresetOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 兼容旧版官方 web 存在 `$DSH_HOME/.agent-presets` 的自定义 preset。
pub fn user_preset_root() -> Option<PathBuf> {
    dsh_home().map(|home| home.join(USER_PRESET_DIR))
}

/// 列出用户根目录里可加载的自定义 Agent preset（不含官方四档）。
pub fn list_user_agent_presets() -> Vec<DshAgentPresetOption> {
    user_preset_root()
        .map(|root| list_user_agent_presets_in(&root))
        .unwrap_or_default()
}

pub fn list_user_agent_presets_in(root: &Path) -> Vec<DshAgentPresetOption> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(id) = name.to_str().map(str::trim) else {
            continue;
        };
        if !is_valid_preset_id(id) || is_builtin_preset_id(id) || !is_loadable_preset_dir(&path) {
            continue;
        }
        let (label, description) = read_preset_metadata(&path);
        out.push(DshAgentPresetOption {
            id: id.to_string(),
            label: label.unwrap_or_else(|| id.to_string()),
            description,
        });
    }
    out.sort_by(|left, right| left.id.cmp(&right.id));
    out
}

/// 合法 preset id；其它值（含空、未知自定义）回落 `standard`。
///
/// 自定义 id 必须同时满足官方目录名规则 `[a-z0-9][a-z0-9-]*`，并且
/// `$DSH_HOME/.agent-presets/<id>/agent.cordis.yml` 存在——否则写进 patch 会让
/// dsh 起不来，未知字符串也绝不能原样插进 YAML。
pub fn normalize_agent_preset(value: Option<&str>) -> String {
    resolve_agent_preset(value, user_preset_root().as_deref())
}

fn resolve_agent_preset(value: Option<&str>, user_root: Option<&Path>) -> String {
    let Some(id) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return "standard".to_string();
    };
    let id = if id == "code" { "ptc" } else { id };
    if is_builtin_preset_id(id) {
        return id.to_string();
    }
    if is_valid_preset_id(id)
        && user_root.is_some_and(|root| is_loadable_preset_dir(&root.join(id)))
    {
        return id.to_string();
    }
    "standard".to_string()
}

fn is_builtin_preset_id(id: &str) -> bool {
    id == "code" || BUILTIN_PRESET_IDS.contains(&id)
}

/// 官方 `copy()` 校验：`[a-z0-9][a-z0-9-]*`。顺带封顶长度，挡住 YAML 注入。
fn is_valid_preset_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 128 {
        return false;
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_loadable_preset_dir(dir: &Path) -> bool {
    dir.join(COMPOSITION_FILE).is_file()
}

fn read_preset_metadata(dir: &Path) -> (Option<String>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(dir.join(METADATA_FILE)) else {
        return (None, None);
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return (None, None);
    };
    let Some(map) = value.as_mapping() else {
        return (None, None);
    };
    let field = |key: &str| {
        map.get(serde_yaml::Value::String(key.to_string()))
            .and_then(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    (field("name"), field("description"))
}

#[tauri::command]
pub fn chat_dsh_list_agent_presets() -> Vec<DshAgentPresetOption> {
    list_user_agent_presets()
}

/// Profile bundle applied before DSH's editable profile and home patches.
fn render_patch() -> String {
    let mut out =
        String::from("- id: hmr\n  disabled: true\n- id: session-title-llm\n  disabled: true\n");
    for id in HOST_PLANE_TOOL_IDS {
        out.push_str(&format!("- id: {id}\n  disabled: true\n"));
    }
    out.push_str(
        r#"- insert:
    - id: agent-preset-registry
      name: '@deepseek-ai/dsh-agent-preset-registry'
      config:
        default: standard
    - id: subagent-model-selection-settings
      name: '@deepseek-ai/dsh-tool-subagent/model-selection-settings'
    - id: workspace
      name: '@deepseek-ai/dsh-workspace'
    - id: schedule
      name: '@deepseek-ai/dsh-schedule'
    - id: cordis-host-runner
      name: '@deepseek-ai/dsh-cordis-host-runner'
    - id: cordis-inspect-providers
      name: '@deepseek-ai/dsh-tool-cordis/host'
    - id: kivio-dsh-jsonrpc-bridge
      name: './kivio-dsh-bridge.mjs'
"#,
    );
    out
}

fn append_preset_declarations(
    out: &mut String,
    profile: &Path,
    user_root: &Path,
) -> Result<(), String> {
    let package = package_dir(profile, "@deepseek-ai/dsh-web-app").expect("scoped package");
    for id in BUILTIN_PRESET_IDS {
        let path = package.join("presets").join(format!("{id}.patch.yml"));
        let declaration = std::fs::read_to_string(&path)
            .map_err(|e| format!("读取 dsh preset {id} 失败：{e}"))?;
        out.push('\n');
        out.push_str(&declaration);
        out.push('\n');
    }
    for preset in list_user_agent_presets_in(user_root) {
        let path = user_root.join(&preset.id).join(COMPOSITION_FILE);
        let url = url::Url::from_file_path(&path)
            .map_err(|_| format!("dsh preset 路径必须为绝对路径：{}", path.display()))?;
        let mut declaration = serde_yaml::to_value(serde_json::json!({ "insert": [{
            "id": format!("kivio-preset-{}", preset.id),
            "name": "@deepseek-ai/dsh-agent-preset",
            "config": { "id": preset.id, "name": preset.label }
        }] }))
        .map_err(|e| e.to_string())?;
        let config = &mut declaration["insert"][0]["config"];
        // A native include outside the profile cannot resolve the CLI's bare
        // plugin packages. Register the same child rows in the profile scope.
        let composition = std::fs::read_to_string(&path)
            .ok()
            .and_then(|src| super::dsh_plugins::parse_cordis_yaml(&src).ok());
        let mut plugins = None;
        if let Some(Yaml::Sequence(mut rows)) = composition {
            let module_roots: Vec<_> = path
                .parent()
                .into_iter()
                .flat_map(Path::ancestors)
                .map(|dir| dir.join("node_modules"))
                .filter(|dir| dir.is_dir())
                .collect();
            if rebase_preset_rows(&mut rows, &url, &module_roots)? {
                plugins = Some(Yaml::Sequence(rows));
            }
        }
        // Preserve Node's package exports and imports when the preset supplies
        // a dependency. A malformed composition also keeps its native diagnostic.
        config["plugins"] = match plugins {
            Some(rows) => rows,
            None => serde_yaml::to_value(serde_json::json!([{ "id": "composition",
                "name": "@deepseek-ai/cordis-plugin-include", "config": { "path": url.as_str() } }]))
                .map_err(|e| e.to_string())?,
        };
        out.push('\n');
        out.push_str(&super::dsh_plugins::render_cordis_yaml(&Yaml::Sequence(
            vec![declaration],
        ))?);
    }
    Ok(())
}

fn rebase_preset_rows(
    rows: &mut [Yaml],
    source: &url::Url,
    module_roots: &[PathBuf],
) -> Result<bool, String> {
    for row in rows {
        let name = row["name"].as_str().unwrap_or("");
        if name.starts_with('#') {
            return Ok(false);
        }
        if name.starts_with("./") || name.starts_with("../") {
            row["name"] = source
                .join(name)
                .map_err(|e| e.to_string())?
                .as_str()
                .into();
        } else if !name.is_empty() && !name.contains(':') && !name.starts_with('/') {
            let mut parts = name.split('/');
            let mut package = PathBuf::from(parts.next().unwrap_or(""));
            if name.starts_with('@') {
                if let Some(part) = parts.next() {
                    package.push(part);
                }
            }
            if module_roots
                .iter()
                .any(|root| root.join(&package).join("package.json").is_file())
            {
                return Ok(false);
            }
        }
        let name = row["name"].as_str().unwrap_or("");
        if matches!(name, "cordis:group" | "@deepseek-ai/cordis-plugin-group")
            || row["group"].as_bool() == Some(true)
        {
            if let Some(children) = row.get_mut("config").and_then(Yaml::as_sequence_mut) {
                if !rebase_preset_rows(children, source, module_roots)? {
                    return Ok(false);
                }
            }
        } else if name == "@deepseek-ai/cordis-plugin-include" {
            if let Some(path) = row["config"]["path"].as_str() {
                row["config"]["path"] = source
                    .join(path)
                    .map_err(|e| e.to_string())?
                    .as_str()
                    .into();
            }
        }
    }
    Ok(true)
}

#[derive(Debug)]
struct ActiveDshProvider {
    route: String,
    default_model: String,
    config: Value,
}

fn parse_active_provider(provider: &ExternalCliProvider) -> Result<ActiveDshProvider, String> {
    let route = provider.native_provider_id.trim();
    if route.is_empty()
        || route.len() > 64
        || !route.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
        || !route
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !route
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
        return Err(
            "dsh 供应商 ID 只能使用小写字母、数字、点、下划线和连字符，且不能以符号开头或结尾"
                .to_string(),
        );
    }
    let config: Value = serde_json::from_str(&provider.config_json)
        .map_err(|err| format!("dsh 供应商 {} 的 configJson 无效：{err}", provider.name))?;
    let object = config
        .as_object()
        .ok_or_else(|| format!("dsh 供应商 {} 的 configJson 必须是对象", provider.name))?;
    if object.contains_key("apiKey") || object.contains_key("api_key") {
        return Err(format!(
            "dsh 供应商 {} 不能把 API Key 写进 configJson；请使用 apiKeyEnv",
            provider.name
        ));
    }
    let api_key_env = object
        .get("apiKeyEnv")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("dsh 供应商 {} 缺少 apiKeyEnv", provider.name))?;
    if !provider
        .env
        .iter()
        .any(|pair| pair.key == api_key_env && !pair.value.trim().is_empty())
    {
        return Err(format!(
            "dsh 供应商 {} 缺少环境变量 {api_key_env} 的 API Key",
            provider.name
        ));
    }
    for field in ["api", "baseURL"] {
        if !object
            .get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(format!("dsh 供应商 {} 缺少 {field}", provider.name));
        }
    }
    let models = object
        .get("models")
        .and_then(Value::as_array)
        .filter(|models| !models.is_empty())
        .ok_or_else(|| format!("dsh 供应商 {} 至少需要一个模型", provider.name))?;
    let default_model = provider.default_model.trim();
    if default_model.is_empty()
        || !models.iter().any(|model| {
            model.get("id").and_then(Value::as_str).map(str::trim) == Some(default_model)
        })
    {
        return Err(format!(
            "dsh 供应商 {} 的默认模型不在模型列表中",
            provider.name
        ));
    }
    Ok(ActiveDshProvider {
        route: route.to_string(),
        default_model: default_model.to_string(),
        config,
    })
}

/// Cordis replaces the whole row config. Start from its effective native config
/// so a per-session selection does not remove the user's routes or options.
fn render_launch_patch(
    reasoning: Option<&str>,
    preset: Option<&str>,
    providers: &[ExternalCliProvider],
    native: &Mapping,
) -> Result<String, String> {
    let mut rows = Vec::new();
    let mut config_for = |id: &str, key: &str, value: Yaml| -> Result<(), String> {
        let mut config = match native.get(id) {
            None => Mapping::new(),
            Some(value) => value
                .as_mapping()
                .cloned()
                .ok_or_else(|| format!("dsh {id} 配置必须是对象"))?,
        };
        config.insert(key.into(), value);
        rows.push(Yaml::Mapping(Mapping::from_iter([
            (Yaml::from("id"), Yaml::from(id)),
            (Yaml::from("config"), Yaml::Mapping(config)),
        ])));
        Ok(())
    };
    if let Some(effort) = normalize_reasoning_effort(reasoning) {
        config_for("llm-deepseek", "reasoningEffort", effort.into())?;
    }
    if let Some(preset) = preset {
        config_for(
            "agent-preset-registry",
            "default",
            normalize_agent_preset(Some(preset)).into(),
        )?;
    }
    let mut routes = std::collections::HashSet::new();
    let mut merged = match native
        .get("llm-pi-ai")
        .and_then(|config| config.get("providers"))
    {
        None => Mapping::new(),
        Some(value) => value
            .as_mapping()
            .cloned()
            .ok_or_else(|| "dsh llm-pi-ai.providers 必须是对象".to_owned())?,
    };
    for provider in providers.iter().filter(|provider| !provider.disabled) {
        let active = parse_active_provider(provider)?;
        if !routes.insert(active.route.clone()) {
            return Err(format!("dsh 供应商路由重复：{}", active.route));
        }
        let mut config = active.config;
        if let Some(object) = config.as_object_mut() {
            if let Some(effort) = normalize_pi_reasoning_effort(reasoning) {
                object.insert("reasoning".into(), Value::String(effort.into()));
            } else {
                object.remove("reasoning");
            }
        }
        merged.insert(
            active.route.into(),
            serde_yaml::to_value(config).map_err(|e| e.to_string())?,
        );
    }
    if !routes.is_empty() {
        config_for("llm-pi-ai", "providers", Yaml::Mapping(merged))?;
    }
    if rows.is_empty() {
        return Ok(String::new());
    }
    super::dsh_plugins::render_cordis_yaml(&Yaml::Sequence(rows))
}

pub fn active_provider_default_route() -> Result<Option<(String, String)>, String> {
    let Some(provider) = crate::external_agents::overrides::active_provider("dsh") else {
        return Ok(None);
    };
    let active = parse_active_provider(&provider)?;
    Ok(Some((active.route, active.default_model)))
}

/// 空 / `"default"` / 未知值视为「不指定」，与 `types::normalize_model` 同语义。
///
/// 只认 `defs/dsh.rs` 的合法档位 `off|low|high|max`。其它字符串（含换行）绝不能写进
/// 共享的 `cordis.patch.yml`：那是顶层 YAML 数组，插一行就能重开 host-plane 工具。
fn normalize_reasoning_effort(value: Option<&str>) -> Option<&'static str> {
    match value.map(str::trim) {
        Some("off") => Some("off"),
        Some("low") => Some("low"),
        Some("high") => Some("high"),
        Some("max") => Some("max"),
        _ => None,
    }
}

/// `llm-pi-ai` 供应商级 `reasoning`：官方档位表 `off|minimal|low|medium|high|xhigh|max`。
/// 未知值整段省略，避免把任意字符串写进共享 patch。
fn normalize_pi_reasoning_effort(value: Option<&str>) -> Option<&'static str> {
    match value.map(str::trim) {
        Some("off") => Some("off"),
        Some("minimal") => Some("minimal"),
        Some("low") => Some("low"),
        Some("medium") => Some("medium"),
        Some("high") => Some("high"),
        Some("xhigh") => Some("xhigh"),
        Some("max") => Some("max"),
        _ => None,
    }
}

fn package_dir(dir: &Path, package: &str) -> Option<PathBuf> {
    let relative = package.trim_start_matches('@');
    let (scope, name) = relative.split_once('/')?;
    Some(
        dir.join("node_modules")
            .join(format!("@{scope}"))
            .join(name),
    )
}

fn installed_package_version(dir: &Path, package: &str) -> Option<String> {
    let raw = std::fs::read_to_string(package_dir(dir, package)?.join("package.json")).ok()?;
    serde_json::from_str::<Value>(&raw)
        .ok()?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

/// SDK and official preset declarations must match the installed core.
fn packages_match_core(dir: &Path, core_version: &str) -> bool {
    REQUIRED_PACKAGES
        .iter()
        .all(|package| installed_package_version(dir, package).as_deref() == Some(core_version))
}

fn versioned_package_specs(core_version: &str) -> Vec<String> {
    REQUIRED_PACKAGES
        .iter()
        .map(|package| format!("{package}@{core_version}"))
        .collect()
}

async fn dsh_core_version(bin: &Path) -> Result<String, String> {
    if let Some(version) = crate::external_agents::spawn::cached_cli_version(bin)
        .as_deref()
        .and_then(crate::external_agents::installer::extract_semver)
    {
        return Ok(version);
    }
    let output = crate::external_agents::spawn::cli_probe_command(bin)
        .arg("--version")
        .group_output()
        .await
        .map_err(|err| format!("无法读取 dsh 版本：{err}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    crate::external_agents::installer::extract_semver(&stdout)
        .or_else(|| crate::external_agents::installer::extract_semver(&stderr))
        .ok_or_else(|| "无法确定 dsh core 版本，不能安全安装匹配的 profile SDK".to_string())
}

/// Install the matching SDK, prepare the shared profile, and return the
/// invocation's private overlay. Its owner keeps it until the child exits.
pub async fn ensure_profile_ready(
    bin: &Path,
    reasoning: Option<&str>,
    preset: Option<&str>,
) -> Result<Option<tempfile::TempPath>, String> {
    let home = dsh_home().ok_or_else(|| "无法定位 dsh 主目录（$DSH_HOME / ~/.dsh）".to_string())?;
    let dir = home.join("profiles").join(KIVIO_PROFILE);

    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 dsh profile 目录失败：{e}"))?;
    write_generated_file(&dir.join(".npmrc"), PROFILE_NPMRC)?;

    let core_version = dsh_core_version(bin).await?;
    if !packages_match_core(&dir, &core_version) {
        install_packages(bin, &core_version, &home).await?;
    }
    let config = crate::external_agents::overrides::agent_config("dsh").unwrap_or_default();
    write_profile_bundle(&home, &dir, &config.providers)?;
    if preset.is_none()
        && normalize_reasoning_effort(reasoning).is_none()
        && config.providers.iter().all(|provider| provider.disabled)
    {
        return Ok(None);
    }
    let native = super::dsh_plugins::read_profile_settings(&home)?;
    let overlay = render_launch_patch(reasoning, preset, &config.providers, &native)?;
    let mut file = tempfile::Builder::new()
        .prefix("kivio-launch-")
        .suffix(".yml")
        .tempfile_in(&dir)
        .map_err(|e| format!("创建 dsh 启动配置失败：{e}"))?;
    file.write_all(overlay.as_bytes())
        .map_err(|e| format!("写入 dsh 启动配置失败：{e}"))?;
    Ok(Some(file.into_temp_path()))
}

fn write_generated_file(path: &Path, content: &str) -> Result<(), String> {
    match std::fs::read(path) {
        Ok(bytes) if bytes == content.as_bytes() => Ok(()),
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("读取 {} 失败：{err}", path.display()))
        }
        _ => write_private_atomic(path, content),
    }
}

fn write_profile_bundle(
    home: &Path,
    dir: &Path,
    providers: &[ExternalCliProvider],
) -> Result<(), String> {
    let _guard = super::provider_profile::lock_native_config()?;
    let bundle = dir.join(PROFILE_BUNDLE);
    let mut patch = render_patch();
    append_preset_declarations(&mut patch, dir, &home.join(USER_PRESET_DIR))?;
    write_generated_file(&bundle.join(BRIDGE_FILENAME), BRIDGE_SOURCE)?;
    write_generated_file(&bundle.join(PATCH_FILE), &patch)?;
    write_generated_file(
        &bundle.join("package.json"),
        &serde_json::json!({
            "name": "kivio-dsh-composition", "version": env!("CARGO_PKG_VERSION"),
            "private": true, "dsh": { "bundle": { "patch": "./cordis.patch.yml" } }
        })
        .to_string(),
    )?;
    normalize_profile_manifest(dir)?;
    let path = dir.join(PATCH_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读取 dsh profile 配置失败：{e}")),
    };
    if let Some(native) = legacy_native_patch(&text, providers)? {
        write_private_atomic(&dir.join("kivio-pre-bundle.patch.yml"), &text)?;
        write_private_atomic(&path, &native)?;
    }
    super::dsh_plugins::preserve_legacy_launch_settings(home)
}

pub(super) fn looks_like_legacy_profile(text: &str) -> bool {
    text.starts_with(LEGACY_HEADER)
        || text.starts_with("# 由 Kivio 生成并维护")
        || (text.contains("kivio-dsh-jsonrpc-bridge") && text.contains(BRIDGE_FILENAME))
}

/// Older releases wrote generated and native rows into the same patch. The
/// native editor appends overrides or updates an existing row in place.
pub(crate) fn legacy_native_patch(
    text: &str,
    providers: &[ExternalCliProvider],
) -> Result<Option<String>, String> {
    if !looks_like_legacy_profile(text) {
        return Ok(None);
    }
    let doc = super::dsh_plugins::parse_cordis_yaml(text)?;
    let rows = doc
        .as_sequence()
        .ok_or("旧版 Kivio profile 必须是 YAML 数组")?;
    let has_bridge = rows
        .iter()
        .filter_map(|row| row.get("insert").and_then(Yaml::as_sequence))
        .flatten()
        .any(|entry| {
            entry["id"].as_str() == Some("kivio-dsh-jsonrpc-bridge")
                && entry["name"].as_str() == Some("./kivio-dsh-bridge.mjs")
        });
    if !has_bridge && !text.starts_with(LEGACY_HEADER) && !text.starts_with("# 由 Kivio 生成并维护")
    {
        return Ok(None);
    }
    // Keep generated block-style rows verbatim when possible. An editor can
    // also save the sequence in flow style; fall back to its parsed rows.
    let mut starts: Vec<usize> = text.match_indices("\n- ").map(|(i, _)| i + 1).collect();
    if text.starts_with("- ") {
        starts.insert(0, 0);
    }
    let raw_rows = starts.len() == rows.len();
    starts.push(text.len());
    let mut native = String::new();
    let mut cleared_pi = false;
    let mut cleared_effort = false;
    let mut generated_insert_done = false;
    for (index, row) in rows.iter().enumerate() {
        let mut kept = row.clone();
        let id = row.get("id").and_then(Yaml::as_str).unwrap_or("");
        if let Some(inserts) = kept.get_mut("insert").and_then(Yaml::as_sequence_mut) {
            inserts.retain(|entry| {
                let id = entry["id"].as_str().unwrap_or("");
                let name = entry["name"].as_str().unwrap_or("");
                let shipped = name == "@deepseek-ai/dsh-agent-preset"
                    && (id.strip_prefix("preset-").is_some_and(is_builtin_preset_id)
                        || id.starts_with("kivio-preset-"));
                let host = matches!(
                    (id, name),
                    ("kivio-dsh-jsonrpc-bridge", "./kivio-dsh-bridge.mjs")
                        | ("agent-presets", "@deepseek-ai/dsh-agent-presets")
                        | (
                            "agent-preset-registry",
                            "@deepseek-ai/dsh-agent-preset-registry"
                        )
                        | (
                            "subagent-model-selection-settings",
                            "@deepseek-ai/dsh-tool-subagent/model-selection-settings"
                        )
                        | ("workspace", "@deepseek-ai/dsh-workspace")
                        | ("schedule", "@deepseek-ai/dsh-schedule")
                        | ("cordis-host-runner", "@deepseek-ai/dsh-cordis-host-runner")
                        | (
                            "cordis-inspect-providers",
                            "@deepseek-ai/dsh-tool-cordis/host"
                        )
                );
                generated_insert_done |= shipped;
                !(host || shipped)
            });
            if inserts.is_empty() {
                continue;
            }
        } else if row.get("name").is_none() {
            if row.as_mapping().is_some_and(|map| map.len() == 2)
                && row["disabled"].as_bool() == Some(true)
                && (HOST_PLANE_TOOL_IDS.contains(&id)
                    || matches!(
                        id,
                        "hmr"
                            | "session-title-llm"
                            | "tool-subagent-list-agents"
                            | "workflow-worker-thread"
                            | "tool-ralph"
                    ))
            {
                continue;
            }
            let field = match id {
                "llm-deepseek" if !cleared_effort && !generated_insert_done => {
                    cleared_effort = true;
                    Some("reasoningEffort")
                }
                "llm-pi-ai" if !cleared_pi && !generated_insert_done => {
                    cleared_pi = true;
                    if let Some(routes) = kept
                        .get_mut("config")
                        .and_then(|config| config.get_mut("providers"))
                        .and_then(Yaml::as_mapping_mut)
                    {
                        for provider in providers {
                            routes.remove(provider.native_provider_id.trim());
                        }
                        routes.is_empty().then_some("providers")
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let (Some(field), Some(config)) =
                (field, kept.get_mut("config").and_then(Yaml::as_mapping_mut))
            {
                config.remove(field);
                if config.is_empty() {
                    let map = kept.as_mapping_mut().expect("row has config");
                    map.remove("config");
                    if map.len() == 1 {
                        continue;
                    }
                }
            }
        }
        if raw_rows && &kept == row {
            native.push_str(&text[starts[index]..starts[index + 1]]);
        } else {
            native.push_str(&super::dsh_plugins::render_cordis_yaml(&Yaml::Sequence(
                vec![kept],
            ))?);
        }
        if !native.ends_with('\n') {
            native.push('\n');
        }
    }
    if native.is_empty() {
        native.push_str("[]\n");
    }
    Ok(Some(native))
}

fn normalize_profile_manifest(dir: &Path) -> Result<(), String> {
    let package_json = dir.join("package.json");
    let raw = std::fs::read_to_string(&package_json)
        .map_err(|e| format!("读取 dsh profile package.json 失败：{e}"))?;
    let mut value: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("解析 dsh profile package.json 失败：{e}"))?;
    let manifest = value
        .as_object_mut()
        .ok_or_else(|| "dsh profile package.json 必须是 JSON 对象".to_string())?;
    // dsh 0.1.5 的默认插件清单扩展会在首轮请求前读取活动插件的
    // package.json；没有 version 会以 REQUEST_EXTENSION 失败。
    let version = env!("CARGO_PKG_VERSION");
    let version_changed = manifest.get("version").and_then(Value::as_str) != Some(version);
    if version_changed {
        manifest.insert("version".to_string(), Value::String(version.to_string()));
    }
    let removed_preset = manifest
        .get_mut("dependencies")
        .and_then(Value::as_object_mut)
        .is_some_and(|deps| deps.remove(PROFILE_PRESET_PACKAGE).is_some());
    let mut profile = &mut value;
    for key in ["dsh", "profile"] {
        profile = profile
            .as_object_mut()
            .ok_or_else(|| "dsh profile 元数据必须是 JSON 对象".to_string())?
            .entry(key)
            .or_insert_with(|| serde_json::json!({}));
    }
    let bundles = profile
        .as_object_mut()
        .ok_or_else(|| "dsh profile 元数据必须是 JSON 对象".to_string())?
        .entry("bundles")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or_else(|| "dsh profile bundles 必须是数组".to_string())?;
    let previous_bundles = bundles.clone();
    // web-app supplies preset assets; its browser application must not boot here.
    bundles.retain(|bundle| bundle.as_str() != Some("@deepseek-ai/dsh-web-app"));
    if !bundles
        .iter()
        .any(|bundle| bundle.as_str() == Some("@deepseek-ai/dsh-base"))
    {
        bundles.insert(0, serde_json::json!("@deepseek-ai/dsh-base"));
    }
    if !bundles
        .iter()
        .any(|bundle| bundle.as_str() == Some(PROFILE_BUNDLE))
    {
        let base = bundles
            .iter()
            .position(|bundle| bundle.as_str() == Some("@deepseek-ai/dsh-base"))
            .expect("base bundle present");
        bundles.insert(base + 1, serde_json::json!(PROFILE_BUNDLE));
    }
    let bundles_changed = *bundles != previous_bundles;
    if version_changed || removed_preset || bundles_changed {
        let next = serde_json::to_string_pretty(&value)
            .map_err(|e| format!("序列化 dsh profile package.json 失败：{e}"))?;
        write_private_atomic(&package_json, &format!("{next}\n"))?;
    }
    let relative = PROFILE_PRESET_PACKAGE.trim_start_matches('@');
    if let Some((scope, name)) = relative.split_once('/') {
        let _ = std::fs::remove_dir_all(
            dir.join("node_modules")
                .join(format!("@{scope}"))
                .join(name),
        );
    }
    Ok(())
}

/// `dsh plugin --profile kivio add <pkgs>`。
///
/// 用探测到的那个绝对二进制（与跑轮次的是同一个），不走 PATH 再查一次。
fn configure_profile_install(
    command: &mut tokio::process::Command,
    providers: &[ExternalCliProvider],
    home: &Path,
) {
    for provider in providers.iter().filter(|provider| !provider.disabled) {
        for env in &provider.env {
            command.env_remove(&env.key);
        }
    }
    command.env("DSH_HOME", home);
}

async fn install_packages(bin: &Path, core_version: &str, home: &Path) -> Result<(), String> {
    use tokio::io::AsyncReadExt;

    crate::external_agents::installer::expose_node_on_path();

    // Let dsh choose its package manager: Desktop carries its own pnpm.
    // Installation never needs model credentials. Build a clean command so pnpm and dependency
    // lifecycle scripts cannot inherit the active Kivio provider API key.
    let mut command = tokio::process::Command::new(bin);
    crate::external_agents::spawn::strip_parent_session_env(&mut command);
    crate::external_agents::installer::pin_official_npm_registry(&mut command);
    let config = crate::external_agents::overrides::agent_config("dsh").unwrap_or_default();
    configure_profile_install(&mut command, &config.providers, home);
    let package_specs = versioned_package_specs(core_version);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    command
        .arg("plugin")
        .arg("--profile")
        .arg(KIVIO_PROFILE)
        .arg("add")
        .args(&package_specs)
        // Official preset dependency; keep other dependency builds unapproved.
        .arg("--allow-build=koffi@3.1.1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .no_console_window();

    let mut child = crate::external_agents::spawn::AgentChild::spawn(&mut command)
        .map_err(|e| format!("无法执行 dsh plugin：{e}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = stdout {
            let _ = pipe.read_to_end(&mut bytes).await;
        }
        bytes
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = stderr {
            let _ = pipe.read_to_end(&mut bytes).await;
        }
        bytes
    });

    let status = match tokio::time::timeout(std::time::Duration::from_secs(180), child.wait()).await
    {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => {
            crate::external_agents::spawn::kill_agent_process_tree(&mut child);
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err(format!("等待 dsh plugin 失败：{err}"));
        }
        Err(_) => {
            crate::external_agents::spawn::kill_agent_process_tree(&mut child);
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            return Err("安装 dsh 插件超时（180s）".to_string());
        }
    };
    let _stdout = stdout_task.await.unwrap_or_default();
    let stderr = stderr_task.await.unwrap_or_default();

    if status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&stderr);
    let tail = stderr.trim();
    if pnpm_missing_in_output(status.code(), tail) {
        return Err(
            "dsh 需要 pnpm 来安装 profile 插件，但 PATH 上没有找到 pnpm。\
             请到设置 → 本地 CLI Agent → DeepSeek Harness 点一次安装，或先运行 npm install -g pnpm 后重试。"
                .to_string(),
        );
    }
    Err(crate::external_agents::spawn::fold_stderr(
        "安装 dsh 插件失败".to_string(),
        tail,
    ))
}

fn pnpm_missing_in_output(status_code: Option<i32>, tail: &str) -> bool {
    status_code == Some(127)
        || tail.contains("pnpm not found")
        || tail.contains("pnpm failed")
        || tail.contains("'pnpm'")
        || tail.contains("\"pnpm\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay_provider() -> ExternalCliProvider {
        ExternalCliProvider {
            id: "p-relay".to_string(),
            name: "Relay".to_string(),
            native_provider_id: "relay-one".to_string(),
            default_model: "gpt-test".to_string(),
            config_json: serde_json::json!({
                "displayName": "Relay",
                "apiKeyEnv": "KIVIO_DSH_RELAY_ONE_API_KEY",
                "api": "openai-responses",
                "baseURL": "https://relay.example/v1",
                "models": [{
                    "id": "gpt-test",
                    "name": "GPT Test",
                    "contextWindow": 256000,
                    "maxTokens": 32768
                }]
            })
            .to_string(),
            env: vec![crate::settings::CliEnvVar {
                key: "KIVIO_DSH_RELAY_ONE_API_KEY".to_string(),
                value: "sk-secret".to_string(),
            }],
            ..Default::default()
        }
    }

    /// 装配 bridge 的 insert 必须在，否则 `initialize` 没有 resumable server 应答。
    #[test]
    fn patch_always_mounts_the_kivio_bridge() {
        let yml = render_patch();
        assert!(yml.contains("- insert:"));
        assert!(yml.contains("name: './kivio-dsh-bridge.mjs'"));
        assert!(yml.contains("id: kivio-dsh-jsonrpc-bridge"));
        assert!(yml.contains("name: '@deepseek-ai/dsh-agent-preset-registry'"));
        assert!(yml.contains("default: standard"));
        assert!(yml.contains("name: '@deepseek-ai/dsh-tool-subagent/model-selection-settings'"));
        assert!(yml.contains("name: '@deepseek-ai/dsh-workspace'"));
        assert!(yml.contains("- id: tool-bash\n  disabled: true"));
    }

    #[test]
    fn bridge_packages_are_pinned_to_the_dsh_core_version() {
        assert_eq!(
            versioned_package_specs("0.1.5-rc.2"),
            vec![
                "@deepseek-ai/dsh-sdk-jsonrpc-server@0.1.5-rc.2",
                "@deepseek-ai/dsh-sdk-protocol@0.1.5-rc.2",
                "@deepseek-ai/dsh-web-app@0.1.5-rc.2",
                "@deepseek-ai/dsh-mcp-client@0.1.5-rc.2",
            ]
        );
    }

    #[test]
    fn installed_bridge_versions_must_match_the_core() {
        let dir = tempfile::tempdir().unwrap();
        for package in REQUIRED_PACKAGES {
            let package_dir = package_dir(dir.path(), package).unwrap();
            std::fs::create_dir_all(&package_dir).unwrap();
            std::fs::write(
                package_dir.join("package.json"),
                r#"{"version":"0.1.5-rc.2"}"#,
            )
            .unwrap();
        }
        assert!(packages_match_core(dir.path(), "0.1.5-rc.2"));
        assert!(!packages_match_core(dir.path(), "0.1.6-alpha.2"));

        let protocol = package_dir(dir.path(), "@deepseek-ai/dsh-sdk-protocol").unwrap();
        std::fs::write(protocol.join("package.json"), r#"{"version":"0.0.1-rc.1"}"#).unwrap();
        assert!(!packages_match_core(dir.path(), "0.1.5-rc.2"));
    }

    #[test]
    fn patch_reuses_base_storage_and_mounts_the_workspace_registry() {
        let yml = render_patch();
        assert!(yml.contains("name: '@deepseek-ai/dsh-workspace'"));
        assert!(!yml.contains("name: '@deepseek-ai/dsh-storage'"));
        assert!(!yml.contains("id: tool-str-replace-editor"));
    }

    #[test]
    fn profile_composes_shipped_and_custom_presets_without_evaluating_expressions() {
        let dir = tempfile::tempdir().unwrap();
        let presets = package_dir(dir.path(), "@deepseek-ai/dsh-web-app")
            .unwrap()
            .join("presets");
        std::fs::create_dir_all(&presets).unwrap();
        for id in BUILTIN_PRESET_IDS {
            std::fs::write(presets.join(format!("{id}.patch.yml")), format!("- insert:\n    - id: preset-{id}\n      disabled: !!js process.platform === 'win32'\n")).unwrap();
        }
        let users = dir.path().join("user presets");
        write_user_preset(&users, "my-reviewer", Some("name: Reviewer"), true);
        std::fs::write(users.join("my-reviewer").join(COMPOSITION_FILE),
            "- name: '@deepseek-ai/dsh-persona'\n- name: ./review.mjs\n  disabled: !!js process.platform === 'win32'\n").unwrap();
        let mut patch = render_patch();
        append_preset_declarations(&mut patch, dir.path(), &users).unwrap();
        assert!(patch.contains("!!js process.platform === 'win32'"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&patch).unwrap();
        let rows = parsed.as_sequence().unwrap();
        let custom = rows.last().unwrap();
        assert_eq!(
            custom["insert"][0]["config"]["id"].as_str(),
            Some("my-reviewer")
        );
        let plugins = &custom["insert"][0]["config"]["plugins"];
        assert_eq!(
            plugins[0]["name"].as_str(),
            Some("@deepseek-ai/dsh-persona")
        );
        let path = plugins[1]["name"].as_str().unwrap();
        assert_eq!(
            url::Url::parse(path).unwrap().to_file_path().unwrap(),
            users.join("my-reviewer/review.mjs")
        );
        let local = users.join("my-reviewer/node_modules/@deepseek-ai/dsh-persona");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("package.json"), "{}").unwrap();
        let mut patch = String::new();
        append_preset_declarations(&mut patch, dir.path(), &users).unwrap();
        let parsed: Yaml = serde_yaml::from_str(&patch).unwrap();
        let local_config = &parsed.as_sequence().unwrap().last().unwrap()["insert"][0]["config"];
        let include_path = local_config["plugins"][0]["config"]["path"]
            .as_str()
            .unwrap();
        assert_eq!(
            url::Url::parse(include_path)
                .unwrap()
                .to_file_path()
                .unwrap(),
            users.join("my-reviewer").join(COMPOSITION_FILE)
        );
        std::fs::remove_file(presets.join("ptc.patch.yml")).unwrap();
        assert!(append_preset_declarations(&mut String::new(), dir.path(), &users).is_err());
    }

    #[test]
    fn preparing_profile_twice_preserves_native_settings_and_removes_legacy_managed_rows() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("profiles/kivio");
        let presets = package_dir(&dir, "@deepseek-ai/dsh-web-app")
            .unwrap()
            .join("presets");
        std::fs::create_dir_all(&presets).unwrap();
        for id in BUILTIN_PRESET_IDS {
            std::fs::write(presets.join(format!("{id}.patch.yml")), format!("- insert:\n    - id: preset-{id}\n      name: '@deepseek-ai/dsh-agent-preset'\n")).unwrap();
        }
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        // The native editor can reserialize the old generated rows without their banner.
        let legacy = "- id: hmr\n  disabled: true\n- id: llm-pi-ai\n  config:\n    providers:\n      relay-one: {}\n      native: {apiKeyEnv: NATIVE_KEY}\n- insert:\n    - id: kivio-dsh-jsonrpc-bridge\n      name: './kivio-dsh-bridge.mjs'\n- id: agent-loop\n  config: {maxParallelToolCalls: 3}\n- id: user-plugin\n  disabled: !!js process.platform === 'win32'\n";
        std::fs::write(dir.join(PATCH_FILE), legacy).unwrap();
        let provider = relay_provider();
        for _ in 0..2 {
            write_profile_bundle(home.path(), &dir, std::slice::from_ref(&provider)).unwrap();
            let native = std::fs::read_to_string(dir.join(PATCH_FILE)).unwrap();
            assert!(native.contains("apiKeyEnv: NATIVE_KEY"));
            assert!(native.contains("maxParallelToolCalls: 3"));
            assert!(native.contains("disabled: !!js process.platform === 'win32'"));
            assert!(!native.contains("relay-one"));
            assert!(!native.contains("kivio-dsh-jsonrpc-bridge"));
            assert_eq!(
                std::fs::read_to_string(dir.join("kivio-pre-bundle.patch.yml")).unwrap(),
                legacy
            );
        }
        let native = super::super::dsh_plugins::read_profile_settings(home.path()).unwrap();
        let launch = render_launch_patch(Some("high"), None, &[provider], &native).unwrap();
        let document: Yaml = serde_yaml::from_str(&launch).unwrap();
        let routes = &document
            .as_sequence()
            .unwrap()
            .iter()
            .find(|row| row["id"].as_str() == Some("llm-pi-ai"))
            .unwrap()["config"]["providers"];
        assert_eq!(routes["native"]["apiKeyEnv"].as_str(), Some("NATIVE_KEY"));
        assert_eq!(
            routes["relay-one"]["apiKeyEnv"].as_str(),
            Some("KIVIO_DSH_RELAY_ONE_API_KEY")
        );
    }

    #[test]
    fn bridge_source_owns_resume_cancel_and_parent_cleanup() {
        assert!(BRIDGE_SOURCE.contains("ctx.agents.resume"));
        assert!(BRIDGE_SOURCE.contains("agent.cancel({ kind: 'user' })"));
        assert!(BRIDGE_SOURCE.contains("process.ppid !== parentPid"));
        assert!(BRIDGE_SOURCE.contains("input.once('end', onInputClosed)"));
        assert!(BRIDGE_SOURCE.contains("DSH_AGENT_PRESET"));
        assert!(BRIDGE_SOURCE.contains("agentPresets"));
        assert!(BRIDGE_SOURCE.contains("ctx.on('user-questions/request'"));
        assert!(BRIDGE_SOURCE.contains("session/ask"));
        assert!(BRIDGE_SOURCE.contains("withExclusiveAgentCall"));
        let cancel_at = BRIDGE_SOURCE
            .find("async cancel(params)")
            .expect("cancel handler");
        assert!(
            BRIDGE_SOURCE[cancel_at..cancel_at.saturating_add(700)]
                .contains("withExclusiveAgentCall"),
            "session/cancel must join the prompt/steer exclusive queue",
        );
        assert!(BRIDGE_SOURCE.contains("agent.steer(message)"));
        assert!(BRIDGE_SOURCE.contains("'userQuestions'"));
        assert!(BRIDGE_SOURCE.contains("'attachments'"));
        assert!(BRIDGE_SOURCE.contains("'jobs'"));
        assert!(BRIDGE_SOURCE.contains("'subagents'"));
        assert!(BRIDGE_SOURCE.contains("saveImage"));
        assert!(BRIDGE_SOURCE.contains("session/commands"));
        assert!(BRIDGE_SOURCE.contains("session/stop-job"));
        assert!(BRIDGE_SOURCE.contains("commands.list"));
        assert!(BRIDGE_SOURCE.contains("executeCommand"));
        assert!(BRIDGE_SOURCE.contains("encodedCommandImages"));
        assert!(BRIDGE_SOURCE.contains("commandExecuteTakesImages"));
        assert!(BRIDGE_SOURCE.contains("jobs.kill"));
        assert!(BRIDGE_SOURCE.contains("subagents.interrupt"));
        assert!(BRIDGE_SOURCE.contains("workspaceRegistry"));
        assert!(BRIDGE_SOURCE.contains("attachSession"));
        assert!(BRIDGE_SOURCE.contains("shouldAttachWorkspace"));
        assert!(BRIDGE_SOURCE.contains("process.chdir(this.cwd)"));
        assert!(BRIDGE_SOURCE.contains("[kivio-dsh] workspace attach failed:"));
        assert!(!BRIDGE_SOURCE.contains("waitForProviderAdapter"));
        assert!(!BRIDGE_SOURCE.contains("@deepseek-ai/dsh-session"));
    }

    #[test]
    fn plugin_install_explicitly_removes_provider_key_env() {
        let provider = relay_provider();
        let mut command = tokio::process::Command::new("dsh");
        configure_profile_install(&mut command, &[provider], Path::new("test-home"));
        let key = std::ffi::OsStr::new("KIVIO_DSH_RELAY_ONE_API_KEY");
        let entry = command.as_std().get_envs().find(|(name, _)| *name == key);
        assert!(matches!(entry, Some((_, None))));
    }

    #[test]
    fn maintenance_review_install_uses_the_resolved_profile_home() {
        let temp = tempfile::tempdir().unwrap();
        let mut command = tokio::process::Command::new("dsh");
        configure_profile_install(&mut command, &[relay_provider()], temp.path());
        let value = command
            .as_std()
            .get_envs()
            .find_map(|(key, value)| (key == "DSH_HOME").then_some(value).flatten());
        assert_eq!(value, Some(temp.path().as_os_str()));
    }

    #[test]
    fn provider_patch_mounts_llm_pi_ai_without_serializing_the_api_key() {
        let provider = relay_provider();
        let yml = render_launch_patch(Some("high"), None, &[provider], &Mapping::new()).unwrap();
        assert!(yml.contains("- id: llm-pi-ai"));
        assert!(yml.contains("relay-one:"));
        assert!(yml.contains("apiKeyEnv: KIVIO_DSH_RELAY_ONE_API_KEY"));
        assert!(yml.contains("baseURL: https://relay.example/v1"));
        assert!(yml.contains("id: gpt-test"));
        assert!(yml.contains("reasoning: high"));
        assert!(!yml.contains("sk-secret"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yml).unwrap();
        assert!(parsed.is_sequence());
    }

    #[test]
    fn provider_patch_mounts_multiple_providers_together() {
        let first = relay_provider();
        let mut second = relay_provider();
        second.id = "relay-two".to_string();
        second.name = "Relay Two".to_string();
        second.native_provider_id = "relay-two".to_string();
        second.default_model = "claude-test".to_string();
        second.env = vec![crate::settings::CliEnvVar {
            key: "KIVIO_DSH_RELAY_TWO_API_KEY".to_string(),
            value: "sk-second".to_string(),
        }];
        second.config_json = second
            .config_json
            .replace("KIVIO_DSH_RELAY_ONE_API_KEY", "KIVIO_DSH_RELAY_TWO_API_KEY")
            .replace("gpt-test", "claude-test");

        let mut disabled = relay_provider();
        disabled.id = "relay-disabled".to_string();
        disabled.native_provider_id = "relay-disabled".to_string();
        disabled.disabled = true;

        let yml =
            render_launch_patch(None, None, &[first, second, disabled], &Mapping::new()).unwrap();
        assert!(yml.contains("relay-one:"));
        assert!(yml.contains("relay-two:"));
        assert!(!yml.contains("relay-disabled:"));
        assert!(yml.contains("id: gpt-test"));
        assert!(yml.contains("id: claude-test"));
        assert!(!yml.contains("sk-secret"));
        assert!(!yml.contains("sk-second"));
    }

    #[test]
    fn provider_patch_writes_pi_ai_reasoning_for_custom_efforts() {
        let provider = relay_provider();
        let yml =
            render_launch_patch(Some("low"), None, &[provider.clone()], &Mapping::new()).unwrap();
        assert!(yml.contains("reasoning: low"));
        assert!(yml.contains("reasoningEffort: low"));

        let yml = render_launch_patch(Some("minimal"), None, &[provider.clone()], &Mapping::new())
            .unwrap();
        assert!(yml.contains("reasoning: minimal"));
        assert!(!yml.contains("reasoningEffort:"));
        assert!(!yml.contains("- id: llm-deepseek"));

        let yml =
            render_launch_patch(Some("xhigh"), None, &[provider.clone()], &Mapping::new()).unwrap();
        assert!(yml.contains("reasoning: xhigh"));
        assert!(!yml.contains("reasoningEffort:"));

        let yml = render_launch_patch(Some("default"), None, &[provider], &Mapping::new()).unwrap();
        assert!(!yml.contains("reasoning:"));
    }

    #[test]
    fn provider_patch_rejects_missing_key_and_invalid_default_model() {
        let mut provider = relay_provider();
        provider.env.clear();
        assert!(
            render_launch_patch(None, None, &[provider], &Mapping::new())
                .unwrap_err()
                .contains("KIVIO_DSH_RELAY_ONE_API_KEY")
        );

        let mut provider = relay_provider();
        provider.default_model = "missing".to_string();
        assert!(
            render_launch_patch(None, None, &[provider], &Mapping::new())
                .unwrap_err()
                .contains("默认模型")
        );
    }

    /// 两条 disable 是钱和稳定性：标题 LLM 每轮一次白花的调用，hmr 是意外重连。
    #[test]
    fn patch_disables_hmr_and_title_llm() {
        let yml = render_patch();
        assert!(yml.contains("- id: hmr\n  disabled: true"));
        assert!(yml.contains("- id: session-title-llm\n  disabled: true"));
    }

    #[test]
    fn normalize_profile_manifest_removes_the_shadowing_preset() {
        let dir =
            std::env::temp_dir().join(format!("kivio-dsh-preset-strip-{}", std::process::id()));
        let pkg_dir = dir
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh-agent-presets");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(pkg_dir.join("package.json"), "{}").unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"dsh-profile-kivio","dependencies":{"@deepseek-ai/dsh-agent-presets":"0.0.1-rc.1","@deepseek-ai/dsh-sdk-protocol":"0.0.1-rc.1"}}"#,
        )
        .unwrap();

        normalize_profile_manifest(&dir).unwrap();

        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert!(value["dependencies"]
            .get("@deepseek-ai/dsh-agent-presets")
            .is_none());
        assert_eq!(
            value["dependencies"]["@deepseek-ai/dsh-sdk-protocol"],
            "0.0.1-rc.1"
        );
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert!(!pkg_dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn profile_manifest_without_legacy_preset_still_gets_a_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"dsh-profile-kivio","private":true,"dependencies":{"@deepseek-ai/dsh-sdk-protocol":"0.1.5-rc.2"},"dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app","custom-bundle"]}}}"#,
        )
        .unwrap();

        normalize_profile_manifest(dir.path()).unwrap();

        let value: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("package.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(
            value["dependencies"]["@deepseek-ai/dsh-sdk-protocol"],
            "0.1.5-rc.2"
        );
        assert_eq!(
            value["dsh"]["profile"]["bundles"],
            serde_json::json!(["@deepseek-ai/dsh-base", PROFILE_BUNDLE, "custom-bundle"])
        );
    }

    #[test]
    fn malformed_profile_metadata_is_rejected_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("package.json");
        for raw in [
            r#"{"dsh":false}"#,
            r#"{"dsh":{"profile":{"bundles":false}}}"#,
        ] {
            std::fs::write(&path, raw).unwrap();
            assert!(normalize_profile_manifest(dir.path()).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        }
    }

    #[test]
    fn normalize_agent_preset_accepts_builtins_and_rejects_unknown() {
        assert_eq!(normalize_agent_preset(None), "standard");
        assert_eq!(normalize_agent_preset(Some("")), "standard");
        assert_eq!(normalize_agent_preset(Some("default")), "standard");
        assert_eq!(normalize_agent_preset(Some("code")), "ptc");
        assert_eq!(normalize_agent_preset(Some("minimal")), "minimal");
        assert_eq!(normalize_agent_preset(Some("cordis")), "cordis");
        assert_eq!(normalize_agent_preset(Some("missing-custom")), "standard");
        assert_eq!(
            normalize_agent_preset(Some("x\n- id: injected-tool")),
            "standard"
        );
    }

    #[test]
    fn patch_writes_the_selected_agent_preset() {
        let yml = render_launch_patch(None, Some("minimal"), &[], &Mapping::new()).unwrap();
        assert!(yml.contains("id: agent-preset-registry"));
        assert!(yml.contains("default: minimal"));
        assert!(!yml.contains("default: standard"));
    }

    #[test]
    fn unknown_preset_is_not_interpolated_into_the_patch() {
        let yml = render_launch_patch(
            None,
            Some("x\n- id: injected-tool\n  disabled: false"),
            &[],
            &Mapping::new(),
        )
        .unwrap();
        assert!(yml.contains("default: standard"));
        assert!(!yml.contains("injected-tool"));
    }

    fn write_user_preset(root: &Path, id: &str, metadata: Option<&str>, composition: bool) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        if composition {
            std::fs::write(dir.join(COMPOSITION_FILE), "- name: tool-bash\n").unwrap();
        }
        if let Some(text) = metadata {
            std::fs::write(dir.join(METADATA_FILE), text).unwrap();
        }
    }

    #[test]
    fn lists_user_presets_and_normalizes_existing_custom_ids() {
        let root = std::env::temp_dir().join(format!("kivio-dsh-presets-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        write_user_preset(
            &root,
            "code-review",
            Some("name: 代码审查\ndescription: 只读评审\n"),
            true,
        );
        write_user_preset(&root, "standard", None, true);
        write_user_preset(&root, "empty-one", None, false);
        write_user_preset(&root, "Bad_ID", None, true);

        let listed = list_user_agent_presets_in(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "code-review");
        assert_eq!(listed[0].label, "代码审查");
        assert_eq!(listed[0].description.as_deref(), Some("只读评审"));

        assert_eq!(
            resolve_agent_preset(Some("code-review"), Some(&root)),
            "code-review"
        );
        assert_eq!(
            resolve_agent_preset(Some("missing"), Some(&root)),
            "standard"
        );
        assert_eq!(
            resolve_agent_preset(Some("Bad_ID"), Some(&root)),
            "standard"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn patch_writes_reasoning_effort_when_chosen() {
        let yml = render_launch_patch(Some("max"), None, &[], &Mapping::new()).unwrap();
        assert!(yml.contains("- id: llm-deepseek"));
        assert!(yml.contains("reasoningEffort: max"));
    }

    /// `default` 是 Kivio 侧的「不指定」哨兵，**不是** dsh 的合法档位值
    /// （schema 只认 `off|low|high|max`）。写进去整棵配置树会在启动时被拒。
    #[test]
    fn patch_omits_reasoning_for_default_and_blank() {
        for value in [None, Some(""), Some("   "), Some("default")] {
            let yml = render_launch_patch(value, None, &[], &Mapping::new()).unwrap();
            assert!(
                !yml.contains("reasoningEffort"),
                "reasoning={value:?} 不该写档位"
            );
            assert!(!yml.contains("llm-deepseek"));
        }
    }

    /// `off` 是合法档位（关思考），不能被当成「不指定」吞掉。
    #[test]
    fn off_is_a_real_effort_not_an_absence() {
        assert!(render_launch_patch(Some("off"), None, &[], &Mapping::new())
            .unwrap()
            .contains("reasoningEffort: off"));
        assert!(render_launch_patch(Some("low"), None, &[], &Mapping::new())
            .unwrap()
            .contains("reasoningEffort: low"));
    }

    /// 未知档位（含换行 / YAML 注入）必须整段省略，不能原样插进共享 patch。
    #[test]
    fn unknown_reasoning_is_omitted_not_interpolated() {
        for value in [
            Some("HIGH"),
            Some("none"),
            Some("high\n- id: injected-tool\n  disabled: false"),
            Some("off\n- insert:\n    - id: evil"),
        ] {
            let yml = render_launch_patch(value, None, &[], &Mapping::new()).unwrap();
            assert!(
                !yml.contains("reasoningEffort"),
                "reasoning={value:?} 不该写档位"
            );
            assert!(
                !yml.contains("injected-tool"),
                "注入条目不得进 patch：{value:?}"
            );
            assert!(!yml.contains("evil"), "注入条目不得进 patch：{value:?}");
        }
    }

    /// patch 必须是**顶层 YAML 数组**（上游对这个文件的要求就是 "a top-level YAML array of
    /// patch entries"，空文件或映射都会 fail loud）。仓库没有 YAML 解析依赖，所以这里做
    /// 结构检查而不是真解析：每个非空非注释的顶层行，要么是 `- ` 开头的条目，要么是缩进的
    /// 续行 —— 出现顶层 `key:` 就说明写成映射了。
    #[test]
    fn patch_is_a_top_level_sequence() {
        {
            let yml = render_patch();
            let mut entries = 0;
            for line in yml.lines() {
                if line.trim().is_empty() || line.trim_start().starts_with('#') {
                    continue;
                }
                if line.starts_with("- ") {
                    entries += 1;
                    continue;
                }
                assert!(
                    line.starts_with(' '),
                    "顶层只能是数组条目或其缩进续行，出现了 {line:?}"
                );
            }
            assert!(
                entries >= 3,
                "至少 hmr / title-llm / insert 三条，实际 {entries}"
            );
        }
    }

    #[test]
    fn profile_dir_lives_under_dsh_home() {
        if let Some(dir) = profile_dir() {
            let shown = dir.to_string_lossy().replace('\\', "/");
            assert!(shown.ends_with("profiles/kivio"), "got {shown}");
        }
    }

    #[test]
    fn windows_pnpm_not_on_path_is_recognized_even_when_cmd_garbles_the_rest() {
        let real = "dsh: initialized profile kivio at C:\\Users\\18758\\.dsh\\profiles\\kivio\n\
                    'pnpm' \u{fffd}\u{fffd}\u{fffd}\n\
                    dsh: pnpm failed in profile directory C:\\Users\\18758\\.dsh\\profiles\\kivio";
        assert!(pnpm_missing_in_output(None, real));
        assert!(pnpm_missing_in_output(Some(127), "pnpm not found on PATH"));
        assert!(!pnpm_missing_in_output(
            Some(1),
            "network timeout contacting registry"
        ));
    }

    #[test]
    fn profile_npmrc_pins_official_registry() {
        assert!(PROFILE_NPMRC.contains("registry=https://registry.npmjs.org/"));
        assert!(PROFILE_NPMRC.contains("@deepseek-ai:registry=https://registry.npmjs.org/"));
    }
}
