use std::path::Path;

use super::{truncate_title, ImportableSession};
use crate::external_agents::import_history::{parse_acp_updates, ImportedMessage};
use crate::external_agents::providers::antigravity::ANTIGRAVITY_AGENT_DEF;
use crate::external_agents::session::acp::{
    probe_acp_session_history, probe_acp_sessions, AcpSessionSummary,
};
use crate::external_agents::spawn::resolve_binary;

pub(super) async fn list_sessions(project_root: &str) -> Result<Vec<ImportableSession>, String> {
    let bin = resolve_binary(&ANTIGRAVITY_AGENT_DEF)
        .await
        .ok_or_else(|| "找不到 Antigravity CLI（agy）".to_string())?;
    let sessions = probe_acp_sessions(&bin, &["acp"], Path::new(project_root), 25)
        .await
        .ok_or_else(|| "当前 Antigravity CLI 无法通过 ACP 枚举原生会话".to_string())?;
    Ok(sessions
        .into_iter()
        .filter_map(importable_session)
        .collect())
}

pub(super) async fn load_history(
    project_root: &str,
    session_id: &str,
) -> Result<Vec<ImportedMessage>, String> {
    let bin = resolve_binary(&ANTIGRAVITY_AGENT_DEF)
        .await
        .ok_or_else(|| "找不到 Antigravity CLI（agy）".to_string())?;
    let updates =
        probe_acp_session_history(&bin, &["acp"], Path::new(project_root), session_id, 60)
            .await
            .ok_or_else(|| "无法重放 Antigravity CLI 原生会话的历史".to_string())?;
    Ok(parse_acp_updates(&updates))
}

fn importable_session(summary: AcpSessionSummary) -> Option<ImportableSession> {
    let cwd = session_cwd(summary.cwd.as_deref()?)?;
    Some(ImportableSession {
        agent_id: ANTIGRAVITY_AGENT_DEF.id.into(),
        import_history: ANTIGRAVITY_AGENT_DEF.import.history_support(),
        session_id: summary.session_id,
        title: summary.title.as_deref().map(truncate_title),
        cwd,
        updated_at: summary.updated_at,
        message_count: None,
        already_imported: false,
        bound_conversation_id: None,
    })
}

fn session_cwd(cwd: &str) -> Option<String> {
    // agy 1.5.10 reports the escaped file URI path, e.g. /c%3A/Work/My%20Project.
    let uri = if cwd.starts_with("file://") {
        url::Url::parse(cwd).ok()?
    } else if cfg!(windows) && Path::new(cwd).is_absolute() {
        return Some(cwd.to_owned());
    } else if cwd.starts_with('/') {
        url::Url::parse(&format!("file://{cwd}")).ok()?
    } else {
        return None;
    };
    if uri.query().is_some() || uri.fragment().is_some() {
        return None;
    }
    if cfg!(windows) {
        let mut segments = uri.path_segments()?;
        if segments.next()?.is_empty() || (uri.host_str().is_none() && segments.next().is_none()) {
            return None;
        }
    }
    let path = uri.to_file_path().ok()?;
    path.into_os_string().into_string().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_require_the_original_absolute_working_directory() {
        for cwd in [None, Some(""), Some("relative/project")] {
            let session = importable_session(AcpSessionSummary {
                session_id: "native-session".into(),
                cwd: cwd.map(str::to_owned),
                title: Some("Original title".into()),
                updated_at: Some(123),
            });
            assert!(session.is_none());
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_and_unc_paths_preserve_original_directory_names() {
        assert_eq!(session_cwd("/c%3A/").as_deref(), Some(r"c:\"));
        assert_eq!(
            session_cwd("/c%3A/Work/My%20Project%25%20%E9%A1%B9%E7%9B%AE%23draft").as_deref(),
            Some(r"c:\Work\My Project% 项目#draft")
        );
        assert_eq!(
            session_cwd("file://server/share/My%20Project").as_deref(),
            Some(r"\\server\share\My Project")
        );
        assert_eq!(
            session_cwd(r"C:\Work\Literal%20Path").as_deref(),
            Some(r"C:\Work\Literal%20Path")
        );
        assert!(session_cwd("/c%3A/Work/project#unescaped").is_none());
        assert!(session_cwd("/c%3A").is_none());
        assert!(session_cwd("file://server/").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn unix_file_uri_paths_preserve_original_directory_names() {
        assert_eq!(
            session_cwd("/work/My%20Project%25%20%E9%A1%B9%E7%9B%AE%23draft").as_deref(),
            Some("/work/My Project% 项目#draft")
        );
        assert!(session_cwd("/work/project#unescaped").is_none());
    }
}
