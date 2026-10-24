//! Native status-item lifetime belongs to the GPUI application.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn init(cx: &mut gpui_kit::App) -> Result<(), String> {
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };
    struct Tray {
        _icon: tray_icon::TrayIcon,
    }
    impl gpui_kit::Global for Tray {}
    let menu = Menu::new();
    let show = MenuItem::new("打开 Kivio", true, None);
    let quit = MenuItem::new("退出 Kivio", true, None);
    menu.append(&show).map_err(|e| e.to_string())?;
    menu.append(&quit).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let bytes = include_bytes!("../../../src-tauri/icons/tray-icon.png");
    #[cfg(target_os = "windows")]
    let bytes = include_bytes!("../../../src-tauri/icons/windows-tray.png");
    let image = image::load_from_memory(bytes)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let icon = Icon::from_rgba(image.into_raw(), width, height).map_err(|e| e.to_string())?;
    let tray = TrayIconBuilder::new()
        .with_tooltip("Kivio Native")
        .with_menu(Box::new(menu))
        .with_icon(icon)
        .with_icon_as_template(cfg!(target_os = "macos"))
        .build()
        .map_err(|e| e.to_string())?;
    cx.set_global(Tray { _icon: tray });
    let show_id = show.id().clone();
    let quit_id = quit.id().clone();
    let (tx, rx) = async_channel::unbounded();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = tx.try_send(event);
    }));
    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            let _ = cx.update(|cx| {
                if event.id == show_id {
                    crate::activate_main_window(cx);
                } else if event.id == quit_id {
                    crate::app_menus::request_quit(cx);
                }
            });
        }
    })
    .detach();
    Ok(())
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn init(_: &mut gpui_kit::App) -> Result<(), String> {
    Err("此平台尚无原生托盘宿主".into())
}
