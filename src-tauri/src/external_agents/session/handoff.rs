//! A local native thread may be resumed only from the exact visible prefix it last answered.
use std::path::{Path, PathBuf};

use crate::chat::types::{ChatMessage, Conversation, ExternalHistoryMethod};
use crate::external_agents::types::{ExternalAgentSession, RuntimeAgentDef, StreamFormat};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::LiveSessionHandle;

#[derive(Serialize, Deserialize)]
struct Checkpoint {
    conversation_id: String,
    #[serde(flatten)]
    handle: LiveSessionHandle,
    session: Option<ExternalAgentSession>,
    seen: usize,
    prefix_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    portable_digest: Option<String>,
    clear_id: Option<String>,
    selections: std::collections::HashMap<String, String>,
}

pub(super) fn checkpoint_path(dir: &Path, conversation_id: &str, agent: &str) -> PathBuf {
    dir.join(format!("handoff-{agent}-{conversation_id}.json"))
}

/// Native history migration moves exactly these two wire transcripts into the SDK tree.
/// Hash the remaining UI rows on departure/return only, without copying attachments or tools.
fn portable_digest(messages: &mut [ChatMessage]) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"[");
    for (index, message) in messages.iter_mut().enumerate() {
        if index > 0 {
            digest.update(b",");
        }
        let api = std::mem::take(&mut message.api_messages);
        let model = std::mem::take(&mut message.model_messages);
        let result = serde_json::to_writer(&mut digest, &*message);
        message.api_messages = api;
        message.model_messages = model;
        result.map_err(|error| error.to_string())?;
    }
    digest.update(b"]");
    Ok(format!("{:x}", digest.finalize()))
}

/// Bind the portable representation only to a prefix previously proven to be this CLI's own.
pub(crate) fn park_for_switch(
    app: &AppHandle,
    chat: &mut Conversation,
    agent: &str,
) -> Result<(), String> {
    let Some(def) = crate::external_agents::registry::get_agent_def(agent) else {
        return Ok(());
    };
    let path = checkpoint_path(&super::sessions_dir(app)?, &chat.id, def.id);
    let Ok(raw) = std::fs::read(&path) else {
        return Ok(());
    };
    let Ok(mut checkpoint) = serde_json::from_slice::<Checkpoint>(&raw) else {
        return Ok(());
    };
    if checkpoint.seen == chat.messages.len()
        && crate::chat::history_projection::settled_message_digest(app, &chat.id, chat.revision)
            .as_deref()
            == Some(&checkpoint.prefix_digest)
    {
        checkpoint.portable_digest = Some(portable_digest(&mut chat.messages)?);
        crate::chat::storage::atomic_write(
            &path,
            &serde_json::to_string(&checkpoint).map_err(|e| e.to_string())?,
            "provider handoff binding",
        )?;
    }
    Ok(())
}

impl Checkpoint {
    fn matches(
        &self,
        chat: &mut Conversation,
        agent: &str,
        cwd: &Path,
        through: usize,
    ) -> Result<bool, String> {
        let Some(def) = crate::external_agents::registry::get_agent_def(agent) else {
            return Ok(false);
        };
        let binding_matches = self
            .session
            .as_ref()
            .map_or(!def.resumes_session_via_cli, |s| {
                s.conversation_id == chat.id
                    && s.agent_id == agent
                    && s.session_id == self.handle.native_id
            });
        Ok(self.conversation_id == chat.id
            && binding_matches
            && self.handle.can_resume(
                agent,
                crate::external_agents::run::persistent_protocol_tag(def.stream_format),
            )
            && crate::external_agents::import::paths_match(
                &self.handle.cwd,
                &cwd.to_string_lossy(),
            )
            && self.clear_id.as_deref()
                == chat
                    .context_state
                    .clear_boundaries
                    .last()
                    .map(|clear| clear.id.as_str())
            && self.seen <= through
            && chat.messages.get(..self.seen).is_some()
            && chat.messages[..self.seen]
                .iter()
                .filter_map(|m| m.group_id.as_ref())
                .all(|id| self.selections.get(id) == chat.group_selections.get(id))
            && if let Some(portable) = &self.portable_digest {
                portable_digest(&mut chat.messages[..self.seen])? == *portable
            } else {
                use sha2::{Digest, Sha256};
                let mut digest = Sha256::new();
                serde_json::to_writer(&mut digest, &chat.messages[..self.seen])
                    .map_err(|e| e.to_string())?;
                format!("{:x}", digest.finalize()) == self.prefix_digest
            })
    }
}

