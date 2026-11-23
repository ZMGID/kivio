//! DSH's editable settings live in its profile Cordis patch. Pre-migration homes
//! use the legacy namespace document. Credentials remain in the DSH-managed store.

mod config;
use config::SettingsDocument;
pub(crate) use config::{
    native_settings_path, parse_cordis_yaml, preserve_legacy_launch_settings,
    read_effective_settings, read_profile_settings, render_cordis_yaml,
};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::time::timeout;

use crate::external_agents::dsh_profile::{profile_dir, KIVIO_PROFILE};
use crate::external_agents::registry::get_agent_def;
use crate::external_agents::spawn::{agent_cli_command, resolve_binary};
use crate::proc::NoConsoleWindow;
use crate::settings::ExternalCliProvider;

const SHELL_NS: &str = "shell";
const AGENT_LOOP_NS: &str = "agent-loop";
const WEB_SEARCH_NS: &str = "web-search-deepseek";
const LLM_PI_AI_NS: &str = "llm-pi-ai";
const CREDENTIALS_FILENAME: &str = ".credentials.yaml";
const DOTENV_FILENAME: &str = ".env";
const DEFAULT_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";
const DSH_OFFICIAL_PROVIDER_ID: &str = "deepseek-official";

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 64_000;
const DEFAULT_MAX_PARALLEL: u64 = 10;
const DEFAULT_MAX_USES: u64 = 5;
const DEFAULT_SEARCH_BASE_URL: &str = "https://api.deepseek.com/anthropic/v1";

