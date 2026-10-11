//! Stable Bot identity for a private chat or a dedicated group-member thread.
//! An unresolved member thread is an error. It does not use global memory.

use tauri::AppHandle;

use super::types::{
    assistant_id_from_conversation, assistant_id_from_member_conversation, validate_assistant_id,
    MEMBER_CONVERSATION_MARK,
};

/// Private `conv_bot_{assistantId}`, a `conv_bot_member_{assistantId}@{roomId}`
/// thread, or a collaboration member row whose `conversation_id` is this thread
/// and whose `bot_id` is that assistant id.
///
/// `Ok(None)` is an ordinary chat. A member-shaped id that does not parse, a
/// member row whose bot id is not an assistant id, or two different Bot ids
/// for one thread all return `Err`.
pub fn resolve_stable_assistant_id(
    app: &AppHandle,
    conversation_id: &str,
) -> Result<Option<String>, String> {
    crate::chat::bot_collaboration::member_assistant_id(app, conversation_id)
}

/// `(conversation_id, bot_id)` rows from collaboration rooms.
/// Private `conv_bot_` rows are ignored. A dedicated thread with an invalid
/// bot id, or two different assistant ids, is an error.
pub fn assistant_id_from_member_rows<'a, I>(
    conversation_id: &str,
    rows: I,
) -> Result<Option<String>, String>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    if assistant_id_from_conversation(conversation_id).is_some() {
        return Ok(None);
    }
    let mut found: Option<String> = None;
    for (member_conversation_id, bot_id) in rows {
        if member_conversation_id != conversation_id {
            continue;
        }
        if assistant_id_from_conversation(member_conversation_id).is_some() {
            continue;
        }
        let assistant_id = validate_assistant_id(bot_id).map_err(|err| {
            format!("群成员会话「{conversation_id}」没有稳定 Bot，没有改用全局记忆：{err}")
        })?;
        if let Some(previous) = &found {
            if previous != assistant_id {
                return Err(format!(
                    "群成员会话「{conversation_id}」对应了多个 Bot，没有改用全局记忆"
                ));
            }
        } else {
            found = Some(assistant_id.to_string());
        }
    }
    Ok(found)
}

pub fn encoded_member_bot(conversation_id: &str) -> Result<Option<String>, String> {
    if !conversation_id.starts_with(MEMBER_CONVERSATION_MARK) {
        return Ok(None);
    }
    assistant_id_from_member_conversation(conversation_id)
        .map(|id| id.to_string())
        .ok_or_else(|| format!("群成员会话「{conversation_id}」没有稳定 Bot，没有改用全局记忆"))
        .map(Some)
}
