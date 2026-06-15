use std::path::PathBuf;

use serde_json::Value;
use tokio::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use super::{resolve_tool_existing_dir, NativeToolWorkspace};
use crate::settings::{CHAT_TOOL_MAX_TIMEOUT_MS, CHAT_TOOL_MIN_TIMEOUT_MS};

const COMMAND_DENYLIST: &[&str] = &[
    "sudo ",
    "sudo\n",
    "rm -rf /",
    "rm -rf /*",
    ":(){ :|:& };:",
    "mkfs.",
    "dd if=/dev/zero",
    "> /dev/sd",
];

const HOST_PYTHON_PACKAGE_INSTALL_PATTERNS: &[&str] = &[
    "pip install",
    "pip3 install",
    "python -m pip install",
    "python3 -m pip install",
    "uv pip install",
];

/// Dev servers and other long-running processes are spawned in the background.
const LONG_RUNNING_DEV_PATTERNS: &[&str] = &[
    "tauri dev",
    "npm run tauri dev",
    "npm run dev",
    "npm run dev:",
    "next dev",
    "nuxt dev",
    "webpack serve",
    "webpack-dev-server",
    "cargo watch",
    "flutter run",
    "expo start",
    "deno task dev",
];

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub async fn run_command(
    workspace: &NativeToolWorkspace,
    default_timeout_ms: u64,
    arguments: &Value,
) -> Result<String, String> {
    let command = arguments
        .get("command")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "run_command requires command".to_string())?;

    let lowered = command.to_ascii_lowercase();
    for denied in COMMAND_DENYLIST {
        if lowered.contains(denied) {
            return Err("command is blocked by safety policy".to_string());
        }
    }
    let allow_host_python_package_install = arguments
        .get("allow_host_python_package_install")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !allow_host_python_package_install {
        for denied in HOST_PYTHON_PACKAGE_INSTALL_PATTERNS {
            if lowered.contains(denied) {
                return Err(
                    "run_command cannot install Python packages or modify the host Python environment unless allow_host_python_package_install is true. Do not retry with variants — use run_python for sandboxed Python, or (if the user explicitly wants host installs) create/activate a venv and pass allow_host_python_package_install=true."
                        .to_string(),
                );
            }
        }
    } else if HOST_PYTHON_PACKAGE_INSTALL_PATTERNS
        .iter()
        .any(|pattern| lowered.contains(pattern))
    {
        if !lowered.contains("--user")
            && !lowered.contains("venv")
            && !lowered.contains(".venv")
            && !lowered.contains("virtualenv")
        {
            return Err(
                "Host Python package installs must target a user or virtual environment; add --user or run inside a venv."
                    .to_string(),
            );
        }
    }

    let explicit_cwd = arguments
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|path| !path.is_empty());

    let (command, cd_extracted) = normalize_run_command(command, explicit_cwd)?;

    let cwd = if let Some(cd_path) = cd_extracted.as_deref() {
        resolve_tool_existing_dir(workspace, Some(cd_path))?
    } else {
        resolve_command_cwd(arguments, workspace)?
    };

    if !cwd.is_dir() {
        return Err(format!(
            "Working directory is not a directory: {}",
            cwd.display()
        ));
    }

    let background = arguments
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| is_long_running_dev_command(&command));
    if background {
        return run_shell_command_background(&command, cwd).await;
    }

    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(default_timeout_ms)
        .clamp(CHAT_TOOL_MIN_TIMEOUT_MS, CHAT_TOOL_MAX_TIMEOUT_MS)
        .max(default_timeout_ms);

    let output = run_shell_command(&command, cwd, timeout_ms).await?;
    let formatted = format_command_output(&output);
    if let Some(code) = output.status_code {
        if code != 0 {
            return Err(formatted);
        }
    }
    Ok(formatted)
}

fn resolve_command_cwd(
    arguments: &Value,
    workspace: &NativeToolWorkspace,
) -> Result<PathBuf, String> {
    resolve_tool_existing_dir(workspace, arguments.get("cwd").and_then(|v| v.as_str()))
}

/// Reject fragile `cd ... &&` prefixes; auto-strip simple `cd foo &&` forms.
fn normalize_run_command(
    command: &str,
    explicit_cwd: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let Some((cd_path, rest)) = parse_leading_cd_prefix(command) else {
        return Ok((command.to_string(), None));
    };

    if explicit_cwd.is_some() {
        return Err(
            "run_command: do not combine the `cwd` parameter with `cd ... &&` in `command`. \
             Set `cwd` to the target directory and run only the remaining shell command."
                .to_string(),
        );
    }

    if cd_path.contains(' ') {
        return Err(format!(
            "run_command: paths with spaces must use the `cwd` parameter instead of `cd ... &&`.\n\
             Suggested cwd: {cd_path}\n\
             Suggested command: {rest}"
        ));
    }

    Ok((rest, Some(cd_path)))
}

