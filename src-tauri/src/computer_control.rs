//! Setup only: agents use the installed CLIs through the existing shell + skills.
use std::{path::Path, process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::Mutex,
};

use crate::{proc::NoConsoleWindow, skills::SkillMeta};

static INSTALL_LOCK: Mutex<()> = Mutex::const_new(());

pub(crate) const CUA_MCP_SERVER_ID: &str = "computer-control-cua-driver";
pub(crate) const LEGACY_CUA_MCP_SERVER_ID: &str = "plugin-cua-driver";
pub(crate) const LEGACY_CUA_MCP_CONNECTOR_ID: &str = "plugin:cua-driver";

pub(crate) fn is_cua_mcp_server_id(id: &str) -> bool {
    id == CUA_MCP_SERVER_ID || id == LEGACY_CUA_MCP_SERVER_ID
}

pub(crate) fn mcp_server_ids_equivalent(left: &str, right: &str) -> bool {
    left == right || (is_cua_mcp_server_id(left) && is_cua_mcp_server_id(right))
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlTool {
    Cua,
    Playwright,
}

impl ControlTool {
    fn command(self) -> &'static str {
        match self {
            Self::Cua => "cua-driver",
            Self::Playwright => "playwright-cli",
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlToolStatus {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
}

#[derive(Deserialize)]
struct CuaUpdateStatus {
    current_version: Option<String>,
    latest_version: Option<String>,
    update_available: bool,
}

fn extract_version(output: &str) -> String {
    output
        .split_whitespace()
        .find_map(|part| {
            let candidate = part.trim_start_matches('v').trim_matches(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
            });
            let mut numbers = candidate.split('.');
            let valid = numbers
                .by_ref()
                .take(3)
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
            (valid && candidate.matches('.').count() >= 1).then(|| candidate.to_string())
        })
        .unwrap_or_else(|| output.trim().to_string())
}

fn numeric_version(version: &str) -> Vec<u64> {
    version
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .take(3)
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect()
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    numeric_version(latest) > numeric_version(current)
}

fn validate_self_update_result(
    result: Result<String, String>,
    previous_version: &str,
    observed_version: &str,
) -> Result<(), String> {
    match result {
        Ok(_) => Ok(()),
        Err(_) if is_newer_version(observed_version, previous_version) => Ok(()),
        Err(error) => Err(error),
    }
}

async fn read_output(mut stream: impl AsyncRead + Unpin) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = stream.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        // Drain pipes even after reaching the display limit, so installs cannot block.
        let keep = n.min((128 * 1024usize).saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn run(
    command: &str,
    args: &[&str],
    cwd: Option<&Path>,
    seconds: u64,
) -> Result<String, String> {
    let mut process = rmcp::transport::which_command(command)
        .map_err(|e| format!("Cannot find {command}: {e}"))?;
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process.kill_on_drop(true).no_console_window();
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }
    let mut child = process
        .spawn()
        .map_err(|e| format!("Cannot start {command}: {e}"))?;
    let stdout = child.stdout.take().ok_or("Missing stdout")?;
    let stderr = child.stderr.take().ok_or("Missing stderr")?;
    let execution = async {
        tokio::try_join!(
            async { child.wait().await.map_err(|e| e.to_string()) },
            read_output(stdout),
            read_output(stderr),
        )
    };
    let (status, out, err) = tokio::time::timeout(Duration::from_secs(seconds), execution)
        .await
        .map_err(|_| {
            format!("{command} timed out. Check installation status before retrying.")
        })??;
    if !status.success() {
        return Err(format!("{command}: {status}\n{out}\n{err}"));
    }
    Ok(out.trim().to_string())
}

#[tauri::command]
pub async fn computer_control_check(tool: ControlTool) -> Result<String, String> {
    crate::path_env::refresh_path_now();
    run(tool.command(), &["--version"], None, 15).await
}

#[tauri::command]
pub async fn computer_control_status(tool: ControlTool) -> Result<ControlToolStatus, String> {
    let current_version = extract_version(&computer_control_check(tool).await?);
    let mut latest_version = None;
    let mut update_available = false;

    match tool {
        ControlTool::Cua => {
            if let Ok(output) = run("cua-driver", &["check-update", "--json"], None, 30).await {
                if let Ok(status) = serde_json::from_str::<CuaUpdateStatus>(&output) {
                    latest_version = status.latest_version;
                    update_available = status.update_available;
                    if latest_version.is_none() {
                        latest_version = status.current_version;
                    }
                }
            }
        }
        ControlTool::Playwright => {
            if let Ok(output) = run("npm", &["view", "@playwright/cli", "version"], None, 30).await
            {
                let latest = extract_version(&output);
                update_available = is_newer_version(&latest, &current_version);
                latest_version = Some(latest);
            }
        }
    }

    Ok(ControlToolStatus {
        current_version,
        latest_version,
        update_available,
    })
}

fn import_control_skill(app: AppHandle, source: &Path) -> Result<SkillMeta, String> {
    let result = crate::skills::chat_skills_import(app, source.to_string_lossy().into_owned());
    result.skill.filter(|_| result.success).ok_or_else(|| {
        result
            .error
            .unwrap_or_else(|| "Skill installation failed".into())
    })
}

#[tauri::command]
pub async fn computer_control_install(
    app: AppHandle,
    tool: ControlTool,
) -> Result<SkillMeta, String> {
    let _guard = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Another computer-control installation is running")?;
    if computer_control_check(tool).await.is_err() {
        match tool {
            ControlTool::Playwright => {
                run(
                    "npm",
                    &["install", "-g", "@playwright/cli@latest"],
                    None,
                    300,
                )
                .await?;
            }
            ControlTool::Cua => {
                #[cfg(windows)]
                run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; irm https://cua.ai/driver/install.ps1 | iex"], None, 300).await?;
                #[cfg(not(windows))]
                run(
                    "bash",
                    &[
                        "-c",
                        "set -o pipefail; curl -fsSL https://cua.ai/driver/install.sh | bash",
                    ],
                    None,
                    300,
                )
                .await?;
            }
        }
    }
    computer_control_check(tool).await?;
    let home = directories::BaseDirs::new()
        .ok_or("Home directory unavailable")?
        .home_dir()
        .to_path_buf();
    let source = match tool {
        ControlTool::Cua => {
            run("cua-driver", &["skills", "install"], None, 120).await?;
            home.join(".cua-driver/skills/cua-driver")
        }
        ControlTool::Playwright => {
            let staging = home.join(".kivio/tool-setup/playwright");
            std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
            run(
                "playwright-cli",
                &["install", "--skills"],
                Some(&staging),
                120,
            )
            .await?;
            staging.join(".claude/skills/playwright-cli")
        }
    };
    // Reuse the normal importer: retain upstream references and expose the skill
    // in ~/.kivio/skills, where existing discovery and enable/disable already work.
    import_control_skill(app, &source)
}

#[tauri::command]
pub async fn computer_control_update(
    app: AppHandle,
    state: State<'_, crate::state::AppState>,
    tool: ControlTool,
) -> Result<SkillMeta, String> {
    let _guard = INSTALL_LOCK
        .try_lock()
        .map_err(|_| "Another computer-control installation is running")?;
    let previous_version = extract_version(&computer_control_check(tool).await?);

    let home = directories::BaseDirs::new()
        .ok_or("Home directory unavailable")?
        .home_dir()
        .to_path_buf();
    let source = match tool {
        ControlTool::Cua => {
            // Cua's MCP server is part of the driver binary. Update the binary first,
            // then refresh its separately versioned official Skill pack.
            state.mcp_disconnect_server(CUA_MCP_SERVER_ID).await;
            state.mcp_disconnect_server(LEGACY_CUA_MCP_SERVER_ID).await;
            let update_result =
                run("cua-driver", &["update", "--apply", "--json"], None, 300).await;
            crate::path_env::refresh_path_now();
            let observed_version = extract_version(&computer_control_check(tool).await?);
            validate_self_update_result(update_result, &previous_version, &observed_version)?;
            run("cua-driver", &["skills", "update"], None, 180).await?;
            home.join(".cua-driver/skills/cua-driver")
        }
        ControlTool::Playwright => {
            run(
                "npm",
                &["install", "-g", "@playwright/cli@latest"],
                None,
                300,
            )
            .await?;
            crate::path_env::refresh_path_now();
            let staging = home.join(".kivio/tool-setup/playwright");
            std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
            run(
                "playwright-cli",
                &["install", "--skills"],
                Some(&staging),
                120,
            )
            .await?;
            staging.join(".claude/skills/playwright-cli")
        }
    };

    computer_control_check(tool).await?;
    import_control_skill(app, &source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_cli_versions() {
        assert_eq!(extract_version("cua-driver 0.28.2"), "0.28.2");
        assert_eq!(extract_version("0.1.20"), "0.1.20");
    }

    #[test]
    fn compares_numeric_versions() {
        assert!(is_newer_version("0.28.2", "0.28.1"));
        assert!(!is_newer_version("0.28.1", "0.28.2"));
        assert!(!is_newer_version("0.28.2", "0.28.2"));
    }

    #[test]
    fn accepts_self_update_when_the_binary_was_replaced_despite_process_error() {
        assert!(validate_self_update_result(
            Err("installer process exited unsuccessfully".to_string()),
            "0.28.1",
            "0.28.2",
        )
        .is_ok());
    }
}
