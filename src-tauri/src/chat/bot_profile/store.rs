//! One JSON file per Bot. A corrupt file is an error and is never replaced
//! with an empty profile. The process lock covers read-modify-write.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use tauri::AppHandle;

use super::types::{validate_assistant_id, BotProfile, PROFILE_VERSION};

pub struct ProfileStore {
    dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ClearedPrimary {
    pub previous: BotProfile,
    /// Revision written when this profile's primary flag was cleared.
    pub written_revision: u64,
}

#[derive(Debug, Clone)]
pub struct InsertRestore {
    pub created_id: String,
    pub created_revision: u64,
    pub previous_primaries: Vec<ClearedPrimary>,
}

/// `restore` is set only when this insert changed a profile and could not undo it.
/// A rejection before any write, or a rollback that already succeeded, leaves it empty.
#[derive(Debug)]
pub struct InsertNewError {
    pub message: String,
    pub restore: Option<InsertRestore>,
}

fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl ProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn for_app(app: &AppHandle) -> Result<Self, String> {
        Ok(Self::new(
            crate::chat::storage::conversations_dir(app)?.join("bot-profiles"),
        ))
    }

    pub fn load(&self, assistant_id: &str) -> Result<Option<BotProfile>, String> {
        let _guard = lock();
        self.load_unlocked(assistant_id)
    }

    pub fn list(&self) -> Result<Vec<BotProfile>, String> {
        let _guard = lock();
        self.list_unlocked()
    }

    pub fn insert_new(&self, profile: &BotProfile) -> Result<InsertRestore, InsertNewError> {
        let _guard = lock();
        validate_profile(profile).map_err(|message| InsertNewError {
            message,
            restore: None,
        })?;
        if self.path(&profile.assistant_id).exists() {
            return Err(InsertNewError {
                message: "这份 Bot 配置已经存在".into(),
                restore: None,
            });
        }
        let previous_primaries =
            self.clear_other_primaries(profile)
                .map_err(|message| InsertNewError {
                    message,
                    restore: None,
                })?;
        let restore = InsertRestore {
            created_id: profile.assistant_id.clone(),
            created_revision: profile.revision,
            previous_primaries,
        };
        if let Err(err) = self.write_unlocked(profile) {
            return Err(match self.rollback_insert_unlocked(&restore) {
                Ok(()) => InsertNewError {
                    message: err,
                    restore: None,
                },
                Err(rollback_err) => InsertNewError {
                    message: format!("{err}；配置回滚：{rollback_err}"),
                    restore: Some(restore),
                },
            });
        }
        Ok(restore)
    }

    pub fn rollback_insert(&self, restore: &InsertRestore) -> Result<(), String> {
        let _guard = lock();
        self.rollback_insert_unlocked(restore)
    }

    /// Writes `profile` only when this Bot's on-disk revision is `expected_revision`.
    /// A missing file matches revision 0. `profile.revision` must be that value plus one.
    /// Other primary flags cleared by this write are returned so a later failure can
    /// put them back only if they still have the revision this call wrote.
    pub fn compare_and_swap(
        &self,
        expected_revision: u64,
        profile: &BotProfile,
    ) -> Result<Vec<ClearedPrimary>, String> {
        let _guard = lock();
        validate_profile(profile)?;
        match self.load_unlocked(&profile.assistant_id)? {
            None if expected_revision != 0 => {
                return Err("Bot 配置不存在，没有按过期版本创建覆盖".into());
            }
            None => {}
            Some(current) if current.revision != expected_revision => {
                return Err(stale_revision(
                    &current.assistant_id,
                    current.revision,
                    expected_revision,
                ));
            }
            Some(_) => {}
        }
        let Some(next_revision) = expected_revision.checked_add(1) else {
            return Err("Bot 配置版本号已满，没有覆盖".into());
        };
        if profile.revision != next_revision {
            return Err("写入的 Bot 配置版本不是当前版本加一，没有覆盖".into());
        }
        let cleared = self.clear_other_primaries(profile)?;
        if let Err(err) = self.write_unlocked(profile) {
            let rollback = self.restore_cleared_unlocked(&cleared);
            return Err(match rollback {
                Ok(()) => err,
                Err(rollback_err) => format!("{err}；主 Bot 标记回滚：{rollback_err}"),
            });
        }
        Ok(cleared)
    }

