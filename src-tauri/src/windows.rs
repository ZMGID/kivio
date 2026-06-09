use tauri::{
    window::Color, AppHandle, LogicalPosition, LogicalSize, Manager, TitleBarStyle, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

/// 侧栏收起时主内容区最小宽度（与前端 `CHAT_MIN_SIZE_COLLAPSED` 一致）。
pub const CHAT_MIN_INNER_WIDTH_COLLAPSED: f64 = 400.0;
/// 侧栏展开时整窗最小宽度（260px 侧栏 + 主内容区最小宽度）。
pub const CHAT_MIN_INNER_WIDTH_EXPANDED: f64 = 660.0;
pub const CHAT_MIN_INNER_HEIGHT: f64 = 400.0;
const CHAT_DEFAULT_INNER_WIDTH: f64 = 1280.0;
const CHAT_DEFAULT_INNER_HEIGHT: f64 = 800.0;

#[cfg(target_os = "windows")]
const CHAT_WINDOW_EDGE_GUTTER: f64 = 12.0;

fn chat_window_size_for_visible_content(width: f64, height: f64) -> (f64, f64) {
    #[cfg(target_os = "windows")]
    {
        let gutter = CHAT_WINDOW_EDGE_GUTTER * 2.0;
        return (width + gutter, height + gutter);
    }

    #[cfg(not(target_os = "windows"))]
    {
        (width, height)
    }
}

pub fn apply_chat_window_min_size(window: &WebviewWindow, sidebar_expanded: bool) {
    let width = if sidebar_expanded {
        CHAT_MIN_INNER_WIDTH_EXPANDED
    } else {
        CHAT_MIN_INNER_WIDTH_COLLAPSED
    };
    let (width, height) = chat_window_size_for_visible_content(width, CHAT_MIN_INNER_HEIGHT);
    let _ = window.set_min_size(Some(LogicalSize::new(width, height)));
}

#[cfg(target_os = "macos")]
const CHAT_TRAFFIC_LIGHT_X: f64 = 14.0;

/// 与侧栏顶栏图标（52px 行高居中）垂直对齐。
#[cfg(target_os = "macos")]
const CHAT_TRAFFIC_LIGHT_Y: f64 = 29.0;

#[cfg(target_os = "macos")]
pub(crate) fn apply_macos_traffic_light_position(window: &WebviewWindow) {
    use cocoa::base::id;

    let window_for_main = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(ptr) = window_for_main.ns_window() else {
            return;
        };
        if ptr.is_null() {
            return;
        }
        unsafe {
            hide_macos_window_title(ptr as id);
            inset_traffic_lights(ptr as id, CHAT_TRAFFIC_LIGHT_X, CHAT_TRAFFIC_LIGHT_Y);
        }
    });
}

/// NSWindowTitleHidden — 隐藏 Overlay 标题栏中的窗口标题文字。
#[cfg(target_os = "macos")]
unsafe fn hide_macos_window_title(window: cocoa::base::id) {
    use objc::{msg_send, sel, sel_impl};

    const NS_WINDOW_TITLE_HIDDEN: u64 = 1;
    let _: () = msg_send![window, setTitleVisibility: NS_WINDOW_TITLE_HIDDEN];
}

/// 与 wry `inset_traffic_lights` 相同逻辑，用于运行时微调原生交通灯位置。
#[cfg(target_os = "macos")]
unsafe fn inset_traffic_lights(window: cocoa::base::id, x: f64, y: f64) {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSRect;
    use objc::{msg_send, sel, sel_impl};

    let close: id = msg_send![window, standardWindowButton: 0u64];
    if close == nil {
        return;
    }
    let miniaturize: id = msg_send![window, standardWindowButton: 1u64];
    if miniaturize == nil {
        return;
    }
    let zoom: id = msg_send![window, standardWindowButton: 2u64];

    let title_bar_container_view: id = msg_send![close, superview];
    let title_bar_container_view: id = msg_send![title_bar_container_view, superview];

    let close_rect: NSRect = msg_send![close, frame];
    let title_bar_frame_height = close_rect.size.height + y;
    let mut title_bar_rect: NSRect = msg_send![title_bar_container_view, frame];
    title_bar_rect.size.height = title_bar_frame_height;
    let window_frame: NSRect = msg_send![window, frame];
    title_bar_rect.origin.y = window_frame.size.height - title_bar_frame_height;
    let _: () = msg_send![title_bar_container_view, setFrame: title_bar_rect];

    let miniaturize_rect: NSRect = msg_send![miniaturize, frame];
    let space_between = miniaturize_rect.origin.x - close_rect.origin.x;

    let mut buttons = vec![close, miniaturize];
    if zoom != nil {
        buttons.push(zoom);
    }

    for (i, button) in buttons.into_iter().enumerate() {
        let mut rect: NSRect = msg_send![button, frame];
        rect.origin.x = x + (i as f64 * space_between);
        let _: () = msg_send![button, setFrameOrigin: rect.origin];
    }
}

/// Chat 作为普通桌面窗口：不置顶、不跨全 Space（与 Lens overlay 区分）。
pub fn normalize_chat_window_behavior(window: &WebviewWindow) {
    let _ = window.set_always_on_top(false);
    let _ = window.set_skip_taskbar(false);
    #[cfg(target_os = "macos")]
    let _ = window.set_visible_on_all_workspaces(false);
}

