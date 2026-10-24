// Shared installation plans, bounded process execution and version queries.
use crate::external_agents::spawn::CliProcessOutput;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tokio::io::AsyncReadExt;

use crate::external_agents::types::{
    AgentInstallSpec, LatestVersionStrategy, PostInstallStrategy, UpdateStrategy,
};
use crate::proc::NoConsoleWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostPlatform {
    Unix,
    Windows,
}

fn host_platform() -> HostPlatform {
    if cfg!(windows) {
        HostPlatform::Windows
    } else {
        HostPlatform::Unix
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandPlan {
    program: String,
    args: Vec<String>,
    display: String,
    current_dir: Option<PathBuf>,
}

impl CommandPlan {
    fn direct(program: impl Into<String>, args: &[&str]) -> Self {
        Self::owned(program, args.iter().map(|arg| (*arg).to_string()).collect())
    }

    fn owned(program: impl Into<String>, args: Vec<String>) -> Self {
        let program = program.into();
        let display = std::iter::once(program.as_str())
            .chain(args.iter().map(String::as_str))
            .map(display_token)
            .collect::<Vec<_>>()
            .join(" ");
        Self {
            program,
            args,
            display,
            current_dir: None,
        }
    }

    fn unix_script(script: &str) -> Self {
        Self {
            program: "bash".to_string(),
            args: vec!["-lc".to_string(), format!("set -o pipefail; {script}")],
            display: script.to_string(),
            current_dir: None,
        }
    }

    fn powershell(script: &str) -> Self {
        Self {
            program: "powershell.exe".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                script.to_string(),
            ],
            display: script.to_string(),
            current_dir: None,
        }
    }
}

fn display_token(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

const NODE_DOWNLOAD_URL: &str = "https://nodejs.org";
const NPMJS_REGISTRY: &str = "https://registry.npmjs.org/";
const NPMJS_REGISTRY_FLAG: &str = "--registry=https://registry.npmjs.org/";
const DEEPSEEK_SCOPE_REGISTRY_FLAG: &str = "--@deepseek-ai:registry=https://registry.npmjs.org/";

fn uses_npm(plan: &CommandPlan) -> bool {
    plan.program == "npm" || plan.program == "npm.cmd"
}

fn uses_js_package_manager(plan: &CommandPlan) -> bool {
    matches!(
        plan.program.as_str(),
        "npm" | "npm.cmd" | "pnpm" | "pnpm.cmd" | "yarn" | "yarn.cmd" | "bun" | "bun.exe"
    )
}

/// 覆盖用户 `.npmrc` / `npm_config_registry` 里的淘宝等镜像。CLI `--registry` 盖不住
/// 作用域配置（`@deepseek-ai:registry=`），所以安装 `@deepseek-ai/*` 时另外带一条。
pub(crate) fn pin_official_npm_registry(command: &mut tokio::process::Command) {
    command.env("npm_config_registry", NPMJS_REGISTRY);
    command.env("NPM_CONFIG_REGISTRY", NPMJS_REGISTRY);
}

fn needs_deepseek_scope_registry(package: &str, extra_args: &[&str]) -> bool {
    package.starts_with("@deepseek-ai/")
        || extra_args
            .iter()
            .any(|arg| arg.starts_with("@deepseek-ai/"))
}

/// `node --version` 输出 → (major, minor, patch)。认 `v24.13.0` / `24.13.0`。
fn npm_install_plan(package: &str, extra_args: &[&str], platform: HostPlatform) -> CommandPlan {
    let program = if platform == HostPlatform::Windows {
        "npm.cmd"
    } else {
        "npm"
    };
    // `--progress=false`：npm 默认进度条用 `\r` 刷同一行。按行读会看起来像卡住，
    // 管道缓冲区填满后安装进程还会死锁。关掉进度条，改走普通日志行。
    let mut args = vec!["install", "-g", "--progress=false", NPMJS_REGISTRY_FLAG];
    if needs_deepseek_scope_registry(package, extra_args) {
        args.push(DEEPSEEK_SCOPE_REGISTRY_FLAG);
    }
    args.extend_from_slice(extra_args);
    let package = format!("{package}@latest");
    args.push(&package);
    CommandPlan::direct(program, &args)
}

fn js_global_add_plan(
    program: &str,
    prefix: &[&str],
    package: &str,
    extra_args: &[&str],
) -> CommandPlan {
    let latest = format!("{package}@latest");
    let mut args = prefix.to_vec();
    args.push(NPMJS_REGISTRY_FLAG);
    if needs_deepseek_scope_registry(package, extra_args) {
        args.push(DEEPSEEK_SCOPE_REGISTRY_FLAG);
    }
    args.extend_from_slice(extra_args);
    args.push(&latest);
    CommandPlan::direct(program, &args)
}

fn install_plan(spec: &AgentInstallSpec, platform: HostPlatform) -> Option<CommandPlan> {
    match platform {
        HostPlatform::Unix => spec.script_unix.map(CommandPlan::unix_script),
        HostPlatform::Windows => spec.script_windows.map(CommandPlan::powershell),
    }
    .or_else(|| {
        spec.npm_package
            .map(|package| npm_install_plan(package, spec.npm_install_args, platform))
    })
}

fn normalized_resolved_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn managed_package_update_plan(
    path: &Path,
    package: &str,
    brew_formula: &str,
    platform: HostPlatform,
    extra_args: &[&str],
) -> Option<CommandPlan> {
    let normalized = normalized_resolved_path(path);
    if normalized.contains(&format!("/cellar/{brew_formula}/")) {
        return Some(CommandPlan::direct("brew", &["upgrade", brew_formula]));
    }
    if normalized.contains("/.pnpm/") || normalized.contains("/pnpm/global/") {
        return Some(js_global_add_plan(
            "pnpm",
            &["add", "-g"],
            package,
            extra_args,
        ));
    }
    if normalized.contains("/.config/yarn/global/") || normalized.contains("/.yarn/global/") {
        return Some(js_global_add_plan(
            "yarn",
            &["global", "add"],
            package,
            extra_args,
        ));
    }
    if normalized.contains("/.bun/install/global/") {
        return Some(js_global_add_plan(
            "bun",
            &["add", "-g"],
            package,
            extra_args,
        ));
    }
    if normalized.contains("/node_modules/")
        || (platform == HostPlatform::Windows
            && matches!(path.extension().and_then(|ext| ext.to_str()), Some("cmd")))
    {
        return Some(npm_install_plan(package, extra_args, platform));
    }
    None
}

/// 已安装时优先让当前二进制自更新；Kimi/Gemini 没有可用的无交互入口，按安装来源处理。
fn update_plan(
    spec: &AgentInstallSpec,
    resolved_path: &Path,
    platform: HostPlatform,
) -> Option<CommandPlan> {
    match spec.update {
        UpdateStrategy::None => None,
        UpdateStrategy::Command(args) => Some(CommandPlan::direct(
            resolved_path.to_string_lossy().into_owned(),
            args,
        )),
        UpdateStrategy::ManagedPackage {
            package,
            brew_formula,
        } => managed_package_update_plan(
            resolved_path,
            package,
            brew_formula,
            platform,
            spec.npm_install_args,
        ),
        UpdateStrategy::KimiManaged {
            package,
            brew_formula,
        } => {
            let normalized = normalized_resolved_path(resolved_path);
            if normalized.contains("/.kimi-code/bin/kimi") {
                install_plan(spec, platform)
            } else {
                managed_package_update_plan(
                    resolved_path,
                    package,
                    brew_formula,
                    platform,
                    spec.npm_install_args,
                )
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallInfo {
    pub agent_id: String,
    pub local_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    /// 可直接执行的安装/更新命令；`None` 时前端只显示文档链接。
    pub command: Option<String>,
    pub docs_url: String,
    /// 存在的配置目录绝对路径（不存在则 `None`，不去创建）。
    pub config_dir: Option<String>,
}

/// npm registry 上的最新版。查不到（离线 / 非 npm 包）一律 `None`。
async fn npm_latest_version(http: &reqwest::Client, package: &str) -> Option<String> {
    let url = format!("https://registry.npmjs.org/{package}/latest");
    let value: serde_json::Value = http
        .get(&url)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    value.get("version")?.as_str().map(str::to_string)
}

async fn pypi_latest_version(http: &reqwest::Client, package: &str) -> Option<String> {
    let url = format!("https://pypi.org/pypi/{package}/json");
    let value: serde_json::Value = http
        .get(&url)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    value
        .get("info")?
        .get("version")?
        .as_str()
        .map(str::to_string)
}

async fn latest_version(http: &reqwest::Client, spec: &AgentInstallSpec) -> Option<String> {
    if let Some(package) = spec.npm_package {
        npm_latest_version(http, package).await
    } else if let Some(package) = spec.pypi_package {
        pypi_latest_version(http, package).await
    } else {
        None
    }
}

async fn antigravity_latest_version(http: &reqwest::Client) -> Option<String> {
    let value: serde_json::Value = http
        .get("https://api.github.com/repos/google-antigravity/antigravity-cli/releases/latest")
        .header(reqwest::header::USER_AGENT, "Kivio")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    extract_semver(value.get("tag_name")?.as_str()?)
}

async fn hermes_latest_version(http: &reqwest::Client) -> Option<String> {
    let value: serde_json::Value = http
        .get("https://api.github.com/repos/NousResearch/hermes-agent/releases/latest")
        .header(reqwest::header::USER_AGENT, "Kivio")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    hermes_version_from_release(&value)
}

fn hermes_version_from_release(value: &serde_json::Value) -> Option<String> {
    // Hermes 的 tag 是日历版本 `v2026.9.14`，CLI 版本 `0.21.3` 在 release name 里。
    // 只读 name，绝不能把 tag 当成可与本机 semver 比较的版本号。
    value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .and_then(extract_semver)
}

/// `--version` 输出里抓语义化版本号：CLI 们的首行格式各不相同
/// （`2.1.207 (Claude Code)` / `codex-cli 0.146.0` / 裸 `0.53.1` / `0.1.0-rc.7`）。
/// 保留 prerelease（dsh 目前全是 rc），丢掉 `+build` metadata。
pub(crate) fn extract_semver(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut start = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_digit() {
            if start.is_none() {
                start = Some(idx);
            }
        } else if ch == '.' {
            continue;
        } else if ch == '-' {
            if let Some(s) = start {
                let core = &text[s..idx];
                if is_numeric_core(core) {
                    let pre_len = prerelease_len(&text[idx + 1..]);
                    if pre_len > 0 {
                        return Some(text[s..idx + 1 + pre_len].to_string());
                    }
                    return Some(core.to_string());
                }
            }
            start = None;
        } else if let Some(s) = start {
            let candidate = &text[s..idx];
            if is_numeric_core(candidate) {
                return Some(candidate.to_string());
            }
            start = None;
        }
    }
    let s = start?;
    let candidate = std::str::from_utf8(&bytes[s..]).ok()?;
    is_numeric_core(candidate).then(|| candidate.to_string())
}

fn is_numeric_core(value: &str) -> bool {
    let mut parts = 0u32;
    for part in value.split('.') {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        parts += 1;
    }
    parts >= 3
}

fn prerelease_len(rest: &str) -> usize {
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].trim_end_matches(['.', '-']).len()
}

#[derive(Clone, PartialEq, Eq)]
enum PreIdent {
    Num(u64),
    Text(String),
}

impl Ord for PreIdent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Num(a), Self::Num(b)) => a.cmp(b),
            (Self::Num(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Num(_)) => std::cmp::Ordering::Greater,
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PreIdent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

struct ParsedVersion {
    core: Vec<u64>,
    pre: Option<Vec<PreIdent>>,
}

fn parse_version(version: &str) -> Option<ParsedVersion> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version
        .split_once('+')
        .map(|(core, _)| core)
        .unwrap_or(version);
    let (core_str, pre_str) = match version.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        _ => (version, None),
    };
    let core = core_str
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    if core.len() < 3 {
        return None;
    }
    let pre = pre_str.map(|value| {
        value
            .split('.')
            .map(|ident| {
                if !ident.is_empty() && ident.chars().all(|c| c.is_ascii_digit()) {
                    ident
                        .parse::<u64>()
                        .map(PreIdent::Num)
                        .unwrap_or_else(|_| PreIdent::Text(ident.to_string()))
                } else {
                    PreIdent::Text(ident.to_string())
                }
            })
            .collect()
    });
    Some(ParsedVersion { core, pre })
}

fn cmp_versions(left: &ParsedVersion, right: &ParsedVersion) -> std::cmp::Ordering {
    let length = left.core.len().max(right.core.len());
    for index in 0..length {
        let left_part = left.core.get(index).copied().unwrap_or(0);
        let right_part = right.core.get(index).copied().unwrap_or(0);
        if left_part != right_part {
            return left_part.cmp(&right_part);
        }
    }
    match (&left.pre, &right.pre) {
        (None, None) => std::cmp::Ordering::Equal,
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(left_pre), Some(right_pre)) => {
            let n = left_pre.len().min(right_pre.len());
            for index in 0..n {
                let order = left_pre[index].cmp(&right_pre[index]);
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            left_pre.len().cmp(&right_pre.len())
        }
    }
}

fn version_is_newer(local: &str, latest: &str) -> bool {
    let (Some(local), Some(latest)) = (parse_version(local), parse_version(latest)) else {
        return false;
    };
    cmp_versions(&local, &latest) == std::cmp::Ordering::Less
}

fn existing_config_dir(spec: &AgentInstallSpec) -> Option<String> {
    let dir = if let Some(env_key) = spec.config_dir_env {
        std::env::var_os(env_key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                directories::UserDirs::new()
                    .and_then(|dirs| spec.config_dir.map(|dir| dirs.home_dir().join(dir)))
            })?
    } else {
        directories::UserDirs::new()?
            .home_dir()
            .join(spec.config_dir?)
    };
    dir.is_dir().then(|| dir.to_string_lossy().into_owned())
}


/// Metadata-only update check; callers choose when to make network requests.
pub async fn management_info(
    http: &reqwest::Client,
    def: super::management_catalog::Definition,
    resolved_path: Option<PathBuf>,
    version: Option<String>,
) -> InstallInfo {
    let spec = &def.install;
    let local_version = version.as_deref().and_then(extract_semver);
    let latest_version = match spec.latest_version {
        LatestVersionStrategy::Registry => latest_version(http, spec).await,
        LatestVersionStrategy::AntigravityRelease => antigravity_latest_version(http).await,
        LatestVersionStrategy::HermesRelease => hermes_latest_version(http).await,
    };
    let update_available = match (&local_version, &latest_version) {
        (Some(local), Some(latest)) => version_is_newer(local, latest),
        _ => false,
    };
    let command = match resolved_path.as_deref() {
        Some(path) => update_plan(spec, path, host_platform()),
        None => install_plan(spec, host_platform()),
    };
    InstallInfo { agent_id: def.id.into(), local_version, latest_version, update_available,
        command: command.map(|plan| plan.display), docs_url: spec.docs.into(), config_dir: existing_config_dir(spec) }
}

fn parse_node_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().trim_start_matches('v');
    let mut parts = text
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty());
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

