//! 第三方供应商（中转站）的落地层：把设置页里选中的供应商变成**子进程能看见的东西**。
//!
//! Claude / Codex 继续使用 Kivio 私有配置；OpenCode / Pi / Grok 按各自官方约定写入原生配置：
//!
//! 1. **环境变量** —— claude / gemini / 其余 env 系直接注入 `provider.env`
//!    （出口是 `overrides::env_for` → `spawn::agent_cli_command` / `cli_command`）。
//! 2. **claude 的 `--settings` 压制** —— 光注入环境变量**不够**：Claude Code 会把
//!    `~/.claude/settings.json` 的 `env` 段注入自己进程，盖掉继承来的同名变量。用户那份
//!    文件通常已被 cc-switch 写满了 `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`，
//!    于是「在 Kivio 里选了供应商却还是走老中转站」。所以额外物化一份只含 `{"env": …}`
//!    的文件用 `--settings` 传进去，并把本供应商**没设的路由键补成空串**显式压掉。
//!    聊天里选的模型按 cc-switch live 口径写入 `~/.claude/settings.json`：顶层 `model` +
//!    `env.ANTHROPIC_MODEL`（能认出档位时再写对应 `ANTHROPIC_DEFAULT_*_MODEL`）。凭证 /
//!    路由键仍不改用户文件。
//! 3. **codex 的私有 `CODEX_HOME`** —— codex 的 base_url 只能来自 `config.toml`，没有
//!    环境变量通道。物化一个私有 home（config.toml + auth.json）后注入 `CODEX_HOME`，
//!    用户自己的 `~/.codex` 凭证与供应商表不动。聊天里选的模型按 cc-switch live 口径写入
//!    `~/.codex/config.toml` 顶层 `model`（文件已存在时），并同步 CLI 正在读的那份
//!    （挂了中转 = 私有 home）。**会话 jsonl** 仍接到用户 `~/.codex/sessions`，这样
//!    Codex CLI / TUI 的 `/resume` 能看见 Kivio 里聊过的原生会话。WSL 二进制在没有
//!    私有 home 时直接把 `CODEX_HOME` 指到 Windows 的 `~/.codex`。
//! 4. **opencode / pi 的原生配置** —— 字段级合并 Kivio 管理的 provider、凭据与默认模型；
//!    其他 provider 和顶层设置原样保留。切回「CLI 自身配置」时恢复 Kivio 接管前的默认模型。
//! 5. **grok 的 `~/.grok/config.toml`** —— 与 cc-switch 一样落盘（Grok 没有 env 通道，
//!    base_url 只能写进 config.toml）。把供应商的 `config_toml` 里的 `[models]` /
//!    `[model.*]` 合并进现有文件，marketplace / ui / cli 等用户段原样保留；首次接管前
//!    整份备份，切回「CLI 自身配置」时还原。
//! 6. **kimi 的 `~/.kimi-code/config.toml`** —— Kimi Code CLI 凭证只认 config.toml（不读
//!    shell 环境变量）。把供应商片段里的 `default_model` / `[providers.*]` / `[models.*]`
//!    合并进现有文件；`managed:kimi-code` OAuth 段与其它用户配置原样保留。首次接管前
//!    整份备份，切回「CLI 自身配置」时还原。
//! 7. **dsh 的 Kivio 私有 profile** —— provider 本体由 `dsh_profile.rs` 写进
//!    `profiles/kivio/cordis.patch.yml` 的 `llm-pi-ai.providers`；本层把 `apiKeyEnv`
//!    指向的 Key 注入 Kivio 启动的进程。`settings.yaml` 只回写已存在路由上的模型
//!    `input` / `defaultInput` / `reasoningEfforts`（官方 web 贴图与 effort 门控），
//!    不新建供应商、不写密钥。
//!
//! 设置保存事务负责物化与删除，并在设置文件写入失败时恢复精确文件原貌，
//! 不是每轮。ccgui 用的是 per-turn 临时目录 + `Drop` 删除，那套在 Kivio 会把常驻 claude
//! 会话中途要读的文件删掉。代价是文件长期留在 app data 里（0600，与 settings.json 里
//! 本来就明文存 key 同一威胁模型）。
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::hash::BuildHasher;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::external_agents::registry::get_agent_def;
use crate::external_agents::types::{ProviderProfileStrategy, RuntimeAgentDef};
use crate::settings::{ExternalCliAgentConfig, ExternalCliProvider};

/// 设置保存与删除命令可能并发触发；三份原生文件必须作为一个逻辑事务串行合并。
static NATIVE_CONFIG_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone)]
struct NativePaths {
    config: PathBuf,
    alternate_configs: Vec<PathBuf>,
    auth: PathBuf,
    settings: Option<PathBuf>,
    state: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct NativeManagedState {
    managed_provider_ids: Vec<String>,
    defaults_managed: bool,
    previous_defaults: HashMap<String, BackedUpField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct BackedUpField {
    present: bool,
    value: Value,
}

impl Default for BackedUpField {
    fn default() -> Self {
        Self {
            present: false,
            value: Value::Null,
        }
    }
}

/// claude 的「路由键」：决定请求打到哪、用哪个模型的那些环境变量。
///
/// 用途有二：物化 `--settings` 时把**没设的**补成空串（否则用户 `~/.claude/settings.json`
/// 里的同名键会漏进来，出现「base_url 是新的、模型名还是旧供应商的」这种半切换）；
/// 以及注入前先从子进程环境里删一遍，清掉父进程残留。
pub const CLAUDE_ROUTING_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_MODEL",
    // 2.1.236：新会话默认模型（`/model` 仍覆盖并持久化）。不列入路由键的话，
    // 切供应商时会留下上一套的默认模型。
    // **不要**放进 `CLAUDE_CHAT_MODEL_ENV_KEYS`：那一轮会从 overlay / live
    // settings 把空串盖回去，路由空值等于没写。
    "ANTHROPIC_DEFAULT_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
];

/// 聊天选模相关的 env 键：供应商没给值时，从已有 overlay / live settings 补回来。
///
/// `ANTHROPIC_DEFAULT_MODEL` 不在这里：它是新会话默认，不是当前聊天选中的模型。
/// 放进来会把路由键刚补的空串盖掉，切供应商后 Auto 仍走上一套中转的默认模型。
const CLAUDE_CHAT_MODEL_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
];

pub(crate) struct NativeFileUpdate {
    _resource: super::session::live::ResourceLease,
    _lock: std::sync::MutexGuard<'static, ()>,
}
pub(crate) fn native_file_update(path: &Path) -> Result<NativeFileUpdate, String> {
    let resource = super::session::live::ResourceLease::acquire(
        vec![format!(
            "profile:{}",
            super::session::live::resource_path(path)
        )],
        true,
    )?;
    let lock = NATIVE_CONFIG_LOCK
        .lock()
        .map_err(|_| "原生 CLI 配置写锁已损坏")?;
    Ok(NativeFileUpdate {
        _resource: resource,
        _lock: lock,
    })
}

fn effective_env_path(
    def: &RuntimeAgentDef,
    config: &ExternalCliAgentConfig,
    key: &str,
) -> Option<PathBuf> {
    super::overrides::env_for_config(def.id, Some(config))
        .iter()
        .find(|(name, value)| {
            !value.is_empty()
                && if cfg!(windows) {
                    name.eq_ignore_ascii_case(key)
                } else {
                    name.as_str() == key
                }
        })
        .map(|(_, value)| PathBuf::from(value))
        .or_else(|| {
            def.env
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| PathBuf::from(value))
        })
        .or_else(|| nonempty_env_path(key))
}
fn effective_user_home(def: &RuntimeAgentDef, config: &ExternalCliAgentConfig) -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    effective_env_path(def, config, key)
        .or_else(|| effective_env_path(def, config, "HOME"))
        .or_else(|| directories::BaseDirs::new().map(|base| base.home_dir().to_path_buf()))
}
fn bound_state(path: PathBuf, target: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(super::session::live::resource_path(target).as_bytes());
    path.with_file_name(format!(
        "{}-{digest:x}.json",
        path.file_stem().unwrap_or_default().to_string_lossy()
    ))
}

fn effective_native_paths(
    def: &RuntimeAgentDef,
    config: &ExternalCliAgentConfig,
) -> Option<NativePaths> {
    let mut paths = match def.provider_profile {
        ProviderProfileStrategy::OpenCode => opencode_paths_in(
            Some(
                effective_env_path(def, config, "XDG_CONFIG_HOME")
                    .unwrap_or(effective_user_home(def, config)?.join(".config")),
            ),
            Some(
                effective_env_path(def, config, "XDG_DATA_HOME")
                    .unwrap_or(effective_user_home(def, config)?.join(".local/share")),
            ),
        )?,
        ProviderProfileStrategy::Pi => pi_paths_in(
            effective_env_path(def, config, "PI_CODING_AGENT_DIR")
                .unwrap_or(effective_user_home(def, config)?.join(".pi/agent")),
        )?,
        _ => return None,
    };
    paths.state = bound_state(paths.state, &paths.config);
    Some(paths)
}
fn effective_toml_paths(
    def: &RuntimeAgentDef,
    config: &ExternalCliAgentConfig,
) -> Option<(PathBuf, PathBuf)> {
    let (key, fallback, state) = if def.provider_profile == ProviderProfileStrategy::Grok {
        (
            "GROK_HOME",
            effective_user_home(def, config)?.join(".grok/config.toml"),
            grok_state_path()?,
        )
    } else {
        (
            "KIMI_CODE_HOME",
            effective_user_home(def, config)?.join(".kimi-code/config.toml"),
            kimi_state_path()?,
        )
    };
    let target = effective_env_path(def, config, key)
        .map(|home| home.join("config.toml"))
        .unwrap_or(fallback);
    Some((target.clone(), bound_state(state, &target)))
}
pub(super) fn effective_dsh_path(config: &ExternalCliAgentConfig) -> Option<PathBuf> {
    let def = get_agent_def("dsh")?;
    effective_env_path(def, config, "DSH_HOME")
        .or_else(|| effective_user_home(def, config).map(|home| home.join(".dsh")))
        .map(|home| home.join("settings.yaml"))
}

fn validate_private_target(path: &Path) -> Result<(), String> {
    let Some(root) = profiles_dir() else {
        return Ok(());
    };
    if !path.starts_with(&root) {
        return Ok(());
    }
    for ancestor in path
        .ancestors()
        .take_while(|ancestor| ancestor.starts_with(&root))
    {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() || is_reparse_point(ancestor) => {
                return Err(format!("拒绝链接配置路径：{}", ancestor.display()))
            }
            Ok(metadata) if ancestor == path && !metadata.is_file() => {
                return Err(format!("配置路径不是文件：{}", path.display()))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}
fn remove_profile_file(path: &Path) -> Result<(), String> {
    validate_private_target(path)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除 {} 失败：{error}", path.display())),
    }
}

struct FilePreimage {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
    permissions: Option<std::fs::Permissions>,
}
pub(crate) struct ProfileUndo {
    files: Vec<FilePreimage>,
    new_links: Vec<PathBuf>,
    new_dirs: Vec<PathBuf>,
    _lock: std::sync::MutexGuard<'static, ()>,
    resource: Option<super::session::live::ResourceLease>,
}
impl ProfileUndo {
    fn capture(
        paths: Vec<PathBuf>,
        links: Vec<PathBuf>,
        lock: std::sync::MutexGuard<'static, ()>,
    ) -> Result<Self, String> {
        let mut files = Vec::new();
        let mut new_dirs = Vec::new();
        let mut seen = HashSet::new();
        for path in paths {
            if !seen.insert(path.clone()) {
                continue;
            }
            validate_private_target(&path)?;
            let (bytes, permissions) = match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => (
                    Some(std::fs::read(&path).map_err(|e| e.to_string())?),
                    Some(metadata.permissions()),
                ),
                Ok(_) => return Err(format!("配置路径不是普通文件：{}", path.display())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (None, None),
                Err(error) => return Err(error.to_string()),
            };
            for parent in path
                .ancestors()
                .skip(1)
                .take_while(|parent| !parent.exists())
            {
                if !new_dirs.contains(&parent.to_path_buf()) {
                    new_dirs.push(parent.to_path_buf());
                }
            }
            files.push(FilePreimage {
                path,
                bytes,
                permissions,
            });
        }
        Ok(Self {
            files,
            new_links: links
                .into_iter()
                .filter(|path| std::fs::symlink_metadata(path).is_err())
                .collect(),
            new_dirs,
            _lock: lock,
            resource: super::session::live::MAINTENANCE_OWNER
                .try_with(Clone::clone)
                .ok(),
        })
    }
    pub(crate) fn rollback(self) -> Result<(), String> {
        let mut errors = Vec::new();
        for file in self.files.iter().rev() {
            let result = match &file.bytes {
                Some(bytes) => (|| {
                    validate_private_target(&file.path)?;
                    write_private_bytes(&file.path, bytes)?;
                    if let Some(permissions) = &file.permissions {
                        std::fs::set_permissions(&file.path, permissions.clone())
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })(),
                None => remove_profile_file(&file.path),
            };
            if let Err(error) = result {
                errors.push(error);
            }
        }
        for link in &self.new_links {
            if is_reparse_point(link) {
                unlink_directory_link(link);
            }
        }
        for dir in &self.new_dirs {
            let _ = std::fs::remove_dir(dir);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            *FAILED_RECOVERY.lock().unwrap_or_else(|e| e.into_inner()) = Some(ProfileRecovery {
                files: self.files,
                new_links: self.new_links,
                new_dirs: self.new_dirs,
                resource: self.resource,
            });
            Err(format!(
                "凭据回滚失败，已阻止受影响 CLI 启动；重试保存以恢复：{}",
                errors.join("; ")
            ))
        }
    }
}
struct ProfileRecovery {
    files: Vec<FilePreimage>,
    new_links: Vec<PathBuf>,
    new_dirs: Vec<PathBuf>,
    resource: Option<super::session::live::ResourceLease>,
}
static FAILED_RECOVERY: LazyLock<Mutex<Option<ProfileRecovery>>> = LazyLock::new(Default::default);
pub(crate) fn retry_profile_recovery() -> Result<(), String> {
    if FAILED_RECOVERY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_none()
    {
        return Ok(());
    }
    let lock = NATIVE_CONFIG_LOCK.lock().map_err(|_| "原生配置锁不可用")?;
    let recovery = FAILED_RECOVERY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    if let Some(recovery) = recovery {
        ProfileUndo {
            files: recovery.files,
            new_links: recovery.new_links,
            new_dirs: recovery.new_dirs,
            resource: recovery.resource,
            _lock: lock,
        }
        .rollback()?;
    }
    Ok(())
}

pub(crate) fn changed_agents(
    previous: &crate::settings::Settings,
    next: &crate::settings::Settings,
) -> Vec<String> {
    super::registry::AGENT_DEFS
        .iter()
        .filter(|def| {
            previous.chat.external_cli_agents.get(def.id)
                != next.chat.external_cli_agents.get(def.id)
        })
        .map(|def| def.id.to_string())
        .collect()
}

fn legacy_native_paths(def: &RuntimeAgentDef) -> Option<NativePaths> {
    let paths = match def.provider_profile {
        ProviderProfileStrategy::OpenCode => opencode_paths()?,
        ProviderProfileStrategy::Pi => {
            let home = nonempty_env_path("PI_CODING_AGENT_DIR").or_else(|| {
                directories::BaseDirs::new().map(|base| base.home_dir().join(".pi/agent"))
            })?;
            pi_paths_in(home)?
        }
        _ => return None,
    };
    if paths.state.exists()
        && read_managed_state(&paths.state)
            .is_ok_and(|state| state.managed_provider_ids.is_empty() && !state.defaults_managed)
    {
        return None;
    }
    paths.state.exists().then_some(paths)
}
fn legacy_toml_paths(def: &RuntimeAgentDef) -> Option<(PathBuf, PathBuf)> {
    let pair = match def.provider_profile {
        ProviderProfileStrategy::Grok => (grok_config_path()?, grok_state_path()?),
        ProviderProfileStrategy::Kimi => (kimi_config_path()?, kimi_state_path()?),
        _ => return None,
    };
    let inactive = if def.provider_profile == ProviderProfileStrategy::Grok {
        read_grok_state(&pair.1).is_ok_and(|state| !state.managed)
    } else {
        read_kimi_state(&pair.1).is_ok_and(|state| !state.managed)
    };
    (pair.1.exists() && !inactive).then_some(pair)
}
fn legacy_profile_files(def: &RuntimeAgentDef) -> Vec<PathBuf> {
    if let Some(paths) = legacy_native_paths(def) {
        let mut files = vec![paths.config, paths.auth, paths.state];
        files.extend(paths.settings);
        files.extend(paths.alternate_configs);
        files
    } else if let Some((path, state)) = legacy_toml_paths(def) {
        vec![path, state]
    } else {
        Vec::new()
    }
}

fn profile_paths(
    def: &RuntimeAgentDef,
    config: &ExternalCliAgentConfig,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let mut paths = Vec::new();
    let mut links = Vec::new();
    match def.provider_profile {
        ProviderProfileStrategy::Claude | ProviderProfileStrategy::Codex => {
            for provider in &config.providers {
                if def.provider_profile == ProviderProfileStrategy::Claude {
                    paths.push(claude_settings_path_for(&provider.id).ok_or("非法供应商 id")?);
                } else {
                    let home = codex_home_for(&provider.id).ok_or("非法供应商 id")?;
                    paths.extend([home.join("config.toml"), home.join("auth.json")]);
                    links.push(home.join("sessions"));
                }
            }
        }
        ProviderProfileStrategy::OpenCode | ProviderProfileStrategy::Pi => {
            let native = effective_native_paths(def, config).ok_or("无法定位原生配置")?;
            paths.extend([native.config, native.auth, native.state]);
            paths.extend(native.alternate_configs);
            paths.extend(native.settings);
        }
        ProviderProfileStrategy::Grok | ProviderProfileStrategy::Kimi => {
            let (target, state) = effective_toml_paths(def, config).ok_or("无法定位原生配置")?;
            paths.extend([target, state]);
        }
        ProviderProfileStrategy::Dsh => paths.extend(effective_dsh_path(config)),
        _ => {}
    }
    Ok((paths, links))
}

pub(crate) fn config_resource_keys(
    def: &RuntimeAgentDef,
    config: &ExternalCliAgentConfig,
) -> Vec<String> {
    profile_paths(def, config)
        .map(|(paths, _)| {
            paths
                .into_iter()
                .map(|path| format!("profile:{}", super::session::live::resource_path(&path)))
                .collect()
        })
        .unwrap_or_default()
}
pub(crate) fn settings_resource_keys(
    previous: &crate::settings::Settings,
    next: &crate::settings::Settings,
) -> Vec<String> {
    let empty = ExternalCliAgentConfig::default();
    let mut keys = Vec::new();
    for agent in changed_agents(previous, next) {
        keys.push(format!("agent:{agent}"));
        if let Some(def) = get_agent_def(&agent) {
            keys.extend(
                legacy_profile_files(def)
                    .into_iter()
                    .map(|path| format!("profile:{}", super::session::live::resource_path(&path))),
            );
            for settings in [previous, next] {
                keys.extend(config_resource_keys(
                    def,
                    settings
                        .chat
                        .external_cli_agents
                        .get(&agent)
                        .unwrap_or(&empty),
                ));
            }
        }
    }
    keys
}

pub(crate) fn persist_with_profiles(
    previous: &crate::settings::Settings,
    next: &crate::settings::Settings,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let profiles = apply_settings_profiles(previous, next)?;
    match persist() {
        Ok(()) => Ok(()),
        Err(error) => match profiles.rollback() {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!("{error}; {rollback}")),
        },
    }
}

fn apply_settings_profiles(
    previous: &crate::settings::Settings,
    next: &crate::settings::Settings,
) -> Result<ProfileUndo, String> {
    let lock = NATIVE_CONFIG_LOCK
        .lock()
        .map_err(|_| "原生 CLI 配置写锁已损坏")?;
    let empty = ExternalCliAgentConfig::default();
    let agents = changed_agents(previous, next);
    let mut paths = Vec::new();
    let mut links = Vec::new();
    for id in &agents {
        let def = get_agent_def(id).ok_or("未知外部 Agent")?;
        let before = previous.chat.external_cli_agents.get(id).unwrap_or(&empty);
        let after = next.chat.external_cli_agents.get(id).unwrap_or(&empty);
        if let Some(legacy) = legacy_native_paths(def)
            .map(|paths| paths.state)
            .or_else(|| legacy_toml_paths(def).map(|(_, state)| state))
        {
            return Err(format!("旧版供应商备份缺少原始目录记录，无法安全迁移。请先确认原安装目录并恢复备份，再移除旧备份记录后重试：{}", legacy.display()));
        }
        if cfg!(windows)
            && (after.path.starts_with('/') || super::wsl::is_wsl_target(Path::new(&after.path)))
            && !after.providers.is_empty()
            && !matches!(
                def.provider_profile,
                ProviderProfileStrategy::Claude
                    | ProviderProfileStrategy::Codex
                    | ProviderProfileStrategy::Environment
            )
        {
            return Err("WSL 原生供应商配置请在发行版中维护".into());
        }
        if matches!(
            def.provider_profile,
            ProviderProfileStrategy::OpenCode | ProviderProfileStrategy::Pi
        ) {
            parse_native_entries_for(def.provider_profile, &after.providers)?;
        }
        if def.provider_profile == ProviderProfileStrategy::Codex {
            if let Some(provider) = super::overrides::selected_provider(after) {
                let private_home = codex_home_for(&provider.id).ok_or("非法供应商 id")?;
                for pair in &after.env {
                    let is_codex_home = if cfg!(windows) {
                        pair.key.eq_ignore_ascii_case("CODEX_HOME")
                    } else {
                        pair.key == "CODEX_HOME"
                    };
                    if is_codex_home
                        && super::session::live::resource_path(Path::new(&pair.value))
                            != super::session::live::resource_path(&private_home)
                    {
                        return Err("私有 Codex 供应商不能同时覆盖 CODEX_HOME。请移除该环境变量，或切换到 CLI 自身配置模式".into());
                    }
                }
            }
        }
        for provider in &after.providers {
            if !provider.config_toml.trim().is_empty() {
                toml::from_str::<toml::Value>(&provider.config_toml).map_err(|e| e.to_string())?;
            }
            if !provider.auth_json.trim().is_empty() {
                let auth: Value =
                    serde_json::from_str(&provider.auth_json).map_err(|e| e.to_string())?;
                if !auth.is_object() {
                    return Err("auth.json 必须是对象".into());
                }
            }
        }
        for config in [before, after] {
            if cfg!(windows)
                && matches!(
                    def.provider_profile,
                    ProviderProfileStrategy::OpenCode
                        | ProviderProfileStrategy::Pi
                        | ProviderProfileStrategy::Grok
                        | ProviderProfileStrategy::Kimi
                        | ProviderProfileStrategy::Dsh
                )
            {
                let names: std::collections::HashSet<_> =
                    config.env.iter().map(|pair| &pair.key).collect();
                for key in [
                    "HOME",
                    "USERPROFILE",
                    "XDG_CONFIG_HOME",
                    "XDG_DATA_HOME",
                    "PI_CODING_AGENT_DIR",
                    "GROK_HOME",
                    "KIMI_CODE_HOME",
                    "DSH_HOME",
                ] {
                    if names
                        .iter()
                        .filter(|name| name.eq_ignore_ascii_case(key))
                        .count()
                        > 1
                    {
                        return Err(format!(
                            "Windows 环境变量 {key} 使用了多种大小写，请统一变量名后重试"
                        ));
                    }
                }
            }
            let (files, session_links) = profile_paths(def, config)?;
            if files.iter().any(|path| !path.is_absolute()) {
                return Err("原生 CLI 配置目录必须使用绝对路径".into());
            }
            paths.extend(files);
            links.extend(session_links);
        }
    }
    let undo = ProfileUndo::capture(paths, links, lock)?;
    let result = (|| {
        for id in &agents {
            let def = get_agent_def(id).ok_or("未知外部 Agent")?;
            let before = previous.chat.external_cli_agents.get(id).unwrap_or(&empty);
            let after = next.chat.external_cli_agents.get(id).unwrap_or(&empty);
            if profile_paths(def, before)?.0 != profile_paths(def, after)?.0
                && matches!(
                    def.provider_profile,
                    ProviderProfileStrategy::OpenCode
                        | ProviderProfileStrategy::Pi
                        | ProviderProfileStrategy::Grok
                        | ProviderProfileStrategy::Kimi
                )
            {
                let restore = ExternalCliAgentConfig::default();
                match def.provider_profile {
                    ProviderProfileStrategy::OpenCode | ProviderProfileStrategy::Pi => {
                        materialize_native_with_strategy_at(
                            def.provider_profile,
                            id,
                            &restore,
                            &effective_native_paths(def, before).ok_or("无法定位先前配置")?,
                        )?
                    }
                    ProviderProfileStrategy::Grok | ProviderProfileStrategy::Kimi => {
                        let (path, state) =
                            effective_toml_paths(def, before).ok_or("无法定位先前配置")?;
                        if def.provider_profile == ProviderProfileStrategy::Grok {
                            materialize_grok_at(&restore, &path, &state)?;
                        } else {
                            materialize_kimi_at(&restore, &path, &state)?;
                        }
                    }
                    _ => {}
                }
            }
            materialize_config(id, after)?;
            for provider in &before.providers {
                if after.providers.iter().any(|item| item.id == provider.id) {
                    continue;
                }
                match def.provider_profile {
                    ProviderProfileStrategy::Claude => remove_profile_file(
                        &claude_settings_path_for(&provider.id).ok_or("非法供应商 id")?,
                    )?,
                    ProviderProfileStrategy::Codex => {
                        let home = codex_home_for(&provider.id).ok_or("非法供应商 id")?;
                        remove_profile_file(&home.join("config.toml"))?;
                        remove_profile_file(&home.join("auth.json"))?;
                    }
                    ProviderProfileStrategy::Dsh
                        if !provider.native_provider_id.trim().is_empty() =>
                    {
                        super::dsh_plugins::delete_native_provider_at(
                            &effective_dsh_path(before).ok_or("无法定位 dsh 配置")?,
                            &provider.native_provider_id,
                        )?
                    }
                    _ => {}
                }
            }
        }
        Ok::<_, String>(())
    })();
    if let Err(error) = result {
        return match undo.rollback() {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!("{error}; {rollback}")),
        };
    }
    Ok(undo)
}

