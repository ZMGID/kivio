use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use super::super::{provider_profile, spawn::agent_cli_command, types::RuntimeAgentDef, wsl};

const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Store the last quota reading for a locally identifiable Codex account.
pub(in crate::external_agents) struct PersistedQuota {
    path: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    owner: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Reading {
    owner: String,
    checked_at: i64,
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
    pub(in crate::external_agents) fn path(usage_dir: &Path) -> PathBuf {
        usage_dir.join("codex-quota-v1.json")
    }

    pub(in crate::external_agents) fn for_codex(
        path: PathBuf,
        def: &RuntimeAgentDef,
        binary: &Path,
        cwd: &Path,
    ) -> Option<Self> {
        if def.id != "codex" || wsl::is_wsl_target(binary) {
            return None;
        }
        let command = agent_cli_command(def, binary);
        let command_env = |key: &str| -> Option<Option<OsString>> {
            command.as_std().get_envs().find_map(|(name, value)| {
                let matches = if cfg!(windows) {
                    name.to_string_lossy().eq_ignore_ascii_case(key)
                } else {
                    name == OsStr::new(key)
                };
                matches.then(|| value.map(OsStr::to_os_string))
            })
        };
        let env = |key: &str| command_env(key).unwrap_or_else(|| std::env::var_os(key));
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
                if command_env("HOME").is_some() || command_env("USERPROFILE").is_some() {
                    return None;
                }
                provider_profile::native_codex_home()?
            }
        };
        if !home.is_absolute() {
            return None;
        }
        Self::for_profile(path, fs::canonicalize(home).ok()?, cwd.to_owned())
    }

    fn for_profile(path: PathBuf, home: PathBuf, cwd: PathBuf) -> Option<Self> {
        let owner = account_fingerprint(&home, &cwd)?;
        Some(Self {
            path,
            home,
            cwd,
            owner,
        })
    }

    fn matches_current_profile(&self) -> bool {
        account_fingerprint(&self.home, &self.cwd).as_deref() == Some(self.owner.as_str())
    }

    pub(super) fn restore(&self) -> Option<Value> {
        let reading = self.read()?;
        if reading.owner != self.owner || !self.matches_current_profile() {
            return None;
        }
        Some(json!({
            "agentId": "codex", "account": null, "credits": null, "unavailable": null,
            "checkedAt": reading.checked_at, "windows": reading.windows, "nextRefreshAt": 0,
        }))
    }

    pub(super) fn save(&self, snapshot: &Value) {
        if snapshot["account"]["type"] != "chatgpt" {
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
            windows,
        };
        if let Ok(content) = serde_json::to_string(&reading) {
            let _ = crate::chat::storage::atomic_write(&self.path, &content, "CLI quota cache");
        }
    }

    pub(super) fn invalidate(&self) {
        if self.matches_current_profile()
            && self
                .read()
                .is_some_and(|reading| reading.owner == self.owner)
        {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn read(&self) -> Option<Reading> {
        let reading: Reading = serde_json::from_slice(&read_small(&self.path).ok()?).ok()?;
        valid_reading(reading.checked_at, &reading.windows).then_some(reading)
    }
}

fn valid_reading(checked_at: i64, windows: &[Window]) -> bool {
    checked_at > 0 && checked_at <= chrono::Utc::now().timestamp() && !windows.is_empty()
}

fn read_small(path: &Path) -> io::Result<Vec<u8>> {
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

fn account_fingerprint(home: &Path, cwd: &Path) -> Option<String> {
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
    let bytes =
        serde_json::to_vec(&(home, cwd, &tokens["account_id"], &tokens["id_token"])).ok()?;
    Some(format!("{:x}", Sha256::digest(bytes)))
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
    if managed_codex_preferences() {
        return true;
    }
    ["config.toml", "requirements.toml", "managed_config.toml"]
        .into_iter()
        .any(|name| !file_absent(&system.join(name)))
}

#[cfg(target_os = "macos")]
fn managed_codex_preferences() -> bool {
    use core_foundation::{
        base::TCFType,
        string::{CFString, CFStringRef},
    };

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFPreferencesAppValueIsForced(key: CFStringRef, application_id: CFStringRef) -> u8;
    }

    let application = CFString::new("com.openai.codex");
    ["config_toml_base64", "requirements_toml_base64"]
        .into_iter()
        .any(|key| {
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
        directory: tempfile::TempDir,
        home: PathBuf,
        workspace: PathBuf,
    }

    impl Profile {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let home = directory.path().join("home");
            let workspace = directory.path().join("workspace");
            fs::create_dir(&home).unwrap();
            fs::create_dir(&workspace).unwrap();
            let profile = Self {
                directory,
                home,
                workspace,
            };
            profile.login("profile-user-one", "fixture-token-one");
            profile
        }

        fn login(&self, user: &str, token: &str) {
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
            PersistedQuota::path(self.directory.path())
        }

        fn disk(&self) -> PersistedQuota {
            PersistedQuota::for_profile(self.path(), self.home.clone(), self.workspace.clone())
                .unwrap()
        }

        async fn read(
            &self,
            cache: &AccountSnapshots,
            mode: AccountSnapshotMode,
            probe: impl FnOnce() -> Result<ProbeReading, String>,
        ) -> (Option<Value>, bool) {
            let def = get_agent_def("codex").unwrap();
            let env = [("CODEX_HOME".into(), self.home.clone().into_os_string())].into();
            let identity = LaunchIdentity::from_environment(
                def,
                Path::new("codex-fixture"),
                Some(0),
                None,
                &env,
            );
            cache
                .read(
                    def.id,
                    (identity, self.workspace.clone()),
                    mode,
                    Instant::now(),
                    || Ok(()),
                    || ready(probe()),
                    Some(&self.disk()),
                )
                .await
                .unwrap()
        }
    }

    fn reported() -> ProbeReading {
        ProbeReading {
            snapshot: json!({
                "agentId": "codex", "checkedAt": 100, "unavailable": null,
                "account": {"type": "chatgpt", "email": "user@example.test"},
                "credits": {"balance": "125"},
                "windows": [{"name": "Weekly", "usedPercent": 51, "resetsAt": 1784840060, "windowDurationMins": 10080}],
            }),
            definitively_unavailable: false,
        }
    }

    #[tokio::test]
    async fn reopens_only_quota_readings_and_refreshes_after_restore() {
        let profile = Profile::new();
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
        assert!(restored["account"].is_null());

        let (stale, updated) = profile
            .read(&restarted, AccountSnapshotMode::Automatic, || {
                Err("timeout".into())
            })
            .await;
        assert!(updated);
        let stale = stale.unwrap();
        assert_eq!(stale["windows"], restored["windows"]);
        assert_eq!(stale["refreshError"], "timeout");
        assert_eq!(fs::read_to_string(profile.path()).unwrap(), content);

        profile.read(&restarted, AccountSnapshotMode::Refresh, || Ok(ProbeReading {
            snapshot: json!({"agentId":"codex", "checkedAt":200, "unavailable":"signed out", "windows":[]}),
            definitively_unavailable: true,
        })).await;
        assert!(!profile.path().exists());
    }

    #[tokio::test]
    async fn changed_account_and_corrupt_file_cannot_restore_quota() {
        let profile = Profile::new();
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
        assert!(PersistedQuota::for_profile(
            profile.path(),
            profile.home.clone(),
            profile.workspace.clone()
        )
        .is_none());
    }
}
