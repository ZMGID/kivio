use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::time::timeout;

use crate::external_agents::types::{AgentHomeStrategy, AgentLaunchPolicy, RuntimeAgentDef};
use crate::proc::NoConsoleWindow;

pub use super::process_io::*;
#[cfg(test)]
use super::process_io::accumulate_tail;

pub struct SpawnedAgent {
    pub child: AgentChild,
    pub resolved_bin: PathBuf,
}

/// 构造一个拉起外部 CLI 的 `Command`：等价于 `Command::new(program)` 但**已剥离父会话身份变量**。
///
/// 新增拉起 CLI 的代码请一律用它而不是 `Command::new`——忘记剥离不会编译报错、
/// 也不会立刻出错，只在「Kivio 从某个 agent 里启动」这种特定场景下才炸，极难排查。
fn command_for_program(program: &OsStr) -> Command {
    if let Some(target) = crate::external_agents::wsl::parse_wsl_target(Path::new(program)) {
        return crate::external_agents::wsl::wsl_exec_command(&target);
    }
    Command::new(program)
}

fn apply_env_for_cli(command: &mut Command, program: &Path, key: &str, value: String) {
    let value = crate::external_agents::wsl::env_value_for_cli(program, key, value);
    command.env(key, value);
}

fn cli_bin_stem(bin: &Path) -> Option<String> {
    if let Some(target) = crate::external_agents::wsl::parse_wsl_target(bin) {
        return Path::new(&target.linux_path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_string);
    }
    bin.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
}

fn is_codex_cli_bin(bin: &Path) -> bool {
    cli_bin_stem(bin).is_some_and(|stem| {
        stem.eq_ignore_ascii_case("codex") || stem.eq_ignore_ascii_case("codex-app-server")
    })
}

/// Compatibility adapter for call sites that only have a binary path. Agent-aware
/// orchestration must use `RuntimeAgentDef::launch` instead of inferring policy here.
fn compatibility_launch_policy_for_bin(bin: &Path) -> AgentLaunchPolicy {
    if is_codex_cli_bin(bin) {
        AgentLaunchPolicy::WSL_SHARED_CODEX_HOME
    } else {
        AgentLaunchPolicy::DEFAULT
    }
}

/// WSL Codex 在没有私有 `CODEX_HOME` 时改读用户那份 `~/.codex`，否则会话写进 Linux 家目录，
/// Windows 上的 Codex CLI / TUI 看不见。
fn extra_codex_home_for_spawn(
    cli_bin: &Path,
    home_strategy: AgentHomeStrategy,
    already_has_codex_home: bool,
) -> Option<PathBuf> {
    if already_has_codex_home {
        return None;
    }
    if home_strategy != AgentHomeStrategy::WslSharedCodexHome {
        return None;
    }
    crate::external_agents::provider_profile::wsl_shared_codex_home(cli_bin)
}

fn finish_codex_home(
    command: &mut Command,
    program: &Path,
    launch: AgentLaunchPolicy,
    already_has_codex_home: bool,
) {
    if already_has_codex_home {
        crate::external_agents::provider_profile::ensure_private_codex_sessions_shared();
        return;
    }
    if let Some(home) = extra_codex_home_for_spawn(program, launch.home, false) {
        apply_env_for_cli(
            command,
            program,
            "CODEX_HOME",
            home.to_string_lossy().into_owned(),
        );
    }
}

pub fn cli_command(program: impl AsRef<OsStr>) -> Command {
    let program = program.as_ref();
    let mut command = command_for_program(program);
    #[cfg(unix)]
    command.process_group(0);
    strip_parent_session_env(&mut command);
    // 设置页里针对这个 CLI 填的环境变量覆盖（`ANTHROPIC_BASE_URL` 之类），按二进制名反查。
    // 必须用原始路径（UNC 的 file_stem 仍是 `claude`），不能用改写后的 `wsl.exe`。
    let extra = crate::external_agents::overrides::env_for_bin(Path::new(program));
    let has_codex_home = extra.contains_key("CODEX_HOME");
    for (key, value) in extra {
        apply_env_for_cli(&mut command, Path::new(program), &key, value);
    }
    let launch = compatibility_launch_policy_for_bin(Path::new(program));
    finish_codex_home(&mut command, Path::new(program), launch, has_codex_home);
    command
}

