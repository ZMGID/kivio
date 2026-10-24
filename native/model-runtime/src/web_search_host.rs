//! Native search lifecycle. HTTP adapters and pooled MCP/OAuth are original code.
use crate::{
    mcp::{NativeMcp, types::McpToolCallResult},
    settings::{ChatMcpServer, LensWebSearchConfig, Settings},
    web_search::{WebFetchPage, WebSearchResult},
};
use std::{path::PathBuf, sync::Arc};
pub type AppContext = ();
pub struct SearchState {
    pub http: reqwest::Client,
    settings: Settings,
    pool: Arc<NativeMcp>,
}
impl SearchState {
    pub fn settings_read(&self) -> &Settings {
        &self.settings
    }
    pub async fn mcp_call_tool(
        &self,
        _: Option<&AppContext>,
        server: &ChatMcpServer,
        name: &str,
        args: serde_json::Value,
    ) -> Result<McpToolCallResult, String> {
        self.pool.call(server, name, args).await
    }
}
pub struct NativeSearch {
    root: PathBuf,
    http: reqwest::Client,
    pool: Arc<NativeMcp>,
}
impl NativeSearch {
    pub fn new(root: PathBuf) -> Self {
        let http = reqwest::Client::new();
        Self {
            pool: Arc::new(NativeMcp::new(root.clone())),
            root,
            http,
        }
    }
    fn state(&self) -> Result<SearchState, String> {
        Ok(SearchState {
            http: self.http.clone(),
            settings: crate::native_preferences::load(&self.root)?,
            pool: self.pool.clone(),
        })
    }
    pub async fn search(
        &self,
        config: &LensWebSearchConfig,
        query: &str,
    ) -> Result<Vec<WebSearchResult>, String> {
        let state = self.state()?;
        let attempts = if state.settings.retry_enabled {
            state.settings.retry_attempts as usize
        } else {
            1
        };
        crate::web_search::search_web(&state, config, query, attempts, None).await
    }
    pub async fn fetch(
        &self,
        config: &LensWebSearchConfig,
        url: &str,
    ) -> Result<WebFetchPage, String> {
        let state = self.state()?;
        let attempts = if state.settings.retry_enabled {
            state.settings.retry_attempts as usize
        } else {
            1
        };
        crate::web_search::fetch_web(&state, config, url, attempts, None).await
    }
    pub async fn authorize(
        &self,
        browser: &crate::connectors::oauth_host::Browser,
        endpoint: &str,
    ) -> Result<ChatMcpServer, String> {
        crate::connectors::oauth::run_oauth_connect(
            browser,
            &self.http,
            "tinyfish-mcp",
            "TinyFish MCP",
            endpoint,
            None,
        )
        .await
    }
    pub async fn maintain(&self) {
        self.pool.maintain().await;
    }
    pub async fn shutdown(&self) {
        self.pool.shutdown().await;
    }
}
