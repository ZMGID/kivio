use super::{
    antigravity_manifest_url, environment_value, extract_semver, find_npm, npm_install_plan,
    proven_npm_prefix, resource_path, verify_cli_version_in, BTreeMap, CommandPlan, Duration,
    InstallAction, OsString, Path, PathBuf, PreparedInstall, INSTALL_TIMEOUT_SECS,
};
use crate::external_agents::{
    spawn::{self, CliProcessOutput},
    types::{RuntimeAgentDef, UpdateStrategy},
    wsl,
};
use sha2::{Digest, Sha512};
use tokio::io::AsyncWriteExt;

type Environment = BTreeMap<OsString, OsString>;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ManagedSource {
    Antigravity,
    Claude,
    GrokNpm {
        prefix: PathBuf,
        home: PathBuf,
    },
    Homebrew {
        prefix: PathBuf,
        kind: &'static str,
        package: String,
        installed: String,
    },
}

pub(super) struct UpdateOffer {
    pub version: String,
    pub action: InstallAction,
}

impl UpdateOffer {
    fn command(version: String, command: CommandPlan) -> Self {
        Self {
            version,
            action: InstallAction::Command(command),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct AntigravityAsset {
    pub url: reqwest::Url,
    sha512: String,
}

impl AntigravityAsset {
    fn from_manifest(manifest: &serde_json::Value) -> Option<UpdateOffer> {
        let version = normalized_version(manifest["version"].as_str()?)?;
        let url = reqwest::Url::parse(manifest["url"].as_str()?).ok()?;
        let hash = manifest["sha512"].as_str()?;
        if !url
            .as_str()
            .starts_with("https://storage.googleapis.com/antigravity-public/antigravity-cli/")
            || !url.path().ends_with(".exe")
            || hash.len() != 128
            || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return None;
        }
        Some(UpdateOffer {
            version,
            action: InstallAction::Antigravity(Self {
                url,
                sha512: hash.to_ascii_lowercase(),
            }),
        })
    }

    pub(super) async fn install(
        &self,
        http: &reqwest::Client,
        def: &RuntimeAgentDef,
        plan: &PreparedInstall,
        emit: &impl Fn(String),
    ) -> Result<(), String> {
        const MAX_BYTES: usize = 512 * 1024 * 1024;
        let parent = plan.binary.parent().ok_or("Antigravity CLI 安装路径无效")?;
        emit(format!("正在下载 Antigravity CLI {}", plan.version));
        let mut response = http
            .get(self.url.clone())
            .timeout(Duration::from_secs(INSTALL_TIMEOUT_SECS))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| format!("下载 Antigravity CLI 失败：{e}"))?;
        let staged = tempfile::Builder::new()
            .suffix(".exe")
            .tempfile_in(parent)
            .map_err(|e| format!("无法在 Antigravity CLI 安装目录暂存更新：{e}"))?;
        let (file, staged) = staged.into_parts();
        let mut file = tokio::fs::File::from_std(file);
        let mut hasher = Sha512::new();
        let mut bytes = 0;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            bytes += chunk.len();
            if bytes > MAX_BYTES {
                return Err("Antigravity CLI 下载超过大小限制".into());
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        }
        if format!("{:x}", hasher.finalize()) != self.sha512 {
            return Err("Antigravity CLI 下载未通过 SHA-512 校验".into());
        }
        file.sync_all().await.map_err(|e| e.to_string())?;
        drop(file);
        let version = verify_cli_version_in(
            def,
            &staged,
            Duration::from_secs(20),
            Some(&plan.target_environment),
        )
        .await?;
        if version != plan.version || !plan.identity.still_current(def, &plan.binary) {
            return Err("Antigravity CLI 的版本或安装路径已变更，请刷新安装计划".into());
        }
        staged
            .persist(&plan.binary)
            .map_err(|e| format!("无法替换 Antigravity CLI：{e}"))?;
        Ok(())
    }
}

impl ManagedSource {
    pub fn resolve(
        def: &RuntimeAgentDef,
        binary: &Path,
        environment: &Environment,
    ) -> Option<Self> {
        if wsl::is_wsl_target(binary) {
            return None;
        }
        if cfg!(windows) {
            if def.id != "antigravity" {
                return None;
            }
            let default = PathBuf::from(environment_value(environment, "LOCALAPPDATA")?)
                .join("agy/bin/agy.exe");
            return (binary.file_name()?.eq_ignore_ascii_case("agy.exe")
                && binary.symlink_metadata().ok()?.file_type().is_file()
                && resource_path(binary.parent()?) == resource_path(default.parent()?))
            .then_some(Self::Antigravity);
        }
        let real = std::fs::canonicalize(binary).ok()?;
        for directory in real.ancestors().skip(3) {
            let kind = match directory.file_name().and_then(|name| name.to_str()) {
                Some("Cellar") => "--formula",
                Some("Caskroom") => "--cask",
                _ => continue,
            };
            let prefix = directory.parent()?;
            let mut parts = real.strip_prefix(directory).ok()?.iter();
            let package = parts.next()?.to_str()?;
            let installed = parts.next()?.to_str()?.to_owned();
            let names = || std::iter::once(def.bin).chain(def.fallback_bins.iter().copied());
            let provider_package = match def.install.update {
                UpdateStrategy::ManagedPackage { brew_formula, .. }
                | UpdateStrategy::KimiManaged { brew_formula, .. } => package == brew_formula,
                UpdateStrategy::NpmPackageMigration {
                    brew_formula,
                    previous_brew_formula,
                    ..
                } => package == brew_formula || package == previous_brew_formula,
                _ => match def.id {
                    "claude" => matches!(package, "claude-code" | "claude-code@latest"),
                    "grok" => package == "grok" && kind == "--cask",
                    _ => names().any(|name| name == package),
                },
            };
            if provider_package
                && names().any(|name| follows_launcher(binary, &prefix.join("bin").join(name)))
            {
                return Some(Self::Homebrew {
                    prefix: prefix.to_owned(),
                    kind,
                    package: package.to_owned(),
                    installed,
                });
            }
            return None;
        }

        if def.id == "claude" {
            let home = PathBuf::from(environment_value(environment, "HOME")?);
            let data = environment_value(environment, "XDG_DATA_HOME")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"));
            let versions = std::fs::canonicalize(data.join("claude/versions")).ok()?;
            if real.parent() == Some(versions.as_path())
                && follows_launcher(binary, &home.join(".local/bin/claude"))
            {
                return Some(Self::Claude);
            }
        }
        if def.id == "grok" {
            let home = environment_value(environment, "GROK_HOME")
                .filter(|home| !home.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    Some(PathBuf::from(environment_value(environment, "HOME")?).join(".grok"))
                })?;
            if !home.is_absolute() {
                return None;
            }
            let bin = std::fs::canonicalize(home.join("bin")).ok()?;
            let version = real.file_name()?.to_str()?.strip_prefix("grok-")?;
            if real.parent() != Some(bin.as_path())
                || normalized_version(version).is_none()
                || !follows_launcher(binary, &bin.join("grok"))
            {
                return None;
            }
            let npm = find_npm(environment)?;
            let prefix = proven_npm_prefix(&npm.with_file_name("grok"), "@xai-official/grok")?;
            return Some(Self::GrokNpm { prefix, home });
        }
        None
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Antigravity => "Antigravity CLI 原生安装",
            Self::Claude => "Claude Code 原生安装",
            Self::GrokNpm { .. } => "Grok npm",
            Self::Homebrew { .. } => "Homebrew",
        }
    }

    pub fn lock_key(&self) -> Option<String> {
        match self {
            Self::Homebrew { prefix, .. } => Some(format!("brew:{}", resource_path(prefix))),
            Self::GrokNpm { prefix, .. } => Some(format!("prefix:{}", resource_path(prefix))),
            Self::Antigravity | Self::Claude => None,
        }
    }

    pub async fn offer(
        &self,
        http: &reqwest::Client,
        binary: &Path,
        environment: &Environment,
    ) -> Result<UpdateOffer, String> {
        let mut captured = environment.clone();
        self.configure_environment(&mut captured);
        let environment = &captured;
        match self {
            Self::Antigravity => {
                let url =
                    antigravity_manifest_url().ok_or("此平台不支持 Antigravity CLI 自动更新")?;
                let manifest = http
                    .get(url)
                    .timeout(Duration::from_secs(8))
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                    .map_err(|_| "查询 Antigravity CLI 发布版本失败")?
                    .json()
                    .await
                    .map_err(|_| "读取 Antigravity CLI 发布清单失败")?;
                AntigravityAsset::from_manifest(&manifest)
                    .ok_or_else(|| "Antigravity CLI 发布清单缺少可校验的 Windows 安装包".into())
            }
            Self::Claude => {
                let doctor = probe(binary, &["doctor"], environment)
                    .await
                    .ok_or("无法读取 Claude Code 的原生更新渠道")?;
                if !doctor
                    .lines()
                    .any(|line| line.trim().starts_with("Running: native ("))
                {
                    return Err("Claude Code 未报告原生安装，请用原安装方式更新".into());
                }
                let channel = doctor
                    .lines()
                    .find_map(
                        |line| match line.trim().strip_prefix("Auto-update channel: ")? {
                            "latest" => Some("latest"),
                            "stable" => Some("stable"),
                            _ => None,
                        },
                    )
                    .ok_or("无法读取 Claude Code 的原生更新渠道")?;
                let release = http
                    .get(format!(
                        "https://downloads.claude.ai/claude-code-releases/{channel}"
                    ))
                    .timeout(Duration::from_secs(8))
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                    .map_err(|_| "查询 Claude Code 发布版本失败")?
                    .text()
                    .await
                    .map_err(|_| "读取 Claude Code 发布版本失败")?;
                let version =
                    normalized_version(release.trim()).ok_or("无效的 Claude Code 发布版本")?;
                let mut command =
                    CommandPlan::direct(binary.to_string_lossy(), &["install", &version]);
                command.current_dir = Some(std::env::temp_dir());
                Ok(UpdateOffer::command(version, command))
            }
            Self::GrokNpm { prefix, .. } => {
                let npm = find_npm(environment).ok_or("未找到管理此安装的 npm")?;
                let reported = probe(&npm, &["prefix", "-g"], environment)
                    .await
                    .ok_or("无法读取 Grok 的 npm 安装目录")?;
                if resource_path(Path::new(reported.trim())) != resource_path(prefix) {
                    return Err("npm 当前使用的目录与 Grok 安装目录不符".into());
                }
                let status = probe(binary, &["update", "--check", "--json"], environment)
                    .await
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                    .ok_or("无法读取 Grok 的当前更新渠道")?;
                if status["installer"].as_str() != Some("npm") || !status["error"].is_null() {
                    return Err("Grok 未确认由 npm 管理，请用原安装方式更新".into());
                }
                let version = status["latestVersion"]
                    .as_str()
                    .and_then(normalized_version)
                    .ok_or("Grok 未提供可比较的版本号")?;
                let command = npm_install_plan(&npm, prefix, "@xai-official/grok", &version, &[]);
                Ok(UpdateOffer::command(version, command))
            }
            Self::Homebrew {
                prefix,
                kind,
                package,
                installed,
            } => {
                let brew = prefix.join("bin/brew");
                let reported = probe(&brew, &["--prefix"], environment)
                    .await
                    .ok_or("未找到管理此安装的 Homebrew")?;
                if std::fs::canonicalize(reported.trim()).as_ref().ok() != Some(prefix) {
                    return Err("Homebrew 的安装目录与 CLI 路径不符".into());
                }
                let info = probe(&brew, &["info", "--json=v2", kind, package], environment)
                    .await
                    .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                    .ok_or("读取 Homebrew 版本失败")?;
                let (item, name, version, linked) = if *kind == "--formula" {
                    let item = &info["formulae"][0];
                    (
                        item,
                        &item["full_name"],
                        item["versions"]["stable"].as_str(),
                        item["linked_keg"].as_str(),
                    )
                } else {
                    let item = &info["casks"][0];
                    (
                        item,
                        &item["full_token"],
                        item["version"].as_str(),
                        item["installed"].as_str(),
                    )
                };
                if linked != Some(installed.as_str())
                    || item["disabled"].as_bool() == Some(true)
                    || item["pinned"].as_bool() == Some(true)
                {
                    return Err("Homebrew 未链接、禁用或固定了此安装，请在 Homebrew 中检查".into());
                }
                let package_ref = name
                    .as_str()
                    .filter(|name| {
                        name.rsplit('/').next() == Some(package.as_str())
                            && !name.starts_with('-')
                            && name
                                .bytes()
                                .all(|c| c.is_ascii_alphanumeric() || b"-_./@+".contains(&c))
                    })
                    .ok_or("Homebrew 软件包与 CLI 路径不符")?;
                let version = version
                    .and_then(|version| version.split(',').next())
                    .and_then(normalized_version)
                    .ok_or("Homebrew 未提供可比较的版本号")?;
                Ok(UpdateOffer::command(
                    version,
                    CommandPlan::direct(brew.to_string_lossy(), &["upgrade", kind, package_ref]),
                ))
            }
        }
    }

    pub fn configure_environment(&self, environment: &mut Environment) {
        match self {
            Self::Antigravity | Self::Claude => {}
            Self::Homebrew { .. } => {
                environment.insert("HOMEBREW_NO_AUTO_UPDATE".into(), "1".into());
            }
            Self::GrokNpm { home, .. } => {
                environment.insert("GROK_HOME".into(), home.as_os_str().to_owned());
                environment.remove(std::ffi::OsStr::new("npm_config_user_agent"));
                environment.insert(
                    "npm_config_allow_scripts".into(),
                    "@xai-official/grok".into(),
                );
            }
        }
    }
}

fn normalized_version(raw: &str) -> Option<String> {
    let version = extract_semver(raw)?;
    (raw.trim_start_matches('v') == version).then_some(version)
}

async fn probe(program: &Path, args: &[&str], environment: &Environment) -> Option<String> {
    let mut command = spawn::cli_probe_command(program);
    command
        .env_clear()
        .envs(environment)
        .args(args)
        .current_dir(std::env::temp_dir());
    let output = tokio::time::timeout(Duration::from_secs(10), command.group_output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn follows_launcher(binary: &Path, launcher: &Path) -> bool {
    fn normalize_parent(path: &Path) -> Option<PathBuf> {
        Some(
            std::fs::canonicalize(path.parent()?)
                .ok()?
                .join(path.file_name()?),
        )
    }
    let Some(launcher) = normalize_parent(launcher) else {
        return false;
    };
    let mut candidate = binary.to_path_buf();
    for _ in 0..16 {
        let Some(path) = normalize_parent(&candidate) else {
            return false;
        };
        if path == launcher {
            return true;
        }
        let Ok(link) = std::fs::read_link(&path) else {
            return false;
        };
        candidate = path.with_file_name(link);
    }
    false
}