/// dsh 官方 `engines.node`：`^22.19.0 || >=24.0.0`（不含 23）。
fn node_meets_dsh(version: (u64, u64, u64)) -> bool {
    let (major, minor, _) = version;
    (major == 22 && minor >= 19) || major >= 24
}

fn missing_node_message(for_dsh: bool) -> String {
    if for_dsh {
        format!(
            "安装 DeepSeek Harness 需要 Node.js 22.19+ 或 24+（以及 npm）。\
             当前电脑没有可用的 Node.js，自动安装也没有成功。\
             请打开 {NODE_DOWNLOAD_URL} 下载 LTS 版本，装好后重新打开 Kivio 再点安装。"
        )
    } else {
        format!(
            "安装该 CLI 需要 Node.js 和 npm。当前电脑没有可用的 Node.js，自动安装也没有成功。\
             请打开 {NODE_DOWNLOAD_URL} 下载 LTS 版本后重试。"
        )
    }
}

fn dsh_node_too_old_message(found: &str) -> String {
    format!(
        "当前 Node.js 为 {found}，DeepSeek Harness 需要 22.19+ 或 24+（不含 23）。\
         请打开 {NODE_DOWNLOAD_URL} 安装匹配的 LTS 版本后重试。"
    )
}

fn known_node_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(root) = std::env::var(var) {
                let root = root.trim().trim_end_matches('\\');
                if !root.is_empty() {
                    dirs.push(PathBuf::from(root).join("nodejs"));
                }
            }
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        dirs.push(PathBuf::from(r"D:\Program Files\nodejs"));
        dirs.push(PathBuf::from(r"E:\Program Files\nodejs"));
    }
    #[cfg(not(windows))]
    {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(&home).join(".local").join("bin"));
        }
    }
    dirs
}





