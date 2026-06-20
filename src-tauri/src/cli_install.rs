//! "Install command line tool" — register the `kivio` command on the user's PATH
//! so `kivio code` works from any terminal.
//!
//! Triggered by a button in the Kivio Code settings page (never automatically at
//! install time). The implementation is platform-specific:
//!
//! - **Windows**: add the directory containing `kivio.exe` to the *user* `Path`
//!   (`HKCU\Environment\Path`) and broadcast `WM_SETTINGCHANGE` so newly-opened
//!   terminals pick it up. The main `kivio.exe` itself carries the `code`
//!   subcommand, so `kivio code` resolves to it. We touch only the per-user
//!   registry hive — never the system PATH.
//! - **macOS**: symlink `~/.local/bin/kivio` → the app's main binary. `kivio code`
//!   then dispatches to the terminal agent. `/usr/local/bin` needs sudo, so we
//!   prefer the writable user-owned `~/.local/bin`.
//!
//! All paths derive from [`std::env::current_exe`], so the link/PATH entry always
//! points at the running app.

/// Result of an install attempt, surfaced to the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallCliResult {
    /// `true` if the command is now (or was already) installed and usable.
    pub ok: bool,
    /// `true` when nothing needed to change (already on PATH / link present).
    pub already_installed: bool,
    /// Human-readable status / next-step hint for the UI.
    pub message: String,
}

/// Install the `kivio` command on the current user's PATH. See module docs.
pub fn install_cli() -> Result<InstallCliResult, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot resolve app path: {e}"))?;
    #[cfg(target_os = "windows")]
    {
        install_windows(&exe)
    }
    #[cfg(target_os = "macos")]
    {
        install_macos(&exe)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = exe;
        Err("Installing the command line tool is not supported on this platform.".to_string())
    }
}

#[cfg(target_os = "windows")]
fn install_windows(exe: &std::path::Path) -> Result<InstallCliResult, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LPARAM, WPARAM};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let dir = exe
        .parent()
        .ok_or_else(|| "cannot resolve install directory".to_string())?
        .to_path_buf();
    let dir_str = dir.to_string_lossy().to_string();

    // Helper: NUL-terminated UTF-16.
    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    unsafe {
        // Open HKCU\Environment for read+write (create not needed; it always exists).
        let mut hkey = HKEY::default();
        let subkey = wide("Environment");
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ | KEY_WRITE,
            &mut hkey,
        );
        if status != ERROR_SUCCESS {
            return Err(format!("cannot open user environment registry: {status:?}"));
        }

        // Read the existing user Path (may be absent → treat as empty).
        let value_name = wide("Path");
        let mut value_type = windows::Win32::System::Registry::REG_VALUE_TYPE(0);
        let mut size: u32 = 0;
        let q = RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        );
        let current = if q == ERROR_SUCCESS && size > 0 {
            let mut buf = vec![0u8; size as usize];
            let mut sz = size;
            let q2 = RegQueryValueExW(
                hkey,
                PCWSTR(value_name.as_ptr()),
                None,
                Some(&mut value_type),
                Some(buf.as_mut_ptr()),
                Some(&mut sz),
            );
            if q2 != ERROR_SUCCESS {
                let _ = RegCloseKey(hkey);
                return Err(format!("cannot read user Path: {q2:?}"));
            }
            // Bytes → UTF-16 → String, trimming the trailing NUL.
            let u16s: Vec<u16> = buf
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let mut s = String::from_utf16_lossy(&u16s);
            while s.ends_with('\0') {
                s.pop();
            }
            s
        } else if q == ERROR_FILE_NOT_FOUND {
            String::new()
        } else if q == ERROR_SUCCESS {
            String::new()
        } else {
            let _ = RegCloseKey(hkey);
            return Err(format!("cannot query user Path: {q:?}"));
        };

        // Already present? (case-insensitive, ignore trailing slashes / empty segments)
        let normalized = |p: &str| p.trim().trim_end_matches('\\').to_ascii_lowercase();
        let target_norm = normalized(&dir_str);
        let already = current
            .split(';')
            .map(|seg| normalized(seg))
            .any(|seg| seg == target_norm);
        if already {
            let _ = RegCloseKey(hkey);
            return Ok(InstallCliResult {
                ok: true,
                already_installed: true,
                message: "kivio is already on your PATH. Open a new terminal and run `kivio code`."
                    .to_string(),
            });
        }

        // Append our directory. Preserve REG_EXPAND_SZ if the existing value used it
        // (so %VAR% entries keep expanding); default to REG_SZ for a fresh value.
        let new_value = if current.trim().is_empty() {
            dir_str.clone()
        } else if current.ends_with(';') {
            format!("{current}{dir_str}")
        } else {
            format!("{current};{dir_str}")
        };
        let write_type = if value_type == REG_EXPAND_SZ {
            REG_EXPAND_SZ
        } else {
            REG_SZ
        };
        let wide_new = wide(&new_value);
        let bytes: &[u8] = std::slice::from_raw_parts(
            wide_new.as_ptr() as *const u8,
            wide_new.len() * std::mem::size_of::<u16>(),
        );
        let set = RegSetValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            write_type,
            Some(bytes),
        );
        let _ = RegCloseKey(hkey);
        if set != ERROR_SUCCESS {
            return Err(format!("cannot write user Path: {set:?}"));
        }

        // Broadcast the environment change so already-running shells / Explorer
        // refresh. Best-effort: a failure here just means the user must open a new
        // terminal (which they need to anyway), so we don't fail the install.
        let env = wide("Environment");
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(env.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        );
    }

    Ok(InstallCliResult {
        ok: true,
        already_installed: false,
        message: format!(
            "Added {dir_str} to your user PATH. Open a NEW terminal and run `kivio code`."
        ),
    })
}

