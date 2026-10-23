//! 无 Tauri handle 的 per-app data 目录解析。
//!
//! 部分模块(plugins、skills 的 headless 发现)需要在没有
//! `AppHandle` 的情况下定位 Kivio 的 per-app data 目录。这里提供与 Tauri
//! `app_data_dir` 完全一致的纯函数实现。

use std::path::PathBuf;

#[path = "app_identity.rs"]
pub mod identity;

/// Effective Tauri identifier, including the CLI configuration overlay.
pub const APP_IDENTIFIER: &str = env!("KIVIO_APP_IDENTIFIER");

const IDENTITY: identity::AppIdentity = match identity::AppIdentity::from_identifier(APP_IDENTIFIER)
{
    Some(identity) => identity,
    None => panic!("unsupported Kivio application identity"),
};

pub const fn is_dev() -> bool {
    IDENTITY.is_development()
}

pub fn product_name() -> &'static str {
    IDENTITY.product_name()
}

pub fn user_home_name() -> &'static str {
    IDENTITY.user_home_name()
}

pub fn oauth_service() -> &'static str {
    IDENTITY.oauth_service()
}

/// 与 Tauri `app_data_dir` = `dirs::data_dir()/<identifier>` 完全一致:
///   Windows: `%APPDATA%\com.zmair.kivio`(Roaming,无 `\data` 子目录)
///   macOS:   `~/Library/Application Support/com.zmair.kivio`
///   Linux:   `$XDG_DATA_HOME/com.zmair.kivio`(或 `~/.local/share/...`)
///
/// 注意:必须用 `BaseDirs::data_dir()`,不能用 `ProjectDirs::data_dir()`——
/// 后者在 Windows 会多追加一层 `\data`,导致读到错误路径。
/// 无法确定 home/data 目录时返回 `None`。
pub fn app_data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|base| base.data_dir().join(IDENTITY.identifier()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_identity_keeps_storage_credentials_and_defaults_in_one_channel() {
        let context = crate::app_context();
        let (identifier, name, home, service, dev) = match context.config().identifier.as_str() {
            "com.zmair.kivio.dev" => (
                "com.zmair.kivio.dev",
                "Kivio Dev",
                ".kivio-dev",
                "Kivio.Dev.ModelOAuth",
                true,
            ),
            "com.zmair.kivio" => (
                "com.zmair.kivio",
                "Kivio Desktop",
                ".kivio",
                "Kivio.ModelOAuth",
                false,
            ),
            other => panic!("unexpected identity: {other}"),
        };
        assert_eq!(context.config().product_name.as_deref(), Some(name));
        assert_eq!(APP_IDENTIFIER, identifier);
        assert_eq!(product_name(), name);
        assert_eq!(user_home_name(), home);
        assert_eq!(oauth_service(), service);
        let base = directories::BaseDirs::new().unwrap();
        assert_eq!(app_data_dir().unwrap(), base.data_dir().join(identifier));
        assert_eq!(
            crate::skills::kivio_skills_dir().unwrap(),
            base.home_dir().join(home).join("skills")
        );
        let settings = crate::settings::sanitize_settings(crate::settings::Settings::default());
        assert_eq!(settings.auto_check_update, !dev);
        assert!(!settings.launch_at_startup);
        for hotkey in [
            &settings.hotkey,
            &settings.chat_hotkey,
            &settings.close_chat_hotkey,
            &settings.screenshot_translation.hotkey,
            &settings.screenshot_translation.text_hotkey,
            &settings.screenshot_translation.replace_hotkey,
            &settings.screenshot_annotate.hotkey,
            &settings.lens.hotkey,
        ] {
            assert_eq!(hotkey.is_empty(), dev);
        }
    }
}
