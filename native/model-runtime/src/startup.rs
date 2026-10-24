//! OS registration and settings commit share one user-visible outcome.
use crate::native_preferences::{self, Change, FieldEdit};
use std::path::Path;
static UPDATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn registration() -> Result<auto_launch::AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let path = exe.to_str().ok_or("应用路径不是有效 UTF-8")?;
    // auto-launch 0.5 writes ProgramArguments directly into XML on macOS.
    // Reject characters it cannot encode rather than installing a broken agent.
    #[cfg(target_os = "macos")]
    if path.contains(['&', '<', '>']) {
        return Err(
            "当前应用路径包含登录项不支持的 XML 字符，请将应用放在 Applications 中。".into(),
        );
    }
    auto_launch::AutoLaunchBuilder::new()
        .set_app_name("com.zmair.kivio.gpui.preview")
        .set_app_path(path)
        .set_use_launch_agent(true)
        .build()
        .map_err(|e| e.to_string())
}
pub fn save(root: &Path, edit: FieldEdit) -> Result<crate::settings::Settings, String> {
    let _guard = UPDATE.lock().map_err(|e| e.to_string())?;
    let enabled = edit.after.as_bool().ok_or("无效的开机启动设置")?;
    if edit.path != "/launchAtStartup" {
        return Err("无效的开机启动字段".into());
    }
    let registration = registration()?;
    let previous = registration.is_enabled().map_err(|e| e.to_string())?;
    if enabled {
        registration.enable()
    } else {
        registration.disable()
    }
    .map_err(|e| e.to_string())?;
    match native_preferences::save(root, Change::Fields(vec![edit])) {
        Ok(settings) => Ok(settings),
        Err(error) => {
            let restored = if previous {
                registration.enable()
            } else {
                registration.disable()
            };
            Err(match restored {
                Ok(()) => error,
                Err(rollback) => format!("{error}；登录项回滚失败：{rollback}"),
            })
        }
    }
}
