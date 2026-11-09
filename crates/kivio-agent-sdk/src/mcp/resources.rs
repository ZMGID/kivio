use std::{
    collections::HashSet,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};

use rmcp::model::{
    ClientRequest, ListResourcesRequest, PaginatedRequestParams, ProtocolVersion,
    ReadResourceRequest, ReadResourceRequestParams, ServerNotification, ServerResult,
    SubscriptionFilter,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Notify};

use super::{InFlight, Lifecycle, McpClient};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpResourceContents {
    #[serde(rename_all = "camelCase")]
    Text {
        uri: String,
        mime_type: Option<String>,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    Blob {
        uri: String,
        mime_type: Option<String>,
        blob: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpResourceUpdate {
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpResourceUpdateError {
    Closed,
    Lagged(u64),
    SubscriptionEnded { uri: String, message: String },
}

pub(super) type ResourceUpdateSender =
    broadcast::Sender<Result<McpResourceUpdate, McpResourceUpdateError>>;

/// One bounded connection-scoped stream. A slow consumer receives an explicit lag error.
pub struct McpResourceUpdates {
    receiver: broadcast::Receiver<Result<McpResourceUpdate, McpResourceUpdateError>>,
    peer: rmcp::service::Peer<rmcp::RoleClient>,
    lifecycle: Arc<StdMutex<Lifecycle>>,
    closing: Arc<Notify>,
}

impl McpResourceUpdates {
    pub async fn recv(&mut self) -> Result<McpResourceUpdate, McpResourceUpdateError> {
        let result = tokio::select! {
            biased;
            _ = wait_closed(&self.peer, &self.lifecycle, &self.closing) => return Err(McpResourceUpdateError::Closed),
            result = self.receiver.recv() => result,
        };
        result.map_err(|error| match error {
            broadcast::error::RecvError::Closed => McpResourceUpdateError::Closed,
            broadcast::error::RecvError::Lagged(count) => McpResourceUpdateError::Lagged(count),
        })?
    }
}

async fn wait_closed(
    peer: &rmcp::service::Peer<rmcp::RoleClient>,
    lifecycle: &StdMutex<Lifecycle>,
    closing: &Notify,
) {
    loop {
        let notified = closing.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if lifecycle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed
            || peer.is_transport_closed()
        {
            return;
        }
        tokio::select! {
            _ = notified => {},
            _ = tokio::time::sleep(Duration::from_millis(250)) => {},
        }
    }
}

pub(super) fn next_connection_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn validate_uri(uri: &str) -> Result<(), String> {
    if uri.len() > 16_384 || uri.chars().any(char::is_control) || reqwest::Url::parse(uri).is_err()
    {
        return Err("MCP resource URI must be an absolute URI of at most 16384 bytes".into());
    }
    Ok(())
}

impl McpClient {
    /// Identity changes on every connection, including a reconnect to the same server.
    pub fn connection_id(&self) -> u64 {
        self.connection_id
    }

    pub fn supports_resource_subscriptions(&self) -> bool {
        self.peer.peer_info().is_some_and(|info| {
            info.capabilities
                .resources
                .as_ref()
                .is_some_and(|resources| resources.subscribe == Some(true))
        })
    }

    /// Register before subscribing or reading so an immediate notification cannot be lost.
    pub fn subscribe_resource_updates(&self) -> McpResourceUpdates {
        McpResourceUpdates {
            receiver: self.resource_updates.subscribe(),
            peer: self.peer.clone(),
            lifecycle: self.lifecycle.clone(),
            closing: self.closing.clone(),
        }
    }

    /// Explicit close wakes immediately. The transport is also checked for remote EOF.
    pub async fn wait_closed(&self) {
        wait_closed(&self.peer, &self.lifecycle, &self.closing).await;
    }

    pub async fn list_resources(
        &self,
        timeout: Duration,
        cancelled: impl Future<Output = ()> + Send,
    ) -> Result<Vec<McpResource>, String> {
        let deadline = tokio::time::Instant::now() + timeout;
        tokio::pin!(cancelled);
        let mut cursor = None;
        let mut cursors = HashSet::new();
        let mut resources = Vec::new();
        for _ in 0..100 {
            let result = self
                .request(
                    ClientRequest::ListResourcesRequest(ListResourcesRequest::with_param(
                        PaginatedRequestParams::default().with_cursor(cursor),
                    )),
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                    cancelled.as_mut(),
                )
                .await
                .map_err(|error| error.message)?;
            let ServerResult::ListResourcesResult(page) = result else {
                return Err("MCP resources/list returned an unexpected result type".into());
            };
            for resource in page.resources {
                validate_uri(&resource.uri)?;
                resources.push(McpResource {
                    uri: resource.uri,
                    name: resource.name,
                    title: resource.title,
                    description: resource.description,
                    mime_type: resource.mime_type,
                    size: resource.size,
                });
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(resources);
            }
            if !cursors.insert(cursor.clone()) {
                return Err("MCP resources/list repeated a pagination cursor".into());
            }
        }
        Err("MCP resources/list exceeded 100 pages".into())
    }

    pub async fn read_resource(
        &self,
        uri: &str,
        timeout: Duration,
        cancelled: impl Future<Output = ()> + Send,
    ) -> Result<Vec<McpResourceContents>, String> {
        validate_uri(uri)?;
        let result = self
            .request(
                ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
                    ReadResourceRequestParams::new(uri),
                )),
                timeout,
                cancelled,
            )
            .await
            .map_err(|error| error.message)?;
        let ServerResult::ReadResourceResult(result) = result else {
            return Err("MCP resources/read returned an unexpected result type".into());
        };
        let contents = serde_json::to_value(result.contents)
            .map_err(|error| self.redact(error.to_string()))?;
        serde_json::from_value(contents)
            .map_err(|error| self.redact(format!("Invalid MCP resource contents: {error}")))
    }

    /// Own one subscription per URI for this connection. No resource body is read here.
    pub async fn subscribe_resource(
        &self,
        uri: &str,
        timeout: Duration,
        cancelled: impl Future<Output = ()> + Send,
    ) -> Result<(), String> {
        validate_uri(uri)?;
        if !self.supports_resource_subscriptions() {
            return Err("MCP server does not support resource subscriptions".into());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        tokio::pin!(cancelled);
        let mut subscriptions = tokio::select! {
            biased;
            _ = &mut cancelled => return Err("MCP resource subscription cancelled".into()),
            _ = self.wait_closed() => return Err("MCP connection is closed".into()),
            result = tokio::time::timeout_at(deadline, self.resource_subscriptions.lock()) =>
                result.map_err(|_| "MCP resource subscription timed out".to_owned())?,
        };
        if subscriptions
            .get(uri)
            .is_some_and(|task| task.as_ref().is_none_or(|task| !task.is_finished()))
        {
            return Ok(());
        }
        let modern = self
            .peer
            .peer_info()
            .is_some_and(|info| info.protocol_version >= ProtocolVersion::V_2026_07_28);
        if !modern {
            #[allow(deprecated)]
            let request = ClientRequest::SubscribeRequest(rmcp::model::SubscribeRequest::new(
                rmcp::model::SubscribeRequestParams::new(uri),
            ));
            self.request(
                request,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
                cancelled.as_mut(),
            )
            .await
            .map_err(|error| error.message)?;
            subscriptions.insert(uri.into(), None);
            return Ok(());
        }

        let closing = self.closing.notified();
        tokio::pin!(closing);
        closing.as_mut().enable();
        {
            let mut lifecycle = self
                .lifecycle
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if lifecycle.closed {
                return Err("MCP connection is closed".into());
            }
            lifecycle.in_flight += 1;
        }
        let _flight = InFlight(self);
        let filter = SubscriptionFilter::builder()
            .resource_subscription(uri)
            .build();
        let mut subscription = tokio::select! {
            biased;
            _ = closing => return Err("MCP connection is closed".into()),
            _ = &mut cancelled => return Err("MCP resource subscription cancelled".into()),
            result = tokio::time::timeout_at(deadline, self.peer.listen(filter)) =>
                result.map_err(|_| "MCP resource subscription timed out".to_owned())?
                    .map_err(|error| self.redact(super::conn::classify_error("resources/subscribe", &error)))?,
        };
        if !subscription
            .acknowledged()
            .resource_subscriptions
            .as_ref()
            .is_some_and(|uris| uris.iter().any(|accepted| accepted == uri))
        {
            let _ = subscription.cancel().await;
            return Err(
                "MCP server did not acknowledge the requested resource subscription".into(),
            );
        }
        let updates = self.resource_updates.clone();
        let secrets = self.secrets.clone();
        let uri_owned = uri.to_owned();
        let task = tokio::spawn(async move {
            let message = loop {
                match subscription.next().await {
                    Ok(Some(ServerNotification::ResourceUpdatedNotification(notification))) => {
                        if validate_uri(&notification.params.uri).is_ok() {
                            let _ = updates.send(Ok(McpResourceUpdate { uri: notification.params.uri }));
                        }
                    }
                    Ok(Some(_)) => {},
                    Ok(None) => break "MCP resource subscription ended; resubscribe before relying on updates".into(),
                    Err(error) => break super::redact(format!("MCP resource subscription failed: {error}"), &secrets),
                }
            };
            let _ = updates.send(Err(McpResourceUpdateError::SubscriptionEnded {
                uri: uri_owned,
                message,
            }));
        });
        subscriptions.insert(uri.into(), Some(task));
        Ok(())
    }
}
