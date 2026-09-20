//! 连接器：目录化 + 一键授权的外部数据源接入。
//!
//! Token 连接沿用设置保存入口；OAuth 在这里负责发现、已注册应用 / DCR、
//! PKCE、浏览器回调与 token 刷新。没有自动注册的服务须提供自己的应用身份。

mod github_device;
pub mod oauth;
pub mod obsidian;

pub use obsidian::list_obsidian_vaults_cmd;

use std::{collections::HashMap, sync::Mutex};
use tauri::{AppHandle, State};
use tauri_plugin_shell::ShellExt;
use tokio::sync::watch;

use crate::settings::ChatMcpServer;
use crate::state::AppState;

/// One authorization per initiating window. Dropping an operation removes its handle.
#[derive(Default)]
pub struct OAuthFlows(Mutex<HashMap<String, (String, watch::Sender<bool>)>>);

struct OAuthFlow<'a> {
    owner: &'a OAuthFlows,
    window: String,
    request: String,
}

impl OAuthFlows {
    pub fn cancel_window(&self, window: &str) {
        if let Some((_, sender)) = self.0.lock().unwrap().remove(window) {
            let _ = sender.send(true);
        }
    }
    fn begin(&self, window: &str, request: &str) -> (OAuthFlow<'_>, watch::Receiver<bool>) {
        let (sender, receiver) = watch::channel(false);
        if let Some((_, previous)) = self
            .0
            .lock()
            .unwrap()
            .insert(window.into(), (request.into(), sender))
        {
            let _ = previous.send(true);
        }
        (
            OAuthFlow {
                owner: self,
                window: window.into(),
                request: request.into(),
            },
            receiver,
        )
    }

    fn cancel(&self, window: &str, request: &str) {
        if let Some((current, sender)) = self.0.lock().unwrap().get(window) {
            if current == request {
                let _ = sender.send(true);
            }
        }
    }
}

impl Drop for OAuthFlow<'_> {
    fn drop(&mut self) {
        let mut flows = self.owner.0.lock().unwrap();
        if flows
            .get(&self.window)
            .is_some_and(|(request, _)| request == &self.request)
        {
            flows.remove(&self.window);
        }
    }
}

#[tauri::command]
pub fn connector_oauth_cancel(
    window: tauri::WebviewWindow,
    flows: State<'_, OAuthFlows>,
    request_id: String,
) {
    flows.cancel(window.label(), &request_id);
}

/// 内置 OAuth 连接器目录：catalog_id → (展示名, MCP resource URL)。
/// 与前端 `connectorCatalog.ts` 中 authKind:'oauth' 的项保持一致。
fn builtin_oauth_url(catalog_id: &str) -> Option<(&'static str, &'static str)> {
    match catalog_id {
        "notion" => Some(("Notion", "https://mcp.notion.com/mcp")),
        "github" => Some(("GitHub", github_device::RESOURCE)),
        _ => None,
    }
}

/// 跑一次完整 OAuth 流程，返回物化好的 ChatMcpServer。
///
/// - 传 `catalog_id`：命中内置 OAuth 连接器（如 notion），用其 resource URL。
/// - 传 `url`（+可选 `name`）：自定义 OAuth 连接器，直接用该 URL。
///
/// 不直接写 settings——返回给前端，由前端合并进 `chat_tools.servers` 并保存
/// （沿用既有「前端改 settings → save_settings」模式）。
#[tauri::command]
pub async fn connector_oauth_connect(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, AppState>,
    flows: State<'_, OAuthFlows>,
    catalog_id: Option<String>,
    url: Option<String>,
    name: Option<String>,
    client: Option<oauth::OAuthClientConfig>,
    request_id: String,
    on_device_code: tauri::ipc::Channel<github_device::DevicePrompt>,
) -> Result<ChatMcpServer, String> {
    let http = state.http.clone();
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("Invalid OAuth request id".into());
    }
    let catalog_id = catalog_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let builtin = catalog_id.and_then(builtin_oauth_url);
    let resource_url = builtin
        .map(|(_, url)| url)
        .or_else(|| url.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .ok_or_else(|| "OAuth connector requires a catalog id or URL".to_string())?;
    let display_name = builtin
        .map(|(name, _)| name)
        .or_else(|| name.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .or(catalog_id)
        .unwrap_or("Custom Connector");
    let connector_id = catalog_id
        .map(str::to_string)
        .unwrap_or_else(|| format!("custom-{}", slugify(display_name)));
    let (_flow, mut cancelled) = flows.begin(window.label(), &request_id);
    let operation = async {
        if github_device::is_resource(resource_url)
            && client
                .as_ref()
                .is_none_or(|c| c.client_id.trim().is_empty())
        {
            github_device::connect(&http, &connector_id, display_name, resource_url, |prompt| {
                let uri = prompt.verification_uri.clone();
                on_device_code
                    .send(prompt)
                    .map_err(|_| "Authorization window closed".to_string())?;
                #[allow(deprecated)]
                app.shell()
                    .open(uri, None)
                    .map_err(|_| "Failed to open GitHub authorization in your browser".to_string())
            })
            .await
        } else {
            oauth::run_oauth_connect(
                &app,
                &http,
                &connector_id,
                display_name,
                resource_url,
                client.as_ref(),
            )
            .await
        }
    };
    tokio::select! {
        biased;
        _ = cancelled.changed() => Err("OAUTH_CANCELLED".into()),
        result = operation => result,
    }
}

/// 与前端 `slugify` 对齐：转小写、非字母数字折叠为连字符、去首尾连字符；空则 "custom"。
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "custom".to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_matches_frontend() {
        assert_eq!(slugify("My Server"), "my-server");
        assert_eq!(slugify("  Acme!! Corp  "), "acme-corp");
        assert_eq!(slugify("***"), "custom");
        assert_eq!(slugify(""), "custom");
    }

    #[test]
    fn builtin_oauth_url_known_and_unknown() {
        assert!(builtin_oauth_url("notion").is_some());
        assert_eq!(
            builtin_oauth_url("github"),
            Some(("GitHub", github_device::RESOURCE))
        );
        assert!(builtin_oauth_url("unknown").is_none());
    }

    #[test]
    fn cancellation_is_scoped_and_old_cleanup_does_not_remove_a_new_flow() {
        let flows = OAuthFlows::default();
        let (old, old_cancelled) = flows.begin("chat", "old");
        let (current, current_cancelled) = flows.begin("chat", "current");
        assert!(*old_cancelled.borrow());
        drop(old);
        flows.cancel("other-window", "current");
        flows.cancel("chat", "old");
        assert!(!*current_cancelled.borrow());
        flows.cancel("chat", "current");
        assert!(*current_cancelled.borrow());
        drop(current);
        assert!(flows.0.lock().unwrap().is_empty());
    }

    #[test]
    fn closing_the_window_cancels_its_authorization() {
        let flows = OAuthFlows::default();
        let (_flow, cancelled) = flows.begin("chat", "request");
        flows.cancel_window("chat");
        assert!(*cancelled.borrow());
        assert!(flows.0.lock().unwrap().is_empty());
    }
}
