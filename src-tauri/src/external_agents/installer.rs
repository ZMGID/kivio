//! 设置页「本地 CLI Agent」的安装/更新支撑：官方安装方式表、npm registry 最新版查询、
//! 带流式日志的安装执行、配置目录打开。
//!
//! ponytail: 查版本 / 跑官方命令 / 打开目录。**没有**卸载、没有取消、没有多后端策略协商。
//! 自动维护只执行预览捕获的 npm 目标；未证明归属的安装使用官方手动指引。
//! npm / pnpm / yarn / bun 的安装命令一律钉死 `registry.npmjs.org`：国内镜像经常
//! 缺 `@deepseek-ai/*` 这类新包，用户自己的 `.npmrc` 不能把安装带跑偏。
use crate::external_agents::spawn::CliProcessOutput;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncReadExt;

use crate::external_agents::registry::get_agent_def;
use crate::external_agents::types::{AgentInstallSpec, LatestVersionStrategy, PostInstallStrategy};
use crate::proc::NoConsoleWindow;

use super::session::live::{resource_path, runtime_resources, ResourceLease, MAINTENANCE_OWNER};
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

struct PreparedInstall {
    agent: String,
    binary: PathBuf,
    prefix: PathBuf,
    package: String,
    version: String,
    identity: super::overrides::LaunchIdentity,
    command: CommandPlan,
    environment: BTreeMap<OsString, OsString>,
    target_environment: BTreeMap<OsString, OsString>,
    created: Instant,
    existed: bool,
}
static INSTALL_PLANS: LazyLock<Mutex<HashMap<String, PreparedInstall>>> =
    LazyLock::new(Default::default);

pub(crate) fn proven_npm_prefix(binary: &Path, package: &str) -> Option<PathBuf> {
    if super::wsl::is_wsl_target(binary) {
        return None;
    }
    let real = std::fs::canonicalize(binary).ok()?;
    let mut prefixes = Vec::new();
    if let Some(parent) = binary.parent() {
        prefixes.push(parent.to_path_buf());
    }
    for ancestor in real.ancestors() {
        if ancestor
            .file_name()
            .is_some_and(|name| name == "node_modules")
        {
            let parent = ancestor.parent()?;
            prefixes.push(if parent.file_name().is_some_and(|name| name == "lib") {
                parent.parent()?.to_path_buf()
            } else {
                parent.to_path_buf()
            });
        }
    }
    for prefix in prefixes {
        for modules in [if cfg!(windows) {
            prefix.join("node_modules")
        } else {
            prefix.join("lib/node_modules")
        }] {
            let root = modules.join(package);
            let Some(real_prefix) = std::fs::canonicalize(&prefix).ok() else {
                continue;
            };
            let Some(real_root) = std::fs::canonicalize(&root).ok() else {
                continue;
            };
            let expected_root = real_prefix
                .join(if cfg!(windows) {
                    "node_modules"
                } else {
                    "lib/node_modules"
                })
                .join(package);
            if !path_eq(
                &real_root.to_string_lossy().replace('\\', "/"),
                &expected_root.to_string_lossy().replace('\\', "/"),
            ) {
                continue;
            }
            let manifest: serde_json::Value = match std::fs::read(root.join("package.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            {
                Some(value) => value,
                None => continue,
            };
            if manifest["name"].as_str() != Some(package) {
                continue;
            }
            let entries: Vec<&str> = if let Some(bin) = manifest["bin"].as_str() {
                vec![bin]
            } else {
                manifest["bin"]
                    .as_object()
                    .map(|bins| {
                        bins.values()
                            .filter_map(serde_json::Value::as_str)
                            .collect()
                    })
                    .unwrap_or_default()
            };
            for entry in entries {
                let target = std::fs::canonicalize(root.join(entry)).ok()?;
                if !target.starts_with(&real_root) {
                    continue;
                }
                if target == real {
                    return Some(prefix);
                }
                if binary
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
                {
                    let shim = std::fs::read_to_string(binary)
                        .ok()?
                        .replace('\\', "/")
                        .to_lowercase();
                    let relative = format!(
                        "%dp0%/node_modules/{package}/{}",
                        entry.trim_start_matches("./")
                    )
                    .to_lowercase();
                    if npm_shim_matches(&shim, &relative) {
                        return Some(prefix);
                    }
                }
            }
        }
    }
    None
}

fn npm_shim_matches(shim: &str, target: &str) -> bool {
    let expected = format!(
        r#"@echo off
goto start
:find_dp0
set dp0=%~dp0
exit /b
:start
setlocal
call :find_dp0
if exist "%dp0%/node.exe" (
set "_prog=%dp0%/node.exe"
) else (
set "_prog=node"
set pathext=%pathext:;.js;=;%
)
endlocal & goto #_undefined_# 2>nul || title %comspec% & "%_prog%" "{target}" %*"#
    );
    shim.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .eq(expected.lines())
}

fn consume_plan(id: &str, agent: &str) -> Result<PreparedInstall, String> {
    let plan = INSTALL_PLANS
        .lock()
        .map_err(|_| "安装计划锁不可用")?
        .remove(id)
        .ok_or("安装计划已使用或过期，请刷新")?;
    if plan.agent != agent || plan.created.elapsed() > Duration::from_secs(600) {
        return Err("安装计划已过期，请刷新".into());
    }
    Ok(plan)
}

fn npm_command_name(
    def: &super::types::RuntimeAgentDef,
    manifest: &serde_json::Value,
) -> Option<String> {
    let bins = manifest["bin"].as_object()?;
    std::iter::once(def.bin)
        .chain(def.fallback_bins.iter().copied())
        .find(|name| bins.get(*name).is_some_and(serde_json::Value::is_string))
        .map(str::to_string)
}

fn environment_key_matches(left: &std::ffi::OsStr, right: &std::ffi::OsStr) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}
fn environment_value<'a>(
    environment: &'a BTreeMap<OsString, OsString>,
    key: &str,
) -> Option<&'a OsString> {
    environment
        .iter()
        .find(|(name, _)| environment_key_matches(name, std::ffi::OsStr::new(key)))
        .map(|(_, value)| value)
}
fn capture_command_environment(
    environment: &mut BTreeMap<OsString, OsString>,
    command: &tokio::process::Command,
) {
    for (key, value) in command.as_std().get_envs() {
        environment.retain(|existing, _| !environment_key_matches(existing, key));
        if let Some(value) = value {
            environment.insert(key.to_owned(), value.to_owned());
        }
    }
}

