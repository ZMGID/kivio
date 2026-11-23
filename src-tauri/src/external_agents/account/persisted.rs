use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use super::super::{
    provider_profile,
    spawn::agent_cli_command,
    types::{AccountProtocol, RuntimeAgentDef},
    wsl,
};
use super::{command_env, explicit_command_env, grok_identity, kimi_binding};

const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Store display-only quota for a locally identifiable CLI account.
pub(in crate::external_agents) struct PersistedQuota {
    path: Option<PathBuf>,
    protocol: AccountProtocol,
    agent: &'static str,
    home: PathBuf,
    cwd: PathBuf,
    owner: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Reading {
    owner: String,
    checked_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    windows: Vec<Window>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    name: String,
    used_percent: f64,
    resets_at: Option<ResetTime>,
    window_duration_mins: Option<i64>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ResetTime {
    Seconds(i64),
    Timestamp(String),
}

impl PersistedQuota {
    pub(in crate::external_agents) fn path(
        usage_dir: &Path,
        def: &RuntimeAgentDef,
    ) -> Option<PathBuf> {
        account_type(def.account?.protocol)?;
        Some(usage_dir.join(format!("{}-quota-v1.json", def.id)))
    }

    pub(in crate::external_agents) fn for_cli(
        path: Option<PathBuf>,
        def: &RuntimeAgentDef,
        binary: &Path,
        cwd: &Path,
    ) -> Option<Self> {
        let protocol = def.account?.protocol;
        account_type(protocol)?;
        if wsl::is_wsl_target(binary) {
            return None;
        }
        let command = agent_cli_command(def, binary);
        let home = match protocol {
            AccountProtocol::CodexAppServer => codex_home(&command)?,
            AccountProtocol::GrokBilling => {
                if command_env(&command, "GROK_AUTH").is_some() {
                    return None;
                }
                grok_identity::profile_home(&command, binary, cwd)?
            }
            AccountProtocol::KimiWeb => kimi_binding::profile_home(&command)?,
            _ => return None,
        };
        Self::for_profile(path, def, home, cwd.to_owned())
    }

    fn for_profile(
        path: Option<PathBuf>,
        def: &RuntimeAgentDef,
        home: PathBuf,
        cwd: PathBuf,
    ) -> Option<Self> {
        let protocol = def.account?.protocol;
        let owner = account_fingerprint(protocol, &home, &cwd)?;
        Some(Self {
            path,
            protocol,
            agent: def.id,
            home,
            cwd,
            owner,
        })
    }

    pub(super) fn owner(&self) -> &str {
        &self.owner
    }

    pub(super) fn matches_current_profile(&self) -> bool {
        account_fingerprint(self.protocol, &self.home, &self.cwd).as_deref() == Some(self.owner())
    }

    pub(super) fn restore(&self) -> Option<Value> {
        let reading = self.read()?;
        if reading.owner != self.owner || !self.matches_current_profile() {
            return None;
        }
        Some(json!({
            "agentId": self.agent,
            "account": reading.plan.map(|plan| json!({"type":account_type(self.protocol),"email":null,"plan":plan})),
            "credits": null, "unavailable": null,
            "checkedAt": reading.checked_at, "windows": reading.windows, "nextRefreshAt": 0,
        }))
    }

    pub(super) fn save(&self, snapshot: &Value) {
        let Some(path) = &self.path else {
            return;
        };
        if snapshot["account"]["type"].as_str() != account_type(self.protocol) {
            return;
        }
        let Some(checked_at) = snapshot["checkedAt"].as_i64() else {
            return;
        };
        let Ok(windows) = serde_json::from_value::<Vec<Window>>(snapshot["windows"].clone()) else {
            return;
        };
        if !valid_reading(checked_at, &windows) || !self.matches_current_profile() {
            return;
        }
        let reading = Reading {
            owner: self.owner.clone(),
            checked_at,
            plan: snapshot["account"]["plan"].as_str().map(str::to_owned),
            windows,
        };
        if let Ok(content) = serde_json::to_string(&reading) {
            let _ = crate::chat::storage::atomic_write(path, &content, "CLI quota cache");
        }
    }

    pub(super) fn invalidate(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if self.matches_current_profile()
            && self
                .read()
                .is_some_and(|reading| reading.owner == self.owner)
        {
            let _ = fs::remove_file(path);
        }
    }

    fn read(&self) -> Option<Reading> {
        let reading: Reading =
            serde_json::from_slice(&read_small(self.path.as_ref()?).ok()?).ok()?;
        valid_reading(reading.checked_at, &reading.windows).then_some(reading)
    }
}

fn account_type(protocol: AccountProtocol) -> Option<&'static str> {
    match protocol {
        AccountProtocol::CodexAppServer => Some("chatgpt"),
        AccountProtocol::GrokBilling => Some("grok"),
        AccountProtocol::KimiWeb => Some("kimiCode"),
        _ => None,
    }
}

fn valid_reading(checked_at: i64, windows: &[Window]) -> bool {
    checked_at > 0 && checked_at <= chrono::Utc::now().timestamp() && !windows.is_empty()
}

pub(super) fn read_small(path: &Path) -> io::Result<Vec<u8>> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quota cache input is not a file",
        ));
    }
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quota cache input too large",
        ));
    }
    Ok(bytes)
}

