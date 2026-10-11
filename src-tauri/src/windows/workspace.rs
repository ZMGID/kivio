//! Agent and Bot workspaces share one native window, with separate restoration state.
use tauri::{AppHandle, Manager, WebviewWindow};

pub(crate) fn is_bot_workspace(window: &WebviewWindow) -> bool {
    window.url().ok().is_some_and(|url| {
        url.query_pairs()
            .any(|(key, value)| key == "workspace" && value == "bot")
    })
}

#[tauri::command]
pub fn switch_chat_workspace(window: WebviewWindow, bot: bool) -> Result<(), String> {
    if window.label() != "chat" {
        return Err("Workspace switching requires the main chat window".into());
    }
    switch_workspace(&window, bot, None)
}

pub(crate) fn switch_workspace(
    window: &WebviewWindow,
    bot: bool,
    route: Option<&str>,
) -> Result<(), String> {
    let app = window.app_handle();
    let mut url = window.url().map_err(|error| error.to_string())?;
    let current_bot = is_bot_workspace(window);
    if let Some(current_route) = url
        .fragment()
        .filter(|route| super::valid_workspace_route(route, current_bot))
    {
        super::chat_remember_last_route(
            app.clone(),
            window.clone(),
            Some(current_route.into()),
            Some(current_bot),
        )?;
    }
    let route = route
        .map(str::to_owned)
        .or_else(|| super::load_stored_last_chat_route(app, bot))
        .unwrap_or_else(|| "chat".into());
    if !super::valid_workspace_route(&route, bot) {
        return Err("Conversation belongs to another workspace".into());
    }
    url.set_query(if bot { Some("workspace=bot") } else { None });
    url.set_fragment(Some(route.trim_start_matches('#')));
    // Navigation reloads only the webview. Backend runs continue and the new shell resyncs them.
    window.navigate(url).map_err(|error| error.to_string())
}

pub(crate) fn open_workspace(app: &AppHandle, conversation_id: Option<&str>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    crate::shortcuts::set_macos_regular_activation_policy(app);
    let window = super::ensure_chat_window(app)?;
    let route =
        conversation_id.map(|id| format!("chat/{}", super::urlencoding_conversation_id(id)));
    switch_workspace(&window, true, route.as_deref())?;
    let _ = window.unminimize();
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}
