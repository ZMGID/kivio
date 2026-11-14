//! Kivio owns only `profiles/kivio`; native credentials and sessions stay in DSH_HOME.
//! The profile composes version-matched official preset declarations and the stdio bridge.

use crate::external_agents::spawn::CliProcessOutput;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::proc::NoConsoleWindow;
use crate::settings::ExternalCliProvider;

/// Kivio 私有 profile 名。与用户的 `web` / `tui` 并存。
pub const KIVIO_PROFILE: &str = "kivio";

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
    super::provider_profile::effective_dsh_path(
        &super::overrides::agent_config("dsh").unwrap_or_default(),
    )
    .and_then(|path| path.parent().map(Path::to_path_buf))
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

/// profile 的 `cordis.patch.yml` 内容。
///
/// `reasoning` 为 `None` / 空 / `"default"` 时**不写** `llm-deepseek` 那条 —— 让适配器用它
/// 自己的默认（实测 `defaultEffort: high`）。写一个 `default` 字面量会被 schema 拒（合法值
/// 只有 `off|low|high|max`），整棵树起不来。
fn render_patch(reasoning: Option<&str>, preset: Option<&str>) -> String {
    let mut out = String::from("# Kivio-managed profile.\n- id: hmr\n  disabled: true\n- id: session-title-llm\n  disabled: true\n");
    if let Some(effort) = normalize_reasoning_effort(reasoning) {
        out.push_str(&format!(
            "- id: llm-deepseek\n  config:\n    reasoningEffort: {effort}\n"
        ));
    }
    for id in HOST_PLANE_TOOL_IDS {
        out.push_str(&format!("- id: {id}\n  disabled: true\n"));
    }
    out.push_str(&format!(
        r#"- insert:
    - id: agent-preset-registry
      name: '@deepseek-ai/dsh-agent-preset-registry'
      config:
        default: {}
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
        normalize_agent_preset(preset)
    ));
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
        let declaration = serde_json::json!({ "insert": [{
            "id": format!("kivio-preset-{}", preset.id),
            "name": "@deepseek-ai/dsh-agent-preset",
            "config": {
                "id": preset.id, "name": preset.label,
                "plugins": [{ "id": "composition", "name": "@deepseek-ai/cordis-plugin-include",
                    "config": { "path": url.as_str() } }]
            }
        }] });
        out.push_str("\n- ");
        out.push_str(&declaration.to_string());
        out.push('\n');
    }
    Ok(())
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

