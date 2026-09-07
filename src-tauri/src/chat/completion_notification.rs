//! Desktop notification for a completed chat reply.
//!
//! Shows the conversation title and a preview of the latest reply in this turn.

use crate::chat::{ChatMessage, Conversation};
use crate::state::AppState;
use tauri::Manager;

pub(crate) fn notify_reply_completed(
    app: &tauri::AppHandle,
    state: &AppState,
    conversation: &Conversation,
) {
    let language = {
        let settings = state.settings_read();
        if !settings.chat_completion_notifications {
            return;
        }
        crate::settings::resolve_chat_language(&settings)
    };

    let (title, body) = completion_copy(&language, &conversation.title, &conversation.messages);
    let conversation_id = conversation.id.clone();
    let queued_at = std::time::Instant::now();
    let handle = app.clone();
    // Window getters perform a blocking event-loop round trip on worker threads.
    // Never keep the send reservation / IPC reply waiting for that round trip.
    // On the main thread Tauri executes these getters directly. No application
    // locks or borrowed conversation state cross this dispatch boundary.
    if let Err(error) = app.run_on_main_thread(move || {
        // Avoid a burst of stale notifications after sleep or an unresponsive UI.
        if queued_at.elapsed() > std::time::Duration::from_secs(15) {
            eprintln!("completion notification skipped: main-thread dispatch exceeded 15s");
            return;
        }
        if !is_viewing_conversation(&handle, &conversation_id) {
            crate::automation::notify::show(&handle, &title, &body);
        }
    }) {
        eprintln!("completion notification dispatch failed: {error}");
    }
}

fn is_viewing_conversation(app: &tauri::AppHandle, conversation_id: &str) -> bool {
    // Read live state when dispatched: the user may have switched apps or
    // conversations while the reply was being generated.
    app.webview_windows().values().any(|window| {
        let label = window.label();
        if label != "chat" && !crate::chat::popout::is_popout_label(label) {
            return false;
        }
        let focused = is_foreground_window(window);
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        if !is_viewing_window(focused, visible, minimized) {
            return false;
        }
        if let Some(id) = crate::chat::popout::conversation_id_from_label(label) {
            return id == conversation_id;
        }
        window
            .url()
            .ok()
            .is_some_and(|url| route_shows_conversation(url.fragment(), conversation_id))
    })
}

fn is_foreground_window(window: &tauri::WebviewWindow) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Tauri's Windows is_focused uses GetActiveWindow (thread-local).
        // GetForegroundWindow answers whether this window is active system-wide.
        window.hwnd().ok().is_some_and(|hwnd| unsafe {
            ::windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 == hwnd.0
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        window.is_focused().unwrap_or(false)
    }
}

fn is_viewing_window(focused: bool, visible: bool, minimized: bool) -> bool {
    focused && visible && !minimized
}

fn route_shows_conversation(fragment: Option<&str>, conversation_id: &str) -> bool {
    // Conversation IDs are generated as URL-safe `conv_…` identifiers. Ignore
    // hash query parameters, matching the frontend's chatRoutes.ts parser.
    fragment
        .and_then(|fragment| fragment.split('?').next())
        .and_then(|route| route.strip_prefix("chat/"))
        == Some(conversation_id)
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
    use super::{completion_copy, is_viewing_window, route_shows_conversation};

    fn message(role: &str, content: &str) -> crate::chat::ChatMessage {
        serde_json::from_value(serde_json::json!({
            "id": role, "role": role, "content": content, "timestamp": 0
        }))
        .unwrap()
    }

    #[test]
    fn only_a_visible_focused_non_minimized_window_suppresses_notifications() {
        assert!(is_viewing_window(true, true, false));
        assert!(!is_viewing_window(false, true, false)); // Another app is active.
        assert!(!is_viewing_window(true, false, false)); // Hidden keep-alive window.
        assert!(!is_viewing_window(true, true, true)); // Minimized window.
    }

    #[test]
    fn suppresses_only_the_conversation_being_viewed() {
        assert!(route_shows_conversation(
            Some("chat/conv_current"),
            "conv_current"
        ));
        assert!(route_shows_conversation(
            Some("chat/conv_current?mode=chat"),
            "conv_current"
        ));
        for route in [
            None,
            Some("chat"),
            Some("chat/conv_other"),
            Some("chat/conv_current_more"),
            Some("chat/settings?tab=general"),
            Some("chat/popout/conv_current"),
        ] {
            assert!(
                !route_shows_conversation(route, "conv_current"),
                "{route:?}"
            );
        }
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
