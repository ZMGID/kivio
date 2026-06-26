use std::{path::PathBuf, time::Duration};

use arboard::Clipboard;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_shell::ShellExt;

use crate::api::{
    call_openai_text, effective_retry_attempts, resolve_provider_credentials, send_with_failover,
    send_with_retry, with_standard_request_timeout, ProviderConnectionInput,
};
use crate::prompts::{
    build_translation_prompt, DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE,
    DEFAULT_SELECTED_TEXT_TRANSLATION_TEMPLATE, DEFAULT_TRANSLATION_TEMPLATE,
};
use crate::rapidocr;
use crate::settings::{
    default_chat_system_prompt, default_lens_system_prompt, default_question_prompt,
    persist_settings, sanitize_settings, Settings,
};
#[cfg(target_os = "macos")]
use crate::shortcuts::{check_accessibility, check_screen_recording_permission};
use crate::shortcuts::{
    open_chat_settings_window as open_settings_window_impl, register_hotkeys,
    restore_runtime_settings, send_paste_shortcut, setup_tray,
};
use crate::state::AppState;
use crate::utils::{language_name, resolve_target_lang};
use crate::windows::get_main_window;

#[cfg(target_os = "linux")]
fn linux_autostart_dir_from_home(home: Option<&str>) -> Option<PathBuf> {
    home.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| PathBuf::from(value).join(".config").join("autostart"))
}

#[cfg(target_os = "linux")]
fn ensure_linux_autostart_dir() -> Result<(), String> {
    let dir = linux_autostart_dir_from_home(std::env::var("HOME").ok().as_deref())
        .ok_or_else(|| "could not resolve Linux autostart directory".to_string())?;
    // auto-launch 0.5 的 Linux 实现固定写入 `$HOME/.config/autostart`，
    // 不读取 XDG_CONFIG_HOME。新 HOME 下目录可能不存在，先创建再调用插件。
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())
}

