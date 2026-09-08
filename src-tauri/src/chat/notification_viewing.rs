//! Event-fed notification suppression. No native window getters, UI dispatch,
//! filesystem access or blocking lock acquisition may be added to this path.

use std::{collections::BTreeMap, sync::Mutex};

#[derive(Default)]
struct ViewingWindows(BTreeMap<String, String>);

impl ViewingWindows {
    fn update(&mut self, label: &str, route: &str, viewing: bool) {
        let id = if !viewing {
            None
        } else if label == "chat" {
            route
                .trim_start_matches('#')
                .split('?')
                .next()
                .and_then(|path| path.strip_prefix("chat/"))
        } else {
            // A popout may only claim the conversation bound to its own label.
            super::popout::conversation_id_from_label(label)
        };
        let id = id.filter(|id| {
            id.starts_with("conv_")
                && id.len() > 5
                && id.len() <= 256
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        });
        if let Some(id) = id {
            self.0.insert(label.into(), id.into());
        } else {
            self.0.remove(label);
        }
    }

    fn contains(&self, conversation_id: &str) -> bool {
        self.0.values().any(|id| id == conversation_id)
    }
}

static VIEWING: Mutex<ViewingWindows> = Mutex::new(ViewingWindows(BTreeMap::new()));

fn cached_is_viewing(cache: &Mutex<ViewingWindows>, conversation_id: &str) -> bool {
    // Contention can at worst produce an extra notification, never a hung reply.
    cache
        .try_lock()
        .is_ok_and(|windows| windows.contains(conversation_id))
}

pub(super) fn is_viewing(conversation_id: &str) -> bool {
    cached_is_viewing(&VIEWING, conversation_id)
}

#[tauri::command]
pub(crate) fn chat_report_notification_view(
    window: tauri::WebviewWindow,
    route: String,
    viewing: bool,
) {
    if let Ok(mut windows) = VIEWING.try_lock() {
        windows.update(window.label(), &route, viewing);
    }
}

pub(crate) fn clear_window(label: &str) {
    if let Ok(mut windows) = VIEWING.try_lock() {
        windows.0.remove(label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_route_focus_visibility_and_window_destruction() {
        let mut windows = ViewingWindows::default();
        windows.update("chat", "#chat/conv_first?mode=chat", true);
        assert!(windows.contains("conv_first"));
        windows.update("chat", "#chat/conv_second", true);
        assert!(!windows.contains("conv_first"));
        assert!(windows.contains("conv_second"));
        windows.update("chat", "#chat/conv_second", false);
        assert!(!windows.contains("conv_second"));
        windows.update("chat", "#chat/conv_second", true);
        windows.0.remove("chat");
        assert!(!windows.contains("conv_second"));
    }

    #[test]
    fn popout_identity_is_independent_of_main_window_and_reported_route() {
        let mut windows = ViewingWindows::default();
        windows.update("chat-popout-conv_pop", "#chat/conv_spoof", true);
        windows.update("chat", "#chat/settings", true);
        assert!(windows.contains("conv_pop"));
        assert!(!windows.contains("conv_spoof"));
        windows.update("chat-popout-conv_pop", "", false);
        assert!(!windows.contains("conv_pop"));
    }

    #[test]
    fn non_conversation_routes_and_non_chat_windows_do_not_suppress() {
        let mut windows = ViewingWindows::default();
        for route in [
            "#chat",
            "#chat/settings",
            "#chat/popout/conv_a",
            "#chat/conv_a/extra",
            "#chat/conv_",
        ] {
            windows.update("chat", route, true);
            assert!(windows.0.is_empty());
        }
        windows.update("lens", "#chat/conv_a", true);
        assert!(windows.0.is_empty());
    }

    #[test]
    fn busy_cache_does_not_wait_for_a_lock() {
        let cache = Mutex::new(ViewingWindows::default());
        let _held = cache.lock().unwrap();
        // Same-thread contention: a blocking lock here would deadlock this test.
        assert!(!cached_is_viewing(&cache, "conv_a"));
    }
}