pub(super) fn fingerprint(input: &impl Serialize) -> Option<String> {
    Some(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(input).ok()?)
    ))
}

fn account_fingerprint(protocol: AccountProtocol, home: &Path, cwd: &Path) -> Option<String> {
    match protocol {
        AccountProtocol::CodexAppServer => codex_fingerprint(home, cwd),
        AccountProtocol::GrokBilling => grok_identity::quota_owner(&fs::canonicalize(home).ok()?),
        AccountProtocol::KimiWeb => kimi_binding::quota_owner(&fs::canonicalize(home).ok()?),
        _ => None,
    }
}

fn codex_home(command: &tokio::process::Command) -> Option<PathBuf> {
    let env = |key| explicit_command_env(command, key).unwrap_or_else(|| std::env::var_os(key));
    if [
        "CODEX_ACCESS_TOKEN",
        "CODEX_API_KEY",
        "OPENAI_API_KEY",
        "OPENAI_IDENTITY_TOKEN_FILE",
    ]
    .into_iter()
    .any(|key| env(key).is_some_and(|value| !value.is_empty()))
    {
        return None;
    }
    let home = match env("CODEX_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => {
            if explicit_command_env(command, "HOME").is_some()
                || explicit_command_env(command, "USERPROFILE").is_some()
            {
                return None;
            }
            provider_profile::native_codex_home()?
        }
    };
    if !home.is_absolute() {
        return None;
    }
    fs::canonicalize(home).ok()
}

fn codex_fingerprint(home: &Path, cwd: &Path) -> Option<String> {
    if managed_codex_configuration() {
        return None;
    }
    let config_path = home.join("config.toml");
    let config = match read_small(&config_path) {
        Ok(bytes) => toml::from_str::<toml::Table>(std::str::from_utf8(&bytes).ok()?).ok()?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(_) => return None,
    };
    if config
        .get("cli_auth_credentials_store")
        .is_some_and(|mode| mode.as_str() != Some("file"))
        || config.get("chatgpt_base_url").is_some()
        || config.get("profile").is_some()
        || config
            .get("forced_login_method")
            .is_some_and(|mode| mode.as_str() != Some("chatgpt"))
        || cwd.ancestors().any(|parent| {
            let project_config = parent.join(".codex/config.toml");
            project_config != config_path && !file_absent(&project_config)
        })
        || (cwd != home && !file_absent(&cwd.join("config.toml")))
    {
        return None;
    }
    // Codex's file-based ChatGPT login provides stable user/workspace claims across token rotations.
    let auth = provider_profile::codex_launch_auth(&read_small(&home.join("auth.json")).ok()?)?;
    if !auth["OPENAI_API_KEY"].is_null() {
        return None;
    }
    let tokens = &auth["tokens"];
    fingerprint(&(home, cwd, &tokens["account_id"], &tokens["id_token"]))
}

