//! Window presentation preferences for the isolated native client.
//!
//! The old client's `uiFontScale` scales its entire WebView, so it is not a
//! point-size setting. Keep this native view state separate from the existing
//! business settings until the formal settings/data migration is implemented.
use serde::{Deserialize, Serialize};
use std::{fs, io::ErrorKind, path::Path};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TextSize {
    Compact,
    #[default]
    Default,
    Comfortable,
}

impl TextSize {
    pub(crate) fn points(self) -> f32 {
        match self {
            Self::Compact => 13.,
            Self::Default => 14.,
            Self::Comfortable => 16.,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Preferences {
    pub(crate) mode: AppearanceMode,
    pub(crate) text_size: TextSize,
    // None follows the platform; an explicit false must survive restart too.
    pub(crate) reduce_motion: Option<bool>,
}

impl Preferences {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        match fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|error| format!("读取外观设置失败：{error}"))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("读取外观设置失败：{error}")),
        }
    }

    pub(crate) fn save(self, path: &Path) -> Result<Self, String> {
        let content = serde_json::to_string_pretty(&self)
            .map_err(|error| format!("保存外观设置失败：{error}"))?;
        crate::file_io::atomic_write(path, &content, "appearance")?;
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopens_preferences_and_preserves_file_on_failed_save_or_invalid_read() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("appearance.json");
        assert_eq!(
            Preferences::load(&path).unwrap().mode,
            AppearanceMode::System
        );
        fs::write(
            &path,
            r#"{"mode":"dark","textSize":"comfortable","reduceMotion":false}"#,
        )
        .unwrap();
        let mut preferences = Preferences::load(&path).unwrap();
        assert_eq!(preferences.text_size.points(), 16.);
        assert_eq!(preferences.reduce_motion, Some(false));
        preferences.mode = AppearanceMode::Light;
        preferences.save(&path).unwrap();
        assert_eq!(
            Preferences::load(&path).unwrap().mode,
            AppearanceMode::Light
        );

        let saved = fs::read(&path).unwrap();
        preferences.mode = AppearanceMode::System;
        crate::file_io::fail_atomic_replace_for_current_test(true);
        let result = preferences.save(&path);
        crate::file_io::fail_atomic_replace_for_current_test(false);
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);

        fs::write(&path, "invalid settings").unwrap();
        assert!(Preferences::load(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "invalid settings");
    }
}