async fn prepare_install(
    http: &reqwest::Client,
    def: &super::types::RuntimeAgentDef,
    binary: Option<&Path>,
    version: Option<&str>,
) -> Result<PreparedInstall, String> {
    if super::overrides::custom_path(def.id).is_some() && binary.is_none() {
        return Err("自定义 CLI 路径不可用，请先修复该路径".into());
    }
    if binary.is_some_and(super::wsl::is_wsl_target) {
        return Err("WSL 安装需在对应发行版中按官方文档维护".into());
    }
    if def.install.requires_pnpm && !pnpm_is_ready().await {
        return Err("请先按 https://pnpm.io/installation 安装 pnpm，再刷新安装计划".into());
    }
    let package = def
        .install
        .npm_package
        .ok_or("此原生安装尚无可验证的固定版本命令，请按官方文档维护")?;
    let version = version
        .filter(|value| extract_semver(value).as_deref() == Some(*value))
        .ok_or("无法确定准确目标版本，请刷新后重试")?;
    let environment: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
    let npm_name = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let npm = environment_value(&environment, "PATH")
        .and_then(|path| {
            std::env::split_paths(path)
                .map(|dir| dir.join(npm_name))
                .find(|path| path.is_absolute() && path.is_file())
        })
        .ok_or("请先按官方文档安装 Node.js/npm")?;
    let prefix = if let Some(binary) = binary {
        proven_npm_prefix(binary, package)
            .ok_or("无法验证此 CLI 的 npm 安装归属，请按官方文档更新")?
    } else {
        let mut command = super::spawn::cli_command(&npm);
        command
            .env_clear()
            .envs(&environment)
            .args(["prefix", "-g"])
            .current_dir(isolated_npm_cwd());
        let output = tokio::time::timeout(Duration::from_secs(10), command.group_output())
            .await
            .map_err(|_| "npm prefix 查询超时")?
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("无法确认 npm 安装目录".into());
        }
        let prefix = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if !prefix.is_absolute() {
            return Err("npm 返回了非绝对安装目录".into());
        }
        prefix
    };
    let command_name = if binary.is_none() {
        let manifest: serde_json::Value = http
            .get(format!("https://registry.npmjs.org/{package}/{version}"))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        if manifest["name"].as_str() != Some(package)
            || manifest["version"].as_str() != Some(version)
        {
            return Err("npm 元数据与计划版本不符".into());
        }
        npm_command_name(def, &manifest).ok_or("无法验证 npm 包的命令入口")?
    } else {
        def.bin.to_string()
    };
    let target = binary.map(Path::to_path_buf).unwrap_or_else(|| {
        if cfg!(windows) {
            prefix.join(format!("{command_name}.cmd"))
        } else {
            prefix.join("bin").join(command_name)
        }
    });
    if binary.is_none() && target.exists() {
        return Err("安装目录已存在未知来源的命令，请手动检查".into());
    }
    let mut args = npm_install_plan(package, def.install.npm_install_args, host_platform()).args;
    args.retain(|arg| !arg.ends_with("@latest"));
    args.extend([
        "--prefix".into(),
        prefix.to_string_lossy().into_owned(),
        format!("{package}@{version}"),
    ]);
    let mut command = CommandPlan::owned(npm.to_string_lossy(), args);
    command.current_dir = Some(isolated_npm_cwd());
    let target_command = super::spawn::agent_probe_command(def, &target);
    let mut target_environment = environment.clone();
    capture_command_environment(&mut target_environment, &target_command);
    Ok(PreparedInstall {
        agent: def.id.into(),
        identity: super::overrides::LaunchIdentity::capture(def, &target),
        binary: target,
        prefix,
        package: package.into(),
        version: version.into(),
        command,
        environment,
        target_environment,
        created: Instant::now(),
        existed: binary.is_some(),
    })
}

