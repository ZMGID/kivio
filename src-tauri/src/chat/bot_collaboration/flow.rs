//! Shared submit path for Tauri commands and conversation tools.

use tauri::AppHandle;

use super::runtime::{
    actor_for_bot, actor_for_conversation, now_secs, publish, require_bot, try_service,
};
use super::types::{
    normalize_attachment_ids, Actor, BroadcastReceipt, Delivery, DeliveryKind, Enqueue,
    MemberInput, MemberSession, PendingAction, PendingKind, Room, RoomMember,
};

pub fn submit(
    app: &AppHandle,
    from: Actor,
    to_bot_id: &str,
    to_name_hint: Option<String>,
    room_id: Option<String>,
    kind: DeliveryKind,
    body: String,
    exchange_id: Option<String>,
    parent_delivery_id: Option<String>,
    approved: bool,
    attachment_ids: Vec<String>,
    source_conversation_id: Option<&str>,
) -> Result<Delivery, String> {
    let service = try_service(app)?;
    let now = now_secs();
    let attachment_ids = normalize_attachment_ids(&attachment_ids)?;
    let attachment_sources = if attachment_ids.is_empty() {
        Vec::new()
    } else {
        attachment_conversations(app, &from, room_id.as_deref(), source_conversation_id)?
    };
    require_known_attachments(app, &attachment_ids, &attachment_sources)?;
    let resolved = match require_bot(app, to_bot_id) {
        Ok(member) => member,
        Err(reason) => {
            let delivery = service.record_unavailable(
                Enqueue {
                    from,
                    to_bot_id: to_bot_id.trim().to_string(),
                    to_name: to_name_hint.unwrap_or_else(|| to_bot_id.trim().to_string()),
                    room_id,
                    kind,
                    body,
                    exchange_id,
                    parent_delivery_id,
                    sensitive: kind == DeliveryKind::Handoff,
                    approved,
                    attachment_ids: attachment_ids.clone(),
                    attachment_sources: attachment_sources.clone(),
                },
                reason,
                now,
            )?;
            publish(app);
            return Ok(delivery);
        }
    };
    let delivery = service.enqueue(
        Enqueue {
            from,
            to_bot_id: resolved.bot_id,
            to_name: resolved.name,
            room_id,
            kind,
            body,
            exchange_id,
            parent_delivery_id,
            sensitive: kind == DeliveryKind::Handoff,
            approved,
            attachment_ids,
            attachment_sources,
        },
        now,
    )?;
    publish(app);
    Ok(delivery)
}

pub fn members_from_ids(app: &AppHandle, bot_ids: &[String]) -> Result<Vec<MemberInput>, String> {
    bot_ids.iter().map(|id| require_bot(app, id)).collect()
}

pub async fn create_room(
    app: &AppHandle,
    title: String,
    bot_ids: Vec<String>,
) -> Result<Room, String> {
    let members = members_from_ids(app, &bot_ids)?;
    let room = try_service(app)?.create_room(&title, members, now_secs())?;
    if let Err(error) = ensure_member_threads(app, &room).await {
        let _ = try_service(app)?.discard_room(&room.id);
        return Err(error);
    }
    Ok(room)
}

pub async fn set_members(
    app: &AppHandle,
    room_id: String,
    bot_ids: Vec<String>,
) -> Result<Room, String> {
    let members = members_from_ids(app, &bot_ids)?;
    let room = try_service(app)?.set_members(&room_id, members, now_secs())?;
    ensure_member_threads(app, &room).await?;
    Ok(room)
}

pub async fn ensure_member_threads(app: &AppHandle, room: &Room) -> Result<(), String> {
    for member in &room.members {
        open_member_thread(app, room, member).await?;
    }
    Ok(())
}