#[cfg(test)]
thread_local! { static TEST_PROFILE_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) }; }
fn profiles_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(root) = TEST_PROFILE_ROOT.with(|root| root.borrow().clone()) {
        return Some(root);
    }
    crate::app_data::app_data_dir().map(|dir| dir.join("external-cli-providers"))
}

fn nonempty_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn default_user_home() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(root) = TEST_PROFILE_ROOT.with(|root| root.borrow().clone()) {
        return Some(root.join("home"));
    }
    directories::BaseDirs::new().map(|base| base.home_dir().to_path_buf())
}

fn opencode_paths() -> Option<NativePaths> {
    opencode_paths_in(
        nonempty_env_path("XDG_CONFIG_HOME"),
        nonempty_env_path("XDG_DATA_HOME"),
    )
}

fn opencode_paths_in(
    config_home: Option<PathBuf>,
    data_home: Option<PathBuf>,
) -> Option<NativePaths> {
    let home = default_user_home()?;
    let config_home = config_home.unwrap_or_else(|| home.join(".config"));
    let data_home = data_home.unwrap_or_else(|| home.join(".local/share"));
    let config_dir = config_home.join("opencode");
    let candidates: Vec<PathBuf> = ["config.json", "opencode.json", "opencode.jsonc"]
        .into_iter()
        .map(|name| config_dir.join(name))
        .collect();
    let config = candidates
        .iter()
        .rev()
        .find(|path| path.is_file())
        .cloned()
        .unwrap_or_else(|| config_dir.join("opencode.json"));
    let alternate_configs = candidates
        .into_iter()
        .filter(|path| path != &config && path.is_file())
        .collect();
    Some(NativePaths {
        config,
        alternate_configs,
        auth: data_home.join("opencode/auth.json"),
        settings: None,
        state: profiles_dir()?.join("opencode-native-state.json"),
    })
}

pub fn pi_agent_dir() -> Option<PathBuf> {
    let def = get_agent_def("pi")?;
    let config = super::overrides::agent_config("pi").unwrap_or_default();
    Some(
        effective_env_path(def, &config, "PI_CODING_AGENT_DIR")
            .unwrap_or(effective_user_home(def, &config)?.join(".pi/agent")),
    )
}

fn pi_paths_in(agent: PathBuf) -> Option<NativePaths> {
    Some(NativePaths {
        config: agent.join("models.json"),
        alternate_configs: Vec::new(),
        auth: agent.join("auth.json"),
        settings: Some(agent.join("settings.json")),
        state: profiles_dir()?.join("pi-native-state.json"),
    })
}

/// 供应商 id 直接当文件/目录名用，必须消毒。照 ccgui 的 `sanitize_provider_path_segment`：
/// 拒绝路径分隔符、`..`、控制字符与 Windows 保留名，越界一律返回 None 而不是「尽量修复」。
fn sanitize_segment(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() || id == "." || id == ".." || id.ends_with('.') {
        return None;
    }
    if id.chars().any(|ch| {
        ch.is_control() || matches!(ch, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
    }) {
        return None;
    }
    let upper = id.to_ascii_uppercase();
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL"];
    if RESERVED.contains(&upper.as_str())
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
    {
        return None;
    }
    Some(id.to_string())
}

fn codex_home_for(provider_id: &str) -> Option<PathBuf> {
    Some(profiles_dir()?.join(format!("codex-{}", sanitize_segment(provider_id)?)))
}

/// Codex CLI / TUI 读的那份用户 home（`~/.codex`）。Kivio 进程在 Windows 上时就是
/// `%USERPROFILE%\.codex`，与用户在终端里跑 `codex` 看到的是同一处。
pub fn native_codex_home() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(root) = TEST_PROFILE_ROOT.with(|root| root.borrow().clone()) {
        return Some(root.join("shared-codex"));
    }
    directories::BaseDirs::new().map(|base| base.home_dir().join(".codex"))
}

/// WSL 里的 Codex 若继承 Windows 环境、自己却把会话写到 Linux `~/.codex`，
/// TUI 在 Windows 上看不见。没有私有供应商 home 时，把 `CODEX_HOME` 指到用户那份。
///
/// 返回的是 **host 路径**；注入子进程时由 `spawn::apply_env_for_cli` 再翻成 `/mnt/...`。
pub fn wsl_shared_codex_home(cli_bin: &Path) -> Option<PathBuf> {
    if !crate::external_agents::wsl::is_wsl_target(cli_bin) {
        return None;
    }
    native_codex_home()
}

/// 中转私有 home 的 `sessions/` 接到用户 `~/.codex/sessions`。保存供应商时物化一次，
/// 拉起 Codex 时再补一次，这样升级前已经写在私有目录里的 rollout 也会露给 TUI。
pub fn ensure_private_codex_sessions_shared() {
    let Some(home) = provider_env("codex").get("CODEX_HOME").cloned() else {
        return;
    };
    let home = PathBuf::from(home);
    if !is_kivio_private_codex_home(&home) {
        return;
    }
    if let Err(err) = share_codex_sessions_with_user_home(&home) {
        eprintln!("[external-agent] Codex 会话目录未能接到用户 ~/.codex：{err}");
    }
}

fn claude_settings_path_for(provider_id: &str) -> Option<PathBuf> {
    Some(profiles_dir()?.join(format!("claude-{}.json", sanitize_segment(provider_id)?)))
}

fn dsh_provider_env(config: &ExternalCliAgentConfig) -> HashMap<String, String> {
    let mut env = HashMap::new();
    for provider in config
        .providers
        .iter()
        .filter(|provider| !provider.disabled)
    {
        for pair in &provider.env {
            env.insert(pair.key.clone(), pair.value.clone());
        }
    }
    // 重名环境变量无法真正并存；让默认供应商保持旧行为并取得最终优先级。
    if let Some(provider) = config
        .providers
        .iter()
        .find(|provider| provider.id == config.current_provider && !provider.disabled)
    {
        for pair in &provider.env {
            env.insert(pair.key.clone(), pair.value.clone());
        }
    }
    env
}

/// 要注入这个 CLI 子进程的供应商环境变量。dsh 合并全部并存供应商，其余 CLI 只用默认项。
pub fn provider_env(agent_id: &str) -> HashMap<String, String> {
    super::overrides::agent_config(agent_id)
        .map(|config| provider_env_for(agent_id, &config))
        .unwrap_or_default()
}

pub(super) fn provider_env_for(
    agent_id: &str,
    config: &ExternalCliAgentConfig,
) -> HashMap<String, String> {
    let Some(def) = get_agent_def(agent_id) else {
        return HashMap::new();
    };
    if matches!(def.provider_profile, ProviderProfileStrategy::Dsh) {
        return dsh_provider_env(config);
    }

    let Some(provider) = super::overrides::selected_provider(config) else {
        return HashMap::new();
    };
    if matches!(def.provider_profile, ProviderProfileStrategy::Codex) {
        // codex 读不到 base_url 环境变量，只认 config.toml；私有 home 是唯一通道。
        return match codex_home_for(&provider.id) {
            Some(home) => HashMap::from([(
                "CODEX_HOME".to_string(),
                home.to_string_lossy().into_owned(),
            )]),
            None => HashMap::new(),
        };
    }
    provider
        .env
        .iter()
        .map(|pair| (pair.key.clone(), pair.value.clone()))
        .collect()
}

pub(super) fn launch_profile_changed(
    strategy: ProviderProfileStrategy,
    before: &ExternalCliAgentConfig,
    after: &ExternalCliAgentConfig,
) -> bool {
    use ProviderProfileStrategy::*;

    let selected = super::overrides::selected_provider;
    if matches!(strategy, OpenCode | Pi | Dsh) {
        if selected(before).map(|provider| &provider.id)
            != selected(after).map(|provider| &provider.id)
        {
            return true;
        }
        let before = before
            .providers
            .iter()
            .filter(|provider| !provider.disabled);
        let after = after.providers.iter().filter(|provider| !provider.disabled);
        return before.clone().count() != after.clone().count()
            || before.zip(after).any(|(before, after)| {
                before.id != after.id
                    || before.native_provider_id != after.native_provider_id
                    || before.config_json != after.config_json
                    || before.default_model != after.default_model
                    || (strategy != Dsh
                        && (before.auth_json != after.auth_json
                            || resolve_native_provider_id(before).ok()
                                != resolve_native_provider_id(after).ok()))
                    || (strategy == Pi && before.default_reasoning != after.default_reasoning)
            });
    }
    match (selected(before), selected(after)) {
        (Some(before), Some(after)) => match strategy {
            Claude => before.id != after.id || before.env != after.env,
            Codex => {
                before.id != after.id
                    || before.config_toml != after.config_toml
                    || before.auth_json != after.auth_json
            }
            Grok | Kimi => before.config_toml != after.config_toml,
            _ => false,
        },
        (None, None) => false,
        _ => strategy != Environment,
    }
}

pub(super) fn launch_file_revision(
    def: &RuntimeAgentDef,
    bin: &Path,
    config: Option<&ExternalCliAgentConfig>,
    env: &BTreeMap<OsString, OsString>,
    hasher: &impl BuildHasher,
) -> u64 {
    use ProviderProfileStrategy::*;

    let env_path = |key: &str| {
        env.iter().find(|(name, _)| {
            if cfg!(windows) {
                name.to_string_lossy().eq_ignore_ascii_case(key)
            } else {
                name.as_os_str() == OsStr::new(key)
            }
        }).map(|(_, value)| value)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let wsl = super::wsl::is_wsl_target(bin);
    let mut files = Vec::new();
    match def.provider_profile {
        Codex => {
            if let Some(home) = env_path("CODEX_HOME")
                .or_else(native_codex_home)
                .filter(|home| !wsl || (home.is_absolute() && !super::wsl::is_wsl_target(home)))
            {
                files.extend([home.join("config.toml"), home.join("auth.json")]);
            }
        }
        Claude => {
            if let Some(path) = config
                .and_then(super::overrides::selected_provider)
                .and_then(|provider| claude_settings_path_for(&provider.id))
            {
                files.push(path);
            }
            if !wsl {
                let settings = env_path("CLAUDE_CONFIG_DIR")
                    .map(claude_settings_path_in)
                    .or_else(claude_native_settings_path);
                if let Some(settings) = settings {
                    files.push(settings.with_file_name(".credentials.json"));
                    files.push(settings);
                }
            }
        }
        OpenCode | Pi if !wsl => {
            let paths = if def.provider_profile == Pi {
                env_path("PI_CODING_AGENT_DIR")
                    .or_else(pi_agent_dir)
                    .and_then(pi_paths_in)
            } else {
                opencode_paths_in(env_path("XDG_CONFIG_HOME"), env_path("XDG_DATA_HOME"))
            };
            if let Some(paths) = paths {
                files.extend([paths.config, paths.auth]);
                files.extend(paths.alternate_configs);
                files.extend(paths.settings);
            }
        }
        Grok if !wsl => files.extend(
            env_path("GROK_HOME")
                .map(|home| home.join("config.toml"))
                .or_else(grok_config_path),
        ),
        Kimi if !wsl => files.extend(
            env_path("KIMI_CODE_HOME")
                .map(|home| home.join("config.toml"))
                .or_else(kimi_config_path),
        ),
        Dsh if !wsl => {
            if let Some(home) = env_path("DSH_HOME").or_else(super::dsh_profile::dsh_home) {
                files.extend([home.join(".credentials.yaml"), home.join(".env")]);
            }
        }
        _ => {}
    }

    let revisions: Vec<_> = files
        .into_iter()
        .filter_map(|path| {
            let revision = launch_config_file_revision(&path, def.provider_profile, hasher)?;
            Some((path, revision))
        })
        .collect();
    hasher.hash_one(revisions)
}

fn launch_config_file_revision(
    path: &Path,
    strategy: ProviderProfileStrategy,
    hasher: &impl BuildHasher,
) -> Option<u64> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    // shortcut: use metadata for large files. Stream a digest if a CLI starts storing large credential files.
    const LIMIT: u64 = 2 * 1024 * 1024;
    let metadata_revision = || hasher.hash_one((metadata.len(), metadata.modified().ok()));
    if metadata.len() > LIMIT {
        return Some(metadata_revision());
    }
    let mut content = Vec::new();
    if std::fs::File::open(path)
        .and_then(|file| file.take(LIMIT + 1).read_to_end(&mut content))
        .is_err()
        || content.len() as u64 > LIMIT
    {
        return Some(metadata_revision());
    }
    match (strategy, path.file_name().and_then(OsStr::to_str)) {
        (ProviderProfileStrategy::Codex, Some("config.toml")) => {
            if let Some(mut config) = std::str::from_utf8(&content)
                .ok()
                .and_then(|text| toml::from_str::<toml::Table>(text).ok())
            {
                config.remove("model");
                config.remove("model_reasoning_effort");
                return (!config.is_empty())
                    .then(|| hasher.hash_one(toml::Value::Table(config).to_string()));
            }
        }
        (ProviderProfileStrategy::Claude, Some(name)) if name != ".credentials.json" => {
            if let Ok(mut config) = serde_json::from_slice::<Map<String, Value>>(&content) {
                config.remove("model");
                if let Some(env) = config.get_mut("env").and_then(Value::as_object_mut) {
                    env.retain(|key, _| !CLAUDE_CHAT_MODEL_ENV_KEYS.contains(&key.as_str()));
                    if env.is_empty() {
                        config.remove("env");
                    }
                }
                return (!config.is_empty())
                    .then(|| hasher.hash_one(Value::Object(config).to_string()));
            }
        }
        _ => {}
    }
    Some(hasher.hash_one(content))
}