/// `cli_command` + 该 agent 的静态 `def.env`。有 `def` 在手时用它，覆盖按 id 精确取。
///
/// 探测（version / auth / 列模型）也要带上环境变量覆盖：用户配了 `ANTHROPIC_BASE_URL` 指向中转，
/// 只在跑轮次时注入的话，认证探测仍打官方端点，设置页会显示「未认证」而实际能用。
pub fn agent_cli_command(def: &RuntimeAgentDef, program: impl AsRef<OsStr>) -> Command {
    let program = program.as_ref();
    let mut command = command_for_program(program);
    #[cfg(unix)]
    command.process_group(0);
    strip_parent_session_env(&mut command);
    let mut has_codex_home = false;
    for (key, value) in def.env {
        has_codex_home |= *key == "CODEX_HOME";
        apply_env_for_cli(&mut command, Path::new(program), key, value.to_string());
    }
    for (key, value) in crate::external_agents::overrides::env_for(def.id) {
        has_codex_home |= key == "CODEX_HOME";
        apply_env_for_cli(&mut command, Path::new(program), &key, value);
    }
    finish_codex_home(&mut command, Path::new(program), def.launch, has_codex_home);
    command
}

/// Read-only discovery must not start Antigravity's detached self-updater.
/// CREATE_NO_WINDOW only controls our direct child, not a CLI's detached helpers.
/// Apply the official opt-out to probes only; normal CLI runs and updates retain
/// their configured behavior. Other CLIs ignore this Antigravity-specific key.
/// https://antigravity.google/docs/cli/troubleshooting/
fn configure_probe_command(command: &mut Command) {
    command
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "true")
        .stdin(Stdio::null())
        .no_console_window()
        .kill_on_drop(true);
}

pub fn cli_probe_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = cli_command(program);
    configure_probe_command(&mut command);
    command
}

pub fn agent_probe_command(def: &RuntimeAgentDef, program: impl AsRef<OsStr>) -> Command {
    let mut command = agent_cli_command(def, program);
    configure_probe_command(&mut command);
    command
}

include!("spawn_probe.rs");

pub async fn resolve_binary(def: &RuntimeAgentDef) -> Option<PathBuf> {
    resolve_binary_configured(def, crate::external_agents::overrides::custom_path(def.id)).await
}

pub async fn spawn_agent(
    def: &RuntimeAgentDef,
    resolved_bin: &Path,
    args: &[String],
    cwd: &Path,
    extra_env: &HashMap<String, String>,
) -> Result<SpawnedAgent, String> {
    let mut command = agent_cli_command(def, resolved_bin);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .no_console_window()
        .kill_on_drop(true);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("spawn {}: {e}", def.id))?;
    Ok(SpawnedAgent {
        child: AgentChild::from(child),
        resolved_bin: resolved_bin.to_path_buf(),
    })
}

/// 一行 claude stream-json `user` 消息（含结尾换行）。
///
/// 一次性路径（`write_prompt_stdin`）与常驻会话（`session/claude_stream.rs`）共用同一份构造，
/// 不写第二份（spec 第 2 条）：「slash 命令走裸字符串」与「图片块的白名单/形状」这两条语义
/// 一旦分叉，两条路的行为差异会极难查。
pub fn stream_json_user_line(
    prompt: &str,
    images: &[crate::external_agents::attachments::ImageBlock],
) -> Result<String, String> {
    let line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": stream_json_user_content(prompt, images)
        },
        "parent_tool_use_id": null
    });
    let mut payload = serde_json::to_string(&line).map_err(|e| e.to_string())?;
    payload.push('\n');
    Ok(payload)
}

