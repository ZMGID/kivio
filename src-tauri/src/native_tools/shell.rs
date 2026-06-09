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
                    "run_command cannot install Python packages or modify the host Python environment unless allow_host_python_package_install is true. Use run_python for sandboxed Python instead."
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

    let cwd = resolve_command_cwd(arguments, workspace)?;

    if !cwd.is_dir() {
        return Err(format!(
            "Working directory is not a directory: {}",
            cwd.display()
        ));
    }

    let timeout_ms = arguments
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(default_timeout_ms)
        .clamp(CHAT_TOOL_MIN_TIMEOUT_MS, CHAT_TOOL_MAX_TIMEOUT_MS)
        .max(default_timeout_ms);

    let output = run_shell_command(command, cwd, timeout_ms).await?;
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
            c.args(["/C", command]);
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