#[cfg(target_os = "macos")]
fn install_macos(exe: &std::path::Path) -> Result<InstallCliResult, String> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| "cannot resolve home directory".to_string())?;
    let bin_dir = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("cannot create {}: {e}", bin_dir.display()))?;
    let link = bin_dir.join("kivio");

    // If a correct symlink already exists, report idempotent success.
    if let Ok(existing) = std::fs::read_link(&link) {
        if existing == exe {
            return Ok(InstallCliResult {
                ok: true,
                already_installed: true,
                message: format!(
                    "kivio is already linked at {}. Ensure {} is on your PATH, then run `kivio code`.",
                    link.display(),
                    bin_dir.display()
                ),
            });
        }
    }

    // Replace any stale link/file, then create the symlink to the running app.
    if link.exists() || std::fs::symlink_metadata(&link).is_ok() {
        std::fs::remove_file(&link)
            .map_err(|e| format!("cannot replace existing {}: {e}", link.display()))?;
    }
    std::os::unix::fs::symlink(exe, &link)
        .map_err(|e| format!("cannot create symlink {}: {e}", link.display()))?;

    let on_path = std::env::var("PATH")
        .map(|p| p.split(':').any(|seg| std::path::Path::new(seg) == bin_dir))
        .unwrap_or(false);

    if on_path {
        return Ok(InstallCliResult {
            ok: true,
            already_installed: false,
            message: format!(
                "Linked kivio at {}. Run `kivio code` in a terminal.",
                link.display()
            ),
        });
    }

    // `~/.local/bin` isn't on PATH yet. Try to wire it into the user's shell rc
    // automatically (idempotent, marker-wrapped) so reopening a terminal is enough.
    let manual_hint = format!(
        "Linked kivio at {}. Add {} to your PATH (e.g. in ~/.zshrc), then run `kivio code`.",
        link.display(),
        bin_dir.display()
    );

    let shell = std::env::var("SHELL").ok();
    let message = match macos_rc_plan(shell.as_deref(), &home) {
        Some(plan) => match ensure_path_block_in_rc(&plan.rc_path, &plan.block) {
            Ok(_) => format!(
                "Linked kivio and updated {}. Open a NEW terminal and run `kivio code`.",
                plan.rc_path.display()
            ),
            // Writing the rc failed — degrade to the manual hint instead of erroring.
            Err(_) => manual_hint,
        },
        // Unknown shell: don't guess, fall back to the manual hint.
        None => manual_hint,
    };

    Ok(InstallCliResult {
        ok: true,
        already_installed: false,
        message,
    })
}

/// The marker comment that opens our managed block in a shell rc file. Kept
/// stable so a future "uninstall" can locate and remove the block cleanly.
#[cfg(target_os = "macos")]
const RC_BLOCK_START: &str = "# >>> kivio cli >>>";
/// The marker comment that closes our managed block.
#[cfg(target_os = "macos")]
const RC_BLOCK_END: &str = "# <<< kivio cli <<<";

/// Which rc file to edit and what PATH line to put in the managed block, chosen
/// from the user's `$SHELL`. Returns `None` for unknown shells (caller falls
/// back to a manual hint rather than guessing).
#[cfg(target_os = "macos")]
struct RcPlan {
    rc_path: std::path::PathBuf,
    /// The full marker-wrapped block, ready to append (ends with a newline).
    block: String,
}

/// Pure shell → rc-file + block decision, split out so it's unit-testable
/// without touching the environment or filesystem. `shell` is the raw `$SHELL`
/// value (e.g. `/bin/zsh`); we match on its basename.
#[cfg(target_os = "macos")]
fn macos_rc_plan(shell: Option<&str>, home: &std::path::Path) -> Option<RcPlan> {
    let basename = shell
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .and_then(|s| s.rsplit('/').next())
        .unwrap_or("");

    let (rc_path, path_line) = match basename {
        "zsh" => (
            home.join(".zshrc"),
            r#"export PATH="$HOME/.local/bin:$PATH""#.to_string(),
        ),
        "bash" => (
            home.join(".bash_profile"),
            r#"export PATH="$HOME/.local/bin:$PATH""#.to_string(),
        ),
        "fish" => (
            home.join(".config").join("fish").join("config.fish"),
            r#"fish_add_path "$HOME/.local/bin""#.to_string(),
        ),
        _ => return None,
    };

    let block = format!("{RC_BLOCK_START}\n{path_line}\n{RC_BLOCK_END}\n");
    Some(RcPlan { rc_path, block })
}