/// The normal send owns the conversation reservation. An edit, rewind or change to a chosen
/// answer after this CLI's last settled turn means its native copy can no longer be certified.
pub(crate) fn check_before_user_append(app: &AppHandle, chat: &Conversation) -> Result<(), String> {
    let Some(agent) = chat
        .agent_runtime
        .external_agent_id
        .as_deref()
        .and_then(crate::external_agents::registry::get_agent_def)
    else {
        return Ok(());
    };
    if chat.external_history.is_some() {
        return Ok(());
    }
    let path = checkpoint_path(&super::sessions_dir(app)?, &chat.id, agent.id);
    let Ok(raw) = std::fs::read(&path) else {
        return Ok(());
    };
    let intact = serde_json::from_slice::<Checkpoint>(&raw)
        .ok()
        .is_some_and(|c| {
            c.seen == chat.messages.len()
                && c.selections == chat.group_selections
                && crate::chat::history_projection::settled_message_digest(
                    app,
                    &chat.id,
                    chat.revision,
                )
                .as_deref()
                    == Some(&c.prefix_digest)
        });
    if !intact {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A fresh first turn or a verified full/delta handoff can certify a new native thread.
/// For an ordinary continuation, the completed turn must still belong to the previously proven ID.
pub(crate) enum Coverage {
    Fresh,
    SameThread(String),
    Unverified,
}

/// Until the next answer is durably settled this thread may have received unacknowledged input.
/// Consuming the checkpoint before dispatch also makes interrupted delta delivery retry fresh.
pub(crate) fn take_for_turn(
    app: &AppHandle,
    chat: &mut Conversation,
    def: &RuntimeAgentDef,
    cwd: &Path,
    slash: bool,
) -> Result<(Option<(String, usize)>, Coverage), String> {
    let initial = if chat.messages.len() == 1 || chat.external_history.is_some() {
        Coverage::Fresh
    } else {
        Coverage::Unverified
    };
    let path = checkpoint_path(&super::sessions_dir(app)?, &chat.id, def.id);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((None, initial)),
        Err(error) => return Err(format!("Read provider handoff binding: {error}")),
    };
    let returning = chat
        .external_history
        .as_ref()
        .filter(|h| h.method == ExternalHistoryMethod::ProviderHandoff)
        .cloned();
    if slash && returning.is_some() {
        // The command runs on a separate fresh session, leaving delivery to the next ordinary turn.
        return Ok((None, initial));
    }
    let through = returning
        .as_ref()
        .map(|h| h.prefix(chat).map(<[_]>::len))
        .transpose()?
        .unwrap_or_default();
    let mut result = (None, initial);
    if let Ok(checkpoint) = serde_json::from_slice::<Checkpoint>(&raw) {
        match returning {
            Some(transition)
                if transition.allow_provider_resume
                    && checkpoint.matches(chat, def.id, cwd, through)? =>
            {
                // Publish the active binding before removing its parked import claim.
                super::forget_native_session(app, &chat.id)?;
                if let Some(session) = &checkpoint.session {
                    super::save_session(app, session)?;
                }
                super::save_live_handle(app, &chat.id, &checkpoint.handle)?;
                result = (
                    Some((checkpoint.handle.native_id, checkpoint.seen)),
                    Coverage::Fresh,
                );
            }
            None if chat.messages.len() == checkpoint.seen + 1
                && chat.external_history.is_none() =>
            {
                result.1 = Coverage::SameThread(checkpoint.handle.native_id);
            }
            _ => {}
        }
    }
    std::fs::remove_file(path).map_err(|e| format!("Retire provider handoff binding: {e}"))?;
    Ok(result)
}

pub(crate) fn remember_answer(
    app: &AppHandle,
    chat: &Conversation,
    def: &RuntimeAgentDef,
    coverage: Coverage,
) -> Result<(), String> {
    if chat.runtime_locked || chat.external_history.is_some() {
        return Ok(());
    }
    let Some(handle) = super::load_live_handle(app, &chat.id)
        .filter(|h| h.agent_id == def.id && !h.native_id.is_empty())
    else {
        return Ok(());
    };
    match coverage {
        Coverage::Unverified => return Ok(()),
        Coverage::SameThread(expected) if expected != handle.native_id => return Ok(()),
        _ => {}
    }
    let Some(prefix_digest) =
        crate::chat::history_projection::settled_message_digest(app, &chat.id, chat.revision)
    else {
        return Ok(());
    };
    let session = super::load_session(app, &chat.id)
        .filter(|s| s.agent_id == def.id && s.session_id == handle.native_id);
    if def.resumes_session_via_cli && session.is_none() {
        return Ok(());
    }
    // ACP adapters negotiate loadSession per launch. An unconfirmed handle can only start fresh.
    if def.stream_format == StreamFormat::AcpJsonRpc && !handle.supports_load {
        return Ok(());
    }
    let checkpoint = Checkpoint {
        conversation_id: chat.id.clone(),
        handle,
        session,
        seen: chat.messages.len(),
        prefix_digest,
        portable_digest: None,
        selections: chat.group_selections.clone(),
        clear_id: chat
            .context_state
            .clear_boundaries
            .last()
            .map(|clear| clear.id.clone()),
    };
    let path = checkpoint_path(&super::sessions_dir(app)?, &chat.id, def.id);
    crate::chat::storage::atomic_write(
        &path,
        &serde_json::to_string(&checkpoint).map_err(|e| e.to_string())?,
        "provider handoff binding",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    #[test]
    fn provider_handoff_checkpoint_accepts_only_unchanged_native_prefix_and_cwd() {
        let cwd = tempfile::tempdir().unwrap();
        let mut chat: Conversation = serde_json::from_value(json!({
            "id":"c","title":"","provider_id":"a","model":"m","created_at":0,"updated_at":0,
            "messages":[{"id":"u","role":"user","content":"old","timestamp":0}]
        }))
        .unwrap();
        let mut record = Checkpoint {
            conversation_id: chat.id.clone(),
            seen: 1,
            clear_id: None,
            portable_digest: None,
            session: None,
            selections: Default::default(),
            handle: LiveSessionHandle {
                supports_load: true,
                agent_id: "codex".into(),
                protocol: "codex_app_server".into(),
                native_id: "thread-first".into(),
                native_path: None,
                cwd: cwd.path().to_string_lossy().into_owned(),
            },
            // The projection hashes the serialized JSON array exactly once at commit.
            prefix_digest: format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&chat.messages).unwrap())
            ),
        };
        chat.messages.push(
            serde_json::from_value(
                json!({"id":"other","role":"assistant","content":"new","timestamp":1}),
            )
            .unwrap(),
        );
        assert!(record.matches(&mut chat, "codex", cwd.path(), 2).unwrap());
        assert!(!record.matches(&mut chat, "codex", cwd.path(), 0).unwrap());
        assert!(!record
            .matches(&mut chat, "codex", &cwd.path().join("elsewhere"), 2)
            .unwrap());
        chat.context_state.clear_boundaries.push(
            serde_json::from_value(json!({
                "id":"cleared","source_until_message_id":"u","created_at":1
            }))
            .unwrap(),
        );
        assert!(!record.matches(&mut chat, "codex", cwd.path(), 2).unwrap());
        chat.context_state.clear_boundaries.clear();
        chat.messages[0]
            .api_messages
            .push(json!({"content":"old private wire text"}));
        record.portable_digest = Some(portable_digest(&mut chat.messages[..1]).unwrap());
        assert_eq!(chat.messages[0].api_messages.len(), 1);
        chat.messages[0].api_messages.clear();
        assert!(record.matches(&mut chat, "codex", cwd.path(), 2).unwrap());
        chat.messages[0].content = "edited".into();
        assert!(!record.matches(&mut chat, "codex", cwd.path(), 2).unwrap());
    }
}
