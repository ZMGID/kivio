//! Tauri commands for peer delivery, rooms, and exchange query.

use tauri::AppHandle;

use super::flow::{
    actor_from_command, broadcast, create_room, member_session, rename_room, set_members, submit,
};
use super::runtime::{member_conversation_identity, now_secs, publish, try_service};
use super::types::{
    Actor, BroadcastReceipt, CollaborationSnapshot, Delivery, DeliveryKind, Exchange,
    MemberConversationIdentity, MemberSession, PendingAction, Room, RoomLogEntry,
};

#[tauri::command]
pub fn bot_collaboration_member_identity(
    app: AppHandle,
    conversation_id: String,
) -> Result<Option<MemberConversationIdentity>, String> {
    member_conversation_identity(&app, &conversation_id)
}

#[tauri::command]
pub fn bot_collaboration_snapshot(app: AppHandle) -> Result<CollaborationSnapshot, String> {
    Ok(try_service(&app)?.snapshot())
}

#[tauri::command]
pub fn bot_peer_send(
    app: AppHandle,
    to_bot_id: String,
    body: String,
    room_id: Option<String>,
    from_bot_id: Option<String>,
    attachment_ids: Option<Vec<String>>,
    source_conversation_id: Option<String>,
) -> Result<Delivery, String> {
    let from = actor_from_command(&app, from_bot_id)?;
    let delivery = submit(
        &app,
        from,
        &to_bot_id,
        None,
        room_id,
        DeliveryKind::Peer,
        body,
        None,
        None,
        true,
        attachment_ids.unwrap_or_default(),
        source_conversation_id.as_deref(),
    )?;
    publish(&app);
    Ok(delivery)
}

#[tauri::command]
pub fn bot_exchange_query(app: AppHandle, exchange_id: String) -> Result<Exchange, String> {
    try_service(&app)?.exchange(&exchange_id)
}

#[tauri::command]
pub async fn bot_room_create(
    app: AppHandle,
    title: String,
    bot_ids: Vec<String>,
) -> Result<Room, String> {
    let room = create_room(&app, title, bot_ids).await?;
    publish(&app);
    Ok(room)
}

#[tauri::command]
pub async fn bot_room_set_members(
    app: AppHandle,
    room_id: String,
    bot_ids: Vec<String>,
) -> Result<Room, String> {
    let room = set_members(&app, room_id, bot_ids).await?;
    publish(&app);
    Ok(room)
}

#[tauri::command]
pub fn bot_room_mention(
    app: AppHandle,
    room_id: String,
    to_bot_id: String,
    body: String,
    from_bot_id: Option<String>,
    attachment_ids: Option<Vec<String>>,
    source_conversation_id: Option<String>,
) -> Result<Delivery, String> {
    let from = actor_from_command(&app, from_bot_id)?;
    let delivery = submit(
        &app,
        from,
        &to_bot_id,
        None,
        Some(room_id),
        DeliveryKind::Mention,
        body,
        None,
        None,
        true,
        attachment_ids.unwrap_or_default(),
        source_conversation_id.as_deref(),
    )?;
    publish(&app);
    Ok(delivery)
}

#[tauri::command]
pub fn bot_room_handoff(
    app: AppHandle,
    room_id: String,
    to_bot_id: String,
    body: String,
    from_bot_id: Option<String>,
    attachment_ids: Option<Vec<String>>,
    source_conversation_id: Option<String>,
) -> Result<Delivery, String> {
    let from = actor_from_command(&app, from_bot_id)?;
    let delivery = submit(
        &app,
        from,
        &to_bot_id,
        None,
        Some(room_id),
        DeliveryKind::Handoff,
        body,
        None,
        None,
        true,
        attachment_ids.unwrap_or_default(),
        source_conversation_id.as_deref(),
    )?;
    publish(&app);
    Ok(delivery)
}

#[tauri::command]
pub fn bot_room_pause(app: AppHandle, room_id: String) -> Result<Room, String> {
    let room = try_service(&app)?.pause_room(&room_id, now_secs())?;
    publish(&app);
    Ok(room)
}

#[tauri::command]
pub fn bot_room_continue(app: AppHandle, room_id: String) -> Result<Room, String> {
    let room = try_service(&app)?.continue_room(&room_id, now_secs())?;
    publish(&app);
    Ok(room)
}

#[tauri::command]
pub fn bot_room_stop(app: AppHandle, room_id: String) -> Result<Room, String> {
    let room = try_service(&app)?.stop_room(&room_id, now_secs())?;
    publish(&app);
    Ok(room)
}

#[tauri::command]
pub fn bot_room_log(app: AppHandle, room_id: String) -> Result<Vec<RoomLogEntry>, String> {
    Ok(try_service(&app)?.room(&room_id)?.log)
}

#[tauri::command]
pub async fn bot_collaboration_resolve(
    app: AppHandle,
    action_id: String,
    approve: bool,
) -> Result<PendingAction, String> {
    let action = try_service(&app)?.resolve(&action_id, approve, now_secs())?;
    if approve && action.kind == super::types::PendingKind::AddMember {
        if let Some(room_id) = action.room_id.as_deref() {
            let room = try_service(&app)?.room(room_id)?;
            super::flow::ensure_member_threads(&app, &room).await?;
        }
    }
    publish(&app);
    Ok(action)
}

#[tauri::command]
pub async fn bot_room_member_session(
    app: AppHandle,
    room_id: String,
    bot_id: String,
) -> Result<MemberSession, String> {
    member_session(&app, &room_id, &bot_id).await
}

#[tauri::command]
pub async fn bot_room_broadcast(
    app: AppHandle,
    room_id: String,
    body: String,
) -> Result<BroadcastReceipt, String> {
    let room = try_service(&app)?.room(&room_id)?;
    super::flow::ensure_member_threads(&app, &room).await?;
    broadcast(
        &app,
        Actor::User,
        &room_id,
        body,
        Vec::new(),
        None,
        None,
        None,
    )
}

#[tauri::command]
pub fn bot_room_rename(app: AppHandle, room_id: String, title: String) -> Result<Room, String> {
    rename_room(&app, &room_id, &title)
}