/// claude 启动时要追加的 `--settings <path>`；无供应商 / 文件没物化成功时返回 None。
pub(crate) fn claude_overlay_args() -> Vec<String> {
    claude_settings_override("claude")
        .map(|path| vec!["--settings".into(), path.to_string_lossy().into_owned()])
        .unwrap_or_default()
}

pub fn claude_settings_override(agent_id: &str) -> Option<PathBuf> {
    let def = get_agent_def(agent_id)?;
    if !matches!(def.provider_profile, ProviderProfileStrategy::Claude) {
        return None;
    }
    let provider = super::overrides::active_provider(agent_id)?;
    let path = claude_settings_path_for(&provider.id)?;
    path.is_file().then_some(path)
}

/// 给所有 CLI 物化一遍当前生效的供应商。由 `persist_settings` 在同步完镜像后调用。
/// 尝试全部定义并汇总错误，让设置事务可以回滚已经部分落地的配置。

/// 把供应商写到盘上。OpenCode / Pi / Grok / Kimi 即使当前未启用，也要同步（或恢复）原生配置。

fn materialize_config(agent_id: &str, config: &ExternalCliAgentConfig) -> Result<(), String> {
    let def = get_agent_def(agent_id).ok_or("未知外部 Agent")?;
    match def.provider_profile {
        ProviderProfileStrategy::OpenCode | ProviderProfileStrategy::Pi => {
            let paths = effective_native_paths(def, config).ok_or("无法定位原生配置目录")?;
            materialize_native_with_strategy_at(def.provider_profile, agent_id, config, &paths)
        }
        ProviderProfileStrategy::Grok | ProviderProfileStrategy::Kimi => {
            let (path, state) = effective_toml_paths(def, config).ok_or("无法定位原生配置目录")?;
            if def.provider_profile == ProviderProfileStrategy::Grok {
                materialize_grok_at(config, &path, &state)
            } else {
                materialize_kimi_at(config, &path, &state)
            }
        }
        ProviderProfileStrategy::Dsh => {
            let path = effective_dsh_path(config).ok_or("无法定位 dsh 配置目录")?;
            if !path.exists() {
                return Ok(());
            }
            let providers = config
                .providers
                .iter()
                .filter(|p| !p.disabled)
                .cloned()
                .collect::<Vec<_>>();
            super::dsh_plugins::sync_kivio_model_capabilities_at(&path, &providers)
        }
        _ => match super::overrides::selected_provider(config) {
            Some(provider) if def.provider_profile == ProviderProfileStrategy::Claude => {
                materialize_claude(provider)
            }
            Some(provider) if def.provider_profile == ProviderProfileStrategy::Codex => {
                materialize_codex(provider)
            }
            _ => Ok(()),
        },
    }
}

fn materialize_claude(provider: &ExternalCliProvider) -> Result<(), String> {
    let path = claude_settings_path_for(&provider.id)
        .ok_or_else(|| format!("供应商 id 不能作为文件名：{}", provider.id))?;
    materialize_claude_to(&path, provider)
}

fn provider_sets_env(provider: &ExternalCliProvider, key: &str) -> bool {
    provider
        .env
        .iter()
        .any(|pair| pair.key == key && !pair.value.trim().is_empty())
}

fn materialize_claude_to(path: &Path, provider: &ExternalCliProvider) -> Result<(), String> {
    let mut env: serde_json::Map<String, serde_json::Value> = provider
        .env
        .iter()
        .map(|pair| {
            (
                pair.key.clone(),
                serde_json::Value::String(pair.value.clone()),
            )
        })
        .collect();
    // 没设的路由键补空串：settings.json 的 env 段是「显式赋值」而不是「没写就沿用」，
    // 不补的话用户 ~/.claude/settings.json 里的旧值会从另一边漏进来。
    for key in CLAUDE_ROUTING_ENV_KEYS {
        env.entry((*key).to_string())
            .or_insert_with(|| serde_json::Value::String(String::new()));
    }
    let live = claude_native_settings_path();
    let previous_model = read_json_string_field(path, "model").or_else(|| {
        live.as_ref()
            .filter(|candidate| *candidate != path)
            .and_then(|candidate| read_json_string_field(candidate, "model"))
    });
    for key in CLAUDE_CHAT_MODEL_ENV_KEYS {
        if provider_sets_env(provider, key) {
            continue;
        }
        let previous = read_json_env_string(path, key).or_else(|| {
            live.as_ref()
                .filter(|candidate| *candidate != path)
                .and_then(|candidate| read_json_env_string(candidate, key))
        });
        if let Some(value) = previous {
            env.insert(key.to_string(), serde_json::Value::String(value));
        }
    }
    let previous_env_model = env
        .get("ANTHROPIC_MODEL")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mut body = serde_json::json!({ "env": env });
    // 聊天选模写在顶层 `model` / `env.ANTHROPIC_MODEL`；重物化 env 时不要把它抹掉。
    if let Some(model) = previous_model.or(previous_env_model) {
        if let Some(root) = body.as_object_mut() {
            root.insert("model".to_string(), serde_json::Value::String(model));
        }
    }
    write_private(
        path,
        &serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?,
    )
}

fn materialize_codex(provider: &ExternalCliProvider) -> Result<(), String> {
    let home = codex_home_for(&provider.id)
        .ok_or_else(|| format!("供应商 id 不能作为目录名：{}", provider.id))?;
    // 写之前先验一遍：坏 TOML 会让 codex 整个起不来，报的错还跟供应商八竿子打不着。
    toml::from_str::<toml::Value>(&provider.config_toml)
        .map_err(|e| format!("config.toml 解析失败：{e}"))?;
    if !provider.auth_json.trim().is_empty() {
        let auth: Value = serde_json::from_str(&provider.auth_json)
            .map_err(|e| format!("auth.json 解析失败：{e}"))?;
        if !auth.is_object() {
            return Err("auth.json 必须是对象".into());
        }
    }
    validate_private_target(&home.join("config.toml"))?;
    validate_private_target(&home.join("auth.json"))?;
    std::fs::create_dir_all(&home).map_err(|e| format!("创建 {} 失败：{e}", home.display()))?;
    let config_path = home.join("config.toml");
    write_codex_config_preserving_chat_model(&config_path, &provider.config_toml)?;
    let auth = provider.auth_json.trim();
    if auth.is_empty() {
        remove_profile_file(&home.join("auth.json"))?;
    } else {
        serde_json::from_str::<serde_json::Value>(auth)
            .map_err(|e| format!("auth.json 解析失败：{e}"))?;
        write_private(&home.join("auth.json"), auth)?;
    }
    let sessions = home.join("sessions");
    if std::fs::symlink_metadata(&sessions).is_err() {
        let target = native_codex_home()
            .ok_or("无法定位 Codex 会话目录")?
            .join("sessions");
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        create_directory_link(&sessions, &target)?;
    }
    Ok(())
}

/// 私有 `CODEX_HOME` 只承载中转 config/auth；把 `sessions/` 接到用户 `~/.codex/sessions`，
/// 这样 Codex CLI / TUI 默认扫描的目录里能看到 Kivio 写下的 rollout。
fn share_codex_sessions_with_user_home(private_home: &Path) -> Result<(), String> {
    let Some(user_sessions) = native_codex_home().map(|home| home.join("sessions")) else {
        return Ok(());
    };
    share_codex_sessions_dir(private_home, &user_sessions)
}

fn is_kivio_private_codex_home(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.starts_with("codex-")
        && path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            == Some("external-cli-providers")
}

fn share_codex_sessions_dir(private_home: &Path, user_sessions: &Path) -> Result<(), String> {
    std::fs::create_dir_all(user_sessions)
        .map_err(|e| format!("创建 {} 失败：{e}", user_sessions.display()))?;
    std::fs::create_dir_all(private_home)
        .map_err(|e| format!("创建 {} 失败：{e}", private_home.display()))?;
    let private_sessions = private_home.join("sessions");
    if paths_point_to_same_dir(&private_sessions, user_sessions) {
        return Ok(());
    }
    if is_reparse_point(&private_sessions) {
        unlink_directory_link(&private_sessions);
    } else if private_sessions.is_dir() {
        merge_dir_contents(&private_sessions, user_sessions)?;
        let _ = std::fs::remove_dir_all(&private_sessions);
    } else if private_sessions.exists() {
        let _ = std::fs::remove_file(&private_sessions);
    }
    create_directory_link(&private_sessions, user_sessions)
}

fn paths_point_to_same_dir(left: &Path, right: &Path) -> bool {
    let Ok(left) = std::fs::canonicalize(left) else {
        return false;
    };
    let Ok(right) = std::fs::canonicalize(right) else {
        return false;
    };
    left == right
}

fn is_reparse_point(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn unlink_directory_link(path: &Path) {
    if !is_reparse_point(path) {
        return;
    }
    let _ = std::fs::remove_dir(path);
    let _ = std::fs::remove_file(path);
}

fn merge_dir_contents(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("创建 {} 失败：{e}", to.display()))?;
    let entries = match std::fs::read_dir(from) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let src = entry.path();
        let dest = to.join(entry.file_name());
        if src.is_dir() {
            merge_dir_contents(&src, &dest)?;
        } else if !dest.exists() {
            std::fs::rename(&src, &dest)
                .or_else(|_| std::fs::copy(&src, &dest).map(|_| ()))
                .map_err(|e| format!("迁移 {} 失败：{e}", src.display()))?;
        }
    }
    Ok(())
}

fn create_directory_link(link: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use crate::proc::NoConsoleWindow;
        // mklink 是 cmd 内建；整条命令塞进一个 `/C` 参数会被 Rust 再包一层引号，
        // 路径末尾的 `\` 会把收尾引号吃掉（「文件名、目录名或卷标语法不正确」）。
        let output = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &link.to_string_lossy(),
                &target.to_string_lossy(),
            ])
            .no_console_window()
            .output()
            .map_err(|e| format!("创建会话目录联接失败：{e}"))?;
        if output.status.success() {
            return Ok(());
        }
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return Ok(());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "创建会话目录联接失败（{} → {}）：{}{}",
            link.display(),
            target.display(),
            stdout.trim(),
            stderr.trim()
        ))
    }
    #[cfg(not(windows))]
    {
        std::os::unix::fs::symlink(target, link)
            .map_err(|e| format!("创建会话目录符号链接失败：{e}"))
    }
}

/// 整份覆盖 `config.toml` 之后，把聊天里选过的顶层 `model` / `model_reasoning_effort` 写回去。
fn write_codex_config_preserving_chat_model(path: &Path, config_toml: &str) -> Result<(), String> {
    let previous = if path.is_file() {
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?
    } else {
        String::new()
    };
    let previous_model = read_toml_toplevel_string(&previous, "model");
    let previous_effort = read_toml_toplevel_string(&previous, "model_reasoning_effort");
    write_private(path, config_toml)?;
    if let Some(model) = previous_model {
        upsert_toml_toplevel_string_file(path, "model", &model)?;
    }
    if let Some(effort) = previous_effort {
        upsert_toml_toplevel_string_file(path, "model_reasoning_effort", &effort)?;
    }
    Ok(())
}

/// 把 Kivio 聊天里选的模型写进 Claude / Codex 的 **cc-switch live 文件**。
///
/// Auto / 空 / `default` 不写：沿用文件里已有的默认。其它 CLI 无操作。
/// 失败由调用方记日志，不阻断换模型。
pub fn persist_selected_model(
    agent_id: &str,
    model: Option<&str>,
    reasoning: Option<&str>,
) -> Result<(), String> {
    let Some(model) = model
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "default")
    else {
        return Ok(());
    };
    let Some(def) = get_agent_def(agent_id) else {
        return Err(format!("未知外部 Agent: {agent_id}"));
    };
    let _resource =
        super::session::live::ResourceLease::acquire(vec![format!("agent:{agent_id}")], false)?;
    match def.provider_profile {
        ProviderProfileStrategy::Claude => {
            let Some(wire) =
                crate::external_agents::session::claude_init::claude_wire_model(Some(model))
            else {
                return Ok(());
            };
            persist_claude_model(&wire)
        }
        ProviderProfileStrategy::Codex => persist_codex_model(model, reasoning),
        _ => Ok(()),
    }
}

fn persist_claude_model(model: &str) -> Result<(), String> {
    let _guard = NATIVE_CONFIG_LOCK
        .lock()
        .map_err(|_| "原生 CLI 配置写锁已损坏".to_string())?;
    if let Some(path) = claude_native_settings_path() {
        upsert_claude_live_model(&path, model)?;
    }
    if let Some(path) = claude_settings_override("claude") {
        upsert_claude_live_model(&path, model)?;
    }
    Ok(())
}

fn persist_codex_model(model: &str, reasoning: Option<&str>) -> Result<(), String> {
    let _guard = NATIVE_CONFIG_LOCK
        .lock()
        .map_err(|_| "原生 CLI 配置写锁已损坏".to_string())?;
    let reasoning = reasoning
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "default");
    // 只改已有文件：不要新建一份只有 `model =` 的残缺 ~/.codex/config.toml。
    for path in existing_unique_files(
        [codex_effective_config_path(), codex_native_config_path()]
            .into_iter()
            .flatten(),
    ) {
        upsert_toml_toplevel_string_file(&path, "model", model)?;
        if let Some(effort) = reasoning {
            upsert_toml_toplevel_string_file(&path, "model_reasoning_effort", effort)?;
        }
    }
    Ok(())
}

/// cc-switch `get_claude_settings_path`：已有 `settings.json` 用它，否则兼容旧 `claude.json`。
fn claude_native_settings_path() -> Option<PathBuf> {
    let dir = nonempty_env_path("CLAUDE_CONFIG_DIR")
        .or_else(|| directories::BaseDirs::new().map(|base| base.home_dir().join(".claude")))?;
    Some(claude_settings_path_in(dir))
}

fn claude_settings_path_in(dir: PathBuf) -> PathBuf {
    let settings = dir.join("settings.json");
    if settings.is_file() {
        return settings;
    }
    let legacy = dir.join("claude.json");
    if legacy.is_file() {
        return legacy;
    }
    settings
}

fn codex_native_config_path() -> Option<PathBuf> {
    native_codex_home().map(|home| home.join("config.toml"))
}

fn codex_effective_config_path() -> Option<PathBuf> {
    if let Some(home) = provider_env("codex").get("CODEX_HOME") {
        return Some(PathBuf::from(home).join("config.toml"));
    }
    codex_native_config_path()
}

fn read_json_string_field(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn read_json_env_string(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("env")
        .and_then(serde_json::Value::as_object)
        .and_then(|env| env.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// cc-switch 写进 Claude live `settings.json` 的模型字段：顶层 `model` + `env.ANTHROPIC_MODEL`，
/// 能认出 opus/sonnet/haiku/fable 时再写对应 `ANTHROPIC_DEFAULT_*_MODEL`。其它键原样保留。
fn upsert_claude_live_model(path: &Path, model: &str) -> Result<(), String> {
    let mut root = if path.is_file() {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        if text.trim().is_empty() {
            serde_json::Map::new()
        } else {
            parse_object_text(&text, &format!("{}", path.display()))?
        }
    } else {
        serde_json::Map::new()
    };
    let already = root.get("model").and_then(serde_json::Value::as_str) == Some(model)
        && root
            .get("env")
            .and_then(serde_json::Value::as_object)
            .and_then(|env| env.get("ANTHROPIC_MODEL"))
            .and_then(serde_json::Value::as_str)
            == Some(model);
    let family_ok = match claude_default_env_key(model) {
        None => true,
        Some(key) => {
            root.get("env")
                .and_then(serde_json::Value::as_object)
                .and_then(|env| env.get(key))
                .and_then(serde_json::Value::as_str)
                == Some(model)
        }
    };
    if already && family_ok {
        return Ok(());
    }
    let env = root
        .entry("env".to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(env) = env.as_object_mut() else {
        return Err(format!("{} 的 env 不是对象", path.display()));
    };
    env.insert(
        "ANTHROPIC_MODEL".to_string(),
        serde_json::Value::String(model.to_string()),
    );
    if let Some(key) = claude_default_env_key(model) {
        env.insert(
            key.to_string(),
            serde_json::Value::String(model.to_string()),
        );
    }
    root.insert(
        "model".to_string(),
        serde_json::Value::String(model.to_string()),
    );
    let rendered = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .map_err(|e| e.to_string())?
        + "\n";
    write_private_atomic(path, &rendered)
}

fn claude_default_env_key(model: &str) -> Option<&'static str> {
    let normalized = model.to_ascii_lowercase();
    if normalized.contains("fable") {
        Some("ANTHROPIC_DEFAULT_FABLE_MODEL")
    } else if normalized.contains("haiku") {
        Some("ANTHROPIC_DEFAULT_HAIKU_MODEL")
    } else if normalized.contains("sonnet") {
        Some("ANTHROPIC_DEFAULT_SONNET_MODEL")
    } else if normalized.contains("opus") {
        Some("ANTHROPIC_DEFAULT_OPUS_MODEL")
    } else {
        None
    }
}

fn upsert_toml_toplevel_string_file(path: &Path, key: &str, value: &str) -> Result<(), String> {
    let existing = if path.is_file() {
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?
    } else {
        String::new()
    };
    let next = upsert_toml_toplevel_string(&existing, key, value);
    if next == existing {
        return Ok(());
    }
    write_private_atomic(path, &next)
}

/// 替换或插入顶层 `key = "value"`，第一个 `[section]` 之前。其它行原样保留。
fn upsert_toml_toplevel_string(text: &str, key: &str, value: &str) -> String {
    let quoted = serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""));
    let replacement = format!("{key} = {quoted}");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut section_at = lines.len();
    let mut replaced = false;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section_at = index;
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((found_key, _)) = trimmed.split_once('=') else {
            continue;
        };
        if found_key.trim() != key {
            continue;
        }
        lines[index] = replacement.clone();
        replaced = true;
        break;
    }
    if !replaced {
        lines.insert(section_at, replacement);
    }
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn read_toml_toplevel_string(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((found_key, raw)) = trimmed.split_once('=') else {
            continue;
        };
        if found_key.trim() != key {
            continue;
        }
        let raw = raw.trim();
        if let Ok(value) = serde_json::from_str::<String>(raw) {
            return Some(value).filter(|value| !value.is_empty());
        }
        let unquoted = raw.trim_matches('"').trim_matches('\'').trim();
        if unquoted.is_empty() {
            return None;
        }
        return Some(unquoted.to_string());
    }
    None
}