fn render_patch_with_providers(
    reasoning: Option<&str>,
    preset: Option<&str>,
    providers: &[ExternalCliProvider],
) -> Result<String, String> {
    let mut out = render_patch(reasoning, preset);
    let providers: Vec<_> = providers
        .iter()
        .filter(|provider| !provider.disabled)
        .collect();
    if providers.is_empty() {
        return Ok(out);
    }

    let mut active_providers = Vec::with_capacity(providers.len());
    let mut routes = std::collections::HashSet::new();
    for provider in providers {
        let active = parse_active_provider(provider)?;
        if !routes.insert(active.route.clone()) {
            return Err(format!("dsh 供应商路由重复：{}", active.route));
        }
        active_providers.push(active);
    }

    out.push_str("\n# 挂载到 Kivio profile 的第三方供应商；API Key 只通过环境变量注入。\n");
    out.push_str("- id: llm-pi-ai\n  config:\n    providers:\n");
    for active in active_providers {
        let mut config = active.config;
        if let Some(object) = config.as_object_mut() {
            if let Some(effort) = normalize_pi_reasoning_effort(reasoning) {
                object.insert("reasoning".to_string(), Value::String(effort.to_string()));
            } else {
                object.remove("reasoning");
            }
        }
        let yaml = serde_yaml::to_string(&config)
            .map_err(|err| format!("序列化 dsh 供应商配置失败：{err}"))?;
        out.push_str("      ");
        out.push_str(&active.route);
        out.push_str(":\n");
        for line in yaml.trim_start_matches("---\n").lines() {
            out.push_str("        ");
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
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

/// 确保 profile 可用：装依赖（首次）+ 按当前设置重写 patch。
///
/// **幂等**。依赖已在时不跑 pnpm（那条要几秒）；patch 每次都重写，因为推理档位可能变了。
///
/// `Err` 的内容是给用户看的：pnpm 没装、网络不通、目录没权限都会走到这里，
/// 而这条错误会成为那一轮的失败气泡 —— 所以不写「bootstrap failed」这种查不到根因的话。
pub async fn ensure_profile_ready(
    bin: &Path,
    reasoning: Option<&str>,
    preset: Option<&str>,
) -> Result<(), String> {
    let home = dsh_home().ok_or_else(|| "无法定位 dsh 主目录（$DSH_HOME / ~/.dsh）".to_string())?;
    let dir = home.join("profiles").join(KIVIO_PROFILE);

    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 dsh profile 目录失败：{e}"))?;
    std::fs::write(dir.join(".npmrc"), PROFILE_NPMRC)
        .map_err(|e| format!("写入 dsh profile npmrc 失败：{e}"))?;

    let core_version = dsh_core_version(bin).await?;
    if !packages_match_core(&dir, &core_version) {
        install_packages(bin, &core_version, &home).await?;
    }
    normalize_profile_manifest(&dir)?;

    // patch 在依赖之后写：`dsh plugin` 首次会按模板初始化 profile 目录并写一份占位
    // `cordis.patch.yml`（内容是 `[]`），先写就会被它覆盖掉。
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 dsh profile 目录失败：{e}"))?;
    std::fs::write(dir.join(BRIDGE_FILENAME), BRIDGE_SOURCE)
        .map_err(|e| format!("写入 Kivio dsh bridge 失败：{e}"))?;
    let config = crate::external_agents::overrides::agent_config("dsh").unwrap_or_default();
    let mut patch = render_patch_with_providers(reasoning, preset, &config.providers)?;
    append_preset_declarations(&mut patch, &dir, &home.join(USER_PRESET_DIR))?;
    std::fs::write(dir.join("cordis.patch.yml"), patch)
        .map_err(|e| format!("写入 dsh profile 配置失败：{e}"))
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
    let bundles_changed = *bundles != previous_bundles;
    if version_changed || removed_preset || bundles_changed {
        let next = serde_json::to_string_pretty(&value)
            .map_err(|e| format!("序列化 dsh profile package.json 失败：{e}"))?;
        std::fs::write(&package_json, format!("{next}\n"))
            .map_err(|e| format!("写入 dsh profile package.json 失败：{e}"))?;
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
    crate::external_agents::installer::ensure_pnpm_for_dsh().await?;

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
        let yml = render_patch(None, None);
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
        let yml = render_patch(None, None);
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
        let mut patch = render_patch(None, Some("code"));
        append_preset_declarations(&mut patch, dir.path(), &users).unwrap();
        assert!(patch.contains("default: ptc"));
        assert!(patch.contains("!!js process.platform === 'win32'"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&patch).unwrap();
        let rows = parsed.as_sequence().unwrap();
        let custom = rows.last().unwrap();
        assert_eq!(
            custom["insert"][0]["config"]["id"].as_str(),
            Some("my-reviewer")
        );
        let path = custom["insert"][0]["config"]["plugins"][0]["config"]["path"]
            .as_str()
            .unwrap();
        assert_eq!(
            url::Url::parse(path).unwrap().to_file_path().unwrap(),
            users.join("my-reviewer").join(COMPOSITION_FILE)
        );
        std::fs::remove_file(presets.join("ptc.patch.yml")).unwrap();
        assert!(append_preset_declarations(&mut String::new(), dir.path(), &users).is_err());
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
        let yml = render_patch_with_providers(Some("high"), None, &[provider]).unwrap();
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

        let yml = render_patch_with_providers(None, None, &[first, second, disabled]).unwrap();
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
        let yml = render_patch_with_providers(Some("low"), None, &[provider.clone()]).unwrap();
        assert!(yml.contains("reasoning: low"));
        assert!(yml.contains("reasoningEffort: low"));

        let yml = render_patch_with_providers(Some("minimal"), None, &[provider.clone()]).unwrap();
        assert!(yml.contains("reasoning: minimal"));
        assert!(!yml.contains("reasoningEffort:"));
        assert!(!yml.contains("- id: llm-deepseek"));

        let yml = render_patch_with_providers(Some("xhigh"), None, &[provider.clone()]).unwrap();
        assert!(yml.contains("reasoning: xhigh"));
        assert!(!yml.contains("reasoningEffort:"));

        let yml = render_patch_with_providers(Some("default"), None, &[provider]).unwrap();
        assert!(!yml.contains("reasoning:"));
    }

    #[test]
    fn provider_patch_rejects_missing_key_and_invalid_default_model() {
        let mut provider = relay_provider();
        provider.env.clear();
        assert!(render_patch_with_providers(None, None, &[provider])
            .unwrap_err()
            .contains("KIVIO_DSH_RELAY_ONE_API_KEY"));

        let mut provider = relay_provider();
        provider.default_model = "missing".to_string();
        assert!(render_patch_with_providers(None, None, &[provider])
            .unwrap_err()
            .contains("默认模型"));
    }

    /// 两条 disable 是钱和稳定性：标题 LLM 每轮一次白花的调用，hmr 是意外重连。
    #[test]
    fn patch_disables_hmr_and_title_llm() {
        let yml = render_patch(None, None);
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
            serde_json::json!(["@deepseek-ai/dsh-base", "custom-bundle"])
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
        let yml = render_patch(None, Some("minimal"));
        assert!(yml.contains("@deepseek-ai/dsh-agent-preset-registry"));
        assert!(yml.contains("default: minimal"));
        assert!(!yml.contains("default: standard"));
    }

    #[test]
    fn unknown_preset_is_not_interpolated_into_the_patch() {
        let yml = render_patch(None, Some("x\n- id: injected-tool\n  disabled: false"));
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
        let yml = render_patch(Some("max"), None);
        assert!(yml.contains("- id: llm-deepseek"));
        assert!(yml.contains("reasoningEffort: max"));
    }

    /// `default` 是 Kivio 侧的「不指定」哨兵，**不是** dsh 的合法档位值
    /// （schema 只认 `off|low|high|max`）。写进去整棵配置树会在启动时被拒。
    #[test]
    fn patch_omits_reasoning_for_default_and_blank() {
        for value in [None, Some(""), Some("   "), Some("default")] {
            let yml = render_patch(value, None);
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
        assert!(render_patch(Some("off"), None).contains("reasoningEffort: off"));
        assert!(render_patch(Some("low"), None).contains("reasoningEffort: low"));
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
            let yml = render_patch(value, None);
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
        for reasoning in [None, Some("high")] {
            let yml = render_patch(reasoning, None);
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
                    "顶层只能是数组条目或其缩进续行，出现了 {line:?}（reasoning={reasoning:?}）"
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
