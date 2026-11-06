//! 设置页「本地 CLI Agent」里用户填的每-CLI 覆盖（自定义路径 / 环境变量 / 自定义模型 / 停用）
//! 的进程内镜像。
//!
//! 为什么是全局而不是参数传递：`spawn::resolve_binary` / `detection::*` 这一整条探测+拉起链
//! 都没有 `AppHandle`（它们也被无 Tauri 上下文的路径复用），要把覆盖顺着传下去得改十几个签名。
//!
//! 仅启动和设置提交同步镜像；普通读取及临时设置校验不能改变运行身份。
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::hash::{BuildHasher, RandomState};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};
use std::time::SystemTime;

use super::{provider_profile, registry, types::RuntimeAgentDef, wsl};
use crate::settings::{CliCustomModel, ExternalCliAgentConfig, ExternalCliProvider, Settings};

static OVERRIDES: LazyLock<RwLock<OverrideState>> = LazyLock::new(Default::default);
static LAUNCH_HASHER: LazyLock<RandomState> = LazyLock::new(RandomState::new);

#[derive(Default)]
struct OverrideState {
    configs: HashMap<String, ExternalCliAgentConfig>,
    epochs: HashMap<&'static str, LaunchEpoch>,
}

#[derive(Default)]
struct LaunchEpoch {
    revision: u64,
    updating: usize,
}

impl OverrideState {
    fn replace(&mut self, configs: HashMap<String, ExternalCliAgentConfig>) {
        for def in registry::AGENT_DEFS {
            if launch_changed(def, self.configs.get(def.id), configs.get(def.id)) {
                self.epochs.entry(def.id).or_default().revision += 1;
            }
        }
        self.configs = configs;
    }

    fn launch_revision(&self, id: &str) -> Option<u64> {
        match self.epochs.get(id) {
            Some(epoch) if epoch.updating > 0 => None,
            Some(epoch) => Some(epoch.revision),
            None => Some(0),
        }
    }

    fn begin_update(&mut self, before: &Settings, after: &Settings) -> Vec<&'static str> {
        registry::AGENT_DEFS
            .iter()
            .filter(|def| {
                launch_changed(
                    def,
                    before.chat.external_cli_agents.get(def.id),
                    after.chat.external_cli_agents.get(def.id),
                )
            })
            .map(|def| {
                let epoch = self.epochs.entry(def.id).or_default();
                epoch.revision += 1;
                epoch.updating += 1;
                def.id
            })
            .collect()
    }

    fn end_update(&mut self, ids: &[&'static str]) {
        for id in ids {
            if let Some(epoch) = self.epochs.get_mut(id) {
                epoch.updating -= 1;
            }
        }
    }
}

fn launch_changed(
    def: &RuntimeAgentDef,
    before: Option<&ExternalCliAgentConfig>,
    after: Option<&ExternalCliAgentConfig>,
) -> bool {
    if before == after {
        return false;
    }
    let empty = ExternalCliAgentConfig::default();
    let before = before.unwrap_or(&empty);
    let after = after.unwrap_or(&empty);
    before.path != after.path
        || env_for_config(def.id, Some(before)) != env_for_config(def.id, Some(after))
        || provider_profile::launch_profile_changed(def.provider_profile, before, after)
}

pub(crate) fn launch_revision(id: &str) -> Option<u64> {
    OVERRIDES.read().ok()?.launch_revision(id)
}