/// Idempotently append `block` to the rc file at `rc_path`:
/// - if the file already contains our start marker, do nothing (already managed);
/// - otherwise create parent dirs / the file as needed and append the block,
///   preserving existing content and ensuring a separating newline.
#[cfg(target_os = "macos")]
fn ensure_path_block_in_rc(rc_path: &std::path::Path, block: &str) -> std::io::Result<()> {
    let existing = match std::fs::read_to_string(rc_path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };

    if let Some(content) = &existing {
        if content.contains(RC_BLOCK_START) {
            // Already managed — idempotent no-op.
            return Ok(());
        }
    }

    if let Some(parent) = rc_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut new_content = existing.unwrap_or_default();
    // Ensure the prior content ends with a newline so our block starts cleanly,
    // and add a blank line of separation if there's any existing content.
    if !new_content.is_empty() {
        if !new_content.ends_with('\n') {
            new_content.push('\n');
        }
        new_content.push('\n');
    }
    new_content.push_str(block);

    std::fs::write(rc_path, new_content)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn home() -> PathBuf {
        PathBuf::from("/Users/test")
    }

    #[test]
    fn zsh_targets_zshrc_with_export() {
        let plan = macos_rc_plan(Some("/bin/zsh"), &home()).expect("zsh plan");
        assert_eq!(plan.rc_path, home().join(".zshrc"));
        assert!(plan.block.contains(r#"export PATH="$HOME/.local/bin:$PATH""#));
        assert!(plan.block.starts_with(RC_BLOCK_START));
        assert!(plan.block.trim_end().ends_with(RC_BLOCK_END));
    }

    #[test]
    fn bash_targets_bash_profile_with_export() {
        let plan = macos_rc_plan(Some("/usr/local/bin/bash"), &home()).expect("bash plan");
        assert_eq!(plan.rc_path, home().join(".bash_profile"));
        assert!(plan.block.contains(r#"export PATH="$HOME/.local/bin:$PATH""#));
    }

    #[test]
    fn fish_targets_config_fish_with_fish_add_path() {
        let plan = macos_rc_plan(Some("/opt/homebrew/bin/fish"), &home()).expect("fish plan");
        assert_eq!(
            plan.rc_path,
            home().join(".config").join("fish").join("config.fish")
        );
        assert!(plan.block.contains(r#"fish_add_path "$HOME/.local/bin""#));
    }

    #[test]
    fn unknown_or_missing_shell_returns_none() {
        assert!(macos_rc_plan(Some("/bin/tcsh"), &home()).is_none());
        assert!(macos_rc_plan(Some(""), &home()).is_none());
        assert!(macos_rc_plan(None, &home()).is_none());
    }

    fn write_temp_rc(name: &str, contents: Option<&str>) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "kivio-cli-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rc");
        if let Some(c) = contents {
            std::fs::write(&path, c).unwrap();
        }
        path
    }

    fn cleanup(path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn append_creates_missing_file() {
        let rc = write_temp_rc("missing", None);
        let block = format!("{RC_BLOCK_START}\nexport X\n{RC_BLOCK_END}\n");
        ensure_path_block_in_rc(&rc, &block).unwrap();
        let written = std::fs::read_to_string(&rc).unwrap();
        assert_eq!(written, block);
        cleanup(&rc);
    }

    #[test]
    fn append_preserves_existing_and_adds_separation() {
        let rc = write_temp_rc("existing", Some("alias foo=bar"));
        let block = format!("{RC_BLOCK_START}\nexport X\n{RC_BLOCK_END}\n");
        ensure_path_block_in_rc(&rc, &block).unwrap();
        let written = std::fs::read_to_string(&rc).unwrap();
        assert_eq!(written, format!("alias foo=bar\n\n{block}"));
        cleanup(&rc);
    }

    #[test]
    fn append_is_idempotent() {
        let rc = write_temp_rc("idem", Some("alias foo=bar\n"));
        let block = format!("{RC_BLOCK_START}\nexport X\n{RC_BLOCK_END}\n");
        ensure_path_block_in_rc(&rc, &block).unwrap();
        let after_first = std::fs::read_to_string(&rc).unwrap();
        // Second call must not duplicate the block.
        ensure_path_block_in_rc(&rc, &block).unwrap();
        let after_second = std::fs::read_to_string(&rc).unwrap();
        assert_eq!(after_first, after_second);
        assert_eq!(after_second.matches(RC_BLOCK_START).count(), 1);
        cleanup(&rc);
    }
}
