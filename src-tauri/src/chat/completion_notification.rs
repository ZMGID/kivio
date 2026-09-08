//! Desktop notification for a completed chat reply.
//!
//! Shows the conversation title and a preview of the latest reply in this turn.

use crate::chat::{ChatMessage, Conversation};
use crate::state::AppState;

pub(crate) fn notify_reply_completed(
    app: &tauri::AppHandle,
    state: &AppState,
    conversation: &Conversation,
) {
    let language = {
        // Notifications are optional; do not wait behind a settings writer.
        let settings = match state.settings.try_read() {
            Ok(settings) => settings,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        if !settings.chat_completion_notifications {
            return;
        }
        crate::settings::resolve_chat_language(&settings)
    };

    // Never query AppKit / WebKit to decide whether to notify. Even dispatching
    // getters to the main thread couples this optional feature to the UI loop.
    if super::notification_viewing::is_viewing(&conversation.id) {
        return;
    }
    let (title, body) = completion_copy(&language, &conversation.title, &conversation.messages);
    crate::automation::notify::show(app, &title, &body);
}

fn completion_copy(
    language: &str,
    conversation_title: &str,
    messages: &[ChatMessage],
) -> (String, String) {
    let is_chinese = language.trim().to_ascii_lowercase().starts_with("zh");
    let title = preview(conversation_title, 77);
    let title = if title.is_empty() {
        if is_chinese {
            "Kivio · 回复已完成"
        } else {
            "Kivio · Reply ready"
        }
        .to_string()
    } else {
        title
    };
    // Stay within the latest turn so an empty/image-only reply cannot show an
    // earlier answer. Use content, never the reasoning or tool transcript.
    let reply = messages
        .iter()
        .rev()
        .take_while(|message| message.role != "user")
        .find(|message| message.role == "assistant")
        .map(|message| message.content.as_str())
        .unwrap_or_default();
    let body = preview(reply, 200);
    let body = if body.is_empty() {
        if is_chinese {
            "你的回复已经生成完成。"
        } else {
            "Your reply is ready."
        }
        .to_string()
    } else {
        body
    };
    (title, body)
}

fn preview(text: &str, max_chars: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    crate::chat::agent::execute::truncate_chars(&text, max_chars)
}

#[cfg(test)]
mod tests {
    use super::completion_copy;

    fn message(role: &str, content: &str) -> crate::chat::ChatMessage {
        serde_json::from_value(serde_json::json!({
            "id": role, "role": role, "content": content, "timestamp": 0
        }))
        .unwrap()
    }

    #[test]
    fn shows_conversation_title_and_actual_reply() {
        let messages = [message("user", "123"), message("assistant", "收到，123。")];
        let (title, body) = completion_copy("zh-CN", "测试对话", &messages);
        assert_eq!(title, "测试对话");
        assert_eq!(body, "收到，123。");
    }

    #[test]
    fn empty_title_and_reply_use_localized_fallbacks() {
        assert_eq!(
            completion_copy("en", "  ", &[]),
            ("Kivio · Reply ready".into(), "Your reply is ready.".into())
        );
        assert_eq!(
            completion_copy("zh", "", &[]),
            ("Kivio · 回复已完成".into(), "你的回复已经生成完成。".into())
        );
    }

    #[test]
    fn later_turn_notifications_show_the_latest_answer() {
        let messages = [
            message("user", "第一轮问题"),
            message("assistant", "第一轮答案"),
            message("user", "123"),
            message("assistant", "收到，123。"),
        ];
        assert_eq!(
            completion_copy("zh", "第一轮标题", &messages).1,
            "收到，123。"
        );
    }

    #[test]
    fn empty_current_reply_never_reuses_an_earlier_answer_or_reasoning() {
        let mut reply = message("assistant", "  ");
        reply.reasoning = Some("Internal reasoning".into());
        let mut messages = vec![
            message("assistant", "Old answer"),
            message("user", "New question"),
        ];
        assert_eq!(
            completion_copy("en", "Title", &messages).1,
            "Your reply is ready."
        );
        messages.push(reply);
        assert_eq!(
            completion_copy("en", "Title", &messages).1,
            "Your reply is ready."
        );
    }

    #[test]
    fn long_multiline_replies_have_a_short_single_line_preview() {
        let reply = format!("  已完成\n\t{}", "测".repeat(300));
        let (title, body) =
            completion_copy("zh", &"题".repeat(100), &[message("assistant", &reply)]);
        assert_eq!(title, format!("{}...", "题".repeat(77)));
        assert_eq!(body, format!("已完成 {}...", "测".repeat(196)));
    }
}
