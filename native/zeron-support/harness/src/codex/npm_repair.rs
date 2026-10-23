//! Repair a global npm install of Codex that lacks its native binary.
//!
//! `@openai/codex` ships the binary as an optional dependency
//! (`@openai/codex-linux-x64` and friends). An install made without optional
//! dependencies — some machine images — leaves a `codex` whose launcher
//! throws "Missing optional dependency" on every start, so no Codex run can
//! begin. Reinstalling that same version with optional dependencies fixes it.

use std::path::{Path, PathBuf};
use std::process::Stdio;

const MISSING: &str = "Missing optional dependency";
const PACKAGE: &str = "@openai/codex";

/// What [`repair`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repair {
    /// Codex starts (or isn't installed, or fails some other way): nothing
    /// for a reinstall to fix.
    NotNeeded,
    /// Codex was missing its binary and starts now.
    Repaired { version: String },
}

/// Check the installed Codex and reinstall its npm package when its binary
/// is missing. `Err` when the reinstall ran and Codex still can't start.
pub async fn repair() -> Result<Repair, String> {
    let Some(codex) = super::resolve_codex_executable() else {
        return Ok(Repair::NotNeeded);
    };
    let Some(npm) = crate::adapter_install::find_npm() else {
        return Ok(Repair::NotNeeded);
    };
    // A root-owned global prefix (a machine image's /usr/local) needs sudo;
    // only without a password prompt.
    let sudo = (std::env::consts::OS != "windows")
        .then(|| crate::executable::find_on_paths("sudo", Vec::new()))
        .flatten();
    let sudo = match sudo {
        Some(sudo) if succeeds(&sudo, &["-n", "true"]).await => Some(sudo),
        _ => None,
    };
    repair_with(&codex, &npm, sudo.as_deref()).await
}