pub(crate) fn apply_launch_at_startup(app: &AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if enabled {
        ensure_linux_autostart_dir()?;
    }

    let auto_launch = app.autolaunch();
    let current = auto_launch.is_enabled().map_err(|e| e.to_string())?;

    if enabled && !current {
        auto_launch.enable().map_err(|e| e.to_string())?;
    } else if !enabled && current {
        auto_launch.disable().map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// 获取当前应用设置
#[tauri::command]
pub(crate) fn get_settings(state: State<AppState>) -> Settings {
    state.settings_read().clone()
}

/// 读取 kivio-code 的独立配置（`<app_data>/kivio-code/config.json`）。它与共享 `Settings`
/// 分开存储，由 GUI 的 Kivio Code 设置页读写,CLI 启动时消费。缺失/损坏时回退默认值（不报错）。
#[tauri::command]
pub(crate) fn get_kivio_code_config() -> crate::kivio_code::config::KivioCodeConfig {
    crate::kivio_code::config::load()
}

/// 保存 kivio-code 的独立配置。失败（无法解析 app data 目录 / 写盘失败）时返回错误串。
#[tauri::command]
pub(crate) fn set_kivio_code_config(
    config: crate::kivio_code::config::KivioCodeConfig,
) -> Result<(), String> {
    crate::kivio_code::config::save(&config)
}

/// 全局指令文件路径:`<app_data>/agents/AGENTS.md`——kivio-code 每轮注入系统提示的全局那一层
/// （等价于 Claude Code 的 `~/.claude/CLAUDE.md`）。用与 CLI 读取相同的 `settings_loader::app_data_dir`
/// 解析,确保 GUI 写入处 == CLI 读取处。
fn kivio_code_global_instructions_path() -> Option<std::path::PathBuf> {
    crate::kivio_code::settings_loader::app_data_dir()
        .map(|dir| dir.join("agents").join("AGENTS.md"))
}

/// 读取全局指令文件内容（缺失/不可读 → 空串,不报错）。
#[tauri::command]
pub(crate) fn get_kivio_code_global_instructions() -> String {
    kivio_code_global_instructions_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// 保存全局指令文件（按需创建 `agents/` 目录）。无法解析目录 / 写盘失败时返回错误串。
#[tauri::command]
pub(crate) fn set_kivio_code_global_instructions(content: String) -> Result<(), String> {
    let path = kivio_code_global_instructions_path()
        .ok_or_else(|| "could not resolve app data directory".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

/// 「安装命令行工具」：把 `kivio` 命令注册进当前用户的 PATH，使任意终端可用 `kivio code`。
/// Windows 改 `HKCU\Environment\Path` 并广播 WM_SETTINGCHANGE；macOS 在 `~/.local/bin`
/// 建到主程序的软链。由 Kivio Code 设置页的按钮手动触发,不在安装时自动改 PATH。
#[tauri::command]
pub(crate) fn install_cli_command() -> Result<crate::cli_install::InstallCliResult, String> {
    crate::cli_install::install_cli()
}

/// 前端解析好主题（含 system）后调用：把 chat 窗口的原生背景设为对应主题色，
/// 避免伸缩时露出白色清屏底色导致暗色下闪白。仅对 label=="chat" 生效；
/// macOS/Linux 透明窗口在 windows 模块内为 no-op。
#[tauri::command]
pub(crate) fn set_chat_window_background(window: tauri::WebviewWindow, is_dark: bool) {
    if window.label() == "chat" {
        crate::windows::apply_chat_window_theme_background(&window, is_dark);
    }
}

/// 获取默认提示词模板
/// 返回翻译模板、截图翻译模板，以及 lens 视觉对话用的系统/提问提示词
#[tauri::command]
pub(crate) fn get_default_prompt_templates() -> serde_json::Value {
    serde_json::json!({
      "translationTemplate": DEFAULT_TRANSLATION_TEMPLATE,
      "screenshotTranslationTemplate": DEFAULT_SCREENSHOT_TRANSLATION_TEMPLATE,
      "selectedTextTranslationTemplate": DEFAULT_SELECTED_TEXT_TRANSLATION_TEMPLATE,
      "lensPrompts": {
        "zh": {
          "system": default_lens_system_prompt("zh", true),
          "question": default_question_prompt("zh", true)
        },
        "en": {
          "system": default_lens_system_prompt("en", true),
          "question": default_question_prompt("en", true)
        }
      },
      "chatPrompts": {
        "zh": default_chat_system_prompt("zh", false),
        "en": default_chat_system_prompt("en", false)
      }
    })
}

/// 保存设置
/// 先对传入的设置进行清理（sanitize），然后应用开机自启动、重新注册热键、持久化设置、更新托盘菜单
/// 如果热键注册失败，则回滚运行时设置到之前的状态
#[tauri::command]
pub(crate) fn save_settings(
    app: AppHandle,
    state: State<AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    apply_settings(&app, &state, settings)
}

/// sanitize → 应用运行时（自启/热键/托盘）→ 持久化，失败回滚。save_settings 与 import_settings 共用。
fn apply_settings(
    app: &AppHandle,
    state: &State<AppState>,
    settings: Settings,
) -> Result<Settings, String> {
    let previous_settings = state.settings_read().clone();
    let sanitized = sanitize_settings(settings);
    apply_launch_at_startup(app, sanitized.launch_at_startup)?;
    {
        let mut guard = state.settings_write();
        *guard = sanitized.clone();
    }
    state
        .sub_agents
        .set_concurrency(sanitized.chat_tools.sub_agent_concurrency);

    if let Err(err) = register_hotkeys(app) {
        restore_runtime_settings(app, state, &previous_settings);
        return Err(err);
    }

    if let Err(err) = persist_settings(app, &sanitized) {
        eprintln!("Failed to save settings: {err}");
        restore_runtime_settings(app, state, &previous_settings);
        return Err(err);
    }

    if let Err(err) = setup_tray(app) {
        eprintln!("Failed to update tray: {err}");
    }

    Ok(sanitized)
}

/// 设置备份文件格式版本。结构变化不兼容时递增。
const SETTINGS_BACKUP_VERSION: u32 = 1;

/// 导出全部设置（含供应商/模型配置与 API Key）到指定路径的 JSON 备份文件。
#[tauri::command]
pub(crate) fn export_settings(state: State<AppState>, path: String) -> Result<(), String> {
    let settings = state.settings_read().clone();
    let backup = serde_json::json!({
        "app": "kivio",
        "type": "settings-backup",
        "version": SETTINGS_BACKUP_VERSION,
        "settings": serde_json::to_value(&settings).map_err(|e| e.to_string())?,
    });
    let json = serde_json::to_string_pretty(&backup).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("写入失败: {e}"))?;
    Ok(())
}

/// 从备份文件导入设置，覆盖当前全部设置并立即生效（与保存同样走 sanitize/回滚）。
#[tauri::command]
pub(crate) fn import_settings(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<Settings, String> {
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("读取失败: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "文件不是有效的 JSON".to_string())?;
    if value.get("type").and_then(|v| v.as_str()) != Some("settings-backup") {
        return Err("这不是 Kivio 设置备份文件".to_string());
    }
    let settings_value = value
        .get("settings")
        .ok_or_else(|| "备份文件缺少 settings 字段".to_string())?;
    let settings: Settings = serde_json::from_value(settings_value.clone())
        .map_err(|e| format!("备份内容无法解析: {e}"))?;
    apply_settings(&app, &state, settings)
}

#[tauri::command]
pub(crate) fn open_settings_window(app: AppHandle) -> Result<(), String> {
    open_settings_window_impl(&app)
}

#[tauri::command]
pub(crate) fn close_translator_window(app: AppHandle) {
    if let Some(window) = get_main_window(&app) {
        let _ = window.close();
    }
}

/// 翻译文本命令
/// 根据设置中的翻译供应商和模型进行翻译；如果 API Key 为空则返回提示信息
#[tauri::command]
pub(crate) async fn translate_text(
    state: State<'_, AppState>,
    text: String,
) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok("".to_string());
    }

    let settings = state.settings_read().clone();
    let provider = settings
        .get_provider(&settings.translator_provider_id)
        .ok_or_else(|| "Translator provider not found".to_string())?;

    if provider.api_keys.is_empty() {
        return Ok("Missing API Key".to_string());
    }
    if settings.translator_model.trim().is_empty() {
        return Ok("Please select a model first".to_string());
    }

    let target_lang = resolve_target_lang(&settings.target_lang, trimmed);
    let lang_name = language_name(&target_lang).to_string();
    let prompt =
        build_translation_prompt(trimmed, &lang_name, settings.translator_prompt.as_deref());

    let retry_attempts = effective_retry_attempts(&settings);
    // 主翻译路径默认关思考：reasoning 模型对单句翻译几乎无质量收益但显著拖慢；非 reasoning 模型该字段被忽略
    call_openai_text(
        &state,
        provider,
        &settings.translator_model,
        prompt,
        retry_attempts,
        false,
        "translator",
        "translate_text",
    )
    .await
}

/// 提交翻译结果
/// 将翻译后的文本写入剪贴板，隐藏主窗口，如果启用了自动粘贴则发送粘贴快捷键到之前的应用
#[tauri::command]
pub(crate) async fn commit_translation(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }

    let auto_paste = state.settings_read().auto_paste;
    let mut clipboard = Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;

    // commit 用下面的 [NSApp hide:] 把前台让回原 App（成熟路径）。先清掉翻译窗的前台快照，
    // 让 main 关闭触发的 CloseRequested 焦点交还变成 no-op，避免与 hide 重复驱动激活。
    #[cfg(target_os = "macos")]
    crate::windows::forget_frontmost_app(&state.prev_frontmost_pid_main);

    // 关闭 main WebView，避免输入翻译页在后台长期占用内存。
    if let Some(window) = get_main_window(&app) {
        let _ = window.close();
    }

    #[cfg(target_os = "macos")]
    #[allow(deprecated, unexpected_cfgs)]
    unsafe {
        use cocoa::base::{id, nil};
        use objc::{class, msg_send, sel, sel_impl};
        let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![ns_app, hide: nil];
    }

    if auto_paste {
        // 增加延迟以确保焦点切换完成
        tokio::time::sleep(Duration::from_millis(600)).await;
        send_paste_shortcut();
    }

    Ok(())
}

/// 读取 Rust 端在 lens_request_internal 中暂存的 selection 文本（peek，不清除）。
/// 不能"读一次清一次"：前端 enterSelect 在 React StrictMode（dev 双调）/ 冷挂载 / 复用事件等
/// 情况下可能被调用多次，take 会让第一次取走、随后被 reqId 作废丢弃，第二次拿到空 → 选区丢失。
/// 每次打开 lens 时 lens_request_internal 都会覆盖 pending_selection（有选区写文本、无选区写
/// None），所以读不清除不会产生跨次 stale：当前这次打开读到的始终是这次的值。
#[tauri::command]
pub(crate) fn take_lens_selection(state: State<'_, AppState>) -> Result<String, String> {
    match state.pending_selection.lock() {
        Ok(guard) => Ok(guard.clone().unwrap_or_default()),
        Err(_) => Ok(String::new()),
    }
}

/// 使用系统默认浏览器打开外部链接（仅限 http/https）
#[tauri::command]
#[allow(deprecated)]
pub(crate) fn open_external(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("Invalid URL".to_string());
    }

    app.shell().open(url, None).map_err(|e| e.to_string())
}

/// 将 Chat 里的 HTML 预览写成临时文件，并用系统默认浏览器打开。
#[tauri::command]
#[allow(deprecated)]
pub(crate) fn open_html_preview(app: AppHandle, html: String) -> Result<(), String> {
    let path =
        std::env::temp_dir().join(format!("kivio-html-preview-{}.html", uuid::Uuid::new_v4()));
    std::fs::write(&path, html).map_err(|e| format!("Write HTML preview failed: {e}"))?;
    let path_str = path
        .to_str()
        .ok_or_else(|| "Invalid HTML preview path".to_string())?;
    app.shell().open(path_str, None).map_err(|e| e.to_string())
}

// ===== RapidOCR 离线 OCR 命令 =====
//
// status: 检查 app data 目录里 4 个文件齐不齐(dylib + det + rec + keys),前端据此决定
// 是否渲染下载按钮。
// install: 顺序下载 4 个文件到 app data 目录,~15-30s,前端转圈圈等返回。

/// 查询 RapidOCR 模型 + dylib 是否就绪。
#[tauri::command]
pub(crate) fn rapidocr_status(state: State<AppState>) -> rapidocr::RapidOcrStatus {
    state.rapidocr.status()
}

/// 下载 RapidOCR 包(onnxruntime dylib + 模型 + 字典)到 app data 目录。
/// 阻塞到全部完成(成功或失败),前端转圈圈等返回。
#[tauri::command]
pub(crate) async fn rapidocr_install(
    state: State<'_, AppState>,
) -> Result<rapidocr::RapidOcrInstallResult, String> {
    let client = state.rapidocr.clone();
    Ok(client.install().await)
}

#[tauri::command]
pub(crate) async fn fetch_models(
    state: State<'_, AppState>,
    provider_id: String,
    provider: Option<ProviderConnectionInput>,
) -> Result<Vec<String>, String> {
    let settings = state.settings_read().clone();
    let (base_url, api_keys) = resolve_provider_credentials(&settings, &provider_id, provider)?;
    let retry_attempts = effective_retry_attempts(&settings);

    if api_keys.is_empty() {
        return Err("Missing API Key".to_string());
    }

    let url = format!("{}/models", base_url.trim_end_matches('/'));

    let response = send_with_failover(
        &state,
        "Models API",
        retry_attempts,
        &provider_id,
        &api_keys,
        |key| with_standard_request_timeout(state.http.get(url.clone()).bearer_auth(key)).send(),
    )
    .await?;

    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse models response JSON: {e}"))?;

    let models = value
        .get("data")
        .and_then(|data| data.as_array())
        .ok_or_else(|| "Invalid response format: expected 'data' array".to_string())?
        .iter()
        .filter_map(|m| {
            if let Some(s) = m.as_str() {
                Some(s.to_string())
            } else {
                m.get("id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string())
            }
        })
        .collect::<Vec<String>>();

    Ok(models)
}

/// 测试供应商连接是否可用
/// 多 key：测试时只用第一个 key（避免一次连接测试遍历多 key 让用户困惑）
#[tauri::command]
pub(crate) async fn test_provider_connection(
    state: State<'_, AppState>,
    provider_id: String,
    provider: Option<ProviderConnectionInput>,
) -> Result<serde_json::Value, String> {
    let settings = state.settings_read().clone();
    let (base_url, api_keys) = resolve_provider_credentials(&settings, &provider_id, provider)?;

    let api_key = match api_keys.first() {
        Some(k) if !k.trim().is_empty() => k.clone(),
        _ => {
            return Ok(serde_json::json!({
              "success": false,
              "error": "Missing API Key"
            }));
        }
    };

    let retry_attempts = effective_retry_attempts(&settings);
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let result = send_with_retry("Provider API", retry_attempts, || {
        with_standard_request_timeout(state.http.get(url.clone()).bearer_auth(&api_key)).send()
    })
    .await;

    match result {
        Ok(_) => Ok(serde_json::json!({ "success": true })),
        Err(err) => Ok(serde_json::json!({ "success": false, "error": err })),
    }
}

/// 获取平台能力契约。前端据此展示/降级，不再自行猜测平台可用性。
#[tauri::command]
pub(crate) fn get_platform_capabilities() -> crate::platform_capabilities::PlatformCapabilities {
    crate::platform_capabilities::current_platform_capabilities()
}

/// 获取平台权限状态（仅限 macOS：辅助功能和屏幕录制权限）
#[tauri::command]
pub(crate) fn get_permission_status() -> serde_json::Value {
    #[cfg(target_os = "macos")]
    {
        let accessibility = check_accessibility(false);
        let screen_recording = check_screen_recording_permission();
        return serde_json::json!({
          "platform": "macos",
          "accessibility": accessibility,
          "screenRecording": screen_recording,
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        serde_json::json!({
          "platform": "other",
          "accessibility": true,
          "screenRecording": true,
        })
    }
}

/// 打开系统权限设置面板（仅限 macOS）
#[tauri::command]
pub(crate) fn open_permission_settings(kind: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let target = match kind.as_str() {
            "accessibility" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            "screen-recording" => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            _ => return Err("Unsupported permission kind".to_string()),
        };

        Command::new("open")
            .arg(target)
            .output()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Err("Permission settings are only available on macOS".to_string())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use super::linux_autostart_dir_from_home;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_autostart_dir_uses_home_config_for_auto_launch_crate() {
        assert_eq!(
            linux_autostart_dir_from_home(Some("/home/user")).unwrap(),
            std::path::PathBuf::from("/home/user/.config/autostart")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_autostart_dir_rejects_empty_home() {
        assert!(linux_autostart_dir_from_home(Some(" ")).is_none());
        assert!(linux_autostart_dir_from_home(None).is_none());
    }
}