/// `npm install -g` 仍会读「当前项目」的 `.npmrc`。`tauri dev` 的 cwd 是仓库根，
/// 仓库 `.npmrc` 写了 `legacy-peer-deps=true`，peer 整包不装。给全局安装一个
/// 自带 package.json 的空项目，npm 就不会再往上走到仓库。
fn isolated_npm_cwd() -> PathBuf {
    let dir = std::env::temp_dir().join("kivio-npm-global");
    let _ = std::fs::create_dir_all(&dir);
    let pkg = dir.join("package.json");
    if !pkg.is_file() {
        let _ = std::fs::write(pkg, "{\"private\":true,\"name\":\"kivio-npm-global\"}\n");
    }
    let npmrc = dir.join(".npmrc");
    if !npmrc.is_file() {
        let _ = std::fs::write(npmrc, "legacy-peer-deps=false\n");
    }
    dir
}

fn configure_install_command(command: &mut tokio::process::Command, plan: &CommandPlan) {
    command.env("PATH", installer_path());
    command.args(&plan.args);
    command.env_remove("npm_config_devdir");
    command.env_remove("NPM_CONFIG_DEVDIR");
    if uses_js_package_manager(plan) {
        pin_official_npm_registry(command);
        command.env_remove("npm_config_legacy_peer_deps");
        command.env_remove("NPM_CONFIG_LEGACY_PEER_DEPS");
        if plan.current_dir.is_none() {
            command.current_dir(isolated_npm_cwd());
        }
    }
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
}