pub(crate) async fn repair_with(
    codex: &Path,
    npm: &Path,
    sudo: Option<&Path>,
) -> Result<Repair, String> {
    match probe(codex).await {
        Probe::Starts | Probe::Fails(_) => return Ok(Repair::NotNeeded),
        Probe::MissingBinary => {}
    }
    let version = installed_version(codex).unwrap_or_else(|| "latest".into());
    let spec = format!("{PACKAGE}@{version}");
    let args = [
        "install",
        "-g",
        "--include=optional",
        "--no-audit",
        "--no-fund",
        spec.as_str(),
    ];
    let mut output = run(npm, &args).await?;
    if !output.status.success()
        && let Some(sudo) = sudo
        && permission_denied(&output)
    {
        let mut with_npm = vec![OsArg::Path(npm.to_path_buf())];
        with_npm.extend(args.iter().map(|arg| OsArg::Str((*arg).to_string())));
        output = run_args(sudo, &[OsArg::Str("-n".into())], &with_npm).await?;
    }
    if !output.status.success() {
        return Err(format!(
            "reinstalling {spec} failed: {}",
            tail(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    match probe(codex).await {
        Probe::Starts => Ok(Repair::Repaired { version }),
        Probe::MissingBinary => Err(format!("{spec} still has no native binary")),
        Probe::Fails(error) => Err(format!("Codex still can't start: {error}")),
    }
}

enum Probe {
    Starts,
    MissingBinary,
    Fails(String),
}

async fn probe(codex: &Path) -> Probe {
    match command(codex).arg("--version").output().await {
        Ok(out) if out.status.success() => Probe::Starts,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains(MISSING) {
                Probe::MissingBinary
            } else {
                Probe::Fails(tail(&stderr))
            }
        }
        Err(err) => Probe::Fails(err.to_string()),
    }
}

/// The version of the npm package behind `codex`
/// (`…/node_modules/@openai/codex/bin/codex.js`, usually through a symlink).
fn installed_version(codex: &Path) -> Option<String> {
    let script = std::fs::canonicalize(codex).ok()?;
    let manifest = script.parent()?.parent()?.join("package.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest).ok()?).ok()?;
    if value["name"] != PACKAGE {
        return None;
    }
    let version = value["version"].as_str()?;
    version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
        .then(|| version.to_string())
}

fn permission_denied(output: &std::process::Output) -> bool {
    let stderr = String::from_utf8_lossy(&output.stderr);
    stderr.contains("EACCES") || stderr.contains("EPERM") || stderr.contains("permission denied")
}

enum OsArg {
    Str(String),
    Path(PathBuf),
}

async fn run(program: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let args: Vec<OsArg> = args
        .iter()
        .map(|arg| OsArg::Str((*arg).to_string()))
        .collect();
    run_args(program, &[], &args).await
}

async fn run_args(
    program: &Path,
    lead: &[OsArg],
    args: &[OsArg],
) -> Result<std::process::Output, String> {
    let mut cmd = command(program);
    for arg in lead.iter().chain(args) {
        match arg {
            OsArg::Str(s) => cmd.arg(s),
            OsArg::Path(p) => cmd.arg(p),
        };
    }
    tokio::time::timeout(std::time::Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| format!("{} timed out", program.display()))?
        .map_err(|err| format!("{}: {err}", program.display()))
}

async fn succeeds(program: &Path, args: &[&str]) -> bool {
    command(program)
        .args(args)
        .output()
        .await
        .is_ok_and(|out| out.status.success())
}

fn command(program: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

fn tail(text: &str) -> String {
    let text = text.trim();
    let start = text.char_indices().rev().nth(399).map_or(0, |(i, _)| i);
    text[start..].to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A global install laid out like npm's: `bin/codex` → the package's
    /// launcher, which fails until the platform package "arrives".
    fn install(dir: &Path) -> PathBuf {
        let package = dir.join("lib/node_modules/@openai/codex");
        std::fs::create_dir_all(package.join("bin")).unwrap();
        std::fs::write(
            package.join("package.json"),
            r#"{"name":"@openai/codex","version":"0.163.0"}"#,
        )
        .unwrap();
        let fixed = dir.join("fixed");
        script(
            &package.join("bin/codex.js"),
            &format!(
                "if [ -f '{}' ]; then echo 'codex-cli 0.163.0'; exit 0; fi\n\
                 echo 'Error: Missing optional dependency @openai/codex-linux-x64. Reinstall Codex' >&2; exit 1",
                fixed.display()
            ),
        );
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let codex = dir.join("bin/codex");
        std::os::unix::fs::symlink(package.join("bin/codex.js"), &codex).unwrap();
        codex
    }

    #[tokio::test]
    async fn reinstalls_the_same_version_with_optional_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let codex = install(dir.path());
        let npm = dir.path().join("npm");
        let calls = dir.path().join("calls");
        script(
            &npm,
            &format!(
                "echo \"$@\" >> '{}'; touch '{}'",
                calls.display(),
                dir.path().join("fixed").display()
            ),
        );
        let outcome = repair_with(&codex, &npm, None).await;
        assert_eq!(
            outcome,
            Ok(Repair::Repaired {
                version: "0.163.0".into()
            })
        );
        let calls = std::fs::read_to_string(calls).unwrap();
        assert_eq!(
            calls.trim(),
            "install -g --include=optional --no-audit --no-fund @openai/codex@0.163.0"
        );
        // Healthy now: nothing more to do.
        assert_eq!(repair_with(&codex, &npm, None).await, Ok(Repair::NotNeeded));
    }

    #[tokio::test]
    async fn a_root_owned_prefix_retries_through_sudo() {
        let dir = tempfile::tempdir().unwrap();
        let codex = install(dir.path());
        let npm = dir.path().join("npm");
        let fixed = dir.path().join("fixed");
        // Plain npm is refused; through "sudo" (which marks itself) it works.
        script(
            &npm,
            &format!(
                "if [ -n \"$AS_ROOT\" ]; then touch '{}'; exit 0; fi\n\
                 echo 'npm error code EACCES' >&2; exit 243",
                fixed.display()
            ),
        );
        let sudo = dir.path().join("sudo");
        script(
            &sudo,
            "[ \"$1\" = -n ] || exit 9; shift; AS_ROOT=1 exec \"$@\"",
        );
        let outcome = repair_with(&codex, &npm, Some(&sudo)).await;
        assert!(
            matches!(outcome, Ok(Repair::Repaired { .. })),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn other_failures_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex");
        script(&codex, "echo 'some other crash' >&2; exit 1");
        let npm = dir.path().join("npm");
        script(&npm, "echo called > /dev/null; exit 1");
        assert_eq!(repair_with(&codex, &npm, None).await, Ok(Repair::NotNeeded));
    }
}
