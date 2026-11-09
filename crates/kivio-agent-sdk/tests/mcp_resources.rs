#![cfg(feature = "mcp")]

use kivio_agent_sdk::mcp::{
    McpClient, McpClientConfig, McpResourceContents, McpResourceUpdateError, McpTransport,
};
use std::{collections::HashMap, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(5);
const URI: &str = "test://resource/current";

async fn connect(modern: bool) -> McpClient {
    connect_with(modern, false).await
}

async fn connect_with(modern: bool, resource_only: bool) -> McpClient {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/resource_server.py");
    let mut args: Vec<String> = if cfg!(windows) {
        vec!["-X".into(), "utf8".into()]
    } else {
        Vec::new()
    };
    args.extend(["-u".into(), script.to_string_lossy().into_owned()]);
    McpClient::connect(
        McpClientConfig {
            transport: McpTransport::Stdio {
                command: if cfg!(windows) { "py" } else { "python3" }.into(),
                args,
                cwd: None,
                env: HashMap::from([
                    (
                        "MCP_RESOURCE_PROTOCOL".into(),
                        if modern { "2026" } else { "2025" }.into(),
                    ),
                    (
                        "MCP_RESOURCE_ONLY".into(),
                        if resource_only { "1" } else { "0" }.into(),
                    ),
                ]),
            },
        },
        reqwest::Client::new(),
    )
    .await
    .unwrap()
}

async fn exercise_protocol(modern: bool) {
    let client = connect(modern).await;
    assert!(client.supports_resource_subscriptions());
    let resources = client
        .list_resources(TIMEOUT, std::future::pending())
        .await
        .unwrap();
    assert_eq!(resources.len(), 2);
    assert_eq!(resources[0].uri, URI);
    let mut updates = client.subscribe_resource_updates();
    client
        .subscribe_resource(URI, TIMEOUT, std::future::pending())
        .await
        .unwrap();
    client
        .subscribe_resource(URI, TIMEOUT, std::future::pending())
        .await
        .unwrap();
    let body = client
        .read_resource(URI, TIMEOUT, std::future::pending())
        .await
        .unwrap();
    assert!(
        matches!(&body[..], [McpResourceContents::Text { text, .. }] if text == "resource-body-1")
    );
    let result = client
        .call_tool(
            "emit",
            serde_json::json!({}),
            TIMEOUT,
            std::future::pending(),
        )
        .await
        .unwrap();
    assert_eq!(result.content, "1");
    assert_eq!(
        tokio::time::timeout(TIMEOUT, updates.recv())
            .await
            .unwrap()
            .unwrap()
            .uri,
        URI
    );
    let result = client
        .call_tool(
            "emit",
            serde_json::json!({}),
            TIMEOUT,
            std::future::pending(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.content, "1",
        "updates must never automatically reread resource bodies"
    );
    client.close().await.unwrap();
    tokio::time::timeout(TIMEOUT, client.wait_closed())
        .await
        .unwrap();
    assert_eq!(
        updates.recv().await,
        Err(McpResourceUpdateError::Closed),
        "queued old updates must be rejected after disconnect"
    );
    assert!(client
        .read_resource(URI, TIMEOUT, std::future::pending())
        .await
        .is_err());
    let replacement = connect(modern).await;
    assert_ne!(replacement.connection_id(), client.connection_id());
    replacement.close().await.unwrap();
}

#[tokio::test]
async fn legacy_resource_list_read_subscribe_notification_disconnect() {
    exercise_protocol(false).await;
}

#[tokio::test]
async fn modern_resource_list_read_subscribe_notification_disconnect() {
    exercise_protocol(true).await;
}

#[tokio::test]
async fn resource_read_timeout_cancels_without_blocking_later_requests() {
    let client = connect(false).await;
    let error = client
        .read_resource(
            "test://resource/hang",
            Duration::from_millis(50),
            std::future::pending(),
        )
        .await
        .unwrap_err();
    assert!(error.contains("outcome is unknown"), "{error}");
    assert!(!client.is_closed());
    client
        .read_resource(URI, TIMEOUT, std::future::pending())
        .await
        .unwrap();
    client.close().await.unwrap();
}

#[tokio::test]
async fn slow_resource_listener_reports_lag() {
    let client = connect(false).await;
    let mut updates = client.subscribe_resource_updates();
    client
        .subscribe_resource(URI, TIMEOUT, std::future::pending())
        .await
        .unwrap();
    client
        .call_tool(
            "emit",
            serde_json::json!({"count": 500}),
            TIMEOUT,
            std::future::pending(),
        )
        .await
        .unwrap();
    assert!(matches!(
        updates.recv().await,
        Err(McpResourceUpdateError::Lagged(_))
    ));
    client.close().await.unwrap();
}

#[tokio::test]
async fn resource_only_server_connects_without_tools_list() {
    let client = connect_with(false, true).await;
    assert!(client.list_tools().await.unwrap().is_empty());
    assert_eq!(
        client
            .list_resources(TIMEOUT, std::future::pending())
            .await
            .unwrap()
            .len(),
        2
    );
    client
        .read_resource(URI, TIMEOUT, std::future::pending())
        .await
        .unwrap();
    client.close().await.unwrap();
}