async fn probe_version(program: &str) -> Option<String> {
    let mut command = crate::external_agents::spawn::cli_command(program);
    command.env("PATH", installer_path());
    command.arg("--version");
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.no_console_window();
    command.kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(8), command.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.lines().find(|line| !line.trim().is_empty())?;
    Some(first.trim().to_string())
}

async fn first_working_program(names: &[&str]) -> Option<String> {
    for name in names {
        if probe_version(name).await.is_some() {
            return Some((*name).to_string());
        }
    }
    None
}

async fn probe_node() -> Option<String> {
    for name in ["node", "node.exe"] {
        if let Some(version) = probe_version(name).await {
            return Some(version);
        }
    }
    None
}

async fn npm_is_ready() -> bool {
    first_working_program(&["npm.cmd", "npm"]).await.is_some()
}

async fn pnpm_is_ready() -> bool {
    first_working_program(&["pnpm.cmd", "pnpm"]).await.is_some()
}

pub(crate) async fn stream_install_child(
    mut child: tokio::process::Child,
    emit: &impl Fn(String),
    timeout_secs: u64,
) -> bool {
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let mut out_acc = Vec::new();
    let mut err_acc = Vec::new();
    let mut out_buf = [0u8; 1024];
    let mut err_buf = [0u8; 1024];

    let pump = async {
        loop {
            let out = async {
                match stdout.as_mut() {
                    Some(reader) => Some(reader.read(&mut out_buf).await),
                    None => std::future::pending().await,
                }
            };
            let err = async {
                match stderr.as_mut() {
                    Some(reader) => Some(reader.read(&mut err_buf).await),
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                // 进程退了就收工。Windows 上 `npm.cmd` 经常在打印完最后一行后
                // 仍占着 stdout/stderr（funding 提示、批处理包装），等管道 EOF
                // 会把前端钉死在「执行中」。
                status = child.wait() => {
                    if let Some(line) = flush_progress_line(&mut out_acc) {
                        emit(line);
                    }
                    if let Some(line) = flush_progress_line(&mut err_acc) {
                        emit(line);
                    }
                    break status;
                }
                Some(result) = out => match result {
                    Ok(0) => {
                        if let Some(line) = flush_progress_line(&mut out_acc) {
                            emit(line);
                        }
                        stdout = None;
                    }
                    Ok(n) => {
                        for line in take_progress_lines(&mut out_acc, &out_buf[..n]) {
                            emit(line);
                        }
                    }
                    Err(e) => {
                        emit(format!("读取安装输出失败: {e}"));
                        stdout = None;
                    }
                },
                Some(result) = err => match result {
                    Ok(0) => {
                        if let Some(line) = flush_progress_line(&mut err_acc) {
                            emit(line);
                        }
                        stderr = None;
                    }
                    Ok(n) => {
                        for line in take_progress_lines(&mut err_acc, &err_buf[..n]) {
                            emit(line);
                        }
                    }
                    Err(e) => {
                        emit(format!("读取安装输出失败: {e}"));
                        stderr = None;
                    }
                },
            }
        }
    };

    match tokio::time::timeout(Duration::from_secs(timeout_secs), pump).await {
        Ok(Ok(status)) => status.success(),
        Ok(Err(e)) => {
            emit(format!("安装命令异常结束: {e}"));
            false
        }
        Err(_) => {
            emit(format!("安装超时（{timeout_secs}s），已放弃"));
            false
        }
    }
}

async fn run_logged_plan(
    plan: &CommandPlan,
    emit: &impl Fn(String),
    timeout_secs: u64,
) -> Result<(), String> {
    emit(format!("$ {}", plan.display));
    let mut command = crate::external_agents::spawn::cli_command(&plan.program);
    configure_install_command(&mut command, plan);
    if let Some(dir) = &plan.current_dir {
        command.current_dir(dir);
    }
    let child = command
        .no_console_window()
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("启动命令失败: {e}"))?;
    if stream_install_child(child, emit, timeout_secs).await {
        Ok(())
    } else {
        Err(format!("命令失败：{}", plan.display))
    }
}

