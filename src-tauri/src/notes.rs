//! 笔记中心：每篇笔记一个 `<app_data>/notes/{id}.md` 文件，YAML-ish frontmatter
//! 存 title / 时间戳 / folder / origin，正文即 markdown。frontmatter 解析复用
//! skills 的 `split_frontmatter`，与 agents / skills 的文件格式约定保持一致。
//!
//! 组织方式：
//! - `origin`：`user`（手动新建，进「库」）| `chat`（对话存来，进「聊天保存」）。缺省 = user。
//! - `folder`：单层文件夹名，空 = 库根。仅对 user 笔记有意义。
//! - 空文件夹也要留存，故文件夹清单单独存 `<app_data>/notes/folders.json`（有序数组）。
//! 文件始终扁平存放（id=文件名），移动文件夹只改 frontmatter，查找保持 O(1)。

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::ShellExt;
use uuid::Uuid;

use crate::skills::parse::split_frontmatter;

const ORIGIN_CHAT: &str = "chat";
const ORIGIN_USER: &str = "user";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteMeta {
    pub id: String,
    pub title: String,
    /// 列表卡片预览：正文压成单行的前若干字符。
    pub preview: String,
    /// 单层文件夹名；空串 = 库根。
    pub folder: String,
    /// `user` | `chat`。
    pub origin: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub title: String,
    pub content: String,
    /// 单层文件夹名；空串 = 库根。
    pub folder: String,
    /// `user` | `chat`。
    pub origin: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 用户笔记目录：`<app_data>/notes`。按需创建。
fn notes_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("app_data_dir unavailable: {err}"))?
        .join("notes");
    std::fs::create_dir_all(&dir).map_err(|err| format!("create notes dir failed: {err}"))?;
    Ok(dir)
}

fn folders_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(notes_dir(app)?.join("folders.json"))
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// id 直接拼进文件名，必须是无路径语义的纯标识（仿 `chat_skills_uninstall`）。
fn validate_note_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("invalid note id".to_string());
    }
    Ok(())
}

/// frontmatter 是单行键值格式，换行会破坏解析，写盘前压成一行。标题/文件夹名共用。
fn sanitize_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_origin(origin: &str) -> String {
    if origin == ORIGIN_CHAT {
        ORIGIN_CHAT.to_string()
    } else {
        ORIGIN_USER.to_string()
    }
}

fn serialize_note(note: &Note) -> String {
    let mut out = format!(
        "---\ntitle: {}\ncreated_at: {}\nupdated_at: {}\n",
        sanitize_line(&note.title),
        note.created_at,
        note.updated_at,
    );
    let folder = sanitize_line(&note.folder);
    if !folder.is_empty() {
        out.push_str(&format!("folder: {folder}\n"));
    }
    if note.origin == ORIGIN_CHAT {
        out.push_str("origin: chat\n");
    }
    out.push_str("---\n");
    out.push_str(&note.content);
    out
}

const PREVIEW_MAX_CHARS: usize = 160;

/// 列表卡片预览：正文空白压平后截前 N 字符。
fn make_preview(content: &str) -> String {
    let mut preview = String::new();
    let mut count = 0;
    for word in content.split_whitespace() {
        if count >= PREVIEW_MAX_CHARS {
            break;
        }
        if count > 0 {
            preview.push(' ');
            count += 1;
        }
        for ch in word.chars().take(PREVIEW_MAX_CHARS - count) {
            preview.push(ch);
            count += 1;
        }
    }
    preview
}

