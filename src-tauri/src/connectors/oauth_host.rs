//! Tauri browser adapter; PKCE and loopback handling live in oauth.
use tauri_plugin_shell::ShellExt;
pub type Browser = tauri::AppHandle;
pub fn open_browser(app: &Browser, url: &str) -> Result<(), String> {
    #[allow(deprecated)]
    app.shell().open(url, None).map_err(|error| format!("Failed to open browser for authorization: {error}"))
}