async fn bootstrap_node(emit: &impl Fn(String)) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut candidates = vec!["winget".to_string()];
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(local)
                    .join("Microsoft")
                    .join("WindowsApps")
                    .join("winget.exe")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        let names: Vec<&str> = candidates.iter().map(String::as_str).collect();
        if let Some(winget) = first_working_program(&names).await {
            emit("未检测到 Node.js，正在通过 winget 安装 Node.js LTS…".to_string());
            let plan = CommandPlan::direct(
                winget,
                &[
                    "install",
                    "--id",
                    "OpenJS.NodeJS.LTS",
                    "-e",
                    "--source",
                    "winget",
                    "--accept-package-agreements",
                    "--accept-source-agreements",
                    "--disable-interactivity",
                ],
            );
            run_logged_plan(&plan, emit, 300).await?;
            return Ok(());
        }
    }
    #[cfg(unix)]
    {
        if let Some(brew) =
            first_working_program(&["brew", "/opt/homebrew/bin/brew", "/usr/local/bin/brew"]).await
        {
            emit("未检测到 Node.js，正在通过 Homebrew 安装 Node.js…".to_string());
            let plan = CommandPlan::direct(brew, &["install", "node"]);
            run_logged_plan(&plan, emit, 300).await?;
            return Ok(());
        }
    }
    Err("本机没有可用的 Node.js 安装器（Windows 需要 winget，macOS 需要 Homebrew）".to_string())
}

