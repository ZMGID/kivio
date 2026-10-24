//! Native host for the shared MCP OAuth flow.
#[path = "../../../src-tauri/src/connectors/oauth.rs"]
pub mod oauth;
pub mod oauth_host {
    pub type Browser = dyn Fn(&str) -> Result<(), String> + Send + Sync;
    pub fn open_browser(browser: &Browser, url: &str) -> Result<(), String> {
        browser(url)
    }
}
