//! 设置页「本地 CLI Agent」的安装/更新支撑：官方安装方式表、npm registry 最新版查询、
//! 带流式日志的安装执行、配置目录打开。
//!
//! ponytail: 查版本 / 跑官方命令 / 打开目录。**没有**卸载、没有取消、没有多后端策略协商。
//! npm 系 CLI 在跑命令前会补 Node（dsh 再补 pnpm）：小白机器上没有 Node 时直接
//! `npm.cmd` 只会丢「找不到文件」。
//! npm / pnpm / yarn / bun 的安装命令一律钉死 `registry.npmjs.org`：国内镜像经常
//! 缺 `@deepseek-ai/*` 这类新包，用户自己的 `.npmrc` 不能把安装带跑偏。
use tauri::{AppHandle, Emitter};
use crate::external_agents::registry::get_agent_def;
include!("installer_core.rs");
fn install_spec(agent_id: &str) -> Option<&'static AgentInstallSpec> {
    get_agent_def(agent_id).map(|def| &def.install)
}
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

pub(crate) fn expose_node_on_path() {
    crate::path_env::refresh_path_now();
    prepend_path_dirs(&known_node_dirs());
}
pub(crate) async fn ensure_pnpm_for_dsh() -> Result<(), String> {
    expose_node_on_path();
    ensure_pnpm(&|line| eprintln!("[dsh] {line}")).await
}