fn existing_unique_files(candidates: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for path in candidates {
        if path.is_file() && !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }
    paths
}

/// Grok 原生配置路径：`$GROK_HOME/config.toml`，否则 `~/.grok/config.toml`。
fn grok_config_path() -> Option<PathBuf> {
    if let Some(home) = nonempty_env_path("GROK_HOME") {
        return Some(home.join("config.toml"));
    }
    default_user_home().map(|home| home.join(".grok/config.toml"))
}

fn grok_state_path() -> Option<PathBuf> {
    Some(profiles_dir()?.join("grok-native-state.json"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct GrokManagedState {
    /// 是否已由 Kivio 接管过 `config.toml`（有备份可还原）。
    managed: bool,
    /// 接管前整份 `config.toml` 原文；切回「CLI 自身配置」时写回。
    previous_config: Option<String>,
}

fn read_grok_state(path: &Path) -> Result<GrokManagedState, String> {
    if !path.is_file() {
        return Ok(GrokManagedState::default());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(GrokManagedState::default());
    }
    serde_json::from_str(&text).map_err(|e| format!("解析 {} 失败：{e}", path.display()))
}

fn write_grok_state(path: &Path, state: &GrokManagedState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())? + "\n";
    write_private_atomic(path, &text)
}

fn read_text_or_empty(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(format!("读取 {} 失败：{err}", path.display())),
    }
}

/// 把供应商 `config_toml` 里的 `[models]` / `[model.*]` 合并进 base。
/// 其它段（marketplace / ui / cli / mcp…）一律保留 base 的——对齐「只换路由、不动用户偏好」。
fn merge_grok_provider_config(base: &str, provider_config: &str) -> Result<String, String> {
    let mut base_doc: toml::Table = if base.trim().is_empty() {
        toml::Table::new()
    } else {
        toml::from_str(base).map_err(|e| format!("现有 Grok config.toml 解析失败：{e}"))?
    };
    let provider_doc: toml::Table = if provider_config.trim().is_empty() {
        return Err("Grok 供应商缺少 config.toml".to_string());
    } else {
        toml::from_str(provider_config)
            .map_err(|e| format!("Grok 供应商 config.toml 解析失败：{e}"))?
    };

    // 至少要有 model 表或 models.default，否则落盘等于空操作。
    let has_models = provider_doc
        .get("models")
        .and_then(|v| v.as_table())
        .is_some_and(|t| t.contains_key("default"));
    let has_model = provider_doc
        .get("model")
        .and_then(|v| v.as_table())
        .is_some_and(|t| !t.is_empty());
    if !has_models && !has_model {
        return Err("Grok 供应商 config.toml 缺少 [models].default 或 [model.*] 段".to_string());
    }

    if let Some(models) = provider_doc.get("models").and_then(|v| v.as_table()) {
        let base_models = base_doc
            .entry("models".to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| "Grok config.toml 的 [models] 不是表".to_string())?;
        for (key, value) in models {
            base_models.insert(key.clone(), value.clone());
        }
    }

    if let Some(model) = provider_doc.get("model").and_then(|v| v.as_table()) {
        let base_model = base_doc
            .entry("model".to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| "Grok config.toml 的 [model] 不是表".to_string())?;
        for (key, value) in model {
            base_model.insert(key.clone(), value.clone());
        }
    }

    // toml crate 的 pretty 序列化对 dotted keys 友好；末尾补换行方便 diff。
    let mut rendered =
        toml::to_string_pretty(&base_doc).map_err(|e| format!("序列化 Grok config 失败：{e}"))?;
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

fn materialize_grok_at(
    config: &ExternalCliAgentConfig,
    path: &Path,
    state_path: &Path,
) -> Result<(), String> {
    let mut state = read_grok_state(state_path)?;

    // 切回「CLI 自身配置」：把接管前的整份文件还原回去。
    if config.current_provider.trim().is_empty() {
        if state.managed {
            if let Some(prev) = state.previous_config.take() {
                if prev.is_empty() {
                    // 接管前文件不存在：删掉我们写过的，回到「无 config.toml」。
                    if path.is_file() {
                        std::fs::remove_file(path)
                            .map_err(|e| format!("删除 {} 失败：{e}", path.display()))?;
                    }
                } else {
                    write_private_atomic(path, &prev)?;
                }
            }
            state.managed = false;
            write_grok_state(state_path, &state)?;
        }
        return Ok(());
    }

    let provider = config
        .providers
        .iter()
        .find(|p| p.id == config.current_provider)
        .ok_or_else(|| format!("当前供应商 {} 不存在", config.current_provider))?;
    if provider.config_toml.trim().is_empty() {
        return Err(format!(
            "供应商 {} 缺少可落盘的 Grok config.toml",
            provider.name
        ));
    }

    // 首次接管：备份现有文件，之后切供应商始终基于这份备份合并，避免 A→B 残留 A 的 model 键。
    if !state.managed {
        state.previous_config = Some(read_text_or_empty(path)?);
        state.managed = true;
    }
    let base = state.previous_config.as_deref().unwrap_or("");
    let merged = merge_grok_provider_config(base, &provider.config_toml)?;
    write_private_atomic(path, &merged)?;
    write_grok_state(state_path, &state)
}

/// Kimi Code CLI 配置路径：`$KIMI_CODE_HOME/config.toml`，否则 `~/.kimi-code/config.toml`。
fn kimi_config_path() -> Option<PathBuf> {
    if let Some(home) = nonempty_env_path("KIMI_CODE_HOME") {
        return Some(home.join("config.toml"));
    }
    default_user_home().map(|home| home.join(".kimi-code/config.toml"))
}

fn kimi_state_path() -> Option<PathBuf> {
    Some(profiles_dir()?.join("kimi-native-state.json"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
struct KimiManagedState {
    managed: bool,
    previous_config: Option<String>,
}

fn read_kimi_state(path: &Path) -> Result<KimiManagedState, String> {
    if !path.is_file() {
        return Ok(KimiManagedState::default());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(KimiManagedState::default());
    }
    serde_json::from_str(&text).map_err(|e| format!("解析 {} 失败：{e}", path.display()))
}

fn write_kimi_state(path: &Path, state: &KimiManagedState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())? + "\n";
    write_private_atomic(path, &text)
}

/// 把供应商 `config_toml` 里的 `default_model` / `providers` / `models` 合并进 base。
/// 其它段（thinking / services / permission / managed OAuth…）一律保留 base 的。
fn merge_kimi_provider_config(base: &str, provider_config: &str) -> Result<String, String> {
    let mut base_doc: toml::Table = if base.trim().is_empty() {
        toml::Table::new()
    } else {
        toml::from_str(base).map_err(|e| format!("现有 Kimi config.toml 解析失败：{e}"))?
    };
    let provider_doc: toml::Table = if provider_config.trim().is_empty() {
        return Err("Kimi 供应商缺少 config.toml".to_string());
    } else {
        toml::from_str(provider_config)
            .map_err(|e| format!("Kimi 供应商 config.toml 解析失败：{e}"))?
    };

    let providers = provider_doc
        .get("providers")
        .and_then(|v| v.as_table())
        .ok_or_else(|| "Kimi 供应商 config.toml 缺少 [providers.*]".to_string())?;
    if providers.is_empty() {
        return Err("Kimi 供应商 config.toml 缺少 [providers.*]".to_string());
    }
    let models = provider_doc
        .get("models")
        .and_then(|v| v.as_table())
        .ok_or_else(|| "Kimi 供应商 config.toml 缺少 [models.*]".to_string())?;
    if models.is_empty() {
        return Err("Kimi 供应商 config.toml 缺少 [models.*]".to_string());
    }

    // 从备份合并时：先去掉本片段要接管的 provider id 对应的旧 models，避免残留。
    // 实际策略更简单——始终基于 previous_config 合并当前供应商，所以只需 overlay。
    let base_providers = base_doc
        .entry("providers".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "Kimi config.toml 的 [providers] 不是表".to_string())?;
    for (key, value) in providers {
        // 永不覆盖 managed:kimi-code（OAuth 托管账号）。
        if key == "managed:kimi-code" {
            continue;
        }
        base_providers.insert(key.clone(), value.clone());
    }

    let base_models = base_doc
        .entry("models".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| "Kimi config.toml 的 [models] 不是表".to_string())?;
    for (key, value) in models {
        base_models.insert(key.clone(), value.clone());
    }

    if let Some(default_model) = provider_doc.get("default_model") {
        base_doc.insert("default_model".to_string(), default_model.clone());
    }

    let mut rendered =
        toml::to_string_pretty(&base_doc).map_err(|e| format!("序列化 Kimi config 失败：{e}"))?;
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

fn materialize_kimi_at(
    config: &ExternalCliAgentConfig,
    path: &Path,
    state_path: &Path,
) -> Result<(), String> {
    let mut state = read_kimi_state(state_path)?;

    if config.current_provider.trim().is_empty() {
        if state.managed {
            if let Some(prev) = state.previous_config.take() {
                if prev.is_empty() {
                    if path.is_file() {
                        std::fs::remove_file(path)
                            .map_err(|e| format!("删除 {} 失败：{e}", path.display()))?;
                    }
                } else {
                    write_private_atomic(path, &prev)?;
                }
            }
            state.managed = false;
            write_kimi_state(state_path, &state)?;
        }
        return Ok(());
    }

    let provider = config
        .providers
        .iter()
        .find(|p| p.id == config.current_provider)
        .ok_or_else(|| format!("当前供应商 {} 不存在", config.current_provider))?;
    if provider.config_toml.trim().is_empty() {
        return Err(format!(
            "供应商 {} 缺少可落盘的 Kimi config.toml",
            provider.name
        ));
    }

    if !state.managed {
        state.previous_config = Some(read_text_or_empty(path)?);
        state.managed = true;
    }
    let base = state.previous_config.as_deref().unwrap_or("");
    let merged = merge_kimi_provider_config(base, &provider.config_toml)?;
    write_private_atomic(path, &merged)?;
    write_kimi_state(state_path, &state)
}

#[derive(Debug, Clone)]
struct NativeProviderEntry {
    source_id: String,
    native_id: String,
    config: Value,
    auth: Option<Value>,
    default_model: String,
    default_thinking_level: Option<String>,
}

fn materialize_native_with_strategy_at(
    strategy: ProviderProfileStrategy,
    agent_id: &str,
    config: &ExternalCliAgentConfig,
    paths: &NativePaths,
) -> Result<(), String> {
    let entries = parse_native_entries_for(strategy, &config.providers)?;
    let active = if config.current_provider.trim().is_empty() {
        None
    } else {
        let provider = config
            .providers
            .iter()
            .find(|provider| provider.id == config.current_provider)
            .ok_or_else(|| format!("当前供应商 {} 不存在", config.current_provider))?;
        if provider.disabled || provider.config_json.trim().is_empty() {
            // 停用项和升级前的 env-only 条目都不参与原生默认值接管。
            None
        } else {
            Some(
                entries
                    .iter()
                    .find(|entry| entry.source_id == config.current_provider)
                    .ok_or_else(|| {
                        format!(
                            "当前供应商 {} 没有可落盘的 {agent_id} 原生配置",
                            config.current_provider
                        )
                    })?,
            )
        }
    };

    let mut state = read_managed_state(&paths.state)?;
    let previous_ids: HashSet<String> = state.managed_provider_ids.iter().cloned().collect();
    let next_ids: HashSet<String> = entries
        .iter()
        .map(|entry| entry.native_id.clone())
        .collect();

    match strategy {
        ProviderProfileStrategy::OpenCode => {
            migrate_shadowed_opencode_configs(paths, &mut state, &previous_ids, &entries)?;
            let mut root = read_object_file(&paths.config, true, false, "OpenCode opencode.json")?;
            let before_root = root.clone();
            ensure_provider_id_available(&root, "provider", &previous_ids, &entries)?;
            sync_provider_map(strategy, &mut root, "provider", &previous_ids, &entries)?;

            let mut auth = read_object_file(&paths.auth, false, true, "OpenCode auth.json")?;
            let before_auth = auth.clone();
            ensure_auth_id_available(&auth, &previous_ids, &entries)?;
            sync_auth_map(&mut auth, &previous_ids, &entries);

            apply_default_fields(
                &mut root,
                &mut state,
                &["model"],
                active.map(|entry| {
                    vec![(
                        "model",
                        Value::String(format!("{}/{}", entry.native_id, entry.default_model)),
                    )]
                }),
            );
            state.managed_provider_ids = sorted_ids(next_ids);
            prewrite_new_backup(&paths.state, &state)?;
            write_object_if_changed(&paths.config, before_root, &root)?;
            write_object_if_changed(&paths.auth, before_auth, &auth)?;
        }
        ProviderProfileStrategy::Pi => {
            let mut models = read_object_file(&paths.config, false, false, "Pi models.json")?;
            let before_models = models.clone();
            ensure_provider_id_available(&models, "providers", &previous_ids, &entries)?;
            sync_provider_map(strategy, &mut models, "providers", &previous_ids, &entries)?;

            let mut auth = read_object_file(&paths.auth, false, true, "Pi auth.json")?;
            let before_auth = auth.clone();
            sync_auth_map(&mut auth, &previous_ids, &entries);

            let settings_path = paths
                .settings
                .as_ref()
                .ok_or_else(|| "Pi settings.json 路径缺失".to_string())?;
            let _settings_lock =
                crate::external_agents::pi_extensions::lock_settings_file(settings_path)?;
            let mut settings = read_object_file(settings_path, false, false, "Pi settings.json")?;
            let before_settings = settings.clone();
            apply_default_fields(
                &mut settings,
                &mut state,
                &["defaultProvider", "defaultModel", "defaultThinkingLevel"],
                active.map(|entry| {
                    let mut values = vec![
                        ("defaultProvider", Value::String(entry.native_id.clone())),
                        ("defaultModel", Value::String(entry.default_model.clone())),
                    ];
                    if let Some(level) = &entry.default_thinking_level {
                        values.push(("defaultThinkingLevel", Value::String(level.clone())));
                    }
                    values
                }),
            );
            state.managed_provider_ids = sorted_ids(next_ids);
            prewrite_new_backup(&paths.state, &state)?;
            write_object_if_changed(&paths.config, before_models, &models)?;
            write_object_if_changed(&paths.auth, before_auth, &auth)?;
            write_object_if_changed(settings_path, before_settings, &settings)?;
        }
        _ => return Ok(()),
    }

    write_managed_state(&paths.state, &state)
}

fn migrate_shadowed_opencode_configs(
    paths: &NativePaths,
    state: &mut NativeManagedState,
    previous_ids: &HashSet<String>,
    entries: &[NativeProviderEntry],
) -> Result<(), String> {
    for path in &paths.alternate_configs {
        let root = read_object_file(path, true, false, "OpenCode shadowed config")?;
        ensure_provider_id_available(&root, "provider", previous_ids, entries)?;
    }
    for path in &paths.alternate_configs {
        let mut root = read_object_file(path, true, false, "OpenCode shadowed config")?;
        let before = root.clone();
        if state.defaults_managed
            && root
                .get("model")
                .and_then(Value::as_str)
                .and_then(|model| model.split_once('/'))
                .is_some_and(|(provider, _)| previous_ids.contains(provider))
        {
            apply_default_fields(&mut root, state, &["model"], None);
        }
        remove_managed_provider_ids(&mut root, "provider", previous_ids)?;
        write_object_if_changed(path, before, &root)?;
    }
    Ok(())
}

fn parse_native_entries_for(
    strategy: ProviderProfileStrategy,
    providers: &[ExternalCliProvider],
) -> Result<Vec<NativeProviderEntry>, String> {
    let mut entries = Vec::new();
    let mut native_ids = HashSet::new();
    for provider in providers.iter().filter(|provider| !provider.disabled) {
        // 兼容升级前创建的 env-only 条目；编辑并保存后才变成原生配置。
        if provider.config_json.trim().is_empty() {
            continue;
        }
        let native_id = resolve_native_provider_id(provider)?;
        if !native_ids.insert(native_id.clone()) {
            return Err(format!("供应商 id 归一化后冲突：{native_id}"));
        }
        let mut config = parse_object_text(&provider.config_json, "provider configJson")?;
        // 旧 configJson 可能还没写 forceAdaptiveThinking；同步到 models.json 时补上，
        // 否则 pi-cache-optimizer 会对 opus≥4.6 / sonnet≥4.6 / fable≥5 弹告警。
        if strategy == ProviderProfileStrategy::Pi {
            ensure_pi_adaptive_thinking_compat(&mut config);
        }
        let auth = if provider.auth_json.trim().is_empty() {
            Map::new()
        } else {
            parse_object_text(&provider.auth_json, "provider authJson")?
        };
        let default_model = provider.default_model.trim().to_string();
        if default_model.is_empty() {
            return Err(format!("供应商 {} 缺少默认模型", provider.name));
        }
        validate_native_provider(strategy, &config, &default_model, &provider.name)?;
        validate_native_auth(strategy, &auth, &provider.name)?;
        let default_thinking_level = if strategy == ProviderProfileStrategy::Pi {
            let level = provider.default_reasoning.trim();
            if level.is_empty() {
                None
            } else if matches!(
                level,
                "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
            ) {
                Some(level.to_string())
            } else {
                return Err(format!("供应商 {} 的 Pi 默认推理等级无效", provider.name));
            }
        } else {
            None
        };
        if let Some(level) = default_thinking_level.as_deref() {
            validate_pi_thinking_level(&config, &default_model, level, &provider.name)?;
        }
        entries.push(NativeProviderEntry {
            source_id: provider.id.clone(),
            native_id,
            config: Value::Object(config),
            auth: (!auth.is_empty()).then(|| Value::Object(auth)),
            default_model,
            default_thinking_level,
        });
    }
    Ok(entries)
}

fn validate_pi_thinking_level(
    config: &Map<String, Value>,
    default_model: &str,
    level: &str,
    provider_name: &str,
) -> Result<(), String> {
    if level == "off" {
        return Ok(());
    }
    let model = config
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|model| model.get("id").and_then(Value::as_str) == Some(default_model))
        })
        .ok_or_else(|| format!("供应商 {provider_name} 的默认模型不在 models 列表中"))?;
    if model.get("reasoning").and_then(Value::as_bool) != Some(true) {
        return Err(format!(
            "供应商 {provider_name} 的默认模型未声明支持推理，仅可使用 off"
        ));
    }

    let mapping = model.get("thinkingLevelMap").and_then(Value::as_object);
    let supported = match mapping.and_then(|mapping| mapping.get(level)) {
        Some(value) => !value.is_null(),
        None => matches!(level, "minimal" | "low" | "medium" | "high"),
    };
    if supported {
        Ok(())
    } else {
        Err(format!(
            "供应商 {provider_name} 的默认模型不支持 Pi 推理等级 {level}"
        ))
    }
}

/// Anthropic adaptive-generation Claude 判定（对齐 pi-cache-optimizer / 前端
/// `isPiAdaptiveThinkingModel`）：opus ≥4.6 / sonnet ≥4.6 / fable ≥5。
fn is_pi_adaptive_thinking_model(model_id: &str) -> bool {
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)(^|[/\s:_-])(opus-4[.-][6-9]|opus-4-[1-9][0-9]|opus-([5-9]|[1-9][0-9])|sonnet-4[.-][6-9]|sonnet-4-[1-9][0-9]|sonnet-([5-9]|[1-9][0-9])|fable-([5-9]|[1-9][0-9]))($|[-_.:/\s\[])",
        )
        .expect("adaptive thinking model pattern")
    });
    RE.is_match(model_id)
}