pub(crate) struct LaunchUpdate(Vec<&'static str>);

pub(crate) fn begin_launch_update(previous: &Settings, next: &Settings) -> LaunchUpdate {
    LaunchUpdate(
        OVERRIDES
            .write()
            .map(|mut state| state.begin_update(previous, next))
            .unwrap_or_default(),
    )
}

impl Drop for LaunchUpdate {
    fn drop(&mut self) {
        if let Ok(mut state) = OVERRIDES.write() {
            state.end_update(&self.0);
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct LaunchIdentity {
    agent: &'static str,
    binary: PathBuf,
    executable: Option<ExecutableRevision>,
    revision: Option<u64>,
    environment: u64,
    profile: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct ExecutableRevision {
    len: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    file_id: (u64, u64, i64, i64),
}

impl ExecutableRevision {
    fn read(bin: &Path) -> Option<Self> {
        // shortcut: use the WSL path as the file identity, add a distro-local revision when available without waking WSL on reuse.
        if wsl::is_wsl_target(bin) {
            return None;
        }
        let metadata = std::fs::metadata(bin).ok()?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Some(Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            file_id: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

impl std::fmt::Debug for LaunchIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LaunchIdentity")
            .finish_non_exhaustive()
    }
}

impl LaunchIdentity {
    pub(crate) fn capture(def: &RuntimeAgentDef, bin: &Path) -> Self {
        let (config, revision) = OVERRIDES
            .read()
            .map(|state| {
                (
                    state.configs.get(def.id).cloned(),
                    state.launch_revision(def.id),
                )
            })
            .unwrap_or_default();
        let mut env: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
        let configured = env_for_config(def.id, config.as_ref());
        if def.launch.home == super::types::AgentHomeStrategy::WslSharedCodexHome
            && !configured.contains_key("CODEX_HOME")
        {
            if let Some(home) = provider_profile::wsl_shared_codex_home(bin) {
                env.insert("CODEX_HOME".into(), home.into_os_string());
            }
        }
        env.extend(
            def.env
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into())),
        );
        env.extend(
            configured
                .into_iter()
                .map(|(key, value)| (key.into(), value.into())),
        );
        Self::from_environment(def, bin, revision, config.as_ref(), &env)
    }

    fn from_environment(
        def: &RuntimeAgentDef,
        bin: &Path,
        revision: Option<u64>,
        config: Option<&ExternalCliAgentConfig>,
        env: &BTreeMap<OsString, OsString>,
    ) -> Self {
        Self {
            agent: def.id,
            binary: bin.to_owned(),
            executable: ExecutableRevision::read(bin),
            revision,
            environment: LAUNCH_HASHER.hash_one(env),
            profile: provider_profile::launch_file_revision(def, bin, config, env, &*LAUNCH_HASHER),
        }
    }

    pub(crate) fn still_current(&self, def: &RuntimeAgentDef, bin: &Path) -> bool {
        self.revision.is_some()
            && self == &Self::capture(def, bin)
            && self.revision == launch_revision(def.id)
    }
}

pub fn sync_from_settings(settings: &Settings) {
    if let Ok(mut guard) = OVERRIDES.write() {
        guard.replace(settings.chat.external_cli_agents.clone());
    }
}

fn get(id: &str) -> Option<ExternalCliAgentConfig> {
    OVERRIDES.read().ok()?.configs.get(id).cloned()
}

/// 原生落盘层需要整份配置：不仅要看当前项，还要同步全部 Kivio 管理的 provider。
pub fn agent_config(id: &str) -> Option<ExternalCliAgentConfig> {
    get(id)
}

/// 用户是否停用了这个 CLI（不再出现在 Chat 的运行时选择器里）。
pub fn is_disabled(id: &str) -> bool {
    get(id).map(|cfg| cfg.disabled).unwrap_or(false)
}

/// 用户指定的可执行文件路径；空/未设 = 走 PATH 探测。
pub fn custom_path(id: &str) -> Option<PathBuf> {
    let path = get(id)?.path;
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// 当前生效的第三方供应商（设置页里选中的那条）。空 `current_provider` = 不托管。
pub fn active_provider(id: &str) -> Option<ExternalCliProvider> {
    let cfg = get(id)?;
    selected_provider(&cfg).cloned()
}

pub(super) fn selected_provider(cfg: &ExternalCliAgentConfig) -> Option<&ExternalCliProvider> {
    if cfg.current_provider.is_empty() {
        return None;
    }
    cfg.providers
        .iter()
        .find(|provider| provider.id == cfg.current_provider && !provider.disabled)
}

/// 注入该 CLI 子进程的环境变量（`ANTHROPIC_BASE_URL` 之类的中转配置）。
///
/// 供应商打底、用户手填的 env 覆盖在上面：手填是「我知道我在干什么」的逃生口，
/// 反过来会让用户改半天发现没生效。
pub fn env_for(id: &str) -> HashMap<String, String> {
    env_for_config(id, get(id).as_ref())
}

pub(crate) fn env_for_config(
    id: &str,
    config: Option<&ExternalCliAgentConfig>,
) -> HashMap<String, String> {
    match config {
        Some(cfg) => merge_env(provider_profile::provider_env_for(id, cfg), &cfg.env),
        None => HashMap::new(),
    }
}

/// 供应商 env 打底、手填 env 盖在上面。抽成纯函数是为了能不碰全局镜像地测优先级。
fn merge_env(
    mut base: HashMap<String, String>,
    manual: &[crate::settings::CliEnvVar],
) -> HashMap<String, String> {
    for pair in manual {
        base.insert(pair.key.clone(), pair.value.clone());
    }
    base
}

/// 用户手填的模型，合并进探测出来的下拉列表。
pub fn custom_models(id: &str) -> Vec<CliCustomModel> {
    get(id).map(|cfg| cfg.custom_models).unwrap_or_default()
}

/// 按二进制路径反查该 agent 的环境变量覆盖。
///
/// 为什么按路径而不是按 id：常驻会话 / 斜杠命令 / 导入这些 spawn 点散落在
/// `session/*.rs` 十几处，签名里都只有 `bin: &Path`，全部改成带 `def` 要顺带改三十多个
/// 调用点（大半是测试）。反查是纯函数，且新增 spawn 点自动覆盖，不会漏。
///
/// 只在**确实配了环境变量**的 agent 里找；各 CLI 的可执行名互不重复（claude / codex /
/// opencode / gemini / kimi / pi / hermes / grok / dsh），所以按文件名匹配是唯一的。
pub fn env_for_bin(bin: &std::path::Path) -> HashMap<String, String> {
    // 先只在锁里定位 agent id，出锁再拼环境变量：`env_for` 自己还要读一次这把锁，
    // 持读锁再读会在有写者排队时死锁。
    let hit = {
        let guard = match OVERRIDES.read() {
            Ok(guard) => guard,
            Err(_) => return HashMap::new(),
        };
        guard
            .configs
            .iter()
            .find(|(id, cfg)| {
                // dsh 的供应商可全部并存；即使 currentProvider 为空，显式选中其中一个模型时也要注入凭据。
                let has_coexisting_dsh_providers =
                    id.as_str() == "dsh" && cfg.providers.iter().any(|provider| !provider.disabled);
                if cfg.env.is_empty()
                    && cfg.current_provider.is_empty()
                    && !has_coexisting_dsh_providers
                {
                    return false;
                }
                if cfg.path.is_empty() {
                    crate::external_agents::registry::get_agent_def(id)
                        .is_some_and(|def| bin_name_matches(bin, def))
                } else {
                    configured_path_matches(&cfg.path, bin)
                }
            })
            .map(|(id, _)| id.clone())
    };
    hit.map(|id| env_for(&id)).unwrap_or_default()
}

fn configured_path_matches(configured: &str, bin: &Path) -> bool {
    Path::new(configured) == bin
        || (cfg!(windows)
            && configured.starts_with('/')
            && wsl::parse_wsl_target(bin).is_some_and(|target| target.linux_path == configured))
}

/// Windows 上 PATH 命中的可能是 `claude.cmd` / `claude.exe`，所以带扩展名和不带都比一遍。
fn bin_name_matches(
    bin: &std::path::Path,
    def: &crate::external_agents::types::RuntimeAgentDef,
) -> bool {
    let stem = bin.file_stem().and_then(|s| s.to_str());
    let name = bin.file_name().and_then(|s| s.to_str());
    std::iter::once(def.bin)
        .chain(def.fallback_bins.iter().copied())
        .any(|candidate| stem == Some(candidate) || name == Some(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::CliEnvVar;

    fn pair(key: &str, value: &str) -> CliEnvVar {
        CliEnvVar {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn launch_revision_tracks_only_effective_changes_and_survives_rollback() {
        let def = crate::external_agents::registry::get_agent_def("codex").unwrap();
        let mut state = OverrideState::default();
        let previous = Settings::default();
        let mut candidate = Settings::default();
        let mut config = ExternalCliAgentConfig::default();
        config.custom_models.push(CliCustomModel::default());
        candidate
            .chat
            .external_cli_agents
            .insert(def.id.to_string(), config.clone());
        assert!(state.begin_update(&previous, &candidate).is_empty());
        state.replace(candidate.chat.external_cli_agents.clone());
        assert_eq!(state.launch_revision(def.id), Some(0));

        config.env.push(pair("CODEX_HOME", "replacement-home"));
        candidate
            .chat
            .external_cli_agents
            .insert(def.id.to_string(), config);
        let affected = state.begin_update(&previous, &candidate);
        assert_eq!(state.launch_revision(def.id), None);
        assert_eq!(state.launch_revision("claude"), Some(0));
        state.replace(candidate.chat.external_cli_agents);
        state.replace(previous.chat.external_cli_agents);
        assert_eq!(state.launch_revision(def.id), None);
        state.end_update(&affected);
        assert!(state.launch_revision(def.id).unwrap() > 0);
    }

    #[test]
    fn launch_identity_changes_when_binary_or_account_changes() {
        let def = crate::external_agents::registry::get_agent_def("codex").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("codex");
        let auth = temp.path().join("auth.json");
        let config = temp.path().join("config.toml");
        std::fs::write(&bin, b"old executable").unwrap();
        std::fs::write(&auth, b"old account").unwrap();
        std::fs::write(&config, "model = 'first'\nmodel_provider = 'relay'\n").unwrap();
        let mut environment = [("CODEX_HOME".into(), temp.path().as_os_str().to_owned())].into();
        let capture = |environment: &BTreeMap<OsString, OsString>| {
            LaunchIdentity::from_environment(def, &bin, Some(0), None, environment)
        };
        let first = capture(&environment);
        assert_eq!(first, capture(&environment));

        std::fs::write(&config, "model = 'second'\nmodel_provider = 'relay'\n").unwrap();
        assert_eq!(first, capture(&environment));

        std::fs::write(&auth, b"new account").unwrap();
        let second = capture(&environment);
        assert_ne!(first, second);

        environment.insert("OPENAI_API_KEY".into(), "fixture-key".into());
        let third = capture(&environment);
        assert_ne!(second, third);

        std::fs::write(&bin, b"replacement executable").unwrap();
        assert_ne!(third, capture(&environment));
    }

    #[test]
    fn manual_env_wins_over_provider_env() {
        let base = HashMap::from([
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://provider".to_string(),
            ),
            (
                "ANTHROPIC_AUTH_TOKEN".to_string(),
                "sk-provider".to_string(),
            ),
        ]);
        let merged = merge_env(
            base,
            &[
                pair("ANTHROPIC_BASE_URL", "https://manual"),
                pair("EXTRA", "1"),
            ],
        );
        // 手填是「我知道我在干什么」的逃生口，必须压过供应商。
        assert_eq!(merged["ANTHROPIC_BASE_URL"], "https://manual");
        // 供应商里手填没提到的键要保留，否则换个键名就把整份供应商配置抹了。
        assert_eq!(merged["ANTHROPIC_AUTH_TOKEN"], "sk-provider");
        assert_eq!(merged["EXTRA"], "1");
    }
}
