use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

use kivio_agent_sdk::{
    agent::{
        host::SessionInput, InjectionContext, InjectionError, InjectionKind, InjectionSender,
        RuntimeMessage, WakePolicy,
    },
    mcp::McpResourceUpdateError,
    model::ModelRole,
};
use tauri::{AppHandle, Manager};

use crate::{
    mcp::{registry::NativeToolContext, resources::McpResourceConnection},
    state::AppState,
};

pub(crate) struct McpNotice {
    pub(crate) connection: McpResourceConnection,
    pub(crate) uri: String,
    authority: Arc<AtomicBool>,
    warning: Option<String>,
}

pub(crate) struct McpKind;

impl InjectionKind for McpKind {
    type Item = McpNotice;

    fn is_stale(&self, notice: &McpNotice, _context: InjectionContext<'_>) -> bool {
        !notice.authority.load(Ordering::Acquire) || !notice.connection.is_current()
    }

    fn build(
        &self,
        notices: &[McpNotice],
        _context: InjectionContext<'_>,
    ) -> Result<Vec<SessionInput>, InjectionError> {
        let mut lines = HashSet::new();
        for notice in notices {
            let path = resource_path(notice.connection.server_id(), &notice.uri);
            lines.insert(match &notice.warning {
                Some(warning) => format!("MCP resource updates from {} need attention. {warning} Use read {} to check current resources.", serde_json::json!(notice.connection.server_id()), serde_json::json!(path)),
                None => format!("MCP resource {} was updated. Use read {} when its current contents are needed.", serde_json::json!(notice.uri), serde_json::json!(path)),
            });
        }
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let mut lines: Vec<_> = lines.into_iter().collect();
        lines.sort();
        Ok(vec![SessionInput::append(RuntimeMessage::text(
            ModelRole::User,
            lines.join("\n"),
        ))])
    }
}

struct McpListener {
    authority: Arc<AtomicBool>,
    uris: Arc<RwLock<HashSet<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for McpListener {
    fn drop(&mut self) {
        self.authority.store(false, Ordering::Release);
        self.task.abort();
    }
}

#[derive(Default)]
pub(crate) struct McpListeners(HashMap<(String, u64), McpListener>);

impl McpListeners {
    pub(crate) fn retain_authorized(&mut self, server_ids: &[String]) {
        self.0.retain(|(id, _), _| server_ids.contains(id));
    }

    pub(crate) fn ensure(
        &mut self,
        app: &AppHandle,
        owner: &str,
        connection: McpResourceConnection,
        uri: &str,
        sender: InjectionSender<McpNotice>,
    ) {
        let owner = owner.to_owned();
        let app = app.clone();
        self.ensure_with_wake(connection, uri, sender, move || super::wake(&app, &owner));
    }