fn parse_leading_cd_prefix(command: &str) -> Option<(String, String)> {
    let trimmed = command.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !lower.starts_with("cd ") {
        return None;
    }

    let after_cd = trimmed.get(3..)?.trim_start();
    let (path_part, rest) = find_cd_and_separator(after_cd)?;
    let cd_path = strip_shell_quotes(path_part.trim());
    let rest = rest.trim();
    if cd_path.is_empty() || rest.is_empty() {
        return None;
    }
    Some((cd_path, rest.to_string()))
}

fn find_cd_and_separator(command: &str) -> Option<(&str, &str)> {
    for pattern in [" && ", "&&"] {
        if let Some(idx) = command.find(pattern) {
            let path = command.get(..idx)?.trim();
            let rest = command.get(idx + pattern.len()..)?.trim();
            if !path.is_empty() && !rest.is_empty() {
                return Some((path, rest));
            }
        }
    }
    None
}

fn strip_shell_quotes(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 {
        if (value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\''))
        {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

fn is_long_running_dev_command(command: &str) -> bool {
    let lowered = command.to_ascii_lowercase();
    if LONG_RUNNING_DEV_PATTERNS
        .iter()
        .any(|pattern| lowered.contains(pattern))
    {
        return true;
    }

    if lowered.contains("vite build") || lowered.contains("vite preview") {
        return false;
    }

    lowered.starts_with("vite")
        || lowered.starts_with("npx vite")
        || lowered.contains(" npx vite")
        || lowered.contains("&& vite")
        || lowered.contains("; vite")
}

async fn run_shell_command_background(command: &str, cwd: PathBuf) -> Result<String, String> {
    let mut cmd = {
        #[cfg(target_os = "windows")]
        {
            let mut c = Command::new("cmd");
            // ponytail: raw_arg 而非 args(),绕开 Rust 对参数的 MSVC 转义。
            // 否则命令里的内部 " 会被转成 \",cmd.exe 不认 → python -c "..." 之类全坏。
            c.raw_arg("/C");
            c.raw_arg(command);
            c
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut c = Command::new("sh");
            c.args(["-c", command]);
            c
        }
    };
    cmd.current_dir(cwd.as_path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        const DETACHED_PROCESS: u32 = 0x00000008;
        cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
    }
    cmd.kill_on_drop(false);
    #[cfg(unix)]
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .map_err(|err| format!("Failed to start background command: {err}"))?;
    let pid = child
        .id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    Ok(format!(
        "background: true\npid: {pid}\ncwd: {}\ncommand: {command}\n\nLong-running dev server started in the background. It keeps running after this tool returns; check the app window or terminal output manually. Do not start the same dev server again unless you have stopped it first.\n",
        cwd.display()
    ))
}

#[derive(Debug)]
struct CommandOutput {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn run_shell_command(
    command: &str,
    cwd: PathBuf,
    timeout_ms: u64,
) -> Result<CommandOutput, String> {
    let mut cmd = {
        #[cfg(target_os = "windows")]
        {
            let mut c = Command::new("cmd");
            // ponytail: raw_arg 而非 args(),绕开 Rust 对参数的 MSVC 转义。
            // 否则命令里的内部 " 会被转成 \",cmd.exe 不认 → python -c "..." 之类全坏。
            c.raw_arg("/C");
            c.raw_arg(command);
            c
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut c = Command::new("sh");
            c.args(["-c", command]);
            c
        }
    };
    cmd.current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.kill_on_drop(true);
    #[cfg(target_os = "macos")]
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = cmd
        .spawn()
        .map_err(|err| format!("Failed to start command: {err}"))?;
    let child_pid = child.id();

    let result = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        terminate_command_group(child_pid);
        format!("Command timed out after {timeout_ms}ms")
    })?
    .map_err(|err| format!("Command failed: {err}"))?;

    Ok(CommandOutput {
        status_code: result.status.code(),
        stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
    })
}

#[cfg(target_os = "macos")]
fn terminate_command_group(child_pid: Option<u32>) {
    if let Some(pid) = child_pid {
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn terminate_command_group(_child_pid: Option<u32>) {}

fn format_command_output(output: &CommandOutput) -> String {
    let mut out = String::new();
    if let Some(code) = output.status_code {
        out.push_str(&format!("exit_code: {code}\n"));
    }
    if !output.stdout.is_empty() {
        out.push_str("stdout:\n");
        out.push_str(&output.stdout);
        if !output.stdout.ends_with('\n') {
            out.push('\n');
        }
    }
    if !output.stderr.is_empty() {
        out.push_str("stderr:\n");
        out.push_str(&output.stderr);
        if !output.stderr.ends_with('\n') {
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        out.push_str("(no output)\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_tools::user_home_dir;

    #[test]
    fn default_cwd_uses_first_workspace_root_when_configured() {
        let home = user_home_dir().expect("home should be available in tests");
        let root = home.join(".kivio-chat-test-root");
        std::fs::create_dir_all(&root).expect("mkdir");
        let args = serde_json::json!({ "command": "pwd" });
        let workspace = NativeToolWorkspace::global(&[root.to_string_lossy().into_owned()]);
        let cwd = resolve_command_cwd(&args, &workspace).expect("workspace root should resolve");

        assert_eq!(cwd, root);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn command_cwd_allows_temp_directory_outside_home() {
        let dir = std::env::temp_dir().join(format!("kivio_cmd_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let args = serde_json::json!({ "command": "pwd", "cwd": dir.to_string_lossy() });
        let workspace = NativeToolWorkspace::global(&[]);
        let cwd = resolve_command_cwd(&args, &workspace).expect("temp cwd should resolve");

        assert_eq!(
            cwd,
            std::fs::canonicalize(&dir).expect("canonical temp dir")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn format_command_output_includes_nonzero_exit_code() {
        let output = CommandOutput {
            status_code: Some(1),
            stdout: String::new(),
            stderr: "boom\n".to_string(),
        };

        let formatted = format_command_output(&output);

        assert!(formatted.contains("exit_code: 1"));
        assert!(formatted.contains("stderr:\nboom"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_run_command_preserves_embedded_quotes() {
        // 回归:内部双引号必须原样到达目标程序。
        // 旧实现 args(["/C", cmd]) 会把 " 转成 \",cmd.exe 不认 → python -c "..." 报
        // "unterminated string literal"。raw_arg 修复后应原样通过。
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let out = rt
            .block_on(run_shell_command(
                "python -c \"print(40 + 2)\"",
                std::env::temp_dir(),
                30_000,
            ))
            .expect("spawn should succeed");
        // python 不在 PATH 时跳过,不让本机环境决定测试成败。
        let unavailable = out.stderr.contains("not recognized")
            || out.stderr.to_lowercase().contains("cannot find")
            || out.stderr.contains("找不到");
        if !unavailable {
            assert!(
                out.stdout.contains("42"),
                "embedded quotes mangled? stdout={:?} stderr={:?}",
                out.stdout,
                out.stderr
            );
        }
    }

    #[test]
    fn normalize_run_command_rejects_cd_with_spaces() {
        let err = normalize_run_command("cd /Users/zmair/ZM database/foo && npm install", None)
            .expect_err("spaced cd path should require cwd");

        assert!(err.contains("Suggested cwd: /Users/zmair/ZM database/foo"));
        assert!(err.contains("Suggested command: npm install"));
    }

    #[test]
    fn normalize_run_command_rejects_cd_when_cwd_is_set() {
        let err = normalize_run_command("cd foo && npm install", Some("/tmp/project"))
            .expect_err("cd and cwd should conflict");

        assert!(err.contains("do not combine"));
    }

    #[test]
    fn normalize_run_command_strips_simple_cd_prefix() {
        let (command, cwd) =
            normalize_run_command("cd focus-pomodoro && npm install", None).expect("normalize");

        assert_eq!(command, "npm install");
        assert_eq!(cwd.as_deref(), Some("focus-pomodoro"));
    }

    #[test]
    fn is_long_running_dev_command_detects_common_dev_servers() {
        assert!(is_long_running_dev_command("npm run tauri dev"));
        assert!(is_long_running_dev_command("npx vite --port 5173"));
        assert!(!is_long_running_dev_command("npm run build"));
        assert!(!is_long_running_dev_command("vite build"));
    }

    #[tokio::test]
    async fn run_command_blocks_host_python_package_installs() {
        let err = run_command(
            &NativeToolWorkspace::global(&[]),
            1_000,
            &serde_json::json!({ "command": "python3 -m pip install matplotlib" }),
        )
        .await
        .expect_err("pip installs should be blocked");

        assert!(err.contains("allow_host_python_package_install"));
    }
}