/// Minimal stdin write to elicit Claude `system/init` during slash-command probing.
pub async fn write_probe_stdin(child: &mut Child) -> Result<(), String> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "stdin unavailable".to_string())?;
    let mut stdin = stdin;
    let line = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": "."
        },
        "parent_tool_use_id": null
    });
    let mut payload = serde_json::to_string(&line).map_err(|e| e.to_string())?;
    payload.push('\n');
    stdin
        .write_all(payload.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn stream_json_user_content(
    prompt: &str,
    images: &[crate::external_agents::attachments::ImageBlock],
) -> serde_json::Value {
    if prompt.trim_start().starts_with('/') {
        serde_json::Value::String(prompt.to_string())
    } else {
        // Anthropic content array: text block first, then a base64 image block per attached image.
        let mut content = vec![serde_json::json!({ "type": "text", "text": prompt })];
        for img in images {
            content.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": img.mime,
                    "data": img.data_base64,
                },
            }));
        }
        serde_json::Value::Array(content)
    }
}

pub fn parse_json_line(line: &str) -> Option<serde_json::Value> {
    serde_json::from_str(line.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[tokio::test]
    async fn discovery_reads_models_without_starting_updater_but_normal_cli_can_update() {
        use crate::external_agents::defs::antigravity::ANTIGRAVITY_AGENT_DEF;

        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("fake cli.cmd");
        let updater_marker = dir.path().join("updater-started");
        std::fs::write(
            &bin,
            "@echo off\r\nif not \"%AGY_CLI_DISABLE_AUTO_UPDATE%\"==\"true\" echo started>\"%~dp0updater-started\"\r\necho model-one\tModel One\r\n",
        )
        .unwrap();

        // Exercise both executable discovery and the model/version/auth probe
        // factory with a Windows batch shim, including a path containing spaces.
        for mut command in [
            cli_probe_command(&bin),
            agent_probe_command(&ANTIGRAVITY_AGENT_DEF, &bin),
        ] {
            let output = command.arg("models").output().await.unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "model-one\tModel One"
            );
            assert!(!updater_marker.exists(), "discovery started an updater");
        }

        let output = cli_command(&bin)
            .env_remove("AGY_CLI_DISABLE_AUTO_UPDATE")
            .no_console_window()
            .arg("models")
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        assert!(
            updater_marker.exists(),
            "normal CLI update behavior changed"
        );
    }

    /// `cli_command` 必须剥掉父会话身份变量，否则 Kivio 从 Claude Code 内启动时，
    /// 这些变量会继承进来再泄漏给 CLI 子进程，子进程以为自己嵌套在别的会话里而拒绝启动。
    ///
    /// 这里断言的是**实际子进程看到的 env**（跑一个 `env` / `printenv` 子进程读回来），
    /// 而不是 `Command` 的内部状态——后者测了等于没测。
    #[tokio::test]
    async fn cli_command_strips_parent_session_env_from_the_child() {
        // 先在本进程设上，模拟「Kivio 被 Claude Code 拉起」。
        std::env::set_var("CLAUDECODE", "1");
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "cli");
        std::env::set_var("CLAUDE_AGENT_SDK_VERSION", "0.0.0");
        // 宿主代管凭据标记：真机验收时正是这两个让 claude 报 "Not logged in"。
        std::env::set_var("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1");
        std::env::set_var("CLAUDE_CODE_HOST_AUTH_ENV_VAR", "ANTHROPIC_AUTH_TOKEN");
        // 对照组：这两个**不该**被剥——前者是用户配置的凭据、后者是自定义端点，
        // 剥了子进程就真的没法认证了。剥得过头与剥得不够同样是 bug。
        std::env::set_var("KIVIO_SPAWN_TEST_KEEP", "keep-me");
        std::env::set_var("ANTHROPIC_AUTH_TOKEN", "sk-test-token");

        let out = if cfg!(windows) {
            let mut cmd = cli_command("cmd.exe");
            cmd.args(["/C", "set"]);
            cmd.output().await.expect("run cmd set")
        } else {
            cli_command("/usr/bin/env").output().await.expect("run env")
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let has = |key: &str| {
            text.lines()
                .any(|line| line.split_once('=').is_some_and(|(k, _)| k == key))
        };

        assert!(!has("CLAUDECODE"), "CLAUDECODE 泄漏给了子进程");
        assert!(
            !has("CLAUDE_CODE_ENTRYPOINT"),
            "CLAUDE_CODE_ENTRYPOINT 泄漏"
        );
        assert!(
            !has("CLAUDE_AGENT_SDK_VERSION"),
            "CLAUDE_AGENT_SDK_VERSION 泄漏"
        );
        // 这两条是真机回归的锚点：泄漏它们会让 claude 以为凭据由宿主代管、
        // 从而不去读 ~/.claude/settings.json 的 env，报 "Not logged in"。
        assert!(
            !has("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"),
            "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST 泄漏 → claude 会报 Not logged in"
        );
        assert!(
            !has("CLAUDE_CODE_HOST_AUTH_ENV_VAR"),
            "CLAUDE_CODE_HOST_AUTH_ENV_VAR 泄漏 → claude 会报 Not logged in"
        );
        assert!(
            has("KIVIO_SPAWN_TEST_KEEP"),
            "剥得过头了：非会话身份的变量也被清掉，子进程会丢失凭据/用户配置"
        );
        assert!(
            has("ANTHROPIC_AUTH_TOKEN"),
            "ANTHROPIC_AUTH_TOKEN 被误剥——那是用户配置的凭据，子进程要用它认证"
        );

        std::env::remove_var("CLAUDECODE");
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
        std::env::remove_var("CLAUDE_AGENT_SDK_VERSION");
        std::env::remove_var("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST");
        std::env::remove_var("CLAUDE_CODE_HOST_AUTH_ENV_VAR");
        std::env::remove_var("KIVIO_SPAWN_TEST_KEEP");
        std::env::remove_var("ANTHROPIC_AUTH_TOKEN");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_cli_owner_stops_descendant_process() {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut command = cli_command("/bin/sh");
        command
            .args(["-c", "sleep 60 & echo $!; wait"])
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let mut child = AgentChild::from(command.spawn().unwrap());
        let pid = child.id().unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).await.unwrap();
        let descendant: i32 = line.trim().parse().unwrap();
        // Check the actual spawn boundary, not a helper's configuration.
        assert_eq!(unsafe { libc::getpgid(pid as i32) }, pid as i32);
        drop(child);
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let status = std::process::Command::new("ps")
                    .args(["-p", &descendant.to_string(), "-o", "stat="])
                    .output()
                    .unwrap();
                let state = String::from_utf8_lossy(&status.stdout);
                if state.trim().is_empty() || state.trim().starts_with('Z') {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await;
        if stopped.is_err() {
            unsafe {
                libc::kill(descendant, libc::SIGKILL);
            }
        }
        stopped.expect("dropping the owner must stop CLI descendants");
    }

    #[test]
    fn cli_command_rewrites_wsl_unc_to_wsl_exe() {
        let cmd = cli_command(r"\\wsl$\Ubuntu\usr\bin\claude");
        let std = cmd.as_std();
        assert_eq!(std.get_program(), std::ffi::OsStr::new("wsl.exe"));
        let args: Vec<String> = std
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..5], ["-d", "Ubuntu", "-e", "/bin/bash", "-c"]);
        assert_eq!(args[6], "/usr/bin/claude");
        // 环境覆盖必须仍按原始二进制名 `claude` 查找，不能变成 `wsl`。
        // UNC 在 Unix Path 里是单个分量，要用 linux 路径取 stem。
        assert_eq!(
            cli_bin_stem(Path::new(r"\\wsl$\Ubuntu\usr\bin\claude")).as_deref(),
            Some("claude")
        );
    }

    #[test]
    fn extra_codex_home_for_spawn_only_fills_wsl_without_private_home() {
        let wsl = PathBuf::from(r"\\wsl$\Ubuntu\usr\bin\codex");
        let win = PathBuf::from(r"C:\codex.exe");
        let claude = PathBuf::from(r"\\wsl$\Ubuntu\usr\bin\claude");
        assert!(
            extra_codex_home_for_spawn(&win, AgentHomeStrategy::WslSharedCodexHome, false)
                .is_none()
        );
        assert!(
            extra_codex_home_for_spawn(&wsl, AgentHomeStrategy::WslSharedCodexHome, true).is_none()
        );
        assert!(extra_codex_home_for_spawn(&claude, AgentHomeStrategy::None, false).is_none());
        let home = extra_codex_home_for_spawn(&wsl, AgentHomeStrategy::WslSharedCodexHome, false)
            .expect("wsl codex shares host home");
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
    fn agent_cli_command_uses_the_definition_home_policy_not_the_binary_name() {
        use crate::external_agents::defs::claude::CLAUDE_AGENT_DEF;
        use crate::external_agents::defs::codex::CODEX_AGENT_DEF;

        let codex_bin = PathBuf::from(r"\\wsl$\Ubuntu\usr\bin\codex");
        let codex = agent_cli_command(&CODEX_AGENT_DEF, &codex_bin);
        assert!(codex
            .as_std()
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME" && value.is_some()));

        // A renamed/wrapped executable must not leak Codex policy into another
        // agent. Static launch behavior belongs to RuntimeAgentDef.
        let claude = agent_cli_command(&CLAUDE_AGENT_DEF, &codex_bin);
        assert!(!claude
            .as_std()
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME" && value.is_some()));
    }

    #[test]
    fn cli_command_keeps_binary_detection_inside_the_compatibility_adapter() {
        let codex = cli_command(r"\\wsl$\Ubuntu\usr\bin\codex");
        assert!(codex
            .as_std()
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME" && value.is_some()));

        let claude = cli_command(r"\\wsl$\Ubuntu\usr\bin\claude");
        assert!(!claude
            .as_std()
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME" && value.is_some()));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn probing_a_wsl_unc_when_wsl_is_broken_is_not_installed() {
        let fake = std::path::PathBuf::from(r"\\wsl$\Ubuntu\usr\bin\claude");
        let started = std::time::Instant::now();
        let (ok, _) = probe_executable(&fake, &["--version"]).await;
        assert!(!ok, "wsl.exe MSI stub must not count as an installed CLI");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(8),
            "WSL probe hung for {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn stream_json_user_content_uses_string_for_slash_commands() {
        let slash = stream_json_user_content("/compact", &[]);
        assert_eq!(slash, serde_json::json!("/compact"));
        let text = stream_json_user_content("hello", &[]);
        assert_eq!(
            text,
            serde_json::json!([{ "type": "text", "text": "hello" }])
        );
    }

    #[test]
    fn stream_json_user_content_appends_image_blocks() {
        let img = crate::external_agents::attachments::ImageBlock {
            data_base64: "AAAA".to_string(),
            mime: "image/png".to_string(),
            path: std::path::PathBuf::from("/tmp/a.png"),
        };
        let content = stream_json_user_content("look", std::slice::from_ref(&img));
        let arr = content.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], serde_json::json!("text"));
        assert_eq!(arr[1]["type"], serde_json::json!("image"));
        assert_eq!(
            arr[1]["source"]["media_type"],
            serde_json::json!("image/png")
        );
        assert_eq!(arr[1]["source"]["data"], serde_json::json!("AAAA"));
    }

    #[test]
    fn stream_json_slash_ignores_images() {
        let img = crate::external_agents::attachments::ImageBlock {
            data_base64: "AAAA".to_string(),
            mime: "image/png".to_string(),
            path: std::path::PathBuf::from("/tmp/a.png"),
        };
        let content = stream_json_user_content("/compact", std::slice::from_ref(&img));
        assert_eq!(content, serde_json::json!("/compact"));
    }

    #[tokio::test]
    async fn accumulate_tail_keeps_only_the_tail_and_finishes_on_eof() {
        // 200 numbered lines far exceeding an 8-char cap → only the last chars survive, and the
        // task completes once the in-memory reader hits EOF.
        let mut input = String::new();
        for i in 0..200 {
            input.push_str(&format!("line{i}\n"));
        }
        let tail = accumulate_tail(input.as_bytes(), 8).await;
        assert!(
            tail.chars().count() <= 8,
            "tail should be capped, got {tail:?}"
        );
        assert!(
            tail.ends_with("line199"),
            "tail should keep the end, got {tail:?}"
        );
    }

    #[tokio::test]
    async fn accumulate_tail_drops_blank_lines() {
        let tail = accumulate_tail("\n\nhello\n\nworld\n".as_bytes(), 8192).await;
        assert_eq!(tail, "hello\nworld");
    }

    /// 版本行提取：首个非空行、原样保留（不解析结构）、超长封顶。
    /// 各 CLI 的 `--version` 形态差异极大，任何「解析成 semver」的尝试都会在某家上错。
    #[test]
    fn version_line_is_the_first_non_empty_line_verbatim() {
        // claude 2.1.220 本机实测输出。
        assert_eq!(
            first_version_line("2.1.220 (Claude Code)\n").as_deref(),
            Some("2.1.220 (Claude Code)")
        );
        // 前导空行 / banner 换行不影响。
        assert_eq!(
            first_version_line("\n\n  codex-cli 0.145.0  \nextra\n").as_deref(),
            Some("codex-cli 0.145.0")
        );
        // 没有输出（未登录的 CLI 把话都说在 stderr 上）→ None。
        assert_eq!(first_version_line(""), None);
        assert_eq!(first_version_line("\n \n"), None);
        // 坏 shim 可能把一整篇日志打在一行里 → 封顶，不把缓存撑爆。
        let long = "x".repeat(5000);
        assert_eq!(
            first_version_line(&long).map(|v| v.chars().count()),
            Some(VERSION_LINE_CAP)
        );
    }

    #[tokio::test]
    async fn join_stderr_tail_returns_after_killing_a_live_child() {
        use crate::proc::NoConsoleWindow;
        let mut command = if cfg!(windows) {
            let mut command = Command::new("ping");
            command.args(["-t", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        };
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .no_console_window()
            .spawn()
            .expect("spawn long-lived child");
        let tail = spawn_stderr_tail(child.stderr.take());
        let started = std::time::Instant::now();
        let _ = join_stderr_tail(&mut child, tail).await;
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "join_stderr_tail blocked for {:?}",
            started.elapsed()
        );
    }
}

// 整模块只在 unix 下编译：每个用例都靠写 shell 脚本 + chmod 造「能跑的 / 坏掉的」二进制，
// Windows 上 `std::os::unix` 与 `Permissions::from_mode` 都不存在 —— 不 gate 会让整个
// lib test 目标在 Windows 编译失败，连带其它模块的单测一个都跑不了。
#[cfg(all(test, unix))]
mod probe_tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// 造一批「坏 shim」样本。分类依据是本机实测（macOS）：
    ///   丢执行权限 → EACCES、空文件 → ENOEXEC、断链/不存在 → ENOENT，
    ///   而「装了但未登录」（exit 1）是 **spawn 成功**的，必须算存在。
    struct Fixtures(std::path::PathBuf);

    impl Fixtures {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "kivio-probe-{}-{}-{tag}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn script(&self, name: &str, body: &str) -> std::path::PathBuf {
            let path = self.0.join(name);
            fs::write(&path, body).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        fn unreadable(&self, name: &str) -> std::path::PathBuf {
            let path = self.script(name, "#!/bin/sh\necho ok\n");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
            path
        }

        fn empty_exec(&self, name: &str) -> std::path::PathBuf {
            let path = self.0.join(name);
            fs::write(&path, "").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            path
        }
    }

    impl Drop for Fixtures {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.0.join("noexec"), fs::Permissions::from_mode(0o755));
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn probe_accepts_a_working_binary() {
        let f = Fixtures::new("ok");
        let good = f.script("good", "#!/bin/sh\necho v1.2.3\n");
        let (ok, version) = probe_executable(&good, &["--version"]).await;
        assert!(ok);
        // 探活顺手把版本行接住了（此前 stdout 丢进 null，版本门控只能另起一次进程）。
        assert_eq!(version.as_deref(), Some("v1.2.3"));
    }

    /// **最重要的一条**：装了但没登录的 CLI，`--version` 非零退出——
    /// 必须仍算「存在」。只认零退出码会把这类 CLI 误判成未安装，
    /// 比原来「信 which 首行」的 bug 更糟。
    #[tokio::test]
    async fn probe_accepts_nonzero_exit_because_unauthenticated_clis_exit_nonzero() {
        let f = Fixtures::new("notauth");
        let notauth = f.script("notauth", "#!/bin/sh\necho 'Not logged in' >&2\nexit 1\n");
        let (ok, version) = probe_executable(&notauth, &["--version"]).await;
        assert!(ok, "非零退出被当成不存在——未登录的 CLI 会被误判为未安装");
        // 版本拿不到（输出全在 stderr）时必须是 None，且**不影响**「存在」的判定。
        assert_eq!(version, None);
    }

    #[tokio::test]
    async fn probe_rejects_broken_shims() {
        let f = Fixtures::new("broken");
        // EACCES：丢了执行权限
        assert!(
            !probe_executable(&f.unreadable("noexec"), &["--version"])
                .await
                .0
        );
        // ENOENT：路径根本不存在（等价于断掉的 symlink）
        assert!(
            !probe_executable(&f.0.join("missing"), &["--version"])
                .await
                .0
        );
    }

    /// 挂了执行位的**空文件**在 unix 上是能跑的：内核回退到 `/bin/sh`，
    /// 读到零条命令、exit 0（本机 `./empty; echo $?` 实测为 0）。
    /// 所以它**不该**被判成不存在——这与「非零退出码算存在」是同一条原则的延伸：
    /// 只有 spawn 阶段失败才算不存在，能起来的一律放行。
    ///
    /// （注：用 Python 的 subprocess 试会看到 ENOEXEC，那是 CPython 自己先拦了；
    /// 内核/Rust 的实际行为以本测试为准。）
    #[tokio::test]
    async fn probe_accepts_empty_executable_because_the_kernel_runs_it() {
        let f = Fixtures::new("emptyexec");
        assert!(
            probe_executable(&f.empty_exec("empty"), &["--version"])
                .await
                .0
        );
    }

    #[tokio::test]
    async fn probe_treats_a_hanging_binary_as_present() {
        // 起来了就说明文件可执行，只是这个 CLI 的 --version 慢/挂住——不能判成不存在。
        let f = Fixtures::new("slow");
        let slow = f.script("slow", "#!/bin/sh\nsleep 30\n");
        let started = std::time::Instant::now();
        assert!(probe_executable(&slow, &["--version"]).await.0);
        assert!(
            started.elapsed() < PROBE_TIMEOUT + Duration::from_secs(2),
            "超时守卫没生效，探活被挂死"
        );
    }

    /// 本机真实 CLI 必须仍能被解析到——这条比上面的单测更重要：
    /// 探活写得太严会把好的 CLI 判死，而那种回归单测抓不到。
    /// 同时打印耗时，用于评估热路径开销（spec 第 9 条）。
    /// `which_all` 必须返回**全部**同名候选，不能只取第一行——这是 G3b 的核心：
    /// PATH 上靠前的那个可能是坏 shim，只看第一个就等于没修。
    /// （单测 `probe_*` 只覆盖单个候选的判定，覆盖不到这个遍历行为。）
    #[tokio::test]
    async fn which_all_returns_every_candidate_not_just_the_first() {
        let f = Fixtures::new("multi");
        // 造两个同名的 fake CLI，分别在两个目录里，都挂进 PATH。
        let dir_a = f.0.join("a");
        let dir_b = f.0.join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        for dir in [&dir_a, &dir_b] {
            let p = dir.join("kivio-fake-cli");
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{}:{}:{original}", dir_a.display(), dir_b.display()),
        );

        let found = which_all("kivio-fake-cli").await;
        std::env::set_var("PATH", original);

        assert_eq!(
            found.len(),
            2,
            "只拿到 {} 个候选，应为 2（只取首行 = G3b 没生效）：{found:?}",
            found.len()
        );
        assert!(found[0].starts_with(&dir_a), "候选顺序应遵循 PATH");
    }

    /// 端到端：靠前的候选是坏 shim 时，`resolve_binary` 必须跳过它选后面那个好的。
    #[tokio::test]
    async fn resolve_binary_skips_a_broken_shim_for_the_working_one() {
        let f = Fixtures::new("shim");
        let bad_dir = f.0.join("bad");
        let good_dir = f.0.join("good");
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::create_dir_all(&good_dir).unwrap();
        // 坏的：丢执行权限（spawn 时 EACCES）
        let bad = bad_dir.join("kivio-shim-cli");
        std::fs::write(&bad, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o000)).unwrap();
        // 好的
        let good = good_dir.join("kivio-shim-cli");
        std::fs::write(&good, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&good, std::fs::Permissions::from_mode(0o755)).unwrap();

        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            format!("{}:{}:{original}", bad_dir.display(), good_dir.display()),
        );

        let mut def = crate::external_agents::registry::AGENT_DEFS[0].clone();
        def.bin = "kivio-shim-cli";
        def.fallback_bins = &[];
        let resolved = resolve_binary(&def).await;

        std::env::set_var("PATH", original);
        let _ = std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o755));

        assert_eq!(
            resolved.as_deref(),
            Some(good.as_path()),
            "应跳过坏 shim 选到好的，实际 {resolved:?}"
        );
    }

    /// 缓存必须真的省掉子进程：`resolve_binary` 在热路径上，`run.rs` 的预算是 <500ms，
    /// 而 kimi/pi 这类 Node 包装脚本单次 `--version` 就要 300ms+。
    /// 这里用一个「每次被调用就往文件里追加一行」的假 CLI 来数实际启动次数。
    #[tokio::test]
    async fn probe_result_is_cached_so_the_hot_path_does_not_respawn() {
        let f = Fixtures::new("cache");
        let counter = f.0.join("calls");
        let bin = f.script(
            "countbin",
            &format!("#!/bin/sh\necho x >> {}\n", counter.display()),
        );
        let count = || {
            std::fs::read_to_string(&counter)
                .map(|s| s.lines().count())
                .unwrap_or(0)
        };

        assert!(probe_executable(&bin, &["--version"]).await.0);
        assert_eq!(count(), 1, "第一次应真的启动一次");

        // 走 resolve_binary 的缓存路径：同一个 key 命中后不得再起进程。
        let key = probe_cache_key(&bin).expect("metadata");
        PROBE_CACHE.lock().unwrap().insert(
            key.clone(),
            ProbeOutcome {
                executable: true,
                version: Some("9.9.9".to_string()),
            },
        );
        assert_eq!(
            PROBE_CACHE
                .lock()
                .unwrap()
                .get(&key)
                .map(|entry| entry.executable),
            Some(true),
            "缓存未写入"
        );
        assert_eq!(count(), 1, "缓存命中后不应再启动子进程");
        // 版本号也要能从同一条缓存读回来——版本门控靠它，且**不得**为此再起进程。
        assert_eq!(cached_cli_version(&bin).as_deref(), Some("9.9.9"));
        assert_eq!(count(), 1, "读版本号不应启动子进程");
    }

    #[tokio::test]
    #[ignore = "reads the real PATH on this machine"]
    async fn resolve_binary_probes_real_clis() {
        // 第二遍走缓存，必须显著快于第一遍——这是 <500ms 热路径预算的依据。
        for round in ["cold", "warm"] {
            eprintln!("--- {round} ---");
            for def in crate::external_agents::registry::AGENT_DEFS {
                let started = std::time::Instant::now();
                let resolved = resolve_binary(def).await;
                // 打印 bin + 全部 fallback 的候选总数（只打 def.bin 会误导：
                // 如 opencode 的 def.bin 是 `opencode-cli`，实际命中的是 fallback `opencode`）。
                let mut candidates = 0usize;
                for name in std::iter::once(def.bin).chain(def.fallback_bins.iter().copied()) {
                    candidates += which_all(name).await.len();
                }
                eprintln!(
                    "{:10} candidates={candidates} {:>5}ms -> {}",
                    def.id,
                    started.elapsed().as_millis(),
                    resolved
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(not installed)".to_string()),
                );
            }
        }
    }
}
