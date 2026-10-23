//! Isolate `DocHost::upload_tool_sidecar`: point it at a one-shot local HTTP
//! listener and report whether the PUT ever arrives. Diagnosing the
//! zero-blobs-in-prod mystery (refs stamped, uploads absent, no warns).
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeron_engine::doc_host::{DocHost, DocHostConfig, EdgeConfig};
use zeron_sync::DocsStore;

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = sock.read(&mut buf).await.unwrap();
        let head = String::from_utf8_lossy(&buf[..n]);
        let first = head.lines().next().unwrap_or("").to_string();
        let auth = head
            .lines()
            .find(|l| l.to_lowercase().starts_with("authorization"))
            .unwrap_or("")
            .to_string();
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 11\r\ncontent-type: application/json\r\n\r\n{\"ok\":true}")
            .await;
        (first, auth)
    });

    let dir = std::env::temp_dir().join(format!("sidecar-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(DocsStore::open(&dir).unwrap());
    let host = DocHost::new(
        store,
        DocHostConfig {
            device_id: "probe-dev".into(),
            default_harness: zeron_proto::HarnessId::ClaudeCode,
            edge: Some(EdgeConfig::with_static_token(
                format!("http://{addr}"),
                "probe-user",
            )),
        },
    );
    host.upload_tool_sidecar(
        "chat-probe",
        zeron_doc::SidecarPayload {
            part_id: "part#1".into(),
            output: Some("full output body".into()),
            diff: None,
        },
    );

    match tokio::time::timeout(std::time::Duration::from_secs(5), server).await {
        Ok(Ok((first, auth))) => {
            println!("PUT ARRIVED: {first}");
            println!("auth header present: {}", !auth.is_empty());
        }
        _ => println!("NO REQUEST within 5s — uploader silently skipped"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
