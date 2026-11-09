use super::*;

struct Fixture {
    root: tempfile::TempDir,
    file: PathBuf,
    log: PathBuf,
    config: LspServerConfig,
}

impl Fixture {
    fn new(mode: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("test.rs");
        std::fs::write(&file, "first").unwrap();
        let log = root.path().join("requests.jsonl");
        let script = root.path().join("fake-server.cjs");
        std::fs::write(&script, include_str!("fake-server.cjs")).unwrap();
        let config = LspServerConfig {
            command: "node".into(),
            args: vec![
                script.to_string_lossy().into(),
                log.to_string_lossy().into(),
                mode.into(),
            ],
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
        };
        Self {
            root,
            file,
            log,
            config,
        }
    }

    async fn change(
        &self,
        service: &LspService,
        owner: &str,
        sender: &mpsc::UnboundedSender<LspDiagnostics>,
    ) {
        let sender = sender.clone();
        service
            .file_changed(
                owner,
                self.root.path(),
                &self.file,
                &self.config,
                Arc::new(move |event| {
                    let _ = sender.send(event);
                }),
            )
            .await
            .unwrap();
    }

    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    async fn wait_for(&self, method: &str, count: usize) {
        timeout(Duration::from_secs(5), async {
            loop {
                if self
                    .requests()
                    .iter()
                    .filter(|message| message["method"] == method)
                    .count()
                    >= count
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

async fn event(receiver: &mut mpsc::UnboundedReceiver<LspDiagnostics>) -> LspDiagnostics {
    timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn configured_servers_for_the_same_language_keep_independent_documents() {
    let fixture = Fixture::new("push");
    let service = LspService::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    fixture.change(&service, "owner", &sender).await;
    let first = event(&mut receiver).await;
    let mut second_config = fixture.config.clone();
    second_config.args[1] = fixture
        .root
        .path()
        .join("second-server.jsonl")
        .to_string_lossy()
        .into_owned();
    let sink = sender.clone();
    service
        .file_changed(
            "owner",
            fixture.root.path(),
            &fixture.file,
            &second_config,
            Arc::new(move |event| {
                let _ = sink.send(event);
            }),
        )
        .await
        .unwrap();
    let second = event(&mut receiver).await;
    assert_ne!(first.generation, second.generation);
    assert!(first.is_current());
    assert!(second.is_current());
    service.retain_configurations("owner", &[second_config]);
    assert!(!first.is_current());
    assert!(second.is_current());
    service.shutdown().await;
}

#[tokio::test]
async fn real_stdio_rejects_late_versions_and_invalidates_queued_events() {
    let fixture = Fixture::new("push");
    let service = LspService::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    fixture.change(&service, "main-runtime", &sender).await;
    let initial = event(&mut receiver).await;
    assert_eq!(initial.version, 1);
    assert!(initial.is_current());
    std::fs::write(&fixture.file, "second").unwrap();
    fixture.change(&service, "main-runtime", &sender).await;
    assert!(!initial.is_current());
    let current = event(&mut receiver).await;
    assert_eq!(current.version, 2);
    assert_eq!(current.owner, "main-runtime");
    assert_eq!(current.diagnostics[0].message, "current version");
    assert!(current.is_current());
    assert!(receiver.try_recv().is_err());
    fixture.wait_for("textDocument/didSave", 2).await;
    let requests = fixture.requests();
    assert_eq!(requests[0]["method"], "initialize");
    assert_eq!(requests[1]["method"], "initialized");
    let open = requests
        .iter()
        .find(|message| message["method"] == "textDocument/didOpen")
        .unwrap();
    assert_eq!(open["params"]["textDocument"]["text"], "first");
    assert_eq!(open["params"]["textDocument"]["languageId"], "rust");
    let change = requests
        .iter()
        .find(|message| message["method"] == "textDocument/didChange")
        .unwrap();
    assert_eq!(change["params"]["textDocument"]["version"], 2);
    assert_eq!(change["params"]["contentChanges"][0]["text"], "second");
    service.shutdown().await;
    assert!(!current.is_current());
    let requests = fixture.requests();
    assert!(requests
        .iter()
        .any(|message| message["method"] == "shutdown"));
    assert!(requests.iter().any(|message| message["method"] == "exit"));
}

#[tokio::test]
async fn pull_responses_use_request_versions_and_versionless_push_is_conservative() {
    let fixture = Fixture::new("pull");
    let service = LspService::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    fixture.change(&service, "child-runtime", &sender).await;
    fixture.wait_for("textDocument/diagnostic", 1).await;
    std::fs::write(&fixture.file, "second").unwrap();
    fixture.change(&service, "child-runtime", &sender).await;
    let current = event(&mut receiver).await;
    assert_eq!(current.version, 2);
    assert_eq!(current.diagnostics[0].message, "current pull");
    assert!(receiver.try_recv().is_err());
    service.shutdown().await;

    let fixture = Fixture::new("versionless");
    let service = LspService::default();
    fixture.change(&service, "main-runtime", &sender).await;
    assert_eq!(event(&mut receiver).await.version, 1);
    std::fs::write(&fixture.file, "second").unwrap();
    fixture.change(&service, "main-runtime", &sender).await;
    fixture.wait_for("textDocument/didSave", 2).await;
    service.shutdown().await;
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn exact_owners_and_workspace_lifetimes_are_isolated() {
    let fixture = Fixture::new("push");
    let sibling = Fixture::new("push");
    let service = LspService::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    fixture.change(&service, "main-runtime", &sender).await;
    let main = event(&mut receiver).await;
    fixture.change(&service, "child-runtime", &sender).await;
    let child = event(&mut receiver).await;
    sibling.change(&service, "main-runtime", &sender).await;
    let sibling_event = event(&mut receiver).await;
    assert_ne!(main.generation, child.generation);
    assert_ne!(main.workspace, sibling_event.workspace);
    service.close_owner("child-runtime");
    assert!(!child.is_current());
    assert!(main.is_current());
    assert!(sibling_event.is_current());
    fixture.change(&service, "child-runtime", &sender).await;
    let reopened = event(&mut receiver).await;
    assert_ne!(child.generation, reopened.generation);
    assert!(!child.is_current());
    assert!(reopened.is_current());
    service.shutdown().await;
}

#[tokio::test]
async fn boundary_rejects_outside_paths_empty_commands_and_oversized_files() {
    let fixture = Fixture::new("push");
    let other = Fixture::new("push");
    let service = LspService::default();
    let callback: DiagnosticsCallback =
        Arc::new(|_| panic!("Rejected files must not emit diagnostics"));
    assert!(service
        .file_changed(
            "main",
            fixture.root.path(),
            &other.file,
            &fixture.config,
            callback.clone()
        )
        .await
        .is_err());
    assert!(!fixture.log.exists());
    let mut config = fixture.config.clone();
    config.command.clear();
    assert!(service
        .file_changed(
            "main",
            fixture.root.path(),
            &fixture.file,
            &config,
            callback.clone()
        )
        .await
        .is_err());
    std::fs::write(&fixture.file, vec![b'x'; MAX_FILE_BYTES as usize + 1]).unwrap();
    assert!(service
        .file_changed(
            "main",
            fixture.root.path(),
            &fixture.file,
            &fixture.config,
            callback.clone()
        )
        .await
        .is_err());
    service.shutdown().await;
    assert!(!fixture
        .requests()
        .iter()
        .any(|message| message["method"] == "textDocument/didOpen"));
}

#[tokio::test]
async fn failed_server_can_be_replaced_and_close_cancels_initialization() {
    let mut fixture = Fixture::new("crash");
    let service = LspService::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    fixture.change(&service, "main", &sender).await;
    fixture.wait_for("initialize", 1).await;
    fixture.config.args[2] = "push".into();
    fixture.change(&service, "main", &sender).await;
    assert!(event(&mut receiver).await.is_current());
    service.shutdown().await;

    let fixture = Fixture::new("hang");
    let service = LspService::default();
    fixture.change(&service, "main", &sender).await;
    fixture.wait_for("initialize", 1).await;
    timeout(Duration::from_secs(2), service.shutdown())
        .await
        .unwrap();
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn malformed_or_oversized_frames_are_bounded() {
    for bytes in [
        b"Content-Length: 999999999\r\n\r\n".as_slice(),
        b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
        b"Content-Length: 2\r\n\r\nxx",
    ] {
        assert!(read_frame(&mut bytes.as_ref()).await.is_err());
    }
    let bytes = vec![b'x'; MAX_HEADER_BYTES + 1];
    assert!(read_frame(&mut bytes.as_slice()).await.is_err());
    let bytes = b"Content-Length: 2\r\n\r\n{}";
    assert_eq!(read_frame(&mut bytes.as_slice()).await.unwrap(), json!({}));
}

#[tokio::test]
async fn deletion_and_recreation_do_not_revive_old_diagnostics() {
    let fixture = Fixture::new("push");
    let service = LspService::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    fixture.change(&service, "main", &sender).await;
    let initial = event(&mut receiver).await;
    let path = tokio::fs::canonicalize(&fixture.file).await.unwrap();
    std::fs::remove_file(&fixture.file).unwrap();
    service
        .file_removed("main", fixture.root.path(), &path)
        .await
        .unwrap();
    assert!(!initial.is_current());
    fixture.wait_for("textDocument/didClose", 1).await;
    std::fs::write(&fixture.file, "recreated").unwrap();
    fixture.change(&service, "main", &sender).await;
    let recreated = event(&mut receiver).await;
    assert_eq!(recreated.version, 2);
    assert!(!initial.is_current());
    assert!(recreated.is_current());
    service.shutdown().await;
}

#[tokio::test]
async fn shutdown_terminates_server_descendants() {
    let fixture = Fixture::new("tree");
    let service = LspService::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    fixture.change(&service, "main", &sender).await;
    let _ = event(&mut receiver).await;
    let pid = std::fs::read_to_string(format!("{}.child", fixture.log.display()))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    async fn alive(pid: u32) -> bool {
        let mut command = Command::new("node");
        command.args(["-e", &format!("try {{ process.kill({pid}, 0); process.exit(0); }} catch {{ process.exit(1); }}")]).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        command.status().await.unwrap().success()
    }
    assert!(alive(pid).await);
    service.shutdown().await;
    timeout(Duration::from_secs(3), async {
        while alive(pid).await {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
