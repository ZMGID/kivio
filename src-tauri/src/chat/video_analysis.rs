//! Tauri attachment access for the shared video-analysis owner.
include!("video_analysis_core.rs");

impl VideoTool {
    pub async fn call(&mut self, app: &tauri::AppHandle, state: &crate::state::AppState, ctx: &super::agent::ToolExecutionContext<'_>, arguments: &Value) -> Result<crate::mcp::types::McpToolCallResult, String> {
        let settings = state.settings_read().clone();
        self.call_with_host(&settings, state, |input| super::commands::context::build_chat_api_messages_with_video(Some(app), "", input, None, None, &[], true), ctx, arguments).await
    }
}