pub async fn open_member_thread(
    app: &AppHandle,
    room: &Room,
    member: &RoomMember,
) -> Result<bool, String> {
    if crate::chat::bot_profile::assistant_id_from_member_conversation(&member.conversation_id)
        .is_none()
    {
        return Err(format!(
            "群成员会话「{}」不是 conv_bot_member_{{助手}}@{{房间}}",
            member.conversation_id
        ));
    }
    if crate::chat::storage::load_conversation(app, &member.conversation_id).is_ok() {
        return Ok(false);
    }
    let private =
        crate::chat::storage::load_conversation(app, &super::bot_conversation_id(&member.bot_id))?;
    let now = now_secs();
    let conversation = crate::chat::types::Conversation {
        id: member.conversation_id.clone(),
        revision: 0,
        goal_state: None,
        title: format!("{} · {}", room.title, member.name),
        provider_id: private.provider_id,
        model: private.model,
        messages: Vec::new(),
        agent_runtime: crate::chat::AgentRuntimeConfig::default(),
        active_skill_id: None,
        assistant_id: Some(member.bot_id.clone()),
        assistant_snapshot: private.assistant_snapshot,
        created_at: now,
        updated_at: now,
        pinned: false,
        archived: false,
        folder: private.folder,
        project_id: private.project_id,
        set_id: None,
        context_state: Default::default(),
        agent_todo_state: Default::default(),
        agent_plan_state: Default::default(),
        knowledge_base_ids: Vec::new(),
        force_knowledge_search: false,
        additional_directories: Vec::new(),
        thinking_level: private.thinking_level,
        web_search_mode: private.web_search_mode,
        reply_models: Vec::new(),
        group_selections: Default::default(),
        forked_from: None,
    };
    if let Err(error) = crate::chat::repository::repository(app)
        .create(app, conversation)
        .await
    {
        let message = crate::chat::repository::repository_error(error);
        if !message.contains("already exists") {
            return Err(message);
        }
        return Ok(false);
    }
    Ok(true)
}

pub fn request_members(
    app: &AppHandle,
    room_id: &str,
    add: &[String],
    remove: &[String],
) -> Result<Vec<PendingAction>, String> {
    let service = try_service(app)?;
    let now = now_secs();
    let mut actions = Vec::new();
    for bot_id in add {
        let member = require_bot(app, bot_id)?;
        actions.push(service.request_member_change(
            room_id,
            PendingKind::AddMember,
            &member.bot_id,
            &member.name,
            now,
        )?);
    }
    for bot_id in remove {
        let name = require_bot(app, bot_id)
            .map(|member| member.name)
            .unwrap_or_else(|_| bot_id.clone());
        actions.push(service.request_member_change(
            room_id,
            PendingKind::RemoveMember,
            bot_id,
            &name,
            now,
        )?);
    }
    if actions.is_empty() {
        return Err("没有要变更的成员".into());
    }
    Ok(actions)
}

pub fn actor_from_command(app: &AppHandle, from_bot_id: Option<String>) -> Result<Actor, String> {
    match from_bot_id {
        Some(bot_id) => {
            let member = require_bot(app, &bot_id)?;
            Ok(actor_for_bot(app, &member.bot_id, member.name))
        }
        None => Ok(Actor::User),
    }
}

pub fn actor_from_tool(app: &AppHandle, conversation_id: &str) -> Result<Actor, String> {
    actor_for_conversation(app, conversation_id)
}

pub fn rename_room(app: &AppHandle, room_id: &str, title: &str) -> Result<Room, String> {
    let room = try_service(app)?.rename_room(room_id, title, now_secs())?;
    publish(app);
    Ok(room)
}

pub fn broadcast(
    app: &AppHandle,
    from: Actor,
    room_id: &str,
    body: String,
    attachment_ids: Vec<String>,
    exchange_id: Option<String>,
    parent_delivery_id: Option<String>,
    source_conversation_id: Option<&str>,
) -> Result<BroadcastReceipt, String> {
    let attachment_ids = normalize_attachment_ids(&attachment_ids)?;
    let attachment_sources = if attachment_ids.is_empty() {
        Vec::new()
    } else {
        attachment_conversations(app, &from, Some(room_id), source_conversation_id)?
    };
    require_known_attachments(app, &attachment_ids, &attachment_sources)?;
    let receipt = try_service(app)?.broadcast(
        from,
        room_id,
        body,
        attachment_ids,
        attachment_sources,
        exchange_id,
        parent_delivery_id,
        now_secs(),
    )?;
    publish(app);
    Ok(receipt)
}

