//! Native host for the existing Codex protocol, with isolated settings and durable binding.
use super::{
    attachments,
    session::{codex_app_server::CodexAppServerSession, live::ApprovalBridge},
    types::UnifiedAgentEvent,
};
use crate::conversation_data::Conversation;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::{mpsc, oneshot};

pub struct Launch {
    pub binary: PathBuf,
    pub env: Vec<crate::settings_config::CliEnvVar>,
    pub cwd: PathBuf,
    pub instructions: String,
}
#[derive(Serialize, Deserialize)]
pub struct Binding {
    #[serde(flatten)]
    pub session: super::types::ExternalAgentSession,
    pub cwd: PathBuf,
}
fn binding_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    crate::conversation_files::validate_conversation_id(id)?;
    Ok(root
        .join("external-agent-sessions")
        .join(format!("{id}.json")))
}
pub fn binding(root: &Path, id: &str) -> Result<Option<Binding>, String> {
    match std::fs::read(binding_path(root, id)?) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("CLI 会话记录损坏：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
impl Launch {
    pub fn prepare(
        root: &Path,
        id: &str,
        agent: &str,
        project: Option<&str>,
        new_chat: bool,
    ) -> Result<Self, String> {
        let binary = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|p| p.join(agent))
            .find(|p| p.is_file());
        Self::prepare_resolved(root, id, agent, project, new_chat, binary)
    }
    pub fn prepare_resolved(
        root: &Path,
        id: &str,
        agent: &str,
        project: Option<&str>,
        new_chat: bool,
        discovered: Option<PathBuf>,
    ) -> Result<Self, String> {
        let settings = crate::native_preferences::load(root)?;
        let config = settings
            .chat
            .external_cli_agents
            .get(agent)
            .cloned()
            .unwrap_or_default();
        if new_chat && config.disabled {
            return Err("此 CLI 已停用".into());
        }
        if !config.current_provider.trim().is_empty() {
            return Err("第三方供应商配置尚未接入此运行入口；未改用其他账号".into());
        }
        let binary = if config.path.trim().is_empty() {
            discovered.ok_or_else(|| format!("找不到 {agent} CLI，请配置可执行文件路径"))?
        } else {
            PathBuf::from(config.path.trim())
        };
        if !binary.is_file() {
            return Err("CLI 路径不存在".into());
        }
        let binary = binary.canonicalize().map_err(|e| e.to_string())?;
        if super::wsl::is_wsl_target(&binary) {
            return Err("原生版本的 WSL 启动尚未接入".into());
        }
        if let Some(saved) = binding(root, id)? {
            if saved.session.agent_id != agent || saved.session.conversation_id != id {
                return Err("CLI 会话绑定不匹配".into());
            }
        }
        let cwd = workspace(root, id, project)?;
        Ok(Self {
            binary,
            env: config.env,
            cwd,
            instructions: settings.chat.system_prompt,
        })
    }
}

/// A failed resume never silently starts a fresh thread or uses an API provider.
pub async fn run(
    root: &Path,
    conversation: &Conversation,
    launch: Launch,
    events: mpsc::Sender<UnifiedAgentEvent>,
    mut cancel: oneshot::Receiver<()>,
    mut approvals: Option<ApprovalBridge>,
) -> Result<(), String> {
    let stored = binding(root, &conversation.id)?;
    if stored.is_none()
        && conversation.messages.iter().any(|m| {
            m.role == "assistant"
                && !matches!(m.stream_outcome.as_deref(), Some("error" | "cancelled"))
        })
    {
        return Err("缺少此对话的 CLI 原生会话绑定，无法安全续聊".into());
    }
    let mut command =
        super::spawn::agent_cli_command(&super::defs::codex::CODEX_AGENT_DEF, &launch.binary);
    for entry in &launch.env {
        if !entry.key.trim().is_empty() {
            command.env(&entry.key, &entry.value);
        }
    }
    super::process_io::strip_parent_session_env(&mut command);
    let runtime = &conversation.agent_runtime;
    let model = runtime
        .external_model
        .as_deref()
        .filter(|m| !m.is_empty() && *m != "default");
    let mut args = vec!["app-server".into()];
    if let Some(tier) = runtime
        .external_service_tier
        .as_deref()
        .filter(|v| !v.is_empty())
    {
        args.extend([
            "-c".into(),
            format!("service_tier={}", serde_json::json!(tier)),
        ]);
    }
    let mut session = tokio::select! {
        _ = &mut cancel => return Err("cancelled".into()),
        result = CodexAppServerSession::connect_with_command(command, &launch.binary, &args, &launch.cwd, model,
            runtime.external_sandbox.as_deref(), stored.as_ref().map(|b| b.session.session_id.as_str())) => result?,
    };
    let path = binding_path(root, &conversation.id)?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let saved = Binding {
        cwd: launch.cwd,
        session: super::types::ExternalAgentSession {
            conversation_id: conversation.id.clone(),
            agent_id: "codex".into(),
            session_id: session.thread_id().into(),
            model: model.map(str::to_owned),
            stable_prompt_hash: None,
            recorded_prompt_hash: None,
        },
    };
    crate::file_io::atomic_write(
        &path,
        &serde_json::to_string_pretty(&saved).map_err(|e| e.to_string())?,
        "CLI session",
    )?;
    let user = conversation.messages.last().ok_or("缺少用户消息")?;
    let directory = root
        .join("conversations")
        .join(format!("{}_attachments", conversation.id));
    let paths = user
        .attachments
        .iter()
        .map(|a| {
            crate::attachment_files::resolve_attachment_file_path_in_dir(Some(&directory), &a.path)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (image_paths, files): (Vec<_>, Vec<_>) = paths
        .into_iter()
        .partition(|p| attachments::image_mime_for_path(p).is_some());
    let (images, degraded) = attachments::load_image_blocks(
        &image_paths,
        &["image/png", "image/jpeg", "image/webp", "image/gif"],
    );
    let mut prompt = user.content.clone();
    if !launch.instructions.trim().is_empty() {
        prompt = format!(
            "# Instructions (read first)\n\n{}\n\n# User request\n\n{prompt}",
            launch.instructions
        );
    }
    prompt.push_str(&attachments::file_attachments_note(&files));
    prompt.push_str(&attachments::image_paths_note(&degraded));
    let mut roots = if user.attachments.is_empty() {
        vec![]
    } else {
        vec![directory.to_string_lossy().into_owned()]
    };
    roots.extend(
        conversation
            .additional_directories
            .iter()
            .map(|d| d.path.trim())
            .filter(|p| !p.is_empty())
            .map(str::to_owned),
    );
    if !images.is_empty() {
        roots.push(std::env::temp_dir().to_string_lossy().into_owned());
    }
    let (control_tx, mut control) = mpsc::channel(4);
    let result = {
        let turn = session.run_turn(
            &prompt,
            model,
            runtime.external_reasoning.as_deref(),
            &images,
            &roots,
            &events,
            &mut control,
            approvals.as_mut(),
        );
        tokio::pin!(turn);
        tokio::select! {
            _ = &mut cancel => {
                let _ = control_tx.send(super::session::live::SessionCommand::Cancel).await;
                let _ = tokio::time::timeout(std::time::Duration::from_secs(3), &mut turn).await;
                Err("cancelled".into())
            }
            result = &mut turn => result,
        }
    };
    session.close().await;
    result
}

pub fn save_binding(root: &Path, binding: &Binding) -> Result<(), String> {
    crate::file_io::atomic_write(
        &binding_path(root, &binding.session.conversation_id)?,
        &serde_json::to_string_pretty(binding).map_err(|e| e.to_string())?,
        "CLI session",
    )
}

/// The same workspace is used by catalogs, chat display, and CLI launch.
pub fn workspace(root: &Path, id: &str, project: Option<&str>) -> Result<PathBuf, String> {
    let settings = crate::native_preferences::load(root)?;
    let saved = binding(root, id)?;
    let cwd = if let Some(saved) = saved {
        saved.cwd
    } else if let Some(project) = project {
        let index = crate::group_catalog::load_project_index(&root.join("conversations"))?;
        index
            .projects
            .iter()
            .find(|p| p.id == project)
            .and_then(|p| p.root_path.as_ref())
            .filter(|p| !p.trim().is_empty())
            .map(PathBuf::from)
            .ok_or("项目没有有效的工作目录")?
    } else {
        let configured = settings.chat_tools.native_tools.working_directory.trim();
        if configured.is_empty() {
            root.join("workspace")
        } else {
            PathBuf::from(configured)
        }
    };
    if project.is_none() && !cwd.exists() {
        std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
    }
    let cwd = cwd
        .canonicalize()
        .map_err(|e| format!("工作目录不可用：{e}"))?;
    if !cwd.is_dir() {
        return Err("工作目录不是文件夹".into());
    }
    Ok(cwd)
}