/// 从 `{id}.md` 原始文本解析；frontmatter 缺失/损坏时回退到文件名标题与文件
/// mtime，保证用户手工放进目录的 md 也能在列表里出现（解析容错仿 `merge_dir`）。
fn parse_note(id: &str, raw: &str, fallback_updated: Option<DateTime<Utc>>) -> Note {
    let (frontmatter, body) = split_frontmatter(raw);
    // split_frontmatter 的 body 含闭合 `---\n` 后的那个换行；剥掉一个，否则每次保存
    // 正文都会多攒一个前导空行。
    let body = body
        .strip_prefix("\r\n")
        .or_else(|| body.strip_prefix('\n'))
        .unwrap_or(body);
    let fallback_ts = fallback_updated
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default();
    let title = frontmatter
        .get("title")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let created_at = frontmatter
        .get("created_at")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_ts.clone());
    let updated_at = frontmatter
        .get("updated_at")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_ts);
    // 兼容早期版本写过的 `category` 字段。
    let folder = frontmatter
        .get("folder")
        .or_else(|| frontmatter.get("category"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let origin = frontmatter
        .get("origin")
        .map(|s| normalize_origin(s.trim()))
        .unwrap_or_else(|| ORIGIN_USER.to_string());
    Note {
        id: id.to_string(),
        title,
        content: body.to_string(),
        folder,
        origin,
        created_at,
        updated_at,
    }
}

fn read_note_file(path: &std::path::Path) -> Option<Note> {
    let raw = std::fs::read_to_string(path).ok()?;
    let id = path.file_stem()?.to_str()?.to_string();
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(DateTime::<Utc>::from);
    Some(parse_note(&id, &raw, mtime))
}

fn note_path(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    validate_note_id(id)?;
    Ok(notes_dir(app)?.join(format!("{id}.md")))
}

/// Read only enough for the complete header and preview. Grow the prefix for
/// long headers/leading whitespace; a byte cutoff must not change note metadata.
fn read_note_meta(
    mut reader: impl Read,
    id: &str,
    mtime: Option<DateTime<Utc>>,
) -> Option<NoteMeta> {
    let mut raw = Vec::new();
    loop {
        let chunk = raw.len().max(4096);
        let read = reader
            .by_ref()
            .take(chunk as u64)
            .read_to_end(&mut raw)
            .ok()?;
        let eof = read < chunk;
        // An incomplete final UTF-8 character belongs to the next chunk, not
        // the preview. Preserve the usual lossy fallback for malformed files.
        let valid_len = std::str::from_utf8(&raw)
            .err()
            .filter(|error| error.error_len().is_none())
            .map_or(raw.len(), |error| error.valid_up_to());
        let text = String::from_utf8_lossy(&raw[..if eof { raw.len() } else { valid_len }]);
        let trimmed = text.trim_start();
        let incomplete_header = "---".starts_with(trimmed)
            || trimmed
                .strip_prefix("---")
                .is_some_and(|rest| !rest.contains("\n---"));
        if !eof && incomplete_header {
            continue;
        }
        let note = parse_note(id, &text, mtime);
        let preview = make_preview(&note.content);
        if eof || preview.chars().count() == PREVIEW_MAX_CHARS {
            return Some(NoteMeta {
                id: note.id,
                title: note.title,
                preview,
                folder: note.folder,
                origin: note.origin,
                created_at: note.created_at,
                updated_at: note.updated_at,
            });
        }
    }
}

#[derive(Default)]
pub(crate) struct NoteCatalog(Mutex<HashMap<PathBuf, (Option<SystemTime>, u64, NoteMeta)>>);
impl NoteCatalog {
    fn list(&self, dir: &Path) -> Result<Vec<NoteMeta>, String> {
        let mut cached = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = HashMap::new();
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            let stamp = metadata.modified().ok();
            let len = metadata.len();
            if let Some((old_stamp, old_len, meta)) = cached.get(&path) {
                if stamp.is_some() && *old_stamp == stamp && *old_len == len {
                    next.insert(path, (stamp, len, meta.clone()));
                    continue;
                }
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(file) = std::fs::File::open(&path) else {
                continue;
            };
            if let Some(meta) = read_note_meta(file, id, stamp.map(DateTime::<Utc>::from)) {
                next.insert(path, (stamp, len, meta));
            }
        }
        let mut metas: Vec<_> = next.values().map(|(_, _, meta)| meta.clone()).collect();
        *cached = next;
        metas.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(metas)
    }
}

/* ===== 文件夹清单（folders.json，有序） ===== */

fn read_folders_raw(app: &AppHandle) -> Vec<String> {
    let Ok(path) = folders_file(app) else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
}

fn write_folders(app: &AppHandle, folders: &[String]) -> Result<(), String> {
    let path = folders_file(app)?;
    let json = serde_json::to_string_pretty(folders).map_err(|err| err.to_string())?;
    std::fs::write(&path, json).map_err(|err| format!("write folders failed: {err}"))
}

#[tauri::command]
pub fn notes_list(app: AppHandle) -> Result<Vec<NoteMeta>, String> {
    app.state::<crate::state::AppState>()
        .note_catalog
        .list(&notes_dir(&app)?)
}

/// 文件夹清单：folders.json 顺序在前，追加被笔记引用但清单里没有的（容错）。
#[tauri::command]
pub fn notes_folders_list(app: AppHandle) -> Result<Vec<String>, String> {
    let mut folders = read_folders_raw(&app);
    for note in notes_list(app.clone())? {
        let f = note.folder.trim();
        if !f.is_empty() && !folders.iter().any(|x| x == f) {
            folders.push(f.to_string());
        }
    }
    Ok(folders)
}

#[tauri::command]
pub fn notes_folder_create(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    let name = sanitize_line(&name);
    if name.is_empty() {
        return Err("folder name empty".to_string());
    }
    let mut folders = read_folders_raw(&app);
    if !folders.iter().any(|x| x == &name) {
        folders.push(name);
        write_folders(&app, &folders)?;
    }
    notes_folders_list(app)
}

#[tauri::command]
pub fn notes_folder_rename(
    app: AppHandle,
    old: String,
    new: String,
) -> Result<Vec<String>, String> {
    let new = sanitize_line(&new);
    if new.is_empty() {
        return Err("folder name empty".to_string());
    }
    // 清单里替换（保序）。
    let mut folders = read_folders_raw(&app);
    for f in folders.iter_mut() {
        if f == &old {
            *f = new.clone();
        }
    }
    if !folders.iter().any(|x| x == &new) {
        folders.push(new.clone());
    }
    write_folders(&app, &folders)?;
    // 归属该文件夹的笔记改到新名。
    for meta in notes_list(app.clone())?
        .into_iter()
        .filter(|meta| meta.folder == old)
    {
        let path = note_path(&app, &meta.id)?;
        if let Some(mut note) = read_note_file(&path) {
            note.folder = new.clone();
            note.updated_at = now_iso();
            std::fs::write(&path, serialize_note(&note))
                .map_err(|err| format!("write note failed: {err}"))?;
        }
    }
    notes_folders_list(app)
}

/// 删除文件夹：清单移除，其中的笔记回到库根（folder=""），不删笔记。
#[tauri::command]
pub fn notes_folder_delete(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    let mut folders = read_folders_raw(&app);
    folders.retain(|x| x != &name);
    write_folders(&app, &folders)?;
    for meta in notes_list(app.clone())?
        .into_iter()
        .filter(|meta| meta.folder == name)
    {
        let path = note_path(&app, &meta.id)?;
        if let Some(mut note) = read_note_file(&path) {
            note.folder = String::new();
            note.updated_at = now_iso();
            std::fs::write(&path, serialize_note(&note))
                .map_err(|err| format!("write note failed: {err}"))?;
        }
    }
    notes_folders_list(app)
}

#[tauri::command]
pub fn notes_read(app: AppHandle, id: String) -> Result<Note, String> {
    let path = note_path(&app, &id)?;
    read_note_file(&path).ok_or_else(|| format!("note not found: {id}"))
}

#[tauri::command]
pub fn notes_create(
    app: AppHandle,
    title: String,
    content: String,
    folder: String,
    origin: String,
) -> Result<Note, String> {
    let now = now_iso();
    let note = Note {
        id: Uuid::new_v4().to_string(),
        title: sanitize_line(&title),
        content,
        folder: sanitize_line(&folder),
        origin: normalize_origin(&origin),
        created_at: now.clone(),
        updated_at: now,
    };
    let path = note_path(&app, &note.id)?;
    std::fs::write(&path, serialize_note(&note))
        .map_err(|err| format!("write note failed: {err}"))?;
    Ok(note)
}

/// 更新：origin 保持不变（笔记来源不因编辑而变），folder 可改（移动文件夹）。
#[tauri::command]
pub fn notes_update(
    app: AppHandle,
    id: String,
    title: String,
    content: String,
    folder: String,
) -> Result<Note, String> {
    let path = note_path(&app, &id)?;
    let existing = read_note_file(&path).ok_or_else(|| format!("note not found: {id}"))?;
    let note = Note {
        id: existing.id,
        title: sanitize_line(&title),
        content,
        folder: sanitize_line(&folder),
        origin: existing.origin,
        created_at: existing.created_at,
        updated_at: now_iso(),
    };
    std::fs::write(&path, serialize_note(&note))
        .map_err(|err| format!("write note failed: {err}"))?;
    Ok(note)
}

#[tauri::command]
pub fn notes_delete(app: AppHandle, id: String) -> Result<(), String> {
    let path = note_path(&app, &id)?;
    if !path.exists() {
        return Err(format!("note not found: {id}"));
    }
    std::fs::remove_file(&path).map_err(|err| format!("delete note failed: {err}"))
}

/// 笔记目录的绝对路径。前端用它订阅 `workspace:activity`，这样用户往目录里拖进
/// 外部 `.md` 后列表自己刷新，不需要手动触发。
#[tauri::command]
pub fn notes_dir_path(app: AppHandle) -> Result<String, String> {
    Ok(notes_dir(&app)?.display().to_string())
}

/// 在系统文件管理器里打开笔记目录，让用户直接拖入外部 `.md`。
/// 目录是扁平的、解析对手工文件容错（`parse_note` 缺 frontmatter 时回退文件名 +
/// mtime），所以拖进来的文件会被下一次 `notes_list` 读到，无需导入步骤。
#[tauri::command]
pub fn notes_open_folder(app: AppHandle) -> Result<String, String> {
    let dir = notes_dir(&app)?;
    let path = dir.display().to_string();
    app.shell()
        .open(&path, None)
        .map_err(|err| format!("open notes dir failed: {err}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(folder: &str, origin: &str) -> Note {
        Note {
            id: "x".into(),
            title: "标题".into(),
            content: "正文\n第二行".into(),
            folder: folder.into(),
            origin: origin.into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-02T00:00:00Z".into(),
        }
    }

    #[test]
    fn metadata_preserves_long_headers_and_previews_after_leading_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let mut note = sample("folder", ORIGIN_CHAT);
        note.title = "长标题".repeat(24 * 1024);
        note.content = format!("{}可见正文", " \n".repeat(40 * 1024));
        std::fs::write(dir.path().join("long.md"), serialize_note(&note)).unwrap();
        let metas = NoteCatalog::default().list(dir.path()).unwrap();
        assert!(
            metas[0].title == note.title,
            "the complete title must survive metadata reads"
        );
        assert_eq!(metas[0].folder, "folder");
        assert_eq!(metas[0].origin, ORIGIN_CHAT);
        assert_eq!(metas[0].preview, "可见正文");
        std::fs::write(
            dir.path().join("plain.md"),
            format!("{}正文", " ".repeat(80 * 1024)),
        )
        .unwrap();
        let metas = NoteCatalog::default().list(dir.path()).unwrap();
        assert_eq!(
            metas.iter().find(|m| m.id == "plain").unwrap().preview,
            "正文"
        );
    }

    #[test]
    fn metadata_does_not_read_large_note_bodies_and_tracks_external_edits() {
        let mut note = sample("old", ORIGIN_USER);
        note.content = "正文 ".repeat(1024 * 1024);
        let raw = serialize_note(&note);
        let mut reader = std::io::Cursor::new(raw.as_bytes());
        let meta = read_note_meta(&mut reader, "x", None).unwrap();
        assert_eq!(meta.preview, make_preview(&note.content));
        assert!(
            reader.position() <= 64 * 1024,
            "listing must not read the full multi-MB body"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.md");
        std::fs::write(&path, raw).unwrap();
        let catalog = NoteCatalog::default();
        assert_eq!(catalog.list(dir.path()).unwrap()[0].folder, "old");
        note.folder = "new folder".into();
        note.content = "changed".into();
        std::fs::write(&path, serialize_note(&note)).unwrap();
        let updated = catalog.list(dir.path()).unwrap();
        assert_eq!(updated[0].folder, "new folder");
        assert_eq!(updated[0].preview, "changed");
        std::fs::remove_file(path).unwrap();
        assert!(catalog.list(dir.path()).unwrap().is_empty());
        std::fs::write(dir.path().join("external.md"), "external note").unwrap();
        assert_eq!(
            catalog.list(dir.path()).unwrap()[0].preview,
            "external note"
        );
    }

    #[test]
    fn folder_and_content_roundtrip() {
        let raw = serialize_note(&sample("工作", ORIGIN_USER));
        assert!(raw.contains("folder: 工作"));
        assert!(!raw.contains("origin:")); // user 缺省不写
        let parsed = parse_note("x", &raw, None);
        assert_eq!(parsed.folder, "工作");
        assert_eq!(parsed.origin, "user");
        assert_eq!(parsed.content, "正文\n第二行");
    }

    #[test]
    fn chat_origin_roundtrips() {
        let raw = serialize_note(&sample("", ORIGIN_CHAT));
        assert!(raw.contains("origin: chat"));
        assert!(!raw.contains("folder:"));
        let parsed = parse_note("x", &raw, None);
        assert_eq!(parsed.origin, "chat");
        assert_eq!(parsed.folder, "");
    }

    #[test]
    fn legacy_category_maps_to_folder() {
        let raw = "---\ntitle: T\ncreated_at: a\nupdated_at: b\ncategory: 旧类\n---\n正文";
        assert_eq!(parse_note("x", raw, None).folder, "旧类");
    }
}
