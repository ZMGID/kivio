//! The native application's global shortcut registrations and event lifetime.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use gpui_kit::*;
use kivio_model_runtime::settings::Settings;
use std::path::PathBuf;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Translate,
    TranslateSelection,
    TranslateScreenshot,
    Lens,
    OpenChat,
    CloseChat,
}
struct Shortcuts {
    manager: GlobalHotKeyManager,
    keys: Vec<(HotKey, Action)>,
    root: PathBuf,
    error: Option<String>,
    selection_task: Option<Task<()>>,
}
impl Global for Shortcuts {}
pub fn status(cx: &App) -> Option<String> {
    cx.try_global::<Shortcuts>()
        .and_then(|state| state.error.clone())
}
pub fn init(root: PathBuf, settings: &Settings, cx: &mut App) {
    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(error) => {
            tracing::error!(%error, "global hotkey manager");
            return;
        }
    };
    cx.set_global(Shortcuts {
        manager,
        keys: vec![],
        root,
        error: None,
        selection_task: None,
    });
    let (tx, rx) = async_channel::unbounded();
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let _ = tx.try_send(event.id);
        }
    }));
    if let Err(error) = apply(settings, cx) {
        cx.global_mut::<Shortcuts>().error = Some(error);
    }
    cx.spawn(async move |cx| {
        while let Ok(id) = rx.recv().await {
            let _ = cx.update(|cx| {
                let Some(state) = cx.try_global::<Shortcuts>() else {
                    return;
                };
                let root = state.root.clone();
                let action = state
                    .keys
                    .iter()
                    .find(|(key, _)| key.id() == id)
                    .map(|(_, action)| *action);
                match action {
                    Some(Action::Lens) => {
                        if let Err(error) = crate::native_capture::toggle_lens(root, cx) {
                            cx.global_mut::<Shortcuts>().error = Some(error);
                        }
                    }
                    Some(Action::TranslateScreenshot) => {
                        if let Err(error) = crate::native_capture::toggle(root, cx) {
                            cx.global_mut::<Shortcuts>().error = Some(error);
                        }
                    }
                    Some(Action::Translate) => {
                        if let Err(error) = crate::input_translation::open(root, cx) {
                            cx.global_mut::<Shortcuts>().error = Some(error);
                        }
                    }
                    Some(Action::TranslateSelection) => {
                        let task = cx.spawn(async move |cx| {
                            let text = cx.background_spawn(async move {
                                kivio_model_runtime::selection_capture::capture_active_selection()
                            }).await;
                            let _ = cx.update(|cx| match text {
                                Some(text) => {
                                    if let Err(error) =
                                        crate::input_translation::open_selection(root, text, cx)
                                    {
                                        cx.global_mut::<Shortcuts>().error = Some(error);
                                    }
                                }
                                None => {
                                    cx.global_mut::<Shortcuts>().error = Some(
                                        "未读取到选中文字，请先选中文字并检查辅助功能权限".into(),
                                    )
                                }
                            });
                        });
                        cx.global_mut::<Shortcuts>().selection_task = Some(task);
                    }
                    Some(Action::OpenChat) => crate::activate_main_window(cx),
                    Some(Action::CloseChat) => {
                        for (handle, shell) in crate::shell_windows(cx) {
                            let _ = cx.update_window(handle.into(), |_, window, cx| {
                                if crate::main_window_should_close(window, Some(shell), cx) {
                                    window.remove_window();
                                }
                            });
                        }
                    }
                    None => {}
                }
            });
        }
    })
    .detach();
}
pub fn apply(settings: &Settings, cx: &mut App) -> Result<(), String> {
    let mut keys = vec![];
    let bindings: [(&str, &str, Action); 6] = [
        ("Lens", if settings.lens.enabled { &settings.lens.hotkey } else { "" }, Action::Lens),
        ("截图翻译", if settings.screenshot_translation.enabled { &settings.screenshot_translation.hotkey } else { "" }, Action::TranslateScreenshot),
        ("输入翻译", &settings.hotkey, Action::Translate),
        (
            "选中文本翻译",
            if settings.screenshot_translation.enabled {
                &settings.screenshot_translation.text_hotkey
            } else {
                ""
            },
            Action::TranslateSelection,
        ),
        ("打开聊天", &settings.chat_hotkey, Action::OpenChat),
        ("关闭聊天", &settings.close_chat_hotkey, Action::CloseChat),
    ];
    for (label, text, action) in bindings {
        if text.trim().is_empty() {
            continue;
        }
        let key: HotKey = text
            .parse()
            .map_err(|error| format!("{label} 快捷键无效：{error}"))?;
        if keys
            .iter()
            .any(|(existing, _): &(HotKey, Action)| existing.id() == key.id())
        {
            return Err(format!("{label} 快捷键与其他功能重复"));
        }
        keys.push((key, action));
    }
    if !cx.has_global::<Shortcuts>() {
        return Err("系统快捷键服务不可用".into());
    }
    let state = cx.global_mut::<Shortcuts>();
    if state
        .keys
        .iter()
        .map(|(key, action)| (key.id(), *action))
        .eq(keys.iter().map(|(key, action)| (key.id(), *action)))
    {
        state.error = None;
        return Ok(());
    }
    // Release the old group, install the new group, and restore on any failure.
    let old = state.keys.clone();
    let mut released = Vec::new();
    for (key, _) in &old {
        if let Err(error) = state.manager.unregister(*key) {
            let mut rollback_errors = Vec::new();
            for key in released {
                if let Err(error) = state.manager.register(key) {
                    state.keys.retain(|(existing, _)| existing.id() != key.id());
                    rollback_errors.push(error.to_string());
                }
            }
            let message = format!(
                "无法注销旧快捷键：{error}{}",
                if rollback_errors.is_empty() {
                    String::new()
                } else {
                    format!("；恢复失败：{}", rollback_errors.join("；"))
                }
            );
            state.error = Some(message.clone());
            return Err(message);
        }
        released.push(*key);
    }
    let mut installed = vec![];
    for (key, _) in &keys {
        if let Err(error) = state.manager.register(*key) {
            for key in installed {
                let _ = state.manager.unregister(key);
            }
            let mut errors = vec![];
            state.keys.clear();
            for (key, action) in old {
                match state.manager.register(key) {
                    Ok(()) => state.keys.push((key, action)),
                    Err(error) => errors.push(error.to_string()),
                }
            }
            let message = format!(
                "快捷键被其他应用占用或注册失败：{error}{}",
                if errors.is_empty() {
                    String::new()
                } else {
                    format!("；恢复旧快捷键失败：{}", errors.join("；"))
                }
            );
            state.error = Some(message.clone());
            return Err(message);
        }
        installed.push(*key);
    }
    state.keys = keys;
    state.error = None;
    Ok(())
}
