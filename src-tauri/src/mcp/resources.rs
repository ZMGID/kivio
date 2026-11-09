//! Runtime-authorized resource access tied to one live MCP connection.

use std::{sync::Arc, time::Duration};

use kivio_agent_sdk::mcp::{McpClient, McpResource, McpResourceContents, McpResourceUpdates};

use crate::chat::agent::control::AgentRunHandle;

#[derive(Clone)]
pub(crate) struct McpResourceConnection {
    server_id: String,
    client: Arc<McpClient>,
    timeout: Duration,
}

impl McpResourceConnection {
    pub(super) fn new(server_id: String, client: Arc<McpClient>, timeout: Duration) -> Self {
        Self {
            server_id,
            client,
            timeout,
        }
    }

    pub(crate) fn server_id(&self) -> &str {
        &self.server_id
    }

    pub(crate) fn connection_id(&self) -> u64 {
        self.client.connection_id()
    }

    /// Reconnect and disconnect close the previous client, invalidating queued notices too.
    pub(crate) fn is_current(&self) -> bool {
        !self.client.is_closed()
    }

    pub(crate) fn subscribe_updates(&self) -> McpResourceUpdates {
        self.client.subscribe_resource_updates()
    }

    pub(crate) async fn wait_closed(&self) {
        self.client.wait_closed().await
    }

    pub(crate) async fn list_resources(
        &self,
        control: Option<&AgentRunHandle>,
    ) -> Result<Vec<McpResource>, String> {
        self.client
            .list_resources(self.timeout, wait_cancelled(control))
            .await
    }

    /// Callers start their notification listener before this method sends the subscription.
    pub(crate) async fn read_resource(
        &self,
        uri: &str,
        control: Option<&AgentRunHandle>,
    ) -> Result<Vec<McpResourceContents>, String> {
        if self.client.supports_resource_subscriptions() {
            self.client
                .subscribe_resource(uri, self.timeout, wait_cancelled(control))
                .await?;
        }
        self.client
            .read_resource(uri, self.timeout, wait_cancelled(control))
            .await
    }
}

async fn wait_cancelled(control: Option<&AgentRunHandle>) {
    match control {
        Some(control) => control.wait_inactive().await,
        None => std::future::pending().await,
    }
}
