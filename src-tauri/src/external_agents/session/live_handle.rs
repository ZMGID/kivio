#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LiveSessionHandle {
    pub agent_id: String,
    /// `"codex_app_server"` | `"acp_json_rpc"` | `"claude_stream_json"`.
    pub protocol: String,
    /// Native thread id (codex) / session id (ACP / claude)。
    pub native_id: String,
    /// Pi RPC reports the active JSONL file path; older handles and other protocols leave it blank.
    #[serde(default)]
    pub native_path: Option<String>,
    pub cwd: String,
}

impl LiveSessionHandle {
    /// Same Kivio conversation + same CLI protocol ⇒ resume this native id.
    ///
    /// Cwd is **not** part of the match. The file is already keyed by conversation_id;
    /// requiring a byte-identical cwd string (Windows `E:\` vs `E:/`, WSL `/mnt/e/...`)
    /// dropped the id, started a blank thread, and overwrote the binding — so reopening
    /// the conversation could not continue the original native session.
    pub fn can_resume(&self, agent_id: &str, protocol: &str) -> bool {
        self.agent_id == agent_id && self.protocol == protocol && !self.native_id.trim().is_empty()
    }
}