    /// Puts `snapshot` back only when the file still has `current_revision`.
    /// A newer revision is left untouched.
    pub fn restore_if_current_revision(
        &self,
        assistant_id: &str,
        current_revision: u64,
        snapshot: &BotProfile,
    ) -> Result<(), String> {
        let _guard = lock();
        if snapshot.assistant_id != assistant_id {
            return Err("回滚配置时助手 id 不一致，没有覆盖其他 Bot".into());
        }
        match self.load_unlocked(assistant_id)? {
            Some(profile) if profile.revision == current_revision => self.write_unlocked(snapshot),
            Some(profile) => Err(stale_revision(
                &profile.assistant_id,
                profile.revision,
                current_revision,
            )),
            None => Err(format!("Bot「{assistant_id}」配置已不存在，没有写回旧版本")),
        }
    }

    pub fn delete_if_current_revision(
        &self,
        assistant_id: &str,
        current_revision: u64,
    ) -> Result<(), String> {
        let _guard = lock();
        match self.load_unlocked(assistant_id)? {
            Some(profile) if profile.revision == current_revision => {
                fs::remove_file(self.path(assistant_id))
                    .map_err(|err| format!("撤销 Bot 配置失败：{err}"))
            }
            Some(profile) => Err(format!(
                "Bot「{}」配置已前进到版本 {}，没有删除",
                profile.assistant_id, profile.revision
            )),
            None => Ok(()),
        }
    }

    pub fn restore_cleared(&self, cleared: &[ClearedPrimary]) -> Result<(), String> {
        let _guard = lock();
        self.restore_cleared_unlocked(cleared)
    }

    #[cfg(test)]
    pub fn update<F>(&self, assistant_id: &str, mutate: F) -> Result<BotProfile, String>
    where
        F: FnOnce(&mut BotProfile) -> Result<(), String>,
    {
        let _guard = lock();
        let mut profile = self
            .load_unlocked(assistant_id)?
            .ok_or_else(|| "Bot 配置不存在".to_string())?;
        mutate(&mut profile)?;
        if profile.assistant_id != assistant_id {
            return Err("Bot 配置不能改助手 id".into());
        }
        validate_profile(&profile)?;
        let cleared = self.clear_other_primaries(&profile)?;
        if let Err(err) = self.write_unlocked(&profile) {
            let rollback = self.restore_cleared_unlocked(&cleared);
            return Err(match rollback {
                Ok(()) => err,
                Err(rollback_err) => format!("{err}；主 Bot 标记回滚：{rollback_err}"),
            });
        }
        Ok(profile)
    }

    fn clear_other_primaries(&self, profile: &BotProfile) -> Result<Vec<ClearedPrimary>, String> {
        let mut cleared = Vec::new();
        if !profile.primary {
            return Ok(cleared);
        }
        for other in self.list_unlocked()? {
            if other.assistant_id == profile.assistant_id || !other.primary {
                continue;
            }
            let Some(written_revision) = other.revision.checked_add(1) else {
                self.restore_cleared_unlocked(&cleared)?;
                return Err(format!(
                    "Bot「{}」配置版本号已满，没有改主 Bot 标记",
                    other.assistant_id
                ));
            };
            let mut next = other.clone();
            next.primary = false;
            next.revision = written_revision;
            if let Err(err) = self.write_unlocked(&next) {
                let rollback = self.restore_cleared_unlocked(&cleared);
                return Err(match rollback {
                    Ok(()) => err,
                    Err(rollback_err) => format!("{err}；主 Bot 标记回滚：{rollback_err}"),
                });
            }
            cleared.push(ClearedPrimary {
                previous: other,
                written_revision,
            });
        }
        Ok(cleared)
    }