fn install_spec(agent_id: &str) -> Option<&'static AgentInstallSpec> {
    get_agent_def(agent_id).map(|def| &def.install)
}

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
}

fn display_token(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

const NPMJS_REGISTRY: &str = "https://registry.npmjs.org/";
const NPMJS_REGISTRY_FLAG: &str = "--registry=https://registry.npmjs.org/";
const DEEPSEEK_SCOPE_REGISTRY_FLAG: &str = "--@deepseek-ai:registry=https://registry.npmjs.org/";

fn uses_js_package_manager(plan: &CommandPlan) -> bool {
    matches!(
        Path::new(&plan.program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(""),
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

/// dsh 官方 `engines.node`：`^22.19.0 || >=24.0.0`（不含 23）。

fn prepend_path_dirs(dirs: &[PathBuf]) {
    let current = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut extra = Vec::new();
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let value = dir.to_string_lossy().into_owned();
        let already = extra.iter().any(|item: &String| path_eq(item, &value))
            || current.split(sep).any(|item| path_eq(item, &value));
        if !already {
            extra.push(value);
        }
    }
    if extra.is_empty() {
        return;
    }
    if current.is_empty() {
        std::env::set_var("PATH", extra.join(&sep.to_string()));
    } else {
        extra.push(current);
        std::env::set_var("PATH", extra.join(&sep.to_string()));
    }
}

fn path_eq(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
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

pub(crate) fn expose_node_on_path() {
    crate::path_env::refresh_path_now();
    prepend_path_dirs(&known_node_dirs());
}

pub(crate) async fn ensure_pnpm_for_dsh() -> Result<(), String> {
    expose_node_on_path();
    if pnpm_is_ready().await {
        Ok(())
    } else {
        Err("DeepSeek Harness 需要 pnpm，请先按 https://pnpm.io/installation 安装后重试".into())
    }
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
    command.arg("--version");
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.no_console_window();
    command.kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(8), command.group_output())
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

async fn pnpm_is_ready() -> bool {
    first_working_program(&["pnpm.cmd", "pnpm"]).await.is_some()
}

async fn stream_install_child(
    mut child: super::spawn::AgentChild,
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
            let _ = child.start_kill();
            let _ = child.wait().await;
            emit(format!("安装超时（{timeout_secs}s），已放弃"));
            false
        }
    }
}

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

/// 已安装时优先让当前二进制自更新；Kimi/Gemini 没有可用的无交互入口，按安装来源处理。

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
    pub plan_id: Option<String>,
    pub manual_reason: Option<String>,
    pub binary_path: Option<String>,
    pub target_version: Option<String>,
    pub environment: Option<String>,
    pub source: Option<String>,
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

#[tauri::command]
pub async fn chat_external_cli_install_info(
    state: tauri::State<'_, crate::state::AppState>,
    agent_id: String,
) -> Result<InstallInfo, String> {
    let def = get_agent_def(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;
    let spec = install_spec(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;

    let preview_revision = super::overrides::launch_revision(def.id);
    let resolved_path = crate::external_agents::spawn::resolve_binary(def).await;
    let preview_identity = resolved_path
        .as_deref()
        .map(|binary| super::overrides::LaunchIdentity::capture(def, binary));
    if preview_revision.is_none() || preview_revision != super::overrides::launch_revision(def.id) {
        return Err("CLI 配置已变更，请刷新".into());
    }
    let local_version = resolved_path
        .as_deref()
        .and_then(crate::external_agents::spawn::cached_cli_version)
        .as_deref()
        .and_then(extract_semver);
    let latest_version = match spec.latest_version {
        LatestVersionStrategy::Registry => latest_version(&state.http, spec).await,
        LatestVersionStrategy::AntigravityRelease => antigravity_latest_version(&state.http).await,
        LatestVersionStrategy::HermesRelease => hermes_latest_version(&state.http).await,
    };
    let update_available = match (&local_version, &latest_version) {
        (Some(local), Some(latest)) => version_is_newer(local, latest),
        _ => false,
    };
    let prepared = prepare_install(
        &state.http,
        def,
        resolved_path.as_deref(),
        latest_version.as_deref(),
    )
    .await;
    let (plan_id, command, manual_reason, binary_path, source) = match prepared {
        Ok(mut plan) => {
            if preview_revision != super::overrides::launch_revision(def.id)
                || preview_identity
                    .as_ref()
                    .is_some_and(|identity| !identity.still_current(def, &plan.binary))
            {
                return Err("CLI 配置已变更，请刷新".into());
            }
            if let Some(identity) = preview_identity {
                plan.identity = identity;
            }
            let id = uuid::Uuid::new_v4().to_string();
            let display = plan.command.display.clone();
            let binary = plan.binary.to_string_lossy().into_owned();
            let source = format!("npm ({})", plan.prefix.display());
            let mut plans = INSTALL_PLANS.lock().map_err(|_| "安装计划锁不可用")?;
            plans.retain(|_, old| {
                old.created.elapsed() < Duration::from_secs(600) && old.agent != agent_id
            });
            plans.insert(id.clone(), plan);
            (Some(id), Some(display), None, Some(binary), Some(source))
        }
        Err(reason) => (
            None,
            None,
            Some(reason),
            resolved_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            None,
        ),
    };
    let config_dir = existing_config_dir(spec);

    Ok(InstallInfo {
        agent_id,
        local_version,
        latest_version: latest_version.clone(),
        update_available,
        command,
        plan_id,
        manual_reason,
        binary_path,
        target_version: latest_version.clone(),
        environment: Some(
            if resolved_path
                .as_deref()
                .is_some_and(super::wsl::is_wsl_target)
            {
                "WSL"
            } else {
                "native"
            }
            .into(),
        ),
        source,
        docs_url: spec.docs.to_string(),
        config_dir,
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallLogEvent {
    agent_id: String,
    plan_id: String,
    line: Option<String>,
    done: bool,
    success: bool,
}

const INSTALL_TIMEOUT_SECS: u64 = 300;

// Discovery deliberately accepts a spawned process even when it exits unsuccessfully.
// Installation needs a stronger check: a successful exit and an actual version.
async fn verify_cli_version(
    def: &crate::external_agents::types::RuntimeAgentDef,
    bin: &Path,
    timeout: Duration,
) -> Result<String, String> {
    verify_cli_version_in(def, bin, timeout, None).await
}
async fn verify_cli_version_in(
    def: &super::types::RuntimeAgentDef,
    bin: &Path,
    timeout: Duration,
    environment: Option<&BTreeMap<OsString, OsString>>,
) -> Result<String, String> {
    let mut command = crate::external_agents::spawn::agent_probe_command(def, bin);
    if let Some(environment) = environment {
        command.env_clear().envs(environment);
        super::spawn::mark_resource(
            &mut command,
            Some(def.id),
            bin,
            super::overrides::launch_revision(def.id),
        );
    }
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

async fn finish_cli_install(
    def: &super::types::RuntimeAgentDef,
    plan: &PreparedInstall,
    emit: &impl Fn(String),
) -> Result<(), String> {
    super::spawn::invalidate_probe_cache(&plan.binary);
    let version = verify_cli_version_in(
        def,
        &plan.binary,
        Duration::from_secs(20),
        Some(&plan.target_environment),
    )
    .await?;
    if version != plan.version {
        return Err(format!(
            "计划目标版本 {}，实际 {}。请修复 {}",
            plan.version,
            version,
            plan.binary.display()
        ));
    }
    emit(format!("{} {version} 启动验证通过", def.name));
    if matches!(def.install.post_install, PostInstallStrategy::DshProfile) {
        super::dsh_profile::ensure_profile_ready(&plan.binary, None, None).await?;
    }
    Ok(())
}

/// 把一块输出拆成日志行。`\n` 和 `\r` 都算换行——npm 进度条只写 `\r`，按 `\n` 读
/// 会一直等，管道塞满后安装进程自己也写不动。
fn take_progress_lines(acc: &mut Vec<u8>, chunk: &[u8]) -> Vec<String> {
    acc.extend_from_slice(chunk);
    let mut out = Vec::new();
    loop {
        let Some(i) = acc.iter().position(|&b| b == b'\n' || b == b'\r') else {
            break;
        };
        let line = String::from_utf8_lossy(&acc[..i]).trim().to_string();
        let crlf = acc[i] == b'\r' && acc.get(i + 1) == Some(&b'\n');
        acc.drain(..i + if crlf { 2 } else { 1 });
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

/// 跑 `install_command` 并把 stdout/stderr 按行流到 `external-cli-install` 事件。
///
/// ponytail: 不支持中途取消——安装是低频一次性动作，超时（300s）就自己结束。
#[tauri::command]
pub async fn chat_external_cli_install(
    app: AppHandle,
    agent_id: String,
    plan_id: String,
) -> Result<(), String> {
    let def = get_agent_def(&agent_id).ok_or("未知外部 Agent")?;
    let plan = consume_plan(&plan_id, &agent_id)?;
    let mut keys = runtime_resources(Some(def.id), &plan.binary);
    keys.push(format!("prefix:{}", resource_path(&plan.prefix)));
    let lease = ResourceLease::acquire(keys, true)?;
    if !plan.identity.still_current(def, &plan.binary)
        || (plan.existed
            && proven_npm_prefix(&plan.binary, &plan.package)
                .as_ref()
                .map(|path| resource_path(path))
                != Some(resource_path(&plan.prefix)))
    {
        return Err("CLI 路径、配置或安装来源已改变，请刷新安装计划".into());
    }
    let emit = |line: String| {
        let _ = app.emit(
            "external-cli-install",
            InstallLogEvent {
                agent_id: agent_id.clone(),
                plan_id: plan_id.clone(),
                line: Some(line),
                done: false,
                success: false,
            },
        );
    };
    let result = MAINTENANCE_OWNER
        .scope(lease.clone(), async {
            let mut command = super::spawn::cli_command(&plan.command.program);
            command.env_clear().envs(&plan.environment);
            configure_install_command(&mut command, &plan.command);
            command.current_dir(
                plan.command
                    .current_dir
                    .as_ref()
                    .expect("npm project directory"),
            );
            pin_official_npm_registry(&mut command);
            emit(format!("$ {}", plan.command.display));
            let child = super::spawn::AgentChild::spawn(&mut command)
                .map_err(|e| format!("启动安装命令失败: {e}"))?;
            if !stream_install_child(child, &emit, INSTALL_TIMEOUT_SECS).await {
                return Err("安装或更新失败；原路径仍保留，请按官方文档修复该安装".into());
            }
            finish_cli_install(def, &plan, &emit).await
        })
        .await;
    let _ = app.emit(
        "external-cli-install",
        InstallLogEvent {
            agent_id,
            plan_id,
            line: None,
            done: true,
            success: result.is_ok(),
        },
    );
    result
}

/// 在系统文件管理器里打开该 CLI 的配置目录。
#[tauri::command]
pub fn chat_external_cli_open_config_dir(agent_id: String) -> Result<(), String> {
    let spec = install_spec(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;
    let dir = existing_config_dir(spec).ok_or_else(|| "配置目录还不存在".to_string())?;
    open_path(Path::new(&dir))
}

fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let program = "xdg-open";

    std::process::Command::new(program)
        .arg(path)
        .no_console_window()
        .spawn()
        .map_err(|e| format!("打开目录失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_version_probe(dir: &Path, windows: &str, unix: &str) -> PathBuf {
        let path = dir.join(if cfg!(windows) { "probe.cmd" } else { "probe" });
        let script = if cfg!(windows) {
            format!("@echo off\r\n{windows}\r\n")
        } else {
            format!("#!/bin/sh\n{unix}\n")
        };
        std::fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    fn fixture_plan(binary: &Path, agent: &str, version: &str) -> PreparedInstall {
        let def = get_agent_def(agent).unwrap();
        let environment: BTreeMap<OsString, OsString> = std::env::vars_os().collect();
        PreparedInstall {
            agent: agent.into(),
            binary: binary.into(),
            prefix: binary.parent().unwrap().into(),
            package: "fixture".into(),
            version: version.into(),
            identity: super::super::overrides::LaunchIdentity::capture(def, binary),
            command: CommandPlan::direct("unused", &[]),
            target_environment: environment.clone(),
            environment,
            created: Instant::now(),
            existed: binary.exists(),
        }
    }
    #[cfg(windows)]
    #[test]
    fn captured_windows_environment_respects_path_case_override_and_removal() {
        let mut environment =
            BTreeMap::from([(OsString::from("Path"), OsString::from("old-path"))]);
        assert_eq!(
            environment_value(&environment, "PATH"),
            Some(&OsString::from("old-path"))
        );
        let mut command = tokio::process::Command::new("unused");
        command.env("PATH", "planned-path");
        capture_command_environment(&mut environment, &command);
        assert_eq!(environment.len(), 1);
        assert_eq!(
            environment_value(&environment, "Path"),
            Some(&OsString::from("planned-path"))
        );
        command.env_remove("path");
        capture_command_environment(&mut environment, &command);
        assert!(environment_value(&environment, "PATH").is_none());
    }

    #[test]
    fn install_plan_is_single_use_expiring_and_rejects_replaced_target() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("fixture");
        std::fs::write(&binary, b"old").unwrap();
        let plan = fixture_plan(&binary, "codex", "1.2.3");
        let identity = plan.identity.clone();
        let id = uuid::Uuid::new_v4().to_string();
        INSTALL_PLANS.lock().unwrap().insert(id.clone(), plan);
        assert!(consume_plan(&id, "codex").is_ok());
        assert!(consume_plan(&id, "codex").is_err());
        let mut expired = fixture_plan(&binary, "codex", "1.2.3");
        expired.created = Instant::now() - Duration::from_secs(601);
        INSTALL_PLANS.lock().unwrap().insert(id.clone(), expired);
        assert!(consume_plan(&id, "codex").is_err());
        std::fs::write(&binary, b"replacement binary").unwrap();
        assert!(!identity.still_current(get_agent_def("codex").unwrap(), &binary));
    }
    #[tokio::test]
    async fn every_provider_verifies_exact_original_target_even_after_first_install() {
        let root = tempfile::tempdir().unwrap();
        let binary = root
            .path()
            .join(if cfg!(windows) { "probe.cmd" } else { "probe" });
        let mut plan = fixture_plan(&binary, "codex", "1.2.3");
        write_version_probe(root.path(), "echo 1.2.2", "echo 1.2.2");
        for def in super::super::registry::AGENT_DEFS {
            assert!(
                finish_cli_install(def, &plan, &|_| {}).await.is_err(),
                "{}",
                def.id
            );
        }
        write_version_probe(root.path(), "echo 1.2.3", "echo 1.2.3");
        finish_cli_install(get_agent_def("codex").unwrap(), &plan, &|_| {})
            .await
            .unwrap();
        plan.target_environment
            .insert("PLAN_TEST_VERSION".into(), "1.2.3".into());
        write_version_probe(
            root.path(),
            "echo %PLAN_TEST_VERSION%",
            "echo $PLAN_TEST_VERSION",
        );
        finish_cli_install(get_agent_def("codex").unwrap(), &plan, &|_| {})
            .await
            .unwrap();
    }
    #[test]
    fn npm_entry_uses_manifest_alias_and_rejects_comment_only_shim() {
        let def = get_agent_def("opencode").unwrap();
        assert_eq!(
            npm_command_name(def, &serde_json::json!({"bin":{"opencode":"bin/opencode"}}))
                .as_deref(),
            Some("opencode")
        );
        assert!(!npm_shim_matches(
            "@echo off\nrem \"%dp0%/node_modules/fixture/bin.js\"\necho wrong",
            "%dp0%/node_modules/fixture/bin.js"
        ));
    }
    #[test]
    fn npm_source_requires_real_manifest_global_layout_and_entry() {
        let root = tempfile::tempdir().unwrap();
        let modules = if cfg!(windows) {
            root.path().join("node_modules/fixture")
        } else {
            root.path().join("lib/node_modules/fixture")
        };
        std::fs::create_dir_all(&modules).unwrap();
        let binary = modules.join("main.js");
        std::fs::write(&binary, b"fixture").unwrap();
        std::fs::write(
            modules.join("package.json"),
            r#"{"name":"fixture","bin":{"fixture":"main.js"}}"#,
        )
        .unwrap();
        assert_eq!(
            proven_npm_prefix(&binary, "fixture").map(|path| resource_path(&path)),
            Some(resource_path(root.path()))
        );
        assert!(proven_npm_prefix(&binary, "other").is_none());
        let wrapper = root.path().join("unrelated.cmd");
        std::fs::write(&wrapper, "@echo off\necho unrelated").unwrap();
        assert!(proven_npm_prefix(&wrapper, "fixture").is_none());
        assert!(proven_npm_prefix(Path::new(r"\\wsl$\Ubuntu\home\u\fixture"), "fixture").is_none());
        #[cfg(unix)]
        {
            let local = root.path().join("node_modules/fixture");
            std::fs::create_dir_all(&local).unwrap();
            std::fs::copy(modules.join("package.json"), local.join("package.json")).unwrap();
            std::fs::write(local.join("main.js"), b"fixture").unwrap();
            assert!(proven_npm_prefix(&local.join("main.js"), "fixture").is_none());
        }
    }

    #[cfg(windows)]
    #[test]
    fn npm_source_rejects_redirected_modules_but_allows_whole_prefix_alias() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        let redirected = root.path().join("redirected");
        let alias = root.path().join("alias");
        let package = real.join("node_modules").join("fixture");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::create_dir_all(&redirected).unwrap();
        std::fs::write(package.join("main.js"), b"fixture").unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"fixture","bin":"main.js"}"#,
        )
        .unwrap();
        let shim = r#"@echo off
goto start
:find_dp0
set dp0=%~dp0
exit /b
:start
setlocal
call :find_dp0
if exist "%dp0%\node.exe" (
set "_prog=%dp0%\node.exe"
) else (
set "_prog=node"
set PATHEXT=%PATHEXT:;.JS;=;%
)
endlocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%" "%dp0%\node_modules\fixture\main.js" %*"#;
        std::fs::write(real.join("fixture.cmd"), shim).unwrap();
        std::fs::write(redirected.join("fixture.cmd"), shim).unwrap();
        for (link, target) in [
            (redirected.join("node_modules"), real.join("node_modules")),
            (alias.clone(), real.clone()),
        ] {
            let output = std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .no_console_window()
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        }
        assert!(proven_npm_prefix(&redirected.join("fixture.cmd"), "fixture").is_none());
        for prefix in [real.clone(), alias] {
            assert_eq!(
                proven_npm_prefix(&prefix.join("fixture.cmd"), "fixture")
                    .map(|path| resource_path(&path)),
                Some(resource_path(&real))
            );
        }
    }

    #[tokio::test]
    async fn pi_version_verification_rejects_failed_exit_and_allows_retry() {
        let dir = tempfile::tempdir().unwrap();
        let def = get_agent_def("pi").unwrap();
        let bin = write_version_probe(
            dir.path(),
            "echo 0.87.1\r\necho enableCompileCache missing 1>&2\r\nexit /b 1",
            "echo 0.87.1\necho enableCompileCache missing >&2\nexit 1",
        );
        let err = verify_cli_version(def, &bin, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert!(err.contains("Pi"));
        assert!(err.contains("enableCompileCache missing"));

        // A failed probe must not poison the same path after a repair/retry.
        write_version_probe(
            dir.path(),
            "echo 0.87.1\r\nexit /b 0",
            "echo 0.87.1\nexit 0",
        );
        assert_eq!(
            verify_cli_version(def, &bin, Duration::from_secs(5)).await,
            Ok("0.87.1".to_string())
        );
    }

    #[tokio::test]
    async fn installed_cli_requires_a_version_even_with_successful_exit() {
        let dir = tempfile::tempdir().unwrap();
        let bin = write_version_probe(
            dir.path(),
            "echo not a version\r\nexit /b 0",
            "echo not a version\nexit 0",
        );
        for id in ["pi", "dsh"] {
            assert!(
                verify_cli_version(get_agent_def(id).unwrap(), &bin, Duration::from_secs(5))
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn installed_cli_accepts_versions_on_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let bin = write_version_probe(
            dir.path(),
            "echo dsh 0.1.0-rc.7 1>&2\r\nexit /b 0",
            "echo dsh 0.1.0-rc.7 >&2\nexit 0",
        );
        assert_eq!(
            verify_cli_version(get_agent_def("dsh").unwrap(), &bin, Duration::from_secs(5)).await,
            Ok("0.1.0-rc.7".to_string())
        );
    }

    #[tokio::test]
    async fn pi_version_verification_handles_missing_command_and_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let def = get_agent_def("pi").unwrap();
        assert!(
            verify_cli_version(def, &dir.path().join("missing.exe"), Duration::from_secs(5))
                .await
                .is_err()
        );
        // Shell builtins only: timing out must not leave a descendant process behind.
        let bin = write_version_probe(dir.path(), ":spin\r\ngoto spin", "while :; do :; done");
        let err = verify_cli_version(def, &bin, Duration::from_millis(200))
            .await
            .unwrap_err();
        assert!(err.contains("超时"));
    }

    #[test]
    fn extract_semver_handles_each_cli_version_line() {
        assert_eq!(
            extract_semver("2.1.207 (Claude Code)").as_deref(),
            Some("2.1.207")
        );
        assert_eq!(
            extract_semver("codex-cli 0.146.0").as_deref(),
            Some("0.146.0")
        );
        assert_eq!(extract_semver("0.53.1").as_deref(), Some("0.53.1"));
        assert_eq!(extract_semver("v1.2.3-beta").as_deref(), Some("1.2.3-beta"));
        assert_eq!(extract_semver("0.1.0-rc.7").as_deref(), Some("0.1.0-rc.7"));
        assert_eq!(
            extract_semver("dsh 0.1.0-rc.7").as_deref(),
            Some("0.1.0-rc.7")
        );
        assert_eq!(extract_semver("no version here"), None);
        // 两段号不是版本号，别把它当成版本报给用户。
        assert_eq!(extract_semver("cli 1.2"), None);
    }

    #[test]
    fn update_is_available_only_when_latest_version_is_newer() {
        assert!(version_is_newer("1.18.13", "1.19.0"));
        assert!(!version_is_newer("1.18.13", "1.18.13"));
        assert!(!version_is_newer("1.19.0", "1.18.13"));
        assert!(!version_is_newer("1.18.13.0", "1.18.13"));
        assert!(version_is_newer("0.1.0-rc.6", "0.1.0-rc.7"));
        assert!(!version_is_newer("0.1.0-rc.7", "0.1.0-rc.7"));
        assert!(!version_is_newer("0.1.0-rc.7", "0.1.0-rc.6"));
        assert!(version_is_newer("0.1.0-rc.7", "0.1.0"));
        assert!(!version_is_newer("0.1.0", "0.1.0-rc.7"));
        assert!(version_is_newer("0.1.0-rc.7", "0.1.1"));
    }

    #[test]
    fn every_registered_agent_has_an_install_spec() {
        for def in crate::external_agents::registry::AGENT_DEFS {
            assert!(install_spec(def.id).is_some(), "缺少安装表: {}", def.id);
        }
    }

    #[test]
    fn isolated_npm_cwd_is_its_own_project() {
        let dir = isolated_npm_cwd();
        let pkg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(pkg["private"], true);
        let npmrc = std::fs::read_to_string(dir.join(".npmrc")).unwrap();
        assert!(npmrc.contains("legacy-peer-deps=false"));
    }

    #[test]
    fn npm_install_pins_official_registry_and_deepseek_scope() {
        let dsh = npm_install_plan(
            "@deepseek-ai/dsh",
            &["@deepseek-ai/cordis-plugin-group@latest"],
            HostPlatform::Windows,
        );
        assert_eq!(dsh.program, "npm.cmd");
        assert!(dsh
            .args
            .contains(&"--registry=https://registry.npmjs.org/".to_string()));
        assert!(dsh
            .args
            .contains(&"--@deepseek-ai:registry=https://registry.npmjs.org/".to_string()));

        let claude = npm_install_plan("@anthropic-ai/claude-code", &[], HostPlatform::Unix);
        assert!(claude
            .args
            .contains(&"--registry=https://registry.npmjs.org/".to_string()));
        assert!(!claude
            .args
            .iter()
            .any(|arg| arg.contains("@deepseek-ai:registry")));
    }

    #[test]
    fn progress_lines_split_on_cr_and_lf() {
        let mut acc = Vec::new();
        assert_eq!(
            take_progress_lines(&mut acc, b"downloading\radded 12 packages\n"),
            vec!["downloading", "added 12 packages"]
        );
        assert!(acc.is_empty());

        assert_eq!(
            take_progress_lines(&mut acc, b"partial"),
            Vec::<String>::new()
        );
        assert_eq!(flush_progress_line(&mut acc).as_deref(), Some("partial"));
        assert_eq!(
            take_progress_lines(&mut acc, b"\r\n  \nwarn\r\n"),
            vec!["warn"]
        );
    }
}
