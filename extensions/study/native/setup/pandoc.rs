//! Official portable Pandoc artifacts; never modifies the system installation.
use super::{download_verified, run, Progress, JOB};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

pub(in crate::reading) const VERSION: &str = "3.12.1";
const LICENSE: &str = include_str!("../../../../docs/licenses/Pandoc-GPL-2.0.txt");

// Version, sizes and SHA-256 digests pinned from jgm/pandoc's official release.
fn artifact() -> Result<(&'static str, &'static str, u64), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok((
            "arm64-macOS",
            "c2146cf33a583f3ac93eccddf1bb662371d5cfcce1aad1c87ad860cca1435d46",
            42_313_957,
        )),
        ("macos", "x86_64") => Ok((
            "x86_64-macOS",
            "81ed3e2cfecf096df15d407152f89a684fd65221082b919963f738e977fad3bc",
            26_487_481,
        )),
        ("windows", "x86_64") => Ok((
            "windows-x86_64",
            "6ef431cb20b2c24a0371fc19200dc49d452be54b1b5c9239f3462841ef53e587",
            42_154_914,
        )),
        _ => Err("当前系统暂不支持自动安装 Pandoc，可取消勾选后继续使用 Study。".into()),
    }
}
pub(super) fn download_size() -> Option<u64> {
    artifact().ok().map(|(_, _, size)| size)
}
pub(in crate::reading) fn executable(dir: &Path) -> PathBuf {
    dir.join("pandoc").join(if cfg!(windows) {
        "pandoc.exe"
    } else {
        "pandoc"
    })
}
pub(in crate::reading) fn installed(dir: &Path) -> bool {
    executable(dir).is_file()
        && std::fs::read_to_string(dir.join("pandoc/ready")).is_ok_and(|s| s == VERSION)
}
fn progress(phase: &str, step: u8) {
    *JOB.lock().unwrap_or_else(|e| e.into_inner()) = Some(Progress::new("pandoc", phase, step));
}
fn extract(bytes: Vec<u8>, target: &Path) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let index = (0..archive.len())
        .find(|&i| {
            archive.by_index(i).is_ok_and(|f| {
                f.name().is_ok_and(|name| {
                    name.ends_with("/bin/pandoc") || name.ends_with("/pandoc.exe")
                })
            })
        })
        .ok_or("Pandoc 压缩包中缺少可执行文件")?;
    let entry = archive.by_index(index).map_err(|e| e.to_string())?;
    // Only the single verified executable is extracted to a fixed path, never archive paths.
    if entry.size() > 256 * 1024 * 1024 {
        return Err("Pandoc 可执行文件超过预期大小".into());
    }
    let mut output = std::fs::File::create(target).map_err(|e| e.to_string())?;
    let mut bounded = entry.take(256 * 1024 * 1024 + 1);
    if std::io::copy(&mut bounded, &mut output).map_err(|e| e.to_string())? > 256 * 1024 * 1024 {
        return Err("Pandoc 解包文件超过预期大小".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) async fn install(dir: &Path) -> Result<(), String> {
    let (platform, checksum, _) = artifact()?;
    let folder = dir.join("pandoc");
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    progress("download", 1);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?;
    let archive = dir.join(format!("pandoc-{VERSION}.zip"));
    let url = format!(
        "https://github.com/jgm/pandoc/releases/download/{VERSION}/pandoc-{VERSION}-{platform}.zip"
    );
    let bytes = download_verified(&client, &url, &archive, checksum).await?;
    progress("extract", 2);
    let pending = folder.join(if cfg!(windows) {
        "pandoc-install.exe"
    } else {
        "pandoc-install"
    });
    let destination = pending.clone();
    tauri::async_runtime::spawn_blocking(move || extract(bytes, &destination))
        .await
        .map_err(|e| e.to_string())??;
    progress("verify", 3);
    let version = run(dir, &pending, &["--version".as_ref()]).await?;
    if version.lines().next() != Some(format!("pandoc {VERSION}").as_str()) {
        return Err("Pandoc 版本检查失败，请重试配置。".into());
    }
    // Exercise embedded templates and conversion without requiring Python or a TeX installation.
    let markdown = folder.join("probe.md");
    let docx = folder.join("probe.docx");
    std::fs::write(&markdown, "# Kivio Study\n\nPandoc conversion check.\n")
        .map_err(|e| e.to_string())?;
    run(
        dir,
        &pending,
        &[
            "--from=markdown".as_ref(),
            "--to=docx".as_ref(),
            markdown.as_os_str(),
            "--output".as_ref(),
            docx.as_os_str(),
        ],
    )
    .await?;
    let plain = run(
        dir,
        &pending,
        &[
            "--from=docx".as_ref(),
            "--to=plain".as_ref(),
            docx.as_os_str(),
        ],
    )
    .await?;
    if !plain.contains("Pandoc conversion check.") {
        return Err("Pandoc 文档转换检查失败，请重试配置。".into());
    }
    for path in [&markdown, &docx] {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    std::fs::write(folder.join("COPYING.md"), LICENSE).map_err(|e| e.to_string())?;
    let target = executable(dir);
    let backup = folder.join("pandoc.previous");
    if backup.exists() {
        std::fs::remove_file(&backup).map_err(|e| e.to_string())?;
    }
    if target.exists() {
        std::fs::rename(&target, &backup).map_err(|e| e.to_string())?;
    }
    if let Err(error) = std::fs::rename(&pending, &target) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &target);
        }
        return Err(error.to_string());
    }
    crate::extension_sdk::atomic_write(&folder.join("ready"), VERSION, "Pandoc readiness")?;
    if backup.exists() {
        let _ = std::fs::remove_file(backup);
    }
    Ok(())
}
