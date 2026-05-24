#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

mod api;
mod apple_intelligence;
mod capture_geometry;
mod commands;
mod lens;
mod lens_commands;
#[cfg(target_os = "macos")]
mod macos_ocr;
mod prompts;
mod rapidocr;
#[cfg(target_os = "macos")]
mod sck;
mod screenshot;
mod settings;
mod shortcuts;
mod state;
mod updates;
mod utils;
mod windows;
#[cfg(target_os = "windows")]
mod windows_ocr;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64},
        Mutex, RwLock,
    },
    time::Duration,
};

use tauri::{Emitter, Manager, State};
#[cfg(target_os = "macos")]
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_single_instance::init as init_single_instance;

use api::build_http_client;
use commands::apply_launch_at_startup;
use screenshot::cleanup_orphan_temp_files;
use settings::load_settings;
use shortcuts::{
    display_hotkey_errors, open_settings_window_for_activation, register_hotkeys, setup_tray,
};
use state::AppState;
use updates::check_github_latest_release;
#[cfg(target_os = "macos")]
use windows::apply_macos_workspace_behavior;

/// 自启动参数，用于区分用户手动启动和系统自动启动
const AUTOSTART_ARG: &str = "--from-autostart";

/// 应用入口函数
/// 初始化 Tauri Builder，加载插件，配置窗口事件处理，设置全局状态、热键和托盘
fn main() {
    let autostart_plugin = {
        #[cfg(target_os = "macos")]
        {
            tauri_plugin_autostart::Builder::new()
                .arg(AUTOSTART_ARG)
                .macos_launcher(MacosLauncher::LaunchAgent)
                .build()
        }
        #[cfg(not(target_os = "macos"))]
        {
            tauri_plugin_autostart::Builder::new()
                .arg(AUTOSTART_ARG)
                .build()
        }
    };

    tauri::Builder::default()
        .plugin(init_single_instance(|app, _args, _cwd| {
            if let Err(err) = open_settings_window_for_activation(app) {
                eprintln!("Single-instance activation failed: {err}");
            }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(autostart_plugin)
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            tauri::WindowEvent::Focused(true) => {
                #[cfg(target_os = "macos")]
                if let Some(webview_window) = window.app_handle().get_webview_window(window.label())
                {
                    apply_macos_workspace_behavior(&webview_window);
                }
            }
            _ => {}
        })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                // 隐藏 Dock 图标，将应用设置为 accessory 激活策略
                let _ = app
                    .handle()
                    .set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            // 清理上次崩溃 / 强杀 / 旧版本遗留的截图 PNG（24h 之前的，避免误删并发实例的活文件）
            cleanup_orphan_temp_files();

            let settings = load_settings(&app.handle());
            if let Err(err) = apply_launch_at_startup(&app.handle(), settings.launch_at_startup) {
                eprintln!("Failed to apply launch-at-startup setting: {err}");
            }

            app.manage(AppState {
                settings: RwLock::new(settings),
                explain_images: Mutex::new(HashMap::new()),
                current_explain_image_id: Mutex::new(None),
                lens_busy: AtomicBool::new(false),
                explain_stream_generation: AtomicU64::new(0),
                pending_selection: Mutex::new(None),
                key_cooldowns: Mutex::new(HashMap::new()),
                active_key_idx: Mutex::new(HashMap::new()),
                http: build_http_client(),
                apple_intelligence: apple_intelligence::AppleIntelligenceClient::new(&app.handle()),
                #[cfg(target_os = "macos")]
                macos_ocr: macos_ocr::MacOcrClient::new(&app.handle()),
                rapidocr: rapidocr::RapidOcrClient::new(&app.handle(), build_http_client()),
            });

            if let Err(err) = register_hotkeys(&app.handle()) {
                eprintln!("Failed to register hotkeys: {}", display_hotkey_errors(&err));
            }
            if let Err(err) = setup_tray(&app.handle()) {
                eprintln!("Failed to setup tray: {err}");
            }

            // 预创建 lens webview（隐藏），让 WebView2 提前完成首次绘制 + 加载 React
            // 资源。第一次按热键时只走 show()，避免"窗口创建 → WebView2 首帧默认背景
            // 渲染白色 → CSS 把 html/body/#root 设成 transparent"这个时序里的全屏白闪。
            // 仅 Windows 启用：macOS 创建隐藏 webview 可能影响前台应用 focus，进而干扰
            // chat 模式的 Cmd+C/AXSelectedText 选区捕获（lens_request_internal 已有
            // 应对，但预创建会把这层风险提前到 setup 之外的代码路径），且 macOS 上
            // WKWebView 默认不会有 WebView2 这种白闪。
            #[cfg(target_os = "windows")]
            if let Err(err) = windows::ensure_lens_window(&app.handle()) {
                eprintln!("Failed to pre-create lens window: {err}");
            }

            // 启动后 5s 静默检查更新（settings.auto_check_update 控制）
            // 发现新版 → emit "update-available" 事件，前端 Settings 打开时会展示提示
            // 失败 / 限流 / 网络问题全部静默，不打扰用户
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let state: State<AppState> = app_handle.state();
                if !state.settings_read().auto_check_update {
                    return;
                }
                if let Ok(value) = check_github_latest_release(state).await {
                    if value
                        .get("available")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        let _ = app_handle.emit("update-available", value);
                    }
                }
            });

            #[cfg(target_os = "windows")]
            {
                // Windows 平台：如果不是通过自启动启动的，则默认打开设置窗口
                let launched_from_autostart = std::env::args().any(|arg| arg == AUTOSTART_ARG);
                if !launched_from_autostart {
                    let app_handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        if let Err(err) = open_settings_window_for_activation(&app_handle) {
                            eprintln!("Failed to open settings on launch: {err}");
                        }
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_default_prompt_templates,
            commands::save_settings,
            commands::translate_text,
            commands::commit_translation,
            commands::open_external,
            lens_commands::explain_read_image,
            commands::fetch_models,
            commands::test_provider_connection,
            commands::get_permission_status,
            commands::open_permission_settings,
            lens_commands::lens_request,
            lens_commands::lens_request_translate,
            lens_commands::lens_request_translate_text,
            lens_commands::lens_list_windows,
            lens_commands::lens_capture_window,
            lens_commands::lens_capture_region,
            lens_commands::lens_register_annotated_image,
            lens_commands::lens_ask,
            lens_commands::lens_translate,
            lens_commands::lens_translate_text,
            lens_commands::lens_cancel_stream,
            lens_commands::lens_close,
            lens_commands::lens_set_floating,
            lens_commands::lens_animate_floating,
            commands::take_lens_selection,
            lens_commands::lens_commit_image_to_history,
            lens_commands::lens_delete_history_image,
            updates::check_github_latest_release,
            updates::download_update_asset,
            updates::install_update_and_quit,
            commands::apple_intelligence_available,
            commands::rapidocr_status,
            commands::rapidocr_install
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