const DUMP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshPluginSettingsSnapshot {
    pub settings_path: String,
    pub shell: DshShellSettings,
    pub agent_loop: DshAgentLoopSettings,
    pub web_search: DshWebSearchSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshShellSettings {
    pub timeout_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub timeout_ms_default: u64,
    pub max_output_bytes_default: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshAgentLoopSettings {
    pub max_parallel_tool_calls: Option<u64>,
    pub max_parallel_tool_calls_default: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshWebSearchSettings {
    pub base_url: Option<String>,
    pub max_uses: Option<u64>,
    pub api_key_env: String,
    pub api_key_configured: bool,
    pub api_key_writable: bool,
    pub base_url_default: String,
    pub max_uses_default: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshPluginSettingsPatch {
    pub shell: Option<DshShellPatch>,
    pub agent_loop: Option<DshAgentLoopPatch>,
    pub web_search: Option<DshWebSearchPatch>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshShellPatch {
    /// `Some(None)` = 恢复默认（从 yaml 删掉该键）。
    #[serde(default, deserialize_with = "patch_field")]
    pub timeout_ms: Option<Option<u64>>,
    #[serde(default, deserialize_with = "patch_field")]
    pub max_output_bytes: Option<Option<u64>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshAgentLoopPatch {
    #[serde(default, deserialize_with = "patch_field")]
    pub max_parallel_tool_calls: Option<Option<u64>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshWebSearchPatch {
    #[serde(default, deserialize_with = "patch_field")]
    pub base_url: Option<Option<String>>,
    #[serde(default, deserialize_with = "patch_field")]
    pub max_uses: Option<Option<u64>>,
    /// 非空才写入 `.credentials.yaml`；空字符串 = 保持现有密钥。
    pub api_key: Option<String>,
}

fn patch_field<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DshPluginEntry {
    pub id: String,
    pub module_name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DshOfficialCredential {
    pub configured: bool,
    pub writable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DshNativeProviderModel {
    pub id: String,
    pub name: String,
}

/// 当前 dsh 原生配置里一条 `llm-pi-ai` 供应商的编辑稿。探测摘要不带密钥；
/// 这个命令只在用户点「修改」时才回读 `.credentials.yaml`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DshNativeProviderDetail {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api: String,
    pub api_key: String,
    pub api_key_env: String,
    pub models: Vec<DshNativeProviderModel>,
    pub default_model: String,
}

fn dsh_home() -> Option<PathBuf> {
    super::dsh_profile::dsh_home()
}

fn settings_path() -> Result<PathBuf, String> {
    dsh_home()
        .map(|home| native_settings_path(&home))
        .ok_or_else(|| "无法定位 dsh 配置目录".to_string())
}

fn credentials_path() -> Result<PathBuf, String> {
    dsh_home()
        .map(|home| home.join(CREDENTIALS_FILENAME))
        .ok_or_else(|| "无法定位 dsh 配置目录".to_string())
}

fn mapping_u64(map: &Mapping, key: &str) -> Option<u64> {
    let value = map.get(serde_yaml::Value::String(key.to_string()))?;
    match value {
        serde_yaml::Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|v| u64::try_from(v).ok())),
        serde_yaml::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn mapping_string(map: &Mapping, key: &str) -> Option<String> {
    map.get(serde_yaml::Value::String(key.to_string()))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn yaml_key(name: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(name.to_string())
}

fn valid_credential_ref(key: &str) -> bool {
    let mut bytes = key.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn parse_credential_mapping(text: &str) -> Result<Mapping, String> {
    serde_yaml::from_str::<Option<Mapping>>(text)
        .map(Option::unwrap_or_default)
        .map_err(|_| "无法解析 dsh 凭据文件的 YAML 映射".into())
}

fn load_credentials_mapping(path: &Path) -> Result<Mapping, String> {
    parse_credential_mapping(&config::read_optional(path)?)
}

fn flat_credentials(root: &Mapping) -> bool {
    !root.is_empty()
        && !root.contains_key("version")
        && root.iter().all(|(k, v)| {
            k.as_str().is_some_and(valid_credential_ref)
                && v.as_str().is_some_and(|s| !s.is_empty())
        })
}

fn credential_lookup(root: &Mapping, api_key_env: &str) -> Option<String> {
    let refs = if root.get("version").and_then(serde_yaml::Value::as_u64) == Some(1) {
        namespace_map(root, "refs")?
    } else if flat_credentials(root) {
        root
    } else {
        return None;
    };
    mapping_string(refs, api_key_env)
}

fn namespace_map<'a>(root: &'a Mapping, key: &str) -> Option<&'a Mapping> {
    root.get(serde_yaml::Value::String(key.to_string()))
        .and_then(serde_yaml::Value::as_mapping)
}

fn credential_status(api_key_env: &str) -> (bool, bool) {
    credential_status_with_files(
        api_key_env,
        credentials_path().ok().as_deref(),
        user_dotenv_path().as_deref(),
        None,
    )
}

fn launch_provides_credential(key: &str) -> bool {
    if let Some((_, value)) = crate::external_agents::overrides::env_for("dsh")
        .into_iter()
        .find(|(name, _)| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case(key)
            } else {
                name == key
            }
        })
    {
        return !value.is_empty();
    }
    !key.is_empty()
        && !key.contains(['=', '\0'])
        && std::env::var_os(key).is_some_and(|value| !value.is_empty())
}

fn credential_status_with_files(
    api_key_env: &str,
    credentials: Option<&Path>,
    user_env: Option<&Path>,
    project_env: Option<&Path>,
) -> (bool, bool) {
    if credentials.is_some_and(|path| credentials_contain(path, api_key_env)) {
        return (true, true);
    }
    // rc.5 accepts a managed ref even when the launch environment already supplies a key.
    let configured = launch_provides_credential(api_key_env)
        || project_env.is_some_and(|path| dotenv_contains(path, api_key_env))
        || user_env.is_some_and(|path| dotenv_contains(path, api_key_env));
    (configured, true)
}

fn user_dotenv_path() -> Option<PathBuf> {
    dsh_home().map(|home| home.join(DOTENV_FILENAME))
}

fn credentials_contain(path: &Path, api_key_env: &str) -> bool {
    load_credentials_mapping(path)
        .ok()
        .and_then(|root| credential_lookup(&root, api_key_env))
        .is_some()
}

fn dotenv_contains(path: &Path, api_key_env: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    dotenv_has_key(&text, api_key_env)
}

fn dotenv_has_key(text: &str, api_key_env: &str) -> bool {
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != api_key_env {
            continue;
        }
        let value = value.trim();
        let unquoted = if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        if !unquoted.is_empty() {
            return true;
        }
    }
    false
}

fn credential_ref_in(root: &Mapping, provider_id: &str) -> Option<String> {
    if provider_id == DSH_OFFICIAL_PROVIDER_ID {
        return namespace_map(root, "llm-deepseek").and_then(|p| mapping_string(p, "apiKeyEnv"));
    }
    namespace_map(root, LLM_PI_AI_NS)
        .and_then(|pi| namespace_map(pi, "providers"))
        .and_then(|providers| namespace_map(providers, provider_id))
        .and_then(|provider| mapping_string(provider, "apiKeyEnv"))
}

fn effective_credential_ref(provider_id: &str) -> Option<String> {
    credential_ref_in(&read_effective_settings(&dsh_home()?).ok()?, provider_id)
}

fn official_credential_ref() -> String {
    effective_credential_ref(DSH_OFFICIAL_PROVIDER_ID)
        .unwrap_or_else(|| DEFAULT_API_KEY_ENV.to_owned())
}

/// Connect 前的凭据门：Kivio 托管供应商由调用方另行判断；这里覆盖官方 DeepSeek
/// 与原生 `llm-pi-ai` 路由（含 `.env` 兜底）。
pub(crate) fn credentials_ready_for_provider(provider: &str, cwd: Option<&Path>) -> bool {
    if provider == "deepseek-account" {
        return true;
    }
    let api_key_env = if provider.trim().is_empty() || provider == DSH_OFFICIAL_PROVIDER_ID {
        official_credential_ref()
    } else if let Some(env) = effective_credential_ref(provider) {
        env
    } else {
        return false;
    };
    let project_env = cwd.map(|path| path.join(DOTENV_FILENAME));
    credential_status_with_files(
        &api_key_env,
        credentials_path().ok().as_deref(),
        user_dotenv_path().as_deref(),
        project_env.as_deref(),
    )
    .0
}

#[cfg(test)]
fn parse_settings_snapshot(
    text: &str,
    settings_path: &Path,
) -> Result<DshPluginSettingsSnapshot, String> {
    let root: Mapping = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    settings_snapshot(&root, &Mapping::new(), settings_path)
}

fn settings_snapshot(
    root: &Mapping,
    inherited: &Mapping,
    settings_path: &Path,
) -> Result<DshPluginSettingsSnapshot, String> {
    let numeric_default = |ns, field, fallback| {
        namespace_map(inherited, ns)
            .and_then(|map| mapping_u64(map, field))
            .unwrap_or(fallback)
    };
    let shell = namespace_map(root, SHELL_NS);
    let agent_loop = namespace_map(&root, AGENT_LOOP_NS);
    let web_search = namespace_map(&root, WEB_SEARCH_NS);
    let api_key_env = web_search
        .and_then(|map| mapping_string(map, "apiKeyEnv"))
        .unwrap_or_else(|| DEFAULT_API_KEY_ENV.to_string());
    let (api_key_configured, api_key_writable) = if web_search
        .and_then(|map| mapping_string(map, "apiKey"))
        .is_some()
    {
        (true, false)
    } else {
        credential_status(&api_key_env)
    };
    Ok(DshPluginSettingsSnapshot {
        settings_path: settings_path.display().to_string(),
        shell: DshShellSettings {
            timeout_ms: shell.and_then(|map| mapping_u64(map, "timeoutMs")),
            max_output_bytes: shell.and_then(|map| mapping_u64(map, "maxOutputBytes")),
            timeout_ms_default: numeric_default(SHELL_NS, "timeoutMs", DEFAULT_TIMEOUT_MS),
            max_output_bytes_default: numeric_default(
                SHELL_NS,
                "maxOutputBytes",
                DEFAULT_MAX_OUTPUT_BYTES,
            ),
        },
        agent_loop: DshAgentLoopSettings {
            max_parallel_tool_calls: agent_loop
                .and_then(|map| mapping_u64(map, "maxParallelToolCalls")),
            max_parallel_tool_calls_default: numeric_default(
                AGENT_LOOP_NS,
                "maxParallelToolCalls",
                DEFAULT_MAX_PARALLEL,
            ),
        },
        web_search: DshWebSearchSettings {
            base_url: web_search.and_then(|map| mapping_string(map, "baseURL")),
            max_uses: web_search.and_then(|map| mapping_u64(map, "maxUses")),
            api_key_env,
            api_key_configured,
            api_key_writable,
            base_url_default: namespace_map(inherited, WEB_SEARCH_NS)
                .and_then(|map| mapping_string(map, "baseURL"))
                .unwrap_or_else(|| DEFAULT_SEARCH_BASE_URL.to_string()),
            max_uses_default: numeric_default(WEB_SEARCH_NS, "maxUses", DEFAULT_MAX_USES),
        },
    })
}

fn set_optional_number(map: &mut Mapping, key: &str, value: Option<Option<u64>>) {
    let Some(next) = value else {
        return;
    };
    let yaml_key = serde_yaml::Value::String(key.to_string());
    match next {
        Some(number) => {
            map.insert(yaml_key, serde_yaml::Value::Number(number.into()));
        }
        None => {
            map.remove(&yaml_key);
        }
    }
}

fn set_optional_string(map: &mut Mapping, key: &str, value: Option<Option<String>>) {
    let Some(next) = value else {
        return;
    };
    let yaml_key = serde_yaml::Value::String(key.to_string());
    match next.and_then(|text| {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }) {
        Some(text) => {
            map.insert(yaml_key, serde_yaml::Value::String(text));
        }
        None => {
            map.remove(&yaml_key);
        }
    }
}

fn upsert_namespace(root: &mut Mapping, key: &str, fields: Mapping) {
    let yaml_key = serde_yaml::Value::String(key.to_string());
    if fields.is_empty() {
        root.remove(&yaml_key);
    } else {
        root.insert(yaml_key, serde_yaml::Value::Mapping(fields));
    }
}

fn apply_patch(mut root: Mapping, patch: &DshPluginSettingsPatch) -> Mapping {
    if let Some(shell) = &patch.shell {
        let mut fields = namespace_map(&root, SHELL_NS).cloned().unwrap_or_default();
        set_optional_number(&mut fields, "timeoutMs", shell.timeout_ms);
        set_optional_number(&mut fields, "maxOutputBytes", shell.max_output_bytes);
        upsert_namespace(&mut root, SHELL_NS, fields);
    }
    if let Some(agent_loop) = &patch.agent_loop {
        let mut fields = namespace_map(&root, AGENT_LOOP_NS)
            .cloned()
            .unwrap_or_default();
        set_optional_number(
            &mut fields,
            "maxParallelToolCalls",
            agent_loop.max_parallel_tool_calls,
        );
        upsert_namespace(&mut root, AGENT_LOOP_NS, fields);
    }
    if let Some(web_search) = &patch.web_search {
        let mut fields = namespace_map(&root, WEB_SEARCH_NS)
            .cloned()
            .unwrap_or_default();
        set_optional_string(&mut fields, "baseURL", web_search.base_url.clone());
        set_optional_number(&mut fields, "maxUses", web_search.max_uses);
        upsert_namespace(&mut root, WEB_SEARCH_NS, fields);
    }
    root
}

fn write_credential(api_key_env: &str, api_key: &str) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Ok(());
    }
    write_credential_at(&credentials_path()?, api_key_env, key)
}

fn write_credential_at(path: &Path, api_key_env: &str, key: &str) -> Result<(), String> {
    use std::str::FromStr;
    if !valid_credential_ref(api_key_env) {
        return Err("dsh 的凭据引用必须是有效的环境变量名".into());
    }
    let mut lock = path.as_os_str().to_os_string();
    lock.push(".lock");
    config::with_dsh_writer_lock(Path::new(&lock), || {
        let original = config::read_optional(path)?;
        let root = parse_credential_mapping(&original)?;
        let text = if root.is_empty() {
            if original
                .lines()
                .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
            {
                format!("{original}\nversion: 1\n")
            } else {
                "version: 1\n".into()
            }
        } else if root.get("version").and_then(serde_yaml::Value::as_u64) == Some(1) {
            original
        } else if flat_credentials(&root)
            && !original.lines().any(|line| {
                line.starts_with('%') || line.starts_with("---") || line.starts_with("...")
            })
        {
            format!(
                "version: 1\nrefs:\n{}",
                original
                    .lines()
                    .map(|line| format!("  {line}\n"))
                    .collect::<String>()
            )
        } else {
            return Err("dsh 凭据文件版本或结构不受支持".into());
        };
        let doc = yaml_edit::Document::from_str(&text).map_err(|_| "无法解析 dsh 凭据文件")?;
        let mapping = doc.as_mapping().ok_or("dsh 凭据文件必须是映射")?;
        if mapping.get("refs").is_none() {
            mapping.set("refs", yaml_edit::Mapping::new());
        }
        let refs = mapping
            .get_mapping("refs")
            .ok_or("dsh 凭据文件 refs 必须是映射")?;
        refs.set(api_key_env, key);
        let rendered = doc.to_string();
        if parse_credential_mapping(&rendered)?
            .get("refs")
            .and_then(|r| r.get(api_key_env))
            .and_then(serde_yaml::Value::as_str)
            != Some(key)
        {
            return Err("无法保存 dsh 凭据".into());
        }
        crate::external_agents::provider_profile::write_private_atomic(path, &rendered)
    })
}

fn persist_settings(path: &Path, root: Mapping) -> Result<(), String> {
    let yaml = if root.is_empty() {
        String::new()
    } else {
        serde_yaml::to_string(&serde_yaml::Value::Mapping(root))
            .map_err(|err| format!("序列化 dsh settings.yaml 失败：{err}"))?
    };
    crate::external_agents::provider_profile::write_private_atomic(path, &yaml)
}

fn is_explicitly_disabled(map: &Mapping) -> bool {
    matches!(
        map.get(serde_yaml::Value::String("disabled".to_string())),
        Some(serde_yaml::Value::Bool(true))
    )
}

fn collect_entries(value: &serde_yaml::Value, out: &mut BTreeMap<String, DshPluginEntry>) {
    match value {
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                collect_entries(item, out);
            }
        }
        serde_yaml::Value::Mapping(map) => {
            let id = mapping_string(map, "id");
            let module_name = mapping_string(map, "name");
            let is_group = map.get("group").and_then(serde_yaml::Value::as_bool) == Some(true)
                || module_name.as_deref() == Some("@deepseek-ai/dsh-group");
            match (id, module_name) {
                (Some(id), Some(module_name)) => {
                    out.insert(
                        id.clone(),
                        DshPluginEntry {
                            id,
                            module_name,
                            enabled: !is_explicitly_disabled(map),
                        },
                    );
                }
                (Some(id), None) => {
                    if let (Some(existing), Some(disabled)) = (
                        out.get_mut(&id),
                        map.get("disabled").and_then(serde_yaml::Value::as_bool),
                    ) {
                        existing.enabled = !disabled;
                    }
                }
                _ => {}
            }
            if let Some(insert) = map.get("insert") {
                collect_entries(insert, out);
            }
            if let Some(children) = map.get("config").filter(|_| is_group) {
                collect_entries(children, out);
            }
        }
        _ => {}
    }
}

fn parse_plugin_manifest(
    text: &str,
    out: &mut BTreeMap<String, DshPluginEntry>,
) -> Result<(), String> {
    let value = parse_cordis_yaml(text)?;
    collect_entries(&value, out);
    Ok(())
}

fn dump_chunks(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("# ==") {
            if !current.trim().is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() && !text.trim().is_empty() {
        chunks.push(text.to_string());
    }
    chunks
}

fn parse_inventory_dump(text: &str) -> Result<Vec<DshPluginEntry>, String> {
    let mut entries = BTreeMap::new();
    for chunk in dump_chunks(text) {
        if let Ok(value) = parse_cordis_yaml(&chunk) {
            collect_entries(&value, &mut entries);
        }
    }
    if entries.is_empty() {
        let stripped: String = text
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .flat_map(|line| [line, "\n"])
            .collect();
        if let Ok(value) = parse_cordis_yaml(&stripped) {
            collect_entries(&value, &mut entries);
        }
    }
    if entries.is_empty() {
        return Err("解析 dsh --dump-config 失败：没有识别到插件条目".to_string());
    }
    Ok(entries.into_values().collect())
}

fn merge_manifest_file(
    path: &Path,
    out: &mut BTreeMap<String, DshPluginEntry>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("读取 {} 失败：{err}", path.display()))?;
    parse_plugin_manifest(&text, out)
}

async fn inventory_from_installed_patches() -> Result<Vec<DshPluginEntry>, String> {
    let def = get_agent_def("dsh").ok_or_else(|| "未注册 dsh".to_string())?;
    let bin = resolve_binary(def).await;
    let mut entries = BTreeMap::new();
    let home = dsh_home().ok_or("无法定位 dsh 配置目录")?;
    let profile = home.join("profiles").join(KIVIO_PROFILE);
    let files = config::bundle_patch_files(&profile, bin.as_deref());
    for patch in files.into_iter().chain([
        profile.join("cordis.patch.yml"),
        home.join("cordis.patch.yml"),
    ]) {
        if patch.is_file() {
            merge_manifest_file(&patch, &mut entries)?;
        }
    }
    if entries.is_empty() {
        return Err("未找到 dsh profile 的插件清单".into());
    }
    Ok(entries.into_values().collect())
}

fn inventory_fallback_error(dump_result: &Result<String, String>, file_err: &str) -> String {
    match dump_result {
        Err(dump_err) => format!("{dump_err}（已回退本地清单：{file_err}）"),
        Ok(_) => format!("无法从 dsh --dump-config 解析插件（已回退本地清单：{file_err}）"),
    }
}

async fn dump_kivio_config() -> Result<String, String> {
    let def = get_agent_def("dsh").ok_or_else(|| "未注册 dsh".to_string())?;
    let bin = resolve_binary(def)
        .await
        .ok_or_else(|| "未找到 dsh 可执行文件".to_string())?;
    let mut command = agent_cli_command(def, &bin);
    command
        .args(["--profile", KIVIO_PROFILE, "--dump-config"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_console_window()
        .kill_on_drop(true);
    if let Some(dir) = profile_dir() {
        command.current_dir(dir);
    }
    if let Some(home) = dsh_home() {
        command.env("DSH_HOME", home);
    }
    let child = crate::external_agents::spawn::AgentChild::spawn(&mut command)
        .map_err(|err| format!("启动 dsh --dump-config 失败：{err}"))?;
    let output = timeout(DUMP_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| "dsh --dump-config 超时".to_string())?
        .map_err(|err| format!("读取 dsh --dump-config 失败：{err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.chars().rev().take(800).collect::<String>();
        let detail: String = detail.chars().rev().collect();
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            "dsh --dump-config 失败".to_string()
        } else {
            format!("dsh --dump-config 失败：{detail}")
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if stdout.trim().is_empty() {
        return Err("dsh --dump-config 没有输出".to_string());
    }
    Ok(stdout)
}

fn open_settings_file(app: &AppHandle, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建 dsh 配置目录失败：{err}"))?;
    }
    if !path.exists() {
        crate::external_agents::provider_profile::write_private_atomic(
            path,
            if path.ends_with("settings.yaml") {
                "{}\n"
            } else {
                "[]\n"
            },
        )?;
    }
    app.shell()
        .open(path.display().to_string(), None)
        .map_err(|err| format!("打开配置文件失败：{err}"))
}

#[tauri::command]
pub fn chat_dsh_plugin_settings_get() -> Result<DshPluginSettingsSnapshot, String> {
    let path = settings_path()?;
    let doc = SettingsDocument::open(&path)?;
    settings_snapshot(&doc.effective, &doc.inherited, &path)
}

fn save_plugin_patch(
    document: &SettingsDocument,
    patch: &DshPluginSettingsPatch,
) -> Result<(), String> {
    let mut next = apply_patch(document.effective.clone(), patch);
    // An explicit reset restores the bundle value; omitted fields keep their current value.
    for (ns, field, reset) in [
        (
            SHELL_NS,
            "timeoutMs",
            patch
                .shell
                .as_ref()
                .is_some_and(|p| p.timeout_ms == Some(None)),
        ),
        (
            SHELL_NS,
            "maxOutputBytes",
            patch
                .shell
                .as_ref()
                .is_some_and(|p| p.max_output_bytes == Some(None)),
        ),
        (
            AGENT_LOOP_NS,
            "maxParallelToolCalls",
            patch
                .agent_loop
                .as_ref()
                .is_some_and(|p| p.max_parallel_tool_calls == Some(None)),
        ),
        (
            WEB_SEARCH_NS,
            "baseURL",
            patch
                .web_search
                .as_ref()
                .is_some_and(|p| p.base_url == Some(None)),
        ),
        (
            WEB_SEARCH_NS,
            "maxUses",
            patch
                .web_search
                .as_ref()
                .is_some_and(|p| p.max_uses == Some(None)),
        ),
    ] {
        if let Some(value) = document
            .inherited
            .get(ns)
            .and_then(|config| config.get(field))
            .filter(|_| reset)
        {
            next.entry(yaml_key(ns))
                .or_insert_with(|| serde_yaml::Value::Mapping(Mapping::new()))[field] =
                value.clone();
        }
    }
    document.save(next)
}

#[tauri::command]
pub fn chat_dsh_plugin_settings_save(
    patch: DshPluginSettingsPatch,
) -> Result<DshPluginSettingsSnapshot, String> {
    let _agent = super::session::live::ResourceLease::acquire(vec!["agent:dsh".into()], true)?;
    let path = settings_path()?;
    let _update = super::provider_profile::native_file_update(&path)?;
    let document = SettingsDocument::open(&path)?;
    if patch
        .web_search
        .as_ref()
        .and_then(|web| web.api_key.as_deref())
        .is_some_and(|key| !key.trim().is_empty())
        && document
            .effective
            .get(WEB_SEARCH_NS)
            .and_then(|config| config.get("apiKey"))
            .and_then(serde_yaml::Value::as_str)
            .is_some_and(|key| !key.is_empty())
    {
        return Err("网页搜索的 apiKey 已由 profile 指定，请在配置文件中修改".into());
    }
    save_plugin_patch(&document, &patch)?;
    let snapshot = chat_dsh_plugin_settings_get()?;
    if let Some(api_key) = patch
        .web_search
        .and_then(|web| web.api_key)
        .filter(|key| !key.trim().is_empty())
    {
        write_credential(&snapshot.web_search.api_key_env, &api_key)?;
        return chat_dsh_plugin_settings_get();
    }
    Ok(snapshot)
}

#[tauri::command]
pub async fn chat_dsh_plugin_inventory() -> Result<Vec<DshPluginEntry>, String> {
    let dump_result = dump_kivio_config().await;
    if let Ok(dump) = &dump_result {
        if let Ok(entries) = parse_inventory_dump(dump) {
            if !entries.is_empty() {
                return Ok(entries);
            }
        }
    }
    match inventory_from_installed_patches().await {
        Ok(entries) if !entries.is_empty() => Ok(entries),
        Ok(_) => Err(inventory_fallback_error(&dump_result, "本地插件清单为空")),
        Err(file_err) => Err(inventory_fallback_error(&dump_result, &file_err)),
    }
}

#[tauri::command]
pub fn chat_dsh_open_settings_file(app: AppHandle) -> Result<(), String> {
    let _agent = super::session::live::ResourceLease::acquire(vec!["agent:dsh".into()], true)?;
    let path = settings_path()?;
    let _update = super::provider_profile::native_file_update(&path)?;
    open_settings_file(&app, &path)
}

fn reject_managed_official_provider(id: &str) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("供应商 id 不能为空".to_string());
    }
    if id == DSH_OFFICIAL_PROVIDER_ID {
        return Err("官方 DeepSeek 不能按第三方供应商编辑".to_string());
    }
    Ok(())
}

fn credential_value(path: &Path, api_key_env: &str) -> Option<String> {
    if api_key_env.is_empty() {
        return None;
    }
    load_credentials_mapping(path)
        .ok()
        .and_then(|root| credential_lookup(&root, api_key_env))
}

fn parse_native_models(value: Option<&serde_yaml::Value>) -> Vec<DshNativeProviderModel> {
    let Some(serde_yaml::Value::Sequence(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            serde_yaml::Value::String(id) => {
                let id = id.trim();
                (!id.is_empty()).then(|| DshNativeProviderModel {
                    id: id.to_string(),
                    name: id.to_string(),
                })
            }
            serde_yaml::Value::Mapping(map) => {
                let id = mapping_string(map, "id")?;
                let name = mapping_string(map, "name").unwrap_or_else(|| id.clone());
                Some(DshNativeProviderModel { id, name })
            }
            _ => None,
        })
        .collect()
}

fn selected_default_model(root: &Mapping) -> (Option<String>, Option<String>) {
    for key in ["agent-default-model", "api-gateway"] {
        if let Some(map) = namespace_map(root, key) {
            return (
                mapping_string(map, "provider"),
                mapping_string(map, "model"),
            );
        }
    }
    (None, None)
}

fn native_provider_detail_at(
    settings_path: &Path,
    credentials_path: &Path,
    id: &str,
) -> Result<DshNativeProviderDetail, String> {
    reject_managed_official_provider(id)?;
    let root = SettingsDocument::open(settings_path)?.effective;
    let provider = namespace_map(&root, LLM_PI_AI_NS)
        .and_then(|pi| namespace_map(pi, "providers"))
        .and_then(|providers| {
            providers
                .get(serde_yaml::Value::String(id.to_string()))
                .and_then(serde_yaml::Value::as_mapping)
        })
        .ok_or_else(|| format!("未找到供应商 {id}"))?;
    let name = mapping_string(provider, "displayName").unwrap_or_else(|| id.to_string());
    let base_url = mapping_string(provider, "baseURL").unwrap_or_default();
    let api = mapping_string(provider, "api").unwrap_or_default();
    let api_key_env = mapping_string(provider, "apiKeyEnv").unwrap_or_default();
    let models = parse_native_models(provider.get(serde_yaml::Value::String("models".to_string())));
    let (default_provider, default_model) = selected_default_model(&root);
    let default_model = if default_provider.as_deref() == Some(id) {
        default_model
            .filter(|model| models.iter().any(|entry| entry.id == *model))
            .or_else(|| models.first().map(|model| model.id.clone()))
            .unwrap_or_default()
    } else {
        models
            .first()
            .map(|model| model.id.clone())
            .unwrap_or_default()
    };
    Ok(DshNativeProviderDetail {
        id: id.to_string(),
        name,
        base_url,
        api,
        api_key: credential_value(credentials_path, &api_key_env).unwrap_or_default(),
        api_key_env,
        models,
        default_model,
    })
}

fn clear_default_if_matches(root: &mut Mapping, id: &str) {
    for key in ["agent-default-model", "api-gateway"] {
        let yaml_key = serde_yaml::Value::String(key.to_string());
        let matches = root
            .get(&yaml_key)
            .and_then(serde_yaml::Value::as_mapping)
            .and_then(|map| mapping_string(map, "provider"))
            .as_deref()
            == Some(id);
        if matches {
            if let Some(map) = root
                .get_mut(&yaml_key)
                .and_then(serde_yaml::Value::as_mapping_mut)
            {
                map.remove("provider");
                map.remove("model");
                map.remove("reasoningEffort");
                if map.is_empty() {
                    root.remove(&yaml_key);
                }
            }
        }
    }
}

fn delete_native_provider_in(root: &mut Mapping, id: &str) {
    let pi_key = serde_yaml::Value::String(LLM_PI_AI_NS.to_string());
    let Some(serde_yaml::Value::Mapping(mut pi)) = root.remove(&pi_key) else {
        return;
    };
    let providers_key = serde_yaml::Value::String("providers".to_string());
    if let Some(serde_yaml::Value::Mapping(mut providers)) = pi.remove(&providers_key) {
        providers.remove(&serde_yaml::Value::String(id.to_string()));
        if !providers.is_empty() {
            pi.insert(providers_key, serde_yaml::Value::Mapping(providers));
        }
    }
    if !pi.is_empty() {
        root.insert(pi_key, serde_yaml::Value::Mapping(pi));
    }
    clear_default_if_matches(root, id);
}

pub(crate) fn delete_native_provider_at(settings_path: &Path, id: &str) -> Result<(), String> {
    reject_managed_official_provider(id)?;
    let doc = SettingsDocument::open(settings_path)?;
    let mut root = doc.effective.clone();
    delete_native_provider_in(&mut root, id);
    doc.save(root)
}

fn modality_list(has_image: bool) -> serde_yaml::Value {
    let mut items = vec![serde_yaml::Value::String("text".to_string())];
    if has_image {
        items.push(serde_yaml::Value::String("image".to_string()));
    }
    serde_yaml::Value::Sequence(items)
}

fn modality_matches(value: Option<&serde_yaml::Value>, has_image: bool) -> bool {
    let Some(serde_yaml::Value::Sequence(items)) = value else {
        return false;
    };
    let has_text = items.iter().any(|item| item.as_str() == Some("text"));
    let image = items.iter().any(|item| item.as_str() == Some("image"));
    has_text && image == has_image
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReasoningDecl {
    Disabled,
    Levels(BTreeMap<String, Option<String>>),
}

#[derive(Debug, Clone)]
struct ModelCaps {
    has_image: Option<bool>,
    reasoning: Option<ReasoningDecl>,
}

fn parse_reasoning_decl(value: &serde_json::Value) -> Option<ReasoningDecl> {
    if value.as_bool() == Some(false) {
        return Some(ReasoningDecl::Disabled);
    }
    let object = value.as_object()?;
    if object.is_empty() {
        return None;
    }
    let mut levels = BTreeMap::new();
    for (key, item) in object {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let wire = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        levels.insert(key.to_string(), wire);
    }
    Some(ReasoningDecl::Levels(levels))
}

fn reasoning_yaml(decl: &ReasoningDecl) -> serde_yaml::Value {
    match decl {
        ReasoningDecl::Disabled => serde_yaml::Value::Bool(false),
        ReasoningDecl::Levels(levels) => {
            let mut map = Mapping::new();
            for (key, wire) in levels {
                map.insert(
                    serde_yaml::Value::String(key.clone()),
                    match wire {
                        Some(value) => serde_yaml::Value::String(value.clone()),
                        None => serde_yaml::Value::Null,
                    },
                );
            }
            serde_yaml::Value::Mapping(map)
        }
    }
}

fn reasoning_matches(existing: Option<&serde_yaml::Value>, decl: &ReasoningDecl) -> bool {
    match (existing, decl) {
        (Some(serde_yaml::Value::Bool(false)), ReasoningDecl::Disabled) => true,
        (Some(serde_yaml::Value::Mapping(map)), ReasoningDecl::Levels(levels)) => {
            if map.len() != levels.len() {
                return false;
            }
            levels.iter().all(
                |(key, wire)| match map.get(serde_yaml::Value::String(key.clone())) {
                    Some(serde_yaml::Value::Null) => wire.is_none(),
                    Some(serde_yaml::Value::String(value)) => {
                        wire.as_deref() == Some(value.as_str())
                    }
                    _ => false,
                },
            )
        }
        _ => false,
    }
}

fn model_caps_from_config_json(config_json: &str) -> BTreeMap<String, ModelCaps> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(config_json) else {
        return BTreeMap::new();
    };
    let Some(models) = value.get("models").and_then(serde_json::Value::as_array) else {
        return BTreeMap::new();
    };
    let mut caps = BTreeMap::new();
    for model in models {
        let Some(id) = model
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let has_image = model
            .get("input")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.iter().any(|item| item.as_str() == Some("image")));
        let reasoning = model.get("reasoningEfforts").and_then(parse_reasoning_decl);
        if has_image.is_none() && reasoning.is_none() {
            continue;
        }
        caps.insert(
            id.to_string(),
            ModelCaps {
                has_image,
                reasoning,
            },
        );
    }
    caps
}

fn write_model_caps(map: &mut Mapping, caps: &ModelCaps) -> bool {
    let mut changed = false;
    if let Some(image) = caps
        .has_image
        .filter(|image| !modality_matches(map.get("input"), *image))
    {
        map.insert(yaml_key("input"), modality_list(image));
        changed = true;
    }
    if let Some(reasoning) = &caps.reasoning {
        if !reasoning_matches(
            map.get(serde_yaml::Value::String("reasoningEfforts".to_string())),
            reasoning,
        ) {
            map.insert(
                serde_yaml::Value::String("reasoningEfforts".to_string()),
                reasoning_yaml(reasoning),
            );
            changed = true;
        }
    }
    changed
}

fn apply_model_caps(models: &mut serde_yaml::Value, caps: &BTreeMap<String, ModelCaps>) -> bool {
    let Some(items) = models.as_sequence_mut() else {
        return false;
    };
    let mut changed = false;
    for item in items.iter_mut() {
        match item {
            serde_yaml::Value::String(id) => {
                let id = id.trim();
                let Some(model) = caps.get(id) else {
                    continue;
                };
                let mut map = Mapping::new();
                map.insert(
                    serde_yaml::Value::String("id".to_string()),
                    serde_yaml::Value::String(id.to_string()),
                );
                write_model_caps(&mut map, model);
                *item = serde_yaml::Value::Mapping(map);
                changed = true;
            }
            serde_yaml::Value::Mapping(map) => {
                let Some(id) = mapping_string(map, "id") else {
                    continue;
                };
                let Some(model) = caps.get(&id) else {
                    continue;
                };
                changed |= write_model_caps(map, model);
            }
            _ => {}
        }
    }
    changed
}

fn apply_provider_caps(
    root: &mut Mapping,
    route: &str,
    caps: &BTreeMap<String, ModelCaps>,
) -> bool {
    let Some(pi) = root
        .get_mut(serde_yaml::Value::String(LLM_PI_AI_NS.to_string()))
        .and_then(serde_yaml::Value::as_mapping_mut)
    else {
        return false;
    };
    let Some(providers) = pi
        .get_mut(serde_yaml::Value::String("providers".to_string()))
        .and_then(serde_yaml::Value::as_mapping_mut)
    else {
        return false;
    };
    let Some(provider) = providers
        .get_mut(serde_yaml::Value::String(route.to_string()))
        .and_then(serde_yaml::Value::as_mapping_mut)
    else {
        return false;
    };
    let mut changed = false;
    if let Some(models) = provider.get_mut(serde_yaml::Value::String("models".to_string())) {
        changed |= apply_model_caps(models, caps);
    }
    let mut modalities = caps.values().filter_map(|model| model.has_image).peekable();
    if modalities.peek().is_some() {
        let default_has_image = modalities.any(|image| image);
        if !modality_matches(provider.get("defaultInput"), default_has_image) {
            provider.insert(yaml_key("defaultInput"), modality_list(default_has_image));
            changed = true;
        }
    }
    changed
}

pub(crate) fn sync_kivio_model_capabilities_at(
    settings_path: &Path,
    providers: &[ExternalCliProvider],
) -> Result<(), String> {
    let doc = SettingsDocument::open(settings_path)?;
    let mut root = doc.effective.clone();
    let mut changed = false;
    for provider in providers {
        let route = provider.native_provider_id.trim();
        if route.is_empty() || route == DSH_OFFICIAL_PROVIDER_ID {
            continue;
        }
        let caps = model_caps_from_config_json(&provider.config_json);
        if caps.is_empty() {
            continue;
        }
        changed |= apply_provider_caps(&mut root, route, &caps);
    }
    if changed {
        doc.save(root)?;
    }
    Ok(())
}

#[tauri::command]
pub fn chat_dsh_native_provider_get(id: String) -> Result<DshNativeProviderDetail, String> {
    native_provider_detail_at(&settings_path()?, &credentials_path()?, id.trim())
}

#[tauri::command]
pub fn chat_dsh_native_provider_delete(id: String) -> Result<(), String> {
    let _agent = super::session::live::ResourceLease::acquire(vec!["agent:dsh".into()], true)?;
    let path = settings_path()?;
    let _update = super::provider_profile::native_file_update(&path)?;
    delete_native_provider_at(&path, id.trim())
}

#[tauri::command]
pub fn chat_dsh_official_credential_status() -> Result<DshOfficialCredential, String> {
    let (configured, writable) = credential_status(&official_credential_ref());
    Ok(DshOfficialCredential {
        configured,
        writable,
    })
}

pub(crate) fn official_deepseek_key_ready() -> bool {
    credential_status(&official_credential_ref()).0
}

#[tauri::command]
pub fn chat_dsh_official_credential_save(api_key: String) -> Result<DshOfficialCredential, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("请输入 DeepSeek API 密钥".to_string());
    }
    write_credential(&official_credential_ref(), key)?;
    chat_dsh_official_credential_status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cordis_edits_preserve_unrelated_rows_and_respect_home_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let path = native_settings_path(home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let shell_id = if cfg!(windows) {
            "pwsh-sandbox"
        } else {
            "bash-sandbox"
        };
        let initial = format!("# native comment\n- id: unrelated\n  disabled: !!js process.env.ENABLED\n- id: {shell_id}\n  config:\n    timeoutMs: 123\n    maxOutputBytes: 310\n    cwd: !!js process.cwd()\n    label: 'literal !!js text'\n- insert:\n    - id: group-fixture\n      group: true\n      config: [{{id: agent-loop, config: {{maxParallelToolCalls: 6}}}}]\n");
        std::fs::write(&path, &initial).unwrap();
        let doc = SettingsDocument::open(&path).unwrap();
        assert_eq!(
            doc.effective["agent-loop"]["maxParallelToolCalls"].as_u64(),
            Some(6)
        );
        let patch =
            serde_json::from_value(serde_json::json!({"shell":{"timeoutMs":null}})).unwrap();
        save_plugin_patch(&doc, &patch).unwrap();
        let reopened = read_effective_settings(home).unwrap();
        assert_eq!(
            reopened["shell"]["timeoutMs"].as_u64(),
            (!cfg!(windows)).then_some(60000)
        );
        assert_eq!(reopened["shell"]["maxOutputBytes"].as_u64(), Some(310));
        let actual = std::fs::read_to_string(&path).unwrap();
        assert!(actual.contains("# native comment"));
        assert!(actual.contains("disabled: !!js process.env.ENABLED"));
        assert!(actual.contains("!!js process.cwd()"));
        assert_eq!(reopened["shell"]["label"], "literal !!js text");
        std::fs::write(
            home.join("cordis.patch.yml"),
            format!("- id: {shell_id}\n  config: {{maxOutputBytes: 41}}\n"),
        )
        .unwrap();
        let doc = SettingsDocument::open(&path).unwrap();
        assert_eq!(doc.effective["shell"]["maxOutputBytes"].as_u64(), Some(41));
        let patch =
            serde_json::from_value(serde_json::json!({"shell":{"maxOutputBytes":100}})).unwrap();
        assert!(save_plugin_patch(&doc, &patch)
            .unwrap_err()
            .contains("覆盖"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), actual);
        assert!(!home.join("settings.yaml").exists());
    }

    #[test]
    fn migration_preserves_native_routes_when_launch_overlay_shadows_import() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let profile = native_settings_path(home);
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(
            &profile,
            "- id: llm-pi-ai\n  config: {providers: {new: {models: [n]}}}\n",
        )
        .unwrap();
        let legacy = home.join("settings.yaml");
        std::fs::write(
            &legacy,
            "llm-pi-ai: {providers: {old: {models: [a], apiKeyEnv: TEST_KEY}}}\n",
        )
        .unwrap();
        assert_eq!(native_settings_path(home), legacy);
        preserve_legacy_launch_settings(home).unwrap();
        std::fs::rename(&legacy, home.join("settings.yaml.imported")).unwrap();
        assert_eq!(native_settings_path(home), profile);
        let settings = read_effective_settings(home).unwrap();
        assert_eq!(settings["llm-pi-ai"]["providers"]["new"]["models"][0], "n");
        assert_eq!(
            settings["llm-pi-ai"]["providers"]["old"]["apiKeyEnv"],
            "TEST_KEY"
        );
        delete_native_provider_at(&profile, "old").unwrap();
        let settings = read_effective_settings(home).unwrap();
        assert!(settings["llm-pi-ai"]["providers"].get("old").is_none());
        assert_eq!(settings["llm-pi-ai"]["providers"]["new"]["models"][0], "n");
    }

    #[test]
    fn empty_settings_inherit_schema_defaults() {
        let snapshot = parse_settings_snapshot("{}", Path::new("/tmp/settings.yaml")).unwrap();
        assert_eq!(snapshot.shell.timeout_ms, None);
        assert_eq!(snapshot.shell.timeout_ms_default, 120_000);
        assert_eq!(snapshot.agent_loop.max_parallel_tool_calls_default, 10);
        assert_eq!(snapshot.web_search.max_uses_default, 5);
        assert_eq!(snapshot.web_search.api_key_env, "DEEPSEEK_API_KEY");
    }

    #[test]
    fn reads_overridden_namespaces() {
        let snapshot = parse_settings_snapshot(
            r#"
ui-onboarding:
  welcomeNoticeVersion: keep-me
shell:
  timeoutMs: 90000
  maxOutputBytes: 32000
agent-loop:
  maxParallelToolCalls: 4
web-search-deepseek:
  baseURL: https://relay.example/anthropic/v1
  maxUses: 2
  apiKeyEnv: SEARCH_KEY
"#,
            Path::new("/tmp/settings.yaml"),
        )
        .unwrap();
        assert_eq!(snapshot.shell.timeout_ms, Some(90_000));
        assert_eq!(snapshot.shell.max_output_bytes, Some(32_000));
        assert_eq!(snapshot.agent_loop.max_parallel_tool_calls, Some(4));
        assert_eq!(
            snapshot.web_search.base_url.as_deref(),
            Some("https://relay.example/anthropic/v1")
        );
        assert_eq!(snapshot.web_search.max_uses, Some(2));
        assert_eq!(snapshot.web_search.api_key_env, "SEARCH_KEY");
    }

    #[test]
    fn patch_updates_one_namespace_and_keeps_neighbors() {
        let root = serde_yaml::from_str::<serde_yaml::Value>(
            "ui-onboarding:\n  keep: 1\nshell:\n  timeoutMs: 1000\n",
        )
        .unwrap()
        .as_mapping()
        .unwrap()
        .clone();
        let patched = apply_patch(
            root,
            &DshPluginSettingsPatch {
                agent_loop: Some(DshAgentLoopPatch {
                    max_parallel_tool_calls: Some(Some(3)),
                }),
                ..DshPluginSettingsPatch::default()
            },
        );
        assert!(patched.contains_key(serde_yaml::Value::String("ui-onboarding".into())));
        assert_eq!(
            mapping_u64(namespace_map(&patched, SHELL_NS).unwrap(), "timeoutMs"),
            Some(1000)
        );
        assert_eq!(
            mapping_u64(
                namespace_map(&patched, AGENT_LOOP_NS).unwrap(),
                "maxParallelToolCalls"
            ),
            Some(3)
        );
    }

    #[test]
    fn reset_removes_namespace_when_empty() {
        let root = serde_yaml::from_str::<serde_yaml::Value>("shell:\n  timeoutMs: 1000\n")
            .unwrap()
            .as_mapping()
            .unwrap()
            .clone();
        let patched = apply_patch(
            root,
            &DshPluginSettingsPatch {
                shell: Some(DshShellPatch {
                    timeout_ms: Some(None),
                    max_output_bytes: Some(None),
                }),
                ..DshPluginSettingsPatch::default()
            },
        );
        assert!(!patched.contains_key(serde_yaml::Value::String(SHELL_NS.into())));
    }

    #[test]
    fn inventory_reads_composed_dump_and_nested_groups() {
        let dump = r#"
# == cordis.yml
- id: include
  name: '@deepseek-ai/cordis-plugin-include'
# == @deepseek-ai/dsh-base
- id: timer
  name: '@deepseek-ai/dsh-timer'
- id: tool-bash
  name: '@deepseek-ai/dsh-tool-bash'
  disabled: true
- id: group-tools
  name: '@deepseek-ai/dsh-group'
  group: true
  config:
    - id: tool-web
      name: '@deepseek-ai/dsh-tool-web'
      disabled: !!js process.platform === 'win32'
"#;
        let entries = parse_inventory_dump(dump).unwrap();
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].id, "group-tools");
        assert!(entries
            .iter()
            .any(|entry| entry.id == "include" && entry.enabled));
        assert!(entries
            .iter()
            .any(|entry| entry.id == "tool-bash" && !entry.enabled));
        // 动态 JS 标记保留，不视为静态禁用。
        assert!(entries
            .iter()
            .any(|entry| entry.id == "tool-web" && entry.enabled));
    }

    #[test]
    fn inventory_skips_unparseable_dump_preamble() {
        let dump = r#"
node: warning this is not yaml
# == cordis.yml
- id: timer
  name: '@deepseek-ai/dsh-timer'
# == broken
:::::
# == @deepseek-ai/dsh-base
- id: tool-bash
  name: '@deepseek-ai/dsh-tool-bash'
  disabled: true
"#;
        let entries = parse_inventory_dump(dump).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .any(|entry| entry.id == "timer" && entry.enabled));
        assert!(entries
            .iter()
            .any(|entry| entry.id == "tool-bash" && !entry.enabled));
    }

    #[test]
    fn inventory_reads_patch_insert_and_disabled_overlay() {
        let mut entries = BTreeMap::new();
        parse_plugin_manifest(
            r#"
- insert:
    - id: timer
      name: '@deepseek-ai/dsh-timer'
    - id: tool-bash
      name: '@deepseek-ai/dsh-tool-bash'
    - id: group-tools
      name: '@deepseek-ai/dsh-group'
      config:
        - id: tool-web
          name: '@deepseek-ai/dsh-tool-web'
"#,
            &mut entries,
        )
        .unwrap();
        parse_plugin_manifest(
            r#"
- id: tool-bash
  disabled: true
- insert:
    - id: kivio-dsh-jsonrpc-bridge
      name: './kivio-dsh-bridge.mjs'
"#,
            &mut entries,
        )
        .unwrap();
        assert!(entries.get("timer").is_some_and(|entry| entry.enabled));
        assert!(entries.get("tool-bash").is_some_and(|entry| !entry.enabled));
        assert_eq!(
            entries
                .get("kivio-dsh-jsonrpc-bridge")
                .map(|entry| entry.module_name.as_str()),
            Some("./kivio-dsh-bridge.mjs")
        );
        assert!(entries.contains_key("tool-web"));
    }

    #[test]
    fn write_credential_skips_empty_key() {
        assert!(write_credential(DEFAULT_API_KEY_ENV, "  ").is_ok());
    }

    fn assert_no_top_level_env_keys(text: &str) {
        assert!(
            !text
                .lines()
                .any(|line| line.starts_with("DEEPSEEK_API_KEY:")
                    || line.starts_with("OTHER_KEY:")
                    || line.starts_with("GPT_API_KEY:")),
            "dsh credentials must not put env names at the YAML root:\n{text}"
        );
    }

    #[test]
    fn official_credential_writes_deepseek_key_and_keeps_neighbors() {
        let dir = std::env::temp_dir().join(format!("kivio-dsh-cred-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".credentials.yaml");
        std::fs::write(&path, "OTHER_KEY: keep-me\n").unwrap();
        write_credential_at(&path, DEFAULT_API_KEY_ENV, "sk-test").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("version:"));
        assert!(text.contains("refs:"));
        assert!(text.contains("DEEPSEEK_API_KEY"));
        assert!(text.contains("sk-test"));
        assert!(text.contains("OTHER_KEY"));
        assert_no_top_level_env_keys(&text);
        assert!(credentials_contain(&path, DEFAULT_API_KEY_ENV));
        assert_eq!(
            credential_value(&path, "OTHER_KEY").as_deref(),
            Some("keep-me")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versioned_credential_edits_preserve_foreign_entries_and_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".credentials.yaml");
        let untouched = "# foreign document metadata\nforeign: {nested: value}\nDEEPSEEK_API_KEY: ignored\nrecords: {}\n";
        std::fs::write(
            &path,
            format!("version: 1\n{untouched}refs:\n  NEIGHBOR: keep\n"),
        )
        .unwrap();
        assert!(!credentials_contain(&path, DEFAULT_API_KEY_ENV));
        write_credential_at(&path, DEFAULT_API_KEY_ENV, "new-key").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(untouched));
        assert_eq!(credential_value(&path, "NEIGHBOR").as_deref(), Some("keep"));
        assert_eq!(
            credential_value(&path, DEFAULT_API_KEY_ENV).as_deref(),
            Some("new-key")
        );
    }

    #[test]
    fn credentials_contain_reads_refs_layout() {
        let dir = std::env::temp_dir().join(format!("kivio-dsh-refs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".credentials.yaml");
        std::fs::write(&path, "version: 1\nrefs:\n  DEEPSEEK_API_KEY: sk-yaml\n").unwrap();
        assert!(credentials_contain(&path, DEFAULT_API_KEY_ENV));
        assert_eq!(
            credential_value(&path, DEFAULT_API_KEY_ENV).as_deref(),
            Some("sk-yaml")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_provider_detail_reads_models_and_key_without_touching_neighbors() {
        let dir = std::env::temp_dir().join(format!("kivio-dsh-native-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.yaml");
        let credentials = dir.join(".credentials.yaml");
        std::fs::write(
            &settings,
            r#"
ui-onboarding:
  welcomeNoticeVersion: keep-me
agent-default-model:
  provider: gpt
  model: gpt-5.6-sol
llm-pi-ai:
  providers:
    gpt:
      displayName: gpt
      apiKeyEnv: GPT_API_KEY
      api: openai-responses
      baseURL: https://relay.example/v1
      models:
        - id: gpt-5.6-luna
          name: gpt-5.6-luna
        - id: gpt-5.6-sol
          name: gpt-5.6-sol
        - id: ""
        - {}
"#,
        )
        .unwrap();
        std::fs::write(&credentials, "GPT_API_KEY: sk-test\nOTHER_KEY: keep-me\n").unwrap();

        let detail = native_provider_detail_at(&settings, &credentials, "gpt").unwrap();
        assert_eq!(detail.id, "gpt");
        assert_eq!(detail.name, "gpt");
        assert_eq!(detail.base_url, "https://relay.example/v1");
        assert_eq!(detail.api, "openai-responses");
        assert_eq!(detail.api_key, "sk-test");
        assert_eq!(detail.api_key_env, "GPT_API_KEY");
        assert_eq!(detail.default_model, "gpt-5.6-sol");
        assert_eq!(
            detail.models,
            vec![
                DshNativeProviderModel {
                    id: "gpt-5.6-luna".into(),
                    name: "gpt-5.6-luna".into(),
                },
                DshNativeProviderModel {
                    id: "gpt-5.6-sol".into(),
                    name: "gpt-5.6-sol".into(),
                },
            ]
        );
        assert!(
            native_provider_detail_at(&settings, &credentials, DSH_OFFICIAL_PROVIDER_ID).is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn native_provider_delete_removes_route_and_stale_default() {
        let dir = std::env::temp_dir().join(format!("kivio-dsh-del-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.yaml");
        std::fs::write(
            &settings,
            r#"
ui-onboarding:
  welcomeNoticeVersion: keep-me
agent-default-model:
  provider: gpt
  model: gpt-5.6-sol
llm-pi-ai:
  providers:
    gpt:
      displayName: gpt
      api: openai-responses
    keep:
      displayName: Keep
"#,
        )
        .unwrap();

        delete_native_provider_at(&settings, "gpt").unwrap();
        let text = std::fs::read_to_string(&settings).unwrap();
        assert!(text.contains("welcomeNoticeVersion"));
        assert!(text.contains("Keep"));
        assert!(!text.contains("displayName: gpt"));
        assert!(!text.contains("agent-default-model"));
        assert!(delete_native_provider_at(&settings, DSH_OFFICIAL_PROVIDER_ID).is_err());
        delete_native_provider_at(&settings, "missing").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kivio_model_caps_write_existing_settings_route_only() {
        let dir = std::env::temp_dir().join(format!("kivio-dsh-caps-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.yaml");
        std::fs::write(
            &settings,
            r#"
ui-onboarding:
  welcomeNoticeVersion: keep-me
llm-pi-ai:
  providers:
    gpt:
      displayName: gpt
      api: openai-responses
      models:
        - id: gpt-5.6-sol
          name: gpt-5.6-sol
        - id: gpt-image-2
          name: gpt-image-2
    keep:
      displayName: Keep
"#,
        )
        .unwrap();

        sync_kivio_model_capabilities_at(
            &settings,
            &[ExternalCliProvider {
                id: "p-dsh-gpt".into(),
                name: "gpt".into(),
                native_provider_id: "gpt".into(),
                config_json: r#"{
                    "models": [
                        {
                            "id":"gpt-5.6-sol",
                            "input":["text","image"],
                            "reasoningEfforts":{"off":null,"low":"low","high":"high","xhigh":"xhigh","max":"max"}
                        },
                        {
                            "id":"gpt-image-2",
                            "input":["text","image"],
                            "reasoningEfforts":false
                        }
                    ]
                }"#
                .into(),
                ..Default::default()
            }],
        )
        .unwrap();

        let text = std::fs::read_to_string(&settings).unwrap();
        assert!(text.contains("welcomeNoticeVersion"));
        assert!(text.contains("Keep"));
        assert!(text.contains("defaultInput"));
        assert!(text.contains("gpt-5.6-sol"));
        assert!(text.contains("reasoningEfforts"));
        assert!(text.contains("xhigh"));
        assert!(text.matches("image").count() >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dotenv_parser_accepts_export_quotes_and_skips_empty() {
        assert!(dotenv_has_key(
            "DEEPSEEK_API_KEY=sk-test\n",
            "DEEPSEEK_API_KEY"
        ));
        assert!(dotenv_has_key(
            "export DEEPSEEK_API_KEY=sk-test\n",
            "DEEPSEEK_API_KEY"
        ));
        assert!(dotenv_has_key(
            "DEEPSEEK_API_KEY=\"sk-test\"\n",
            "DEEPSEEK_API_KEY"
        ));
        assert!(dotenv_has_key(
            "DEEPSEEK_API_KEY='sk-test'\n",
            "DEEPSEEK_API_KEY"
        ));
        assert!(!dotenv_has_key("DEEPSEEK_API_KEY=\n", "DEEPSEEK_API_KEY"));
        assert!(!dotenv_has_key(
            "DEEPSEEK_API_KEY=\"\"\n",
            "DEEPSEEK_API_KEY"
        ));
        assert!(!dotenv_has_key(
            "# DEEPSEEK_API_KEY=sk-test\n",
            "DEEPSEEK_API_KEY"
        ));
        assert!(!dotenv_has_key("OTHER=sk-test\n", "DEEPSEEK_API_KEY"));
    }

    #[test]
    fn pi_ai_api_key_env_reads_settings_route() {
        let yaml = r#"
llm-pi-ai:
  providers:
    gpt:
      displayName: gpt
      apiKeyEnv: OPENAI_API_KEY
"#;
        assert_eq!(
            credential_ref_in(&serde_yaml::from_str(yaml).unwrap(), "gpt").as_deref(),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(
            credential_ref_in(&serde_yaml::from_str(yaml).unwrap(), "missing"),
            None
        );
        assert_eq!(credential_ref_in(&Mapping::new(), "gpt"), None);
    }

    #[test]
    fn credential_status_accepts_dotenv_when_yaml_missing() {
        let dir = std::env::temp_dir().join(format!("kivio-dsh-env-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let user_env = dir.join(".env");
        std::fs::write(&user_env, "DEEPSEEK_API_KEY=sk-from-home\n").unwrap();
        let (configured, writable) =
            credential_status_with_files("DEEPSEEK_API_KEY", None, Some(&user_env), None);
        assert!(configured);
        assert!(writable);

        let project_env = dir.join("project.env");
        std::fs::write(&project_env, "GPT_KEY=sk-project\n").unwrap();
        let (configured, writable) =
            credential_status_with_files("GPT_KEY", None, None, Some(&project_env));
        assert!(configured);
        assert!(writable);

        let credentials = dir.join(".credentials.yaml");
        std::fs::write(&credentials, "DEEPSEEK_API_KEY: sk-yaml\n").unwrap();
        let (configured, writable) =
            credential_status_with_files("DEEPSEEK_API_KEY", Some(&credentials), None, None);
        assert!(configured);
        assert!(writable);

        let (configured, _) = credential_status_with_files(
            "MISSING_KEY",
            Some(&credentials),
            Some(&user_env),
            Some(&project_env),
        );
        assert!(!configured);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn official_deepseek_key_ready_matches_credential_status() {
        assert_eq!(
            official_deepseek_key_ready(),
            credential_status(&official_credential_ref()).0
        );
    }
}
