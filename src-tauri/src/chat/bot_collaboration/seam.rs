use tauri::AppHandle;

use super::runtime::{member_assistant_id, try_service};

pub struct ReplySeam {
    /// Private-memory assistant id for this recipient thread. None for an ordinary chat.
    pub recipient_assistant_id: Option<String>,
    /// Present only after the recipient run has been admitted.
    pub collaboration_note: Option<String>,
}

pub fn reply_seam(app: &AppHandle, conversation_id: &str) -> ReplySeam {
    let recipient_assistant_id = member_assistant_id(app, conversation_id).ok().flatten();
    let collaboration_note = try_service(app)
        .ok()
        .and_then(|service| service.prompt_for(conversation_id));
    ReplySeam {
        recipient_assistant_id,
        collaboration_note,
    }
}
