mod assets;
mod momo;
mod preview;
mod shell;
mod streaming_markdown;
mod theme;
mod transcript;

use gpui_kit::*;

gpui_kit::actions!(kivio, [Quit]);

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            theme::apply_system(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
            cx.set_menus(vec![Menu {
                name: "Kivio Native".into(),
                items: vec![MenuItem::action("退出", Quit)],
                disabled: false,
            }]);
            // Development capture preset; never enabled in release builds.
            let compact =
                cfg!(debug_assertions) && std::env::var_os("KIVIO_PREVIEW_COMPACT").is_some();
            // Physical window bounds are the platform boundary; content uses rem units.
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    if compact {
                        size(px(680.), px(520.))
                    } else {
                        size(px(1160.), px(780.))
                    },
                    cx,
                ))),
                window_min_size: Some(size(px(680.), px(520.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Kivio Native".into()),
                    traffic_light_position: Some(point(px(12.), px(15.))),
                    ..gpui_kit::component::TitleBar::title_bar_options()
                }),
                app_id: Some("com.zmair.kivio.gpui.preview".into()),
                ..gpui_kit::component::TitleBar::window_options()
            };
            if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });
                cx.new(|cx| shell::Shell::new(window, cx))
            }) {
                eprintln!("无法打开 Kivio 原生预览：{error:#}");
                cx.quit();
            }
            cx.activate(true);
        });
}
