//! Tauri host for the shared WebDAV protocol.
include!("webdav/core.rs");
use crate::state::AppState;
use tauri::{AppHandle, Manager, State};
fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app.path().app_data_dir().map_err(err)?;
    // A sibling survives full restore and is outside the archived app-data tree.
    let name = root
        .file_name()
        .ok_or("Invalid app data directory")?
        .to_string_lossy();
    Ok(root.with_file_name(format!("{name}.webdav.json")))
}
fn downloads() -> &'static Mutex<HashMap<String, tempfile::TempDir>> {
    static DOWNLOADS: OnceLock<Mutex<HashMap<String, tempfile::TempDir>>> = OnceLock::new();
    DOWNLOADS.get_or_init(Mutex::default)
}
#[tauri::command]
pub(crate) fn webdav_load(app: AppHandle) -> Result<WebDavConfig, String> {
    Ok(view(load(&config_path(&app)?)?))
}
#[tauri::command]
pub(crate) async fn webdav_save(
    app: AppHandle,
    input: WebDavConfigInput,
) -> Result<WebDavConfig, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let path = config_path(&app)?;
    let config = resolve(input, load(&path)?)?;
    crate::chat::storage::atomic_write(
        &path,
        &serde_json::to_string(&config).map_err(err)?,
        "WebDAV configuration",
    )?;
    Ok(view(config))
}
#[tauri::command]
pub(crate) async fn webdav_test(app: AppHandle, input: WebDavConfigInput) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let dav = Dav::new(&resolve(input, load(&config_path(&app)?)?)?)?;
    let temp = tempfile::tempdir().map_err(err)?;
    let path = temp.path().join("probe");
    std::fs::write(&path, b"Kivio WebDAV connection test").map_err(err)?;
    let name = format!(".kivio-probe-{}", uuid::Uuid::new_v4());
    dav.upload(&path, &name).await?;
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
#[tauri::command]
pub(crate) async fn webdav_list(app: AppHandle) -> Result<Vec<WebDavBackup>, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    Dav::new(&load(&config_path(&app)?)?)?.list().await
}
#[tauri::command]
pub(crate) async fn webdav_upload(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: WebDavBackupKind,
    interface: BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    if super::backup_restore_pending(app.clone())? {
        return Err("请先重启完成恢复 / Restart to finish pending restore".into());
    }
    let dav = Dav::new(&load(&config_path(&app)?)?)?;
    let temporary = tempfile::tempdir().map_err(err)?;
    let name = format!(
        "kivio-{}-{}-{}.{}",
        kind.label(),
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
        uuid::Uuid::new_v4(),
        kind.extension()
    );
    let path = temporary.path().join(&name);
    let skipped = match kind {
        WebDavBackupKind::Full => {
            super::export_full_backup(app, state, path.to_string_lossy().into(), interface).await?
        }
        WebDavBackupKind::Configuration => {
            crate::commands::export_settings(app, state, path.to_string_lossy().into(), None)?;
            Vec::new()
        }
    };
    dav.upload(&path, &name).await?;
    Ok(skipped)
}
#[tauri::command]
pub(crate) async fn webdav_download(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<WebDavDownload, String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    let temporary = tempfile::tempdir().map_err(err)?;
    backup_kind(&name).ok_or("Invalid Kivio backup name")?;
    let path = temporary.path().join(&name);
    Dav::new(&load(&config_path(&app)?)?)?
        .download(&name, &path)
        .await?;
    if matches!(backup_kind(&name), Some(WebDavBackupKind::Configuration)) {
        let recovery = app
            .path()
            .app_data_dir()
            .map_err(err)?
            .join(".backup-webdav-recovery");
        std::fs::create_dir_all(&recovery).map_err(err)?;
        let previous = recovery.join(format!("configuration-{}.json", uuid::Uuid::new_v4()));
        crate::commands::export_settings(app, state, previous.to_string_lossy().into(), None)?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    downloads()
        .lock()
        .map_err(err)?
        .insert(id.clone(), temporary);
    Ok(WebDavDownload {
        id,
        path: path.to_string_lossy().into(),
    })
}
#[tauri::command]
pub(crate) fn webdav_release(id: String) -> Result<(), String> {
    downloads().lock().map_err(err)?.remove(&id);
    Ok(())
}
#[tauri::command]
pub(crate) async fn webdav_delete(app: AppHandle, name: String) -> Result<(), String> {
    let _guard = TRANSFER
        .try_lock()
        .map_err(|_| "WebDAV operation in progress")?;
    backup_kind(&name).ok_or("Invalid Kivio backup name")?;
    Dav::new(&load(&config_path(&app)?)?)?.delete(&name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // Real HTTP boundary; responses model server behavior, not the backup implementation.
    fn server(
        script: Vec<(&'static str, &'static str, u16, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut bodies = vec![];
            for (method, path, status, body) in script {
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(socket) => break socket,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "missing {method} {path}"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                // Windows inherits the listener's nonblocking mode. Only accept
                // polls; request/response I/O uses blocking reads with a timeout.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = vec![];
                let header_end = loop {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(index) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                assert_eq!(
                    headers.lines().next().unwrap(),
                    format!("{method} {path} HTTP/1.1")
                );
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|s| s.parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 4096];
                    let count = socket.read(&mut chunk).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                bodies.push(bytes[header_end..].to_vec());
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            bodies
        });
        (url, thread)
    }
    fn config(url: String) -> Config {
        Config {
            url,
            username: "user".into(),
            password: "secret".into(),
            directory: "Kivio".into(),
        }
    }
    #[tokio::test]
    async fn remote_backup_round_trip_publishes_then_lists_downloads_and_deletes() {
        let name = "kivio-configuration-20261016-test.json";
        let xml = format!(
            r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/Kivio/{name}</D:href><D:propstat><D:prop><D:getcontentlength>6</D:getcontentlength><D:getlastmodified>Fri, 16 Oct 2026 12:00:00 GMT</D:getlastmodified><D:resourcetype/></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#
        );
        let (url, thread) = server(vec![
            ("MKCOL", "/Kivio/", 405, "".into()),
            ("PROPFIND", "/Kivio/", 207, r#"<multistatus xmlns="DAV:"><response><href>/Kivio/</href><propstat><prop><resourcetype><collection/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#.into()),
            (
                "PUT",
                "/Kivio/kivio-configuration-20261016-test.json.partial",
                201,
                "".into(),
            ),
            (
                "MOVE",
                "/Kivio/kivio-configuration-20261016-test.json.partial",
                201,
                "".into(),
            ),
            ("PROPFIND", "/Kivio/", 207, xml),
            (
                "GET",
                "/Kivio/kivio-configuration-20261016-test.json",
                200,
                "backup".into(),
            ),
            (
                "DELETE",
                "/Kivio/kivio-configuration-20261016-test.json",
                204,
                "".into(),
            ),
        ]);
        let dav = Dav::new(&config(url)).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::write(&source, b"backup").unwrap();
        dav.upload(&source, name).await.unwrap();
        let files = dav.list().await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, name);
        assert_eq!(files[0].size, 6);
        let target = temp.path().join("download");
        dav.download(&files[0].name, &target).await.unwrap();
        assert_eq!(std::fs::read(target).unwrap(), b"backup");
        dav.delete(name).await.unwrap();
        assert_eq!(thread.join().unwrap()[2], b"backup");
    }
    #[tokio::test]
    async fn failed_publish_cleans_partial_file_and_reports_failure() {
        let (url, thread) = server(vec![
            ("MKCOL", "/Kivio/", 201, "".into()),
            (
                "PUT",
                "/Kivio/kivio-full-test.kivio.partial",
                201,
                "".into(),
            ),
            (
                "MOVE",
                "/Kivio/kivio-full-test.kivio.partial",
                507,
                "".into(),
            ),
            (
                "DELETE",
                "/Kivio/kivio-full-test.kivio.partial",
                204,
                "".into(),
            ),
        ]);
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::write(&source, b"backup").unwrap();
        let error = Dav::new(&config(url))
            .unwrap()
            .upload(&source, "kivio-full-test.kivio")
            .await
            .unwrap_err();
        assert!(error.contains("507"));
        thread.join().unwrap();
    }
    #[test]
    fn listing_rejects_foreign_paths_collections_partial_files_and_failed_properties() {
        let response = |href: &str, status: &str, resource: &str| {
            format!(
                r#"<response><href>{href}</href><propstat><prop><getcontentlength>9</getcontentlength><resourcetype>{resource}</resourcetype></prop><status>HTTP/1.1 {status}</status></propstat></response>"#
            )
        };
        let xml = format!(
            "<multistatus xmlns=\"DAV:\">{}{}</multistatus>",
            [
                "https://evil.test/Kivio/kivio-full-a.kivio",
                "/other/kivio-full-b.kivio",
                "/Kivio/kivio-full-c.kivio.partial"
            ]
            .map(|href| response(href, "200 OK", ""))
            .join(""),
            [
                response("/Kivio/kivio-full-dir.kivio", "200 OK", "<collection/>"),
                response("/Kivio/kivio-full-missing.kivio", "404 Not Found", ""),
                response("/Kivio/kivio-full-good.kivio", "200 OK", "")
            ]
            .join("")
        );
        let files = parse_listing(
            xml.as_bytes(),
            &Url::parse("https://dav.test/Kivio/").unwrap(),
        )
        .unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "kivio-full-good.kivio");
        assert!(backup_kind("../../kivio-full-good.kivio").is_none());
        assert!(parse_listing(
            b"<html>login</html>",
            &Url::parse("https://dav.test/Kivio/").unwrap()
        )
        .is_err());
    }
    #[test]
    fn listing_accepts_empty_unsupported_properties_on_collections() {
        let xml = br#"<d:multistatus xmlns:d="DAV:">
          <d:response><d:href>/Kivio/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>
          <d:response><d:href>/Kivio/kivio-full-ok.kivio</d:href><d:propstat><d:prop><d:getcontentlength>42</d:getcontentlength><d:getlastmodified>Fri, 16 Oct 2026 12:00:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        </d:multistatus>"#;
        let files = parse_listing(xml, &Url::parse("https://dav.test/Kivio/").unwrap()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 42);
    }

    #[tokio::test]
    async fn conflicting_mkcol_can_reuse_a_verified_existing_collection() {
        let xml = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/Kivio/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let (url, thread) = server(vec![
            ("MKCOL", "/Kivio/", 409, "".into()),
            ("PROPFIND", "/Kivio/", 207, xml.into()),
        ]);
        Dav::new(&config(url))
            .unwrap()
            .ensure_directory()
            .await
            .unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn saved_password_is_preserved_without_exposing_it_and_explicit_clear_works() {
        let input = |password| WebDavConfigInput {
            url: "https://dav.test".into(),
            username: "user".into(),
            directory: "Kivio".into(),
            password,
        };
        let saved = resolve(input(None), config("https://dav.test".into())).unwrap();
        assert_eq!(saved.password, "secret");
        let view = serde_json::to_value(view(saved)).unwrap();
        assert_eq!(view["hasPassword"], true);
        assert!(view.get("password").is_none());
        let cleared = resolve(input(Some("".into())), config("https://dav.test".into())).unwrap();
        assert!(cleared.password.is_empty());
        let mut invalid = input(None);
        invalid.directory = "../outside".into();
        assert!(resolve(invalid, Config::default()).is_err());
    }
}