/// macOS Chat：系统 Overlay 标题栏 + 原生交通灯；其他平台保持无边框自绘控件。
pub fn apply_chat_window_chrome(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        let _ = window.set_decorations(true);
        let _ = window.set_title_bar_style(TitleBarStyle::Overlay);
        let _ = window.set_shadow(true);
        apply_macos_traffic_light_position(window);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.set_decorations(false);
        #[cfg(target_os = "windows")]
        let _ = window.set_shadow(true);
        #[cfg(not(target_os = "windows"))]
        let _ = window.set_shadow(false);
        let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
    }
}

/// 翻译器 / 设置等悬浮小窗：无边框透明壳。
pub fn apply_frameless_window_chrome(window: &WebviewWindow) {
    let _ = window.set_decorations(false);
    let _ = window.set_shadow(false);
    #[cfg(target_os = "macos")]
    {
        let _ = window.set_title_bar_style(TitleBarStyle::Visible);
    }
}

/**
 * 获取主窗口
 */
pub fn get_main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

pub fn get_settings_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("settings")
}

pub fn get_chat_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("chat")
}

/**
 * 确保主窗口存在（不存在则创建）
 * 从 tauri.conf.json 中读取主窗口配置进行创建
 */
pub fn ensure_main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = get_main_window(app) {
        return Ok(window);
    }

    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .ok_or_else(|| "Main window config not found".to_string())?;

    WebviewWindowBuilder::from_config(app, config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

/**
 * 确保独立 Chat 窗口存在。
 */
pub fn ensure_chat_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    ensure_chat_window_with_hash(app, "chat")
}

/**
 * 确保独立 Chat 窗口存在，并在首次创建时进入指定 hash 路由。
 */
pub fn ensure_chat_window_with_hash(app: &AppHandle, hash: &str) -> Result<WebviewWindow, String> {
    if let Some(window) = get_chat_window(app) {
        return Ok(window);
    }

    let route = hash.trim_start_matches('#');
    let route = if route.is_empty() { "chat" } else { route };
    let url = format!("index.html#{route}");
    let (min_width, min_height) =
        chat_window_size_for_visible_content(CHAT_MIN_INNER_WIDTH_COLLAPSED, CHAT_MIN_INNER_HEIGHT);
    let (default_width, default_height) =
        chat_window_size_for_visible_content(CHAT_DEFAULT_INNER_WIDTH, CHAT_DEFAULT_INNER_HEIGHT);
    let mut builder = WebviewWindowBuilder::new(app, "chat", WebviewUrl::App(url.into()))
        .title("Kivio")
        .inner_size(default_width, default_height)
        .min_inner_size(min_width, min_height)
        .resizable(true)
        .visible_on_all_workspaces(false)
        .skip_taskbar(false)
        .visible(false);

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .decorations(true)
            .title_bar_style(TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(LogicalPosition::new(
                CHAT_TRAFFIC_LIGHT_X,
                CHAT_TRAFFIC_LIGHT_Y,
            ))
            .transparent(false)
            .shadow(true);
    }

    #[cfg(not(target_os = "macos"))]
    {
        // 透明 WebView 背景允许前端 shell 裁出圆角和 Windows gutter。
        builder = builder
            .decorations(false)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0));
        #[cfg(target_os = "windows")]
        {
            builder = builder.shadow(true);
        }
        #[cfg(not(target_os = "windows"))]
        {
            builder = builder.shadow(false);
        }
    }

    builder.build().map_err(|e| e.to_string())
}

/**
 * 确保 Lens 窗口存在（不存在则创建）
 * 单 webview 三态：select 全屏 / ready 悬浮 600x72 / answering 悬浮 600x420。
 * 创建时尺寸为悬浮态默认值；后端按需要 set_size 切换。
 */
pub fn ensure_lens_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    if let Some(window) = app.get_webview_window("lens") {
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(app, "lens", WebviewUrl::App("index.html#lens".into()))
        .title("Lens")
        .inner_size(600.0, 72.0)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .resizable(true)
        .decorations(false)
        .shadow(false)
        .transparent(true)
        // 把 WebView2 / WKWebView 的默认背景设成全透明。Windows 上 WebView2 控件
        // 在 HTML/CSS 把 html、body、#root 设为 transparent 之前会用系统主题色（白）
        // 渲染首帧，导致全屏白闪 —— 设了 (0,0,0,0) 后默认背景本身就是透明的。
        // 文档：Windows 8+ 上 webview 层的 alpha=0 被尊重；macOS 上此调用是 no-op。
        .background_color(Color(0, 0, 0, 0))
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    apply_macos_workspace_behavior(&window);

    Ok(window)
}

/**
 * macOS 平台特有：设置窗口在所有工作区可见
 * 确保截图窗口可以跨越桌面空间显示
 */
#[cfg(target_os = "macos")]
pub fn apply_macos_workspace_behavior(window: &WebviewWindow) {
    let window_for_task = window.clone();
    let _ = window.run_on_main_thread(move || {
        let _ = window_for_task.set_visible_on_all_workspaces(true);
    });
}

#[allow(dead_code)]
#[cfg(not(target_os = "macos"))]
pub fn apply_macos_workspace_behavior(_window: &WebviewWindow) {}