async fn ensure_node(for_dsh: bool, emit: &impl Fn(String)) -> Result<(), String> {
    if let Some(version) = probe_node().await {
        if npm_is_ready().await {
            if for_dsh {
                match parse_node_version(&version) {
                    Some(parsed) if node_meets_dsh(parsed) => return Ok(()),
                    _ => return Err(dsh_node_too_old_message(&version)),
                }
            }
            return Ok(());
        }
    }

    match bootstrap_node(emit).await {
        Ok(()) => {}
        Err(err) => emit(format!("自动安装 Node.js 未成功：{err}")),
    }

    let Some(version) = probe_node().await else {
        return Err(missing_node_message(for_dsh));
    };
    if !npm_is_ready().await {
        return Err(missing_node_message(for_dsh));
    }
    if for_dsh {
        match parse_node_version(&version) {
            Some(parsed) if node_meets_dsh(parsed) => {}
            _ => return Err(dsh_node_too_old_message(&version)),
        }
    }
    emit(format!("Node.js {version} 已就绪"));
    Ok(())
}

async fn ensure_pnpm(emit: &impl Fn(String)) -> Result<(), String> {
    if pnpm_is_ready().await {
        return Ok(());
    }
    emit("未检测到 pnpm，正在安装（dsh 的插件安装依赖它）…".to_string());
    let plan = npm_install_plan("pnpm", &[], host_platform());
    if let Err(err) = run_logged_plan(&plan, emit, 180).await {
        emit(format!("自动安装 pnpm 未成功：{err}"));
    }
    if pnpm_is_ready().await {
        emit("pnpm 已就绪".to_string());
        return Ok(());
    }
    Err(format!(
        "DeepSeek Harness 需要 pnpm 来安装 profile 插件，但当前电脑没有 pnpm。\
         请打开 https://pnpm.io/installation 安装后重试。"
    ))
}