    fn restore_cleared_unlocked(&self, cleared: &[ClearedPrimary]) -> Result<(), String> {
        let mut errors = Vec::new();
        for item in cleared {
            if let Err(err) = self.restore_cleared_one(item) {
                errors.push(err);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }

    fn restore_cleared_one(&self, item: &ClearedPrimary) -> Result<(), String> {
        match self.load_unlocked(&item.previous.assistant_id)? {
            Some(current) if current.revision == item.written_revision => {
                self.write_unlocked(&item.previous)
            }
            Some(current) => Err(format!(
                "Bot「{}」配置已前进到版本 {}，没有用版本 {} 覆盖主 Bot 标记",
                current.assistant_id, current.revision, item.previous.revision
            )),
            None => Err(format!(
                "Bot「{}」配置已不存在，没有写回旧的主 Bot 标记",
                item.previous.assistant_id
            )),
        }
    }

    fn rollback_insert_unlocked(&self, restore: &InsertRestore) -> Result<(), String> {
        let mut errors = Vec::new();
        match self.load_unlocked(&restore.created_id)? {
            Some(profile) if profile.revision == restore.created_revision => {
                if let Err(err) = fs::remove_file(self.path(&restore.created_id))
                    .map_err(|err| format!("撤销 Bot 配置失败：{err}"))
                {
                    errors.push(err);
                }
            }
            Some(profile) => errors.push(format!(
                "Bot「{}」配置已前进到版本 {}，没有删除",
                profile.assistant_id, profile.revision
            )),
            None => {}
        }
        if let Err(err) = self.restore_cleared_unlocked(&restore.previous_primaries) {
            errors.push(err);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }

    fn list_unlocked(&self) -> Result<Vec<BotProfile>, String> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let entries =
            fs::read_dir(&self.dir).map_err(|err| format!("读取 Bot 配置目录失败：{err}"))?;
        let mut profiles = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|err| format!("读取 Bot 配置目录失败：{err}"))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with("asst_") || !name.ends_with(".json") {
                continue;
            }
            let id = name.trim_end_matches(".json");
            profiles.push(
                self.load_unlocked(id)?
                    .ok_or_else(|| format!("Bot 配置刚列出就消失了：{id}"))?,
            );
        }
        profiles.sort_by(|a, b| a.assistant_id.cmp(&b.assistant_id));
        Ok(profiles)
    }

    fn load_unlocked(&self, assistant_id: &str) -> Result<Option<BotProfile>, String> {
        let id = validate_assistant_id(assistant_id)?;
        let path = self.path(id);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("无法读取 Bot 配置 {}：{err}", path.display())),
        };
        let text = std::str::from_utf8(&bytes)
            .map_err(|err| format!("无法读取 Bot 配置 {}：{err}", path.display()))?;
        let profile: BotProfile = serde_json::from_str(text)
            .map_err(|err| format!("无法读取 Bot 配置 {}：{err}", path.display()))?;
        if profile.version != PROFILE_VERSION {
            return Err(format!(
                "无法读取 Bot 配置 {}：版本 {} 不受支持，没有覆盖原文件",
                path.display(),
                profile.version
            ));
        }
        if profile.assistant_id != id {
            return Err(format!(
                "无法读取 Bot 配置 {}：文件里的助手 id 与文件名不一致，没有覆盖原文件",
                path.display()
            ));
        }
        Ok(Some(profile))
    }

    fn write_unlocked(&self, profile: &BotProfile) -> Result<(), String> {
        validate_profile(profile)?;
        let path = self.path(&profile.assistant_id);
        let text = serde_json::to_string_pretty(profile)
            .map_err(|err| format!("序列化 Bot 配置失败：{err}"))?;
        crate::chat::storage::atomic_write(&path, &text, "bot profile")
    }

    fn path(&self, assistant_id: &str) -> PathBuf {
        self.dir.join(format!("{assistant_id}.json"))
    }
}

fn stale_revision(assistant_id: &str, current: u64, expected: u64) -> String {
    format!(
        "Bot「{assistant_id}」配置已被其他保存更新（当前版本 {current}，提交基于 {expected}），没有覆盖"
    )
}

fn validate_profile(profile: &BotProfile) -> Result<(), String> {
    validate_assistant_id(&profile.assistant_id)?;
    if profile.version != PROFILE_VERSION {
        return Err(format!("Bot 配置版本 {} 不能写入", profile.version));
    }
    Ok(())
}