fn file_absent(path: &Path) -> bool {
    fs::symlink_metadata(path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
}

fn managed_codex_configuration() -> bool {
    #[cfg(not(windows))]
    let system = PathBuf::from("/etc/codex");
    #[cfg(windows)]
    let system = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("OpenAI/Codex");
    #[cfg(target_os = "macos")]
    if managed_preferences(
        "com.openai.codex",
        &["config_toml_base64", "requirements_toml_base64"],
    ) {
        return true;
    }
    ["config.toml", "requirements.toml", "managed_config.toml"]
        .into_iter()
        .any(|name| !file_absent(&system.join(name)))
}

#[cfg(target_os = "macos")]
pub(super) fn managed_preferences(application: &str, keys: &[&str]) -> bool {
    use core_foundation::{
        base::TCFType,
        string::{CFString, CFStringRef},
    };

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFPreferencesAppValueIsForced(key: CFStringRef, application_id: CFStringRef) -> u8;
    }

    let application = CFString::new(application);
    keys.iter().any(|key| {
        let key = CFString::new(key);
        // Both references belong to CFString values kept alive through the call.
        unsafe {
            CFPreferencesAppValueIsForced(
                key.as_concrete_TypeRef(),
                application.as_concrete_TypeRef(),
            ) != 0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external_agents::{overrides::LaunchIdentity, registry::get_agent_def};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::{future::ready, time::Instant};

    use super::super::{AccountSnapshotMode, AccountSnapshots, ProbeReading};

    struct Profile {
        def: &'static RuntimeAgentDef,
        directory: tempfile::TempDir,
        home: PathBuf,
        workspace: PathBuf,
    }

    impl Profile {
        fn new(agent: &str) -> Self {
            let directory = tempfile::tempdir().unwrap();
            let home = directory.path().join("home");
            let workspace = directory.path().join("workspace");
            fs::create_dir(&home).unwrap();
            fs::create_dir(&workspace).unwrap();
            let profile = Self {
                def: get_agent_def(agent).unwrap(),
                directory,
                home,
                workspace,
            };
            profile.login("profile-user-one", "fixture-token-one");
            profile
        }

        fn login(&self, user: &str, token: &str) {
            if self.def.id == "grok" {
                let auth = json!({
                    "https://auth.x.ai::b1a00492-073a-47ea-816f-4c329264a828":{
                        "auth_mode":"oidc", "email":"user@example.test", "key":token,
                        "user_id":user, "principal_type":"User", "principal_id":user,
                        "team_id":"workspace-fixture"
                    }
                });
                fs::write(self.home.join("auth.json"), auth.to_string()).unwrap();
                return;
            }
            if self.def.id == "kimi" {
                let auth = json!({"access_token":token, "refresh_token":format!("{user}-{token}")});
                fs::write(
                    self.home.join("config.toml"),
                    r#"[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
[providers."managed:kimi-code".oauth]
storage = "file"
key = "oauth/kimi-code"
"#,
                )
                .unwrap();
                fs::create_dir_all(self.home.join("credentials")).unwrap();
                fs::write(
                    self.home.join("credentials/kimi-code.json"),
                    auth.to_string(),
                )
                .unwrap();
                return;
            }
            let claims = URL_SAFE_NO_PAD.encode(
                serde_json::to_vec(&json!({
                    "sub": user, "https://api.openai.com/auth": {
                        "chatgpt_user_id": user, "chatgpt_account_id": "workspace-fixture",
                    },
                }))
                .unwrap(),
            );
            let auth = json!({ "auth_mode": "chatgpt", "tokens": {
                "id_token": format!("header.{claims}.signature"), "account_id": "workspace-fixture",
                "access_token": token, "refresh_token": token,
            }});
            fs::write(self.home.join("auth.json"), auth.to_string()).unwrap();
        }

        fn path(&self) -> PathBuf {
            PersistedQuota::path(self.directory.path(), self.def).unwrap()
        }

        fn disk(&self) -> Option<PersistedQuota> {
            PersistedQuota::for_profile(
                Some(self.path()),
                self.def,
                self.home.clone(),
                self.workspace.clone(),
            )
        }

        async fn read(
            &self,
            cache: &AccountSnapshots,
            mode: AccountSnapshotMode,
            probe: impl FnOnce() -> Result<ProbeReading, String>,
        ) -> (Option<Value>, bool) {
            let home_key = match self.def.id {
                "grok" => "GROK_HOME",
                "kimi" => "KIMI_CODE_HOME",
                _ => "CODEX_HOME",
            };
            let env = [(home_key.into(), self.home.clone().into_os_string())].into();
            let identity = LaunchIdentity::from_environment(
                self.def,
                Path::new("cli-fixture"),
                Some(0),
                None,
                &env,
            );
            cache
                .read(
                    self.def.id,
                    (identity, self.workspace.clone()),
                    mode,
                    Instant::now(),
                    || Ok(()),
                    || ready(probe()),
                    self.disk().as_ref(),
                )
                .await
                .unwrap()
        }
    }

    fn reported() -> ProbeReading {
        ProbeReading {
            snapshot: json!({
                "agentId": "codex", "checkedAt": 100, "unavailable": null,
                "account": {"type": "chatgpt", "email": "user@example.test", "plan": "Pro 20×"},
                "credits": {"balance": "125"},
                "windows": [{"name": "Weekly", "usedPercent": 51, "resetsAt": 1784840060, "windowDurationMins": 10080}],
            }),
            definitively_unavailable: false,
        }
    }

    #[tokio::test]
    async fn reopens_quota_and_plan_before_refresh() {
        let profile = Profile::new("codex");
        let first = AccountSnapshots::default();
        assert!(
            profile
                .read(&first, AccountSnapshotMode::Automatic, || Ok(reported()))
                .await
                .1
        );
        let content = fs::read_to_string(profile.path()).unwrap();
        for private in [
            "profile-user-one",
            "workspace-fixture",
            "fixture-token-one",
            "user@example.test",
            "balance",
        ] {
            assert!(!content.contains(private));
        }

        profile.login("profile-user-one", "rotated-fixture-token");
        let restarted = AccountSnapshots::default();
        let (restored, updated) = profile
            .read(&restarted, AccountSnapshotMode::Cached, || {
                panic!("quota cache read must not probe")
            })
            .await;
        assert!(!updated);
        let restored = restored.unwrap();
        assert_eq!(restored["windows"][0]["usedPercent"], 51.0);
        assert_eq!(restored["checkedAt"], 100);
        assert_eq!(restored["nextRefreshAt"], 0);
        assert_eq!(restored["account"]["plan"], "Pro 20×");

        let (stale, updated) = profile
            .read(&restarted, AccountSnapshotMode::Automatic, || {
                Err("timeout".into())
            })
            .await;
        assert!(updated);
        let stale = stale.unwrap();
        assert_eq!(stale["windows"], restored["windows"]);
        assert_eq!(stale["account"]["plan"], "Pro 20×");
        assert_eq!(stale["refreshError"], "timeout");
        assert_eq!(fs::read_to_string(profile.path()).unwrap(), content);

        let mut legacy: Value = serde_json::from_str(&content).unwrap();
        legacy.as_object_mut().unwrap().remove("plan");
        fs::write(profile.path(), legacy.to_string()).unwrap();
        let legacy = profile.disk().unwrap().restore().unwrap();
        assert_eq!(legacy["windows"], restored["windows"]);
        assert!(legacy["account"].is_null());

        profile.read(&restarted, AccountSnapshotMode::Refresh, || Ok(ProbeReading {
            snapshot: json!({"agentId":"codex", "checkedAt":200, "unavailable":"signed out", "windows":[]}),
            definitively_unavailable: true,
        })).await;
        assert!(!profile.path().exists());
    }

    #[tokio::test]
    async fn changed_account_and_corrupt_file_cannot_restore_quota() {
        let profile = Profile::new("codex");
        profile
            .read(
                &AccountSnapshots::default(),
                AccountSnapshotMode::Automatic,
                || Ok(reported()),
            )
            .await;
        profile.login("different-user", "fixture-token-two");
        assert_eq!(
            profile
                .read(
                    &AccountSnapshots::default(),
                    AccountSnapshotMode::Cached,
                    || panic!("no quota probe")
                )
                .await,
            (None, false)
        );

        fs::write(profile.path(), b"partial JSON").unwrap();
        assert_eq!(
            profile
                .read(
                    &AccountSnapshots::default(),
                    AccountSnapshotMode::Cached,
                    || panic!("no quota probe")
                )
                .await,
            (None, false)
        );
        assert!(
            profile
                .read(
                    &AccountSnapshots::default(),
                    AccountSnapshotMode::Automatic,
                    || Ok(reported())
                )
                .await
                .1
        );

        fs::write(
            profile.home.join("config.toml"),
            "cli_auth_credentials_store = 'auto'",
        )
        .unwrap();
        assert!(profile.disk().is_none());
    }

    #[tokio::test]
    async fn grok_and_kimi_reopen_the_owned_reading_and_hide_it_after_a_login_change() {
        for (agent, kind, plan, credential) in [
            ("grok", "grok", "SuperGrok", "auth.json"),
            ("kimi", "kimiCode", "Andante", "credentials/kimi-code.json"),
        ] {
            let profile = Profile::new(agent);
            let quota = || {
                let mut reading = reported();
                reading.snapshot["agentId"] = json!(agent);
                reading.snapshot["account"]["type"] = json!(kind);
                reading.snapshot["account"]["plan"] = json!(plan);
                reading
            };
            let first = AccountSnapshots::default();
            assert!(
                profile
                    .read(&first, AccountSnapshotMode::Automatic, || Ok(quota()))
                    .await
                    .1
            );
            let content = fs::read_to_string(profile.path()).unwrap();
            for private in [
                "profile-user-one",
                "workspace-fixture",
                "fixture-token-one",
                "user@example.test",
                "balance",
            ] {
                assert!(!content.contains(private));
            }
            if agent == "grok" {
                profile.login("profile-user-one", "rotated");
            }
            let restarted = AccountSnapshots::default();
            let (restored, updated) = profile
                .read(&restarted, AccountSnapshotMode::Cached, || {
                    panic!("cached read must not launch a quota probe")
                })
                .await;
            assert!(!updated);
            let restored = restored.unwrap();
            assert_eq!(restored["agentId"], agent);
            assert_eq!(restored["account"]["plan"], plan);
            assert_eq!(restored["windows"][0]["usedPercent"], 51.0);
            let (stale, _) = profile
                .read(
                    &AccountSnapshots::default(),
                    AccountSnapshotMode::Automatic,
                    || Err("temporary error".into()),
                )
                .await;
            let stale = stale.unwrap();
            assert_eq!(stale["refreshError"], "temporary error");
            assert_eq!(stale["windows"], restored["windows"]);

            let raced = profile
                .read(&restarted, AccountSnapshotMode::Refresh, || {
                    profile.login("different-user", "switched-token");
                    Ok(quota())
                })
                .await;
            assert_eq!(raced, (None, false));
            assert_eq!(fs::read_to_string(profile.path()).unwrap(), content);
            assert_eq!(
                profile
                    .read(&restarted, AccountSnapshotMode::Cached, || panic!(
                        "no probe"
                    ))
                    .await,
                (None, false)
            );

            profile
                .read(&restarted, AccountSnapshotMode::Refresh, || Ok(quota()))
                .await;
            fs::remove_file(profile.home.join(credential)).unwrap();
            assert_eq!(
                profile
                    .read(&restarted, AccountSnapshotMode::Cached, || panic!(
                        "no probe"
                    ))
                    .await,
                (None, false)
            );
        }
    }
}