/// 给 pi 的 anthropic-messages 渠道里 adaptive 模型补 `compat.forceAdaptiveThinking`。
/// 已有 true 不覆盖；非 anthropic-messages / 非 adaptive 模型不动。
fn ensure_pi_adaptive_thinking_compat(config: &mut Map<String, Value>) {
    if config.get("api").and_then(Value::as_str) != Some("anthropic-messages") {
        return;
    }
    let Some(models) = config.get_mut("models").and_then(Value::as_array_mut) else {
        return;
    };
    for model in models {
        let Some(obj) = model.as_object_mut() else {
            continue;
        };
        let Some(id) = obj.get("id").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if !is_pi_adaptive_thinking_model(&id) {
            continue;
        }
        let compat = obj
            .entry("compat".to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(compat_obj) = compat.as_object_mut() {
            if compat_obj
                .get("forceAdaptiveThinking")
                .and_then(Value::as_bool)
                != Some(true)
            {
                compat_obj.insert("forceAdaptiveThinking".to_string(), Value::Bool(true));
            }
        }
    }
}

fn validate_native_provider(
    strategy: ProviderProfileStrategy,
    config: &Map<String, Value>,
    default_model: &str,
    name: &str,
) -> Result<(), String> {
    let nonempty = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    };
    match strategy {
        ProviderProfileStrategy::OpenCode => {
            if !nonempty(config.get("npm")) {
                return Err(format!("供应商 {name} 的 OpenCode 配置缺少 npm"));
            }
            if config.get("npm").and_then(Value::as_str) == Some("@ai-sdk/openai-compatible")
                && !config
                    .get("options")
                    .and_then(Value::as_object)
                    .is_some_and(|options| nonempty(options.get("baseURL")))
            {
                return Err(format!(
                    "供应商 {name} 使用 OpenAI Compatible 时必须填写 options.baseURL"
                ));
            }
        }
        ProviderProfileStrategy::Pi => {
            const APIS: &[&str] = &[
                "openai-completions",
                "openai-responses",
                "anthropic-messages",
                "google-generative-ai",
            ];
            let api = config.get("api").and_then(Value::as_str).unwrap_or("");
            if !nonempty(config.get("baseUrl")) || !APIS.contains(&api) {
                return Err(format!("供应商 {name} 的 Pi baseUrl 或 api 无效"));
            }
        }
        _ => {}
    }
    let model_exists =
        match strategy {
            ProviderProfileStrategy::OpenCode => config
                .get("models")
                .and_then(Value::as_object)
                .is_some_and(|models| models.contains_key(default_model)),
            ProviderProfileStrategy::Pi => config
                .get("models")
                .and_then(Value::as_array)
                .is_some_and(|models| {
                    models
                        .iter()
                        .any(|model| model.get("id").and_then(Value::as_str) == Some(default_model))
                }),
            _ => true,
        };
    if !model_exists {
        return Err(format!("供应商 {name} 的默认模型不在 models 列表中"));
    }
    Ok(())
}

fn validate_native_auth(
    strategy: ProviderProfileStrategy,
    auth: &Map<String, Value>,
    name: &str,
) -> Result<(), String> {
    if strategy == ProviderProfileStrategy::OpenCode && auth.is_empty() {
        return Ok(());
    }
    let expected_type = if strategy == ProviderProfileStrategy::OpenCode {
        "api"
    } else {
        "api_key"
    };
    let valid = auth.get("type").and_then(Value::as_str) == Some(expected_type)
        && auth
            .get("key")
            .and_then(Value::as_str)
            .is_some_and(|key| !key.trim().is_empty());
    if valid {
        Ok(())
    } else {
        Err(format!("供应商 {name} 的原生凭据格式无效"))
    }
}

/// 解析写入 CLI 原生配置的 provider id：显式 nativeProviderId → 显示名 slug → 内部 id slug。
fn resolve_native_provider_id(provider: &ExternalCliProvider) -> Result<String, String> {
    let configured = provider.native_provider_id.trim();
    if !configured.is_empty() {
        if !valid_explicit_native_provider_id(configured) {
            return Err(format!(
                "供应商 {} 的原生 provider id 无效：{}",
                provider.name, configured
            ));
        }
        return Ok(configured.to_string());
    }
    if let Some(slug) = slugify_provider_key(&provider.name) {
        return Ok(slug);
    }
    if let Some(slug) = slugify_provider_key(&provider.id) {
        return Ok(slug);
    }
    Err(format!(
        "供应商 id 无法生成原生 provider id：{}",
        provider.id
    ))
}

/// 把名字/内部 id 压成合法 provider key（小写 ascii + `.` `_` `-`），规则对齐前端 `nativeProviderIdFromName`。
fn slugify_provider_key(raw: &str) -> Option<String> {
    let mut slug = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '.' | '_' | '-') {
            slug.push(ch);
        } else if slug.chars().last() != Some('-') {
            slug.push('-');
        }
    }
    let mut collapsed = String::with_capacity(slug.len());
    for ch in slug.chars() {
        if ch == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(ch);
    }
    let trimmed = collapsed.trim_matches(|c| matches!(c, '.' | '_' | '-'));
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn native_cleanup_ids(
    provider_id: &str,
    provider_name: Option<&str>,
    configured_native_id: Option<&str>,
) -> Vec<String> {
    let mut ids = Vec::new();
    let mut push = |id: String| {
        if !id.is_empty() && !ids.iter().any(|existing| existing == &id) {
            ids.push(id);
        }
    };
    if let Some(id) = configured_native_id
        .map(str::trim)
        .filter(|id| valid_explicit_native_provider_id(id))
    {
        push(id.to_string());
    }
    if let Some(name) = provider_name.map(str::trim).filter(|name| !name.is_empty()) {
        if let Some(slug) = slugify_provider_key(name) {
            push(format!("kivio-{slug}"));
            push(slug);
        }
    }
    if let Some(slug) = slugify_provider_key(provider_id) {
        push(format!("kivio-{slug}"));
        push(slug);
    }
    ids
}

fn valid_explicit_native_provider_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn ensure_provider_id_available(
    root: &Map<String, Value>,
    field: &str,
    previous_ids: &HashSet<String>,
    entries: &[NativeProviderEntry],
) -> Result<(), String> {
    let Some(existing) = root.get(field).and_then(Value::as_object) else {
        return Ok(());
    };
    if let Some(entry) = entries.iter().find(|entry| {
        existing.contains_key(&entry.native_id) && !previous_ids.contains(&entry.native_id)
    }) {
        return Err(format!(
            "原生配置中已存在非 Kivio 管理的供应商 id：{}",
            entry.native_id
        ));
    }
    Ok(())
}

fn ensure_auth_id_available(
    auth: &Map<String, Value>,
    previous_ids: &HashSet<String>,
    entries: &[NativeProviderEntry],
) -> Result<(), String> {
    if let Some(entry) = entries.iter().find(|entry| {
        auth.contains_key(&entry.native_id) && !previous_ids.contains(&entry.native_id)
    }) {
        return Err(format!(
            "原生 auth.json 中已存在非 Kivio 管理的供应商 id：{}",
            entry.native_id
        ));
    }
    Ok(())
}

fn merge_object(base: &mut Map<String, Value>, incoming: &Map<String, Value>) {
    for (key, value) in incoming {
        base.insert(key.clone(), value.clone());
    }
}

fn merge_opencode_model(
    mut existing: Map<String, Value>,
    incoming: &Map<String, Value>,
) -> Map<String, Value> {
    for (key, value) in incoming {
        if let (Some(base), Some(incoming)) = (
            existing.get(key).and_then(Value::as_object).cloned(),
            value.as_object(),
        ) {
            let mut merged = base;
            merge_object(&mut merged, incoming);
            existing.insert(key.clone(), Value::Object(merged));
        } else {
            existing.insert(key.clone(), value.clone());
        }
    }
    existing
}

fn merge_opencode_provider(existing: Value, incoming: &Value) -> Value {
    let (Some(mut existing), Some(incoming)) =
        (existing.as_object().cloned(), incoming.as_object())
    else {
        return incoming.clone();
    };

    for (key, value) in incoming {
        match key.as_str() {
            "options" => {
                let mut merged = existing
                    .get("options")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                if let Some(incoming) = value.as_object() {
                    merge_object(&mut merged, incoming);
                    if !incoming.contains_key("baseURL") {
                        merged.remove("baseURL");
                    }
                    existing.insert(key.clone(), Value::Object(merged));
                } else {
                    existing.insert(key.clone(), value.clone());
                }
            }
            "models" => {
                let old_models = existing.get("models").and_then(Value::as_object);
                let mut models = Map::new();
                if let Some(incoming_models) = value.as_object() {
                    for (model_id, model) in incoming_models {
                        let mut merged = old_models
                            .and_then(|models| models.get(model_id))
                            .and_then(Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        if let Some(incoming_model) = model.as_object() {
                            merged = merge_opencode_model(merged, incoming_model);
                            models.insert(model_id.clone(), Value::Object(merged));
                        } else {
                            models.insert(model_id.clone(), model.clone());
                        }
                    }
                    existing.insert(key.clone(), Value::Object(models));
                } else {
                    existing.insert(key.clone(), value.clone());
                }
            }
            _ => {
                existing.insert(key.clone(), value.clone());
            }
        }
    }
    Value::Object(existing)
}

fn remove_managed_provider_ids(
    root: &mut Map<String, Value>,
    field: &str,
    managed_ids: &HashSet<String>,
) -> Result<(), String> {
    let Some(value) = root.get_mut(field) else {
        return Ok(());
    };
    let providers = value
        .as_object_mut()
        .ok_or_else(|| format!("原生配置的 {field} 必须是对象"))?;
    for id in managed_ids {
        providers.remove(id);
    }
    Ok(())
}

fn sync_provider_map(
    strategy: ProviderProfileStrategy,
    root: &mut Map<String, Value>,
    field: &str,
    previous_ids: &HashSet<String>,
    entries: &[NativeProviderEntry],
) -> Result<(), String> {
    if previous_ids.is_empty() && entries.is_empty() {
        return Ok(());
    }
    let providers = root
        .entry(field.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| format!("原生配置的 {field} 必须是对象"))?;
    let mut previous = HashMap::new();
    for id in previous_ids {
        if let Some(value) = providers.remove(id) {
            previous.insert(id.clone(), value);
        }
    }
    for entry in entries {
        let config = if strategy == ProviderProfileStrategy::OpenCode {
            previous
                .remove(&entry.native_id)
                .map(|existing| merge_opencode_provider(existing, &entry.config))
                .unwrap_or_else(|| entry.config.clone())
        } else {
            entry.config.clone()
        };
        providers.insert(entry.native_id.clone(), config);
    }
    Ok(())
}

fn sync_auth_map(
    auth: &mut Map<String, Value>,
    previous_ids: &HashSet<String>,
    entries: &[NativeProviderEntry],
) {
    for id in previous_ids {
        auth.remove(id);
    }
    for entry in entries {
        if let Some(value) = &entry.auth {
            auth.insert(entry.native_id.clone(), value.clone());
        }
    }
}

fn apply_default_fields(
    root: &mut Map<String, Value>,
    state: &mut NativeManagedState,
    keys: &[&str],
    active_values: Option<Vec<(&str, Value)>>,
) {
    match active_values {
        Some(values) => {
            for key in keys {
                state
                    .previous_defaults
                    .entry((*key).to_string())
                    .or_insert_with(|| {
                        let value = root.get(*key).cloned();
                        BackedUpField {
                            present: value.is_some(),
                            value: value.unwrap_or(Value::Null),
                        }
                    });
            }
            state.defaults_managed = true;
            for (key, value) in values {
                root.insert(key.to_string(), value);
            }
        }
        None if state.defaults_managed => {
            for key in keys {
                match state.previous_defaults.get(*key) {
                    Some(backup) if backup.present => {
                        root.insert((*key).to_string(), backup.value.clone());
                    }
                    _ => {
                        root.remove(*key);
                    }
                }
            }
            state.defaults_managed = false;
            state.previous_defaults.clear();
        }
        None => {}
    }
}

fn sorted_ids(ids: HashSet<String>) -> Vec<String> {
    let mut ids: Vec<String> = ids.into_iter().collect();
    ids.sort();
    ids
}

/// 首次接管默认模型时先把备份状态落盘；即使进程在随后写 CLI 配置时退出，也不会丢恢复点。
fn prewrite_new_backup(path: &Path, state: &NativeManagedState) -> Result<(), String> {
    if state.defaults_managed {
        write_managed_state(path, state)?;
    }
    Ok(())
}

fn read_managed_state(path: &Path) -> Result<NativeManagedState, String> {
    if !path.is_file() {
        return Ok(NativeManagedState::default());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("解析 {} 失败：{e}", path.display()))
}

fn write_managed_state(path: &Path, state: &NativeManagedState) -> Result<(), String> {
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())? + "\n";
    if std::fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    write_private_atomic(path, &text)
}

fn parse_object_text(text: &str, label: &str) -> Result<Map<String, Value>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("{label} 解析失败：{e}"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} 顶层必须是对象"))
}

fn read_object_file(
    path: &Path,
    jsonc: bool,
    empty_ok: bool,
    label: &str,
) -> Result<Map<String, Value>, String> {
    if !path.is_file() {
        return Ok(Map::new());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
    if text.trim().is_empty() && empty_ok {
        return Ok(Map::new());
    }
    let value: Value = if jsonc {
        json5::from_str(&text).map_err(|e| format!("{label} 解析失败：{e}"))?
    } else {
        serde_json::from_str(&text).map_err(|e| format!("{label} 解析失败：{e}"))?
    };
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("{label} 顶层必须是对象"))
}

fn write_object_if_changed(
    path: &Path,
    before: Map<String, Value>,
    after: &Map<String, Value>,
) -> Result<(), String> {
    if &before == after {
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&Value::Object(after.clone()))
        .map_err(|e| e.to_string())?
        + "\n";
    write_private_atomic(path, &text)
}

/// 文件里有 API key，权限收到 0600（Windows 无 unix 权限位，靠 app data 目录本身的 ACL）。
fn write_private(path: &Path, content: &str) -> Result<(), String> {
    write_private_atomic(path, content)
}

