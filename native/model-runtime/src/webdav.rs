// The wire protocol, URL boundaries, size limits and atomic remote upload are
// exactly the implementation used by the Tauri host.
include!("../../../src-tauri/src/backup/webdav/core.rs");

fn path(root: &Path) -> Result<PathBuf, String> {
    let name = root
        .file_name()
        .ok_or("Invalid app data directory")?
        .to_string_lossy();
    Ok(root.with_file_name(format!("{name}.webdav.json")))
}
pub fn configuration(root: &Path) -> Result<WebDavConfig, String> {
    Ok(view(load(&path(root)?)?))
}
pub fn save(root: &Path, input: WebDavConfigInput) -> Result<WebDavConfig, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let path = path(root)?;
    let config = resolve(input, load(&path)?)?;
    crate::file_io::atomic_write(
        &path,
        &serde_json::to_string(&config).map_err(err)?,
        "WebDAV configuration",
    )?;
    Ok(view(config))
}
pub async fn list(root: &Path) -> Result<Vec<WebDavBackup>, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    Dav::new(&load(&path(root)?)?)?.list().await
}
pub async fn test(root: &Path, input: WebDavConfigInput) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let dav = Dav::new(&resolve(input, load(&path(root)?)?)?)?;
    let temp = tempfile::tempdir().map_err(err)?;
    let probe = temp.path().join("probe");
    std::fs::write(&probe, b"Kivio WebDAV connection test").map_err(err)?;
    let name = format!(".kivio-probe-{}", uuid::Uuid::new_v4());
    dav.upload(&probe, &name).await?;
    let result = async {
        let response = checked(
            dav.request("GET", dav.file(&name)?)
                .send()
                .await
                .map_err(network_error)?,
        )?;
        if bounded_bytes(response, 1024).await? != b"Kivio WebDAV connection test" {
            return Err("WebDAV read-back verification failed".to_string());
        }
        dav.list().await
    }
    .await;
    let cleanup = dav.delete(&name).await;
    result?;
    cleanup
}
pub async fn upload(root: &Path, archive: &Path, kind: WebDavBackupKind) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let name = format!(
        "kivio-{}-{}-{}.{}",
        kind.label(),
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        uuid::Uuid::new_v4(),
        kind.extension()
    );
    Dav::new(&load(&path(root)?)?)?.upload(archive, &name).await
}
pub async fn download(root: &Path, name: &str, destination: &Path) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    backup_kind(name).ok_or("Invalid Kivio backup name")?;
    Dav::new(&load(&path(root)?)?)?
        .download(name, destination)
        .await
}
pub async fn delete(root: &Path, name: &str) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    backup_kind(name).ok_or("Invalid Kivio backup name")?;
    Dav::new(&load(&path(root)?)?)?.delete(name).await
}