pub async fn member_session(
    app: &AppHandle,
    room_id: &str,
    bot_id: &str,
) -> Result<MemberSession, String> {
    let bot = require_bot(app, bot_id)?;
    let conversation_id = crate::chat::bot_profile::member_conversation_id(&bot.bot_id, room_id)?;
    let room = try_service(app)?.align_member_thread(
        room_id,
        &bot.bot_id,
        &conversation_id,
        now_secs(),
    )?;
    let member = room
        .members
        .iter()
        .find(|member| member.bot_id == bot.bot_id)
        .cloned()
        .ok_or_else(|| format!("Bot「{}」不是这个房间的成员", bot.bot_id))?;
    let created = open_member_thread(app, &room, &member).await?;
    publish(app);
    Ok(MemberSession {
        room_id: room.id,
        bot_id: member.bot_id,
        name: member.name,
        assistant_id: crate::chat::bot_profile::assistant_id_from_member_conversation(
            &member.conversation_id,
        )
        .map(str::to_string),
        conversation_id: member.conversation_id,
        created,
    })
}

pub(crate) struct PreparedAttachments {
    pub paths: Vec<String>,
    pub text: Vec<crate::chat::attachments::TextAttachmentInput>,
    temps: Vec<std::path::PathBuf>,
}

impl Drop for PreparedAttachments {
    fn drop(&mut self) {
        for path in &self.temps {
            let _ = std::fs::remove_file(path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
}

pub(crate) enum ResolvedAttachment {
    File(std::path::PathBuf),
    Text { name: String, content: String },
}

/// Resolves a stored attachment to the bytes or text the normal send path copies.
/// A relative path stays inside that conversation's attachment directory.
pub(crate) fn resolve_stored_attachment(
    attachment: &crate::chat::types::Attachment,
    attachments_dir: &std::path::Path,
) -> Result<ResolvedAttachment, String> {
    if attachment.path.starts_with("memory://") {
        let content = attachment
            .content
            .clone()
            .filter(|text| !text.is_empty())
            .ok_or_else(|| format!("附件「{}」没有正文", attachment.id))?;
        let name = {
            let trimmed = attachment.name.trim();
            if trimmed.is_empty() {
                "attachment.txt".to_string()
            } else {
                trimmed.to_string()
            }
        };
        return Ok(ResolvedAttachment::Text { name, content });
    }
    let stored = std::path::Path::new(&attachment.path);
    if stored
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!("附件「{}」的路径无效", attachment.id));
    }
    let source = if stored.is_absolute() {
        stored.to_path_buf()
    } else {
        attachments_dir.join(stored)
    };
    if !source.exists() {
        return Err(format!("附件「{}」的文件不存在", attachment.id));
    }
    Ok(ResolvedAttachment::File(source))
}

pub(crate) fn prepare_delivery_attachments(
    app: &AppHandle,
    delivery: &Delivery,
) -> Result<PreparedAttachments, String> {
    if delivery.attachment_ids.is_empty() {
        return Ok(PreparedAttachments {
            paths: Vec::new(),
            text: Vec::new(),
            temps: Vec::new(),
        });
    }
    let room = match delivery.room_id.as_deref() {
        Some(room_id) => Some(try_service(app)?.room(room_id)?),
        None => None,
    };
    // Persisted ids are not a grant. Each one is checked against the sender again.
    let conversations =
        scoped_attachment_sources(&delivery.from, room.as_ref(), &delivery.attachment_sources)?;
    if conversations.is_empty() {
        return Err("附件没有可读取的来源会话，没有扫描其他 Bot 的私聊".into());
    }
    let mut prepared = PreparedAttachments {
        paths: Vec::new(),
        text: Vec::new(),
        temps: Vec::new(),
    };
    for id in &delivery.attachment_ids {
        let (attachment, dir) = find_attachment(app, id, &conversations)?;
        match resolve_stored_attachment(&attachment, &dir)? {
            ResolvedAttachment::Text { name, content } => {
                prepared
                    .text
                    .push(crate::chat::attachments::TextAttachmentInput { name, content });
            }
            ResolvedAttachment::File(source) => {
                let staged = stage_named_copy(&source, &attachment)?;
                if staged != source {
                    prepared.temps.push(staged.clone());
                }
                prepared.paths.push(staged.display().to_string());
            }
        }
    }
    Ok(prepared)
}

fn find_attachment(
    app: &AppHandle,
    id: &str,
    conversations: &[String],
) -> Result<(crate::chat::types::Attachment, std::path::PathBuf), String> {
    for conversation_id in conversations {
        let Ok(conversation) = crate::chat::storage::load_conversation(app, conversation_id) else {
            continue;
        };
        let Some(attachment) = conversation.messages.iter().find_map(|message| {
            message
                .attachments
                .iter()
                .find(|attachment| attachment.id == id)
                .cloned()
        }) else {
            continue;
        };
        let dir = crate::chat::storage::conversation_attachments_dir(app, conversation_id)?;
        return Ok((attachment, dir));
    }
    Err(format!("附件「{id}」不存在"))
}

fn stage_named_copy(
    source: &std::path::Path,
    attachment: &crate::chat::types::Attachment,
) -> Result<std::path::PathBuf, String> {
    if source.is_dir() {
        return Ok(source.to_path_buf());
    }
    let name = display_file_name(&attachment.name);
    let dir = std::env::temp_dir().join(format!("kivio-collab-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|err| format!("无法准备附件：{err}"))?;
    let dest = dir.join(name);
    std::fs::copy(source, &dest)
        .map_err(|err| format!("无法准备附件「{}」：{err}", attachment.id))?;
    Ok(dest)
}

fn display_file_name(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    let sanitized: String = base
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches(['.', ' ', '_']).trim();
    if trimmed.is_empty() {
        "attachment".to_string()
    } else {
        trimmed.to_string()
    }
}

fn attachment_conversations(
    app: &AppHandle,
    from: &Actor,
    room_id: Option<&str>,
    source_conversation_id: Option<&str>,
) -> Result<Vec<String>, String> {
    let room = room_id.map(|id| try_service(app)?.room(id)).transpose()?;
    authorized_attachment_sources(from, room.as_ref(), source_conversation_id)
}

/// A Bot may read only its own private chat and its own member thread in this
/// room. An explicit source must be one of those two. The recipient private
/// chat, every other member's private chat, and every other member thread are
/// never searched.
///
/// A direct user may name one source conversation. With no name, nothing is
/// scanned.
fn authorized_attachment_sources(
    from: &Actor,
    room: Option<&Room>,
    explicit_source: Option<&str>,
) -> Result<Vec<String>, String> {
    let owned = sender_owned_sources(from, room);
    if let Some(source) = explicit_source
        .map(str::trim)
        .filter(|source| !source.is_empty())
    {
        if from.bot_id().is_some() && !owned.iter().any(|id| id == source) {
            return Err(
                "不能读取其他 Bot 的私聊或成员线程附件；只能转发自己的私聊或自己的群成员线程中的附件"
                    .into(),
            );
        }
        return Ok(vec![source.to_string()]);
    }
    if from.bot_id().is_some() {
        return Ok(owned);
    }
    Err("发送附件需要指定来源会话，不会扫描 Bot 私聊或其他成员会话".into())
}

fn sender_owned_sources(from: &Actor, room: Option<&Room>) -> Vec<String> {
    let Some(bot_id) = from.bot_id() else {
        return Vec::new();
    };
    let mut ids = vec![super::bot_conversation_id(bot_id)];
    if let Some(member) = room.and_then(|room| {
        room.members.iter().find(|member| {
            member.bot_id == bot_id && member.state == super::types::MemberState::Active
        })
    }) {
        ids.push(member.conversation_id.clone());
    }
    ids.sort();
    ids.dedup();
    ids
}

/// Re-checks every stored source against the sender's current scope.
/// An empty list uses that scope and does not fall back to other private chats.
fn scoped_attachment_sources(
    from: &Actor,
    room: Option<&Room>,
    persisted: &[String],
) -> Result<Vec<String>, String> {
    if persisted.is_empty() {
        return authorized_attachment_sources(from, room, None);
    }
    let owned = sender_owned_sources(from, room);
    let is_bot = from.bot_id().is_some();
    let mut scoped = Vec::with_capacity(persisted.len());
    for source in persisted {
        let source = source.trim();
        if source.is_empty() || (is_bot && !owned.iter().any(|id| id == source)) {
            return Err("持久投递的附件来源超出发送方可读范围，没有读取其他 Bot 的私聊".into());
        }
        if !scoped.iter().any(|id| id == source) {
            scoped.push(source.to_string());
        }
    }
    Ok(scoped)
}

fn require_known_attachments(
    app: &AppHandle,
    ids: &[String],
    conversations: &[String],
) -> Result<(), String> {
    for id in ids {
        let found = conversations.iter().any(|conversation_id| {
            crate::chat::storage::load_conversation(app, conversation_id)
                .ok()
                .is_some_and(|conversation| {
                    conversation.messages.iter().any(|message| {
                        message
                            .attachments
                            .iter()
                            .any(|attachment| attachment.id == *id)
                    })
                })
        });
        if !found {
            return Err(format!("附件「{id}」不存在"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod attachment_scope_tests {
    use super::*;
    use crate::chat::bot_collaboration::types::{MemberState, RoomStatus};

    fn room_with(ava: &str, bo: &str) -> Room {
        let members = [ava, bo]
            .into_iter()
            .map(|id| RoomMember {
                bot_id: id.into(),
                name: id.into(),
                conversation_id: crate::chat::bot_profile::member_conversation_id(id, "room_one")
                    .unwrap(),
                state: MemberState::Active,
                joined_at: 1,
            })
            .collect();
        Room {
            id: "room_one".into(),
            title: "Work".into(),
            status: RoomStatus::Active,
            cycle: 1,
            members,
            log: Vec::new(),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn cross_bot_attachment_source_is_refused_and_own_group_payload_is_forwarded() {
        let room = room_with("asst_ava", "asst_bo");
        let ava = Actor::bot("asst_ava", "Ava");
        let own_private = "conv_bot_asst_ava";
        let own_member = room.members[0].conversation_id.clone();
        let bo_private = "conv_bot_asst_bo";
        let bo_member = room.members[1].conversation_id.clone();

        let allowed = authorized_attachment_sources(&ava, Some(&room), None).unwrap();
        assert_eq!(
            allowed
                .iter()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            [own_private, own_member.as_str()].into_iter().collect(),
        );
        assert!(!allowed
            .iter()
            .any(|id| id == bo_private || id == &bo_member));

        for source in [bo_private, bo_member.as_str(), "conv_unrelated"] {
            let refused = authorized_attachment_sources(&ava, Some(&room), Some(source));
            assert!(refused.is_err(), "{source}");
            let persisted = scoped_attachment_sources(&ava, Some(&room), &[source.to_string()]);
            assert!(persisted.is_err(), "persisted {source}");
        }
        assert!(authorized_attachment_sources(&ava, None, Some(&own_member)).is_err());
        assert!(scoped_attachment_sources(
            &ava,
            Some(&room),
            &[own_member.clone(), bo_private.into()],
        )
        .is_err());

        let scoped = scoped_attachment_sources(&ava, Some(&room), &[own_member.clone()]).unwrap();
        assert_eq!(scoped, vec![own_member]);

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("draft.txt"), "own group payload").unwrap();
        let attachment = crate::chat::types::Attachment {
            id: "att_guessed_must_not_matter".into(),
            attachment_type: "file".into(),
            name: "draft.txt".into(),
            path: "draft.txt".into(),
            content: None,
        };
        match resolve_stored_attachment(&attachment, dir.path()).unwrap() {
            ResolvedAttachment::File(path) => {
                assert_eq!(std::fs::read_to_string(path).unwrap(), "own group payload");
            }
            ResolvedAttachment::Text { .. } => panic!("expected the member-thread file"),
        }

        assert!(authorized_attachment_sources(&Actor::User, Some(&room), None).is_err());
        assert_eq!(
            authorized_attachment_sources(&Actor::User, Some(&room), Some("conv_user_choice"))
                .unwrap(),
            vec!["conv_user_choice".to_string()],
        );
        assert!(scoped_attachment_sources(&Actor::User, Some(&room), &[]).is_err());
    }

    #[test]
    fn removal_revokes_member_attachment_access_without_revoking_own_private_access() {
        let mut room = room_with("asst_ava", "asst_bo");
        let member_id = room.members[0].conversation_id.clone();
        let from = Actor::bot("asst_ava", "Ava");
        assert_eq!(
            scoped_attachment_sources(&from, Some(&room), &[member_id.clone()]).unwrap(),
            vec![member_id.clone()],
        );
        room.members[0].state = MemberState::PendingRemoval;
        assert!(scoped_attachment_sources(&from, Some(&room), &[member_id]).is_err());
        assert_eq!(
            scoped_attachment_sources(&from, Some(&room), &["conv_bot_asst_ava".into()]).unwrap(),
            vec!["conv_bot_asst_ava".to_string()],
        );
        assert!(scoped_attachment_sources(&Actor::User, None, &[" ".into()]).is_err());
    }
}
