/// 导入留下的指纹，用于**过期检测**（ADR-0002）。
///
/// 存在 `external-agent-sessions/imported-<conv_id>.json`，与会话绑定文件放在一起。
/// 打开对话时拿 `source_path` 现在的 mtime / 条数与这里记的比，对不上就提示"CLI 那边有新内容"。
/// **只提示，不同步。**
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRecord {
    pub conversation_id: String,
    pub agent_id: String,
    pub session_id: String,
    /// 历史来源文件；走 ACP 重放的（opencode）没有文件，为 `None`。
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub source_mtime_ms: Option<i64>,
    #[serde(default)]
    pub message_count: usize,
    pub imported_at: i64,
}