    fn ensure_with_wake(
        &mut self,
        connection: McpResourceConnection,
        uri: &str,
        sender: InjectionSender<McpNotice>,
        wake: impl Fn() + Send + 'static,
    ) {
        let key = (
            connection.server_id().to_owned(),
            connection.connection_id(),
        );
        self.0
            .retain(|(id, generation), _| id != &key.0 || generation == &key.1);
        if let Some(listener) = self
            .0
            .get(&key)
            .filter(|listener| !listener.task.is_finished())
        {
            listener
                .uris
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .insert(uri.into());
            return;
        }
        let uris = Arc::new(RwLock::new(HashSet::from([uri.to_owned()])));
        let task_uris = uris.clone();
        let authority = Arc::new(AtomicBool::new(true));
        let task_authority = authority.clone();
        let mut updates = connection.subscribe_updates();
        let task = tokio::spawn(async move {
            loop {
                let (uri, warning) = tokio::select! {
                    biased;
                    _ = connection.wait_closed() => break,
                    result = updates.recv() => match result {
                        Ok(update) => {
                            if !task_uris.read().unwrap_or_else(|error| error.into_inner()).contains(&update.uri) { continue; }
                            (update.uri, None)
                        }
                        Err(McpResourceUpdateError::Closed) => break,
                        Err(McpResourceUpdateError::Lagged(count)) => (String::new(), Some(format!("The listener missed {count} updates."))),
                        Err(McpResourceUpdateError::SubscriptionEnded { uri, message }) => {
                            if !task_uris.read().unwrap_or_else(|error| error.into_inner()).contains(&uri) { continue; }
                            (uri, Some(message))
                        }
                    },
                };
                if !task_authority.load(Ordering::Acquire) {
                    break;
                }
                if sender
                    .enqueue(
                        McpNotice {
                            connection: connection.clone(),
                            uri,
                            authority: task_authority.clone(),
                            warning,
                        },
                        WakePolicy::WhenIdle,
                    )
                    .is_err()
                {
                    break;
                }
                wake();
            }
            task_authority.store(false, Ordering::Release);
        });
        self.0.insert(
            key,
            McpListener {
                authority,
                uris,
                task,
            },
        );
    }
}

fn resource_path(server_id: &str, uri: &str) -> String {
    if uri.is_empty() {
        return format!("mcp://{server_id}");
    }
    format!(
        "mcp://{server_id}/{}",
        url::form_urlencoded::byte_serialize(uri.as_bytes()).collect::<String>()
    )
}

fn parse_path(path: &str) -> Result<(&str, Option<String>), String> {
    let path = path
        .strip_prefix("mcp://")
        .ok_or("Expected mcp:// resource path")?;
    let (server, uri) = path.split_once('/').unwrap_or((path, ""));
    if server.is_empty()
        || server
            .chars()
            .any(|character| character.is_control() || matches!(character, '?' | '#'))
    {
        return Err("MCP resource path requires an exact server ID".into());
    }
    if uri.is_empty() {
        return Ok((server, None));
    }
    let query = format!("uri={uri}");
    let mut decoded = url::form_urlencoded::parse(query.as_bytes());
    let (_, uri) = decoded.next().ok_or("Missing MCP resource URI")?;
    if decoded.next().is_some() {
        return Err("MCP resource URI must be percent-encoded".into());
    }
    Ok((server, Some(uri.into_owned())))
}

pub(crate) async fn read(
    app: &AppHandle,
    context: &NativeToolContext<'_>,
    path: &str,
) -> Result<String, String> {
    let state = app.state::<AppState>();
    let settings = state.settings_read().clone();
    if path == "mcp://" {
        let servers: Vec<_> = settings.chat_tools.servers.iter()
            .filter(|server| settings.chat_tools.enabled && crate::mcp::registry::mcp_server_is_runtime_eligible(server) && context.mcp_server_ids.contains(&server.id))
            .map(|server| serde_json::json!({"name": server.name, "path": resource_path(&server.id, "")})).collect();
        return serde_json::to_string(&servers).map_err(|error| error.to_string());
    }
    let (server_id, uri) = parse_path(path)?;
    if !context.mcp_server_ids.iter().any(|id| id == server_id) {
        return Err("MCP resource server is not authorized for this runtime".into());
    }
    let server = settings
        .chat_tools
        .servers
        .iter()
        .find(|server| {
            settings.chat_tools.enabled
                && crate::mcp::registry::mcp_server_is_runtime_eligible(server)
                && server.id == server_id
        })
        .cloned()
        .ok_or("MCP resource server is unavailable")?;
    let connection = state
        .mcp_resource_connection(Some(app), &server, context.mcp_server_ids)
        .await?;
    let value =
        match uri {
            None => {
                let resources = connection.list_resources(context.control.as_ref()).await?;
                serde_json::to_value(resources.into_iter().map(|resource| serde_json::json!({
                "path": resource_path(server_id, &resource.uri), "resource": resource,
            })).collect::<Vec<_>>())
            }
            Some(uri) => {
                let owner = crate::chat::sub_agent::services::owner_for_run(
                    &state.sub_agents,
                    &context.conversation_id,
                    &context.run_id,
                );
                state
                    .runtime_injections
                    .listen_mcp(app, &owner, connection.clone(), &uri)?;
                serde_json::to_value(
                    connection
                        .read_resource(&uri, context.control.as_ref())
                        .await?,
                )
            }
        }
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kivio_agent_sdk::agent::provider_runtime::{
        ModelCall, ModelOutputSink, ModelRuntime, ProviderFuture,
    };
    use kivio_agent_sdk::agent::{
        AgentSession, AgentSessionConfig, InjectionQueue, InjectionSources,
    };
    use kivio_agent_sdk::model::{GenerateOutput, ModelError};
    use std::{sync::Mutex, time::Duration};

    #[derive(Default)]
    struct CapturedModel(Mutex<Vec<ModelCall>>);
    impl ModelRuntime for CapturedModel {
        fn generate<'a>(
            &'a self,
            call: ModelCall,
            _: &'a mut dyn ModelOutputSink,
        ) -> ProviderFuture<'a, Result<GenerateOutput, ModelError>> {
            self.0.lock().unwrap().push(call);
            Box::pin(async { Ok(GenerateOutput::text("Done".into(), None)) })
        }
    }

    async fn model_request(queue: InjectionQueue) -> String {
        let model = Arc::new(CapturedModel::default());
        let session = AgentSession::new(
            AgentSessionConfig::new(model.clone(), "test")
                .with_injections(InjectionSources::new().with(queue)),
        );
        session
            .prompt(RuntimeMessage::text(ModelRole::User, "Continue"))
            .await
            .unwrap();
        let calls = model.0.lock().unwrap();
        assert_eq!(calls.len(), 1);
        calls[0]
            .history
            .iter()
            .map(RuntimeMessage::text_content)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn server() -> crate::settings::ChatMcpServer {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/kivio-agent-sdk/tests/fixtures/resource_server.py");
        let mut args: Vec<String> = if cfg!(windows) {
            vec!["-X".into(), "utf8".into()]
        } else {
            Vec::new()
        };
        args.extend(["-u".into(), script.to_string_lossy().into_owned()]);
        crate::settings::ChatMcpServer {
            id: "resources-test".into(),
            name: "Resources".into(),
            enabled: true,
            command: if cfg!(windows) { "py" } else { "python3" }.into(),
            args,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn real_mcp_updates_reach_model_with_uri_and_authority_isolation() {
        let state = crate::state::test_app_state();
        let server = server();
        assert!(state
            .mcp_resource_connection(None, &server, &[])
            .await
            .is_err());
        let allowed = vec![server.id.clone()];
        let connection = state
            .mcp_resource_connection(None, &server, &allowed)
            .await
            .unwrap();
        let parent_queue = InjectionQueue::new();
        let sibling_queue = InjectionQueue::new();
        let parent_sender = parent_queue.register(McpKind).unwrap();
        let sibling_sender = sibling_queue.register(McpKind).unwrap();
        let mut parent = McpListeners::default();
        let mut sibling = McpListeners::default();
        let parent_wake = Arc::new(tokio::sync::Notify::new());
        let sibling_wake = Arc::new(tokio::sync::Notify::new());
        let wake = parent_wake.clone();
        parent.ensure_with_wake(
            connection.clone(),
            "test://resource/current",
            parent_sender,
            move || wake.notify_one(),
        );
        let wake = sibling_wake.clone();
        sibling.ensure_with_wake(
            connection.clone(),
            "test://resource/second",
            sibling_sender,
            move || wake.notify_one(),
        );
        connection
            .read_resource("test://resource/current", None)
            .await
            .unwrap();
        connection
            .read_resource("test://resource/second", None)
            .await
            .unwrap();
        state
            .mcp_call_tool(None, &server, "emit", serde_json::json!({}), None)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), parent_wake.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), sibling_wake.notified())
            .await
            .unwrap();
        let request = model_request(parent_queue.clone()).await;
        assert!(request.contains("test://resource/current"), "{request}");
        assert!(!request.contains("test://resource/second"), "{request}");
        assert!(
            !request.contains("resource-body"),
            "notifications must not read bodies"
        );
        let request = model_request(sibling_queue.clone()).await;
        assert!(request.contains("test://resource/second"), "{request}");
        assert!(!request.contains("test://resource/current"), "{request}");

        state
            .mcp_call_tool(None, &server, "emit", serde_json::json!({}), None)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), parent_wake.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), sibling_wake.notified())
            .await
            .unwrap();
        parent.retain_authorized(&[]);
        assert!(
            !model_request(parent_queue).await.contains("was updated"),
            "removed server authority invalidates queued notices"
        );
        state.mcp_disconnect_server(&server.id).await;
        assert!(!connection.is_current());
        assert!(
            !model_request(sibling_queue).await.contains("was updated"),
            "disconnect invalidates queued notices"
        );
        let replacement = state
            .mcp_resource_connection(None, &server, &allowed)
            .await
            .unwrap();
        assert_ne!(replacement.connection_id(), connection.connection_id());
        state.mcp_disconnect_all().await;
        let _ = std::fs::remove_dir_all(&state.usage_dir);
    }

    #[test]
    fn resource_paths_round_trip_reserved_characters_and_unicode() {
        let uri = "file:///目录/a b+c?x=a&y=b#fragment";
        let path = resource_path("server-id", uri);
        assert_eq!(parse_path(&path).unwrap(), ("server-id", Some(uri.into())));
        assert_eq!(parse_path("mcp://server-id").unwrap(), ("server-id", None));
        assert!(parse_path("mcp://").is_err());
        assert!(parse_path("mcp://server-id/test://x?x=a&y=b").is_err());
    }
}