/// 原生配置会被独立 CLI 同时读取：同目录临时文件 fsync 后 rename，避免读到半截 JSON。
pub(crate) fn write_private_atomic(path: &Path, content: &str) -> Result<(), String> {
    write_private_bytes(path, content.as_bytes())
}
fn write_private_bytes(path: &Path, content: &[u8]) -> Result<(), String> {
    validate_private_target(path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("无效文件名：{}", path.display()))?;
    let temp = path.with_file_name(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), std::io::Error> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    })();
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("原子写入 {} 失败：{err}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
fn native_strategy_for_test(agent_id: &str) -> ProviderProfileStrategy {
    get_agent_def(agent_id)
        .map(|def| def.provider_profile)
        .expect("registered native-provider agent")
}

#[cfg(test)]
fn materialize_native_at(
    agent_id: &str,
    config: &ExternalCliAgentConfig,
    paths: &NativePaths,
) -> Result<(), String> {
    materialize_native_with_strategy_at(native_strategy_for_test(agent_id), agent_id, config, paths)
}

#[cfg(test)]
fn parse_native_entries(
    agent_id: &str,
    providers: &[ExternalCliProvider],
) -> Result<Vec<NativeProviderEntry>, String> {
    parse_native_entries_for(native_strategy_for_test(agent_id), providers)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ProfileRoot;
    impl Drop for ProfileRoot {
        fn drop(&mut self) {
            TEST_PROFILE_ROOT.with(|root| *root.borrow_mut() = None);
        }
    }
    fn isolated_profiles(root: &Path) -> ProfileRoot {
        TEST_PROFILE_ROOT.with(|value| *value.borrow_mut() = Some(root.to_owned()));
        ProfileRoot
    }

    fn assert_user_home_file_changes_launch_identity(agent: &str, relative: &str) {
        use super::super::overrides::LaunchIdentity;

        let keys = if cfg!(windows) {
            vec!["USERPROFILE", "userprofile"]
        } else {
            vec!["HOME"]
        };
        for key in keys {
            let temp = tempfile::tempdir().unwrap();
            let _root = isolated_profiles(temp.path());
            let home = temp.path().join("child-home");
            let path = home.join(relative);
            let def = get_agent_def(agent).unwrap();
            let config = ExternalCliAgentConfig {
                env: vec![crate::settings::CliEnvVar {
                    key: key.into(),
                    value: home.to_string_lossy().into_owned(),
                }],
                ..Default::default()
            };
            let effective = if agent == "opencode" {
                let paths = effective_native_paths(def, &config).unwrap();
                if relative.ends_with("auth.json") {
                    paths.auth
                } else {
                    paths.config
                }
            } else {
                let fallback = match agent {
                    "grok" => grok_config_path().unwrap(),
                    "kimi" => kimi_config_path().unwrap(),
                    _ => unreachable!(),
                };
                assert!(fallback.starts_with(temp.path()));
                effective_toml_paths(def, &config).unwrap().0
            };
            assert_eq!(effective, path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"first account").unwrap();
            let bin = temp.path().join(agent);
            std::fs::write(&bin, b"fixture executable").unwrap();
            let environment = [(OsString::from(key), home.into_os_string())].into();
            let capture = || {
                LaunchIdentity::from_environment(def, &bin, Some(0), Some(&config), &environment)
            };
            let first = capture();
            assert_eq!(first, capture());
            std::fs::write(&path, b"second account").unwrap();
            assert_ne!(
                first,
                capture(),
                "{agent} must observe {relative} under {key}"
            );
        }
    }

    #[test]
    fn launch_identity_tracks_opencode_config_under_user_home_override() {
        assert_user_home_file_changes_launch_identity("opencode", ".config/opencode/opencode.json");
    }

    #[test]
    fn launch_identity_tracks_opencode_auth_under_user_home_override() {
        assert_user_home_file_changes_launch_identity("opencode", ".local/share/opencode/auth.json");
    }

    #[test]
    fn launch_identity_tracks_grok_config_under_user_home_override() {
        assert_user_home_file_changes_launch_identity("grok", ".grok/config.toml");
    }

    #[test]
    fn launch_identity_tracks_kimi_config_under_user_home_override() {
        assert_user_home_file_changes_launch_identity("kimi", ".kimi-code/config.toml");
    }

    #[cfg(windows)]
    #[test]
    fn maintenance_review_lowercase_home_tracks_changed_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(temp.path());
        let home = temp.path().join("custom");
        std::fs::create_dir(&home).unwrap();
        let auth = home.join("auth.json");
        std::fs::write(&auth, "first account").unwrap();
        let env = [(OsString::from("codex_home"), home.into_os_string())].into();
        let def = super::super::registry::get_agent_def("codex").unwrap();
        let hasher = std::collections::hash_map::RandomState::new();
        let first = launch_file_revision(def, Path::new("codex.exe"), None, &env, &hasher);
        std::fs::write(&auth, "second account").unwrap();
        assert_ne!(first, launch_file_revision(def, Path::new("codex.exe"), None, &env, &hasher));
    }
    fn codex_settings(
        providers: Vec<ExternalCliProvider>,
        current: &str,
    ) -> crate::settings::Settings {
        let mut settings = crate::settings::Settings::default();
        settings.chat.external_cli_agents.insert(
            "codex".into(),
            ExternalCliAgentConfig {
                providers,
                current_provider: current.into(),
                ..Default::default()
            },
        );
        settings
    }
    fn codex_provider(id: &str, key: &str) -> ExternalCliProvider {
        ExternalCliProvider {
            id: id.into(),
            name: id.into(),
            config_toml: "model = 'fixture'\n".into(),
            auth_json: format!(r#"{{"OPENAI_API_KEY":"{key}"}}"#),
            ..Default::default()
        }
    }
    #[test]
    fn private_codex_rejects_conflicting_home_but_native_mode_preserves_it() {
        let root = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        let native = root.path().join("native");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("auth.json"), b"native credentials").unwrap();
        let before = codex_settings(vec![], "");
        let mut after = codex_settings(vec![codex_provider("private", "new")], "private");
        after
            .chat
            .external_cli_agents
            .get_mut("codex")
            .unwrap()
            .env
            .push(crate::settings::CliEnvVar {
                key: "CODEX_HOME".into(),
                value: native.to_string_lossy().into_owned(),
            });
        assert!(
            persist_with_profiles(&before, &after, || panic!("conflicting home"))
                .unwrap_err()
                .contains("CODEX_HOME")
        );
        assert!(!root.path().join("codex-private").exists());
        after
            .chat
            .external_cli_agents
            .get_mut("codex")
            .unwrap()
            .current_provider
            .clear();
        persist_with_profiles(&before, &after, || Ok(())).unwrap();
        assert_eq!(
            std::fs::read(native.join("auth.json")).unwrap(),
            b"native credentials"
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_home_accepts_windows_case_alias_and_rejects_ambiguous_aliases() {
        let root = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        let home = root.path().join("grok");
        let mut config = ExternalCliAgentConfig {
            env: vec![crate::settings::CliEnvVar {
                key: "grok_home".into(),
                value: home.to_string_lossy().into_owned(),
            }],
            ..Default::default()
        };
        let def = get_agent_def("grok").unwrap();
        assert_eq!(
            effective_toml_paths(def, &config).unwrap().0,
            home.join("config.toml")
        );
        let mut before = crate::settings::Settings::default();
        before
            .chat
            .external_cli_agents
            .insert("grok".into(), config.clone());
        config.env.push(crate::settings::CliEnvVar {
            key: "GROK_HOME".into(),
            value: root.path().join("different").to_string_lossy().into_owned(),
        });
        let mut after = before.clone();
        after.chat.external_cli_agents.insert("grok".into(), config);
        assert!(
            persist_with_profiles(&before, &after, || panic!("ambiguous home"))
                .unwrap_err()
                .contains("多种大小写")
        );
        assert!(!home.exists());
    }

    #[tokio::test]
    async fn prepared_pi_settings_reuses_one_maintenance_owner_through_file_lock() {
        let root = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        let home = root.path().join("pi");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("settings.json"), "{\"theme\":\"fixture\"}").unwrap();
        let mut before = crate::settings::Settings::default();
        before.chat.external_cli_agents.insert(
            "pi".into(),
            ExternalCliAgentConfig {
                env: vec![crate::settings::CliEnvVar {
                    key: "PI_CODING_AGENT_DIR".into(),
                    value: home.to_string_lossy().into_owned(),
                }],
                ..Default::default()
            },
        );
        let mut after = before.clone();
        after
            .chat
            .external_cli_agents
            .get_mut("pi")
            .unwrap()
            .disabled = true;
        let keys = config_resource_keys(
            get_agent_def("pi").unwrap(),
            &after.chat.external_cli_agents["pi"],
        );
        let registry = super::super::session::live::LiveSessionRegistry::default();
        let lease = registry
            .prepare_settings_change(&["fixture".into()], keys.clone())
            .await
            .unwrap()
            .unwrap();
        super::super::session::live::MAINTENANCE_OWNER.sync_scope(lease.clone(), || {
            let inherited =
                super::super::session::live::ResourceLease::acquire(keys, true).unwrap();
            assert_eq!(inherited.id(), lease.id());
            super::super::session::live::MAINTENANCE_OWNER.sync_scope(inherited, || {
                persist_with_profiles(&before, &after, || {
                    std::fs::write(root.path().join("settings-document"), b"committed")
                        .map_err(|e| e.to_string())
                })
                .unwrap();
            });
        });
        assert_eq!(
            std::fs::read(root.path().join("settings-document")).unwrap(),
            b"committed"
        );
        assert!(std::fs::read_to_string(home.join("settings.json"))
            .unwrap()
            .contains("fixture"));
    }

    #[test]
    fn relative_home_and_unbound_legacy_backup_fail_before_writes() {
        let root = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        let mut before = crate::settings::Settings::default();
        before.chat.external_cli_agents.insert(
            "grok".into(),
            ExternalCliAgentConfig {
                env: vec![crate::settings::CliEnvVar {
                    key: "GROK_HOME".into(),
                    value: root.path().join("native").to_string_lossy().into_owned(),
                }],
                ..Default::default()
            },
        );
        let mut after = before.clone();
        after.chat.external_cli_agents.get_mut("grok").unwrap().env[0].value =
            "relative-home".into();
        assert!(
            persist_with_profiles(&before, &after, || panic!("invalid home cannot persist"))
                .unwrap_err()
                .contains("绝对路径")
        );
        after = before.clone();
        after
            .chat
            .external_cli_agents
            .get_mut("grok")
            .unwrap()
            .disabled = true;
        let backup = root.path().join("grok-native-state.json");
        std::fs::write(&backup, b"legacy sentinel").unwrap();
        assert!(
            persist_with_profiles(&before, &after, || panic!("legacy target must be known"))
                .unwrap_err()
                .contains("原始目录")
        );
        assert_eq!(std::fs::read(backup).unwrap(), b"legacy sentinel");
        assert!(!root.path().join("native").exists());
    }
    #[test]
    fn profile_undo_excludes_native_and_pi_side_writers_until_rollback_finishes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        std::fs::write(&path, b"original native bytes").unwrap();
        let keys = vec![format!(
            "profile:{}",
            super::super::session::live::resource_path(&path)
        )];
        let lease = super::super::session::live::ResourceLease::acquire(keys, true).unwrap();
        let guard = NATIVE_CONFIG_LOCK.lock().unwrap();
        let undo = ProfileUndo::capture(vec![path.clone()], vec![], guard).unwrap();
        std::fs::write(&path, b"changed by account save").unwrap();
        let probe_path = path.clone();
        std::thread::spawn(move || {
            assert!(native_file_update(&probe_path).is_err());
            assert!(
                crate::external_agents::pi_extensions::lock_settings_file(&probe_path).is_err()
            );
        })
        .join()
        .unwrap();
        undo.rollback().unwrap();
        drop(lease);
        assert_eq!(std::fs::read(&path).unwrap(), b"original native bytes");
        assert!(crate::external_agents::pi_extensions::lock_settings_file(&path).is_ok());
    }
    #[test]
    fn backup_identity_survives_first_materialization() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new").join("nested").join("config.toml");
        let state = root.path().join("grok-native-state.json");
        let before = bound_state(state.clone(), &path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"model = 'fixture'").unwrap();
        assert_eq!(before, bound_state(state, &path));
    }

    #[test]
    fn failed_settings_save_restores_active_inactive_bytes_absence_and_shared_sessions() {
        let root = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        let active = root.path().join("codex-active");
        let inactive = root.path().join("codex-inactive");
        std::fs::create_dir_all(&active).unwrap();
        std::fs::create_dir_all(&inactive).unwrap();
        let auth = b"{ \"OPENAI_API_KEY\": \"original bytes\" }\n";
        std::fs::write(active.join("auth.json"), auth).unwrap();
        std::fs::write(inactive.join("auth.json"), b"inactive credential").unwrap();
        let shared = root.path().join("shared-codex").join("sessions");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("sentinel"), b"history").unwrap();
        create_directory_link(&inactive.join("sessions"), &shared).unwrap();
        create_directory_link(&active.join("sessions"), &shared).unwrap();
        let before = codex_settings(
            vec![
                codex_provider("active", "old"),
                codex_provider("inactive", "old"),
            ],
            "active",
        );
        let after = codex_settings(vec![codex_provider("active", "new")], "active");
        let error = persist_with_profiles(&before, &after, || {
            std::fs::write(root.path(), "settings").map_err(|e| e.to_string())
        })
        .unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(std::fs::read(active.join("auth.json")).unwrap(), auth);
        assert_eq!(
            std::fs::read(inactive.join("auth.json")).unwrap(),
            b"inactive credential"
        );
        assert!(!active.join("config.toml").exists());
        assert_eq!(std::fs::read(shared.join("sentinel")).unwrap(), b"history");
        persist_with_profiles(&before, &after, || Ok(())).unwrap();
        assert!(!inactive.join("auth.json").exists());
        assert_eq!(std::fs::read(shared.join("sentinel")).unwrap(), b"history");
        assert!(std::fs::read_to_string(active.join("auth.json"))
            .unwrap()
            .contains("new"));
    }
    #[test]
    fn invalid_auth_is_rejected_before_any_profile_write() {
        let root = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        let before = codex_settings(vec![], "");
        let mut provider = codex_provider("bad", "new");
        provider.auth_json = "not json".into();
        let after = codex_settings(vec![provider], "bad");
        assert!(persist_with_profiles(&before, &after, || panic!("must not persist")).is_err());
        assert!(!root.path().join("codex-bad").exists());
    }
    #[test]
    fn private_parent_junction_cannot_change_outside_credentials() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        std::fs::write(outside.path().join("auth.json"), b"outside").unwrap();
        create_directory_link(&root.path().join("codex-linked"), outside.path()).unwrap();
        let before = codex_settings(vec![], "");
        let after = codex_settings(vec![codex_provider("linked", "new")], "linked");
        assert!(persist_with_profiles(&before, &after, || Ok(())).is_err());
        assert_eq!(
            std::fs::read(outside.path().join("auth.json")).unwrap(),
            b"outside"
        );
    }
    #[test]
    fn moving_provider_home_restores_old_native_bytes_and_binds_new_backup() {
        let root = tempfile::tempdir().unwrap();
        let _root = isolated_profiles(root.path());
        let old = root.path().join("old");
        let new = root.path().join("new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("config.toml"), "[ui]\ncolor = 'old'\n").unwrap();
        std::fs::write(new.join("config.toml"), "[ui]\ncolor = 'new'\n").unwrap();
        let mut provider = ExternalCliProvider { id: "relay".into(), name: "Relay".into(), config_toml: "[models]\ndefault = 'test'\n[model.test]\nmodel = 'test'\nbase_url = 'https://example.test'\n".into(), ..Default::default() };
        provider.env.push(crate::settings::CliEnvVar {
            key: "GROK_HOME".into(),
            value: old.to_string_lossy().into_owned(),
        });
        let mut before = crate::settings::Settings::default();
        before.chat.external_cli_agents.insert(
            "grok".into(),
            ExternalCliAgentConfig {
                providers: vec![provider],
                current_provider: "relay".into(),
                ..Default::default()
            },
        );
        let mut empty = before.clone();
        empty
            .chat
            .external_cli_agents
            .get_mut("grok")
            .unwrap()
            .current_provider
            .clear();
        persist_with_profiles(&empty, &before, || Ok(())).unwrap();
        let mut after = before.clone();
        after
            .chat
            .external_cli_agents
            .get_mut("grok")
            .unwrap()
            .providers[0]
            .env[0]
            .value = new.to_string_lossy().into_owned();
        persist_with_profiles(&before, &after, || Ok(())).unwrap();
        assert_eq!(
            std::fs::read_to_string(old.join("config.toml")).unwrap(),
            "[ui]\ncolor = 'old'\n"
        );
        let def = get_agent_def("grok").unwrap();
        assert_ne!(
            effective_toml_paths(def, &before.chat.external_cli_agents["grok"])
                .unwrap()
                .1,
            effective_toml_paths(def, &after.chat.external_cli_agents["grok"])
                .unwrap()
                .1
        );
        let mut restore = after.clone();
        restore
            .chat
            .external_cli_agents
            .get_mut("grok")
            .unwrap()
            .current_provider
            .clear();
        // Keep the resolved home when returning to the native configuration.
        restore
            .chat
            .external_cli_agents
            .get_mut("grok")
            .unwrap()
            .env
            .push(crate::settings::CliEnvVar {
                key: "GROK_HOME".into(),
                value: new.to_string_lossy().into_owned(),
            });
        persist_with_profiles(&after, &restore, || Ok(())).unwrap();
        assert_eq!(
            std::fs::read_to_string(new.join("config.toml")).unwrap(),
            "[ui]\ncolor = 'new'\n"
        );
    }

    #[test]
    fn unknown_agent_has_no_provider_profile_route() {
        let error =
            materialize_config("unknown-agent", &ExternalCliAgentConfig::default()).unwrap_err();
        assert!(error.contains("未知外部 Agent"));
    }

    fn test_paths(root: &Path, pi: bool) -> NativePaths {
        NativePaths {
            config: root.join(if pi { "models.json" } else { "opencode.json" }),
            alternate_configs: Vec::new(),
            auth: root.join("auth.json"),
            settings: pi.then(|| root.join("settings.json")),
            state: root.join("kivio-state.json"),
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("kivio-{label}-{}", uuid::Uuid::new_v4()))
    }

    fn native_provider(
        id: &str,
        name: &str,
        config_json: Value,
        auth_json: Value,
        default_model: &str,
    ) -> ExternalCliProvider {
        ExternalCliProvider {
            id: id.to_string(),
            name: name.to_string(),
            config_json: serde_json::to_string(&config_json).unwrap(),
            auth_json: serde_json::to_string(&auth_json).unwrap(),
            default_model: default_model.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn dsh_env_includes_all_providers_and_default_wins_conflicts() {
        let make = |id: &str, unique_key: &str, shared_value: &str| ExternalCliProvider {
            id: id.to_string(),
            env: vec![
                crate::settings::CliEnvVar {
                    key: unique_key.to_string(),
                    value: format!("{id}-key"),
                },
                crate::settings::CliEnvVar {
                    key: "SHARED_KEY".to_string(),
                    value: shared_value.to_string(),
                },
            ],
            ..Default::default()
        };
        let mut disabled = make("disabled", "DISABLED_KEY", "disabled");
        disabled.disabled = true;
        let config = ExternalCliAgentConfig {
            providers: vec![
                make("first", "FIRST_KEY", "first"),
                make("second", "SECOND_KEY", "second"),
                disabled,
            ],
            current_provider: "second".to_string(),
            ..Default::default()
        };

        let env = dsh_provider_env(&config);
        assert_eq!(env.get("FIRST_KEY").map(String::as_str), Some("first-key"));
        assert_eq!(
            env.get("SECOND_KEY").map(String::as_str),
            Some("second-key")
        );
        assert!(env.get("DISABLED_KEY").is_none());
        assert_eq!(env.get("SHARED_KEY").map(String::as_str), Some("second"));
    }

    #[test]
    fn sanitize_rejects_path_escapes() {
        assert_eq!(
            sanitize_segment("loki-claude").as_deref(),
            Some("loki-claude")
        );
        assert!(sanitize_segment("../../etc/passwd").is_none());
        assert!(sanitize_segment("a/b").is_none());
        assert!(sanitize_segment("a\\b").is_none());
        assert!(sanitize_segment("..").is_none());
        assert!(sanitize_segment("").is_none());
        assert!(sanitize_segment("CON").is_none());
        assert!(sanitize_segment("COM1").is_none());
        // COM0 不是保留名，别误伤。
        assert_eq!(sanitize_segment("COM0").as_deref(), Some("COM0"));
    }

    #[test]
    fn slugify_provider_key_keeps_dot_and_underscore() {
        assert_eq!(
            slugify_provider_key("My.Relay").as_deref(),
            Some("my.relay")
        );
        assert_eq!(
            slugify_provider_key("hello_world").as_deref(),
            Some("hello_world")
        );
        assert_eq!(
            slugify_provider_key("Relay One").as_deref(),
            Some("relay-one")
        );
        assert_eq!(slugify_provider_key("a--b").as_deref(), Some("a-b"));
    }

    #[test]
    fn native_cleanup_ids_prefer_name_slug_and_keep_legacy_aliases() {
        let ids = native_cleanup_ids("p-msoeiznl", Some("Relay One"), None);
        assert!(ids.iter().any(|id| id == "relay-one"));
        assert!(ids.iter().any(|id| id == "kivio-relay-one"));
        assert!(ids.iter().any(|id| id == "p-msoeiznl"));
        assert!(ids.iter().any(|id| id == "kivio-p-msoeiznl"));
    }

    #[test]
    fn opencode_merges_jsonc_and_restores_previous_default() {
        let root = temp_root("opencode-native");
        let paths = test_paths(&root, false);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &paths.config,
            r#"{
              // user comment: JSONC must parse
              "model": "anthropic/claude-old",
              "mcp": { "keep": true },
              "provider": { "user-relay": { "name": "Keep me" } },
            }"#,
        )
        .unwrap();
        std::fs::write(&paths.auth, r#"{"user-relay":{"type":"api","key":"keep"}}"#).unwrap();

        let provider = native_provider(
            "Relay One",
            "Relay One",
            serde_json::json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "Relay One",
                "options": { "baseURL": "https://relay.example/v1" },
                "models": { "gpt-test": { "name": "GPT Test" } }
            }),
            serde_json::json!({ "type": "api", "key": "sk-test" }),
            "gpt-test",
        );
        let mut config = ExternalCliAgentConfig {
            providers: vec![provider],
            current_provider: "Relay One".to_string(),
            ..Default::default()
        };

        materialize_native_at("opencode", &config, &paths).unwrap();
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert_eq!(written["mcp"]["keep"], true);
        assert_eq!(written["provider"]["user-relay"]["name"], "Keep me");
        assert_eq!(
            written["provider"]["relay-one"]["models"]["gpt-test"]["name"],
            "GPT Test"
        );
        assert_eq!(written["model"], "relay-one/gpt-test");
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.auth).unwrap()).unwrap();
        assert_eq!(auth["user-relay"]["key"], "keep");
        assert_eq!(auth["relay-one"]["key"], "sk-test");

        let mut user_edited = written.clone();
        user_edited["provider"]["relay-one"]["headerTimeout"] = serde_json::json!(12_000);
        user_edited["provider"]["relay-one"]["models"]["gpt-test"]["variants"] =
            serde_json::json!({ "high": { "reasoningEffort": "high" } });
        std::fs::write(
            &paths.config,
            serde_json::to_string_pretty(&user_edited).unwrap(),
        )
        .unwrap();
        materialize_native_at("opencode", &config, &paths).unwrap();
        let merged: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert_eq!(merged["provider"]["relay-one"]["headerTimeout"], 12_000);
        assert_eq!(
            merged["provider"]["relay-one"]["models"]["gpt-test"]["variants"]["high"]
                ["reasoningEffort"],
            "high"
        );

        config.current_provider.clear();
        materialize_native_at("opencode", &config, &paths).unwrap();
        let restored: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert_eq!(restored["model"], "anthropic/claude-old");
        assert!(restored["provider"]["relay-one"].is_object());

        materialize_native_at("opencode", &ExternalCliAgentConfig::default(), &paths).unwrap();
        let cleaned: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert!(cleaned["provider"].get("relay-one").is_none());
        assert!(cleaned["provider"]["user-relay"].is_object());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_uses_stable_id_and_removes_managed_auth_when_key_is_cleared() {
        let root = temp_root("opencode-stable-id");
        let paths = test_paths(&root, false);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&paths.config, "{}").unwrap();
        std::fs::write(&paths.auth, "{}").unwrap();
        let mut provider = native_provider(
            "internal-timestamp-id",
            "Relay",
            serde_json::json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "Relay",
                "options": { "baseURL": "https://relay.example/v1" },
                "models": { "claude-test": { "name": "Claude Test" } }
            }),
            serde_json::json!({ "type": "api", "key": "sk-test" }),
            "claude-test",
        );
        provider.native_provider_id = "team-relay".to_string();
        let mut config = ExternalCliAgentConfig {
            providers: vec![provider],
            current_provider: "internal-timestamp-id".to_string(),
            ..Default::default()
        };

        materialize_native_at("opencode", &config, &paths).unwrap();
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert!(written["provider"]["team-relay"].is_object());
        assert_eq!(written["model"], "team-relay/claude-test");

        config.providers[0].auth_json.clear();
        config.providers[0].config_json = serde_json::to_string(&serde_json::json!({
            "npm": "@ai-sdk/anthropic",
            "name": "Relay",
            "options": {},
            "models": { "claude-test": { "name": "Claude Test" } }
        }))
        .unwrap();
        materialize_native_at("opencode", &config, &paths).unwrap();
        let switched: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert_eq!(
            switched["provider"]["team-relay"]["npm"],
            "@ai-sdk/anthropic"
        );
        assert!(switched["provider"]["team-relay"]["options"]
            .get("baseURL")
            .is_none());
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.auth).unwrap()).unwrap();
        assert!(auth.get("team-relay").is_none());

        materialize_native_at("opencode", &ExternalCliAgentConfig::default(), &paths).unwrap();
        let cleaned: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert!(cleaned["provider"].get("team-relay").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_refuses_to_overwrite_unmanaged_provider_id() {
        let root = temp_root("opencode-provider-collision");
        let paths = test_paths(&root, false);
        std::fs::create_dir_all(&root).unwrap();
        let original = r#"{"provider":{"team-relay":{"name":"User managed"}}}"#;
        std::fs::write(&paths.config, original).unwrap();
        std::fs::write(&paths.auth, "{}").unwrap();
        let mut provider = native_provider(
            "internal-id",
            "Relay",
            serde_json::json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "Relay",
                "options": { "baseURL": "https://relay.example/v1" },
                "models": { "gpt-test": { "name": "GPT Test" } }
            }),
            serde_json::json!({ "type": "api", "key": "sk-test" }),
            "gpt-test",
        );
        provider.native_provider_id = "team-relay".to_string();
        let config = ExternalCliAgentConfig {
            providers: vec![provider],
            current_provider: "internal-id".to_string(),
            ..Default::default()
        };

        let err = materialize_native_at("opencode", &config, &paths).unwrap_err();
        assert!(err.contains("非 Kivio 管理"));
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), original);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_moves_managed_entries_to_the_highest_priority_config() {
        let root = temp_root("opencode-config-precedence");
        std::fs::create_dir_all(&root).unwrap();
        let lower_path = root.join("opencode.json");
        let higher_path = root.join("opencode.jsonc");
        let lower_paths = test_paths(&root, false);
        std::fs::write(&lower_path, r#"{"model":"user/old"}"#).unwrap();
        std::fs::write(&lower_paths.auth, "{}").unwrap();
        let provider = native_provider(
            "Relay One",
            "Relay One",
            serde_json::json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "Relay One",
                "options": { "baseURL": "https://relay.example/v1" },
                "models": { "gpt-test": { "name": "GPT Test" } }
            }),
            serde_json::json!({ "type": "api", "key": "sk-test" }),
            "gpt-test",
        );
        let mut config = ExternalCliAgentConfig {
            providers: vec![provider],
            current_provider: "Relay One".to_string(),
            ..Default::default()
        };
        materialize_native_at("opencode", &config, &lower_paths).unwrap();

        std::fs::write(
            &higher_path,
            r#"{
              // Higher-priority user config.
              "model": "top/selected",
              "provider": { "top": { "name": "Top" } },
            }"#,
        )
        .unwrap();
        let higher_paths = NativePaths {
            config: higher_path.clone(),
            alternate_configs: vec![lower_path.clone()],
            auth: lower_paths.auth.clone(),
            settings: None,
            state: lower_paths.state.clone(),
        };
        materialize_native_at("opencode", &config, &higher_paths).unwrap();

        let lower: Value =
            serde_json::from_str(&std::fs::read_to_string(&lower_path).unwrap()).unwrap();
        assert_eq!(lower["model"], "user/old");
        assert!(lower["provider"].get("relay-one").is_none());
        let higher: Value =
            serde_json::from_str(&std::fs::read_to_string(&higher_path).unwrap()).unwrap();
        assert_eq!(higher["model"], "relay-one/gpt-test");
        assert!(higher["provider"]["top"].is_object());

        config.current_provider.clear();
        materialize_native_at("opencode", &config, &higher_paths).unwrap();
        let restored: Value =
            serde_json::from_str(&std::fs::read_to_string(&higher_path).unwrap()).unwrap();
        assert_eq!(restored["model"], "top/selected");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn opencode_cleanup_active_provider_restores_previous_default() {
        let root = temp_root("opencode-cleanup-active");
        let paths = test_paths(&root, false);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &paths.config,
            r#"{"model":"user-relay/old","provider":{"user-relay":{"name":"User"}}}"#,
        )
        .unwrap();
        std::fs::write(&paths.auth, "{}").unwrap();
        let provider = native_provider(
            "Relay One",
            "Relay One",
            serde_json::json!({
                "npm": "@ai-sdk/openai-compatible",
                "name": "Relay One",
                "options": { "baseURL": "https://relay.example/v1" },
                "models": { "gpt-test": { "name": "GPT Test" } }
            }),
            serde_json::json!({ "type": "api", "key": "sk-test" }),
            "gpt-test",
        );
        let config = ExternalCliAgentConfig {
            providers: vec![provider],
            current_provider: "Relay One".to_string(),
            ..Default::default()
        };

        materialize_native_at("opencode", &config, &paths).unwrap();
        materialize_native_at("opencode", &ExternalCliAgentConfig::default(), &paths).unwrap();
        let cleaned: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert_eq!(cleaned["model"], "user-relay/old");
        assert!(cleaned["provider"].get("relay-one").is_none());
        let state = read_managed_state(&paths.state).unwrap();
        assert!(!state.defaults_managed);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pi_switches_managed_providers_without_overwriting_backup() {
        let root = temp_root("pi-native");
        let paths = test_paths(&root, true);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &paths.config,
            r#"{"providers":{"user":{"name":"User","models":[]}}}"#,
        )
        .unwrap();
        std::fs::write(&paths.auth, "{}").unwrap();
        std::fs::write(
            paths.settings.as_ref().unwrap(),
            r#"{"theme":"light","defaultProvider":"user","defaultModel":"old","defaultThinkingLevel":"low"}"#,
        )
        .unwrap();
        let make = |id: &str, model: &str| {
            let mut provider = native_provider(
                id,
                id,
                serde_json::json!({
                    "name": id,
                    "baseUrl": "https://relay.example/v1",
                    "api": "openai-completions",
                    "models": [{ "id": model, "name": model, "reasoning": true }]
                }),
                serde_json::json!({ "type": "api_key", "key": format!("sk-{id}") }),
                model,
            );
            provider.default_reasoning = "high".to_string();
            provider
        };
        let mut config = ExternalCliAgentConfig {
            providers: vec![make("First", "m1"), make("Second", "m2")],
            current_provider: "First".to_string(),
            ..Default::default()
        };

        materialize_native_at("pi", &config, &paths).unwrap();
        config.current_provider = "Second".to_string();
        materialize_native_at("pi", &config, &paths).unwrap();
        let active: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.settings.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(active["defaultProvider"], "second");
        assert_eq!(active["defaultModel"], "m2");
        assert_eq!(active["defaultThinkingLevel"], "high");
        assert_eq!(active["theme"], "light");

        config.current_provider.clear();
        materialize_native_at("pi", &config, &paths).unwrap();
        let restored: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.settings.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(restored["defaultProvider"], "user");
        assert_eq!(restored["defaultModel"], "old");
        assert_eq!(restored["defaultThinkingLevel"], "low");
        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert!(models["providers"]["user"].is_object());
        assert!(models["providers"]["first"].is_object());
        assert!(models["providers"]["second"].is_object());

        config.providers[0].disabled = true;
        materialize_native_at("pi", &config, &paths).unwrap();
        let filtered: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert!(filtered["providers"].get("first").is_none());
        assert!(filtered["providers"]["second"].is_object());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pi_cleanup_active_provider_restores_previous_default() {
        let root = temp_root("pi-cleanup-active");
        let paths = test_paths(&root, true);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &paths.config,
            r#"{"providers":{"user":{"name":"User","models":[]}}}"#,
        )
        .unwrap();
        std::fs::write(&paths.auth, "{}").unwrap();
        std::fs::write(
            paths.settings.as_ref().unwrap(),
            r#"{"defaultProvider":"user","defaultModel":"old"}"#,
        )
        .unwrap();
        let mut provider = native_provider(
            "Relay One",
            "Relay One",
            serde_json::json!({
                "name": "Relay One",
                "baseUrl": "https://relay.example/v1",
                "api": "openai-completions",
                "models": [{ "id": "gpt-test", "name": "GPT Test", "reasoning": true }]
            }),
            serde_json::json!({ "type": "api_key", "key": "sk-test" }),
            "gpt-test",
        );
        provider.default_reasoning = "high".to_string();
        let config = ExternalCliAgentConfig {
            providers: vec![provider],
            current_provider: "Relay One".to_string(),
            ..Default::default()
        };

        materialize_native_at("pi", &config, &paths).unwrap();
        materialize_native_at("pi", &ExternalCliAgentConfig::default(), &paths).unwrap();
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.settings.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["defaultProvider"], "user");
        assert_eq!(settings["defaultModel"], "old");
        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert!(models["providers"].get("relay-one").is_none());
        let state = read_managed_state(&paths.state).unwrap();
        assert!(!state.defaults_managed);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cleanup_removes_legacy_kivio_alias_when_deleting_by_internal_id() {
        let root = temp_root("pi-cleanup-legacy");
        let paths = test_paths(&root, true);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &paths.config,
            r#"{"providers":{"relay-one":{"name":"Relay"},"kivio-p-msoeiznl":{"name":"Legacy"}}}"#,
        )
        .unwrap();
        std::fs::write(
            &paths.auth,
            r#"{"relay-one":{"type":"api_key","key":"new"},"kivio-p-msoeiznl":{"type":"api_key","key":"old"}}"#,
        )
        .unwrap();
        std::fs::write(
            paths.settings.as_ref().unwrap(),
            r#"{"defaultProvider":"relay-one"}"#,
        )
        .unwrap();
        std::fs::write(
            &paths.state,
            r#"{"managedProviderIds":["relay-one","kivio-p-msoeiznl"],"defaultsManaged":false}"#,
        )
        .unwrap();

        materialize_native_at("pi", &ExternalCliAgentConfig::default(), &paths).unwrap();
        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.config).unwrap()).unwrap();
        assert!(models["providers"].get("relay-one").is_none());
        assert!(models["providers"].get("kivio-p-msoeiznl").is_none());
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.auth).unwrap()).unwrap();
        assert!(auth.get("relay-one").is_none());
        assert!(auth.get("kivio-p-msoeiznl").is_none());
        let state = read_managed_state(&paths.state).unwrap();
        assert!(state.managed_provider_ids.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pi_upgrade_backs_up_existing_thinking_level_before_managing_it() {
        let root = temp_root("pi-thinking-upgrade");
        let paths = test_paths(&root, true);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&paths.config, r#"{"providers":{}}"#).unwrap();
        std::fs::write(&paths.auth, "{}").unwrap();
        std::fs::write(
            paths.settings.as_ref().unwrap(),
            r#"{"defaultProvider":"kivio-relay","defaultModel":"gpt-test","defaultThinkingLevel":"medium"}"#,
        )
        .unwrap();
        std::fs::write(
            &paths.state,
            r#"{
              "managedProviderIds": ["kivio-relay"],
              "defaultsManaged": true,
              "previousDefaults": {
                "defaultProvider": {"present": true, "value": "user"},
                "defaultModel": {"present": true, "value": "old"}
              }
            }"#,
        )
        .unwrap();
        let mut provider = native_provider(
            "Relay",
            "Relay",
            serde_json::json!({
                "name": "Relay",
                "baseUrl": "https://relay.example/v1",
                "api": "openai-responses",
                "models": [{ "id": "gpt-test", "reasoning": true }]
            }),
            serde_json::json!({ "type": "api_key", "key": "sk-test" }),
            "gpt-test",
        );
        provider.default_reasoning = "high".to_string();
        let mut config = ExternalCliAgentConfig {
            providers: vec![provider],
            current_provider: "Relay".to_string(),
            ..Default::default()
        };

        materialize_native_at("pi", &config, &paths).unwrap();
        let managed: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.settings.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(managed["defaultThinkingLevel"], "high");

        config.current_provider.clear();
        materialize_native_at("pi", &config, &paths).unwrap();
        let restored: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.settings.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(restored["defaultProvider"], "user");
        assert_eq!(restored["defaultModel"], "old");
        assert_eq!(restored["defaultThinkingLevel"], "medium");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pi_validates_default_thinking_level_against_model_mapping() {
        let make = |reasoning: bool, mapping: Value, level: &str| {
            let mut provider = native_provider(
                "Relay",
                "Relay",
                serde_json::json!({
                    "name": "Relay",
                    "baseUrl": "https://relay.example/v1",
                    "api": "openai-responses",
                    "models": [{
                        "id": "model",
                        "reasoning": reasoning,
                        "thinkingLevelMap": mapping
                    }]
                }),
                serde_json::json!({ "type": "api_key", "key": "sk-test" }),
                "model",
            );
            provider.default_reasoning = level.to_string();
            provider
        };

        for level in ["off", "minimal", "low", "medium", "high"] {
            assert!(
                parse_native_entries("pi", &[make(true, serde_json::json!({}), level)]).is_ok()
            );
        }
        for level in ["xhigh", "max"] {
            assert!(parse_native_entries(
                "pi",
                &[make(true, serde_json::json!({ level: level }), level)]
            )
            .is_ok());
        }

        assert!(parse_native_entries(
            "pi",
            &[make(
                true,
                serde_json::json!({ "low": null, "xhigh": null }),
                "low"
            )]
        )
        .is_err());
        assert!(parse_native_entries("pi", &[make(true, serde_json::json!({}), "xhigh")]).is_err());
        assert!(parse_native_entries(
            "pi",
            &[make(false, serde_json::json!({ "high": "high" }), "high")]
        )
        .is_err());
        assert!(parse_native_entries(
            "pi",
            &[make(false, serde_json::json!({ "off": null }), "off")]
        )
        .is_ok());
    }

    #[test]
    fn pi_legacy_active_provider_is_unmanaged_but_missing_active_is_rejected() {
        let root = temp_root("pi-legacy-active");
        let paths = test_paths(&root, true);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&paths.config, r#"{"providers":{}}"#).unwrap();
        std::fs::write(&paths.auth, "{}").unwrap();
        std::fs::write(
            paths.settings.as_ref().unwrap(),
            r#"{"defaultProvider":"user","defaultModel":"old"}"#,
        )
        .unwrap();
        let legacy = ExternalCliProvider {
            id: "legacy".to_string(),
            name: "Legacy".to_string(),
            ..Default::default()
        };
        let mut config = ExternalCliAgentConfig {
            providers: vec![legacy],
            current_provider: "legacy".to_string(),
            ..Default::default()
        };

        materialize_native_at("pi", &config, &paths).unwrap();
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(paths.settings.as_ref().unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["defaultProvider"], "user");
        assert_eq!(settings["defaultModel"], "old");

        config.current_provider = "missing".to_string();
        let error = materialize_native_at("pi", &config, &paths).unwrap_err();
        assert!(error.contains("不存在"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn malformed_opencode_root_is_rejected_without_overwrite() {
        let root = temp_root("opencode-malformed");
        let paths = test_paths(&root, false);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&paths.config, "[1, 2, 3]").unwrap();
        let config = ExternalCliAgentConfig {
            providers: vec![native_provider(
                "relay",
                "Relay",
                serde_json::json!({ "models": { "m": { "name": "M" } } }),
                serde_json::json!({ "type": "api", "key": "sk" }),
                "m",
            )],
            current_provider: "relay".to_string(),
            ..Default::default()
        };
        assert!(materialize_native_at("opencode", &config, &paths).is_err());
        assert_eq!(std::fs::read_to_string(&paths.config).unwrap(), "[1, 2, 3]");
        let _ = std::fs::remove_dir_all(root);
    }

    fn grok_provider(id: &str, name: &str, config_toml: &str) -> ExternalCliProvider {
        ExternalCliProvider {
            id: id.to_string(),
            name: name.to_string(),
            config_toml: config_toml.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn grok_merge_preserves_marketplace_and_sets_default_model() {
        let base = r#"[models]
default = "old"

[marketplace]
official_marketplace_auto_installed = true

[ui]
yolo = false
"#;
        let provider = r#"[models]
default = "grok-4.5"

[model."grok-4.5"]
model = "grok-4.5"
base_url = "https://relay.example/v1"
api_key = "sk-x"
api_backend = "responses"
context_window = 500000
"#;
        let merged = merge_grok_provider_config(base, provider).unwrap();
        let doc: toml::Table = toml::from_str(&merged).unwrap();
        assert_eq!(
            doc.get("models")
                .and_then(|v| v.get("default"))
                .and_then(|v| v.as_str()),
            Some("grok-4.5")
        );
        assert!(doc
            .get("marketplace")
            .and_then(|v| v.get("official_marketplace_auto_installed"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        let model = doc
            .get("model")
            .and_then(|v| v.get("grok-4.5"))
            .and_then(|v| v.as_table())
            .expect("model entry");
        assert_eq!(
            model.get("base_url").and_then(|v| v.as_str()),
            Some("https://relay.example/v1")
        );
        assert_eq!(model.get("api_key").and_then(|v| v.as_str()), Some("sk-x"));
    }

    #[test]
    fn grok_materialize_restores_previous_config_on_clear() {
        let root = temp_root("grok-restore");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let state_path = root.join("state.json");
        let original = "[models]\ndefault = \"native\"\n\n[ui]\nyolo = true\n";
        std::fs::write(&path, original).unwrap();

        let mut config = ExternalCliAgentConfig {
            providers: vec![grok_provider(
                "relay",
                "Relay",
                r#"[models]
default = "relay-model"

[model."relay-model"]
model = "relay-model"
base_url = "https://relay.example/v1"
api_key = "sk"
"#,
            )],
            current_provider: "relay".to_string(),
            ..Default::default()
        };
        materialize_grok_at(&config, &path, &state_path).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("relay-model"));
        assert!(after.contains("yolo = true"));

        config.current_provider.clear();
        materialize_grok_at(&config, &path, &state_path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn grok_switch_providers_does_not_accumulate_old_model_keys() {
        let root = temp_root("grok-switch");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let state_path = root.join("state.json");
        std::fs::write(&path, "[cli]\nauto_update = true\n").unwrap();

        let mut config = ExternalCliAgentConfig {
            providers: vec![
                grok_provider(
                    "a",
                    "A",
                    r#"[models]
default = "model-a"

[model."model-a"]
model = "model-a"
base_url = "https://a.example/v1"
api_key = "ska"
"#,
                ),
                grok_provider(
                    "b",
                    "B",
                    r#"[models]
default = "model-b"

[model."model-b"]
model = "model-b"
base_url = "https://b.example/v1"
api_key = "skb"
"#,
                ),
            ],
            current_provider: "a".to_string(),
            ..Default::default()
        };
        materialize_grok_at(&config, &path, &state_path).unwrap();
        config.current_provider = "b".to_string();
        materialize_grok_at(&config, &path, &state_path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("model-b"));
        assert!(!text.contains("model-a"));
        assert!(text.contains("auto_update = true"));
        let _ = std::fs::remove_dir_all(root);
    }

    fn kimi_provider(id: &str, name: &str, config_toml: &str) -> ExternalCliProvider {
        ExternalCliProvider {
            id: id.to_string(),
            name: name.to_string(),
            config_toml: config_toml.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn kimi_merge_preserves_managed_oauth_and_sets_default_model() {
        let base = r#"default_model = "kimi-code/k3"

[thinking]
enabled = true

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
api_key = ""

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
"#;
        let provider = r#"default_model = "relay/gpt-5"

[providers.relay]
type = "openai"
base_url = "https://relay.example/v1"
api_key = "sk-x"

[models."relay/gpt-5"]
provider = "relay"
model = "gpt-5"
max_context_size = 128000
display_name = "GPT 5"
"#;
        let merged = merge_kimi_provider_config(base, provider).unwrap();
        let doc: toml::Table = toml::from_str(&merged).unwrap();
        assert_eq!(
            doc.get("default_model").and_then(|v| v.as_str()),
            Some("relay/gpt-5")
        );
        assert!(doc
            .get("thinking")
            .and_then(|v| v.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        let managed = doc
            .get("providers")
            .and_then(|v| v.get("managed:kimi-code"))
            .and_then(|v| v.as_table())
            .expect("managed provider kept");
        assert_eq!(managed.get("type").and_then(|v| v.as_str()), Some("kimi"));
        let relay = doc
            .get("providers")
            .and_then(|v| v.get("relay"))
            .and_then(|v| v.as_table())
            .expect("relay provider");
        assert_eq!(
            relay.get("base_url").and_then(|v| v.as_str()),
            Some("https://relay.example/v1")
        );
        let model = doc
            .get("models")
            .and_then(|v| v.get("relay/gpt-5"))
            .and_then(|v| v.as_table())
            .expect("model entry");
        assert_eq!(model.get("model").and_then(|v| v.as_str()), Some("gpt-5"));
        assert_eq!(
            model.get("max_context_size").and_then(|v| v.as_integer()),
            Some(128000)
        );
    }

    #[test]
    fn kimi_materialize_restores_previous_config_on_clear() {
        let root = temp_root("kimi-restore");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let state_path = root.join("state.json");
        let original = "default_model = \"kimi-code/k3\"\n\n[thinking]\nenabled = true\n";
        std::fs::write(&path, original).unwrap();

        let mut config = ExternalCliAgentConfig {
            providers: vec![kimi_provider(
                "relay",
                "Relay",
                r#"default_model = "relay/m1"

[providers.relay]
type = "openai"
base_url = "https://relay.example/v1"
api_key = "sk"

[models."relay/m1"]
provider = "relay"
model = "m1"
max_context_size = 128000
"#,
            )],
            current_provider: "relay".to_string(),
            ..Default::default()
        };
        materialize_kimi_at(&config, &path, &state_path).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("relay/m1"));
        assert!(after.contains("enabled = true"));

        config.current_provider.clear();
        materialize_kimi_at(&config, &path, &state_path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn kimi_switch_providers_starts_from_backup_not_previous_relay() {
        let root = temp_root("kimi-switch");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        let state_path = root.join("state.json");
        std::fs::write(
            &path,
            "[providers.\"managed:kimi-code\"]\ntype = \"kimi\"\n",
        )
        .unwrap();

        let mut config = ExternalCliAgentConfig {
            providers: vec![
                kimi_provider(
                    "a",
                    "A",
                    r#"default_model = "a/m-a"

[providers.a]
type = "openai"
base_url = "https://a.example/v1"
api_key = "ska"

[models."a/m-a"]
provider = "a"
model = "m-a"
max_context_size = 128000
"#,
                ),
                kimi_provider(
                    "b",
                    "B",
                    r#"default_model = "b/m-b"

[providers.b]
type = "openai_responses"
base_url = "https://b.example/v1"
api_key = "skb"

[models."b/m-b"]
provider = "b"
model = "m-b"
max_context_size = 200000
"#,
                ),
            ],
            current_provider: "a".to_string(),
            ..Default::default()
        };
        materialize_kimi_at(&config, &path, &state_path).unwrap();
        config.current_provider = "b".to_string();
        materialize_kimi_at(&config, &path, &state_path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("m-b"));
        assert!(text.contains("openai_responses"));
        // 从备份合并：managed OAuth 还在；A 的 provider 不应残留
        assert!(text.contains("managed:kimi-code"));
        assert!(!text.contains("[providers.a]") && !text.contains("providers.a"));
        // toml pretty 可能写成 [providers.a] 或 nested table — 用解析确认
        let doc: toml::Table = toml::from_str(&text).unwrap();
        let providers = doc.get("providers").and_then(|v| v.as_table()).unwrap();
        assert!(providers.get("a").is_none());
        assert!(providers.get("b").is_some());
        assert!(providers.get("managed:kimi-code").is_some());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn pi_adaptive_thinking_model_detection() {
        for id in [
            "claude-opus-4-8",
            "claude-opus-4-6",
            "claude-opus-4.7",
            "claude-sonnet-5",
            "claude-fable-5",
            "claude-opus-4-6[1M]",
            "claude-sonnet-4-6-20250929",
            "claude-opus-5",
        ] {
            assert!(is_pi_adaptive_thinking_model(id), "expected adaptive: {id}");
        }
        for id in [
            "claude-sonnet-4-5",
            "claude-opus-4-1",
            "claude-3-5-sonnet",
            "gpt-5",
        ] {
            assert!(
                !is_pi_adaptive_thinking_model(id),
                "expected not adaptive: {id}"
            );
        }
    }

    #[test]
    fn ensure_pi_adaptive_thinking_compat_patches_anthropic_models() {
        let mut config = serde_json::json!({
            "api": "anthropic-messages",
            "baseUrl": "https://relay.example",
            "models": [
                { "id": "claude-opus-4-8", "reasoning": true },
                { "id": "claude-sonnet-4-5", "reasoning": true },
                { "id": "claude-fable-5", "reasoning": true, "compat": { "allowEmptySignature": true } }
            ]
        })
        .as_object()
        .cloned()
        .unwrap();
        ensure_pi_adaptive_thinking_compat(&mut config);
        let models = config.get("models").and_then(Value::as_array).unwrap();
        assert_eq!(
            models[0]
                .get("compat")
                .and_then(|c| c.get("forceAdaptiveThinking"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(models[1].get("compat").is_none());
        // 已有其它 compat 键时只补 forceAdaptiveThinking，不抹掉原字段。
        assert_eq!(
            models[2]
                .get("compat")
                .and_then(|c| c.get("forceAdaptiveThinking"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            models[2]
                .get("compat")
                .and_then(|c| c.get("allowEmptySignature"))
                .and_then(Value::as_bool),
            Some(true)
        );

        // OpenAI 线不动。
        let mut openai = serde_json::json!({
            "api": "openai-completions",
            "models": [{ "id": "claude-opus-4-8" }]
        })
        .as_object()
        .cloned()
        .unwrap();
        ensure_pi_adaptive_thinking_compat(&mut openai);
        assert!(openai.get("models").and_then(Value::as_array).unwrap()[0]
            .get("compat")
            .is_none());
    }

    #[test]
    fn claude_live_model_matches_cc_switch_env_fields() {
        let root = temp_root("claude-model");
        let path = root.join("settings.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &path,
            r#"{"effortLevel":"high","env":{"ANTHROPIC_BASE_URL":"https://x"}}"#,
        )
        .unwrap();
        upsert_claude_live_model(&path, "claude-sonnet-5").unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["model"], "claude-sonnet-5");
        assert_eq!(value["effortLevel"], "high");
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://x");
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-sonnet-5");
        assert_eq!(
            value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            "claude-sonnet-5"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn claude_live_model_skips_family_env_for_unrecognized_ids() {
        let root = temp_root("claude-model-freeform");
        let path = root.join("settings.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&path, r#"{"env":{}}"#).unwrap();
        upsert_claude_live_model(&path, "glm-5.2").unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["model"], "glm-5.2");
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "glm-5.2");
        assert!(value["env"].get("ANTHROPIC_DEFAULT_SONNET_MODEL").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn claude_materialize_keeps_chat_model_and_provider_family_maps() {
        let root = temp_root("claude-overlay");
        let path = root.join("claude-relay.json");
        std::fs::create_dir_all(&root).unwrap();
        upsert_claude_live_model(&path, "claude-sonnet-5").unwrap();
        let provider = ExternalCliProvider {
            id: "relay".to_string(),
            env: vec![
                crate::settings::CliEnvVar {
                    key: "ANTHROPIC_BASE_URL".to_string(),
                    value: "https://relay.example".to_string(),
                },
                crate::settings::CliEnvVar {
                    key: "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
                    value: "glm-5.2".to_string(),
                },
            ],
            ..Default::default()
        };
        materialize_claude_to(&path, &provider).unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["model"], "claude-sonnet-5");
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-sonnet-5");
        assert_eq!(value["env"]["ANTHROPIC_BASE_URL"], "https://relay.example");
        assert_eq!(value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "glm-5.2");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn claude_materialize_blanks_stale_default_model_instead_of_preserving_it() {
        let root = temp_root("claude-default-model");
        let path = root.join("claude-relay.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &path,
            r#"{"env":{"ANTHROPIC_DEFAULT_MODEL":"old-relay-opus","ANTHROPIC_MODEL":"claude-sonnet-5"}}"#,
        )
        .unwrap();
        let provider = ExternalCliProvider {
            id: "relay".to_string(),
            env: vec![crate::settings::CliEnvVar {
                key: "ANTHROPIC_BASE_URL".to_string(),
                value: "https://relay.example".to_string(),
            }],
            ..Default::default()
        };
        materialize_claude_to(&path, &provider).unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-sonnet-5");
        assert_eq!(
            value["env"]["ANTHROPIC_DEFAULT_MODEL"], "",
            "切供应商后新会话默认模型不该从旧 overlay 填回来：{value}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn codex_model_upsert_keeps_provider_tables() {
        let original =
            "approval_policy = \"on-request\"\n\n[model_providers.relay]\nname = \"relay\"\n";
        let next = upsert_toml_toplevel_string(original, "model", "gpt-5.5");
        assert!(next.contains("model = \"gpt-5.5\""));
        assert!(next.contains("approval_policy = \"on-request\""));
        assert!(next.contains("[model_providers.relay]"));
        assert!(
            next.find("model = \"gpt-5.5\"").unwrap()
                < next.find("[model_providers.relay]").unwrap()
        );
        let replaced = upsert_toml_toplevel_string(&next, "model", "gpt-5.4");
        assert!(replaced.contains("model = \"gpt-5.4\""));
        assert!(!replaced.contains("model = \"gpt-5.5\""));
        assert_eq!(
            read_toml_toplevel_string(&replaced, "model").as_deref(),
            Some("gpt-5.4")
        );
    }

    #[test]
    fn codex_rematerialize_keeps_chat_selected_model() {
        let root = temp_root("codex-preserve-model");
        let path = root.join("config.toml");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &path,
            "model = \"gpt-5.5\"\nmodel_reasoning_effort = \"high\"\n\n[model_providers.relay]\nname = \"relay\"\n",
        )
        .unwrap();
        write_codex_config_preserving_chat_model(
            &path,
            "model = \"gpt-5\"\n\n[model_providers.relay]\nname = \"relay\"\nbase_url = \"https://x\"\n",
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("model = \"gpt-5.5\""));
        assert!(text.contains("model_reasoning_effort = \"high\""));
        assert!(text.contains("base_url = \"https://x\""));
        assert!(!text.contains("model = \"gpt-5\"\n"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn existing_unique_files_skips_missing_and_dedups() {
        let root = temp_root("codex-existing-files");
        let present = root.join("config.toml");
        let missing = root.join("missing.toml");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&present, "model = \"gpt-5\"\n").unwrap();
        let paths = existing_unique_files([present.clone(), present.clone(), missing]);
        assert_eq!(paths, vec![present]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn persist_selected_model_skips_auto_and_other_clis() {
        persist_selected_model("claude", Some("default"), None).unwrap();
        persist_selected_model("claude", Some(""), None).unwrap();
        persist_selected_model("claude", None, None).unwrap();
        persist_selected_model("pi", Some("gpt-test"), None).unwrap();
    }

    #[test]
    fn native_codex_home_is_user_dot_codex() {
        let home = native_codex_home().expect("home dir");
        assert_eq!(
            home.file_name().and_then(|name| name.to_str()),
            Some(".codex")
        );
    }

    #[test]
    fn wsl_shared_codex_home_is_host_path_only_for_wsl_codex() {
        let wsl = PathBuf::from(r"\\wsl$\Ubuntu\usr\bin\codex");
        let win = PathBuf::from(r"C:\codex.exe");
        assert!(wsl_shared_codex_home(&win).is_none());
        let home = wsl_shared_codex_home(&wsl).expect("wsl shares the host Codex home");
        assert_eq!(
            home.file_name().and_then(|name| name.to_str()),
            Some(".codex")
        );
        assert!(!home
            .to_string_lossy()
            .replace('\\', "/")
            .starts_with("/mnt/"));
    }

    #[test]
    fn kivio_private_codex_home_is_the_materialized_dir() {
        // Join so macOS CI Path semantics match Windows (`\` is not a separator on Unix).
        let private = Path::new("com.zmair.kivio")
            .join("external-cli-providers")
            .join("codex-relay");
        assert!(is_kivio_private_codex_home(&private));
        assert!(!is_kivio_private_codex_home(Path::new(".codex")));
        assert!(!is_kivio_private_codex_home(
            &Path::new("external-cli-providers").join("other")
        ));
    }

    #[test]
    fn share_codex_sessions_dir_migrates_private_rollouts_then_links() {
        let root = temp_root("codex-share-sessions");
        let private_home = root.join("external-cli-providers").join("codex-relay");
        let user_sessions = root.join("user-codex").join("sessions");
        let nested = private_home
            .join("sessions")
            .join("2026")
            .join("08")
            .join("24");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("rollout-kivio.jsonl"), "kivio\n").unwrap();
        std::fs::create_dir_all(user_sessions.join("2026")).unwrap();
        std::fs::write(user_sessions.join("2026").join("keep-me.jsonl"), "tui\n").unwrap();

        share_codex_sessions_dir(&private_home, &user_sessions).unwrap();
        share_codex_sessions_dir(&private_home, &user_sessions).unwrap();

        assert_eq!(
            std::fs::read_to_string(
                user_sessions
                    .join("2026")
                    .join("08")
                    .join("24")
                    .join("rollout-kivio.jsonl")
            )
            .unwrap(),
            "kivio\n"
        );
        assert_eq!(
            std::fs::read_to_string(user_sessions.join("2026").join("keep-me.jsonl")).unwrap(),
            "tui\n"
        );
        assert!(paths_point_to_same_dir(
            &private_home.join("sessions"),
            &user_sessions
        ));
        std::fs::write(
            private_home.join("sessions").join("via-link.jsonl"),
            "linked\n",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(user_sessions.join("via-link.jsonl")).unwrap(),
            "linked\n"
        );

        unlink_directory_link(&private_home.join("sessions"));
        std::fs::write(user_sessions.join("still-there.jsonl"), "ok\n").unwrap();
        assert!(user_sessions.join("still-there.jsonl").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
