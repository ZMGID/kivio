//! Real loopback DAV boundary: published uploads, verified probes and saved credentials.
use kivio_model_runtime::webdav::{self, WebDavBackupKind, WebDavConfigInput};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
#[tokio::test]
async fn webdav_upload_publish_download_delete_and_probe_cleanup() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let stopped = Arc::new(AtomicBool::new(false));
    let done = stopped.clone();
    let server_url = url.clone();
    let server = std::thread::spawn(move || {
        let mut files = BTreeMap::<String, Vec<u8>>::new();
        let mut methods = vec![];
        while !done.load(Ordering::Relaxed) {
            let (mut socket, _) = match listener.accept() {
                Ok(v) => v,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = vec![];
            let mut buffer = [0u8; 4096];
            let end = loop {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let mut lines = headers.lines();
            let first = lines.next().unwrap();
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap();
            let path = parts.next().unwrap().to_owned();
            let headers: BTreeMap<_, _> = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
                .collect();
            assert_eq!(
                headers.get("authorization").map(String::as_str),
                Some("Basic dXNlcjpwYXNz")
            );
            let length = headers
                .get("content-length")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            while bytes.len() < end + length {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            let body = &bytes[end..end + length];
            let mut response = vec![];
            let status;
            match method {
                "MKCOL" => status = "201 Created",
                "PUT" => {
                    assert!(path.ends_with(".partial"));
                    assert_eq!(headers["if-none-match"], "*");
                    files.insert(path, body.to_vec());
                    status = "201 Created";
                }
                "MOVE" => {
                    assert_eq!(headers["overwrite"], "F");
                    let destination = headers["destination"]
                        .strip_prefix(&server_url)
                        .unwrap()
                        .to_owned();
                    let data = files.remove(&path).unwrap();
                    assert!(files.insert(destination, data).is_none());
                    status = "201 Created";
                }
                "GET" => {
                    response = files.get(&path).unwrap().clone();
                    status = "200 OK";
                }
                "DELETE" => {
                    assert!(files.remove(&path).is_some());
                    status = "204 No Content";
                }
                "PROPFIND" => {
                    let entries=files.iter().map(|(name,bytes)|format!("<d:response><d:href>{name}</d:href><d:propstat><d:prop><d:resourcetype/><d:getcontentlength>{}</d:getcontentlength><d:getlastmodified>Sat, 24 Oct 2026 00:00:00 GMT</d:getlastmodified></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>",bytes.len())).collect::<String>();
                    response=format!("<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\">{entries}</d:multistatus>").into_bytes();
                    status = "207 Multi-Status";
                }
                _ => panic!("unexpected DAV request {method}"),
            }
            methods.push(method.to_owned());
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .unwrap();
            socket.write_all(&response).unwrap();
        }
        (files, methods)
    });
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("profile");
    let config = || WebDavConfigInput {
        url: url.clone(),
        username: "user".into(),
        password: Some("pass".into()),
        directory: "Kivio".into(),
    };
    webdav::save(&root, config()).unwrap();
    let mut preserve = config();
    preserve.password = None;
    assert!(webdav::save(&root, preserve).unwrap().has_password);
    let result = async {
        webdav::test(&root, config()).await?;
        assert!(webdav::list(&root).await?.is_empty());
        let archive = directory.path().join("config.json");
        std::fs::write(&archive, b"backup payload").unwrap();
        webdav::upload(&root, &archive, WebDavBackupKind::Configuration).await?;
        let files = webdav::list(&root).await?;
        assert_eq!(files.len(), 1);
        assert!(files[0].name.ends_with(".json"));
        let target = directory.path().join("download.json");
        webdav::download(&root, &files[0].name, &target).await?;
        assert_eq!(std::fs::read(target).unwrap(), b"backup payload");
        assert!(webdav::delete(&root, "../../outside.json").await.is_err());
        webdav::delete(&root, &files[0].name).await?;
        assert!(webdav::list(&root).await?.is_empty());
        Ok::<_, String>(())
    }
    .await;
    stopped.store(true, Ordering::Relaxed);
    let (files, methods) = server.join().unwrap();
    result.unwrap();
    assert!(files.is_empty());
    assert_eq!(methods.iter().filter(|m| *m == "MOVE").count(), 2);
    let mut clear = config();
    clear.password = Some(String::new());
    assert!(!webdav::save(&root, clear).unwrap().has_password);
}
