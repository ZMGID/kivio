#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(target_os = "macos", allow(unexpected_cfgs))]

mod api;
mod apple_intelligence;
mod capture_geometry;
mod chat;
mod commands;
mod lens;
mod lens_commands;
#[cfg(target_os = "macos")]
mod macos_ocr;
mod mcp;
mod native_tools;
mod prompts;
mod rapidocr;
#[cfg(target_os = "macos")]
mod sck;
mod screenshot;
mod settings;
mod shortcuts;
mod skills;
mod state;
mod updates;
mod utils;
mod web_search;
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
    display_hotkey_errors, open_chat_window, open_settings_window_for_activation, register_hotkeys,
    setup_tray,
};
use state::AppState;
use updates::check_github_latest_release;
#[cfg(target_os = "macos")]
use windows::apply_macos_workspace_behavior;

/// 自启动参数，用于区分用户手动启动和系统自动启动
const AUTOSTART_ARG: &str = "--from-autostart";

#[cfg(target_os = "macos")]
const USER_WINDOW_LABELS: &[&str] = &["chat", "settings", "main"];

#[cfg(target_os = "macos")]
fn first_visible_user_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    USER_WINDOW_LABELS.iter().find_map(|label| {
        app.get_webview_window(label)
            .filter(|window| window.is_visible().ok().unwrap_or(false))
    })
}

#[cfg(target_os = "macos")]
fn has_user_window_except(app: &tauri::AppHandle, excluded_label: &str) -> bool {
    USER_WINDOW_LABELS
        .iter()
        .any(|label| *label != excluded_label && app.get_webview_window(label).is_some())
}

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
                if window.label() == "lens" {
                    api.prevent_close();
                    let _ = window.hide();
                    return;
                }
                #[cfg(target_os = "macos")]
                if USER_WINDOW_LABELS
                    .iter()
                    .any(|label| *label == window.label())
                    && !has_user_window_except(&window.app_handle(), window.label())
                {
                    let _ = window
                        .app_handle()
                        .set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            }
            tauri::WindowEvent::Focused(true) =>
            {
                #[cfg(target_os = "macos")]
                if window.label() == "lens" {
                    if let Some(webview_window) = window.app_handle().get_webview_window("lens") {
                        apply_macos_workspace_behavior(&webview_window);
                    }
                }
            }
            _ => {}
        })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                // 启动瞬间先 Accessory，open_chat_window 会切到 Regular
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
                chat_stream_generations: Mutex::new(HashMap::new()),
                pending_chat_tool_approvals: Mutex::new(HashMap::new()),
                pending_python_runs: Mutex::new(HashMap::new()),
                chat_create_conversation_lock: Mutex::new(()),
                chat_tool_list_cache: Mutex::new(HashMap::new()),
                pending_chat_external_sends: Mutex::new(Vec::new()),
                pending_selection: Mutex::new(None),
                lens_freeze_frame_image_id: Mutex::new(None),
                key_cooldowns: Mutex::new(HashMap::new()),
                active_key_idx: Mutex::new(HashMap::new()),
                http: build_http_client(),
                apple_intelligence: apple_intelligence::AppleIntelligenceClient::new(&app.handle()),
                #[cfg(target_os = "macos")]
                macos_ocr: macos_ocr::MacOcrClient::new(&app.handle()),
                rapidocr: rapidocr::RapidOcrClient::new(&app.handle(), build_http_client()),
            });

            if let Err(err) = register_hotkeys(&app.handle()) {
                eprintln!(
                    "Failed to register hotkeys: {}",
                    display_hotkey_errors(&err)
                );
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

            // 如果不是通过自启动启动的，则默认打开 AI 客户端。
            let launched_from_autostart = std::env::args().any(|arg| arg == AUTOSTART_ARG);
            if !launched_from_autostart {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if let Err(err) = open_chat_window(&app_handle) {
                        eprintln!("Failed to open chat on launch: {err}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_default_prompt_templates,
            commands::save_settings,
            commands::open_settings_window,
            commands::close_translator_window,
            commands::translate_text,
            commands::commit_translation,
            commands::open_external,
            commands::open_html_preview,
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
            lens_commands::lens_send_to_chat,
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
            commands::rapidocr_install,
            // Chat 模块命令
            chat::commands::chat_get_conversations,
            chat::commands::chat_get_conversation,
            chat::commands::chat_create_conversation,
            chat::commands::chat_get_assistants,
            chat::commands::chat_create_assistant,
            chat::commands::chat_update_assistant,
            chat::commands::chat_duplicate_assistant,
            chat::commands::chat_delete_assistant,
            chat::commands::chat_get_projects,
            chat::commands::chat_create_project,
            chat::commands::chat_update_project,
            chat::commands::chat_delete_project,
            chat::commands::chat_get_context_stats,
            chat::commands::chat_compress_context,
            chat::commands::chat_take_external_sends,
            chat::commands::chat_send_message,
            chat::commands::chat_cancel_stream,
            chat::commands::chat_confirm_tool_call,
            chat::commands::chat_python_complete,
            chat::commands::chat_read_attachment,
            chat::commands::chat_open_attachment,
            chat::commands::chat_save_pasted_image,
            chat::commands::chat_save_pasted_attachment,
            chat::commands::chat_read_clipboard_files,
            chat::commands::chat_delete_conversation,
            chat::commands::chat_update_conversation,
            chat::commands::chat_update_message,
            chat::commands::chat_delete_message,
            chat::commands::chat_regenerate_message,
            chat::memory::chat_memory_get,
            chat::memory::chat_memory_save,
            chat::memory::chat_memory_open_folder,
            mcp::registry::chat_mcp_list_tools,
            mcp::registry::chat_mcp_test_server,
            mcp::registry::chat_mcp_import_json,
            skills::chat_skills_list,
            skills::chat_skills_read,
            skills::chat_skills_import,
            skills::chat_skills_open_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } => {
                if !has_visible_windows {
                    if let Err(err) = open_chat_window(app_handle) {
                        eprintln!("Failed to open chat on dock reopen: {err}");
                    }
                } else if let Some(window) = first_visible_user_window(app_handle) {
                    let _ = window.show();
                    let _ = window.set_focus();
                } else if let Err(err) = open_chat_window(app_handle) {
                    eprintln!("Failed to open chat on dock reopen: {err}");
                }
            }
            _ => {}
        });
}