async fn prepare_npm_toolchain(
    spec: &AgentInstallSpec,
    plan: &CommandPlan,
    emit: &impl Fn(String),
) -> Result<(), String> {
    if !uses_npm(plan) && !spec.requires_pnpm {
        return Ok(());
    }
    ensure_node(spec.requires_pnpm, emit).await?;
    if spec.requires_pnpm {
        ensure_pnpm(emit).await?;
    }
    Ok(())
}





fn take_progress_lines(acc: &mut Vec<u8>, chunk: &[u8]) -> Vec<String> {
    acc.extend_from_slice(chunk);
    let mut out = Vec::new();
    loop {
        // Installers can emit an arbitrarily long progress line without a newline.
        // Bound the retained fragment as well as the UI's line count.
        const MAX_FRAGMENT: usize = 8192;
        let delimiter = acc.iter().take(MAX_FRAGMENT).position(|&b| b == b'\n' || b == b'\r');
        let (i, consumed) = match delimiter {
            Some(i) => (i, i + if acc[i] == b'\r' && acc.get(i + 1) == Some(&b'\n') { 2 } else { 1 }),
            None if acc.len() >= MAX_FRAGMENT => (MAX_FRAGMENT, MAX_FRAGMENT),
            None => break,
        };
        let line = String::from_utf8_lossy(&acc[..i]).trim().to_string();
        acc.drain(..consumed);
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

fn flush_progress_line(acc: &mut Vec<u8>) -> Option<String> {
    if acc.is_empty() {
        return None;
    }
    let line = String::from_utf8_lossy(acc).trim().to_string();
    acc.clear();
    (!line.is_empty()).then_some(line)
}


fn installer_path() -> std::ffi::OsString {
    let mut paths = known_node_dirs().into_iter().filter(|path| path.is_dir()).collect::<Vec<_>>();
    for path in std::env::split_paths(&crate::path_env::command_path()) {
        if !paths.contains(&path) { paths.push(path); }
    }
    std::env::join_paths(paths).unwrap_or_else(|_| crate::path_env::command_path())
}
/// Execute only a catalog-owned installation plan. Hosts own progress delivery and post-install work.
pub async fn run_install_command(spec: &AgentInstallSpec, path: Option<&Path>, emit: &(impl Fn(String) + Sync)) -> Result<(), String> {
    let plan = match path {
        Some(path) => update_plan(spec, path, host_platform()).ok_or("无法安全识别该 CLI 的安装来源，请按官方文档手动更新")?,
        None => install_plan(spec, host_platform()).ok_or("该 CLI 没有一键安装方式，请按官方文档手动安装")?,
    };
    prepare_npm_toolchain(spec, &plan, emit).await?;
    let mut command = crate::external_agents::spawn::cli_command(&plan.program);
    configure_install_command(&mut command, &plan);
    let child = command.no_console_window().kill_on_drop(true).spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound && uses_npm(&plan) { missing_node_message(spec.requires_pnpm) }
        else { format!("启动安装命令失败: {error}") }
    })?;
    emit(format!("$ {}", plan.display));
    if stream_install_child(child, emit, 300).await { Ok(()) } else { Err("安装或更新命令执行失败".into()) }
}

pub async fn verify_installed_cli(def: super::management_catalog::Definition, mut command: tokio::process::Command, timeout: Duration) -> Result<String, String> {
    command
        .args(def.version_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let output = tokio::time::timeout(timeout, command.group_output())
        .await
        .map_err(|_| format!("{} --version 超时，安装未通过启动验证", def.bin))?
        .map_err(|e| format!("无法启动 {}：{e}", def.name))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stderr}\n{stdout}");
    if output.status.success() && !combined.contains("ERR_MODULE_NOT_FOUND") {
        if let Some(version) = stdout
            .lines()
            .chain(stderr.lines())
            .find_map(extract_semver)
        {
            return Ok(version);
        }
    }
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|line| !line.is_empty())
        .unwrap_or("没有返回有效版本号");
    let detail: String = detail.chars().take(4000).collect();
    Err(format!(
        "{} 已写入，但 `{} --version` 未通过启动验证（{}）：{detail}",
        def.name, def.bin, output.status
    ))
}