#[tauri::command]
pub async fn chat_external_cli_install_info(
    state: tauri::State<'_, crate::state::AppState>,
    agent_id: String,
) -> Result<InstallInfo, String> {
    let def = get_agent_def(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;
    let path = crate::external_agents::spawn::resolve_binary(def).await;
    let version = path.as_deref().and_then(crate::external_agents::spawn::cached_cli_version);
    Ok(management_info(&state.http, def.into(), path, version).await)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallLogEvent {
    agent_id: String,
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
    verify_installed_cli(def.into(), crate::external_agents::spawn::agent_probe_command(def, bin), timeout).await
}

async fn finish_cli_install(
    def: &crate::external_agents::types::RuntimeAgentDef,
    emit: &impl Fn(String),
) -> Result<(), String> {
    if matches!(def.install.post_install, PostInstallStrategy::None) {
        return Ok(());
    }
    let Some(bin) = crate::external_agents::spawn::resolve_binary(def).await else {
        return Err(format!(
            "{} 安装命令已结束，但当前进程还找不到 {} 命令。请检查 CLI 路径设置，或重新打开 Kivio 后扫描。",
            def.name, def.bin
        ));
    };
    crate::external_agents::spawn::invalidate_probe_cache(&bin);
    let version = verify_cli_version(def, &bin, Duration::from_secs(20)).await?;
    emit(format!("{} {version} 启动验证通过", def.name));
    if matches!(def.install.post_install, PostInstallStrategy::DshProfile) {
        emit("正在初始化 Kivio dsh profile（首次需要下载插件）…".to_string());
        crate::external_agents::dsh_profile::ensure_profile_ready(&bin, None, None).await?;
        emit("dsh profile 已就绪".to_string());
    }
    Ok(())
}

/// 把一块输出拆成日志行。`\n` 和 `\r` 都算换行——npm 进度条只写 `\r`，按 `\n` 读
/// 会一直等，管道塞满后安装进程自己也写不动。
/// 跑 `install_command` 并把 stdout/stderr 按行流到 `external-cli-install` 事件。
///
/// ponytail: 不支持中途取消——安装是低频一次性动作，超时（300s）就自己结束。
#[tauri::command]
pub async fn chat_external_cli_install(app: AppHandle, agent_id: String) -> Result<(), String> {
    let def = get_agent_def(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;
    let spec = install_spec(&agent_id).ok_or_else(|| format!("未知外部 Agent: {agent_id}"))?;
    let resolved_path = crate::external_agents::spawn::resolve_binary(def).await;
    let emit_done = |success: bool| {
        let _ = app.emit(
            "external-cli-install",
            InstallLogEvent {
                agent_id: agent_id.clone(),
                line: None,
                done: true,
                success,
            },
        );
    };
    let emit = |line: String| {
        let _ = app.emit(
            "external-cli-install",
            InstallLogEvent {
                agent_id: agent_id.clone(),
                line: Some(line),
                done: false,
                success: false,
            },
        );
    };

    if let Err(error) = run_install_command(spec, resolved_path.as_deref(), &emit).await {
        emit_done(false);
        return Err(error);
    }
    let success = true;
    if success {
        emit("安装命令已结束，正在确认能否启动…".to_string());
        // npm -g 刚把 shims 写进 %APPDATA%\npm；装之前这个目录可能还不存在，
        // PATH 里也就没有它。不刷新的话紧接着的探测会说「未安装」。
        expose_node_on_path();
        if let Err(err) = finish_cli_install(def, &emit).await {
            emit(err.clone());
            emit_done(false);
            return Err(err);
        }
    }

    let _ = app.emit(
        "external-cli-install",
        InstallLogEvent {
            agent_id,
            line: None,
            done: true,
            success,
        },
    );
    if success {
        Ok(())
    } else {
        Err("安装或更新命令执行失败".to_string())
    }
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
    fn antigravity_uses_official_native_installers_and_self_update() {
        let spec = install_spec("antigravity").unwrap();
        assert!(spec.npm_package.is_none());
        assert!(install_plan(&spec, HostPlatform::Unix)
            .unwrap()
            .display
            .contains("https://antigravity.google/cli/install.sh"));
        assert!(install_plan(&spec, HostPlatform::Windows)
            .unwrap()
            .display
            .contains("https://antigravity.google/cli/install.ps1"));
        assert_eq!(spec.update, UpdateStrategy::Command(&["update"]));
        assert_eq!(spec.config_dir, Some(".gemini/antigravity-cli"));
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
    fn installed_clis_use_their_own_update_commands() {
        let cases: &[(&str, &[&str])] = &[
            ("claude", &["update"]),
            ("codex", &["update"]),
            ("opencode", &["upgrade"]),
            ("pi", &["update", "--self"]),
            ("grok", &["update"]),
            ("hermes", &["update"]),
        ];
        for (agent_id, expected_args) in cases {
            let spec = install_spec(agent_id).unwrap();
            let path = PathBuf::from(format!("/custom/bin/{agent_id}"));
            let plan = update_plan(spec, &path, HostPlatform::Unix).unwrap();
            assert_eq!(plan.program, path.to_string_lossy());
            assert_eq!(plan.args, *expected_args, "{agent_id}");
        }
    }

    #[test]
    fn kimi_and_gemini_updates_preserve_recognized_install_source() {
        let kimi = install_spec("kimi").unwrap();
        let native = update_plan(
            kimi,
            Path::new("/Users/u/.kimi-code/bin/kimi"),
            HostPlatform::Unix,
        )
        .unwrap();
        assert_eq!(native.program, "bash");
        assert!(native
            .display
            .contains("code.kimi.com/kimi-code/install.sh"));

        let npm = update_plan(
            kimi,
            Path::new("/prefix/lib/node_modules/@moonshot-ai/kimi-code/dist/main.mjs"),
            HostPlatform::Unix,
        )
        .unwrap();
        assert_eq!(npm.program, "npm");
        assert!(npm
            .args
            .contains(&"@moonshot-ai/kimi-code@latest".to_string()));

        let gemini = install_spec("gemini").unwrap();
        let brew = update_plan(
            gemini,
            Path::new("/opt/homebrew/Cellar/gemini-cli/1.2.3/bin/gemini"),
            HostPlatform::Unix,
        )
        .unwrap();
        assert_eq!(brew.display, "brew upgrade gemini-cli");
        assert!(
            update_plan(gemini, Path::new("/custom/bin/gemini"), HostPlatform::Unix,).is_none()
        );
    }

    #[test]
    fn install_commands_follow_official_platform_recipes() {
        let kimi = install_spec("kimi").unwrap();
        assert!(install_plan(&kimi, HostPlatform::Unix)
            .unwrap()
            .display
            .contains("code.kimi.com/kimi-code/install.sh"));
        assert_eq!(
            install_plan(&kimi, HostPlatform::Windows).unwrap().program,
            "powershell.exe"
        );

        let grok = install_spec("grok").unwrap();
        assert!(install_plan(&grok, HostPlatform::Unix)
            .unwrap()
            .display
            .contains("x.ai/cli/install.sh"));
        let grok_win = install_plan(&grok, HostPlatform::Windows).unwrap();
        assert_eq!(grok_win.program, "powershell.exe");
        assert!(grok_win.display.contains("x.ai/cli/install.ps1"));

        let pi_spec = install_spec("pi").unwrap();
        for platform in [HostPlatform::Unix, HostPlatform::Windows] {
            let pi = install_plan(pi_spec, platform).unwrap();
            assert!(pi.args.contains(&"--ignore-scripts".to_string()));
            assert!(
                pi.args.contains(&"--engine-strict".to_string()),
                "Pi must reject unsupported Node versions instead of reporting a broken install as successful"
            );
        }
        assert_eq!(pi_spec.post_install, PostInstallStrategy::VerifyVersion);

        let hermes = install_spec("hermes").unwrap();
        assert!(install_plan(&hermes, HostPlatform::Unix)
            .unwrap()
            .display
            .contains("hermes-agent.nousresearch.com/install.sh"));
        assert!(hermes.pypi_package.is_none());
        assert_eq!(
            extract_semver("Hermes Agent v0.21.3 (v2026.9.14)").as_deref(),
            Some("0.21.3")
        );
        assert_eq!(
            hermes_version_from_release(&serde_json::json!({
                "tag_name": "v2026.9.14",
                "name": "Hermes Agent v0.21.3 (v2026.9.14)"
            }))
            .as_deref(),
            Some("0.21.3")
        );
        assert_eq!(
            hermes_version_from_release(&serde_json::json!({
                "tag_name": "v2026.9.14"
            })),
            None
        );
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
    fn dsh_install_is_only_the_official_cli_package() {
        let spec = install_spec("dsh").unwrap();
        let install = install_plan(&spec, HostPlatform::Windows).unwrap();
        assert_eq!(install.program, "npm.cmd");
        assert!(install
            .args
            .contains(&"@deepseek-ai/dsh@latest".to_string()));
        assert!(install
            .args
            .contains(&"--registry=https://registry.npmjs.org/".to_string()));
        assert!(
            !install
                .args
                .iter()
                .any(|arg| arg.contains("cordis-plugin-group")
                    || arg.contains("cordis-plugin-loader")
                    || arg == "@deepseek-ai/cordis@latest"),
            "额外 -g peer 会破坏 dsh 启动: {:?}",
            install.args
        );

        let update = update_plan(
            spec,
            Path::new(r"C:\Users\u\AppData\Roaming\npm\dsh.cmd"),
            HostPlatform::Windows,
        )
        .unwrap();
        assert!(update.args.contains(&"@deepseek-ai/dsh@latest".to_string()));
        assert!(!update
            .args
            .iter()
            .any(|arg| arg.contains("cordis-plugin-group")));
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
    fn parse_node_version_reads_common_version_lines() {
        assert_eq!(parse_node_version("v24.13.0"), Some((24, 13, 0)));
        assert_eq!(parse_node_version("22.19.1"), Some((22, 19, 1)));
        assert_eq!(parse_node_version("v22.19.0-pre"), Some((22, 19, 0)));
        assert_eq!(parse_node_version("not a version"), None);
    }

    #[test]
    fn dsh_node_engine_matches_official_range() {
        assert!(!node_meets_dsh((18, 20, 0)));
        assert!(!node_meets_dsh((20, 19, 0)));
        assert!(!node_meets_dsh((22, 18, 0)));
        assert!(node_meets_dsh((22, 19, 0)));
        assert!(!node_meets_dsh((23, 0, 0)));
        assert!(node_meets_dsh((24, 0, 0)));
        assert!(node_meets_dsh((25, 1, 0)));
    }

    #[test]
    fn blank_machine_errors_point_at_the_download_page() {
        let dsh = missing_node_message(true);
        assert!(dsh.contains("nodejs.org"));
        assert!(dsh.contains("22.19"));
        let other = missing_node_message(false);
        assert!(other.contains("nodejs.org"));
        assert!(dsh_node_too_old_message("v18.20.0").contains("v18.20.0"));
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
